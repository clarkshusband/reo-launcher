use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use futures_util::{StreamExt, stream};
use interfrost::api::minecraft::{
    Argument, ArgumentType, Asset, AssetsIndex, DownloadType, Library, LibraryDownload,
    LogConfigDownload, LoggingConfiguration, LoggingSide, Os, Version, VersionInfo,
};
use interfrost::api::modded::LoaderVersion;
use reqwest::Method;

use crate::download::{download_to_path, fetch_bytes_verified};
use crate::rules::validate_rules;

use crate::McCtx;
use crate::error::{McError, McResult};
use crate::manifest::{MetadataStore, entry_matches_version, manifest_supports_version};
use reo_common::domain::GameLoader;
use reo_common::os_ext::OsExt;
use reo_common::paths;
use reo_events::{Choice, GroupedProgressSession, Prompt, TaskCategory, TaskPhase};

/// Asset objects are tiny (median ~10 KiB) and latency-bound so throughput
/// scales with how many are in flight not with bandwidth
const ASSET_DOWNLOAD_CONCURRENCY: usize = 32;
/// Libraries are larger and fewer less fan-out is needed to saturate the link
const LIBRARY_DOWNLOAD_CONCURRENCY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativesMode {
    Extract,
    LeaveInPlace,
}

#[derive(Debug, Clone)]
pub struct PlannedLibrary {
    pub library: Library,
    pub artifact: bool,
    pub natives: bool,
}

/// Computed up-front so the pre-download size estimate and the progress-bar
/// denominators come from the same numbers
#[derive(Debug, Default)]
pub struct DownloadPlan {
    pub assets: Vec<(String, Asset)>,
    pub asset_bytes: u64,
    pub libraries: Vec<PlannedLibrary>,
    pub library_bytes: u64,
    pub client: bool,
    pub client_bytes: u64,
}

impl DownloadPlan {
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.asset_bytes + self.library_bytes + self.client_bytes
    }
}

#[must_use]
pub fn uses_legacy_assets(assets: &str) -> bool {
    matches!(assets, "legacy" | "pre-1.6")
}

fn asset_object_path(dir: &Path, name: &str, hash: &str, legacy: bool) -> McResult<PathBuf> {
    if legacy {
        return Ok(dir.join(name.replace('/', std::path::MAIN_SEPARATOR_STR)));
    }

    Ok(dir.join(asset_subhash(name, hash)?).join(hash))
}

fn asset_subhash<'a>(name: &str, hash: &'a str) -> McResult<&'a str> {
    hash.get(..2)
        .ok_or_else(|| McError::Minecraft(format!("asset {name} has a malformed hash '{hash}'")))
}

#[derive(serde::Deserialize)]
struct ResourcesIndex {
    #[serde(default)]
    map_to_resources: bool,
    objects: HashMap<String, Asset>,
}

/// Pre-1.6 clients read their sounds from `<gameDir>/resources` rather than an
/// assets directory
pub(crate) fn map_assets_to_resources(asset_index: &str, game_dir: &Path) -> McResult<()> {
    if !uses_legacy_assets(asset_index) {
        return Ok(());
    }

    let path = paths::assets_index_dir()?.join(format!("{asset_index}.json"));
    let index: ResourcesIndex = serde_json::from_slice(&std::fs::read(&path)?)?;
    if !index.map_to_resources && asset_index != "pre-1.6" {
        return Ok(());
    }

    link_resources(
        index.objects,
        &paths::legacy_assets_dir()?,
        &game_dir.join("resources"),
    )
}

/// Hard links where it can a copy where the game directory is on another volume
fn link_resources(objects: HashMap<String, Asset>, source: &Path, target: &Path) -> McResult<()> {
    for (name, asset) in objects {
        let from = asset_object_path(source, &name, &asset.hash, true)?;
        let to = asset_object_path(target, &name, &asset.hash, true)?;
        // a missing source failed its download and was reported there
        if !from.is_file() || matches_expected_size(&to, u64::from(asset.size)) {
            continue;
        }

        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&to);
        if std::fs::hard_link(&from, &to).is_err() {
            std::fs::copy(&from, &to)?;
        }
    }

    Ok(())
}

fn client_log_config(version: &VersionInfo) -> Option<(&str, &LogConfigDownload)> {
    match version.logging.as_ref()?.get(&LoggingSide::Client)? {
        LoggingConfiguration::Log4j2Xml { argument, file } => Some((argument.as_str(), file)),
    }
}

/// Mojang's two client configs as `(id, sha1, size)`, one per log4j generation
const LOG4J_BETA9_CONFIG: (&str, &str, u32) = (
    "client-1.7.xml",
    "50c9cc4af6d853d9fc137c84bcd153e2bd3a9a82",
    966,
);
const LOG4J_CONFIG: (&str, &str, u32) = (
    "client-1.12.xml",
    "bd65e7d2e3c237be76cfbef4c2405033d7f91521",
    888,
);

/// The metadata mirror leaves `logging` out so the config Mojang's own version
/// file names is put back going by the bundled log4j the way Mojang picks it
/// 2.0-beta9 (1.7 to 1.11) takes client-1.7.xml and anything newer client-1.12.xml
/// Forge and NeoForge on ModLauncher bring their own config and opt out
fn fill_missing_log_config(info: &mut VersionInfo, game_loader: GameLoader) {
    if client_log_config(info).is_some() {
        return;
    }
    if matches!(game_loader, GameLoader::Forge | GameLoader::NeoForge)
        && !info.main_class.contains("launchwrapper")
    {
        return;
    }

    let Some(log4j) = info
        .libraries
        .iter()
        .find_map(|lib| lib.name.strip_prefix("org.apache.logging.log4j:log4j-core:"))
    else {
        return;
    };
    let (id, sha1, size) = if log4j.starts_with("2.0-beta9") {
        LOG4J_BETA9_CONFIG
    } else {
        LOG4J_CONFIG
    };

    info.logging.get_or_insert_with(HashMap::new).insert(
        LoggingSide::Client,
        LoggingConfiguration::Log4j2Xml {
            argument: "-Dlog4j.configurationFile=${path}".to_string(),
            file: LogConfigDownload {
                id: id.to_string(),
                sha1: sha1.to_string(),
                size,
                url: format!("https://piston-data.mojang.com/v1/objects/{sha1}/{id}"),
            },
        },
    );
}

/// Mojang's config as downloaded and the copy the game is pointed at
fn log_config_paths(file: &LogConfigDownload) -> McResult<(PathBuf, PathBuf)> {
    let dir = paths::assets_dir()?.join("log_configs");
    let name = polyio::sanitize_path(&file.id);
    Ok((dir.join(&name), dir.join("plain").join(&name)))
}

/// Mojang's configs log XML events to stdout for the vanilla launcher's viewer
/// but stdout is read here as plain lines so the console takes the pattern the
/// file appender already uses which keeps the lookup mitigation
fn plain_console_config(xml: &str) -> Option<String> {
    let layout = xml
        .find("<LegacyXMLLayout")
        .or_else(|| xml.find("<XMLLayout"))?;
    let layout_end = layout + xml[layout..].find("/>")? + "/>".len();
    let pattern = xml
        .split("<PatternLayout pattern=\"")
        .nth(1)?
        .split('"')
        .next()?;

    Some(format!(
        "{}<PatternLayout pattern=\"{pattern}\" />{}",
        &xml[..layout],
        &xml[layout_end..]
    ))
}

fn write_plain_log_config(original: &Path, plain: &Path) -> McResult<()> {
    let xml = std::fs::read_to_string(original)?;
    let Some(converted) = plain_console_config(&xml) else {
        // an XML console would garble the log so no config beats this one
        tracing::warn!(path = %original.display(), "log4j config has no layout to convert");
        let _ = std::fs::remove_file(plain);
        return Ok(());
    };

    if let Some(parent) = plain.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(plain, converted)?;
    Ok(())
}

/// Folded into the JVM arguments the launch already builds from rather than
/// persisted since the path is this machine's
/// `java_arguments` drops it again while the file is missing
fn add_log_config_argument(info: &mut VersionInfo) -> McResult<()> {
    let Some((argument, file)) = client_log_config(info) else {
        return Ok(());
    };
    let (_, plain) = log_config_paths(file)?;
    let argument = argument.replace("${path}", &plain.display().to_string());

    info.arguments
        .get_or_insert_with(HashMap::new)
        .entry(ArgumentType::Jvm)
        .or_default()
        .push(Argument::Normal(argument));

    Ok(())
}

/// A missing config only costs the launch its log4j hardening so the caller
/// does not fail the install over it
#[tracing::instrument(skip_all, level = "debug")]
async fn download_log_config(
    ctx: &McCtx,
    progress: &GroupedProgressSession,
    version: &VersionInfo,
    force: bool,
) -> McResult<()> {
    let Some((_, file)) = client_log_config(version) else {
        return Ok(());
    };

    let (path, plain) = log_config_paths(file)?;
    let stale = force || !matches_expected_size(&path, u64::from(file.size));
    if stale {
        download_to_path(
            &ctx.net,
            &ctx.events,
            progress,
            format!("Log config ({})", file.id),
            TaskCategory::Metadata,
            u64::from(file.size),
            &file.url,
            &path,
            Some(file.sha1.as_str()).filter(|sha1| !sha1.is_empty()),
        )
        .await?;
    }

    if stale || !plain.is_file() {
        write_plain_log_config(&path, &plain)?;
    }

    Ok(())
}

/// Size not hash catches interrupted-download truncation without reading
/// ~5000 assets per launch
/// `expected` of 0 means the manifest gave no size
fn matches_expected_size(path: &Path, expected: u64) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => expected == 0 || meta.len() == expected,
        Err(_) => false,
    }
}

/// 0 when the entry carries no artifact metadata (maven-coordinate libraries
/// from loader manifests)
fn library_artifact_size(lib: &Library) -> u64 {
    lib.downloads
        .as_ref()
        .and_then(|downloads| downloads.artifact.as_ref())
        .map_or(0, |artifact| u64::from(artifact.size))
}

