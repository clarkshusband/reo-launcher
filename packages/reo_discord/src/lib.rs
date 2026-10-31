use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use discord_rich_presence::activity::{Activity, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

use reo_common::constants::DISCORD_CLIENT_ID;
use reo_common::domain::{ContentType, GameLoader};

const RECONNECT_BACKOFF: Duration = Duration::from_secs(60);

const IDLE_POLL: Duration = Duration::from_secs(3600);

/// Page changes land in bursts. Waiting this long folds a burst into one update and keeps
/// clear of Discord's limit of five activity updates every twenty seconds
const ACTIVITY_DEBOUNCE: Duration = Duration::from_secs(2);

/// Asset key of the large Rich Presence image. Must match an image uploaded
/// under Rich Presence → Art Assets of the Discord application referenced by
/// `DISCORD_CLIENT_ID`.
const LARGE_IMAGE: &str = "reo_logo";
const LARGE_TEXT: &str = concat!("Reo Launcher ", env!("CARGO_PKG_VERSION"));

/// Discord takes an https URL in place of an uploaded asset key, so the loader marks the
/// launcher already ships are served straight from the repository
const LOADER_ICON_BASE: &str = "https://raw.githubusercontent.com/clarkshusband/reo-launcher/main/packages/reo_app/assets/icons";

const DOWNLOAD_URL: &str = "https://github.com/clarkshusband/reo-launcher/releases/latest";
const COMMUNITY_URL: &str = "https://discord.gg/ReoMc";

/// Discord refuses activity text longer than this
const MAX_TEXT_CHARS: usize = 128;

/// A game the launcher started or re-adopted
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameSession {
    pub cluster: String,
    pub mc_version: String,
    pub loader: GameLoader,
    /// Mods enabled when the game started, `None` when the folder could not be read
    pub mods: Option<usize>,
}

/// What the launcher window shows, used while no game is running
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LauncherActivity {
    #[default]
    Home,
    Browsing(ContentType),
    Instances,
    Settings,
}

/// Which details the player lets Discord show to others
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceOptions {
    /// Instance name, version, loader and mod count while playing
    pub show_game_details: bool,
    /// What the launcher is showing while no game runs
    pub show_launcher_activity: bool,
}

impl Default for PresenceOptions {
    fn default() -> Self {
        Self {
            show_game_details: true,
            show_launcher_activity: true,
        }
    }
}

/// The text and images of one activity, kept so an unchanged one is never sent twice
#[derive(Debug, Clone, PartialEq, Eq)]
struct Card {
    details: String,
    state: Option<String>,
    start: i64,
    small_image: Option<(String, &'static str)>,
}

fn card(
    game: Option<&(GameSession, i64)>,
    activity: LauncherActivity,
    launcher_since: i64,
    options: PresenceOptions,
) -> Card {
    match game {
        Some((session, since)) if options.show_game_details => Card {
            details: clamp(&format!("Playing {}", session.cluster)),
            state: Some(clamp(&game_line(session))),
            start: *since,
            small_image: loader_icon(session.loader),
        },
        Some((_, since)) => Card {
            details: "Playing Minecraft".to_owned(),
            state: None,
            start: *since,
            small_image: None,
        },
        None => {
            let (details, state) = if options.show_launcher_activity {
                launcher_lines(activity)
            } else {
                ("In the launcher", None)
            };
            Card {
                details: details.to_owned(),
                state: state.map(str::to_owned),
                // One timer for the whole visit, so moving between pages does not reset it
                start: launcher_since,
                small_image: None,
            }
        }
    }
}

fn game_line(session: &GameSession) -> String {
    let mut parts = vec![format!("Minecraft {}", session.mc_version)];
    if session.loader.is_modded() {
        parts.push(loader_label(session.loader).to_owned());
        match session.mods {
            Some(1) => parts.push("1 mod".to_owned()),
            Some(count) if count > 0 => parts.push(format!("{count} mods")),
            _ => {}
        }
    }
    parts.join(" · ")
}

fn launcher_lines(activity: LauncherActivity) -> (&'static str, Option<&'static str>) {
    match activity {
        LauncherActivity::Home => ("In the launcher", Some("Picking what to play")),
        LauncherActivity::Browsing(kind) => {
            (browsing_line(kind), Some("Looking for something new"))
        }
        LauncherActivity::Instances => ("Managing instances", None),
        LauncherActivity::Settings => ("Tweaking settings", None),
    }
}

fn browsing_line(kind: ContentType) -> &'static str {
    match kind {
        ContentType::Mod => "Browsing mods",
        ContentType::ResourcePack => "Browsing resource packs",
        ContentType::Shader => "Browsing shaders",
        ContentType::DataPack => "Browsing data packs",
        ContentType::World => "Browsing worlds",
        ContentType::Modpack => "Browsing modpacks",
    }
}

fn loader_label(loader: GameLoader) -> &'static str {
    match loader {
        GameLoader::Fabric => "Fabric",
        GameLoader::Forge => "Forge",
        GameLoader::NeoForge => "NeoForge",
        GameLoader::Quilt => "Quilt",
        GameLoader::Ornithe => "Ornithe",
        GameLoader::Vanilla => "Vanilla",
    }
}

