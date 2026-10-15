//! `squid purge` — deep clean for the things rules cannot know about:
//! build artefacts of projects that were checked out once and never opened
//! again (`node_modules`, `target/`, `Pods/`, `.venv/`, `.next/`, …).
//!
//! The walk is deliberately shallow and boring: it looks for a small allowlist
//! of directory names that are *always* regenerable, plus a handful of ambiguous
//! names (`dist`, `build`, `out`) that only count inside a recognised project.
//!
//! Two guards decide whether an artefact may be offered:
//!
//! 1. **git** — `git check-ignore` proves the folder is ignored (safe);
//!    `git ls-files` proves it is *tracked* (refused, unless `--all`).
//! 2. **age** — artefacts touched in the last `purge_min_age_days` belong to an
//!    active build and are skipped.
//!
//! APFS local snapshots are reported as informational items only: they can hold
//! tens of gigabytes, but only `tmutil` with admin rights can drop them.

use crate::{age_days, Engine};
use squid_core::progress::{CancelToken, Progress};
use squid_core::{Bytes, Category, CleanupItem, ItemKind, LockReason, ScanReport};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Directories that are regenerable wherever they appear.
const ALWAYS: &[&str] = &[
    "node_modules",
    "target",
    "Pods",
    ".venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".dart_tool",
    ".terraform",
    ".serverless",
    ".expo",
    "htmlcov",
    "DerivedData",
    ".cxx",
    ".svelte-kit",
    ".output",
    ".astro",
];

/// Same, but only when the folder clearly belongs to a project: `~/build` or
/// `~/Downloads/dist` are somebody's data, not build output.
const AMBIGUOUS: &[&str] = &[
    "dist", "build", "out", "coverage", ".build", "venv", ".gradle",
];

/// Files that prove the containing folder is a project root.
const PROJECT_MARKERS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "Podfile",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "pom.xml",
    "Gemfile",
    "pubspec.yaml",
    "composer.json",
    "Package.swift",
    "mix.exs",
    "deno.json",
    "CMakeLists.txt",
    "Makefile",
];

/// Never descended into: OS containers, personal media, tool caches that are
/// the job of `squid clean`, and anything that could be a mount point.
const DO_NOT_DESCEND: &[&str] = &[
    ".Trash",
    ".DocumentRevisions-V100",
    ".PKInstallSandboxManager",
    ".Spotlight-V100",
    ".fseventsd",
    ".vol",
    "Library",
    "Movies",
    "Music",
    "Pictures",
    "Applications",
    "System",
    "Private",
    "Volumes",
    ".git",
    ".svn",
    ".hg",
    ".cargo",
    ".rustup",
    ".npm",
    ".pnpm-store",
    ".yarn",
    ".bun",
    ".nvm",
    ".rbenv",
    ".pyenv",
    ".conda",
    ".anaconda",
    ".virtualenvs",
    ".local",
    ".config",
    ".cache",
    ".docker",
    ".colima",
    ".orbstack",
    ".lima",
    ".minikube",
    ".ollama",
    ".android",
    ".gradle",
    ".m2",
    ".sbt",
    ".ivy2",
    ".cocoapods",
    ".pub-cache",
    ".flutter",
    ".bundle",
    ".composer",
    ".dotnet",
    ".nuget",
    ".gvm",
    ".sdkman",
    ".oh-my-zsh",
    ".zsh_sessions",
    ".vscode",
    ".vscode-shared",
    ".cursor",
    ".jetbrains",
    ".macos",
    ".cups",
    ".ssh",
    ".gnupg",
    ".codex",
];

/// How deep the walk goes from each root.
const DEFAULT_DEPTH: usize = 6;
/// Hard stop so a pathological tree cannot spin forever.
const MAX_DIRS: usize = 300_000;

#[derive(Clone, Debug, Default)]
pub struct PurgeOptions {
    /// Where to look. Empty means "the roots from settings plus `$HOME`".
    pub roots: Vec<PathBuf>,
    /// `None` = use `settings.purge_min_age_days`, `Some(0)` = disable.
    pub min_age_days: Option<u64>,
    /// Ignore the age rule *and* the "tracked by git" refusal.
    pub force: bool,
    /// Maximum depth below each root.
    pub max_depth: Option<usize>,
}

