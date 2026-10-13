//! Generic application discovery used by the per-OS adapters.

use squid_core::progress::CancelToken;
use squid_core::{AppInfo, Bytes};
use std::path::{Path, PathBuf};

/// Metadata pulled out of a macOS bundle.
#[derive(Clone, Debug, Default)]
pub struct BundleInfo {
    pub name: Option<String>,
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    pub short_version: Option<String>,
    pub publisher: Option<String>,
    pub executable: Option<String>,
}

fn string_field(dict: &plist::Dictionary, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(|v| v.as_string())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn read_bundle(app_path: &Path) -> Option<BundleInfo> {
    let plist_path = app_path.join("Contents/Info.plist");
    let value = plist::Value::from_file(&plist_path).ok()?;
    let dict = value.as_dictionary()?;
    let info = BundleInfo {
        name: string_field(dict, "CFBundleDisplayName")
            .or_else(|| string_field(dict, "CFBundleName")),
        bundle_id: string_field(dict, "CFBundleIdentifier"),
        version: string_field(dict, "CFBundleVersion"),
        short_version: string_field(dict, "CFBundleShortVersionString"),
        publisher: string_field(dict, "CFBundleGetInfoString").map(|info| {
            info.split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        }),
        executable: string_field(dict, "CFBundleExecutable"),
    };
    Some(info)
}

/// Walk a source directory looking for `.app` bundles, without descending into
/// the bundles themselves (millions of files we never need to list).
pub fn discover_bundles(dir: &Path, max_depth: usize, cancel: &CancelToken) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if !dir.is_dir() {
        return found;
    }
    let mut stack: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    let limit = max_depth.max(1);
    while let Some((current, depth)) = stack.pop() {
        if cancel.is_cancelled() {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = entry
                .file_type()
                .map(|ft| ft.is_dir())
                .unwrap_or_else(|_| path.is_dir());
            if !is_dir {
                continue;
            }
            if name.ends_with(".app") {
                found.push(path);
                continue;
            }
            // Never wander into system frameworks or a volume's Trash.
            if matches!(
                name.as_str(),
                "Contents" | "System" | "Library" | ".Trashes" | "Volumes" | "node_modules"
            ) {
                continue;
            }
            if depth < limit {
                stack.push((path, depth + 1));
            }
        }
    }
    found.sort();
    found
}

pub fn app_from_bundle(
    path: PathBuf,
    source: &str,
    running: &[String],
    measure: bool,
) -> Option<AppInfo> {
    let dir_name = path
        .file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(".app").to_string())
        .unwrap_or_default();
    if dir_name.is_empty() {
        return None;
    }
    let bundle = read_bundle(&path).unwrap_or_default();
    let name = bundle.name.clone().unwrap_or(dir_name.clone());
    let mut app = AppInfo {
        name,
        bundle_id: bundle.bundle_id.clone(),
        path: path.clone(),
        version: bundle
            .short_version
            .clone()
            .or_else(|| bundle.version.clone()),
        size: if measure {
            Bytes(
                squid_core::fs::measure(&path, squid_core::ItemKind::Path)
                    .allocated
                    .as_u64(),
            )
        } else {
            Bytes::ZERO
        },
        source: source.to_string(),
        running: is_running(&dir_name, bundle.executable.as_deref(), running),
        has_sibling_install: false,
        publisher: bundle.publisher.clone(),
        installed_ms: path
            .metadata()
            .ok()
            .and_then(|m| m.created().ok().or_else(|| m.modified().ok()))
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64),
    };
    if app.name.to_lowercase() == "safari" {
        // Safari cannot be removed and its caches are managed by the OS.
        app.bundle_id = app.bundle_id.or_else(|| Some("com.apple.Safari".into()));
    }
    Some(app)
}

/// Match "is this app running" against the process executable list.
pub fn is_running(dir_name: &str, executable: Option<&str>, running: &[String]) -> bool {
    let mut candidates: Vec<String> = Vec::with_capacity(3);
    if let Some(exe) = executable {
        candidates.push(exe.to_lowercase());
    }
    candidates.push(dir_name.to_lowercase());
    if dir_name.to_lowercase().ends_with(".app") {
        candidates.push(dir_name.to_lowercase().trim_end_matches(".app").to_string());
    }
    running.iter().any(|proc| {
        let proc = proc.to_lowercase();
        let leaf = proc.rsplit('/').next().unwrap_or(&proc).to_string();
        candidates
            .iter()
            .any(|c| !c.is_empty() && (leaf == *c || leaf == format!("{c}.app")))
    })
}

/// Case-insensitive file-name stem, e.g. `Safari.app` -> `safari`.
pub fn stem_lower(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(".app").to_lowercase())
        .unwrap_or_default()
}