/// Only loaders the launcher ships a raster mark for; Discord does not draw SVG
fn loader_icon(loader: GameLoader) -> Option<(String, &'static str)> {
    let file = match loader {
        GameLoader::Fabric => "fabric.png",
        GameLoader::Forge => "forge.png",
        GameLoader::NeoForge => "neo-forge.png",
        _ => return None,
    };
    Some((format!("{LOADER_ICON_BASE}/{file}"), loader_label(loader)))
}

fn clamp(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT_CHARS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(MAX_TEXT_CHARS - 1).collect();
    cut.push('…');
    cut
}

enum Command {
    SetGame(Option<GameSession>),
    SetActivity(LauncherActivity),
    SetOptions(PresenceOptions),
    SetEnabled(bool),
    Shutdown,
}

#[derive(Clone)]
pub struct DiscordRpc {
    tx: Sender<Command>,
}

impl DiscordRpc {
    pub fn spawn(enabled: bool, options: PresenceOptions) -> Self {
        let (tx, rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("discord-rpc".to_owned())
            .spawn(move || Worker::new(enabled, options).run(&rx))
            .expect("failed to spawn the Discord RPC thread");

        Self { tx }
    }

    /// `None` once no game is running, which hands the presence back to the launcher activity
    pub fn set_game(&self, game: Option<GameSession>) {
        self.send(Command::SetGame(game));
    }

    pub fn set_activity(&self, activity: LauncherActivity) {
        self.send(Command::SetActivity(activity));
    }

    pub fn set_options(&self, options: PresenceOptions) {
        self.send(Command::SetOptions(options));
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.send(Command::SetEnabled(enabled));
    }

    pub fn shutdown(&self) {
        self.send(Command::Shutdown);
    }

    fn send(&self, command: Command) {
        if self.tx.send(command).is_err() {
            tracing::debug!("Discord RPC worker is gone; dropping command");
        }
    }
}

struct Worker {
    client: DiscordIpcClient,
    enabled: bool,
    connected: bool,
    ever_connected: bool,
    game: Option<(GameSession, i64)>,
    activity: LauncherActivity,
    options: PresenceOptions,
    launcher_since: i64,
    /// What Discord shows right now, `None` until something was sent on this connection
    shown: Option<Card>,
    /// A debounced update waiting to go out
    publish_at: Option<Instant>,
    next_attempt: Instant,
}

impl Worker {
    fn new(enabled: bool, options: PresenceOptions) -> Self {
        Self {
            client: DiscordIpcClient::new(DISCORD_CLIENT_ID),
            enabled,
            connected: false,
            ever_connected: false,
            game: None,
            activity: LauncherActivity::default(),
            options,
            launcher_since: chrono::Utc::now().timestamp(),
            shown: None,
            publish_at: None,
            next_attempt: Instant::now(),
        }
    }