/// One folder the walk wants to reclaim.
#[derive(Clone, Debug)]
struct Candidate {
    path: PathBuf,
    /// Nearest directory that looks like a project root (or the artefact itself).
    project: PathBuf,
    name: String,
    always: bool,
}

/// Scan for reclaimable project artefacts.
pub fn scan(
    engine: &Engine,
    options: &PurgeOptions,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<ScanReport> {
    let roots = resolve_roots(engine, options);
    if roots.is_empty() {
        progress.warning(
            "no purge roots exist yet — add some with `squid whitelist`-style config or pass paths",
        );
        return Err(squid_core::SquidError::NothingToDo);
    }
    let depth = options.max_depth.unwrap_or(DEFAULT_DEPTH);
    progress.phase(
        "purge",
        format!("searching {} location(s) for build artefacts", roots.len()),
    );
    progress.tick(0, None, None);

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut visited = 0usize;
    for root in &roots {
        let top_level = roots.len() > 1 && root.components().count() <= 3;
        let cap = if top_level { depth.min(4) } else { depth };
        walk(root, cap, &mut candidates, &mut visited, progress, cancel)?;
    }
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    candidates.dedup_by(|a, b| a.path == b.path);
    progress.note(format!("{} artefact folders found", candidates.len()));

    let min_age = options
        .min_age_days
        .unwrap_or(engine.settings().purge_min_age_days);

    progress.phase("verify", "checking git status and ages");
    let mut report = ScanReport::default();
    let mut git_cache: Vec<(PathBuf, GitState)> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(squid_core::SquidError::Cancelled);
        }
        progress.tick(
            index as u64 + 1,
            Some(candidates.len() as u64),
            Some(candidate.path.clone()),
        );
        let mut item = item_for(candidate);

        if !options.force {
            let project_root = match project_with_git(&candidate.path) {
                Some(dir) => dir,
                None => candidate.project.clone(),
            };
            let state = match git_cache.iter().find(|(dir, _)| *dir == project_root) {
                Some((_, state)) => *state,
                None => {
                    let state = git_state(&project_root, &candidate.path);
                    git_cache.push((project_root.clone(), state));
                    state
                }
            };
            match state {
                GitState::Tracked => {
                    item = item
                        .locked(LockReason::Protected)
                        .with_note("tracked by git — pass --all to delete anyway");
                }
                GitState::Ignored => item.note = Some("git-ignored build output".into()),
                GitState::Untracked => item.note = Some("untracked by git".into()),
                GitState::Unknown => {
                    let age = age_days(&candidate.path);
                    match age {
                        Some(age) if age >= min_age as f64 => {
                            item.note = Some(format!("no git repo · {:.0} days old", age));
                        }
                        Some(age) => {
                            item = item.locked(LockReason::Protected).with_note(format!(
                                "no git repo and only {:.0} days old — pass --all",
                                age
                            ));
                        }
                        None => item.note = Some("age unknown".into()),
                    }
                }
            }
        }

        if item.lock.is_none() {
            let age = age_days(&candidate.path).unwrap_or(0.0);
            if !options.force && min_age > 0 && age < min_age as f64 {
                item = item
                    .locked(LockReason::Protected)
                    .with_note(format!("modified {:.0} days ago — an active build", age));
            }
        }
        if !candidate.always && item.lock.is_none() {
            // `dist`/`build`/`out` are guesses, not proofs: show them, but let
            // the user opt in.
            item.selected = false;
            item.note = Some(format!(
                "{} · name only, review before deleting",
                item.note.unwrap_or_default()
            ));
        }
        if engine.is_whitelisted(&item.path) {
            item = item.locked(LockReason::Whitelisted);
        } else if let Some(reason) = engine.block_reason(&item) {
            item = item.locked(LockReason::Protected).with_note(reason);
        }
        if !crate::writable_by_us(&candidate.path) && !crate::is_root() {
            item = item
                .locked(LockReason::NotOwned)
                .with_note("owned by another user");
        }
        report.items.push(item);
    }

    measure(&mut report, progress, cancel);
    report.items.sort_by(|a, b| {
        b.on_disk()
            .cmp(&a.on_disk())
            .then_with(|| a.path.cmp(&b.path))
    });
    report.unreadable = report
        .items
        .iter()
        .filter(|item| {
            item.note
                .as_deref()
                .map(|note| note.contains("unreadable"))
                .unwrap_or(false)
        })
        .count() as u32;
    Ok(report)
}

