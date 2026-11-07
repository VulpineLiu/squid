//! Two kinds of memory, both on disk, neither in a database.
//!
//! [`ScanCache`] remembers the verdict for a file identity — path, size,
//! modification time and file id — so a second scan does not re-hash a file that
//! has not moved. Identity rather than content sampling, because the whole point
//! is to be able to say "this exact file, unchanged" without reading it.
//!
//! [`Store`] keeps the scan the user is looking at plus the recent-activity list
//! the Security page shows. Findings live here precisely so quarantine can take
//! an id: the frontend never gets to name a path, and a stale id is an error
//! rather than a surprise move.
//!
//! Both files are bounded. A cleaner that grows its own state forever is its own
//! kind of junk, so the cache drops its oldest half when it doubles past the
//! limit, and the activity list keeps a fixed number of entries.

use crate::findings::{Confidence, Evidence, Finding};
use serde::{Deserialize, Serialize};
use squid_core::paths;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The three facts that make a file the same file: where it is, how big it is,
/// and when it last changed. Anything else would mean reading it again.
pub struct FileIdentity {
    pub path: PathBuf,
    pub size: u64,
    pub modified_ms: u64,
    pub file_id: u64,
}

impl FileIdentity {
    pub fn of(path: &Path, metadata: &std::fs::Metadata) -> FileIdentity {
        FileIdentity {
            path: path.to_path_buf(),
            size: metadata.len(),
            modified_ms: modified_ms(metadata),
            file_id: file_id(metadata),
        }
    }
}

/// A reusable answer for one file identity.
#[derive(Clone)]
pub struct Hit {
    pub evidence: Vec<Evidence>,
    pub sha256: Option<String>,
    pub signed: bool,
}

/// One remembered verdict, and when a scan last asked for it.
///
/// `path` is stored as the canonical key rather than the original spelling, which
/// is what lets the map below be keyed by it. Nothing displays that field: it is
/// there so a person opening `verdicts.json` can see which file a verdict is about.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedVerdict {
    path: String,
    size: u64,
    modified_ms: u64,
    #[serde(default)]
    file_id: u64,
    evidence: Vec<Evidence>,
    sha256: Option<String>,
    signed: bool,
    /// Scan counter, not a clock: only the order matters, and a clock would make a
    /// test depend on how fast the machine is.
    #[serde(default)]
    used: u64,
}

/// The file on disk. A list, so the format stays readable and diffable, even
/// though the cache is a map in memory.
#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: Vec<CachedVerdict>,
}

/// Verdicts keyed by file identity rather than by path alone: a file that was
/// replaced between scans has a different mtime, and reusing the old answer would
/// report the wrong bytes.
///
/// The map is the point. A scan offers thousands of files, and looking each one up
/// in a list costs one path comparison per cached entry per file — which is
/// quadratic, and was measured at several minutes of CPU on a quick scan whose
/// entire budget is a few seconds. Trimming needs an ordering, so each entry keeps
/// the scan number it was last used at and the list is only sorted when it is
/// written.
pub struct ScanCache {
    entries: HashMap<String, CachedVerdict>,
    /// Counts lookups and stores, to order the cache by recency without a clock.
    clock: u64,
    dirty: bool,
    loaded: bool,
    misses: usize,
}

impl Default for ScanCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanCache {
    pub fn new() -> ScanCache {
        ScanCache {
            entries: HashMap::new(),
            clock: 0,
            dirty: false,
            loaded: false,
            misses: 0,
        }
    }

    fn path() -> PathBuf {
        cache_dir().join("verdicts.json")
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.entries = std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
            .map(|file| {
                file.entries
                    .into_iter()
                    .map(|entry| (entry.path.clone(), entry))
                    .collect()
            })
            .unwrap_or_default();
        self.loaded = true;
    }

    /// The remembered verdict for a file that has not changed since.
    pub fn verdict(&mut self, identity: &FileIdentity) -> Option<Hit> {
        self.load();
        self.clock += 1;
        let used = self.clock;
        let found = self
            .entries
            .get_mut(&paths::canonical_key(&identity.path))?;
        if found.size != identity.size
            || found.modified_ms != identity.modified_ms
            || found.file_id != identity.file_id
        {
            // The same path holding different bytes. Re-hash it: "you already
            // looked at this" is the one answer a scan may not give wrongly.
            self.misses += 1;
            return None;
        }
        found.used = used;
        Some(Hit {
            evidence: found.evidence.clone(),
            sha256: found.sha256.clone(),
            signed: found.signed,
        })
    }

