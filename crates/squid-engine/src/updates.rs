//! Update-installers collector (升级安装包): A+B combined.
//!
//! Layer A is a table of known apps whose update drop folders never
//! self-clean (WeChat, WXWork, WPS, …) — precise paths plus the process
//! that must not be running while we sweep. Layer B is a heuristic sweep
//! of one level under each AppData root, descending into directories
//! *named* like update staging areas (update/upgrade/setup/patch/…) so
//! unknown apps are still caught. Layer A results are also fed back into
//! B's seen-set, so one folder is never offered twice.
//!
//! Both layers only ever propose individual **installer-shaped files**
//! (`.exe .msi .msp .zip .7z .rar .cab .crx .nupkg .appx .patch`, ≥ 1 MiB,
//! untouched for ≥ 7 days). Runtime components that apps keep next to
//! those packages (`.bin`, `.dll`, version folders) are deliberately never
//! matched: WeChat's `update\RadiumWMPF.bin` is the *current* version, not
//! junk. Per-file items reuse the plain File deletion + undo path.

use squid_core::{Category, CleanupItem, ItemKind, LockReason};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 7 天：用户确认的阈值。升级后一周内没人再碰的包，应用自己也不会再用了。
pub const MIN_AGE_DAYS: u64 = 7;
/// 1 MiB：小碎片不值得列出来。
pub const MIN_SIZE_BYTES: u64 = 1024 * 1024;

const INSTALLER_EXTENSIONS: [&str; 11] = [
    "exe", "msi", "msp", "zip", "7z", "rar", "cab", "crx", "nupkg", "appx", "patch",
];

/// Directory names that mark an update staging area (layer B).
const UPDATE_DIR_NAMES: [&str; 9] = [
    "update",
    "updates",
    "upgrade",
    "upgrades",
    "setup",
    "installer",
    "installers",
    "patch",
    "patches",
];

/// Vendor folders layer B must not sweep. WinGet's package cache belongs to
/// the developer category; Tencent/kingsoft/Netease are covered precisely by
/// layer A (the heuristic would only double-report the same bytes).
const SKIP_VENDORS: [&str; 4] = ["Microsoft", "winget", "Tencent", "kingsoft"];

/// Layer A: known apps → (directory relative to the AppData roots, display
/// label, process that locks the group while running).
struct KnownApp {
    dir: &'static str,
    label: &'static str,
    process: &'static str,
}

#[cfg(windows)]
const KNOWN_APPS: &[KnownApp] = &[
    KnownApp {
        dir: r"Tencent\xwechat\update",
        label: "微信",
        process: "weixin.exe",
    },
    KnownApp {
        dir: r"Tencent\WXWork\patch",
        label: "企业微信",
        process: "wxwork.exe",
    },
    KnownApp {
        dir: r"kingsoft\office6\update",
        label: "WPS Office",
        process: "wps.exe",
    },
    KnownApp {
        dir: r"kingsoft\WPS Cloud Files\update",
        label: "WPS 云文档",
        process: "wpscloudsvr.exe",
    },
    KnownApp {
        dir: r"Netease\MailMaster\Update",
        label: "网易邮箱大师",
        process: "mailmaster.exe",
    },
];

/// macOS layer A: Squirrel's update payload folders (`*.nupkg`, full
/// machines) under AppData-style roots.
#[cfg(not(windows))]
const KNOWN_APPS: &[KnownApp] = &[KnownApp {
    dir: "SquirrelTemp",
    label: "Squirrel 更新包",
    process: "",
}];

#[cfg(windows)]
fn appdata_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local));
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(roaming));
    }
    roots
}

#[cfg(not(windows))]
fn appdata_roots() -> Vec<PathBuf> {
    let home = squid_core::paths::home_dir();
    vec![
        home.join("Library/Caches"),
        home.join("Library/Application Support"),
    ]
}

pub fn is_installer(file: &Path) -> bool {
    let Some(ext) = file.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    INSTALLER_EXTENSIONS
        .iter()
        .any(|wanted| ext.eq_ignore_ascii_case(wanted))
}

fn stale_enough(meta: &std::fs::Metadata) -> bool {
    if meta.len() < MIN_SIZE_BYTES {
        return false;
    }
    let modified = meta
        .modified()
        .or_else(|_| meta.created())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    modified
        .elapsed()
        .map(|age| age.as_secs() >= MIN_AGE_DAYS * 86_400)
        .unwrap_or(true)
}