fn item_for(candidate: &Candidate) -> CleanupItem {
    let category = if candidate.name.eq_ignore_ascii_case("DerivedData") {
        Category::Developer
    } else {
        Category::ProjectArtifacts
    };
    let project = squid_core::paths::pretty(&candidate.project);
    // The anchor is what authorises a deletion, and the policy never deletes an
    // anchor itself — anchoring an artefact at its own path would lock every
    // single candidate. Anchor it at the project that contains it instead.
    let mut anchor = candidate.project.clone();
    if anchor == candidate.path {
        if let Some(parent) = candidate.path.parent() {
            anchor = parent.to_path_buf();
        }
    }
    CleanupItem::new(
        candidate.path.clone(),
        ItemKind::Path,
        category,
        format!("{}/", candidate.name),
    )
    .with_rule("purge-artifacts")
    .with_anchors(vec![anchor])
    .with_note(project)
}

/// Default roots: everything the user configured, plus the home folder itself
/// (shallow) so a project in an unexpected place is still found.
fn resolve_roots(engine: &Engine, options: &PurgeOptions) -> Vec<PathBuf> {
    if !options.roots.is_empty() {
        return options
            .roots
            .iter()
            .map(|root| squid_core::paths::expand(&root.to_string_lossy()))
            .filter(|path| path.is_dir())
            .collect();
    }
    let mut roots = engine.settings().purge_root_paths();
    let home = squid_core::paths::home_dir();
    if !roots.iter().any(|root| root == &home) {
        roots.push(home);
    }
    roots.retain(|root| root.is_dir());
    roots
}

fn walk(
    root: &Path,
    max_depth: usize,
    out: &mut Vec<Candidate>,
    visited: &mut usize,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<()> {
    let mut stack: Vec<(PathBuf, usize, bool)> = vec![(root.to_path_buf(), 0, false)];
    while let Some((dir, depth, in_project)) = stack.pop() {
        if cancel.is_cancelled() {
            return Err(squid_core::SquidError::Cancelled);
        }
        if *visited >= MAX_DIRS {
            progress.warning(format!(
                "stopped after {} folders — narrow the search with explicit paths",
                MAX_DIRS
            ));
            return Ok(());
        }
        *visited += 1;
        if depth % 32 == 0 {
            progress.tick(*visited as u64, None, Some(dir.clone()));
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut dirs: Vec<String> = Vec::new();
        let mut is_project = false;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or_else(|_| entry.path().is_dir());
            if is_dir {
                dirs.push(name);
            } else if PROJECT_MARKERS.iter().any(|marker| marker == &name) {
                is_project = true;
            }
        }
        for name in dirs {
            if ALWAYS.iter().any(|known| known.owns(&name)) {
                out.push(Candidate {
                    path: dir.join(&name),
                    project: if is_project || in_project {
                        dir.clone()
                    } else {
                        nearest_project(&dir).unwrap_or_else(|| dir.clone())
                    },
                    name,
                    always: true,
                });
                continue;
            }
            if AMBIGUOUS.iter().any(|known| known.owns(&name)) && (is_project || in_project) {
                out.push(Candidate {
                    path: dir.join(&name),
                    project: dir.clone(),
                    name,
                    always: false,
                });
                continue;
            }
            if DO_NOT_DESCEND.iter().any(|skip| skip.owns(&name)) {
                continue;
            }
            if depth < max_depth {
                stack.push((dir.join(&name), depth + 1, is_project || in_project));
            }
        }
    }
    Ok(())
}

/// Compare a directory entry name against a known name, case-insensitively for
/// ASCII (macOS file systems are case-insensitive, git is not).
trait Owns {
    fn owns(&self, name: &str) -> bool;
}

impl Owns for &str {
    fn owns(&self, name: &str) -> bool {
        self.eq_ignore_ascii_case(name)
    }
}

/// Walk up looking for a directory that declares itself a project.
fn nearest_project(dir: &Path) -> Option<PathBuf> {
    let mut cursor = dir.to_path_buf();
    for _ in 0..4 {
        if !cursor.pop() {
            return None;
        }
        let Ok(entries) = std::fs::read_dir(&cursor) else {
            continue;
        };
        let hit = entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            PROJECT_MARKERS.iter().any(|marker| marker == &name)
        });
        if hit {
            return Some(cursor);
        }
    }
    None
}