    /// Remember what this scan decided. Called for every file a scan looked at, so
    /// a re-scan of an unchanged tree is a sequence of hits rather than a re-read.
    pub fn remember(&mut self, identity: &FileIdentity, verdict: Hit) {
        self.load();
        self.clock += 1;
        let key = paths::canonical_key(&identity.path);
        // One row per path: a re-scan replaces the answer rather than leaving the
        // file with two verdicts, one of which is stale and might win.
        self.entries.insert(
            key.clone(),
            CachedVerdict {
                path: key,
                size: identity.size,
                modified_ms: identity.modified_ms,
                file_id: identity.file_id,
                evidence: verdict.evidence,
                sha256: verdict.sha256,
                signed: verdict.signed,
                used: self.clock,
            },
        );
        self.dirty = true;
    }

    /// Write the cache, trimming the least recently used half when it outgrows its
    /// budget. A cleaner that grows its own state forever is its own kind of junk.
    pub fn commit(&mut self) {
        self.load();
        if !self.dirty {
            return;
        }
        const LIMIT: usize = 40_000;
        let mut entries: Vec<CachedVerdict> = self.entries.values().cloned().collect();
        entries.sort_by_key(|entry| entry.used);
        if entries.len() > LIMIT * 2 {
            let drop = entries.len() - LIMIT;
            entries.drain(0..drop);
        }
        let file = CacheFile {
            version: 1,
            entries,
        };
        if let Some(parent) = Self::path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(&file) {
            let _ = std::fs::write(Self::path(), bytes);
        }
        self.dirty = false;
    }

    pub fn entries(&self) -> usize {
        self.entries.len()
    }

    pub fn misses(&self) -> usize {
        self.misses
    }

    /// Forget everything. Offered when the rule pack changes, because a verdict
    /// recorded by the old rules is not an answer the new rules gave.
    pub fn invalidate(&mut self) {
        self.entries.clear();
        self.dirty = true;
        self.commit();
    }
}

/// The scan the user is looking at, plus the activity list.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub at_ms: u64,
    pub kind: String,
    pub detail: String,
    pub path: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredScan {
    pub scan_id: String,
    pub mode: String,
    pub finished_ms: u64,
    pub elapsed_ms: u64,
    pub cancelled: bool,
    pub examined: usize,
    pub cached: usize,
    pub skipped: usize,
    pub coverage_complete: bool,
    pub gaps: Vec<(String, String)>,
    pub findings: Vec<Finding>,
}

impl StoredScan {
    /// How many findings at each level still need a decision: not ignored, and
    /// not already sitting in the quarantine.
    ///
    /// This is the definition the badge, the tray line and the page header all
    /// count, so it is one function on the record itself. When it was written
    /// twice — once here, once in the desktop layer — the tray and the Security
    /// page could disagree after a restart while both looked correct.
    pub fn counts(&self) -> (usize, usize, usize) {
        let actionable = |level: Confidence| {
            self.findings
                .iter()
                .filter(|finding| {
                    finding.confidence == level && !finding.ignored && !finding.quarantined
                })
                .count()
        };
        (
            actionable(Confidence::Confirmed),
            actionable(Confidence::Suspicious),
            actionable(Confidence::Advisory),
        )
    }

    /// Freeze one finished scan for storage. The coverage is kept as a boolean
    /// plus the list of gaps, because that is everything the next session needs
    /// in order to refuse to say a plain "clean".
    pub fn record(report: &crate::scan::ScanReport) -> StoredScan {
        StoredScan {
            scan_id: report.scan_id.clone(),
            mode: report.mode.as_str().to_string(),
            finished_ms: report.finished_ms,
            elapsed_ms: report.elapsed_ms,
            cancelled: report.cancelled,
            examined: report.examined,
            cached: report.cached,
            skipped: report.skipped,
            coverage_complete: report.coverage.complete(),
            gaps: report
                .coverage
                .gaps()
                .into_iter()
                .map(|gap| {
                    (
                        gap.display_path.clone(),
                        gap.reason
                            .clone()
                            .unwrap_or_else(|| gap.outcome.as_str().into()),
                    )
                })
                .collect(),
            findings: report.findings.clone(),
        }
    }
}

/// In-memory state for one app session, persisted where it helps the next one.
pub struct Store {
    last_scan: Option<StoredScan>,
    activity: Vec<ActivityEntry>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Store {
        let activity = read_activity();
        Store {
            last_scan: read_last_scan(),
            activity,
        }
    }

    pub fn set_scan(&mut self, scan: StoredScan) {
        write_last_scan(&scan);
        self.last_scan = Some(scan);
    }