pub(crate) fn has_main_artifact(lib: &Library) -> bool {
    lib.downloads
        .as_ref()
        .is_none_or(|downloads| downloads.artifact.is_some())
}

fn native_download<'a>(lib: &'a Library, java_arch: &str) -> Option<&'a LibraryDownload> {
    let (os_key, classifiers) = lib.natives.as_ref().and_then(|natives| {
        Some((
            natives.get(&Os::native_arch(java_arch))?,
            lib.downloads.as_ref()?.classifiers.as_ref()?,
        ))
    })?;

    classifiers.get(&os_key.replace("${arch}", natives_arch_width(java_arch)))
}

/// `${arch}` in a natives classifier is the JVM's data model not the launcher's
fn natives_arch_width(java_arch: &str) -> &'static str {
    match java_arch {
        "x86" | "i386" | "i486" | "i586" | "i686" | "arm" | "arm32" | "aarch32" | "armv7"
        | "armv7l" => "32",
        "amd64" | "x86_64" | "x86-64" | "x64" | "aarch64" | "arm64" => "64",
        _ => reo_common::constants::ARCH_WIDTH,
    }
}

fn native_download_size(lib: &Library, java_arch: &str) -> u64 {
    native_download(lib, java_arch).map_or(0, |native| u64::from(native.size))
}

const NATIVES_RECEIPTS: &str = ".extracted";

fn natives_receipt(natives_dir: &Path, lib: &Library) -> PathBuf {
    natives_dir
        .join(NATIVES_RECEIPTS)
        .join(polyio::sanitize_path(lib.name.replace(':', "_")))
}

fn natives_extracted(natives_dir: &Path, lib: &Library, java_arch: &str) -> bool {
    let Some(native) = native_download(lib, java_arch) else {
        return true;
    };

    match std::fs::read_to_string(natives_receipt(natives_dir, lib)) {
        Ok(recorded) => native.sha1.is_empty() || recorded.trim() == native.sha1,
        Err(_) => false,
    }
}

fn natives_classifier(lib: &Library) -> Option<&str> {
    lib.name
        .split(':')
        .nth(3)
        .filter(|classifier| classifier.starts_with("natives-"))
}

/// Pre-LWJGL3 macOS natives jars ship `.jnilib` so a single suffix would read
/// an intact extraction as empty and repair it on every check
const NATIVE_SUFFIXES: &[&str] = if cfg!(windows) {
    &[".dll"]
} else if cfg!(target_os = "macos") {
    &[".dylib", ".jnilib"]
} else {
    &[".so"]
};

