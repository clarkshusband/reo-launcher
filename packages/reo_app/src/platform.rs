use std::collections::HashSet;

use freya::prelude::{Platform, WinitPlatformExt, spawn_forever};
use freya::radio::RadioStation;
use reo_core::settings::LaunchBehaviour;

use crate::state::{AppChannel, AppState};

const ALLOWED_URL_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

pub fn focus_window() {
    Platform::get().with_window(Platform::window_id(), |win| {
        win.set_visible(true);
        win.set_minimized(false);
        win.focus_window();
    });
}

pub fn hide_window() {
    Platform::get().with_window(Platform::window_id(), |win| {
        win.set_visible(false);
    });
}

pub fn close() {
    let background =
        crate::launcher::state().is_ok_and(|state| state.settings.read().run_in_background);
    if background { hide_window() } else { quit() }
}

pub fn quit() {
    let platform = Platform::get();
    spawn_forever(async move {
        if let Ok(state) = crate::launcher::state() {
            reo_core::shutdown(&state).await;
        }
        drop(platform.post_callback(|_, ctx| ctx.exit()));
    });
}

pub fn follow_game(
    station: &RadioStation<AppState, AppChannel>,
    launched: &HashSet<i64>,
    previous: Option<i64>,
) -> Option<i64> {
    let (current, any, behaviour) = {
        let state = station.peek();
        (
            state
                .game
                .running_clusters()
                .filter(|id| launched.contains(id))
                .min(),
            state.game.running_clusters().next().is_some(),
            state.settings.settings.launch_behaviour,
        )
    };

    tray::set_game_running(any);

    if current == previous {
        return previous;
    }

    match (behaviour, current.is_some()) {
        (LaunchBehaviour::HideWhilePlaying, true) => hide_window(),
        (LaunchBehaviour::HideWhilePlaying, false) => focus_window(),
        (LaunchBehaviour::CloseLauncher, true) => quit(),
        _ => {}
    }

    current
}

pub fn open_url(url: &str) {
    if !has_allowed_scheme(url) {
        tracing::warn!("refused to open url with a disallowed scheme: {url}");
        return;
    }

    open_target(url);
}

/// For paths the launcher itself produced, which carry no url scheme.
pub fn open_path(path: &str) {
    open_target(path);
}

fn has_allowed_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };

    ALLOWED_URL_SCHEMES
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}