    fn run(mut self, rx: &mpsc::Receiver<Command>) {
        self.publish();

        loop {
            match rx.recv_timeout(self.poll_interval()) {
                Ok(Command::SetGame(game)) => self.set_game(game),
                Ok(Command::SetActivity(activity)) => self.set_activity(activity),
                Ok(Command::SetOptions(options)) => self.set_options(options),
                Ok(Command::SetEnabled(enabled)) => self.set_enabled(enabled),
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    self.publish_at = None;
                    self.publish();
                }
            }
        }

        self.disconnect();
    }

    fn poll_interval(&self) -> Duration {
        let reconnect = if !self.enabled || self.connected {
            IDLE_POLL
        } else {
            self.next_attempt.saturating_duration_since(Instant::now())
        };
        let pending = self
            .publish_at
            .map_or(IDLE_POLL, |at| at.saturating_duration_since(Instant::now()));

        reconnect.min(pending).max(Duration::from_millis(100))
    }

    fn set_game(&mut self, game: Option<GameSession>) {
        let same = match (&self.game, &game) {
            (Some((current, _)), Some(next)) => current == next,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }

        self.game = game.map(|session| (session, chrono::Utc::now().timestamp()));
        // Starting or closing a game is worth showing straight away
        self.publish_at = None;
        self.publish();
    }

    fn set_activity(&mut self, activity: LauncherActivity) {
        if self.activity == activity {
            return;
        }
        self.activity = activity;
        self.schedule();
    }

    fn set_options(&mut self, options: PresenceOptions) {
        if self.options == options {
            return;
        }
        self.options = options;
        self.schedule();
    }

    fn schedule(&mut self) {
        if self.publish_at.is_none() {
            self.publish_at = Some(Instant::now() + ACTIVITY_DEBOUNCE);
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }

        self.enabled = enabled;

        if enabled {
            self.next_attempt = Instant::now();
            self.publish();
        } else {
            self.disconnect();
        }
    }

    fn publish(&mut self) {
        if !self.enabled || !self.ensure_connected() {
            return;
        }

        let card = card(
            self.game.as_ref(),
            self.activity,
            self.launcher_since,
            self.options,
        );
        if self.shown.as_ref() == Some(&card) {
            return;
        }

        let mut assets = Assets::new()
            .large_image(LARGE_IMAGE)
            .large_text(LARGE_TEXT);
        if let Some((url, text)) = &card.small_image {
            assets = assets.small_image(url.as_str()).small_text(*text);
        }
        let mut activity = Activity::new()
            .details(&card.details)
            .timestamps(Timestamps::new().start(card.start))
            .assets(assets)
            .buttons(vec![
                Button::new("Get Reo Launcher", DOWNLOAD_URL),
                Button::new("Join the Discord", COMMUNITY_URL),
            ]);
        if let Some(state) = &card.state {
            activity = activity.state(state);
        }

        match self.client.set_activity(activity) {
            Ok(()) => self.shown = Some(card),
            Err(err) => {
                tracing::warn!("Discord RPC set_activity failed: {err}");
                self.drop_connection();
            }
        }
    }

    fn ensure_connected(&mut self) -> bool {
        if self.connected {
            return true;
        }

        if Instant::now() < self.next_attempt {
            return false;
        }
        self.next_attempt = Instant::now() + RECONNECT_BACKOFF;

        let result = if self.ever_connected {
            self.client.reconnect()
        } else {
            self.client.connect()
        };

        match result {
            Ok(()) => {
                tracing::info!("Discord RPC connected");
                self.connected = true;
                self.ever_connected = true;
                // A fresh connection starts blank on Discord's side
                self.shown = None;
                true
            }
            Err(err) => {
                tracing::debug!("Discord RPC connect failed (is Discord running?): {err}");
                false
            }
        }
    }

    fn drop_connection(&mut self) {
        self.connected = false;
        self.shown = None;
        self.next_attempt = Instant::now() + RECONNECT_BACKOFF;
    }