fn is_native_file(name: &str) -> bool {
    NATIVE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

fn carries_natives(lib: &Library, java_arch: &str) -> bool {
    native_download(lib, java_arch).is_some() || natives_classifier(lib).is_some()
}

async fn record_natives(natives_dir: &Path, lib: &Library, sha1: &str) -> McResult<()> {
    let path = natives_receipt(natives_dir, lib);
    if let Some(parent) = path.parent() {
        polyio::create_dir_all(parent).await?;
    }
    polyio::write(&path, sha1.as_bytes()).await?;
    Ok(())
}

/// Stats thousands of files so it runs on the blocking pool and keeps the
/// download fan-outs from stat-ing inside a concurrency slot
#[tracing::instrument(skip_all, level = "debug")]
pub async fn plan_downloads(
    version: &VersionInfo,
    assets_index: AssetsIndex,
    java_arch: &str,
    minecraft_updated: bool,
    force: bool,
    natives_mode: NativesMode,
) -> McResult<DownloadPlan> {
    let legacy = uses_legacy_assets(&version.assets);
    let asset_dir = if legacy {
        paths::legacy_assets_dir()?
    } else {
        paths::assets_object_dir()?
    };
    let lib_dir = paths::libraries_dir()?;

    let client = version.downloads.get(&DownloadType::Client).map(|client| {
        (
            paths::versions_dir()
                .map(|dir| dir.join(&version.id).join(format!("{}.jar", version.id))),
            u64::from(client.size),
        )
    });

    let libraries = version.libraries.clone();
    let java_arch = java_arch.to_string();
    let natives_dir = paths::natives_dir()?.join(&version.id);

    tokio::task::spawn_blocking(move || {
        let mut plan = DownloadPlan::default();

        for (name, asset) in assets_index.objects {
            // an asset with no usable path stays in the plan so its download reports it
            let present = asset_object_path(&asset_dir, &name, &asset.hash, legacy)
                .is_ok_and(|path| matches_expected_size(&path, u64::from(asset.size)));
            if !force && present {
                continue;
            }
            plan.asset_bytes += u64::from(asset.size);
            plan.assets.push((name, asset));
        }

        for lib in libraries {
            if let Some(rules) = &lib.rules
                && !validate_rules(rules, &java_arch, minecraft_updated)
            {
                continue;
            }
            if !lib.downloadable {
                continue;
            }

            // A library whose coordinates don't resolve is kept in the plan so the
            // download path still reports it as a failure rather than skipping it
            let Ok(artifact_path) = interfrost::utils::get_path_from_artifact(&lib.name) else {
                plan.libraries.push(PlannedLibrary {
                    library: lib,
                    artifact: true,
                    natives: true,
                });
                continue;
            };

            let artifact = has_main_artifact(&lib)
                && (force
                    || !matches_expected_size(
                        &lib_dir.join(&artifact_path),
                        library_artifact_size(&lib),
                    ));
            let natives = match natives_mode {
                NativesMode::Extract => {
                    force || !natives_extracted(&natives_dir, &lib, &java_arch)
                }
                NativesMode::LeaveInPlace => false,
            };

            if !artifact && !natives {
                continue;
            }

            if artifact {
                plan.library_bytes += library_artifact_size(&lib);
            }
            if natives {
                plan.library_bytes += native_download_size(&lib, &java_arch);
            }
            plan.libraries.push(PlannedLibrary {
                library: lib,
                artifact,
                natives,
            });
        }

        if let Some((path, size)) = client {
            let present = path
                .map(|path| matches_expected_size(&path, size))
                .unwrap_or(false);
            if force || !present {
                plan.client = true;
                plan.client_bytes = size;
            }
        }

        plan
    })
    .await
    .map_err(std::io::Error::other)
    .map_err(Into::into)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VerifyReport {
    pub checked: usize,
    /// Mismatched the manifest and were removed
    pub corrupt: usize,
    pub missing: usize,
    pub natives_unextracted: usize,
}

impl VerifyReport {
    #[must_use]
    pub fn needs_repair(&self) -> bool {
        self.corrupt > 0 || self.missing > 0
    }
}

const ARCH_SEGMENTS: [&str; 4] = ["x64", "arm64", "x86", "arm32"];

fn arch_segment(java_arch: &str) -> &'static str {
    match java_arch {
        "aarch64" => "arm64",
        "arm" => "arm32",
        "x86" => "x86",
        _ => "x64",
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NativesReport {
    pub repairable: Vec<String>,
    pub unsupported: Vec<String>,
    pub unextracted: Vec<String>,
}

impl NativesReport {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.repairable.is_empty() && self.unsupported.is_empty()
    }
}

enum JarVerdict {
    Usable,
    WrongArch,
    Unreadable,
}

async fn inspect_jar(path: &Path, java_arch: &str) -> JarVerdict {
    let names = match polyio::zip_entry_names(path).await {
        Ok(names) => names,
        Err(err) => {
            tracing::debug!(path = %path.display(), "could not read natives jar: {err}");
            return JarVerdict::Unreadable;
        }
    };

    let natives: Vec<&String> = names.iter().filter(|name| is_native_file(name)).collect();

    if natives.is_empty() {
        return JarVerdict::WrongArch;
    }

    let wanted = arch_segment(java_arch);
    if natives
        .iter()
        .any(|name| name.split('/').any(|part| part == wanted))
    {
        return JarVerdict::Usable;
    }

    if natives
        .iter()
        .all(|name| !name.split('/').any(|part| ARCH_SEGMENTS.contains(&part)))
    {
        return JarVerdict::Usable;
    }

    JarVerdict::WrongArch
}

fn natives_family(lib: &Library) -> String {
    let parts: Vec<&str> = lib.name.split(':').collect();
    match parts.as_slice() {
        [group, artifact, ..] => format!("{group}:{artifact}"),
        _ => lib.name.clone(),
    }
}

async fn any_native_unpacked(natives_dir: &Path) -> bool {
    let Ok(mut entries) = polyio::read_dir(natives_dir).await else {
        return false;
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        if is_native_file(&entry.file_name().to_string_lossy()) {
            return true;
        }
    }

    false
}

#[tracing::instrument(skip(version), fields(version_id = %version.id), level = "debug")]
pub async fn check_natives(
    version: &VersionInfo,
    java_arch: &str,
    minecraft_updated: bool,
) -> McResult<NativesReport> {
    let natives_dir = paths::natives_dir()?.join(&version.id);
    let lib_dir = paths::libraries_dir()?;

    let host_natives = natives_for_host(version, java_arch, minecraft_updated)?;

    // One scan for the whole sweep the directory is the same for every library
    let unpacked = any_native_unpacked(&natives_dir).await;

    let mut families: Vec<(String, Vec<(JarVerdict, bool)>)> = Vec::new();
    for lib in host_natives {
        let extracted = native_download(lib, java_arch).is_some();
        let verdict = if extracted {
            if unpacked && natives_extracted(&natives_dir, lib, java_arch) {
                JarVerdict::Usable
            } else {
                JarVerdict::Unreadable
            }
        } else {
            match interfrost::utils::get_path_from_artifact(&lib.name) {
                Ok(rel) => inspect_jar(&lib_dir.join(&rel), java_arch).await,
                Err(_) => continue,
            }
        };

        let family = natives_family(lib);
        match families.iter_mut().find(|(name, _)| name == &family) {
            Some((_, verdicts)) => verdicts.push((verdict, extracted)),
            None => families.push((family, vec![(verdict, extracted)])),
        }
    }

    let mut report = NativesReport::default();
    for (family, verdicts) in families {
        if verdicts.iter().any(|(v, _)| matches!(v, JarVerdict::Usable)) {
            continue;
        }
        if verdicts.iter().any(|(v, _)| matches!(v, JarVerdict::Unreadable)) {
            if verdicts
                .iter()
                .any(|(v, extracted)| *extracted && matches!(v, JarVerdict::Unreadable))
            {
                report.unextracted.push(family.clone());
            }
            report.repairable.push(family);
        } else {
            report.unsupported.push(family);
        }
    }

    if !report.repairable.is_empty() {
        tracing::warn!(libraries = ?report.repairable, "natives are missing or unreadable");
    }
    if !report.unsupported.is_empty() {
        tracing::error!(
            libraries = ?report.unsupported,
            platform = %format!("{}-{java_arch}", reo_common::constants::TARGET_OS),
            "natives are installed but hold nothing loadable on this platform"
        );
    }

    Ok(report)
}

/// Expensive reads every asset and library so it belongs behind an explicit
/// user action not the launch path
/// Nothing is re-downloaded here corrupt files are deleted so the next prepare
/// refetches them
#[tracing::instrument(skip(progress, version, assets_index), fields(version_id = %version.id), level = "debug")]
pub async fn verify_game_files(
    progress: &GroupedProgressSession,
    version: &VersionInfo,
    assets_index: AssetsIndex,
    java_arch: &str,
    minecraft_updated: bool,
) -> McResult<VerifyReport> {
    let legacy = uses_legacy_assets(&version.assets);
    let asset_dir = if legacy {
        paths::legacy_assets_dir()?
    } else {
        paths::assets_object_dir()?
    };
    let lib_dir = paths::libraries_dir()?;

    let mut targets: Vec<(PathBuf, String)> = Vec::new();

    for (name, asset) in assets_index.objects {
        // the download path reports an asset it cannot place
        let Ok(path) = asset_object_path(&asset_dir, &name, &asset.hash, legacy) else {
            continue;
        };
        targets.push((path, asset.hash));
    }

    for lib in &version.libraries {
        if let Some(rules) = &lib.rules
            && !validate_rules(rules, java_arch, minecraft_updated)
        {
            continue;
        }
        if !lib.downloadable {
            continue;
        }
        let Ok(artifact_path) = interfrost::utils::get_path_from_artifact(&lib.name) else {
            continue;
        };
        // Only libraries whose manifest entry carries a hash can be checked
        // maven-coordinate entries from loader manifests publish none
        if let Some(artifact) = lib
            .downloads
            .as_ref()
            .and_then(|downloads| downloads.artifact.as_ref())
            && !artifact.sha1.is_empty()
        {
            targets.push((lib_dir.join(&artifact_path), artifact.sha1.clone()));
        }
    }

    // An empty hash would read every jar as corrupt and delete it
    if let Some(client) = version.downloads.get(&DownloadType::Client)
        && !client.sha1.is_empty()
    {
        let path = paths::versions_dir()?
            .join(&version.id)
            .join(format!("{}.jar", version.id));
        targets.push((path, client.sha1.clone()));
    }

    let total = targets.len() as u64;
    let child = progress.child("Verifying game files", total.max(1), TaskCategory::Assets);
    child.set_phase(TaskPhase::Verifying);

    let started = std::time::Instant::now();
    let child_for_progress = child.clone();

    // One blocking task for the whole sweep a spawn_blocking per small asset
    // would cost more than the hashing itself
    let mut report = tokio::task::spawn_blocking(move || {
        let mut report = VerifyReport::default();

        for (index, (path, expected)) in targets.iter().enumerate() {
            if !path.is_file() {
                report.missing += 1;
                continue;
            }

            match polyio::sha1_file_sync(path) {
                Ok(actual)
                    if polyio::normalize_hash(&actual) == polyio::normalize_hash(expected) =>
                {
                    report.checked += 1;
                }
                Ok(actual) => {
                    tracing::warn!(
                        path = %path.display(),
                        %actual,
                        %expected,
                        "corrupt game file; removing so it is re-downloaded"
                    );
                    report.checked += 1;
                    report.corrupt += 1;
                    if let Err(err) = std::fs::remove_file(path) {
                        tracing::error!(path = %path.display(), "could not remove: {err}");
                    }
                }
                Err(err) => {
                    // Unreadable is as good as corrupt removing it lets the
                    // download path replace it
                    tracing::warn!(path = %path.display(), "could not hash: {err}");
                    report.corrupt += 1;
                    let _ = std::fs::remove_file(path);
                }
            }

            // The child's sampling is per-call so throttle here or this emits
            // thousands of events
            if index % 64 == 0 {
                child_for_progress.set_progress(index as u64, Some(total.max(1)));
            }
        }

        report
    })
    .await
    .map_err(std::io::Error::other)?;

    child.finish();

    // A version that ships nothing for this platform is a fact about the version
    // not a verification failure so the rest of the sweep still counts
    let natives = match check_natives(version, java_arch, minecraft_updated).await {
        Ok(natives) => natives,
        Err(err @ McError::NoNativesForPlatform { .. }) => {
            tracing::error!("{err}");
            NativesReport::default()
        }
        Err(err) => return Err(err),
    };
    report.missing += natives.repairable.len();
    report.natives_unextracted = natives.unextracted.len();
    report.checked += natives.repairable.len() + natives.unsupported.len();

    tracing::info!(
        checked = report.checked,
        corrupt = report.corrupt,
        missing = report.missing,
        natives_repairable = natives.repairable.len(),
        natives_unsupported = natives.unsupported.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "verified game files"
    );

    Ok(report)
}

#[tracing::instrument(skip_all)]
pub async fn download_minecraft(
    ctx: &McCtx,
    progress: &GroupedProgressSession,
    version: &VersionInfo,
    java_arch: &str,
    minecraft_updated: bool,
    force: bool,
    natives_mode: NativesMode,
) -> McResult<()> {
    let started = std::time::Instant::now();
    let asset_index = download_assets_index(ctx, progress, version, force).await?;
    if let Err(err) = download_log_config(ctx, progress, version, force).await {
        tracing::warn!(version = %version.id, "could not download the log4j config: {err}");
    }
    let plan = plan_downloads(
        version,
        asset_index,
        java_arch,
        minecraft_updated,
        force,
        natives_mode,
    )
    .await?;

    tracing::info!(
        version = %version.id,
        assets = plan.assets.len(),
        libraries = plan.libraries.len(),
        bytes = plan.total_bytes(),
        ?natives_mode,
        "planned minecraft download"
    );

    // Reserve each category's real total before any child appears so the bars
    // start at the right denominator instead of growing towards it
    progress.expect(
        TaskCategory::Assets,
        plan.assets.len() as u64,
        plan.asset_bytes,
    );
    progress.expect(
        TaskCategory::Libraries,
        plan.libraries.len() as u64,
        plan.library_bytes,
    );
    progress.expect(
        TaskCategory::Client,
        u64::from(plan.client),
        plan.client_bytes,
    );

    let DownloadPlan {
        assets, libraries, ..
    } = plan;

    let (failed_assets, _client, failed_libraries) = tokio::try_join!(
        download_assets(ctx, progress, uses_legacy_assets(&version.assets), assets,),
        download_client(ctx, progress, version, force),
        download_libraries(ctx, progress, version.id.clone(), libraries, java_arch,),
    )?;

    confirm_incomplete_install(ctx, failed_assets, failed_libraries).await?;

    tracing::info!(
        version = %version.id,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "minecraft download complete"
    );

    Ok(())
}

enum IncompleteInstall {
    LaunchAnyway,
}

/// Download failures are counted not propagated so one unreachable file does
/// not abandon the other four thousand
/// They already survived every retry but it stays the user's call refusing to
/// launch would strand offline players
#[tracing::instrument(skip(ctx), level = "debug")]
pub async fn confirm_incomplete_install(
    ctx: &McCtx,
    failed_assets: usize,
    failed_libraries: usize,
) -> McResult<()> {
    let failed = failed_assets + failed_libraries;
    if failed == 0 {
        return Ok(());
    }

    tracing::warn!(
        failed_assets,
        failed_libraries,
        "some files could not be downloaded; asking the user how to proceed"
    );

    let body = match (failed_assets, failed_libraries) {
        (0, libraries) => format!(
            "{libraries} librar{} could not be downloaded after several attempts. \
             The game will most likely fail to start without {}.",
            if libraries == 1 { "y" } else { "ies" },
            if libraries == 1 { "it" } else { "them" },
        ),
        (assets, 0) => format!(
            "{assets} game asset{} could not be downloaded after several attempts. \
             Sounds or textures may be missing.",
            if assets == 1 { "" } else { "s" },
        ),
        (assets, libraries) => format!(
            "{assets} asset(s) and {libraries} librar{} could not be downloaded after \
             several attempts. The game may fail to start or be missing content.",
            if libraries == 1 { "y" } else { "ies" },
        ),
    };

    let answer = ctx
        .events
        .ask(
            Prompt::new("Some files could not be downloaded", body)
                .option(
                    Choice::new("launch", "Launch anyway"),
                    IncompleteInstall::LaunchAnyway,
                )
                .dismiss("Cancel"),
        )
        .await;

    match answer {
        Ok(Some(chosen)) => match chosen.value {
            IncompleteInstall::LaunchAnyway => {
                tracing::warn!(failed, "user chose to launch with files missing");
                Ok(())
            }
        },
        // Dismissed or undeliverable nobody agreed to launch a broken install
        // and a headless caller must not silently opt in
        Ok(None) => Err(McError::IncompleteInstallCancelled { failed }),
        Err(err) => {
            tracing::warn!("could not ask about the incomplete install: {err}");
            Err(McError::IncompleteInstallCancelled { failed })
        }
    }
}

static VERSION_INFO_REFRESHED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

#[tracing::instrument(skip(ctx, progress), level = "debug")]
pub async fn download_version_info(
    ctx: &McCtx,
    progress: Option<&GroupedProgressSession>,
    version: &Version,
    game_loader: GameLoader,
    loader: Option<&LoaderVersion>,
    force: bool,
) -> McResult<VersionInfo> {
    let version_id = loader
        .map(|it| format!("{}-{}", version.id, it.id))
        .unwrap_or_else(|| version.id.clone());

    let dir = paths::versions_dir()?.join(&version_id);
    let format_version = game_loader.get_format_version();
    let stem = match loader {
        Some(_) => format!("{version_id}.{}", game_loader.get_format_name()),
        None => version_id.clone(),
    };
    let path = dir.join(version_info_file_name(&stem, format_version));

    let cached = if force {
        None
    } else {
        let mut cached = read_cached_version_info(&path).await;
        if cached.is_none() && loader.is_some() {
            let legacy = version_info_file_name(&version_id, format_version);
            cached = read_cached_version_info(&dir.join(legacy))
                .await
                .filter(|info| profile_matches_loader(&info.libraries, game_loader));
        }
        cached
    };

    if let Some(mut cached) = cached {
        if loader.is_some()
            && reo_net::status::current().online
            && VERSION_INFO_REFRESHED.lock().unwrap().insert(path.clone())
        {
            let ctx = ctx.clone();
            let version = version.clone();
            let loader = loader.cloned();
            let installed = processor_inputs(&cached);
            tokio::spawn(async move {
                let refresh = async {
                    let info =
                        fetch_version_info(&ctx, None, &version, loader.as_ref(), &version_id)
                            .await?;
                    if processor_inputs(&info) != installed {
                        tracing::warn!(
                            version_id = %version_id,
                            "loader processors changed upstream, keeping cached profile until reinstall"
                        );
                        return Ok(());
                    }
                    polyio::write_json_atomic(&path, &info).await?;
                    McResult::Ok(())
                };
                if let Err(err) = refresh.await {
                    tracing::warn!(
                        version_id = %version_id,
                        "could not refresh version metadata, keeping cached copy: {err}"
                    );
                }
            });
        }
        fill_missing_log_config(&mut cached, game_loader);
        add_log_config_argument(&mut cached)?;
        return Ok(cached);
    }

    let mut info = fetch_version_info(ctx, progress, version, loader, &version_id).await?;
    polyio::write_json_atomic(&path, &info).await?;
    if loader.is_some() {
        VERSION_INFO_REFRESHED.lock().unwrap().insert(path);
    }
    fill_missing_log_config(&mut info, game_loader);
    add_log_config_argument(&mut info)?;
    Ok(info)
}

async fn fetch_version_info(
    ctx: &McCtx,
    progress: Option<&GroupedProgressSession>,
    version: &Version,
    loader: Option<&LoaderVersion>,
    version_id: &str,
) -> McResult<VersionInfo> {
    tracing::debug!(
        version_id = %version_id,
        "downloading Minecraft version metadata"
    );

    let version_url = version.url.parse().map_err(McError::Url)?;
    let requester = ctx.net.clone();
    let mut info: VersionInfo = match progress {
        Some(progress) => {
            progress
                .run_child(
                    format!("Version metadata ({version_id})"),
                    1,
                    TaskCategory::Metadata,
                    |child| {
                        let requester = requester.clone();
                        async move {
                            child.set_progress(0, Some(1));
                            let result = requester
                                .send_json(Method::GET, version_url, None, &[])
                                .await
                                .map_err(McError::from)?;
                            child.set_progress(1, Some(1));
                            Ok::<VersionInfo, McError>(result)
                        }
                    },
                )
                .await?
        }
        None => requester
            .send_json(Method::GET, version_url, None, &[])
            .await
            .map_err(McError::from)?,
    };

    if let Some(loader) = loader {
        let loader_url = loader.url.parse().map_err(McError::Url)?;
        let requester = ctx.net.clone();
        let partial: interfrost::api::modded::PartialVersionInfo = match progress {
            Some(progress) => {
                progress
                    .run_child(
                        format!("Loader metadata ({version_id})"),
                        1,
                        TaskCategory::Metadata,
                        |child| {
                            let requester = requester.clone();
                            async move {
                                child.set_progress(0, Some(1));
                                let result = requester
                                    .send_json(Method::GET, loader_url, None, &[])
                                    .await
                                    .map_err(McError::from)?;
                                child.set_progress(1, Some(1));
                                Ok::<interfrost::api::modded::PartialVersionInfo, McError>(result)
                            }
                        },
                    )
                    .await?
            }
            None => requester
                .send_json(Method::GET, loader_url, None, &[])
                .await
                .map_err(McError::from)?,
        };

        let legacy_args = info.minecraft_arguments.clone();
        info = interfrost::api::modded::merge_partial_version(partial, info);
        if info.minecraft_arguments.is_none() {
            info.minecraft_arguments = legacy_args;
        }

        for lib in &mut info.libraries {
            lib.name = lib.name.replace("${interpulse.gameVersion}", &version.id);
        }
    }

    info.id = version_id.to_owned();

    Ok(info)
}

#[tracing::instrument(skip_all, level = "debug")]
pub async fn download_assets_index(
    ctx: &McCtx,
    progress: &GroupedProgressSession,
    version: &VersionInfo,
    force: bool,
) -> McResult<AssetsIndex> {
    let path = paths::assets_index_dir()?.join(format!("{}.json", version.asset_index.id));

    if path.exists() && !force {
        if let Ok(data) = polyio::read_json::<AssetsIndex>(&path).await {
            return Ok(data);
        }
        tracing::warn!("cached assets index is invalid, redownloading");
    }

    download_to_path(
        &ctx.net,
        &ctx.events,
        progress,
        format!("Assets index ({})", version.asset_index.id),
        TaskCategory::Assets,
        u64::from(version.asset_index.size),
        &version.asset_index.url,
        &path,
        Some(version.asset_index.sha1.as_str()),
    )
    .await?;

    polyio::read_json(&path).await.map_err(Into::into)
}

#[tracing::instrument(skip_all, level = "debug")]
pub async fn download_assets(
    ctx: &McCtx,
    progress: &GroupedProgressSession,
    legacy: bool,
    assets: Vec<(String, Asset)>,
) -> McResult<usize> {
    if assets.is_empty() {
        return Ok(0);
    }

    let dir = if legacy {
        paths::legacy_assets_dir()?
    } else {
        paths::assets_object_dir()?
    };

    polyio::create_dir_all(&dir).await?;

    // Objects are sharded into 256 fixed subdirectories creating them once
    // here saves a blocking-pool round trip on every one of the ~5000 files
    if !legacy {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || {
            for subhash in 0u16..256 {
                let _ = std::fs::create_dir_all(dir.join(format!("{subhash:02x}")));
            }
        })
        .await
        .map_err(std::io::Error::other)?;
    }

    let started = std::time::Instant::now();
    let count = assets.len();
    let requester = ctx.net.clone();
    let events = ctx.events.clone();
    let progress = progress.clone();
    let requests = stream::iter(assets.into_iter().map(|(name, asset)| {
        let dir = dir.clone();
        let requester = requester.clone();
        let events = events.clone();
        let progress = progress.clone();

        async move {
            let hash = &asset.hash;
            let subhash = asset_subhash(&name, hash)?;
            let path = asset_object_path(&dir, &name, hash, legacy)?;

            let url = format!("https://resources.download.minecraft.net/{subhash}/{hash}");
            download_to_path(
                &requester,
                &events,
                &progress,
                format!("Asset {name}"),
                TaskCategory::Assets,
                u64::from(asset.size),
                &url,
                &path,
                Some(hash),
            )
            .await
        }
    }))
    .buffer_unordered(ASSET_DOWNLOAD_CONCURRENCY)
    .collect::<Vec<_>>();

    let mut failed = 0;
    for res in requests.await {
        if let Err(err) = res {
            tracing::error!("failed to download asset: {err:?}");
            failed += 1;
        }
    }

    tracing::info!(
        count,
        failed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "assets ready"
    );

    Ok(failed)
}

#[tracing::instrument(skip(ctx, progress, version), fields(version_id = %version.id, force), level = "debug")]
pub async fn download_client(
    ctx: &McCtx,
    progress: &GroupedProgressSession,
    version: &VersionInfo,
    force: bool,
) -> McResult<PathBuf> {
    let client = version
        .downloads
        .get(&DownloadType::Client)
        .ok_or_else(|| McError::NoClientDownload(version.id.clone()))?;

    let path = paths::versions_dir()?
        .join(&version.id)
        .join(format!("{}.jar", version.id));

    // Size not hash the jar is only published by rename after its hash
    // matched so a wrong-but-right-length jar is unreachable
    let expected_size = u64::from(client.size);
    if !force
        && let Ok(meta) = polyio::stat(&path).await
        && (expected_size == 0 || meta.len() == expected_size)
    {
        return Ok(path);
    }

    if path.exists() {
        tracing::warn!("client jar is the wrong size, redownloading");
    }

    download_to_path(
        &ctx.net,
        &ctx.events,
        progress,
        format!("Client {}", version.id),
        TaskCategory::Client,
        u64::from(client.size),
        &client.url,
        &path,
        Some(client.sha1.as_str()).filter(|sha1| !sha1.is_empty()),
    )
    .await?;
    Ok(path)
}

fn natives_for_host<'a>(
    version_info: &'a VersionInfo,
    java_arch: &str,
    minecraft_updated: bool,
) -> Result<Vec<&'a Library>, McError> {
    let mut selected = Vec::new();
    let mut any_declared = false;

    for lib in &version_info.libraries {
        if natives_classifier(lib).is_some() || lib.natives.is_some() {
            any_declared = true;
        }

        if let Some(rules) = &lib.rules
            && !validate_rules(rules, java_arch, minecraft_updated)
        {
            continue;
        }
        if !lib.downloadable {
            continue;
        }
        if carries_natives(lib, java_arch) {
            selected.push(lib);
        }
    }

    if any_declared && selected.is_empty() {
        return Err(McError::NoNativesForPlatform {
            version: version_info.id.clone(),
            platform: format!("{}-{java_arch}", reo_common::constants::TARGET_OS),
        });
    }

    Ok(selected)
}

