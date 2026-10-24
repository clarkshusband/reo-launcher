use std::cmp::Ordering;
use std::path::PathBuf;

use crate::checker::{JavaCheckInfo, check_java_runtime};
use crate::data::java_executable_relative_path;
use crate::error::JavaResult;
use crate::platform::matches_host_arch;
use crate::service::parse_major_version;
use reo_common::constants::JAVA_BIN;

pub type LocatedJava = (PathBuf, JavaCheckInfo);

/// Best first
#[tracing::instrument(level = "debug")]
pub async fn locate_java() -> JavaResult<Vec<LocatedJava>> {
    let paths = internal_locate_java();
    let mut valid: Vec<LocatedJava> = Vec::new();

    for path in paths {
        if valid.iter().any(|(existing, _)| existing == &path) {
            continue;
        }

        let Ok(info) = check_java_runtime(path.display().to_string()).await else {
            tracing::warn!("java installation at '{}' is not valid", path.display());
            continue;
        };
        valid.push((path, info));
    }

    valid.sort_by(|(_, a), (_, b)| compare_candidates(a, b));

    tracing::debug!(count = valid.len(), "located Java runtimes");

    Ok(valid)
}

/// `major` is a filter not a ranking input
/// a newer JDK never stands in for the requested major
#[must_use]
pub fn best_for_major(candidates: &[LocatedJava], major: u32) -> Option<&LocatedJava> {
    candidates
        .iter()
        .filter(|(_, info)| parse_major_version(&info.version).is_ok_and(|found| found == major))
        .min_by(|(_, a), (_, b)| compare_candidates(a, b))
}

/// Best first a JVM built for this host beats one that is not, as a 32-bit JRE
/// cannot reserve a large heap, then a JDK beats a JRE outright and then the
/// newer version wins
fn compare_candidates(a: &JavaCheckInfo, b: &JavaCheckInfo) -> Ordering {
    matches_host_arch(&b.os_arch)
        .cmp(&matches_host_arch(&a.os_arch))
        .then_with(|| b.is_jdk.cmp(&a.is_jdk))
        .then_with(|| version_key(b).cmp(&version_key(a)))
}

/// Numeric so `21.0.10` sorts above `21.0.9`
fn version_key(info: &JavaCheckInfo) -> (u32, Vec<u32>) {
    let major = parse_major_version(&info.version).unwrap_or(0);
    let parts = info
        .version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse::<u32>().ok())
        .collect();

    (major, parts)
}

