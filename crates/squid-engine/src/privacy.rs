//! 隐私清理 (privacy cleaning) — the traces page.
//!
//! Different from every other engine on purpose: these deletions do **not**
//! go to the Recycle Bin. A browser profile half-restored from the bin is
//! worse than no restore at all (Chrome refuses profiles whose History file
//! was swapped behind its back), so the honest answer is: gone is gone, but
//! *what* was gone is written to the same history ledger, and the browser
//! lock check happens at scan time — an open browser shows a 🔒 chip instead
//! of silently failing mid-delete.

use squid_core::progress::CancelToken;
use squid_core::{Bytes, Category, CleanupItem, ItemKind, LockReason};
use std::path::PathBuf;

/// One privacy target: display copy, the paths it wipes, and the process
/// that must be closed first (browser profiles are exclusively locked).
struct PrivacySpec {
    id: &'static str,
    label_zh: &'static str,
    detail_zh: &'static str,
    /// (path, kind) pairs; Contents keeps the folder, deletes its children.
    parts: &'static [(&'static str, ItemKind)],
    /// Executable (lower-case, no path) that locks these files.
    process: &'static str,
}

const SPECS: &[PrivacySpec] = &[
    PrivacySpec {
        id: "privacy-edge",
        label_zh: "Edge 浏览记录与 Cookie",
        detail_zh: "历史、Cookie、会话恢复数据；清空后需要重新登录网站",
        parts: &[
            (
                r"AppData\Local\Microsoft\Edge\User Data\Default\History",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Microsoft\Edge\User Data\Default\History-journal",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Microsoft\Edge\User Data\Default\Network\Cookies",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Microsoft\Edge\User Data\Default\Network\Cookies-wal",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Microsoft\Edge\User Data\Default\Network\Cookies-shm",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Microsoft\Edge\User Data\Default\Session Storage",
                ItemKind::Contents,
            ),
        ],
        process: "msedge.exe",
    },
    PrivacySpec {
        id: "privacy-chrome",
        label_zh: "Chrome 浏览记录与 Cookie",
        detail_zh: "历史、Cookie、会话恢复数据；清空后需要重新登录网站",
        parts: &[
            (
                r"AppData\Local\Google\Chrome\User Data\Default\History",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Google\Chrome\User Data\Default\History-journal",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Google\Chrome\User Data\Default\Network\Cookies",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Google\Chrome\User Data\Default\Network\Cookies-wal",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Google\Chrome\User Data\Default\Network\Cookies-shm",
                ItemKind::File,
            ),
            (
                r"AppData\Local\Google\Chrome\User Data\Default\Session Storage",
                ItemKind::Contents,
            ),
        ],
        process: "chrome.exe",
    },
    PrivacySpec {
        id: "privacy-recent",
        label_zh: "最近使用的文件 / 跳转列表",
        detail_zh: "开始菜单与任务栏右键里的最近记录（链接本身，不删文件）",
        parts: &[
            (
                r"AppData\Roaming\Microsoft\Windows\Recent",
                ItemKind::Contents,
            ),
            (
                r"AppData\Roaming\Microsoft\Windows\Recent\AutomaticDestinations",
                ItemKind::Contents,
            ),
            (
                r"AppData\Roaming\Microsoft\Windows\Recent\CustomDestinations",
                ItemKind::Contents,
            ),
        ],
        process: "",
    },
    PrivacySpec {
        id: "privacy-clipboard",
        label_zh: "剪贴板历史（Win+V）",
        detail_zh: "Win+V 面板里缓存的历史条目，删除后列表清空",
        parts: &[(r"AppData\Local\Microsoft\Clipboard", ItemKind::Contents)],
        process: "",
    },
    PrivacySpec {
        id: "privacy-activity",
        label_zh: "活动历史记录（时间线）",
        detail_zh: "设置→隐私→活动历史；删除后「恢复上次会话」不再可用",
        parts: &[(
            r"AppData\Local\ConnectedDevicesPlatform",
            ItemKind::Contents,
        )],
        process: "",
    },
];

/// Catalog for the UI: id + copy + estimated size, current lock state.
pub fn collect(engine: &crate::Engine, cancel: &CancelToken) -> Vec<CleanupItem> {
    let home = squid_core::paths::home_dir();
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
    let mut out = Vec::new();
    for spec in SPECS {
        if cancel.is_cancelled() {
            return out;
        }
        let blocked = !spec.process.is_empty() && running.iter().any(|p| p == spec.process);
        for (tail, kind) in spec.parts {
            let path = home.join(tail);
            if !path.exists() {
                continue;
            }
            let bytes = if path.is_dir() {
                walkdir::WalkDir::new(&path)
                    .max_depth(4)
                    .into_iter()
                    .filter_entry(|e| !e.file_type().is_symlink())
                    .flatten()
                    .filter(|e| e.file_type().is_file())
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            } else {
                std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
            };
            let mut item = CleanupItem::new(path.clone(), *kind, Category::Privacy, spec.label_zh)
                .with_rule(spec.id)
                .with_anchors(std::iter::once(home.clone()))
                .with_note(spec.detail_zh);
            item.size = Bytes(bytes);
            item.allocated = Bytes(bytes);
            item.selected = false; // privacy deletes are opt-in, always
            item.evidence = vec!["record:privacy-trace".into()];
            if blocked {
                item = item
                    .locked(LockReason::AppRunning)
                    .with_note(format!("{}：先完全退出浏览器再清理", spec.process));
            }
            out.push(item);
        }
    }
    out
}

/// Expand a spec id back into the concrete paths it wipes. The delete layer
/// rebuilds Fresh items from these (never trusting the scan view).
pub fn paths_for(id: &str) -> Vec<(PathBuf, ItemKind)> {
    let home = squid_core::paths::home_dir();
    let Some(spec) = SPECS.iter().find(|spec| spec.id == id) else {
        return Vec::new();
    };
    spec.parts
        .iter()
        .map(|(tail, kind)| (home.join(tail), *kind))
        .filter(|(path, _)| path.exists())
        .collect()
}

/// The lock reason if this spec cannot be cleaned right now.
pub fn lock_reason(engine: &crate::Engine, id: &str) -> Option<String> {
    let spec = SPECS.iter().find(|spec| spec.id == id)?;
    if spec.process.is_empty() {
        return None;
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
    running
        .iter()
        .any(|p| p == spec.process)
        .then(|| format!("{} 正在运行，请先完全退出", spec.process))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_id_is_unique_and_prefixed() {
        let mut seen = std::collections::HashSet::new();
        for spec in SPECS {
            assert!(spec.id.starts_with("privacy-"), "{}", spec.id);
            assert!(seen.insert(spec.id), "duplicate id {}", spec.id);
        }
    }

    #[test]
    fn unknown_ids_expand_to_nothing() {
        assert!(paths_for("privacy-nonsense").is_empty());
        assert!(paths_for("").is_empty());
    }

    #[test]
    fn known_ids_expand_without_panicking() {
        // Real machine: some paths exist, some do not — either way the
        // expansion must be a subset of the home tree, never a foreign root.
        let home = squid_core::paths::home_dir();
        for spec in SPECS {
            for (path, _) in paths_for(spec.id) {
                assert!(path.starts_with(&home) || home.join(&path).starts_with(&home));
            }
        }
    }
}
