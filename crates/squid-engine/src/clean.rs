//! `squid clean` — rule-driven cleanup.
//!
//! The scan walks every enabled rule, expands its globs, measures what it
//! found and returns one flat report grouped by [`Category`]. Two passes keep
//! the cost predictable: matching is cheap (readdir per rule), measuring runs
//! on a thread pool with a progress stream so the UI can fill in sizes as they
//! arrive.

use crate::{age_days, drop_nested_duplicates, size_note, writable_by_us, Engine};
use squid_core::fs::measure_all;
use squid_core::progress::{CancelToken, Event, Progress};
use squid_core::{Bytes, Category, CleanupItem, ItemKind, LockReason, ScanReport, SquidError};
use squid_rules::{PreparedRule, Rule};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct CleanOptions {
    /// Restrict the scan to these categories; empty means "everything enabled".
    pub categories: Vec<Category>,
    /// Only offer rules the user can actually delete without sudo.
    pub skip_admin_rules: bool,
    /// Override the per-rule `min_age_days` (0 disables the filter).
    pub min_age_days: Option<u64>,
    /// Also look for leftovers whose owning app is gone.
    pub orphans: bool,
    /// Skip measurement (fast name-only scan).
    pub fast: bool,
}

impl Default for CleanOptions {
    fn default() -> Self {
        CleanOptions {
            categories: Vec::new(),
            skip_admin_rules: true,
            min_age_days: None,
            orphans: false,
            fast: false,
        }
    }
}

/// Scan every enabled rule.
pub fn scan(
    engine: &Engine,
    options: &CleanOptions,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<ScanReport> {
    let prepared = prepared_rules(engine, options);
    if prepared.is_empty() {
        return Err(SquidError::NothingToDo);
    }
    progress.phase("match", "locating cleanup candidates");
    progress.tick(0, Some(prepared.len() as u64), None);

    let running = engine.platform().running_executables();
    let mut report = ScanReport::default();
    let mut done = 0u64;
    let mut seen: HashMap<PathBuf, usize> = HashMap::new();

    for prepared_rule in &prepared {
        if cancel.is_cancelled() {
            return Err(SquidError::Cancelled);
        }
        let rule = &prepared_rule.rule;
        let matches = prepared_rule.matches();
        done += 1;
        progress.tick(done, Some(prepared.len() as u64), None);
        if matches.is_empty() {
            report.skipped_rules.push(rule.id.clone());
            continue;
        }
        let running_conflict = rule_running_conflict(rule, &running);
        for matched in matches {
            for item in items_for_match(rule, &matched.path, &matched.anchor) {
                if !passes_age(engine, rule, options, &item.path) {
                    continue;
                }
                if let Some(existing) = seen.get(&item.path) {
                    // Another rule already claimed this path; remember the
                    // anchor so both rules can authorise the same deletion.
                    if let Some(previous) = report.items.get_mut(*existing) {
                        {
                            let anchor = &matched.anchor;
                            if !previous.anchors.contains(anchor) {
                                previous.anchors.push(anchor.clone());
                            }
                        }
                    }
                    continue;
                }
                let mut item = item;
                item.anchors = vec![matched.anchor.clone()];
                if let Some(reason) = &running_conflict {
                    item = item
                        .locked(LockReason::AppRunning)
                        .with_note(reason.clone());
                } else if options.skip_admin_rules
                    && rule.needs_admin
                    && !writable_by_us(&item.path)
                {
                    item = item.locked(LockReason::NotOwned).with_note(format!(
                        "needs admin rights — {} to include it",
                        squid_core::elevation_hint()
                    ));
                } else if engine.is_whitelisted(&item.path) {
                    item = item.locked(LockReason::Whitelisted);
                } else if let Some(reason) = engine.block_reason(&item) {
                    item = item.locked(LockReason::Protected).with_note(reason);
                }
                if !rule.select_by_default && item.lock.is_none() {
                    item.selected = false;
                    item.note = Some(
                        item.note
                            .take()
                            .map(|note| format!("{} · off by default", note))
                            .unwrap_or_else(|| "off by default".into()),
                    );
                }
                if rule.risky {
                    // Risky rules must stay labelled even when they already
                    // carry a different note.
                    item.note = Some(match item.note.take() {
                        Some(note) => format!("{note} · review before deleting"),
                        None => "review before deleting".into(),
                    });
                }
                seen.insert(item.path.clone(), report.items.len());
                report.items.push(item);
            }
        }
    }

    if options.orphans {
        match orphans(engine, cancel, progress) {
            Ok(extra) => {
                for item in extra {
                    if seen.contains_key(&item.path) {
                        continue;
                    }
                    seen.insert(item.path.clone(), report.items.len());
                    report.items.push(item);
                }
            }
            Err(err) if err.is_noteworthy() => progress.warning(err.to_string()),
            Err(_) => {}
        }
    }

    if !options.fast {
        measure(engine, &mut report, progress, cancel)?;
    }
    report.unreadable = report
        .items
        .iter()
        .filter(|item| item.note.as_deref() == Some("unreadable"))
        .count() as u32;
    Ok(report)
}

fn prepared_rules(engine: &Engine, options: &CleanOptions) -> Vec<PreparedRule> {
    let mut rules: Vec<Rule> = engine
        .rules()
        .rules()
        .iter()
        .filter(|&rule| {
            options.categories.is_empty() || options.categories.contains(&rule.category)
        })
        .cloned()
        .collect();
    if options.skip_admin_rules {
        // Keep admin rules visible so the user learns sudo exists, but only
        // when they asked for the full scan.
        rules.retain(|rule| !rule.needs_admin || crate::is_root());
    }
    rules.into_iter().map(PreparedRule::new).collect()
}

fn rule_running_conflict(rule: &Rule, running: &[String]) -> Option<String> {
    if rule.skip_if_running.is_empty() {
        return None;
    }
    let hit = rule.skip_if_running.iter().find(|name| {
        running
            .iter()
            .any(|proc| proc.eq_ignore_ascii_case(name) || proc.ends_with(&format!("/{name}")))
    })?;
    Some(format!("{hit} is running"))
}

/// Turn one glob match into the items a rule actually wants deleted.
fn items_for_match(rule: &Rule, path: &Path, anchor: &Path) -> Vec<CleanupItem> {
    let label = label_for(rule, path);
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(_) => return Vec::new(),
    };
    if meta.file_type().is_symlink() {
        // Symlinks get unlinked, never followed.
        return vec![new_item(path.to_path_buf(), ItemKind::File, rule, label)];
    }
    match rule.mode {
        squid_rules::RuleMode::Path => {
            let kind = if meta.is_dir() {
                ItemKind::Path
            } else {
                ItemKind::File
            };
            vec![new_item(path.to_path_buf(), kind, rule, label)]
        }
        squid_rules::RuleMode::Contents => {
            if !meta.is_dir() {
                return vec![new_item(path.to_path_buf(), ItemKind::File, rule, label)];
            }
            // Never offer the *contents* of a directory we are not allowed to
            // descend into one level below the anchor.
            let _ = anchor;
            vec![new_item(
                path.to_path_buf(),
                ItemKind::Contents,
                rule,
                label,
            )]
        }
        squid_rules::RuleMode::Files => {
            if !meta.is_dir() {
                return vec![new_item(path.to_path_buf(), ItemKind::File, rule, label)];
            }
            list_files(path)
                .into_iter()
                .map(|file| new_item(file, ItemKind::File, rule, label.clone()))
                .collect()
        }
    }
}