    pub fn scan(&self) -> Option<&StoredScan> {
        self.last_scan.as_ref()
    }

    /// Apply one change to the stored scan and write it back. Findings are
    /// mutable state — quarantined, restored, ignored — and the file is the
    /// record of that state, so every mutation goes through here rather than
    /// editing `self.last_scan` and hoping something saves it later.
    pub fn update_scan(&mut self, change: impl FnOnce(&mut StoredScan)) {
        let Some(scan) = self.last_scan.as_mut() else {
            return;
        };
        change(scan);
        write_last_scan(scan);
    }

    /// The finding behind an id. This is the only way a caller reaches a path in
    /// the quarantine path, and it fails loudly when the id is unknown.
    pub fn finding(&self, id: &str) -> Option<&Finding> {
        self.last_scan
            .as_ref()
            .and_then(|scan| scan.findings.iter().find(|finding| finding.id == id))
    }

    pub fn finding_mut(&mut self, id: &str) -> Option<&mut Finding> {
        self.last_scan
            .as_mut()
            .and_then(|scan| scan.findings.iter_mut().find(|finding| finding.id == id))
    }

    pub fn note(&mut self, kind: &str, detail: impl Into<String>, path: Option<PathBuf>) {
        self.activity.push(ActivityEntry {
            at_ms: now_ms(),
            kind: kind.into(),
            detail: detail.into(),
            path: path.map(|path| paths::pretty(&path)),
        });
        const KEEP: usize = 200;
        if self.activity.len() > KEEP {
            let drop = self.activity.len() - KEEP;
            self.activity.drain(0..drop);
        }
        write_activity(&self.activity);
    }

    pub fn activity(&self) -> &[ActivityEntry] {
        &self.activity
    }

    /// Most recent first, which is how a log is read.
    pub fn recent_activity(&self, limit: usize) -> Vec<ActivityEntry> {
        self.activity.iter().rev().take(limit).cloned().collect()
    }

    /// Fold the quarantine ledger into the displayed findings, so a file that
    /// has been dealt with stops being offered as actionable.
    pub fn mark_quarantined(&mut self, quarantined: &[(String, PathBuf)]) {
        let Some(scan) = self.last_scan.as_mut() else {
            return;
        };
        for finding in &mut scan.findings {
            if let Some(path) = finding.path.as_ref() {
                if quarantined
                    .iter()
                    .any(|(_, stored)| paths::same_path(stored, path))
                {
                    finding.quarantined = true;
                }
            }
        }
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        self.last_scan
            .as_ref()
            .map(StoredScan::counts)
            .unwrap_or((0, 0, 0))
    }
}

fn cache_dir() -> PathBuf {
    squid_core::Settings::state_dir()
        .join("security")
        .join("cache")
}

fn activity_path() -> PathBuf {
    squid_core::Settings::state_dir()
        .join("security")
        .join("activity.json")
}

/// The last finished scan, kept so a finding id outlives the process that
/// produced it. `squid guard scan` and `squid guard quarantine` are two runs of
/// one conversation, and the Security page reopening after a restart should say
/// what the last scan found rather than "never scanned".
fn last_scan_path() -> PathBuf {
    squid_core::Settings::state_dir()
        .join("security")
        .join("last-scan.json")
}

