use std::cell::Cell;

use cargo_packager_updater::{Config, Update, check_update};
use reo_events::{Choice, EventBus, Prompt};
use uuid::Uuid;

use crate::constants::{RELEASES_URL, UPDATER_ENDPOINT, UPDATER_PUBKEY};

pub const UPDATE_CHOICE_INSTALL: &str = "update.install";

enum UpdateAnswer {
    Install,
}

fn update_prompt(version: &str) -> Prompt<UpdateAnswer> {
    Prompt::new(
        "Update available",
        format!("Reo Launcher {version} is ready to install. Download and install it now?"),
    )
    .option(
        Choice::primary(UPDATE_CHOICE_INSTALL, "Install"),
        UpdateAnswer::Install,
    )
    .dismiss("Not now")
}

const PROGRESS_STEP: u64 = 256 * 1024;

pub fn spawn_update_check(auto_install: bool, events: EventBus) {
    tokio::spawn(async move {
        if let Err(err) = run_check(auto_install, events).await {
            tracing::warn!("update check failed: {err:#}");
        }
    });
}

/// Debug-only drives the full auto-update UX
pub fn spawn_simulated_update() {
    tokio::spawn(async move {
        if let Err(err) = run_simulated_update().await {
            tracing::warn!("simulated update failed: {err:#}");
        }
    });
}

async fn run_simulated_update() -> anyhow::Result<()> {
    const FAKE_VERSION: &str = "9999.9999.9999";
    const FAKE_TOTAL: u64 = 48 * 1024 * 1024;

    let events = crate::launcher::state()?.services.events.clone();

    if events.ask(update_prompt(FAKE_VERSION)).await?.is_none() {
        tracing::info!("user declined simulated update");
        return Ok(());
    }

    let progress_id = Uuid::new_v4();
    let label = format!("Downloading Reo Launcher {FAKE_VERSION}");

    let mut downloaded = 0u64;
    events.progress(progress_id, &label, downloaded, FAKE_TOTAL);
    while downloaded < FAKE_TOTAL {
        downloaded = (downloaded + PROGRESS_STEP * 8).min(FAKE_TOTAL);
        events.progress(progress_id, &label, downloaded, FAKE_TOTAL);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    events.finish_progress(
        progress_id,
        "Finished Downloading",
        format!("Reo Launcher {FAKE_VERSION} is ready. Restart to apply."),
    );

    Ok(())
}

async fn run_check(auto_install: bool, events: EventBus) -> anyhow::Result<()> {
    // `check_update` performs a blocking HTTP request so offload it to a thread pool
    let Some(update) = tokio::task::spawn_blocking(check_for_update).await?? else {
        tracing::info!("no update available");
        return Ok(());
    };

    tracing::info!("update available: {}", update.version);

    // cargo-packager-updater can only replace an AppImage in place deb/rpm installs live
    // under a package-managed path that an install would fail on or clobber
    if !can_self_update() {
        tracing::info!("install is not self-updatable (non-AppImage Linux); notifying only");
        events
            .notify("Update available")
            .body(format!(
                "Reo Launcher {} is available. Download the latest package from {} to update.",
                update.version, RELEASES_URL
            ))
            .send();
        return Ok(());
    }

    if !auto_install && events.ask(update_prompt(&update.version)).await?.is_none() {
        tracing::info!("user declined update {}", update.version);
        return Ok(());
    }

    download_and_install(update, events).await
}

fn can_self_update() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }

    if std::env::var_os("REO_DISABLE_AUTOUPDATE")
        .or_else(|| std::env::var_os("ONECLIENT_DISABLE_AUTOUPDATE"))
        .is_some_and(|val| {
            ["1", "true", "yes"]
                .into_iter()
                .any(|on| val.eq_ignore_ascii_case(on))
        })
    {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        std::env::var_os("APPIMAGE").is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

fn check_for_update() -> anyhow::Result<Option<Update>> {
    let current = env!("CARGO_PKG_VERSION").parse()?;
    let config = Config {
        endpoints: vec![UPDATER_ENDPOINT.parse()?],
        pubkey: UPDATER_PUBKEY.into(),
        ..Default::default()
    };

    Ok(check_update(current, config)?)
}

async fn download_and_install(update: Update, events: EventBus) -> anyhow::Result<()> {
    let progress_id = Uuid::new_v4();
    let version = update.version.clone();

    let (update, bytes) = match download(update, events.clone(), progress_id).await {
        Ok(downloaded) => downloaded,
        Err(err) => {
            // A card left loading can never be dismissed
            events.finish_progress(
                progress_id,
                "Update failed",
                format!("Couldn't download Reo Launcher {version}: {err}"),
            );
            return Err(err);
        }
    };

    install(update, bytes, events, progress_id).await
}

/// Download and signature check, `Update::install` is left to the caller
async fn download(
    update: Update,
    events: EventBus,
    progress_id: Uuid,
) -> anyhow::Result<(Update, Vec<u8>)> {
    let label = format!("Downloading Reo Launcher {}", update.version);

    events.progress(progress_id, &label, 0, 0);

    tokio::task::spawn_blocking(move || -> anyhow::Result<(Update, Vec<u8>)> {
        let downloaded = Cell::new(0u64);
        let last_sent = Cell::new(0u64);

        let bytes = update.download_extended(
            |chunk, total| {
                let now = downloaded.get() + chunk as u64;
                downloaded.set(now);
                let total = total.unwrap_or(0);

                if now == chunk as u64
                    || (total > 0 && now >= total)
                    || now - last_sent.get() >= PROGRESS_STEP
                {
                    last_sent.set(now);
                    events.progress(progress_id, &label, now, total);
                }
            },
            || {},
        )?;

        let total = downloaded.get().max(1);
        events.progress(progress_id, &label, total, total);

        Ok((update, bytes))
    })
    .await?
}

#[cfg(not(windows))]
async fn install(
    update: Update,
    bytes: Vec<u8>,
    events: EventBus,
    progress_id: Uuid,
) -> anyhow::Result<()> {
    let version = update.version.clone();

    let installed = tokio::task::spawn_blocking(move || update.install(bytes)).await;
    let failure = match installed {
        Ok(Ok(())) => None,
        Ok(Err(err)) => Some(anyhow::Error::from(err)),
        Err(err) => Some(anyhow::Error::from(err)),
    };
    if let Some(err) = failure {
        events.finish_progress(
            progress_id,
            "Update failed",
            format!("Couldn't install Reo Launcher {version}: {err}"),
        );
        return Err(err);
    }

    // Converts the same download card into its finished state rather than adding a second
    events.finish_progress(
        progress_id,
        "Finished Downloading",
        format!("Reo Launcher {version} is ready. Restart to apply."),
    );

    tracing::info!("update installed; restart to apply");
    Ok(())
}

/// How often an install held back by a game or a folder move looks again
#[cfg(windows)]
const BLOCKED_POLL: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(windows)]
fn restart_prompt(version: &str) -> Prompt<UpdateAnswer> {
    Prompt::new(
        "Update ready",
        format!(
            "Reo Launcher {version} is downloaded. Restart now to install it? The launcher \
             closes while the update installs."
        ),
    )
    .option(
        Choice::primary(UPDATE_CHOICE_INSTALL, "Restart and install"),
        UpdateAnswer::Install,
    )
    .dismiss("Not now")
}