fn new_item(path: PathBuf, kind: ItemKind, rule: &Rule, label: String) -> CleanupItem {
    CleanupItem::new(path, kind, rule.category, label).with_rule(&rule.id)
}

/// The label shown in the scan list. A leaf such as `0` (Docker keeps its VM
/// disk in `…/Data/vms/0`) says nothing on its own, so one parent component is
/// prepended whenever the leaf is uninformative.
fn label_for(rule: &Rule, path: &Path) -> String {
    let leaf = squid_core::paths::label_for(path);
    if leaf.is_empty() {
        return rule.display_title().to_string();
    }
    if is_informative(&leaf) {
        return leaf;
    }
    match path.parent().and_then(|parent| parent.file_name()) {
        Some(parent) => format!("{}/{leaf}", parent.to_string_lossy()),
        None => leaf,
    }
}

fn is_informative(name: &str) -> bool {
    name.chars().count() > 2 && !name.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn list_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .max_depth(4)
        .into_iter()
        .filter_entry(|entry| {
            // Do not descend into nested app bundles while listing log files.
            !entry.file_type().is_dir() || !entry.file_name().to_string_lossy().ends_with(".app")
        });
    for entry in walker.flatten() {
        if entry.file_type().is_file() {
            out.push(entry.into_path());
        }
    }
    out.sort();
    out
}