#[tracing::instrument(skip(version_info), level = "debug")]
pub fn natives_missing(
    version_info: &VersionInfo,
    java_arch: &str,
    minecraft_updated: bool,
) -> McResult<bool> {
    let host_natives = natives_for_host(version_info, java_arch, minecraft_updated)?;
    let natives_dir = paths::natives_dir()?.join(&version_info.id);

    if !natives_dir.is_dir() {
        tracing::warn!(path = %natives_dir.display(), "natives directory is missing; will repair");
        return Ok(true);
    }

    let lib_dir = paths::libraries_dir()?;

    for lib in host_natives {
        if native_download(lib, java_arch).is_some() {
            if !natives_extracted(&natives_dir, lib, java_arch) {
                tracing::warn!(library = %lib.name, "natives were never extracted; will repair");
                return Ok(true);
            }
            continue;
        }

        if !lib.include_in_classpath {
            tracing::error!(
                library = %lib.name,
                "a natives jar was dropped from the classpath; the game will not find it"
            );
        }

        if !has_main_artifact(lib) {
            continue;
        }
        let Ok(rel) = interfrost::utils::get_path_from_artifact(&lib.name) else {
            continue;
        };
        if !matches_expected_size(&lib_dir.join(&rel), library_artifact_size(lib)) {
            tracing::warn!(library = %lib.name, "missing or truncated natives jar; will repair");
            return Ok(true);
        }
    }

    Ok(false)
}