fn read_last_scan() -> Option<StoredScan> {
    std::fs::read_to_string(last_scan_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

fn write_last_scan(scan: &StoredScan) {
    let path = last_scan_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(scan) {
        let _ = std::fs::write(path, text);
    }
}

fn read_activity() -> Vec<ActivityEntry> {
    std::fs::read_to_string(activity_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_activity(entries: &[ActivityEntry]) {
    let path = activity_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(entries) {
        let _ = std::fs::write(path, text);
    }
}

fn modified_ms(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// inode on unix, file index on Windows; the part of the identity that survives a
/// rename and catches a replacement in place.
fn file_id(metadata: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.ino()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.creation_time()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = metadata;
        0
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn state_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("squid-store-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn probe(tag: &str, bytes: &[u8]) -> PathBuf {
        let dir = state_dir(tag);
        let path = dir.join("payload");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        path
    }

    fn hit(evidence: &[Evidence], sha256: Option<&str>, signed: bool) -> Hit {
        Hit {
            evidence: evidence.to_vec(),
            sha256: sha256.map(str::to_string),
            signed,
        }
    }

    fn evidence() -> Vec<Evidence> {
        vec![Evidence::DeniedHash {
            sha256: "a".repeat(64),
            threat: "probe".into(),
        }]
    }

    #[test]
    fn a_verdict_is_reused_only_while_the_file_is_unchanged() {
        let path = probe("reuse", b"the bytes that were judged");
        let metadata = std::fs::metadata(&path).unwrap();
        let identity = FileIdentity::of(&path, &metadata);
        let mut cache = ScanCache::new();
        assert!(
            cache.verdict(&identity).is_none(),
            "an empty cache cannot know this file"
        );
        cache.remember(&identity, hit(&evidence(), None, false));
        let stored = cache.verdict(&identity).expect("the file has not changed");
        assert_eq!(stored.evidence.len(), 1);

        // Different size, different answer: the file was replaced.
        let bigger = FileIdentity {
            size: identity.size + 1,
            ..identity
        };
        assert!(cache.verdict(&bigger).is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_replaced_file_loses_its_cached_verdict() {
        let path = probe("replaced", b"first content");
        let before = std::fs::metadata(&path).unwrap();
        let mut cache = ScanCache::new();
        cache.remember(
            &FileIdentity::of(&path, &before),
            hit(&evidence(), None, false),
        );
        std::fs::write(&path, b"a longer second content").unwrap();
        let after = std::fs::metadata(&path).unwrap();
        assert_ne!(before.len(), after.len());
        assert!(
            cache.verdict(&FileIdentity::of(&path, &after)).is_none(),
            "the new bytes must not inherit the old verdict"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn the_cache_survives_a_restart_and_can_be_invalidated() {
        let _guard = crate::state_guard("cache-persist");
        let path = probe("persist", b"remembered across sessions");
        let metadata = std::fs::metadata(&path).unwrap();
        let identity = FileIdentity::of(&path, &metadata);
        {
            let mut cache = ScanCache::new();
            cache.remember(&identity, hit(&evidence(), Some("beef"), false));
            cache.commit();
        }
        let mut reopened = ScanCache::new();
        let stored = reopened.verdict(&identity).expect("the cache is on disk");
        assert_eq!(stored.sha256.as_deref(), Some("beef"));
        reopened.invalidate();
        assert!(
            ScanCache::new().verdict(&identity).is_none(),
            "invalidating must forget everything, because the rules changed"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn findings_are_reachable_by_id_and_only_by_id() {
        let scan = StoredScan {
            scan_id: "s1".into(),
            findings: vec![
                finding("s1#0", Confidence::Confirmed, "/tmp/one"),
                finding("s1#1", Confidence::Suspicious, "/tmp/two"),
            ],
            ..Default::default()
        };
        let mut store = Store::new();
        store.set_scan(scan);
        assert_eq!(
            store.finding("s1#1").unwrap().confidence,
            Confidence::Suspicious
        );
        assert!(store.finding("/tmp/one").is_none(), "a path is not an id");
        assert!(store.finding("s1#9").is_none());
        let (confirmed, suspicious, _) = store.counts();
        assert_eq!((confirmed, suspicious), (1, 1));
    }

    #[test]
    fn quarantining_a_file_stops_it_being_offered_again() {
        let mut scan = StoredScan {
            scan_id: "s2".into(),
            findings: vec![finding("s2#0", Confidence::Confirmed, "/tmp/x")],
            ..Default::default()
        };
        scan.findings[0].path = Some(PathBuf::from("/tmp/x"));
        let mut store = Store::new();
        store.set_scan(scan);
        assert_eq!(store.counts().0, 1);
        store.mark_quarantined(&[("q1".into(), PathBuf::from("/tmp/x"))]);
        assert_eq!(
            store.counts().0,
            0,
            "a dealt-with finding is not outstanding"
        );
        assert!(store.finding("s2#0").unwrap().quarantined);
    }

    #[test]
    fn the_activity_list_is_bounded_and_newest_first() {
        let _guard = crate::state_guard("activity");
        let mut store = Store::new();
        for index in 0..250 {
            store.note("scan", format!("run {index}"), None);
        }
        assert!(store.activity().len() <= 200, "the log must stop growing");
        let recent = store.recent_activity(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].detail, "run 249", "newest first");
        assert!(
            activity_path().is_file(),
            "the log is written for the next session"
        );
    }

    fn finding(id: &str, confidence: Confidence, path: &str) -> Finding {
        Finding {
            id: id.into(),
            confidence,
            path: Some(PathBuf::from(path)),
            display_path: Some(path.into()),
            name: "probe".into(),
            title: "probe".into(),
            description: String::new(),
            evidence: evidence(),
            size: 1,
            sha256: None,
            signed: false,
            publisher: None,
            found_at_ms: 0,
            quarantined: false,
            ignored: false,
        }
    }
}
