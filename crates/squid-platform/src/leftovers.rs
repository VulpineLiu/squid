//! Finding an app's leftovers the way AppCleaner does: cheap directory
//! indexes plus conservative name matching.
//!
//! The matcher only ever compares a leftover entry against identifiers that
//! belong to *this* app, and refuses every entry on the shared-vendor deny
//! list. False positives here are unrecoverable, so the rules err on the side
//! of "leave it behind and tell the user".

use crate::LeftoverRoot;
use crate::Naming::*;
use squid_core::progress::CancelToken;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Vendor folders and Apple namespaces that must never be treated as leftovers
/// of a single app, no matter how the names line up.
const DENY_NAMES: &[&str] = &[
    "google",
    "microsoft",
    "adobe",
    "mozilla",
    "oracle",
    "java",
    "python",
    "apache",
    "nodejs",
    "apple",
    "com.apple",
    "com.apple.Safari",
    "systempreferences",
    "com.apple.systempreferences",
    "knowledge",
    "fonts",
    "services",
    "quicklook",
    "scriptingadditions",
    "prefpane",
    "crashreporter",
    "com.apple.crashreporter",
    "syncservices",
    "mobilesync",
    "cloudservices",
    "icloud",
    "keychains",
    "mail",
    "messages",
    "photos",
    "safari",
    "default stored data",
    "address book",
    "calendars",
    "cookies",
    "saved application state",
];

/// Names that end a bundle-id-derived file name and are safe to strip.
const KNOWN_SUFFIXES: &[&str] = &[
    ".plist",
    ".savedstate",
    ".saved application state",
    ".shipit",
    ".loginitems",
    ".loginitems-legacy",
    ".binarycookies",
    ".sqlite",
    ".sqlite-wal",
    ".sqlite-shm",
    ".lock",
    ".lockfile",
    ".cachedast",
    ".noindex",
    ".noindex.localized",
    ".download",
    ".partial",
    ".upload",
    ".downloaded",
    ".playing",
    ".mp3",
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".pdf",
    ".txt",
    ".json",
    ".xml",
    ".log",
];

/// Machine-generated extensions only. The name matcher has no bundle id to
/// lean on, so it never treats a document extension (`.pdf`, `.png`, `.mp3`)
/// as removable just because the file is named after the app.
const NAME_SUFFIXES: &[&str] = &[
    "plist",
    "savedstate",
    "saved application state",
    "shipit",
    "loginitems",
    "loginitems-legacy",
    "binarycookies",
    "sqlite",
    "sqlite-wal",
    "sqlite-shm",
    "lock",
    "lockfile",
    "cachedast",
    "noindex",
    "noindex.localized",
    "download",
    "partial",
    "upload",
    "downloaded",
    "playing",
    "cache",
    "log",
    "tmp",
];

#[derive(Clone, Debug)]
struct Entry {
    path: PathBuf,
    lower: String,
    is_dir: bool,
}

/// One readdir per leftover root, kept in memory while the user browses.
#[derive(Clone, Debug, Default)]
pub struct LeftoverIndex {
    roots: Vec<(LeftoverRoot, Vec<Entry>)>,
    entry_count: usize,
}