#[tracing::instrument(skip(version_info), level = "debug")]
pub fn libraries_missing(
    version_info: &VersionInfo,
    java_arch: &str,
    minecraft_updated: bool,
) -> McResult<bool> {
    let lib_dir = paths::libraries_dir()?;
    for lib in &version_info.libraries {
        if let Some(rules) = &lib.rules
            && !validate_rules(rules, java_arch, minecraft_updated)
        {
            continue;
        }
        if !lib.include_in_classpath {
            continue;
        }
        if !has_main_artifact(lib) {
            continue;
        }
        let Ok(rel) = interfrost::utils::get_path_from_artifact(&lib.name) else {
            continue;
        };
        if !matches_expected_size(&lib_dir.join(&rel), library_artifact_size(lib)) {
            tracing::warn!(library = %lib.name, "missing or truncated classpath library; will repair");
            return Ok(true);
        }
    }

    natives_missing(version_info, java_arch, minecraft_updated)
}

/// Takes its paths rather than resolving them so it can be tested without the
/// process-global launcher directory
fn assets_tree_missing(index: &Path, objects: &Path) -> bool {
    if !index.is_file() {
        tracing::warn!(path = %index.display(), "assets index is missing; will repair");
        return true;
    }

    if !objects.is_dir() {
        tracing::warn!(path = %objects.display(), "assets directory is missing; will repair");
        return true;
    }

    false
}

/// A `Ready` cluster skips preparation so nothing else re-checks its files
/// this turns a deleted assets tree into an automatic repair
/// Coarse checks only per-object verification belongs to `download_minecraft`'s plan
#[tracing::instrument(skip(version_info), level = "debug")]
pub fn game_files_missing(
    version_info: &VersionInfo,
    java_arch: &str,
    minecraft_updated: bool,
) -> McResult<bool> {
    let index = paths::assets_index_dir()?.join(format!("{}.json", version_info.asset_index.id));
    let objects = if uses_legacy_assets(&version_info.assets) {
        paths::legacy_assets_dir()?
    } else {
        paths::assets_object_dir()?
    };

    if assets_tree_missing(&index, &objects) {
        return Ok(true);
    }

    // installs from before the config was fetched pick it up here offline the
    // repair could not reach it and would only delay the launch
    if let Some((_, file)) = client_log_config(version_info)
        && reo_net::status::current().online
        && !matches_expected_size(&log_config_paths(file)?.0, u64::from(file.size))
    {
        tracing::warn!(id = %file.id, "log4j config is missing; will repair");
        return Ok(true);
    }

    if let Some(client) = version_info.downloads.get(&DownloadType::Client) {
        let jar = paths::versions_dir()?
            .join(&version_info.id)
            .join(format!("{}.jar", version_info.id));
        if !matches_expected_size(&jar, u64::from(client.size)) {
            tracing::warn!(path = %jar.display(), "client jar is missing or truncated; will repair");
            return Ok(true);
        }
    }

    libraries_missing(version_info, java_arch, minecraft_updated)
}

fn lib_short(name: &str) -> String {
    let parts: Vec<&str> = name.split(':').collect();
    match parts.as_slice() {
        [_group, artifact, version, ..] => format!("{artifact} {version}"),
        _ => name.to_string(),
    }
}

#[tracing::instrument(skip(ctx, progress, libraries), level = "debug")]
pub async fn download_libraries(
    ctx: &McCtx,
    progress: &GroupedProgressSession,
    version: String,
    libraries: Vec<PlannedLibrary>,
    java_arch: &str,
) -> McResult<usize> {
    let lib_dir = paths::libraries_dir()?;
    let natives_dest = paths::natives_dir()?.join(&version);
    let java_arch = java_arch.to_string();

    polyio::create_dir_all(&natives_dest).await?;

    if libraries.is_empty() {
        return Ok(0);
    }

    polyio::create_dir_all(&lib_dir).await?;

    let started = std::time::Instant::now();
    let count = libraries.len();
    let requests = stream::iter(libraries.into_iter().map(|planned| {
        let PlannedLibrary {
            library: lib,
            artifact: wants_artifact,
            natives: wants_natives,
        } = planned;
        let lib_dir = lib_dir.clone();
        let natives_dest = natives_dest.clone();
        let java_arch = java_arch.clone();
        let requester = ctx.net.clone();
        let events = ctx.events.clone();
        let progress = progress.clone();

        async move {
            let artifact_path = interfrost::utils::get_path_from_artifact(&lib.name)
                .map_err(|_| McError::LibraryPath(lib.name.clone()))?;
            let path = lib_dir.join(&artifact_path);

            tokio::try_join!(
                async {
                    if !wants_artifact {
                        return Ok::<_, McError>(());
                    }

                    if let Some(interfrost::api::minecraft::LibraryDownloads {
                        artifact: Some(ref artifact),
                        ..
                    }) = lib.downloads
                        && !artifact.url.is_empty()
                    {
                        download_to_path(
                            &requester,
                            &events,
                            &progress,
                            format!("Library {}", lib_short(&lib.name)),
                            TaskCategory::Libraries,
                            u64::from(artifact.size),
                            &artifact.url,
                            &path,
                            Some(artifact.sha1.as_str()).filter(|sha1| !sha1.is_empty()),
                        )
                        .await?;
                        return Ok::<_, McError>(());
                    }

                    let url = [
                        lib.url
                            .as_deref()
                            .unwrap_or("https://libraries.minecraft.net/"),
                        &artifact_path,
                    ]
                    .concat();
                    download_to_path(
                        &requester,
                        &events,
                        &progress,
                        format!("Library {}", lib_short(&lib.name)),
                        TaskCategory::Libraries,
                        0,
                        &url,
                        &path,
                        None,
                    )
                    .await?;
                    Ok(())
                },
                async {
                    if !wants_natives {
                        return Ok(());
                    }

                    if let Some(native) = native_download(&lib, &java_arch) {
                        let data = fetch_bytes_verified(
                            &requester,
                            &events,
                            &progress,
                            format!("Natives {}", lib_short(&lib.name)),
                            TaskCategory::Natives,
                            u64::from(native.size),
                            &native.url,
                            Some(native.sha1.as_str()).filter(|sha1| !sha1.is_empty()),
                        )
                        .await?;

                        let extract = progress.child(
                            format!("Natives {}", lib_short(&lib.name)),
                            1,
                            TaskCategory::Natives,
                        );
                        extract.set_phase(reo_events::TaskPhase::Extracting);
                        polyio::unzip_bytes_filtered(
                            data,
                            Some(|name: &str| !name.starts_with("META-INF")),
                            &natives_dest,
                        )
                        .await?;
                        record_natives(&natives_dest, &lib, &native.sha1).await?;
                        extract.finish();
                    }

                    Ok(())
                }
            )?;

            Ok::<(), McError>(())
        }
    }))
    .buffer_unordered(LIBRARY_DOWNLOAD_CONCURRENCY)
    .collect::<Vec<_>>();

    let mut failed = 0;
    for res in requests.await {
        if let Err(err) = res {
            tracing::error!("failed to download library: {err:?}");
            failed += 1;
        }
    }

    tracing::info!(
        count,
        failed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "libraries ready"
    );

    Ok(failed)
}

#[tracing::instrument(skip(metadata, ctx), level = "debug")]
pub async fn get_loader_versions(
    metadata: &mut MetadataStore,
    ctx: &McCtx,
    mc_version: &str,
    loader: GameLoader,
) -> McResult<Vec<String>> {
    if loader == GameLoader::Vanilla {
        return Ok(Vec::new());
    }

    let manifest = metadata.get_modded_or_fetch(ctx, loader).await?;
    if !manifest_supports_version(manifest, mc_version) {
        return Ok(Vec::new());
    }

    for entry in &manifest.game_versions {
        if entry_matches_version(&entry.id, mc_version) && !entry.loaders.is_empty() {
            return Ok(entry.loaders.iter().map(|l| l.id.clone()).collect());
        }
    }
    Ok(Vec::new())
}

