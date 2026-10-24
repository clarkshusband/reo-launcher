use crate::{LauncherError, LauncherResult};
use reo_common::paths;
use reo_net::RequestClient;
use reo_net::{EtagPolicy, fetch_cached};

const CHANGELOG_URL: &str =
    "https://github.com/drako1k/reo-launcher/releases/latest/download/changelog.json";

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ChangelogEntry {
    pub version: String,
    pub body: String,
}

#[tracing::instrument(level = "debug", skip(net))]
pub async fn fetch_changelog(net: &RequestClient) -> LauncherResult<Vec<ChangelogEntry>> {
    let cache_path = paths::caches_dir()?.join("changelog.json");

    let fetched = fetch_cached(net, CHANGELOG_URL, &cache_path, EtagPolicy::CommitNow)
        .await?
        .ok_or_else(|| LauncherError::InvalidSettingsProfile {
            reason: "changelog is unavailable and not cached".to_string(),
        })?;

    Ok(fetched.json()?)
}