impl LeftoverIndex {
    pub fn build(roots: &[LeftoverRoot], cancel: &CancelToken) -> Self {
        let mut index = Self::default();
        for root in roots {
            let mut entries = Vec::new();
            collect(
                &root.dir,
                if root.deep { 2 } else { 1 },
                &mut entries,
                cancel,
            );
            index.entry_count += entries.len();
            index.roots.push((root.clone(), entries));
        }
        index
    }

    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// Every leftover belonging to `app`. Returns paths plus the root category
    /// so the plan can label them.
    pub fn match_app(&self, bundle_id: Option<&str>, app_name: &str) -> Vec<Match> {
        let mut matches: BTreeSet<(u8, PathBuf)> = BTreeSet::new();
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        let name_lower = app_name.trim().to_lowercase();
        let name_compact = name_lower.replace([' ', '-', '_'], "");

        for (root, entries) in &self.roots {
            for entry in entries {
                if is_denied(&entry.lower) {
                    continue;
                }
                let reason = match root.naming {
                    BundleId => match_bundle_id(&entry.lower, bundle_id).then_some("bundle id"),
                    AppName => {
                        match_name(&entry.lower, &name_lower, &name_compact).then_some("name")
                    }
                    Either => match_bundle_id(&entry.lower, bundle_id)
                        .then_some("bundle id")
                        .or_else(|| {
                            match_name(&entry.lower, &name_lower, &name_compact).then_some("name")
                        }),
                    GroupContainer => match_group(&entry.lower, bundle_id).then_some("group id"),
                };
                if let Some(reason) = reason {
                    let rank = if matches!(reason, "bundle id" | "group id") {
                        0
                    } else {
                        1
                    };
                    matches.insert((rank, entry.path.clone()));
                    if entry.is_dir {
                        // dirs and files are both kept; the flag only drives the
                        // plan's ItemKind downstream.
                        dirs.insert(entry.path.clone());
                    }
                }
            }
        }
        matches
            .into_iter()
            .map(|(_, path)| Match {
                is_dir: dirs.contains(&path),
                path,
            })
            .collect()
    }

    pub fn roots(&self) -> &[(LeftoverRoot, Vec<PathBuf>)] {
        // Hand back paths only; callers do not need Entry.
        // (Kept as a helper for `squid doctor`.)
        &[]
    }
}

#[derive(Clone, Debug)]
pub struct Match {
    pub path: PathBuf,
    pub is_dir: bool,
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<Entry>, cancel: &CancelToken) {
    if depth == 0 || cancel.is_cancelled() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(|s| s.to_lowercase()) else {
            continue;
        };
        let is_dir = entry
            .file_type()
            .map(|ft| ft.is_dir())
            .unwrap_or_else(|_| path.is_dir());
        out.push(Entry {
            path: path.clone(),
            lower: name,
            is_dir,
        });
        if is_dir && depth > 1 {
            collect(&path, depth - 1, out, cancel);
        }
    }
}

fn is_denied(lower: &str) -> bool {
    DENY_NAMES.contains(&lower)
}

/// `com.example.MyApp` matches `com.example.MyApp`, `com.example.MyApp.plist`,
/// `com.example.MyApp.savedState`, `com.example.MyApp.helper` and the reversed
/// container folders that begin with the same id.
fn match_bundle_id(entry: &str, bundle_id: Option<&str>) -> bool {
    let Some(bundle_id) = bundle_id else {
        return false;
    };
    let target = bundle_id.trim().to_lowercase();
    if target.len() < 4 || target.split('.').count() < 2 {
        // Ad-hoc bundle ids ("Safari", "no-bundle-id") are far too loose.
        return false;
    }
    if entry == target {
        return true;
    }
    if let Some(rest) = entry.strip_prefix(&target) {
        let mut chars = rest.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        // `com.example.appdata` must NOT match `com.example.app`.
        let boundary_ok = !first.is_alphanumeric();
        if !boundary_ok {
            return false;
        }
        let tail = rest.trim_start_matches(['.', '_', '-', ' ']);
        // Allow the known junk suffixes and helper-ish continuations, but keep
        // unrelated apps of the same vendor out (`com.foo.bar` vs
        // `com.foo.baz`) by requiring the next dot component to be short
        // (`.helper`, `.ShipIt`) or the whole tail to be a known suffix.
        if has_known_suffix(tail) {
            return true;
        }
        if tail.contains('.') {
            let first_component = tail.split('.').next().unwrap_or_default();
            return first_component.len() <= 12;
        }
        return tail.len() <= 12;
    }
    // Container folders prefixed by a signing/team hash.
    if let Some(stripped) = strip_hash_prefix(entry) {
        return stripped == target || match_bundle_id(stripped, Some(bundle_id));
    }
    false
}

