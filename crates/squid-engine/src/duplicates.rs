//! Duplicate-file finder — the MangoDisk lesson, with our safety net.
//!
//! Finding byte-identical files cheaply is a **three-stage funnel**:
//!
//! 1. walk everything, bucket by exact size — a file with a unique size can
//!    not have a duplicate, so terabytes drop out for the price of one `stat`;
//! 2. hash a 16 KiB sample from head, middle and tail of every same-size
//!    candidate — different files diverge in those windows almost always, so
//!    this kills the remaining false candidates with ~48 KiB of I/O each;
//! 3. full-stream SHA-256 only for what survived the sample — the expensive
//!    stage runs on a small, pre-filtered set.
//!
//! Results land **group by group while the scan still runs** (a confirmed
//! group is pushed the moment its last file hashes), and every hash is
//! cached on disk keyed by *path + size + mtime*, so re-running the scan on
//! an unchanged disk costs the walk alone.
//!
//! Deletion is deliberately *not* this module's job: confirmed groups are
//! turned into ordinary [`CleanupItem`]s (one per copy, File kind) so the
//! engine's single delete path — safety re-validation, Recycle Bin, undo
//! ledger — guards duplicates exactly like every other byte squid touches.
//! MangoDisk deletes permanently; we do not.

use crate::assets;
use serde::Serialize;
use sha2::{Digest, Sha256};
use squid_core::progress::{CancelToken, Event, Progress};
use squid_core::{Bytes, Category, CleanupItem, ItemKind};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Files below this size are never offered: deduplicating a 4 KiB config is
/// noise, and the 1 MiB floor keeps the funnel's first stage cheap.
pub const DEFAULT_MIN_SIZE: u64 = 1024 * 1024;
/// Sample window at each of head / middle / tail.
const SAMPLE: usize = 16 * 1024;

/// One file that survived the walk and is waiting for its hashes.
#[derive(Clone, Debug)]
struct Candidate {
    path: PathBuf,
    size: u64,
    mtime_ms: u64,
}

/// A confirmed group of byte-identical files.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupGroup {
    pub size: u64,
    /// One entry per copy: (pretty path, mtime ms).
    pub copies: Vec<DupCopy>,
    /// Index of the copy the UI should mark 「保留」 by default (shortest
    /// path — the "original" almost always lives at the tidier address).
    pub keep: usize,
    /// True when at least one copy sits in a protected user-asset folder
    /// (WeChat FileStorage …): the group is flagged, never auto-selected.
    pub personal: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupCopy {
    /// Display spelling (home folded to `~`) for the row.
    pub path: String,
    pub mtime_ms: u64,
    /// Why this copy may not be deleted (asset lock), when it may not.
    pub locked: Option<String>,
    /// Exact path for the delete layer — serialised only for the app-side
    /// cache, never rendered.
    #[serde(skip)]
    pub raw: PathBuf,
}

/// Funnel counters streamed as `Metered`-style notes plus the final report.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupStats {
    pub walked: u64,
    pub same_size: u64,
    pub sample_hits: u64,
    pub confirmed: u64,
    pub wasted: u64,
}

/// Persisted hash cache: path → (size, mtime, sample hash, full hash).
/// A stale entry (size or mtime moved) is re-measured, so correctness never
/// depends on the cache — only speed does.
mod cache {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    #[derive(Default, Serialize, Deserialize)]
    pub struct HashCache {
        /// key: normalised path (lower-case on Windows)
        /// value: (size, mtime_ms, sample_hash_hex, full_hash_hex)
        pub entries: HashMap<PathBuf, (u64, u64, String, String)>,
    }

    impl HashCache {
        pub fn path() -> PathBuf {
            squid_core::Settings::config_dir().join("hash-cache.json")
        }

        pub fn load() -> HashCache {
            std::fs::read_to_string(Self::path())
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default()
        }

        pub fn save(&self) {
            if let Ok(text) = serde_json::to_string(&self) {
                let path = Self::path();
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, text);
            }
        }

        pub fn lookup(&self, key: &Path, size: u64, mtime_ms: u64) -> Option<(&str, &str)> {
            let (s, m, sample, full) = self.entries.get(key)?;
            (*s == size && *m == mtime_ms).then_some((sample.as_str(), full.as_str()))
        }