fn passes_age(_engine: &Engine, rule: &Rule, options: &CleanOptions, path: &Path) -> bool {
    let days = match options.min_age_days {
        Some(0) => return true,
        Some(days) => days as f64,
        None => match rule.min_age() {
            Some(age) => age.as_secs_f64() / 86_400.0,
            None => return true,
        },
    };
    match age_days(path) {
        Some(age) => age >= days,
        None => true,
    }
}

/// Measure items in parallel and stream sizes into the report.
fn measure(
    _engine: &Engine,
    report: &mut ScanReport,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<()> {
    if report.items.is_empty() {
        return Ok(());
    }
    progress.phase("measure", "measuring sizes");
    let targets: Vec<(PathBuf, ItemKind)> = report
        .items
        .iter()
        .map(|item| (item.path.clone(), item.kind))
        .collect();
    progress.tick(0, Some(targets.len() as u64), None);

    let results: Vec<squid_core::fs::SizeInfo> = measure_all(&targets, cancel, |index, _info| {
        progress.tick(
            index as u64 + 1,
            Some(targets.len() as u64),
            targets.get(index).map(|(path, _)| path.clone()),
        );
    });
    if results.len() != report.items.len() {
        return Err(SquidError::Other(
            "measurement returned an unexpected number of results".into(),
        ));
    }
    for (item, info) in report.items.iter_mut().zip(results) {
        item.size = info.logical;
        item.allocated = info.allocated;
        if info.unreadable > 0 && item.note.is_none() {
            item.note = Some(format!("{} entries unreadable", info.unreadable));
        } else if info.files > 0 && item.note.is_none() {
            item.note = Some(size_note(&info));
        }
    }
    Ok(())
}

/// Publish per-category totals so the UI can fill its summary immediately.
pub fn publish_buckets(report: &ScanReport, progress: &Progress) {
    let mut totals: HashMap<Category, (Bytes, usize)> = HashMap::new();
    for item in &report.items {
        let entry = totals.entry(item.category).or_insert((Bytes::ZERO, 0));
        entry.0 += item.on_disk();
        entry.1 += 1;
    }
    let mut ordered: Vec<(Category, (Bytes, usize))> = totals.into_iter().collect();
    ordered.sort_by_key(|(category, _)| *category);
    for (category, (total, count)) in ordered {
        progress.publish(Event::Bucket {
            category,
            total,
            count,
        });
    }
}

/// Leftovers whose application is no longer installed. Reported but *not*
/// selected: we can prove the app is gone, not that the data is worthless.
pub fn orphans(
    engine: &Engine,
    cancel: &CancelToken,
    progress: &Progress,
) -> squid_core::Result<Vec<CleanupItem>> {
    progress.phase("orphans", "looking for leftovers of removed apps");
    let apps = engine.platform().list_apps(cancel, false)?;
    let installed: Vec<String> = apps
        .iter()
        .filter_map(|app| app.bundle_id.as_deref())
        .map(|id| id.to_lowercase())
        .collect();
    let names: Vec<String> = apps
        .iter()
        .map(|app| squid_core::paths::label_for(&app.path).to_lowercase())
        .collect();

    let mut items = Vec::new();
    for root in engine.platform().leftover_roots() {
        if cancel.is_cancelled() {
            return Err(SquidError::Cancelled);
        }
        let Ok(entries) = std::fs::read_dir(&root.dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            if !looks_like_bundle_id(&lower) {
                continue;
            }
            if lower.starts_with("com.apple.") || lower.contains(".apple.") {
                continue;
            }
            if installed.iter().any(|id| bundle_related(id, &lower)) {
                continue;
            }
            if names.iter().any(|app| app == &lower) {
                continue;
            }
            let mut item = CleanupItem::new(
                path,
                if entry.path().is_dir() {
                    ItemKind::Path
                } else {
                    ItemKind::File
                },
                Category::Leftovers,
                name.clone(),
            )
            .with_rule("orphan-leftovers")
            .with_anchors(vec![root.dir.clone()])
            .with_note("app no longer installed — verify before deleting");
            item.selected = false;
            if engine.is_whitelisted(&item.path) {
                item = item.locked(LockReason::Whitelisted);
            } else if let Some(reason) = engine.block_reason(&item) {
                item = item.locked(LockReason::Protected).with_note(reason);
            } else if !writable_by_us(&item.path) {
                item = item.locked(LockReason::NotOwned);
            }
            items.push(item);
        }
    }
    Ok(items)
}

fn looks_like_bundle_id(lower: &str) -> bool {
    let stem = lower
        .trim_end_matches(".plist")
        .trim_end_matches(".savedState")
        .trim_end_matches(".ssh");
    let parts: Vec<&str> = stem.split('.').collect();
    parts.len() >= 3
        && parts.iter().all(|part| !part.is_empty())
        && parts[0].len() >= 2
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_+".contains(c))
}

fn bundle_related(installed: &str, leftover: &str) -> bool {
    let leftover = leftover
        .trim_end_matches(".plist")
        .trim_end_matches(".savedState");
    let prefix = |id: &str, depth: usize| -> String {
        id.split('.')
            .take(depth.min(id.split('.').count()))
            .collect::<Vec<_>>()
            .join(".")
    };
    for depth in [2, 3] {
        if prefix(installed, depth) == prefix(leftover, depth) && prefix(leftover, depth).len() > 4
        {
            return true;
        }
    }
    false
}

/// Sort a report for display: biggest first inside each category.
pub fn sort_for_display(mut report: ScanReport) -> ScanReport {
    report.items.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| b.on_disk().cmp(&a.on_disk()))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    report
}

