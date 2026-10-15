//! `squid uninstall` — AppCleaner-style "app plus everything it left behind".
//!
//! The plan is built from two sources: the `.app` bundle itself and a readdir
//! index of the per-app leftover roots (`~/Library/Preferences`,
//! `~/Library/Containers`, `/Library/LaunchDaemons`, …). Matching is by bundle
//! id first — names are ambiguous, reverse-DNS identifiers are not.
//!
//! Nothing is deleted here. [`remove_plan`] hands the approved items to
//! [`crate::delete`], which re-validates every single path.

use crate::delete::{self, DeleteJob, DeleteOutcome};
use crate::Engine;
use squid_core::progress::{CancelToken, Progress};
use squid_core::{
    AppInfo, Bytes, Category, CleanupItem, DeleteMode, ItemKind, LockReason, UninstallPlan,
};
use squid_platform::apps::read_bundle;
use squid_platform::leftovers::LeftoverIndex;
use squid_platform::Naming;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Read the installed-app inventory.
pub fn scan_apps(
    engine: &Engine,
    size_apps: bool,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<Vec<AppInfo>> {
    progress.phase("apps", "reading the application inventory");
    let apps = engine.platform().list_apps(cancel, size_apps)?;
    progress.note(format!("{} applications found", apps.len()));
    Ok(apps)
}

/// Look apps up by name, bundle id or `.app` path.
pub fn find(
    engine: &Engine,
    query: &str,
    cancel: &CancelToken,
    progress: &Progress,
) -> squid_core::Result<Vec<AppInfo>> {
    let needle = query.trim();
    if needle.is_empty() {
        return Err(squid_core::SquidError::NothingToDo);
    }
    let mut apps = scan_apps(engine, false, progress, cancel)?;

    // A literal bundle path beats fuzzy matching: `squid uninstall ./Foo.app`.
    let as_path = squid_core::paths::expand(needle);
    if as_path.extension().map(|e| e == "app").unwrap_or(false) && as_path.is_dir() {
        let bundle = read_bundle(&as_path).unwrap_or_default();
        let running = engine.platform().running_executables();
        if let Some(mut app) =
            squid_platform::apps::app_from_bundle(as_path.clone(), "Path", &running, false)
        {
            app.bundle_id = bundle.bundle_id.or(app.bundle_id);
            return Ok(vec![app]);
        }
    }

    let needle_lower = needle.to_lowercase();
    let needle_compact = needle_lower.replace([' ', '-', '_'], "");
    apps.retain(|app| {
        let name = app.name.to_lowercase();
        let stem = squid_platform::apps::stem_lower(&app.path);
        let id = app.bundle_id.as_deref().unwrap_or_default().to_lowercase();
        name == needle_lower
            || stem == needle_lower
            || id == needle_lower
            || name.contains(&needle_lower)
            || stem.contains(&needle_lower)
            || id.contains(&needle_lower)
            || name.replace([' ', '-', '_'], "").contains(&needle_compact)
    });
    apps.sort_by_key(|app| {
        let name = app.name.to_lowercase();
        match (
            name == needle_lower,
            name.starts_with(&needle_lower),
            name.contains(&needle_lower),
        ) {
            (true, _, _) => 0,
            (_, true, _) => 1,
            (_, _, true) => 2,
            _ => 3,
        }
    });
    Ok(apps)
}

/// Everything that would be removed for `app`, with locks already applied.
pub fn build_plan(
    engine: &Engine,
    app: &AppInfo,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<UninstallPlan> {
    progress.phase("plan", format!("looking for {} leftovers", app.name));
    let mut plan = UninstallPlan {
        app: app.clone(),
        items: Vec::new(),
        warnings: Vec::new(),
    };

    let system = is_system_app(app);
    if system {
        plan.warnings.push(
            "this app ships with macOS (SIP-protected) — squid will refuse to delete it"
                .to_string(),
        );
    }
    if app.running {
        plan.warnings.push(format!(
            "{} is running — quit it before uninstalling",
            app.name
        ));
    }
    if app.has_sibling_install {
        plan.warnings.push(
            "another copy of this bundle id is installed; shared preference files stay behind"
                .to_string(),
        );
    }

    // 1. The bundle itself.
    let mut bundle_item = CleanupItem::new(
        app.path.clone(),
        ItemKind::Path,
        Category::Leftovers,
        app.name.clone(),
    )
    .with_rule("uninstall-bundle")
    .with_anchors(bundle_anchors(&app.path));
    bundle_item.note = Some(match app.version.as_deref() {
        Some(version) => format!("{} · {}", app.source, version),
        None => app.source.clone(),
    });
    if system {
        bundle_item = bundle_item
            .locked(LockReason::Protected)
            .with_note("system application");
    } else if app.running {
        bundle_item = bundle_item
            .locked(LockReason::AppRunning)
            .with_note("quit the app first");
    } else if !crate::writable_by_us(&app.path) && !crate::is_root() {
        bundle_item = bundle_item.locked(LockReason::NotOwned).with_note(format!(
            "{} to remove this copy",
            squid_core::elevation_hint()
        ));
    }
    plan.push(bundle_item);

    // 2. Leftovers. One readdir pass per root, reused for every match.
    let roots = engine.platform().leftover_roots();
    let index = LeftoverIndex::build(&roots, cancel);
    progress.note(format!(
        "indexed {} leftover candidates for {}",
        index.entry_count(),
        app.name
    ));
    let categories = category_index(&roots);
    for matched in index.match_app(app.bundle_id.as_deref(), &app.name) {
        if skip_leftover(&matched.path, &app.path) {
            continue;
        }
        let kind = if matched.is_dir {
            ItemKind::Path
        } else {
            ItemKind::File
        };
        let anchor = anchor_for(&matched.path, &roots);
        let category = categories
            .iter()
            .find(|(dir, _)| matched.path.starts_with(dir))
            .map(|(_, category)| *category)
            .unwrap_or(Category::Leftovers);
        let mut item = CleanupItem::new(
            matched.path.clone(),
            kind,
            category,
            describe(&matched.path),
        )
        .with_rule("uninstall-leftovers")
        .with_anchors(anchor);
        if let Some(root) = roots
            .iter()
            .find(|root| matched.path.starts_with(&root.dir) && root.naming == Naming::BundleId)
        {
            let _ = root;
            item.note = Some("matched by bundle id".into());
        } else if let Some(id) = app.bundle_id.as_deref() {
            let _ = id;
            item.note = Some("matched by name".into());
        }
        if system {
            item = item
                .locked(LockReason::Protected)
                .with_note("system application");
        } else if app.running {
            item = item
                .locked(LockReason::AppRunning)
                .with_note("quit the app first");
        } else if app.has_sibling_install {
            item = item
                .locked(LockReason::SharedWithOtherInstall)
                .with_note("another install still uses this file");
        }
        if let Some(kind) = document_store(&matched.path) {
            item.note = Some(match kind {
                Store::Container => "sandboxed container — may hold saved documents".into(),
                Store::AppSupport => "application support — may hold saved documents".into(),
            });
            plan.warnings.push(format!(
                "{} may contain documents you created",
                squid_core::paths::pretty(&matched.path)
            ));
        }
        if let Some(reason) = engine.block_reason(&item) {
            item = item.locked(LockReason::Protected).with_note(reason);
        }
        plan.push(item);
    }

    measure(&mut plan, progress, cancel)?;
    plan.items.sort_by(|a, b| {
        b.on_disk()
            .cmp(&a.on_disk())
            .then_with(|| a.path.cmp(&b.path))
    });
    let unique: Vec<String> = std::mem::take(&mut plan.warnings)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    plan.warnings = unique;
    Ok(plan)
}

fn measure(
    plan: &mut UninstallPlan,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<()> {
    let targets: Vec<(PathBuf, ItemKind)> = plan
        .items
        .iter()
        .map(|item| (item.path.clone(), item.kind))
        .collect();
    let sizes = squid_core::fs::measure_all(&targets, cancel, |index, _info| {
        progress.tick(
            index as u64 + 1,
            Some(targets.len() as u64),
            targets.get(index).map(|(path, _)| path.clone()),
        );
    });
    for (item, info) in plan.items.iter_mut().zip(sizes) {
        item.size = info.logical;
        item.allocated = info.allocated;
        if info.unreadable > 0 {
            item.note = Some(format!("{} entries unreadable", info.unreadable));
        }
    }
    Ok(())
}

/// Delete an approved plan, running the platform lifecycle hooks around it.
pub fn remove_plan(
    engine: &Engine,
    plan: &UninstallPlan,
    mode: DeleteMode,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<DeleteOutcome> {
    let prepared = engine.platform().prepare_uninstall(&plan.app);
    for note in &prepared.notes {
        progress.note(note.clone());
    }
    for warning in prepared.warnings.iter().chain(plan.warnings.iter()) {
        progress.warning(warning.clone());
    }
    if !prepared.stopped_services.is_empty() {
        progress.note(format!(
            "stopped {} background service(s)",
            prepared.stopped_services.len()
        ));
    }

    let job = DeleteJob::new(plan.items.clone(), mode, "squid uninstall");
    let outcome = delete::execute(engine, &job, progress, cancel)?;

    let removed: Vec<PathBuf> = outcome
        .summary
        .session_id
        .as_deref()
        .and_then(squid_core::ledger::load_session)
        .map(|session| session.entries.into_iter().map(|e| e.path).collect())
        .unwrap_or_default();
    let finished = engine.platform().finish_uninstall(&plan.app, &removed);
    for note in finished.notes {
        progress.note(note);
    }
    for warning in finished.warnings {
        progress.warning(warning);
    }
    Ok(outcome)
}

/// The safety anchor for the bundle itself.
///
/// [`squid_core::fs::default_anchors`] deliberately has no `/Applications`
/// entry — it is a list of folders squid is *allowed to clean inside of*, and
/// an application folder is not one of them. Reusing it for the bundle made
/// every uninstall skip the `.app` with "outside every declared cleanup
/// anchor" while happily deleting the leftovers, which is the opposite of what
/// the user approved. The honest anchor is the folder that holds this one
/// bundle: nothing deeper, nothing wider. `/Applications` itself stays on the
/// hard-deny list, so the anchor can never be used to remove the folder.
fn bundle_anchors(bundle: &Path) -> Vec<PathBuf> {
    match bundle.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => vec![parent.to_path_buf()],
        _ => Vec::new(),
    }
}

/// A leftover path we refuse to offer: inside the bundle, an ancestor of it,
/// or a symlink pointing back into the bundle.
fn skip_leftover(path: &Path, bundle: &Path) -> bool {
    if path == bundle {
        return true;
    }
    if path.starts_with(bundle) || bundle.starts_with(path) {
        return true;
    }
    // `~/Library/Preferences/ByHost` and friends can hold folder placeholders.
    let leaf = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if leaf.is_empty() || leaf == "." || leaf == ".." {
        return true;
    }
    false
}

/// Prefer the tightest leftover root as the safety anchor, so the policy sees
/// `~/Library/Preferences` rather than `/`.
fn anchor_for(path: &Path, roots: &[squid_platform::LeftoverRoot]) -> Vec<PathBuf> {
    let mut best: Option<PathBuf> = None;
    for root in roots {
        if path.starts_with(&root.dir) {
            let better = match &best {
                Some(current) => root.dir.components().count() > current.components().count(),
                None => true,
            };
            if better {
                best = Some(root.dir.clone());
            }
        }
    }
    match best {
        Some(dir) => vec![dir],
        None => squid_core::fs::default_anchors(),
    }
}

fn category_index(roots: &[squid_platform::LeftoverRoot]) -> Vec<(PathBuf, Category)> {
    roots
        .iter()
        .map(|root| {
            // Trailing components, case-insensitively: `ends_with` on the
            // rendered path string only holds on whichever OS wrote the
            // separator, and quietly stopped classifying on the other one.
            let name = root
                .dir
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let parent_is_library = root
                .dir
                .parent()
                .and_then(|parent| parent.file_name())
                .map(|name| name.to_string_lossy().eq_ignore_ascii_case("library"))
                .unwrap_or(false);
            let category = if name == ".cache" || (name == "caches" && parent_is_library) {
                Category::AppCaches
            } else if name == "logs" && parent_is_library {
                Category::Logs
            } else {
                root.category
            };
            (root.dir.clone(), category)
        })
        .collect()
}

/// Folders that can hold a document the user made, not just program state.
/// Split so the warning says *which* kind of folder it is — calling
/// `~/Library/Application Support/Foo` a "sandboxed container" is just wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Store {
    Container,
    AppSupport,
}

fn document_store(path: &Path) -> Option<Store> {
    let text = path.to_string_lossy();
    if text.contains("/Library/Containers/") || text.contains("/Library/Group Containers/") {
        Some(Store::Container)
    } else if text.contains("/Library/Application Support/") {
        Some(Store::AppSupport)
    } else {
        None
    }
}

/// Short human label for a leftover: "Preferences/com.foo.plist".
fn describe(path: &Path) -> String {
    let components: Vec<String> = path
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if components.len() == 2 {
        format!("{}/{}", components[1], components[0])
    } else {
        components
            .first()
            .cloned()
            .unwrap_or_else(|| squid_core::paths::pretty(path))
    }
}

pub fn is_system_app(app: &AppInfo) -> bool {
    app.path.starts_with("/System")
        || app.source.eq_ignore_ascii_case("macOS Apps")
        || app.name.eq_ignore_ascii_case("Safari")
}

/// Total space a plan would reclaim, preferring the on-disk figure.
pub fn plan_size(plan: &UninstallPlan) -> Bytes {
    plan.items.iter().map(|item| item.on_disk()).sum()
}

/// Group a plan by category for the detail screen.
pub fn plan_buckets(plan: &UninstallPlan) -> HashMap<Category, (Bytes, usize)> {
    let mut out: HashMap<Category, (Bytes, usize)> = HashMap::new();
    for item in &plan.items {
        let entry = out.entry(item.category).or_insert((Bytes::ZERO, 0));
        entry.0 += item.on_disk();
        entry.1 += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_app(path: &str, source: &str) -> AppInfo {
        AppInfo {
            name: "Demo".into(),
            bundle_id: Some("com.example.Demo".into()),
            path: PathBuf::from(path),
            version: Some("1.0".into()),
            size: Bytes(1_000),
            source: source.into(),
            running: false,
            has_sibling_install: false,
            publisher: None,
            installed_ms: None,
        }
    }

    #[test]
    fn the_bundle_itself_passes_the_safety_policy() {
        // Regression: uninstall used to approve six items and delete five,
        // because the `.app` had no anchor and was dropped in silence.
        let policy = squid_core::SafetyPolicy::default();
        for bundle in [
            "/Applications/Demo.app",
            "/Applications/Utilities/Demo.app",
            "/Users/tester/Applications/Demo.app",
        ] {
            let anchors = bundle_anchors(Path::new(bundle));
            assert_eq!(anchors, vec![Path::new(bundle).parent().unwrap()]);
            assert_eq!(
                policy.denies(Path::new(bundle), &anchors),
                None,
                "squid must be able to remove the bundle it promised"
            );
        }
    }

    #[test]
    fn the_bundle_anchor_still_cannot_reach_protected_folders() {
        // Widening the anchor to the folder that holds the bundle must not
        // widen what can be destroyed: the folder itself is hard-denied, and a
        // SIP bundle anchors to `/System/Applications`, which stays out of
        // reach even though the bundle path itself looks harmless.
        let policy = squid_core::SafetyPolicy::default();
        let anchors = bundle_anchors(Path::new("/Applications/Demo.app"));
        assert_eq!(anchors, vec![PathBuf::from("/Applications")]);
        assert!(
            policy
                .denies(Path::new("/Applications"), &anchors)
                .is_some(),
            "the anchor folder itself must never be removable"
        );
        assert!(policy
            .denies(
                Path::new("/System/Applications"),
                &bundle_anchors(Path::new("/System/Applications/Notes.app"))
            )
            .is_some());
    }
    #[test]
    fn leftovers_inside_the_bundle_are_never_double_counted() {
        let bundle = Path::new("/Applications/Demo.app");
        assert!(skip_leftover(
            Path::new("/Applications/Demo.app/Contents/Resources/x"),
            bundle
        ));
        assert!(skip_leftover(bundle, bundle));
        assert!(skip_leftover(Path::new("/Applications"), bundle));
        assert!(!skip_leftover(
            Path::new("/Users/x/Library/Preferences/com.example.Demo.plist"),
            bundle
        ));
    }

    #[test]
    fn the_tightest_root_wins() {
        let home = squid_core::paths::home_dir();
        let roots = vec![
            squid_platform::LeftoverRoot {
                dir: home.clone(),
                naming: Naming::Either,
                deep: false,
                category: Category::Leftovers,
            },
            squid_platform::LeftoverRoot {
                dir: home.join("Library/Preferences"),
                naming: Naming::BundleId,
                deep: false,
                category: Category::Leftovers,
            },
        ];
        let path = home.join("Library/Preferences/com.example.Demo.plist");
        let anchors = anchor_for(&path, &roots);
        assert_eq!(anchors.len(), 1);
        assert!(anchors[0].ends_with("Library/Preferences"));
    }

    #[test]
    fn system_apps_are_recognised() {
        assert!(is_system_app(&sample_app(
            "/System/Applications/Chess.app",
            "macOS Apps"
        )));
        assert!(!is_system_app(&sample_app(
            "/Applications/Demo.app",
            "Applications"
        )));
    }

    #[test]
    fn a_plan_for_a_missing_app_still_describes_the_bundle() {
        let engine = Engine::new();
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let app = sample_app("/Applications/SquidDoesNotExist.app", "Applications");
        let plan = build_plan(&engine, &app, &progress, &cancel).expect("plan");
        assert!(!plan.items.is_empty());
        assert_eq!(plan.items[0].path, app.path);
        assert!(plan
            .items
            .iter()
            .all(|item| !item.selected || item.lock.is_none()));
    }

    #[test]
    fn a_system_app_plan_is_locked() {
        let engine = Engine::new();
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let app = sample_app("/System/Applications/Chess.app", "macOS Apps");
        let plan = build_plan(&engine, &app, &progress, &cancel).expect("plan");
        assert!(plan.items[0].lock.is_some());
        assert!(plan
            .items
            .iter()
            .all(|item| !item.selected || item.lock.is_none()));
    }

    #[test]
    fn find_returns_nothing_for_a_nonsense_query() {
        let engine = Engine::new();
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let hits = find(
            &engine,
            "zzz-definitely-not-installed-zzz",
            &cancel,
            &progress,
        )
        .expect("find");
        assert!(hits.is_empty());
    }
}