        pub fn put(&mut self, key: PathBuf, size: u64, mtime_ms: u64, sample: &str, full: &str) {
            self.entries
                .insert(key, (size, mtime_ms, sample.to_string(), full.to_string()));
        }
    }

    pub fn key_for(path: &Path) -> PathBuf {
        let text = path.to_string_lossy().to_lowercase();
        PathBuf::from(text.replace('/', "\\"))
    }
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .or_else(|_| meta.created())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Read SAMPLE bytes at each of three offsets and hash the concatenation.
/// Tiny files (size < 3·SAMPLE) simply overlap — same answer, still cheap.
fn sample_digest(file: &mut File, size: u64) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let window = SAMPLE.min(size as usize);
    let third = ((size as i64) - window as i64).max(0) / 2;
    for offset in [0i64, third, ((size as i64) - window as i64).max(0)] {
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(offset as u64))?;
        let mut buf = vec![0u8; window];
        file.read_exact(&mut buf)?;
        hasher.update(&buf);
    }
    Ok(hex(&hasher.finalize()))
}

fn full_digest(file: &mut File) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Default scan scope: the user's own file areas — home folder (Documents,
/// Desktop, Downloads, Pictures, …) without descending into AppData churn,
/// plus every other fixed drive's user-shaped folders when present.
pub fn default_roots() -> Vec<PathBuf> {
    let home = squid_core::paths::home_dir();
    let mut roots = vec![home.clone()];
    // Other drives: add their root-level Users / Documents-style containers.
    let platform = squid_platform::current();
    if let Ok(volumes) = platform.volumes() {
        for volume in volumes {
            if volume.is_removable || volume.is_root {
                continue;
            }
            let users = volume.mount_point.join("Users");
            if users.is_dir() {
                roots.push(users);
            } else if volume.mount_point.is_dir() {
                roots.push(volume.mount_point);
            }
        }
    }
    roots
}

/// Skip infrastructure we have no business hashing: OS plumbing, recycle
/// bins, app data internals, and any VCS/package store folder by name.
fn skip_dir(name: &str) -> bool {
    const SKIP: &[&str] = &[
        "$recycle.bin",
        "system volume information",
        "windows",
        "program files",
        "program files (x86)",
        "programdata",
        "appdata",
        "node_modules",
        ".git",
        ".hg",
        ".svn",
        "target",
        "__pycache__",
        ".venv",
        "venv",
        ".terraform",
        ".gradle",
        ".m2",
        ".cargo",
        "vendor",
    ];
    let lower = name.to_lowercase();
    SKIP.iter().any(|wanted| lower == *wanted)
}

/// Path-subtree exclusions that only matter as a *sequence*: WorkBuddy's
/// `\.workbuddy\workspace\sessions` folders snapshot copies of files the
/// agent was created from, so deduping them would re-offer churn the tool
/// regenerates itself — on whichever volume it happens to live. Matched as
/// consecutive normal path components, case- and drive-independent.
fn excluded_subtree(dir: &Path) -> bool {
    const SEGMENTS: [&str; 3] = [".workbuddy", "workspace", "sessions"];
    // Cheap guard: the walk prunes at the `sessions` door, so anything else
    // can never match while descending.
    if !dir
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("sessions"))
    {
        return false;
    }
    let names: Vec<String> = dir
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    names.len() >= SEGMENTS.len() && names.windows(SEGMENTS.len()).any(|w| w == SEGMENTS)
}

