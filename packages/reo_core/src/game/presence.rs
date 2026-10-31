use std::path::Path;

use reo_common::paths;
use reo_discord::GameSession;

use crate::clusters::Cluster;

/// What Discord shows while this cluster's game runs
pub(crate) fn discord_session(cluster: &Cluster) -> GameSession {
    let mods = cluster
        .mc_loader
        .is_modded()
        .then(|| paths::cluster_mods_dir(&cluster.folder_name).ok())
        .flatten()
        .and_then(|dir| enabled_mod_count(&dir));

    GameSession {
        cluster: cluster.name.clone(),
        mc_version: cluster.mc_version.clone(),
        loader: cluster.mc_loader,
        mods,
    }
}

/// Jars in the mods folder. A disabled mod ends in `.disabled`, so it is left out
fn enabled_mod_count(mods_dir: &Path) -> Option<usize> {
    let entries = std::fs::read_dir(mods_dir).ok()?;
    Some(
        entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_dir()))
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
            })
            .count(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_enabled_jars_count_as_mods() {
        let dir = std::env::temp_dir().join(format!("reo-presence-mods-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested.jar")).unwrap();
        for file in [
            "sodium.jar",
            "lithium.JAR",
            "iris.jar.disabled",
            "notes.txt",
        ] {
            std::fs::write(dir.join(file), b"").unwrap();
        }

        assert_eq!(enabled_mod_count(&dir), Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_folder_has_no_count_rather_than_zero() {
        let missing = std::env::temp_dir().join("reo-presence-no-such-folder");
        assert_eq!(enabled_mod_count(&missing), None);
    }
}
