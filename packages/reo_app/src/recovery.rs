use std::path::PathBuf;

use freya::prelude::spawn_forever;

use crate::hooks::Actions;
use crate::launcher::off_ui_blocking;

pub fn database_path() -> Option<PathBuf> {
    reo_common::paths::database_file().ok()
}

pub fn snapshots() -> Vec<PathBuf> {
    database_path()
        .map(|path| reo_db::backup::list(&path))
        .unwrap_or_default()
}

pub fn open_data_folder() {
    let Ok(dir) = reo_common::paths::data_dir() else {
        return;
    };

    crate::platform::open_path(&dir.to_string_lossy());
}

pub fn restore_latest(actions: &Actions) {
    let Some(path) = database_path() else { return };
    let Some(snapshot) = snapshots().into_iter().next() else {
        set_error(actions, "There are no snapshots to restore.".to_owned());
        return;
    };

    let actions = actions.clone();
    spawn_forever(async move {
        match off_ui_blocking(move || reo_db::backup::restore(&path, &snapshot)).await {
            Ok(_) => retry(&actions),
            Err(err) => set_error(&actions, format!("Couldn't restore the snapshot: {err}")),
        }
    });
}

pub fn reset(actions: &Actions) {
    let Some(path) = database_path() else { return };

    let actions = actions.clone();
    spawn_forever(async move {
        match off_ui_blocking(move || reo_db::backup::reset(&path)).await {
            Ok(_) => retry(&actions),
            Err(err) => set_error(&actions, format!("Couldn't reset the database: {err}")),
        }
    });
}

fn retry(actions: &Actions) {
    let station = actions.station();
    let events = actions.events();

    {
        let mut guard = station
            .clone()
            .write_channel(crate::state::AppChannel::Launcher);
        guard.launcher.error = None;
        guard.launcher.snapshots = snapshots().len();
    }

    let actions = actions.clone();
    spawn_forever(async move {
        match crate::events::start_launcher(station, events).await {
            Ok(()) => crate::events::finish_startup(&actions),
            Err(err) => crate::events::report_startup_failure(&station, &err),
        }
    });
}

fn set_error(actions: &Actions, message: String) {
    let mut guard = actions
        .station()
        .write_channel(crate::state::AppChannel::Launcher);
    guard.launcher.error = Some(message);
    guard.launcher.snapshots = snapshots().len();
}