#[cfg(target_os = "windows")]
#[tracing::instrument(level = "debug")]
fn internal_locate_java() -> Vec<PathBuf> {
    let mut java_homes = Vec::new();

    // A 32-bit launcher sees the x86 folder as ProgramFiles
    let program_files = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
    let program_files_x86 = std::env::var_os("ProgramFiles(x86)")
        .map_or_else(|| PathBuf::from(r"C:\Program Files (x86)"), PathBuf::from);

    let windows_paths = {
        let mut paths = vec![
            program_files.join("Java"),
            program_files_x86.join("Java"),
            program_files.join("Eclipse Adoptium"),
            program_files.join("Eclipse Foundation"),
            program_files.join("AdoptOpenJDK"),
            program_files.join("Amazon Corretto"),
            program_files.join("BellSoft"),
            program_files.join("Zulu"),
            program_files.join("OpenJDK"),
            program_files.join("Microsoft"),
            program_files.join("Semeru"),
            program_files.join("IBM"),
            PathBuf::from(r"C:\ProgramData\chocolatey\lib"),
        ];
        if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            paths.push(profile.join(".jdks"));
            paths.push(profile.join(".gradle").join("jdks"));
            paths.push(profile.join(".m2").join("wrapper").join("dists"));
            paths.push(profile.join("scoop").join("apps"));
        }
        paths
    };

    for base_path in windows_paths {
        if let Ok(entries) = std::fs::read_dir(base_path) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    let java_exe = entry.path().join(java_executable_relative_path());
                    if java_exe.exists() {
                        java_homes.push(java_exe);
                    }
                }
            }
        }
    }

    // Each IDE bundles the JetBrains Runtime in its own jbr folder
    if let Ok(ides) = std::fs::read_dir(program_files.join("JetBrains")) {
        for ide in ides.flatten() {
            let java_exe = ide
                .path()
                .join("jbr")
                .join(java_executable_relative_path());
            if java_exe.exists() {
                java_homes.push(java_exe);
            }
        }
    }

    let mut minecraft_runtimes = vec![program_files_x86.join("Minecraft Launcher").join("runtime")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        minecraft_runtimes.push(
            local
                .join("Packages")
                .join("Microsoft.4297127D64EC6_8wekyb3d8bbwe")
                .join("LocalCache")
                .join("Local")
                .join("runtime"),
        );
    }
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        minecraft_runtimes.push(roaming.join(".minecraft").join("runtime"));
    }

    // The official launcher keeps each runtime at <component>\<platform>\<component>
    for root in minecraft_runtimes {
        let Ok(components) = std::fs::read_dir(root) else {
            continue;
        };
        for component in components.flatten() {
            let Ok(platforms) = std::fs::read_dir(component.path()) else {
                continue;
            };
            for platform in platforms.flatten() {
                let java_exe = platform
                    .path()
                    .join(component.file_name())
                    .join(java_executable_relative_path());
                if java_exe.exists() {
                    java_homes.push(java_exe);
                }
            }
        }
    }

    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        let java_exe =
            PathBuf::from(java_home.trim().trim_matches('"')).join(java_executable_relative_path());
        if java_exe.exists() {
            java_homes.push(java_exe);
        }
    }

    // split_paths drops the quotes Windows allows around an entry
    if let Some(path) = std::env::var_os("PATH") {
        for path_entry in std::env::split_paths(&path) {
            if path_entry.as_os_str().is_empty() {
                continue;
            }
            let java_exe = path_entry.join(JAVA_BIN);
            if java_exe.exists() && !java_homes.contains(&java_exe) {
                java_homes.push(java_exe);
            }
        }
    }

    java_homes
}

#[cfg(target_os = "macos")]
#[tracing::instrument(level = "debug")]
fn internal_locate_java() -> Vec<PathBuf> {
    let mut found = Vec::new();

    let macos_paths = {
        let mut paths = vec![
            PathBuf::from("/Library/Java/JavaVirtualMachines"),
            PathBuf::from("/System/Library/Java/JavaVirtualMachines"),
            PathBuf::from("/System/Library/Frameworks/JavaVM.framework/Versions/Current/Commands"),
            PathBuf::from("/Library/Internet Plug-Ins/JavaAppletPlugin.plugin/Contents/Home"),
            PathBuf::from("/opt/homebrew/Cellar"),
            PathBuf::from("/opt/homebrew/opt"),
            PathBuf::from("/usr/local/Cellar"),
            PathBuf::from("/usr/local/opt"),
        ];
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            paths.push(
                home.join("Library")
                    .join("Java")
                    .join("JavaVirtualMachines"),
            );
            paths.push(home.join(".jdks"));
            paths.push(home.join(".sdkman").join("candidates").join("java"));
        }
        paths
    };

    for path in macos_paths {
        if let Ok(children) = std::fs::read_dir(path) {
            for child in children.flatten() {
                let java_exe = child.path().join(java_executable_relative_path());
                if java_exe.exists() {
                    found.push(java_exe);
                }
            }
        }
    }

    find_java_in_path(found)
}