    fn disconnect(&mut self) {
        if !self.connected {
            return;
        }

        let _ = self.client.clear_activity();
        let _ = self.client.close();
        self.connected = false;
        self.shown = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(loader: GameLoader, mods: Option<usize>) -> GameSession {
        GameSession {
            cluster: "Wilderness Bound".to_owned(),
            mc_version: "26.3".to_owned(),
            loader,
            mods,
        }
    }

    const EVERYTHING: PresenceOptions = PresenceOptions {
        show_game_details: true,
        show_launcher_activity: true,
    };
    const NOTHING: PresenceOptions = PresenceOptions {
        show_game_details: false,
        show_launcher_activity: false,
    };

    #[test]
    fn a_modded_game_names_the_instance_version_loader_and_mods() {
        let game = (session(GameLoader::Fabric, Some(42)), 100);
        let card = card(Some(&game), LauncherActivity::Home, 5, EVERYTHING);

        assert_eq!(card.details, "Playing Wilderness Bound");
        assert_eq!(
            card.state.as_deref(),
            Some("Minecraft 26.3 · Fabric · 42 mods")
        );
        assert_eq!(card.start, 100, "the timer runs from when the game started");
        let (url, text) = card.small_image.expect("Fabric has a mark");
        assert!(url.ends_with("/fabric.png"), "{url}");
        assert_eq!(text, "Fabric");
    }

    #[test]
    fn mod_counts_read_naturally_and_vanilla_lists_none() {
        let line = |loader, mods| game_line(&session(loader, mods));
        assert_eq!(
            line(GameLoader::Forge, Some(1)),
            "Minecraft 26.3 · Forge · 1 mod"
        );
        assert_eq!(
            line(GameLoader::NeoForge, Some(0)),
            "Minecraft 26.3 · NeoForge"
        );
        assert_eq!(line(GameLoader::Quilt, None), "Minecraft 26.3 · Quilt");
        assert_eq!(line(GameLoader::Vanilla, Some(3)), "Minecraft 26.3");
    }

    #[test]
    fn hidden_game_details_show_only_that_minecraft_is_running() {
        let game = (session(GameLoader::Fabric, Some(42)), 100);
        let card = card(Some(&game), LauncherActivity::Home, 5, NOTHING);

        assert_eq!(card.details, "Playing Minecraft");
        assert_eq!(card.state, None);
        assert_eq!(card.small_image, None);
    }

    #[test]
    fn a_running_game_outranks_whatever_the_launcher_shows() {
        let game = (session(GameLoader::Fabric, None), 100);
        let browsing = LauncherActivity::Browsing(ContentType::Mod);
        assert_eq!(
            card(Some(&game), browsing, 5, EVERYTHING).details,
            "Playing Wilderness Bound"
        );
    }

    #[test]
    fn launcher_activity_follows_the_page_and_keeps_one_timer() {
        let browsing = card(
            None,
            LauncherActivity::Browsing(ContentType::Shader),
            5,
            EVERYTHING,
        );
        assert_eq!(browsing.details, "Browsing shaders");
        assert_eq!(browsing.start, 5);

        let settings = card(None, LauncherActivity::Settings, 5, EVERYTHING);
        assert_eq!(settings.details, "Tweaking settings");
        assert_eq!(settings.start, browsing.start);
    }

    #[test]
    fn hidden_launcher_activity_reads_the_same_on_every_page() {
        for activity in [
            LauncherActivity::Home,
            LauncherActivity::Browsing(ContentType::Modpack),
            LauncherActivity::Instances,
        ] {
            let card = card(None, activity, 5, NOTHING);
            assert_eq!(card.details, "In the launcher");
            assert_eq!(card.state, None);
        }
    }

    #[test]
    fn overlong_instance_names_are_cut_to_what_discord_accepts() {
        let mut long = session(GameLoader::Fabric, None);
        long.cluster = "x".repeat(300);
        let card = card(Some(&(long, 1)), LauncherActivity::Home, 1, EVERYTHING);

        assert_eq!(card.details.chars().count(), MAX_TEXT_CHARS);
        assert!(card.details.ends_with('…'));
    }

    #[test]
    fn only_loaders_with_a_raster_mark_get_a_small_image() {
        assert!(loader_icon(GameLoader::NeoForge).is_some());
        assert!(loader_icon(GameLoader::Quilt).is_none());
        assert!(loader_icon(GameLoader::Vanilla).is_none());
    }
}