fn resolve_loader_from_manifest(
    manifest: &interfrost::api::modded::Manifest,
    mc_version: &str,
    loader_version: Option<&str>,
) -> (bool, Option<LoaderVersion>) {
    if !manifest_supports_version(manifest, mc_version) {
        return (false, None);
    }

    let mut saw_matching_game_version = false;

    for entry in &manifest.game_versions {
        if !entry_matches_version(&entry.id, mc_version) {
            continue;
        }

        saw_matching_game_version = true;

        if let Some(requested) = loader_version {
            if let Some(found) = entry
                .loaders
                .iter()
                .find(|loader_entry| loader_entry.id == requested)
            {
                return (saw_matching_game_version, Some(found.clone()));
            }
            continue;
        }

        if let Some(found) = entry
            .loaders
            .iter()
            .find(|l| l.stable)
            .or_else(|| entry.loaders.first())
        {
            return (saw_matching_game_version, Some(found.clone()));
        }
    }

    (saw_matching_game_version, None)
}

#[tracing::instrument(skip(metadata), level = "debug")]
pub fn get_loader_version_cached(
    metadata: &MetadataStore,
    mc_version: &str,
    loader: GameLoader,
    loader_version: Option<&str>,
) -> McResult<Option<LoaderVersion>> {
    if loader == GameLoader::Vanilla {
        return Ok(None);
    }

    let manifest = metadata.get_modded(loader)?;
    Ok(resolve_loader_from_manifest(manifest, mc_version, loader_version).1)
}

#[tracing::instrument(skip(metadata, ctx), level = "debug")]
pub async fn get_loader_version(
    metadata: &mut MetadataStore,
    ctx: &McCtx,
    mc_version: &str,
    loader: GameLoader,
    loader_version: Option<&str>,
) -> McResult<Option<LoaderVersion>> {
    if loader == GameLoader::Vanilla {
        return Ok(None);
    }

    let resolve_from_manifest = |manifest: &interfrost::api::modded::Manifest| {
        resolve_loader_from_manifest(manifest, mc_version, loader_version)
    };

    let mut manifest = metadata.get_modded_or_fetch(ctx, loader).await?;
    let (mut saw_matching, mut resolved) = resolve_from_manifest(manifest);
    if resolved.is_some() {
        return Ok(resolved);
    }

    if !saw_matching || loader_version.is_some() {
        metadata.fetch_all(ctx).await;
        manifest = metadata.get_modded(loader)?;
        (saw_matching, resolved) = resolve_from_manifest(manifest);
        if resolved.is_some() {
            return Ok(resolved);
        }
    }

    let no_matching_version = || McError::NoMatchingVersion {
        loader,
        version: mc_version.to_string(),
    };

    if let Some(requested) = loader_version {
        if !saw_matching {
            return Err(no_matching_version());
        }
        return Err(McError::RequestedLoaderVersionNotFound {
            requested: requested.to_string(),
        });
    }

    if saw_matching {
        Err(McError::NoMatchingLoader)
    } else {
        Err(no_matching_version())
    }
}

#[tracing::instrument(skip(metadata, ctx), level = "debug")]
pub async fn resolve_minecraft_version(
    metadata: &mut MetadataStore,
    ctx: &McCtx,
    mc_version: &str,
) -> McResult<(Version, usize, bool)> {
    let mut manifest = metadata.get_vanilla_or_fetch(ctx).await?;
    let mut version_index = manifest.versions.iter().position(|it| it.id == mc_version);

    if version_index.is_none() {
        metadata.fetch_all(ctx).await;
        manifest = metadata.get_vanilla()?;
        version_index = manifest.versions.iter().position(|it| it.id == mc_version);
    }

    let version_index =
        version_index.ok_or_else(|| McError::InvalidVersion(mc_version.to_string()))?;
    let versions = &manifest.versions;

    Ok((
        versions[version_index].clone(),
        version_index,
        is_version_updated(version_index, versions),
    ))
}

