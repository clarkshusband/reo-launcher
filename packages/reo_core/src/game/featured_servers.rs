use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Catalog injected into the client's featured-server list.
const CATALOG: &str = include_str!("featured_servers.json");

const PROPERTY: &str = "polyplus.featuredServers.file";

/// The client only accepts `-Dpolyplus.featuredServers.file` when it believes it
/// is running in a development environment. The bundled client jar is patched to
/// take that branch (see `tools/reo-client-branding`), so the launcher can point
/// it at this catalog.
///
/// The catalog and its cache are rewritten on every launch. The dismissals file
/// is only seeded when missing, otherwise un-dismissing the main-menu campaign
/// would be undone at the next start.
#[must_use]
pub fn featured_servers_argument(game_dir: &Path) -> Option<String> {
    let dir = game_dir.join("polyplus");
    std::fs::create_dir_all(&dir).ok()?;

    let catalog_path = dir.join("featured_servers.json");
    std::fs::write(&catalog_path, CATALOG).ok()?;

    let fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    let body = serde_json::to_string(CATALOG).ok()?;
    let cache = format!(r#"{{"fetched_at":{fetched_at},"body":{body}}}"#);
    std::fs::write(dir.join("featured_servers_cache.json"), cache).ok()?;

    let dismissals = dir.join("featured_server_dismissals.json");
    if !dismissals.exists() {
        let seed = r#"{"main_menu_campaign_ids":["reomc"],"multiplayer_campaign_ids":[]}"#;
        std::fs::write(dismissals, seed).ok()?;
    }

    Some(format!("-D{PROPERTY}={}", catalog_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_valid_json() {
        let value: serde_json::Value = serde_json::from_str(CATALOG).expect("catalog parses");
        assert_eq!(
            value["servers"][0]["address"].as_str(),
            Some("mc.reomc.qzz.io")
        );
    }

    #[test]
    fn writes_catalog_and_seeds_dismissals_once() {
        let dir = std::env::temp_dir().join(format!(
            "reo-featured-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let arg = featured_servers_argument(&dir).expect("argument");
        assert!(arg.starts_with("-Dpolyplus.featuredServers.file="));
        assert!(dir.join("polyplus/featured_servers.json").exists());
        assert!(dir.join("polyplus/featured_servers_cache.json").exists());

        let dismissals = dir.join("polyplus/featured_server_dismissals.json");
        assert!(dismissals.exists());

        std::fs::write(
            &dismissals,
            r#"{"main_menu_campaign_ids":[],"multiplayer_campaign_ids":[]}"#,
        )
        .unwrap();
        featured_servers_argument(&dir).expect("argument");
        let text = std::fs::read_to_string(&dismissals).unwrap();
        assert!(text.contains(r#""main_menu_campaign_ids":[]"#));

        std::fs::remove_dir_all(&dir).ok();
    }
}
