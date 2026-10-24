use std::path::Path;

use parking_lot::RwLock;

use crate::{LauncherError, LauncherResult};
use reo_common::paths;
use reo_events::EventBus;

use super::launcher::LauncherSettings;
use reo_cluster::GameSettingsProfile;

#[tracing::instrument(level = "debug", skip(notify))]
pub async fn load_settings(notify: Option<&EventBus>) -> LauncherSettings {
    match async {
        let path = paths::settings_file()?;
        let exists = polyio::try_exists(&path).await?;

        Ok::<LauncherSettings, LauncherError>(if !exists {
            LauncherSettings::default()
        } else {
            let data = polyio::read(&path).await?;
            let mut settings: LauncherSettings = serde_json::from_slice(&data)?;
            settings.migrate();
            settings
        })
    }
    .await
    {
        Ok(settings) => settings,
        Err(err) => {
            tracing::warn!("failed to read settings file: {err}");

            if let Some(notify) = notify {
                notify
                    .notify("Settings")
                    .body("Failed to load settings")
                    .error()
                    .send();
            }

            recover_after_failed_load().await
        }
    }
}

async fn recover_after_failed_load() -> LauncherSettings {
    let Ok(path) = paths::settings_file() else {
        return LauncherSettings::default();
    };

    let Ok(data) = polyio::read(&path).await else {
        return LauncherSettings::default();
    };

    let value = match serde_json::from_slice::<serde_json::Value>(&data) {
        Ok(value) if value.is_object() => value,
        other => {
            match other {
                Err(err) => tracing::warn!("settings file is not valid JSON: {err}"),
                Ok(_) => tracing::warn!("settings file does not hold a settings object"),
            }

            set_aside(&path, false).await;
            return LauncherSettings::default();
        }
    };

    let (mut settings, dropped) = salvage_fields(&value);
    settings.data_dir = salvaged_data_dir(&value);
    settings.migrate();

    if !dropped.is_empty() {
        tracing::warn!(
            fields = ?dropped,
            "settings did not parse; reset only the fields that failed"
        );

        // Copied not moved so what was salvaged still loads if nothing saves first
        set_aside(&path, true).await;
    }

    if let Some(dir) = &settings.data_dir {
        tracing::warn!(
            data_dir = %dir.display(),
            "settings did not parse; kept the data folder they named"
        );
    }

    settings
}

/// Keeps the original so the next save cannot overwrite what the user had
async fn set_aside(path: &Path, keep_in_place: bool) {
    let Ok(aside) = paths::damaged_settings_file() else {
        return;
    };

    let result = if keep_in_place {
        polyio::copy(path, &aside).await.map(|_| ())
    } else {
        polyio::rename(path, &aside).await
    };

    match result {
        Ok(()) => tracing::warn!(path = %aside.display(), "kept the damaged settings file"),
        Err(err) => tracing::warn!("could not set the damaged settings file aside: {err}"),
    }
}

/// Lays each top-level field onto the defaults keeping those that still parse
fn salvage_fields(value: &serde_json::Value) -> (LauncherSettings, Vec<String>) {
    let mut dropped = Vec::new();

    let Ok(serde_json::Value::Object(mut merged)) =
        serde_json::to_value(LauncherSettings::default())
    else {
        return (LauncherSettings::default(), dropped);
    };

    for (key, field) in value.as_object().into_iter().flatten() {
        let previous = merged.insert(key.clone(), field.clone());

        let parses =
            serde_json::from_value::<LauncherSettings>(serde_json::Value::Object(merged.clone()))
                .is_ok();
        if parses {
            continue;
        }

        match previous {
            Some(previous) => {
                merged.insert(key.clone(), previous);
            }
            None => {
                merged.remove(key);
            }
        }
        dropped.push(key.clone());
    }

    let settings: LauncherSettings =
        serde_json::from_value(serde_json::Value::Object(merged)).unwrap_or_default();
    (settings, dropped)
}

fn salvaged_data_dir(value: &serde_json::Value) -> Option<std::path::PathBuf> {
    value
        .get("data_dir")
        .and_then(serde_json::Value::as_str)
        .filter(|raw| !raw.trim().is_empty())
        .map(std::path::PathBuf::from)
}

#[tracing::instrument(level = "debug", skip(settings))]
pub async fn save_settings(settings: &LauncherSettings) -> LauncherResult<()> {
    let path = paths::settings_file()?;

    let data = serde_json::to_string_pretty(settings)?;
    polyio::write_atomic(path, data).await?;
    Ok(())
}

#[tracing::instrument(level = "debug", skip(services, settings))]
pub async fn save_settings_and_apply(
    services: &crate::LauncherServices,
    settings: &LauncherSettings,
) -> LauncherResult<()> {
    save_settings(settings).await?;
    services.requester.set_config(super::net_config(settings));
    Ok(())
}

#[tracing::instrument(level = "debug", skip_all)]
pub async fn save_global_profile(
    settings: &RwLock<LauncherSettings>,
    global: GameSettingsProfile,
) -> LauncherResult<()> {
    {
        let mut lock = settings.write();
        lock.global_game_settings = global;
    }

    let snapshot = settings.read().clone();
    save_settings(&snapshot).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn salvage(raw: &str) -> Option<std::path::PathBuf> {
        salvaged_data_dir(&serde_json::from_str(raw).expect("valid json"))
    }

    #[test]
    fn one_bad_field_does_not_cost_the_user_their_library() {
        let raw = r#"{"max_concurrent_requests":"25","data_dir":"D:\\ReoLauncher"}"#;

        assert_eq!(
            salvage(raw),
            Some(std::path::PathBuf::from("D:\\ReoLauncher")),
            "the location has to survive whatever else is wrong with the file"
        );
    }

    #[test]
    fn one_bad_field_resets_only_itself() {
        let raw = r#"{"max_concurrent_requests":"25","seen_onboarding":true,"accepted_tos_version":3}"#;
        let (settings, dropped) =
            salvage_fields(&serde_json::from_str(raw).expect("valid json"));

        assert_eq!(dropped, vec!["max_concurrent_requests".to_owned()]);
        assert_eq!(settings.max_concurrent_requests, 25);
        assert!(settings.seen_onboarding);
        assert_eq!(settings.accepted_tos_version, 3);
    }

    #[test]
    fn a_sound_file_drops_nothing() {
        let raw = r#"{"seen_onboarding":true,"some_old_field":1}"#;
        let (settings, dropped) =
            salvage_fields(&serde_json::from_str(raw).expect("valid json"));

        assert!(dropped.is_empty());
        assert!(settings.seen_onboarding);
    }

    #[test]
    fn the_default_location_stays_the_default() {
        assert_eq!(salvage(r#"{"log_debug":true}"#), None);
        assert_eq!(salvage(r#"{"data_dir":null}"#), None);
        assert_eq!(salvage(r#"{"data_dir":"   "}"#), None);
        assert_eq!(salvage(r#"{"data_dir":42}"#), None);
    }
}