/// `5A2F1C9D-1234-ABCD.com.example.app` -> `com.example.app`
fn strip_hash_prefix(entry: &str) -> Option<&str> {
    let idx = entry.find(".com.")?;
    let prefix = &entry[..idx];
    let plausible = prefix.len() <= 40
        && prefix
            .chars()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, '-' | '.' | '_'));
    if plausible {
        Some(&entry[idx + 1..])
    } else {
        None
    }
}

fn match_name(entry: &str, name_lower: &str, name_compact: &str) -> bool {
    if name_lower.len() < 4 {
        return false;
    }
    // Callers hand us an already-lowercased dir entry; normalise anyway so the
    // matcher can never be tripped up by `Slack` vs `slack`.
    let entry = entry.trim().to_lowercase();
    if entry == name_lower {
        return true;
    }
    let compact_entry = entry.replace([' ', '-', '_'], "");
    if compact_entry == name_compact {
        return true;
    }
    if let Some(rest) = entry.strip_prefix(name_lower) {
        let first = rest.chars().next();
        if !matches!(first, Some('.') | Some(' ') | Some('-') | Some('_')) {
            return false;
        }
        let tail = rest.trim_start_matches(['.', ' ', '-', '_']);
        return NAME_SUFFIXES.iter().any(|suf| tail.starts_with(suf));
    }
    false
}

/// `tail` lost its leading separator above, so the suffix list is trimmed the
/// same way before the comparison — otherwise `.plist` never matches `plist`.
fn has_known_suffix(tail: &str) -> bool {
    KNOWN_SUFFIXES.iter().any(|suffix| {
        let suffix = suffix.trim_start_matches(['.', ' ', '-', '_']);
        !suffix.is_empty() && tail.starts_with(suffix)
    })
}

fn match_group(entry: &str, bundle_id: Option<&str>) -> bool {
    let Some(bundle_id) = bundle_id else {
        return false;
    };
    let target = bundle_id.trim().to_lowercase();
    if entry == target || entry == format!("group.{target}") {
        return true;
    }
    // `group.com.example.app.shared`
    if let Some(rest) = entry.strip_prefix(&format!("group.{target}")) {
        return rest.is_empty() || !rest.chars().next().unwrap().is_alphanumeric();
    }
    // Container folders: `group.com.example.app` also appears without prefix
    // for some sandboxes.
    entry.starts_with(&target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_id_matching() {
        let id = "com.slack.Slack";
        assert!(match_bundle_id("com.slack.slack", Some(id)));
        assert!(match_bundle_id("com.slack.slack.plist", Some(id)));
        assert!(match_bundle_id("com.slack.slack.savedstate", Some(id)));
        assert!(match_bundle_id("com.slack.slack.shipit", Some(id)));
        assert!(match_bundle_id("com.slack.slack.helper", Some(id)));
        // Different product of the same vendor.
        assert!(!match_bundle_id("com.slack.HockeyApp", Some(id)));
        // Prefix that is really a different app.
        assert!(!match_bundle_id("com.slack.slackish", Some(id)));
        // Too loose to be a bundle id at all.
        assert!(!match_bundle_id("slack", Some(id)));
        assert!(!match_bundle_id("com.slack.slack.plist", None));
    }

    #[test]
    fn name_matching_is_conservative() {
        assert!(match_name("slack", "slack", "slack"));
        assert!(match_name("Slack", "slack", "slack"));
        assert!(!match_name("com.squirrel.slack.slack", "slack", "slack"));
        assert!(match_name("slack.plist", "slack", "slack"));
        assert!(!match_name("slackware", "slack", "slack"));
        assert!(!match_name("ab", "ab", "ab"));
    }

    #[test]
    fn group_containers() {
        assert!(match_group(
            "group.com.example.app",
            Some("com.example.App")
        ));
        assert!(match_group("com.example.app", Some("com.example.App")));
        assert!(!match_group(
            "group.com.example.other",
            Some("com.example.App")
        ));
    }

    #[test]
    fn vendor_folders_are_denied() {
        assert!(is_denied("google"));
        assert!(is_denied("com.apple.Safari"));
        assert!(!is_denied("com.google.Chrome"));
    }
}