/// Nearest ancestor that is a git work tree.
fn project_with_git(artifact: &Path) -> Option<PathBuf> {
    let mut cursor = artifact.parent()?.to_path_buf();
    loop {
        if cursor.join(".git").exists() {
            return Some(cursor);
        }
        if !cursor.pop() {
            return None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitState {
    /// `git check-ignore` says the folder is ignored — safe to delete.
    Ignored,
    /// `git ls-files` says content inside the folder is tracked — never delete.
    Tracked,
    /// Inside a work tree, ignored by git, nothing tracked.
    Untracked,
    /// No git repository around, or git is unavailable.
    Unknown,
}

fn git_state(project: &Path, artifact: &Path) -> GitState {
    if !project.join(".git").exists() || project_with_git(artifact).is_none() {
        return GitState::Unknown;
    }
    let ignored = run_git(
        project,
        &["check-ignore", "-q", &artifact.to_string_lossy()],
    );
    if ignored == Some(true) {
        return GitState::Ignored;
    }
    let tracked = run_git_raw(project, &["ls-files", "--", &artifact.to_string_lossy()]);
    match tracked {
        Some(list) if !list.trim().is_empty() => GitState::Tracked,
        Some(_) => GitState::Untracked,
        None => GitState::Unknown,
    }
}

fn run_git(project: &Path, args: &[&str]) -> Option<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(output.status.success())
}

fn run_git_raw(project: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

fn measure(report: &mut ScanReport, progress: &Progress, cancel: &CancelToken) {
    let targets: Vec<(PathBuf, ItemKind)> = report
        .items
        .iter()
        .map(|item| (item.path.clone(), item.kind))
        .collect();
    if targets.is_empty() {
        return;
    }
    progress.phase("measure", "measuring artefacts");
    let sizes = squid_core::fs::measure_all(&targets, cancel, |index, _info| {
        progress.tick(
            index as u64 + 1,
            Some(targets.len() as u64),
            targets.get(index).map(|(path, _)| path.clone()),
        );
    });
    for (item, info) in report.items.iter_mut().zip(sizes) {
        item.size = info.logical;
        item.allocated = info.allocated;
        if info.unreadable > 0 {
            let suffix = format!("{} entries unreadable", info.unreadable);
            item.note = Some(match item.note.take() {
                Some(note) => format!("{note} · {suffix}"),
                None => suffix,
            });
        }
    }
}

/// APFS local snapshots: they can pin tens of gigabytes. Reported, never
/// removed — dropping them needs admin rights and a decision about backups.
/// tmutil prints a `Snapshots for disk /:` banner that is not a snapshot name.
const SNAPSHOT_PREFIX: &str = "com.apple.TimeMachine";

#[derive(Clone, Debug, Default)]
pub struct LocalSnapshot {
    pub name: String,
    pub size_hint: Bytes,
}

pub fn local_snapshots() -> Vec<LocalSnapshot> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let output = match Command::new("tmutil")
        .arg("listlocalsnapshots")
        .arg("/")
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(SNAPSHOT_PREFIX))
        .map(|line| LocalSnapshot {
            name: line.to_string(),
            size_hint: Bytes::ZERO,
        })
        .collect()
}

/// Human hint for the purge screen.
pub fn snapshot_note(snapshots: &[LocalSnapshot]) -> Option<String> {
    if snapshots.is_empty() {
        return None;
    }
    Some(format!(
        "{} APFS local snapshot(s) present — remove with `tmutil deletelocalsnapshots <date>` (admin)",
        snapshots.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "squid-purge-test-{}-{}",
            std::process::id(),
            squid_core::ledger::new_session_id()
        ));
        std::fs::create_dir_all(root.join("demo/node_modules/left-pad")).unwrap();
        std::fs::write(root.join("demo/node_modules/left-pad/index.js"), b"x").unwrap();
        std::fs::write(root.join("demo/package.json"), b"{}").unwrap();
        std::fs::create_dir_all(root.join("demo/dist")).unwrap();
        std::fs::write(root.join("demo/dist/index.html"), b"x").unwrap();
        std::fs::create_dir_all(root.join("loose/dist")).unwrap();
        std::fs::write(root.join("loose/dist/important.pdf"), b"x").unwrap();
        std::fs::create_dir_all(root.join("rust/checkme/target/debug")).unwrap();
        std::fs::write(root.join("rust/checkme/Cargo.toml"), b"[package]").unwrap();
        std::fs::write(root.join("rust/checkme/target/debug/bin"), b"x").unwrap();
        root
    }

    #[test]
    fn artefacts_are_found_and_loose_dirs_are_left_alone() {
        let root = sandbox();
        let settings = squid_core::settings::Settings {
            purge_roots: vec![root.to_string_lossy().into_owned()],
            ..squid_core::settings::Settings::default()
        };
        let engine = Engine::with_settings(settings, Vec::new());
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let report = scan(
            &engine,
            &PurgeOptions {
                roots: vec![root.clone()],
                min_age_days: Some(0),
                force: true,
                max_depth: Some(6),
            },
            &progress,
            &cancel,
        )
        .expect("scan");
        let found: Vec<&std::path::Path> = report
            .items
            .iter()
            .map(|item| item.path.as_path())
            .collect();
        // Compare trailing *components*, never the rendered string: on Windows
        // `...\demo\node_modules` never ends with "demo/node_modules", which
        // failed the positive asserts and — far worse — made the negative one
        // below pass for the wrong reason on every Windows run.
        let has = |suffix: &str| {
            found
                .iter()
                .any(|path| path.ends_with(std::path::Path::new(suffix)))
        };
        let listed = || {
            found
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
        };
        assert!(has("demo/node_modules"), "{:?}", listed());
        assert!(has("rust/checkme/target"), "{:?}", listed());
        assert!(
            has("demo/dist"),
            "dist inside a project is an artefact: {:?}",
            listed()
        );
        assert!(
            !has("loose/dist"),
            "dist outside a project must not be offered: {:?}",
            listed()
        );
        for item in &report.items {
            assert_eq!(item.kind, ItemKind::Path);
            assert!(!item.anchors.is_empty());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fresh_artefacts_are_locked_without_force() {
        let root = sandbox();
        let settings = squid_core::settings::Settings {
            purge_roots: vec![root.to_string_lossy().into_owned()],
            purge_min_age_days: 30,
            ..squid_core::settings::Settings::default()
        };
        let engine = Engine::with_settings(settings, Vec::new());
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let report = scan(
            &engine,
            &PurgeOptions {
                roots: vec![root.clone()],
                min_age_days: None,
                force: false,
                max_depth: Some(6),
            },
            &progress,
            &cancel,
        )
        .expect("scan");
        assert!(!report.items.is_empty());
        assert!(
            report.items.iter().all(|item| !item.selected),
            "brand new artefacts must not be pre-selected"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_state_understands_an_ignored_folder() {
        let root = std::env::temp_dir().join(format!(
            "squid-purge-git-{}-{}",
            std::process::id(),
            squid_core::ledger::new_session_id()
        ));
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(repo.join("node_modules/pkg")).unwrap();
        std::fs::write(repo.join("package.json"), b"{}").unwrap();
        std::fs::write(repo.join("src/main.js"), b"x").unwrap();
        std::fs::write(repo.join(".gitignore"), b"node_modules/\n").unwrap();
        let init = Command::new("git")
            .current_dir(&repo)
            .args(["init", "-q"])
            .status();
        if init.map(|s| s.success()).unwrap_or(false) {
            let state = git_state(&repo, &repo.join("node_modules"));
            assert_eq!(state, GitState::Ignored);
            let tracked = git_state(&repo, &repo.join("src"));
            assert_eq!(tracked, GitState::Untracked);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn do_not_descend_covers_the_dangerous_names() {
        for name in ["Library", ".Trash", ".git", "Volumes", ".ssh"] {
            assert!(
                DO_NOT_DESCEND.iter().any(|skip| skip.owns(name)),
                "{name} must never be descended into"
            );
        }
        assert!(ALWAYS.iter().any(|known| known.owns("node_modules")));
        assert!(AMBIGUOUS.iter().any(|known| known.owns("build")));
        assert!(!ALWAYS.iter().any(|known| known.owns("build")));
    }
}