/// Depth-capped sweep of one staging directory: every stale installer file
/// becomes its own item (File kind → plain delete + undo path).
fn sweep(
    dir: &Path,
    label: &str,
    rule: &str,
    blocked: bool,
    out: &mut Vec<CleanupItem>,
    seen: &mut HashSet<PathBuf>,
) {
    let walker = walkdir::WalkDir::new(dir)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !entry.file_type().is_symlink());
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !is_installer(path) || !stale_enough(&meta) {
            continue;
        }
        if !seen.insert(path.to_path_buf()) {
            continue;
        }
        let item = CleanupItem::new(
            path.to_path_buf(),
            ItemKind::File,
            Category::UpdateInstallers,
            format!("{label} · {}", squash_name(path)),
        )
        .with_rule(rule)
        .with_anchors(std::iter::once(dir.to_path_buf()))
        .with_note("升级完成后不再需要；需要时应用会自动重新下载");
        let item = if blocked {
            item.locked(LockReason::AppRunning)
        } else {
            item
        };
        out.push(item);
    }
}

/// Long hashed package names read badly in a list; keep a head+tail hint.
fn squash_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= 42 {
        return name;
    }
    let head: String = chars[..24].iter().collect();
    let tail: String = chars[chars.len() - 16..].iter().collect();
    format!("{head}…{tail}")
}

/// One entry point. The caller dedups against rule-matched items and the
/// safety layer (whitelist / protected) is applied like for any other item.
pub fn collect(engine: &crate::Engine, cancel: &squid_core::CancelToken) -> Vec<CleanupItem> {
    let mut out = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    if cancel.is_cancelled() {
        return out;
    }
    let running: Vec<String> = engine
        .platform()
        .running_executables()
        .into_iter()
        .map(|proc| {
            proc.rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_lowercase()
        })
        .collect();
    let roots = appdata_roots();

    // Layer A: precise known-app staging folders.
    for app in KNOWN_APPS {
        let blocked = !app.process.is_empty() && running.iter().any(|name| name == app.process);
        for root in &roots {
            let dir = root.join(app.dir);
            if dir.is_dir() {
                sweep(
                    &dir,
                    app.label,
                    "update-installers-known",
                    blocked,
                    &mut out,
                    &mut seen,
                );
            }
        }
    }

    // Layer B: heuristic sweep of <AppDataRoot>/<vendor>/<update-dir>.
    for root in &roots {
        let Ok(vendors) = std::fs::read_dir(root) else {
            continue;
        };
        for vendor in vendors.flatten() {
            if cancel.is_cancelled() {
                return out;
            }
            if !vendor.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let vendor_name = vendor.file_name().to_string_lossy().into_owned();
            if SKIP_VENDORS
                .iter()
                .any(|skip| vendor_name.eq_ignore_ascii_case(skip))
            {
                continue;
            }
            let Ok(subs) = std::fs::read_dir(vendor.path()) else {
                continue;
            };
            for sub in subs.flatten() {
                let name = sub.file_name().to_string_lossy().into_owned();
                if !UPDATE_DIR_NAMES.contains(&name.to_lowercase().as_str()) || !sub.path().is_dir()
                {
                    continue;
                }
                sweep(
                    &sub.path(),
                    &vendor_name,
                    "update-installers-scan",
                    false,
                    &mut out,
                    &mut seen,
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_shapes_are_matched_and_runtime_is_not() {
        assert!(is_installer(Path::new("a/patch.zip")));
        assert!(is_installer(Path::new("a/Weixin.EXE")));
        assert!(is_installer(Path::new("a/update.bin.patch")));
        // Runtime components must never match — this is the WeChat rule.
        assert!(!is_installer(Path::new("a/RadiumWMPF.bin")));
        assert!(!is_installer(Path::new("a/Weixin.dll")));
        assert!(!is_installer(Path::new("a/manifest.dat")));
        assert!(!is_installer(Path::new("a/noext")));
    }

    #[test]
    fn tiny_files_are_not_offered() {
        let temp = std::env::temp_dir().join(format!("squid-updates-size-{}", std::process::id()));
        let Ok(()) = std::fs::create_dir_all(&temp) else {
            eprintln!("skipping: scratch dir failed");
            return;
        };
        let file = temp.join("small.exe");
        if let Err(error) = std::fs::write(&file, vec![0u8; 1024]) {
            eprintln!("skipping: scratch write denied ({error})");
            return;
        }
        let meta = std::fs::metadata(&file).expect("meta");
        assert!(!stale_enough(&meta));
        let big = temp.join("big.exe");
        if std::fs::write(&big, vec![0u8; 2 * 1024 * 1024]).is_ok() {
            let meta = std::fs::metadata(&big).expect("big meta");
            // fresh: size passes but the 7-day gate holds it back
            assert!(!stale_enough(&meta));
        }
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn long_package_names_get_squashed() {
        let name = squash_name(Path::new(&format!("C:/x/{}-x64_zh-CN.msi", "a".repeat(60))));
        assert!(name.chars().count() <= 42, "{name}");
        assert!(name.ends_with("-x64_zh-CN.msi"));
    }
}