#[tracing::instrument(skip(metadata, ctx), level = "debug")]
pub async fn get_game_versions(
    metadata: &mut MetadataStore,
    ctx: &McCtx,
) -> McResult<Vec<Version>> {
    let manifest = metadata.get_vanilla_or_fetch(ctx).await?;
    Ok(manifest.versions.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameVersionKind {
    Release,
    Snapshot,
    Beta,
    Alpha,
}

impl GameVersionKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Release => "Release",
            Self::Snapshot => "Snapshot",
            Self::Beta => "Beta",
            Self::Alpha => "Alpha",
        }
    }

    #[must_use]
    pub const fn is_release(self) -> bool {
        matches!(self, Self::Release)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameVersionInfo {
    pub id: String,
    pub kind: GameVersionKind,
    pub released: chrono::DateTime<chrono::Utc>,
}

#[tracing::instrument(skip(metadata, ctx), level = "debug")]
pub async fn get_version_ids(
    metadata: &mut MetadataStore,
    ctx: &McCtx,
) -> McResult<Vec<GameVersionInfo>> {
    use interfrost::api::minecraft::VersionType;

    Ok(get_game_versions(metadata, ctx)
        .await?
        .into_iter()
        .map(|version| GameVersionInfo {
            id: version.id,
            kind: match version.type_ {
                VersionType::Release => GameVersionKind::Release,
                VersionType::Snapshot => GameVersionKind::Snapshot,
                VersionType::OldBeta => GameVersionKind::Beta,
                VersionType::OldAlpha => GameVersionKind::Alpha,
            },
            released: version.release_time,
        })
        .collect())
}

#[tracing::instrument(skip(metadata, ctx), level = "debug")]
pub async fn get_loaders_for_version(
    metadata: &mut MetadataStore,
    ctx: &McCtx,
    mc_version: &str,
) -> McResult<Vec<GameLoader>> {
    metadata.get_loaders_for_version(ctx, mc_version).await
}

#[tracing::instrument(skip(metadata, ctx), level = "debug")]
pub async fn get_versions_for_loader(
    metadata: &mut MetadataStore,
    ctx: &McCtx,
    loader: GameLoader,
) -> McResult<Option<Vec<String>>> {
    metadata.get_versions_for_loader(ctx, loader).await
}

#[must_use]
pub fn is_version_updated(version_index: usize, versions: &[Version]) -> bool {
    version_index <= versions.iter().position(|x| x.id == "22w16a").unwrap_or(0)
}

async fn read_cached_version_info(path: &Path) -> Option<VersionInfo> {
    if !path.exists() {
        return None;
    }

    polyio::read_json::<VersionInfo>(path)
        .await
        .inspect_err(|err| {
            tracing::warn!(
                path = %path.display(),
                "cached version metadata is unusable, redownloading: {err}"
            );
        })
        .ok()
}

fn processor_inputs(info: &VersionInfo) -> Option<serde_json::Value> {
    let processors = info.processors.as_deref().unwrap_or_default();
    let refs = processors
        .iter()
        .flat_map(|it| &it.args)
        .chain(info.data.iter().flatten().flat_map(|(_, it)| [&it.client, &it.server]))
        .filter_map(|it| it.strip_prefix('[')?.strip_suffix(']'));
    let named: BTreeSet<&str> = processors
        .iter()
        .flat_map(|it| std::iter::once(&it.jar).chain(&it.classpath))
        .map(String::as_str)
        .chain(refs)
        .collect();
    let libraries: Vec<&Library> = info
        .libraries
        .iter()
        .filter(|lib| named.contains(lib.name.as_str()))
        .collect();
    serde_json::to_value((&info.processors, &info.data, libraries)).ok()
}

fn profile_matches_loader(libraries: &[Library], loader: GameLoader) -> bool {
    let has = |group: &str| libraries.iter().any(|lib| lib.name.starts_with(group));
    match loader {
        GameLoader::Forge => !has("net.neoforged"),
        GameLoader::NeoForge => has("net.neoforged"),
        GameLoader::Fabric => !has("org.quiltmc") && !has("net.ornithemc"),
        GameLoader::Quilt => has("org.quiltmc") && !has("net.ornithemc"),
        GameLoader::Ornithe => has("net.ornithemc"),
        GameLoader::Vanilla => true,
    }
}

fn version_info_file_name(stem: &str, format_version: usize) -> String {
    if format_version == 0 {
        format!("{stem}.json")
    } else {
        format!("{stem}.v{format_version}.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_bump_moves_the_cached_version_info() {
        assert_eq!(version_info_file_name("1.8.9", 0), "1.8.9.json");
        assert_eq!(
            version_info_file_name("1.8.9-0.19.5", 1),
            "1.8.9-0.19.5.v1.json"
        );
    }

    #[test]
    fn a_shared_legacy_profile_only_stands_in_for_its_own_loader() {
        let libs = |names: &[&str]| -> Vec<Library> {
            names
                .iter()
                .map(|name| serde_json::from_value(serde_json::json!({ "name": name })).unwrap())
                .collect()
        };
        let forge = libs(&["net.minecraftforge:forge:1.20.1-47.1.106"]);
        let neo = libs(&[
            "net.minecraftforge:eventbus:6.0.5",
            "net.neoforged:forge:1.20.1-47.1.106",
        ]);
        let fabric = libs(&["net.fabricmc:fabric-loader:0.16.0"]);
        let quilt = libs(&[
            "net.fabricmc:intermediary:1.20.1",
            "org.quiltmc:quilt-loader:0.26.0",
        ]);

        assert!(profile_matches_loader(&forge, GameLoader::Forge));
        assert!(!profile_matches_loader(&forge, GameLoader::NeoForge));
        assert!(profile_matches_loader(&neo, GameLoader::NeoForge));
        assert!(!profile_matches_loader(&neo, GameLoader::Forge));
        assert!(profile_matches_loader(&fabric, GameLoader::Fabric));
        assert!(!profile_matches_loader(&fabric, GameLoader::Quilt));
        assert!(profile_matches_loader(&quilt, GameLoader::Quilt));
        assert!(!profile_matches_loader(&quilt, GameLoader::Fabric));

        let ornithe = libs(&[
            "net.fabricmc:fabric-loader:0.16.5",
            "net.ornithemc:calamus-intermediary:1.14.4",
        ]);
        assert!(profile_matches_loader(&ornithe, GameLoader::Ornithe));
        assert!(!profile_matches_loader(&ornithe, GameLoader::Fabric));
        assert!(!profile_matches_loader(&fabric, GameLoader::Ornithe));
    }

    #[test]
    fn a_changed_processor_library_counts_as_changed_input() {
        let info = |installer: &str, lwjgl: &str| -> VersionInfo {
            serde_json::from_value(serde_json::json!({
                "assetIndex": { "id": "1", "sha1": "", "size": 0, "totalSize": 0, "url": "" },
                "assets": "1",
                "downloads": {},
                "id": "1.20.1-47.1.106",
                "libraries": [
                    { "name": "net.minecraftforge:installertools:1.3.0", "url": installer },
                    { "name": "de.oceanlabs.mcp:mcp_config:1.20.1@zip", "url": installer },
                    { "name": "org.lwjgl:lwjgl:3.3.1", "url": lwjgl },
                ],
                "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
                "minimumLauncherVersion": 0,
                "releaseTime": "2023-06-12T00:00:00Z",
                "time": "2023-06-12T00:00:00Z",
                "type": "release",
                "data": { "MCP": { "client": "[de.oceanlabs.mcp:mcp_config:1.20.1@zip]", "server": "" } },
                "processors": [
                    { "jar": "net.minecraftforge:installertools:1.3.0", "classpath": [], "args": ["{MCP}"] },
                ],
            }))
            .unwrap()
        };

        let installed = processor_inputs(&info("a", "a"));
        assert_eq!(processor_inputs(&info("a", "b")), installed);
        assert_ne!(processor_inputs(&info("b", "a")), installed);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("oneclient-install-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn install_assets(dir: &Path) -> (PathBuf, PathBuf) {
        let indexes = dir.join("assets").join("indexes");
        let objects = dir.join("assets").join("objects");
        std::fs::create_dir_all(&indexes).unwrap();
        std::fs::create_dir_all(&objects).unwrap();

        let index = indexes.join("17.json");
        std::fs::write(&index, b"{}").unwrap();

        (index, objects)
    }

    #[test]
    fn a_natives_only_library_has_no_main_jar_to_fetch() {
        let natives_only = serde_json::from_str::<Library>(
            r#"{
                "name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4+legacyfabric.15",
                "downloads": { "classifiers": { "natives-osx": { "sha1": "a", "size": 1, "url": "https://maven.legacyfabric.net/natives-osx.jar" } } },
                "natives": { "osx-arm64": "natives-osx" }
            }"#,
        )
        .unwrap();
        let with_artifact = serde_json::from_str::<Library>(
            r#"{
                "name": "org.ow2.asm:asm:9.10.1",
                "downloads": { "artifact": { "sha1": "b", "size": 2, "url": "https://maven.fabricmc.net/asm.jar" } }
            }"#,
        )
        .unwrap();
        let coordinates_only =
            serde_json::from_str::<Library>(r#"{ "name": "net.fabricmc:fabric-loader:0.19.5" }"#)
                .unwrap();

        assert!(!has_main_artifact(&natives_only));
        assert!(has_main_artifact(&with_artifact));
        assert!(has_main_artifact(&coordinates_only));
    }

    #[test]
    fn old_versions_keep_their_assets_in_a_flat_tree() {
        assert!(uses_legacy_assets("pre-1.6"));
        assert!(uses_legacy_assets("legacy"));
        assert!(!uses_legacy_assets("1.8"));
        assert!(!uses_legacy_assets("1.14"));
    }

    #[test]
    fn a_manifest_without_a_wildcard_resolves_per_game_version() {
        let manifest = serde_json::from_str::<interfrost::api::modded::Manifest>(
            r#"{"gameVersions": [
                { "id": "b1.7.3", "stable": true, "loaders": [
                    { "id": "0.19.5", "url": "https://meta.example/b1.7.3/0.19.5.json", "stable": true },
                    { "id": "0.19.4", "url": "https://meta.example/b1.7.3/0.19.4.json", "stable": false }
                ]},
                { "id": "1.8.9", "stable": true, "loaders": [
                    { "id": "0.19.5", "url": "https://meta.example/1.8.9/0.19.5.json", "stable": true }
                ]}
            ]}"#,
        )
        .unwrap();

        let (saw, resolved) = resolve_loader_from_manifest(&manifest, "b1.7.3", None);
        assert!(saw);
        assert_eq!(resolved.unwrap().id, "0.19.5");

        let (saw, resolved) = resolve_loader_from_manifest(&manifest, "b1.7.3", Some("0.19.4"));
        assert!(saw);
        assert_eq!(resolved.unwrap().id, "0.19.4");

        let (saw, resolved) = resolve_loader_from_manifest(&manifest, "1.21", None);
        assert!(!saw && resolved.is_none());
    }

    #[test]
    fn a_wildcard_only_supplies_loaders_for_versions_the_manifest_lists() {
        let manifest = serde_json::from_str::<interfrost::api::modded::Manifest>(
            r#"{"gameVersions": [
                { "id": "${interfrost.gameVersion}", "stable": true, "loaders": [
                    { "id": "0.19.5", "url": "https://meta.example/0.19.5.json", "stable": true },
                    { "id": "0.19.4", "url": "https://meta.example/0.19.4.json", "stable": false }
                ]},
                { "id": "1.21.1", "stable": true, "loaders": [] },
                { "id": "1.14", "stable": true, "loaders": [] }
            ]}"#,
        )
        .unwrap();

        let (saw, resolved) = resolve_loader_from_manifest(&manifest, "1.21.1", None);
        assert!(saw);
        assert_eq!(resolved.unwrap().id, "0.19.5");

        let (saw, resolved) = resolve_loader_from_manifest(&manifest, "1.14", Some("0.19.4"));
        assert!(saw);
        assert_eq!(resolved.unwrap().id, "0.19.4");

        let (saw, resolved) = resolve_loader_from_manifest(&manifest, "1.8.9", None);
        assert!(
            !saw && resolved.is_none(),
            "fabric publishes no intermediary below 1.14 so the wildcard must not claim 1.8.9"
        );
    }

    #[test]
    fn a_manifest_that_is_only_a_wildcard_still_covers_every_version() {
        let manifest = serde_json::from_str::<interfrost::api::modded::Manifest>(
            r#"{"gameVersions": [
                { "id": "${interfrost.gameVersion}", "stable": true, "loaders": [
                    { "id": "0.19.5", "url": "https://meta.example/0.19.5.json", "stable": true }
                ]}
            ]}"#,
        )
        .unwrap();

        let (saw, resolved) = resolve_loader_from_manifest(&manifest, "1.8.9", None);
        assert!(saw);
        assert_eq!(resolved.unwrap().id, "0.19.5");
    }

    #[test]
    fn a_complete_assets_tree_needs_no_repair() {
        let dir = scratch("assets-ok");
        let (index, objects) = install_assets(&dir);

        assert!(!assets_tree_missing(&index, &objects));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_deleted_objects_directory_asks_for_a_repair() {
        let dir = scratch("assets-no-objects");
        let (index, objects) = install_assets(&dir);
        std::fs::remove_dir_all(&objects).unwrap();

        assert!(assets_tree_missing(&index, &objects));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_deleted_assets_index_asks_for_a_repair() {
        let dir = scratch("assets-no-index");
        let (index, objects) = install_assets(&dir);
        std::fs::remove_file(&index).unwrap();

        assert!(assets_tree_missing(&index, &objects));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_index_that_is_a_directory_is_not_mistaken_for_a_file() {
        // The launcher has produced this shape before so the check must be
        // `is_file` not `exists`
        let dir = scratch("assets-index-dir");
        let (index, objects) = install_assets(&dir);
        std::fs::remove_file(&index).unwrap();
        std::fs::create_dir(&index).unwrap();

        assert!(assets_tree_missing(&index, &objects));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_check_accepts_a_complete_file() {
        let dir = scratch("size-ok");
        let file = dir.join("object.bin");
        std::fs::write(&file, b"0123456789").unwrap();

        assert!(matches_expected_size(&file, 10));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_check_rejects_a_truncated_file() {
        let dir = scratch("size-short");
        let file = dir.join("object.bin");
        std::fs::write(&file, b"0123").unwrap();

        assert!(!matches_expected_size(&file, 10));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_check_rejects_a_missing_file() {
        let dir = scratch("size-missing");

        assert!(!matches_expected_size(&dir.join("nope.bin"), 10));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The bus is drained by the test so `ask` has somebody to talk to no
    /// request is ever sent through the client
    fn test_ctx() -> (
        McCtx,
        tokio::sync::mpsc::UnboundedReceiver<reo_events::Event>,
    ) {
        let (events, rx) = reo_events::EventBus::channel();
        let net = reo_net::RequestClient::new(reo_net::NetConfig::default())
            .expect("a client");
        (McCtx::new(net, events), rx)
    }

    #[tokio::test]
    async fn a_complete_install_asks_nothing() {
        let (ctx, mut rx) = test_ctx();

        confirm_incomplete_install(&ctx, 0, 0)
            .await
            .expect("nothing failed, so nothing to confirm");

        assert!(
            rx.try_recv().is_err(),
            "a clean install must not interrupt the user"
        );
    }

    #[tokio::test]
    async fn launching_anyway_is_allowed() {
        let (ctx, mut rx) = test_ctx();

        let asking = tokio::spawn(async move { confirm_incomplete_install(&ctx, 3, 0).await });

        let Some(reo_events::Event::Notification(reo_events::Notification::Prompt(
            request,
        ))) = rx.recv().await
        else {
            panic!("expected a prompt");
        };
        assert!(request.body.contains("3 game assets"), "{}", request.body);
        request
            .reply
            .send(Some(reo_events::Answer::new("launch")))
            .unwrap();

        asking.await.unwrap().expect("the user opted in");
    }

    #[tokio::test]
    async fn dismissing_the_prompt_cancels_the_launch() {
        let (ctx, mut rx) = test_ctx();

        let asking = tokio::spawn(async move { confirm_incomplete_install(&ctx, 0, 2).await });

        let Some(reo_events::Event::Notification(reo_events::Notification::Prompt(
            request,
        ))) = rx.recv().await
        else {
            panic!("expected a prompt");
        };
        assert!(request.body.contains("fail to start"), "{}", request.body);
        request.reply.send(None).unwrap();

        let err = asking.await.unwrap().expect_err("dismissal must cancel");
        assert!(
            matches!(err, McError::IncompleteInstallCancelled { failed: 2 }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn a_bus_with_nobody_listening_cancels_rather_than_launching() {
        let (ctx, rx) = test_ctx();
        drop(rx);

        let err = confirm_incomplete_install(&ctx, 1, 1)
            .await
            .expect_err("an unanswerable prompt must not launch the game");
        assert!(
            matches!(err, McError::IncompleteInstallCancelled { failed: 2 }),
            "{err:?}"
        );
    }

    #[test]
    fn a_malformed_asset_hash_is_an_error_not_a_panic() {
        let objects = Path::new("objects");

        assert!(asset_object_path(objects, "icon.png", "a", false).is_err());
        assert!(asset_object_path(objects, "icon.png", "", false).is_err());
        assert_eq!(
            asset_object_path(objects, "icon.png", "ab12", false).unwrap(),
            objects.join("ab").join("ab12")
        );
    }

    #[test]
    fn a_natives_classifier_follows_the_jvm_data_model() {
        let download = |size: u32| serde_json::json!({ "sha1": "", "size": size, "url": "" });
        let lib = serde_json::from_value::<Library>(serde_json::json!({
            "name": "tv.twitch:twitch-platform:5.16",
            "natives": {
                "windows": "natives-windows-${arch}",
                "linux": "natives-linux-${arch}",
                "osx": "natives-osx-${arch}"
            },
            "downloads": { "classifiers": {
                "natives-windows-32": download(32), "natives-windows-64": download(64),
                "natives-linux-32": download(32), "natives-linux-64": download(64),
                "natives-osx-32": download(32), "natives-osx-64": download(64)
            }}
        }))
        .unwrap();

        assert_eq!(native_download(&lib, "x86").map(|n| n.size), Some(32));
        assert_eq!(native_download(&lib, "amd64").map(|n| n.size), Some(64));
    }

    #[test]
    fn pre_1_6_assets_are_mapped_into_the_game_directory() {
        let dir = scratch("resources");
        let source = dir.join("metadata").join("resources");
        let target = dir.join("game").join("resources");
        std::fs::create_dir_all(source.join("sound3")).unwrap();
        std::fs::write(source.join("sound3").join("cave1.ogg"), b"ogg!").unwrap();

        let objects = || -> HashMap<String, Asset> {
            serde_json::from_value(serde_json::json!({
                "sound3/cave1.ogg": { "hash": "aa11", "size": 4 },
                "music/never_downloaded.ogg": { "hash": "bb22", "size": 9 }
            }))
            .unwrap()
        };

        link_resources(objects(), &source, &target).unwrap();
        link_resources(objects(), &source, &target).expect("a second launch finds them in place");

        assert_eq!(
            std::fs::read(target.join("sound3").join("cave1.ogg")).unwrap(),
            b"ogg!"
        );
        assert!(!target.join("music").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_log4j_config_survives_a_loader_merge_and_reaches_the_jvm() {
        let vanilla: VersionInfo = serde_json::from_value(serde_json::json!({
            "assetIndex": { "id": "1.12", "sha1": "", "size": 0, "totalSize": 0, "url": "" },
            "assets": "1.12",
            "downloads": {},
            "id": "1.12.2",
            "libraries": [],
            "logging": { "client": {
                "argument": "-Dlog4j.configurationFile=${path}",
                "file": {
                    "id": "client-1.12.xml",
                    "sha1": "bd65e7d2e3c237be76cfbef4c2405033d7f91521",
                    "size": 888,
                    "url": "https://piston-data.mojang.com/v1/objects/bd65e7d2e3c237be76cfbef4c2405033d7f91521/client-1.12.xml"
                },
                "type": "log4j2-xml"
            }},
            "mainClass": "net.minecraft.client.main.Main",
            "minecraftArguments": "--username ${auth_player_name}",
            "minimumLauncherVersion": 0,
            "releaseTime": "2017-09-18T08:39:46+00:00",
            "time": "2017-09-18T08:39:46+00:00",
            "type": "release"
        }))
        .unwrap();
        let loader: interfrost::api::modded::PartialVersionInfo =
            serde_json::from_value(serde_json::json!({
                "id": "1.12.2-forge-14.23.5.2860",
                "inheritsFrom": "1.12.2",
                "releaseTime": "2017-09-18T08:39:46+00:00",
                "time": "2017-09-18T08:39:46+00:00",
                "libraries": [],
                "type": "release"
            }))
            .unwrap();

        let mut info = interfrost::api::modded::merge_partial_version(loader, vanilla);
        assert!(
            client_log_config(&info).is_some(),
            "a loader profile keeps its parent's logging"
        );

        add_log_config_argument(&mut info).unwrap();

        let jvm = &info.arguments.as_ref().expect("jvm arguments")[&ArgumentType::Jvm];
        let Some(Argument::Normal(arg)) = jvm.last() else {
            panic!("no log4j argument: {jvm:?}");
        };
        assert!(arg.starts_with("-Dlog4j.configurationFile="), "{arg}");
        assert!(arg.ends_with("client-1.12.xml"), "{arg}");
    }

    fn mirrored_version(log4j: Option<&str>, main_class: &str) -> VersionInfo {
        let libraries = log4j
            .map(|version| vec![serde_json::json!({ "name": format!("org.apache.logging.log4j:log4j-core:{version}") })])
            .unwrap_or_default();

        serde_json::from_value(serde_json::json!({
            "assetIndex": { "id": "1.8", "sha1": "", "size": 0, "totalSize": 0, "url": "" },
            "assets": "1.8",
            "downloads": {},
            "id": "1.8.9",
            "libraries": libraries,
            "mainClass": main_class,
            "minecraftArguments": "--username ${auth_player_name}",
            "minimumLauncherVersion": 0,
            "releaseTime": "2015-12-03T09:24:39+00:00",
            "time": "2015-12-03T09:24:39+00:00",
            "type": "release"
        }))
        .unwrap()
    }

    fn filled_config_id(info: &VersionInfo) -> Option<String> {
        client_log_config(info).map(|(_, file)| file.id.clone())
    }

    /// The mirror serves no `logging` and 1.7 to 1.16.5 bundle a log4j the
    /// lookup flag does not reach
    #[test]
    fn a_mirrored_version_gets_the_config_mojang_names_for_its_log4j() {
        let main = "net.minecraft.client.main.Main";

        let mut beta9 = mirrored_version(Some("2.0-beta9"), main);
        fill_missing_log_config(&mut beta9, GameLoader::Vanilla);
        assert_eq!(filled_config_id(&beta9).as_deref(), Some("client-1.7.xml"));

        let mut newer = mirrored_version(Some("2.8.1"), main);
        fill_missing_log_config(&mut newer, GameLoader::Fabric);
        assert_eq!(filled_config_id(&newer).as_deref(), Some("client-1.12.xml"));

        let mut pre_log4j = mirrored_version(None, main);
        fill_missing_log_config(&mut pre_log4j, GameLoader::Vanilla);
        assert_eq!(filled_config_id(&pre_log4j), None, "1.6 and older have no log4j");
    }

    #[test]
    fn modlauncher_forge_keeps_its_own_log4j_config() {
        let mut modlauncher = mirrored_version(Some("2.8.1"), "cpw.mods.modlauncher.Launcher");
        fill_missing_log_config(&mut modlauncher, GameLoader::Forge);
        assert_eq!(filled_config_id(&modlauncher), None);

        let mut launchwrapper =
            mirrored_version(Some("2.0-beta9"), "net.minecraft.launchwrapper.Launch");
        fill_missing_log_config(&mut launchwrapper, GameLoader::Forge);
        assert_eq!(
            filled_config_id(&launchwrapper).as_deref(),
            Some("client-1.7.xml"),
            "LaunchWrapper Forge runs on the vanilla config as in Mojang's launcher"
        );
    }

    #[test]
    fn the_console_logs_plain_lines_rather_than_xml_events() {
        let mojang = r#"<Configuration status="WARN">
    <Appenders>
        <Console name="SysOut" target="SYSTEM_OUT">
            <LegacyXMLLayout />
        </Console>
        <RollingRandomAccessFile name="File" fileName="logs/latest.log" filePattern="logs/%d{yyyy-MM-dd}-%i.log.gz">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level]: %msg{nolookups}%n" />
        </RollingRandomAccessFile>
    </Appenders>
</Configuration>"#;

        let plain = plain_console_config(mojang).expect("a convertible config");

        assert!(!plain.contains("XMLLayout"), "{plain}");
        assert_eq!(
            plain
                .matches(r#"<PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level]: %msg{nolookups}%n" />"#)
                .count(),
            2,
            "{plain}"
        );
        assert!(plain_console_config("<Configuration />").is_none());
    }

    #[test]
    fn size_check_falls_back_to_presence_without_a_manifest_size() {
        let dir = scratch("size-unknown");
        let file = dir.join("object.bin");
        std::fs::write(&file, b"anything").unwrap();

        // Maven-coordinate libraries carry no artifact metadata
        assert!(matches_expected_size(&file, 0));
        assert!(!matches_expected_size(&dir.join("nope.bin"), 0));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