/// `None` once nothing would be cut off by the installer closing the launcher
#[cfg(windows)]
fn install_blocker() -> Option<&'static str> {
    if reo_core::relocate::in_progress() {
        return Some("The data folder is still being moved.");
    }

    let playing = crate::hooks::launch_in_flight()
        || crate::launcher::state().is_ok_and(|state| !state.games.active_ids().is_empty());
    playing.then_some("Minecraft is still starting or running.")
}

/// `Update::install` ends the process here so it only runs when the user asks
/// for the restart, never by itself
#[cfg(windows)]
async fn install(
    update: Update,
    bytes: Vec<u8>,
    events: EventBus,
    progress_id: Uuid,
) -> anyhow::Result<()> {
    let version = update.version.clone();

    events.finish_progress(
        progress_id,
        "Update ready",
        format!("Reo Launcher {version} is downloaded."),
    );

    loop {
        if events.ask(restart_prompt(&version)).await?.is_none() {
            tracing::info!("user put off installing update {version}");
            return Ok(());
        }

        let Some(reason) = install_blocker() else {
            break;
        };

        events
            .notify("Update waiting")
            .body(format!("{reason} Reo Launcher asks again once that is done."))
            .send();

        while install_blocker().is_some() {
            tokio::time::sleep(BLOCKED_POLL).await;
        }
    }

    tracing::info!("closing to install update {version}");

    let state = crate::launcher::state().ok();
    let installed = tokio::task::spawn_blocking(move || {
        // The installer exits the process so this is the only shutdown it gets
        if let Some(state) = state {
            tokio::runtime::Handle::current().block_on(reo_core::shutdown(&state));
        }
        update.install(bytes)
    })
    .await;

    // Only reached when the installer never started
    let err = match installed {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(err)) => anyhow::Error::from(err),
        Err(err) => anyhow::Error::from(err),
    };
    events
        .notify("Update failed")
        .body(format!(
            "Couldn't start the installer for Reo Launcher {version}: {err}. Restart Reo \
             Launcher to keep using it."
        ))
        .error()
        .send();
    Err(err)
}
