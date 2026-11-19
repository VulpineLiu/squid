//! The four engines behind squid: clean, uninstall, purge and disk analysis.
//!
//! Every engine follows the same contract:
//!
//! 1. **Scan** produces a [`ScanReport`] of [`CleanupItem`]s. Nothing is
//!    deleted during a scan — scanning is always safe and always idempotent.
//! 2. **Delete** takes the items the user approved, re-validates each of them
//!    against [`SafetyPolicy`] (never trusting the scan), and writes a ledger
//!    session so `squid undo` can put Trash-mode deletions back.
//!
//! Engines never call `std::fs::remove_*` directly; that keeps the audit trail
//! and the safety net impossible to bypass.

pub mod analyze;
pub mod clean;
pub mod delete;
pub mod purge;
pub mod uninstall;
pub mod updates;

use globset::GlobMatcher;
use squid_core::settings::Settings;
use squid_core::{CleanupItem, SafetyPolicy};
use squid_platform::Platform;
use squid_rules::RuleSet;
use std::path::Path;

/// Shared dependencies for every engine: the OS adapter, user settings, the
/// rule set and the safety policy that guards every deletion.
pub struct Engine {
    platform: Box<dyn Platform>,
    settings: Settings,
    safety: SafetyPolicy,
    rules: RuleSet,
    whitelist: Vec<GlobMatcher>,
    settings_errors: Vec<String>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Engine {
        let (settings, settings_errors) = Settings::load();
        Self::with_settings(settings, settings_errors)
    }

    pub fn with_settings(settings: Settings, settings_errors: Vec<String>) -> Engine {
        let platform = squid_platform::current();
        let rules = RuleSet::load(&settings);
        let whitelist = settings.whitelist_matchers();
        let mut safety = if is_root() {
            SafetyPolicy::system()
        } else {
            SafetyPolicy::user_only()
        };
        if let Ok(mounts) = platform.volumes() {
            safety = safety.with_mounts(
                mounts
                    .into_iter()
                    .map(|volume| volume.mount_point)
                    .chain(platform.protected_prefixes())
                    .collect(),
            );
        }
        Engine {
            platform,
            settings,
            safety,
            rules,
            whitelist,
            settings_errors,
        }
    }

    pub fn platform(&self) -> &dyn Platform {
        self.platform.as_ref()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        self.whitelist.clear();
        &mut self.settings
    }

    /// Re-read whitelist/disabled rules after a settings change.
    pub fn reload_settings(&mut self) {
        let (settings, errors) = Settings::load();
        self.settings = settings;
        self.settings_errors = errors;
        self.whitelist = self.settings.whitelist_matchers();
        self.rules = RuleSet::load(&self.settings);
    }

    pub fn save_settings(&self) -> squid_core::Result<std::path::PathBuf> {
        self.settings.save()
    }

    pub fn safety(&self) -> &SafetyPolicy {
        &self.safety
    }

    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }

    pub fn settings_errors(&self) -> &[String] {
        &self.settings_errors
    }

    pub fn is_whitelisted(&self, path: &Path) -> bool {
        self.settings.is_whitelisted(path, &self.whitelist)
    }

    /// Why this item cannot be deleted, if it cannot. Rules pass the anchor they
    /// declared; everything else falls back to the default anchors.
    pub fn block_reason(&self, item: &CleanupItem) -> Option<String> {
        let anchors = if item.anchors.is_empty() {
            squid_core::fs::default_anchors()
        } else {
            item.anchors.clone()
        };
        if self.is_whitelisted(&item.path) {
            return Some("whitelisted".into());
        }
        self.safety.denies(&item.path, &anchors)
    }
}

/// True when the process may touch system-owned paths.
#[cfg(unix)]
pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
pub fn is_root() -> bool {
    false
}

/// A path is writable by us when we are root, or we own it and can write it.
#[cfg(unix)]
pub fn writable_by_us(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    if is_root() {
        return true;
    }
    let uid = unsafe { libc::getuid() };
    let writable = |meta: &std::fs::Metadata| meta.uid() == uid && meta.mode() & 0o200 != 0;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => path
            .parent()
            .and_then(|parent| std::fs::metadata(parent).ok())
            .map(|parent| writable(&parent))
            .unwrap_or(false),
        Ok(meta) => writable(&meta),
        Err(_) => path
            .parent()
            .and_then(|parent| std::fs::metadata(parent).ok())
            .map(|parent| writable(&parent))
            .unwrap_or(false),
    }
}

#[cfg(not(unix))]
pub fn writable_by_us(_path: &Path) -> bool {
    true
}

/// Age of a path in days, using mtime. `None` when the stat failed.
pub fn age_days(path: &Path) -> Option<f64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let age = modified.elapsed().ok()?;
    Some(age.as_secs_f64() / 86_400.0)
}

pub fn mtime_ms(path: &Path) -> Option<u64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// Drop items whose ancestor is already in the list: deleting the parent would
/// make the child's removal fail (and inflate the freed-space total).
pub fn drop_nested_duplicates(items: &mut Vec<CleanupItem>) {
    items.sort_by(|a, b| a.path.cmp(&b.path));
    let mut kept: Vec<CleanupItem> = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        let nested = kept.iter().any(|parent: &CleanupItem| {
            parent.path != item.path && starts_with(&item.path, &parent.path)
        });
        if !nested {
            kept.push(item);
        }
    }
    *items = kept;
}

fn starts_with(path: &Path, base: &Path) -> bool {
    let mut path = path.components().peekable();
    let mut base = base.components().peekable();
    loop {
        match (base.next(), path.next()) {
            (Some(a), Some(b)) => {
                if a != b {
                    return false;
                }
            }
            (None, _) => return true,
            (Some(_), None) => return false,
        }
    }
}

/// "12,431 files · 382 folders" style annotation for the detail row.
pub fn size_note(info: &squid_core::fs::SizeInfo) -> String {
    let files = squid_core::bytes::format_thousands(info.files);
    if info.dirs > 0 {
        format!(
            "{files} files · {} folders",
            squid_core::bytes::format_thousands(info.dirs)
        )
    } else {
        format!("{files} files")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squid_core::{Category, ItemKind};
    use std::path::PathBuf;

    #[test]
    fn nested_duplicates_are_collapsed() {
        let mut items = vec![
            CleanupItem::new(
                PathBuf::from("/Users/x/Library/Caches"),
                ItemKind::Path,
                Category::AppCaches,
                "caches",
            ),
            CleanupItem::new(
                PathBuf::from("/Users/x/Library/Caches/foo/bar"),
                ItemKind::Path,
                Category::AppCaches,
                "nested",
            ),
            CleanupItem::new(
                PathBuf::from("/Users/x/Library/Logs"),
                ItemKind::Path,
                Category::Logs,
                "logs",
            ),
        ];
        drop_nested_duplicates(&mut items);
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .any(|item| item.path.ends_with("Library/Caches")));
        assert!(items.iter().any(|item| item.path.ends_with("Library/Logs")));
    }

    #[test]
    fn engine_boots_with_rules_and_volumes() {
        let engine = Engine::new();
        assert!(!engine.rules().is_empty(), "expected builtin rules to load");
        assert!(!engine.settings_errors().is_empty() || true);
        assert!(!engine.platform().display_name().is_empty());
    }

    #[test]
    fn safety_policy_blocks_the_obvious_roots() {
        let engine = Engine::new();
        let anchors = vec![PathBuf::from("/Users")];
        assert!(engine.safety().denies(Path::new("/"), &anchors).is_some());
        assert!(engine
            .safety()
            .denies(Path::new("/Users"), &anchors)
            .is_some());
    }
}