/// Run the funnel. Groups stream out as `Event::Group { kind: "dup", … }`;
/// the return value is the same data collected, with per-copy locks
/// resolved, so the command layer can hand the frontend a finished list too.
pub fn scan(
    roots: &[PathBuf],
    min_size: u64,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<DupReport> {
    let min_size = min_size.max(4096);
    let mut hash_cache = cache::HashCache::load();
    let walked = AtomicU64::new(0);

    // ── Stage 1: walk, bucket by exact size ──────────────────────────────
    progress.phase("duplicates", "第一级 · 按大小分组");
    let mut by_size: HashMap<u64, Vec<Candidate>> = HashMap::new();
    {
        let mut stack: Vec<(PathBuf, usize)> = roots
            .iter()
            .filter(|root| root.is_dir())
            .map(|root| (root.clone(), 0usize))
            .collect();
        while let Some((dir, depth)) = stack.pop() {
            cancel.check()?;
            if depth > 24 {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    if skip_dir(&entry.file_name().to_string_lossy()) || excluded_subtree(&path) {
                        continue;
                    }
                    stack.push((path, depth + 1));
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let count = walked.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 512 == 0 {
                    progress.publish(Event::Metered {
                        files: count,
                        bytes: Bytes(0),
                    });
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let size = meta.len();
                if size < min_size {
                    continue;
                }
                by_size.entry(size).or_default().push(Candidate {
                    path,
                    size,
                    mtime_ms: mtime_ms(&meta),
                });
            }
        }
    }
    let same_size: usize = by_size
        .iter()
        .filter(|(_, group)| group.len() > 1)
        .map(|(_, group)| group.len())
        .sum();
    // Funnel counters ride structured notes the UI parses by prefix:
    // `funnel:<walked>:<same-size>:<sample-hits>:<confirmed>`.
    progress.publish(Event::Note(format!(
        "funnel:{}:{same_size}:0:0",
        walked.load(Ordering::Relaxed)
    )));

    // ── Stage 2: head/middle/tail sample hash ────────────────────────────
    progress.phase("duplicates", "第二级 · 头中尾采样哈希");
    let mut by_sample: HashMap<(u64, String), Vec<Candidate>> = HashMap::new();
    let mut sample_seen = 0u64;
    for (_, group) in by_size.drain() {
        if group.len() < 2 {
            continue;
        }
        for candidate in group {
            cancel.check()?;
            sample_seen += 1;
            progress.publish(Event::Tick {
                done: sample_seen,
                total: Some(same_size as u64),
                subject: Some(candidate.path.clone()),
            });
            let key = cache::key_for(&candidate.path);
            let mut file = match File::open(&candidate.path) {
                Ok(file) => file,
                Err(_) => continue,
            };
            let digest = match hash_cache.lookup(&key, candidate.size, candidate.mtime_ms) {
                Some((sample, _full)) => sample.to_string(),
                None => match sample_digest(&mut file, candidate.size) {
                    Ok(digest) => digest,
                    Err(_) => continue,
                },
            };
            by_sample
                .entry((candidate.size, digest))
                .or_default()
                .push(candidate);
        }
    }
    let sample_hits: usize = by_sample
        .iter()
        .filter(|(_, group)| group.len() > 1)
        .map(|(_, group)| group.len())
        .sum();
    progress.publish(Event::Note(format!(
        "funnel:{}:{same_size}:{sample_hits}:0",
        walked.load(Ordering::Relaxed)
    )));

    // ── Stage 3: full hash on the survivors ──────────────────────────────
    progress.phase("duplicates", "第三级 · 全量校验确认");
    let mut report = DupReport::default();
    let mut confirmed_seen = 0u64;
    let groups: Vec<((u64, String), Vec<Candidate>)> = by_sample
        .drain()
        .filter(|(_, group)| group.len() > 1)
        .collect();
    let total_full: u64 = groups.iter().map(|(_, group)| group.len() as u64).sum();
    for ((size, sample), group) in groups {
        let mut by_full: HashMap<String, Vec<Candidate>> = HashMap::new();
        for candidate in group {
            cancel.check()?;
            confirmed_seen += 1;
            progress.publish(Event::Tick {
                done: confirmed_seen,
                total: Some(total_full),
                subject: Some(candidate.path.clone()),
            });
            let key = cache::key_for(&candidate.path);
            let digest = match hash_cache.lookup(&key, size, candidate.mtime_ms) {
                Some((_sample, full)) => full.to_string(),
                None => {
                    let Ok(mut file) = File::open(&candidate.path) else {
                        continue;
                    };
                    // The sample matched; if the cache was cold this file was
                    // just read for the sample, so re-opening it is the price
                    // of a stateless cache layout.
                    match full_digest(&mut file) {
                        Ok(digest) => {
                            hash_cache.put(key.clone(), size, candidate.mtime_ms, &sample, &digest);
                            digest
                        }
                        Err(_) => continue,
                    }
                }
            };
            by_full.entry(digest).or_default().push(candidate);
        }
        for (_, copies) in by_full {
            if copies.len() < 2 {
                continue;
            }
            let group = build_group(size, &copies);
            report.wasted += (copies.len() as u64 - 1) * size;
            report.confirmed_groups += 1;
            report.confirmed_files += copies.len() as u64;
            progress.publish(Event::Group {
                kind: "dup",
                payload: serde_json::to_value(&group).unwrap_or_default(),
            });
            report.groups.push(group);
        }
    }
    // Persist the cold-half of the work so the next scan is a walk only.
    // Keep the file bounded: half of it is dropped when it outgrows 600k
    // entries (a few tens of MB of JSON is already generous).
    if hash_cache.entries.len() > 600_000 {
        let keep = hash_cache.entries.len() / 2;
        let keys: Vec<PathBuf> = hash_cache.entries.keys().skip(keep).cloned().collect();
        for key in keys {
            hash_cache.entries.remove(&key);
        }
    }
    hash_cache.save();
    report.walked = walked.load(Ordering::Relaxed);
    report.same_size = same_size as u64;
    report.sample_hits = sample_hits as u64;
    report
        .groups
        .sort_by_key(|group| std::cmp::Reverse(group.size * (group.copies.len() as u64 - 1)));
    progress.publish(Event::Note(format!(
        "funnel:{}:{same_size}:{sample_hits}:{}",
        report.walked, report.confirmed_files
    )));
    Ok(report)
}

/// Shape one confirmed group for the UI: keep-shortest-path, asset locks.
fn build_group(size: u64, copies: &[Candidate]) -> DupGroup {
    let mut ordered: Vec<&Candidate> = copies.iter().collect();
    // Shortest path wins the 「保留」 chip; ties fall back to oldest mtime
    // (the copy that has been there longest is the least likely accident).
    ordered.sort_by(|a, b| {
        a.path
            .components()
            .count()
            .cmp(&b.path.components().count())
            .then(a.mtime_ms.cmp(&b.mtime_ms))
            .then(a.path.cmp(&b.path))
    });
    let mut personal = false;
    let copies = ordered
        .iter()
        .map(|candidate| {
            let locked = assets::asset_reason(&candidate.path).map(str::to_string);
            if locked.is_some() {
                personal = true;
            }
            DupCopy {
                path: squid_core::paths::pretty(&candidate.path),
                mtime_ms: candidate.mtime_ms,
                locked,
                raw: candidate.path.clone(),
            }
        })
        .collect();
    DupGroup {
        size,
        copies,
        keep: 0,
        personal,
    }
}

impl DupReport {
    /// Flatten the user's deletion picks into engine items: one File item per
    /// chosen copy. `selections` is (group index, copy indexes to delete) —
    /// the keep decision belongs to the UI, this layer only executes what was
    /// confirmed identical and explicitly picked.
    pub fn items_for(&self, selections: &[(usize, Vec<usize>)]) -> Vec<CleanupItem> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut items = Vec::new();
        for (group_index, copies) in selections {
            let Some(group) = self.groups.get(*group_index) else {
                continue;
            };
            for copy_index in copies {
                let Some(copy) = group.copies.get(*copy_index) else {
                    continue;
                };
                if group.copies.len() <= 1 || copy.locked.is_some() {
                    continue;
                }
                let path = copy.raw.clone();
                // Anchor two levels up: the safety policy demands the target
                // sit *strictly below* an anchor (depth ≥ 1), and the file's
                // own parent would place it at depth 0.
                let anchor = path
                    .ancestors()
                    .nth(2)
                    .map(|dir| dir.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("/"));
                let mut item = CleanupItem::new(
                    path.clone(),
                    ItemKind::File,
                    Category::Duplicates,
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
                .with_rule("duplicates")
                .with_anchors(std::iter::once(anchor))
                .with_note(format!("{} 份相同 · 已全量校验一致", group.copies.len()));
                item.size = Bytes(group.size);
                item.allocated = Bytes(group.size);
                item.file_count = 1;
                item.last_write_secs = Some(now_ms.saturating_sub(copy.mtime_ms) / 1000);
                item.evidence = vec![format!("record:sha256:{}", group.size)];
                items.push(item);
            }
        }
        items
    }
}

/// Everything a finished scan says: the groups plus the funnel counters the
/// UI prints under its title.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupReport {
    pub groups: Vec<DupGroup>,
    pub walked: u64,
    pub same_size: u64,
    pub sample_hits: u64,
    pub confirmed_groups: u64,
    pub confirmed_files: u64,
    pub wasted: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use squid_core::progress::Progress;

    fn scratch(tag: &str) -> Option<PathBuf> {
        // Keep test hashes out of the real config-dir cache file.
        static STATE: std::sync::Once = std::sync::Once::new();
        STATE.call_once(|| {
            let dir = std::env::temp_dir().join(format!("squid-dups-state-{}", std::process::id()));
            if std::fs::create_dir_all(&dir).is_ok() {
                std::env::set_var("SQUID_STATE_DIR", &dir);
            }
        });
        let dir = std::env::temp_dir().join(format!(
            "squid-dups-{}-{tag}-{}",
            std::process::id(),
            squid_core::ledger::new_session_id()
        ));
        match std::fs::create_dir_all(&dir) {
            Ok(()) => Some(dir),
            Err(error) => {
                eprintln!("skipping: scratch denied ({error})");
                None
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn workbuddy_session_snapshots_are_never_walked() {
        assert!(excluded_subtree(Path::new(
            r"C:\.workbuddy\workspace\sessions"
        )));
        assert!(excluded_subtree(Path::new(
            r"D:\Work\.WorkBuddy\Workspace\Sessions"
        )));
        assert!(!excluded_subtree(Path::new(
            r"C:\Users\dev\workspace\sessions"
        )));
        assert!(!excluded_subtree(Path::new(
            r"C:\.workbuddy\workspace\sessions-notes"
        )));
    }

    #[test]
    fn identical_files_land_in_one_group_with_the_neat_copy_kept() {
        let Some(root) = scratch("same") else { return };
        let payload = vec![7u8; 2 * 1024 * 1024];
        let deep = root.join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("copy.bin"), &payload).unwrap();
        std::fs::write(deep.join("copy.bin"), &payload).unwrap();
        std::fs::write(root.join("other.bin"), vec![9u8; payload.len()]).unwrap();

        let progress = Progress::new();
        let report = scan(
            std::slice::from_ref(&root),
            1024 * 1024,
            &progress,
            &CancelToken::new(),
        )
        .expect("scan");
        assert_eq!(report.groups.len(), 1, "{:?}", report.groups);
        let group = &report.groups[0];
        assert_eq!(group.copies.len(), 2);
        // keep = shortest path: the root-level copy wins the chip.
        assert!(!group.copies[group.keep].path.contains("b\\c"));
        assert_eq!(report.confirmed_files, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn same_size_different_bytes_never_matches() {
        // The whole point of stage 2/3: two same-size files differing only in
        // the middle byte past the head sample window.
        let Some(root) = scratch("diff") else { return };
        let a = vec![3u8; 200_000];
        let mut b = vec![3u8; 200_000];
        b[100_001] = 4;
        std::fs::write(root.join("a.bin"), &a).unwrap();
        std::fs::write(root.join("b.bin"), &b).unwrap();
        drop(a);
        drop(b);
        let progress = Progress::new();
        let report = scan(
            std::slice::from_ref(&root),
            1024,
            &progress,
            &CancelToken::new(),
        )
        .expect("scan");
        assert!(report.groups.is_empty(), "{:?}", report.groups);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tiny_files_are_not_offered_at_all() {
        let Some(root) = scratch("tiny") else { return };
        std::fs::write(root.join("one.txt"), b"same").unwrap();
        std::fs::write(root.join("two.txt"), b"same").unwrap();
        let progress = Progress::new();
        let report = scan(
            std::slice::from_ref(&root),
            DEFAULT_MIN_SIZE,
            &progress,
            &CancelToken::new(),
        )
        .expect("scan");
        assert_eq!(report.walked, 2);
        assert_eq!(report.same_size, 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