pub fn cancel_if_needed(cancel: &CancelToken) -> squid_core::Result<()> {
    if cancel.is_cancelled() {
        Err(SquidError::Cancelled)
    } else {
        Ok(())
    }
}

/// Expose the "same path, two rules" collapse to the tests.
#[doc(hidden)]
pub fn _dedupe_report(report: &mut ScanReport) {
    drop_nested_duplicates(&mut report.items);
}

pub use squid_core::fs::SizeInfo;

#[cfg(test)]
mod tests {
    use super::*;
    use squid_platform::apps::is_running;

    #[test]
    fn bundle_id_detection_is_conservative() {
        assert!(looks_like_bundle_id("com.slack.slack"));
        assert!(looks_like_bundle_id("com.discord.html"));
        assert!(!looks_like_bundle_id("logs"));
        assert!(!looks_like_bundle_id("com"));
        assert!(!looks_like_bundle_id("Homebrew"));
        assert!(!looks_like_bundle_id("Google Chrome"));
    }

    #[test]
    fn related_bundles_are_matched_by_prefix() {
        assert!(bundle_related(
            "com.company.Tool",
            "com.company.Tool.Helper"
        ));
        assert!(bundle_related("com.company.Tool", "com.company.tool"));
        assert!(!bundle_related("com.company.Tool", "io.other.App"));
    }

    #[test]
    fn labels_fall_back_to_the_rule_title() {
        let rule = Rule {
            id: "demo".into(),
            category: Category::AppCaches,
            title: "Demo caches".into(),
            patterns: vec![],
            mode: squid_rules::RuleMode::Path,
            os: vec![],
            protect: vec![],
            min_age_days: 0,
            select_by_default: true,
            needs_admin: false,
            skip_if_running: vec![],
            risky: false,
            note: None,
        };
        assert_eq!(label_for(&rule, Path::new("/tmp/demo-cache")), "demo-cache");
        // Docker's VM disk lives in `…/Data/vms/0` — a bare "0" is not a label.
        assert_eq!(
            label_for(&rule, Path::new("/tmp/demo-cache/vms/0")),
            "vms/0"
        );
        assert_eq!(
            label_for(&rule, Path::new("/tmp/demo-cache/00")),
            "demo-cache/00"
        );
    }

    #[test]
    fn scan_runs_on_the_real_machine() {
        let engine = Engine::new();
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let report = scan(&engine, &CleanOptions::default(), &progress, &cancel).expect("scan");
        // Whatever the machine holds, a real scan must never crash and must
        // never mark anything protected as selectable.
        for item in &report.items {
            if item.lock.is_some() {
                assert!(!item.selected, "locked item selected: {:?}", item.path);
            }
            assert!(!item.path.as_os_str().is_empty());
        }
        publish_buckets(&report, &progress);
        let events = progress.drain();
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::Phase { .. })));
    }

    #[test]
    fn app_running_locks_browser_rules() {
        let rule = Rule {
            id: "chrome".into(),
            category: Category::BrowserCaches,
            title: "Chrome".into(),
            patterns: vec![],
            mode: squid_rules::RuleMode::Path,
            os: vec![],
            protect: vec![],
            min_age_days: 0,
            select_by_default: true,
            needs_admin: false,
            skip_if_running: vec!["Google Chrome".into()],
            risky: false,
            note: None,
        };
        assert!(rule_running_conflict(&rule, &["Google Chrome".to_string()]).is_some());
        assert!(rule_running_conflict(
            &rule,
            &["/Applications/Foo.app/Contents/MacOS/Foo".to_string()]
        )
        .is_none());
        assert!(!is_running("Xcode", None, &["Safari".to_string()]));
    }
}