#[cfg(target_os = "linux")]
#[tracing::instrument(level = "debug")]
fn internal_locate_java() -> Vec<PathBuf> {
    let mut found = Vec::new();

    let linux_paths = {
        let mut paths = vec![
            PathBuf::from("/usr/lib/jvm"),
            PathBuf::from("/usr/lib64/jvm"),
            PathBuf::from("/usr/lib32/jvm"),
            PathBuf::from("/usr/java"),
            PathBuf::from("/opt/jdk"),
            PathBuf::from("/opt/jdks"),
            PathBuf::from("/app/jdk"),
        ];
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            paths.push(home.join(".jdks"));
            paths.push(home.join(".gradle").join("jdks"));
            paths.push(home.join(".sdkman").join("candidates").join("java"));
        }
        paths
    };

    for path in linux_paths {
        if let Ok(children) = std::fs::read_dir(path) {
            for child in children.flatten() {
                let java_exe = child.path().join(java_executable_relative_path());
                if java_exe.exists() {
                    found.push(java_exe);
                }
            }
        }
    }

    find_java_in_path(found)
}

#[cfg(unix)]
fn find_java_in_path(mut found: Vec<PathBuf>) -> Vec<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for path_entry in path.split(':') {
            let java = PathBuf::from(path_entry).join(JAVA_BIN);
            if java.exists() && java.is_file() && !found.contains(&java) {
                found.push(java);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(path: &str, version: &str, is_jdk: bool) -> LocatedJava {
        (
            PathBuf::from(path),
            JavaCheckInfo {
                version: version.to_string(),
                vendor: String::from("Test"),
                os_arch: crate::platform::host_os_arch().to_string(),
                is_jdk,
            },
        )
    }

    fn foreign(mut located: LocatedJava) -> LocatedJava {
        located.1.os_arch = crate::platform::foreign_os_arch().to_string();
        located
    }

    fn ranked(mut candidates: Vec<LocatedJava>) -> Vec<String> {
        candidates.sort_by(|(_, a), (_, b)| compare_candidates(a, b));
        candidates
            .into_iter()
            .map(|(path, _)| path.display().to_string())
            .collect()
    }

    #[test]
    fn patch_levels_are_compared_as_numbers() {
        let ranking = ranked(vec![
            candidate("/jdk-21.0.9", "21.0.9", true),
            candidate("/jdk-21.0.10", "21.0.10", true),
        ]);

        assert_eq!(ranking, vec!["/jdk-21.0.10", "/jdk-21.0.9"]);
    }

    #[test]
    fn the_requested_major_still_wins_over_a_newer_kit() {
        let candidates = vec![
            candidate("/jdk-21", "21.0.3", true),
            candidate("/jre-8", "1.8.0_412", false),
        ];

        let (path, _) = best_for_major(&candidates, 8).expect("a java 8 candidate");
        assert_eq!(path, &PathBuf::from("/jre-8"));
    }

    #[test]
    fn the_kit_wins_among_candidates_of_the_requested_major() {
        let candidates = vec![
            candidate("/jre-17-newer", "17.0.11", false),
            candidate("/jdk-17", "17.0.2", true),
            candidate("/jdk-21", "21.0.3", true),
        ];

        let (path, _) = best_for_major(&candidates, 17).expect("a java 17 candidate");
        assert_eq!(path, &PathBuf::from("/jdk-17"));
    }

    #[test]
    fn a_runtime_built_for_the_host_beats_a_newer_foreign_kit() {
        let candidates = vec![
            foreign(candidate("/x86/jdk-8", "1.8.0_412", true)),
            candidate("/jre-8", "1.8.0_402", false),
        ];

        let (path, _) = best_for_major(&candidates, 8).expect("a java 8 candidate");
        assert_eq!(path, &PathBuf::from("/jre-8"));
    }

    #[test]
    fn a_foreign_runtime_is_still_picked_when_nothing_else_fits() {
        let candidates = vec![
            foreign(candidate("/x86/jre-8", "1.8.0_412", false)),
            candidate("/jdk-21", "21.0.3", true),
        ];

        let (path, _) = best_for_major(&candidates, 8).expect("a java 8 candidate");
        assert_eq!(path, &PathBuf::from("/x86/jre-8"));
    }

    #[test]
    fn a_major_nobody_satisfies_has_no_best() {
        let candidates = vec![candidate("/jdk-21", "21.0.3", true)];

        assert!(best_for_major(&candidates, 8).is_none());
    }
}
