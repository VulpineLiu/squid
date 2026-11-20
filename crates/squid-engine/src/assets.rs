//! User-asset recognition — the Mole `app_protection` lesson for Windows.
//!
//! Some directories look exactly like caches but hold the user's own work:
//! chat files inside an IM data folder, PWA offline documents in a browser
//! profile's IndexedDB,输入法词库. A broad rule (or an eager user) must never
//! sweep them away, so they are pinned here: the scan lists them as locked,
//! visibly-explained rows, and the delete layer refuses them even when a
//! stale report still claims they are removable.

use squid_core::{Category, CleanupItem, ItemKind};
use std::path::Path;

/// Lower-cased path fragment → human reason. Matching is on the *normalised*
/// (lower-case, `/`→`\`) path so one table covers every drive letter and user.
const ASSET_MARKERS: &[(&str, &str)] = &[
    (
        "\\appdata\\roaming\\texact\\",
        "腾讯 QQ 数据目录（聊天记录与文件）",
    ),
    (
        "\\appdata\\roaming\\tencent\\",
        "腾讯通用数据目录（聊天文件与配置）",
    ),
    (
        "\\documents\\weixin files\\filestorage",
        "微信聊天文件与下载文档",
    ),
    (
        "\\appdata\\roaming\\weixin\\filestorage",
        "微信聊天文件与下载文档",
    ),
    ("\\appdata\\roaming\\txdnd\\", "腾讯游戏数据目录"),
    ("\\sogouexplorer\\", "搜狗输入法词库与用户习惯"),
    (
        "\\appdata\\roaming\\sogouinput\\",
        "搜狗输入法词库与用户习惯",
    ),
];

/// Browser profile storage that holds PWA / offline web app documents —
/// Mole's `PROTECTED_SW_DOMAINS` equivalent. An `IndexedDB` folder inside a
/// Chromium-family browser profile is user work data (Figma / 剪映网页版 save
/// there), unlike `Cache` which is pure rebuildable bytes.
const CHROMIUM_PROFILE_MARKERS: &[&str] = &[
    "\\microsoft\\edge\\user data\\",
    "\\google\\chrome\\user data\\",
    "\\chromium\\application\\user data\\",
];

fn normalise(path: &Path) -> String {
    path.to_string_lossy().to_lowercase().replace('/', "\\")
}

/// Why this path is untouchable user property, if it is. The note lands in
/// the UI lock chip's explanation, mirroring 电脑管家's "创建者" honesty.
pub fn asset_reason(path: &Path) -> Option<&'static str> {
    let lower = normalise(path);
    // Deeper profile sub-paths (…\IndexedDB\foo) stay protected too: match
    // the marker as a path prefix at a separator boundary.
    if lower.contains("\\indexeddb")
        && CHROMIUM_PROFILE_MARKERS
            .iter()
            .any(|marker| lower.starts_with(marker) || lower.contains(marker))
    {
        return Some("可能含离线应用数据（Figma / 剪映网页版等工作文件）");
    }
    ASSET_MARKERS
        .iter()
        .find(|(marker, _)| segment_match(&lower, marker))
        .map(|(_, reason)| *reason)
}

/// True when `marker` appears inside `lower` at path segment boundaries.
/// Markers ending with a separator carry their own trailing boundary, so
/// `TencentGame\cache` must not match `\tencent\`.
fn segment_match(lower: &str, marker: &str) -> bool {
    if marker.ends_with('\\') {
        return lower.contains(marker);
    }
    lower.match_indices(marker).any(|(index, _)| {
        // A marker that opens with `\` carries its own left boundary; a bare
        // marker needs a separator (or drive colon) before it.
        let before_ok = index == 0
            || marker.starts_with('\\')
            || matches!(lower.as_bytes()[index - 1], b'\\' | b':');
        let after = index + marker.len();
        let after_ok = lower.len() == after || lower.as_bytes().get(after) == Some(&b'\\');
        before_ok && after_ok
    })
}

/// Asset rows to surface in the 空间清理 list — existence checked, one row
/// per top-level asset. Rows anchor at their own parent: they are display
/// only (locked), and anchoring at the folder itself would make
/// `block_reason` deny the row's own deletion check.
pub fn known_asset_rows() -> Vec<CleanupItem> {
    const SPECS: [(&str, &str); 6] = [
        (
            r"AppData\Roaming\Tencent",
            "腾讯（QQ/微信旧版）数据目录（聊天记录与文件）",
        ),
        (
            r"Documents\WeChat Files",
            "微信聊天文件与下载文档（旧版目录）",
        ),
        (
            r"Documents\Weixin Files\FileStorage",
            "微信聊天文件与下载文档",
        ),
        (r"AppData\Roaming\SogouInput", "搜狗输入法词库与用户习惯"),
        (
            r"AppData\Local\Microsoft\Edge\User Data\Default\IndexedDB",
            "可能含离线应用数据（Figma / 剪映网页版等工作文件）",
        ),
        (
            r"AppData\Local\Google\Chrome\User Data\Default\IndexedDB",
            "可能含离线应用数据（Figma / 剪映网页版等工作文件）",
        ),
    ];
    let Ok(home) = std::env::var("USERPROFILE") else {
        return Vec::new();
    };
    let home = Path::new(&home);
    let mut rows = Vec::new();
    for (tail, reason) in SPECS {
        let path = home.join(tail);
        if !path.is_dir() {
            continue;
        }
        let label = format!("🔒 {}", asset_label(&path));
        let parent = path
            .parent()
            .map(|dir| dir.to_path_buf())
            .unwrap_or_else(|| path.clone());
        let item = CleanupItem::new(path, ItemKind::Path, Category::AppCaches, label)
            .with_rule("user-assets")
            .with_anchors(std::iter::once(parent))
            .locked(squid_core::LockReason::Protected)
            .with_note(reason);
        rows.push(item);
    }
    rows
}

fn asset_label(path: &Path) -> String {
    let text = squid_core::paths::pretty(path);
    text.rsplit(['\\', '/'])
        .next()
        .filter(|leaf| !leaf.is_empty())
        .unwrap_or(&text)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn im_data_dirs_refuse_deletion_with_a_reason() {
        let path = PathBuf::from(r"C:\Users\me\AppData\Roaming\Tencent\WeChat\111\msg");
        let reason = asset_reason(&path).expect("QQ tree must be protected");
        assert!(reason.contains("聊天"));
    }

    #[test]
    fn wechat_file_storage_is_protected_but_cache_paths_are_not() {
        let storage = PathBuf::from(r"D:\Users\me\Documents\Weixin Files\FileStorage\File\2026-01");
        assert!(asset_reason(&storage).is_some());
        let edge_cache =
            PathBuf::from(r"C:\Users\me\AppData\Local\Microsoft\Edge\User Data\Default\Cache");
        assert!(asset_reason(&edge_cache).is_none());
    }

    #[test]
    fn profile_indexeddb_counts_as_offline_app_data() {
        let pwa = PathBuf::from(
            r"C:\Users\me\AppData\Local\Microsoft\Edge\User Data\Default\IndexedDB\figma_0.indexeddb.leveldb",
        );
        let reason = asset_reason(&pwa).expect("PWA data must be protected");
        assert!(reason.contains("离线应用"));
    }

    #[test]
    fn markers_match_only_at_segment_boundaries() {
        // A folder that merely *starts* with the marker text is not the asset.
        let sneaky = PathBuf::from(r"C:\Users\me\AppData\Roaming\TencentGame\cache");
        assert!(asset_reason(&sneaky).is_none());
    }
}