#[cfg(target_os = "linux")]
fn open_target(target: &str) {
    use std::process::Stdio;

    let spawned = open::commands(target).into_iter().find_map(|mut command| {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    });

    match spawned {
        Some(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        None => tracing::warn!("failed to open {target}"),
    }
}

#[cfg(not(target_os = "linux"))]
fn open_target(target: &str) {
    let target = target.to_owned();
    std::thread::spawn(move || {
        if let Err(err) = open::that_detached(&target) {
            tracing::warn!("failed to open {target}: {err}");
        }
    });
}

pub fn copy_image_to_clipboard(path: std::path::PathBuf) {
    std::thread::spawn(move || {
        let img = match image::open(&path) {
            Ok(img) => img.into_rgba8(),
            Err(err) => {
                tracing::warn!("failed to decode {} for clipboard: {err}", path.display());
                return;
            }
        };
        let (width, height) = (img.width() as usize, img.height() as usize);
        let data = arboard::ImageData {
            width,
            height,
            bytes: std::borrow::Cow::Owned(img.into_raw()),
        };
        match arboard::Clipboard::new() {
            Ok(mut clip) => {
                if let Err(err) = clip.set_image(data) {
                    tracing::warn!("failed to copy image to clipboard: {err}");
                }
            }
            Err(err) => tracing::warn!("failed to open clipboard: {err}"),
        }
    });
}

pub mod tray {
    use std::cell::RefCell;

    use freya::prelude::{LaunchConfig, RendererContext};
    use freya::tray::menu::{Menu, MenuItem, PredefinedMenuItem};
    use freya::tray::{TrayEvent, TrayIcon, TrayIconBuilder};

    use crate::constants;
    use crate::ipc::{self, IpcCommand};

    const OPEN_ID: &str = "oneclient.open";
    const PLAY_ID: &str = "oneclient.play";
    const STOP_ID: &str = "oneclient.stop";
    const LOGS_ID: &str = "oneclient.logs";
    const QUIT_ID: &str = "oneclient.quit";

    const ICON: &[u8] = include_bytes!("../icons/64x64.png");

    thread_local! {
        static GAME_ITEMS: RefCell<Vec<MenuItem>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn set_game_running(running: bool) {
        GAME_ITEMS.with_borrow(|items| {
            for item in items {
                item.set_enabled(running);
            }
        });
    }

    #[cfg(target_os = "linux")]
    pub fn available() -> bool {
        let loaded = ["libayatana-appindicator3.so.1", "libappindicator3.so.1"]
            .into_iter()
            .any(|name| unsafe { libloading::Library::new(name) }.is_ok());
        if !loaded {
            tracing::warn!("tray icon disabled, appindicator failed to load");
        }
        loaded
    }

    #[cfg(not(target_os = "linux"))]
    pub fn available() -> bool {
        true
    }

    pub fn build() -> TrayIcon {
        let live_on_linux = cfg!(target_os = "linux");
        let stop = MenuItem::with_id(STOP_ID, "Stop game", live_on_linux, None);
        let logs = MenuItem::with_id(LOGS_ID, "Show logs", live_on_linux, None);

        let menu = Menu::with_items(&[
            &MenuItem::with_id(OPEN_ID, "Open Reo Launcher", true, None),
            &MenuItem::with_id(PLAY_ID, "Play last version", true, None),
            &PredefinedMenuItem::separator(),
            &stop,
            &logs,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(QUIT_ID, "Quit Reo Launcher", true, None),
        ]);

        let mut builder = TrayIconBuilder::new()
            .with_tooltip(constants::WINDOW_TITLE)
            .with_icon(LaunchConfig::tray_icon(ICON))
            .with_menu_on_left_click(!cfg!(target_os = "windows"));

        match menu {
            Ok(menu) => {
                builder = builder.with_menu(Box::new(menu));
                GAME_ITEMS.with_borrow_mut(|items| *items = vec![stop, logs]);
            }
            Err(err) => {
                tracing::error!("failed to build the tray menu, showing the icon alone: {err}")
            }
        }

        match builder.build() {
            Ok(icon) => icon,
            Err(err) => {
                tracing::error!("failed to create the tray icon, carrying on without one: {err}");
                GAME_ITEMS.with_borrow_mut(|items| items.clear());

                // Freya has to be handed an icon back so a bare hidden one stands in
                let bare = TrayIconBuilder::new()
                    .build()
                    .expect("failed to create even a bare tray icon");
                let _ = bare.set_visible(false);
                bare
            }
        }
    }

    pub fn handle(event: TrayEvent, _ctx: RendererContext<'_>) {
        let command = match event {
            TrayEvent::Menu(menu) => match menu.id.0.as_str() {
                OPEN_ID => IpcCommand::Focus,
                PLAY_ID => return play_last(),
                STOP_ID => IpcCommand::Stop,
                LOGS_ID => IpcCommand::Logs,
                QUIT_ID => IpcCommand::Quit,
                _ => return,
            },
            #[cfg(target_os = "windows")]
            TrayEvent::Icon(freya::tray::TrayIconEvent::Click {
                button: freya::tray::MouseButton::Left,
                button_state: freya::tray::MouseButtonState::Up,
                ..
            }) => IpcCommand::Focus,
            TrayEvent::Icon(_) => return,
        };
        ipc::send(command);
    }

    fn play_last() {
        tokio::spawn(async {
            let Ok(state) = crate::launcher::state() else {
                return;
            };
            let Ok(clusters) = state.clusters.list().await else {
                return;
            };
            if let Some(cluster) = crate::utils::sort_clusters_for_home(clusters)
                .into_iter()
                .next()
            {
                ipc::send(IpcCommand::Launch(cluster.folder_name));
            }
        });
    }
}
