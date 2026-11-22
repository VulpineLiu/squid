//! `security_*` and the rule-pack channel for the desktop app.
//!
//! The shape of every command here follows one rule from the roadmap: the
//! frontend sends back ids, never paths. A finding id is looked up in the stored
//! scan, and the path, hash, size and mode that the quarantine acts on are the
//! ones the scan recorded. A webview that claimed "/usr/sbin" was a finding would
//! have to be believed, and a webview that can be tricked into saying anything is
//! a webview that should not be trusted with a move.
//!
//! Scans reuse the job bookkeeping in [`crate::app::AppState`], so cancel and
//! progress look exactly like a cleanup scan to the frontend.

use crate::app::{AppState, ProgressReporter};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use squid_core::progress::{CancelToken, Progress};
use squid_core::settings::{AllowEntry, AllowKind};
use squid_core::{paths, Settings};
use squid_guard::intel::IntelStatus;
use squid_guard::quarantine::{Quarantine, QuarantineEntry};
use squid_guard::scan::{Guard, ScanReport};

/// Re-exported so the tray can start the same scans the page starts, without the
/// root module reaching into `squid-guard` for itself.
pub use squid_guard::scan::{ScanMode, ScanTarget};
use squid_guard::store::{ActivityEntry, StoredScan};
use squid_guard::{Confidence, Finding};
use squid_platform::protection::ProtectionStatus;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

/// Where the timer thread signals itself. Held in managed state so a settings
/// change can wake the scheduler instead of waiting for the previous deadline.
#[derive(Default)]
pub struct SecurityState {
    wake: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// Whether a scan is running, so the tray can say "Scanning…" and be
    /// unclickable rather than starting a second scan over the first.
    scanning: std::sync::atomic::AtomicBool,
}

impl SecurityState {
    pub(crate) fn set_scanning(&self, scanning: bool) {
        self.scanning
            .store(scanning, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn scanning(&self) -> bool {
        self.scanning.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// What is still outstanding, in the form the tray and the sidebar want.
///
/// Read from the store rather than remembered in memory. The tray kept its own
/// counter, refreshed after each scan, and that made it lie after a restart: the
/// Security page said three findings were waiting while the tray said nothing was,
/// because the counter had gone back to zero and no scan had run yet in this
/// session. Both surfaces now answer from the one record the page draws, with the
/// quarantine ledger folded in, so "the badge and the page agree" is a property of
/// there being one place to ask rather than of two places being kept in step.
pub(crate) fn outstanding() -> (usize, usize, bool) {
    let mut store = squid_guard::store::Store::new();
    let scanned = store.scan().is_some();
    let mut held = Quarantine::new();
    let pairs: Vec<(String, PathBuf)> = held
        .entries()
        .iter()
        .map(|entry| (entry.id.clone(), entry.original_path.clone()))
        .collect();
    store.mark_quarantined(&pairs);
    let (settings, _) = Settings::load();
    let (confirmed, suspicious, _) = store
        .scan()
        .map(|scan| {
            let mut counts = (0, 0, 0);
            for finding in &scan.findings {
                if finding.ignored
                    || finding.quarantined
                    || squid_guard::allowlist::is_allowlisted(finding, &settings.security.allowlist)
                {
                    continue;
                }
                match finding.confidence {
                    Confidence::Confirmed => counts.0 += 1,
                    Confidence::Suspicious => counts.1 += 1,
                    Confidence::Advisory => counts.2 += 1,
                }
            }
            counts
        })
        .unwrap_or((0, 0, 0));
    (confirmed, suspicious, scanned)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GapView {
    path: String,
    reason: String,
}

/// One piece of evidence, as two strings.
///
/// The backend enum is externally tagged, which reaches JSON as
/// `{"deniedHash":{"sha256":…,"threat":…}}` — a shape the frontend would have to
/// pattern-match over nine variants for. `key` says what kind it is and `summary`
/// says what it found, both already written in Rust where the variants live, so
/// the row and the list cannot disagree and a new variant costs no UI change.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceView {
    key: String,
    summary: String,
}

impl From<&squid_guard::Evidence> for EvidenceView {
    fn from(evidence: &squid_guard::Evidence) -> EvidenceView {
        EvidenceView {
            key: evidence.key(),
            summary: evidence.summary(),
        }
    }
}

/// A finding with its paths already shortened and its evidence turned to text.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingView {
    id: String,
    confidence: Confidence,
    path: Option<String>,
    display_path: Option<String>,
    name: String,
    title: String,
    description: String,
    evidence: Vec<EvidenceView>,
    size: u64,
    sha256: Option<String>,
    signed: bool,
    publisher: Option<String>,
    found_at_ms: u64,
    quarantined: bool,
    ignored: bool,
    allowlisted: bool,
}

impl FindingView {
    fn new(finding: &Finding, allowlist: &[AllowEntry]) -> FindingView {
        FindingView {
            id: finding.id.clone(),
            confidence: finding.confidence,
            path: finding
                .path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            display_path: finding.display_path.clone(),
            name: finding.name.clone(),
            title: finding.title.clone(),
            description: finding.description.clone(),
            evidence: finding.evidence.iter().map(EvidenceView::from).collect(),
            size: finding.size,
            sha256: finding.sha256.clone(),
            signed: finding.signed,
            publisher: finding.publisher.clone(),
            found_at_ms: finding.found_at_ms,
            quarantined: finding.quarantined,
            ignored: finding.ignored,
            allowlisted: squid_guard::allowlist::is_allowlisted(finding, allowlist),
        }
    }
}

/// One scan as the UI sees it. The findings are the backend's own serialisation,
/// so a field added in `squid-guard` shows up here without a second mapping.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanView {
    scan_id: String,
    mode: String,
    finished_ms: u64,
    elapsed_ms: u64,
    cancelled: bool,
    examined: usize,
    cached: usize,
    skipped: usize,
    coverage_complete: bool,
    gaps: Vec<GapView>,
    findings: Vec<FindingView>,
    system_advisories: Vec<FindingView>,
    confirmed: usize,
    suspicious: usize,
    advisory: usize,
    /// Nothing found and nothing skipped: the only state allowed to read "clean".
    clean: bool,
}

impl ScanView {
    fn new(report: &ScanReport, allowlist: &[AllowEntry]) -> ScanView {
        let findings: Vec<FindingView> = report
            .findings
            .iter()
            .map(|finding| FindingView::new(finding, allowlist))
            .collect();
        ScanView {
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
                .map(|gap| GapView {
                    path: gap.display_path.clone(),
                    reason: gap
                        .reason
                        .clone()
                        .unwrap_or_else(|| gap.outcome.as_str().into()),
                })
                .collect(),
            findings: findings.clone(),
            system_advisories: report
                .system_advisories
                .iter()
                .map(|finding| FindingView::new(finding, allowlist))
                .collect(),
            confirmed: count_open(&findings, Confidence::Confirmed),
            suspicious: count_open(&findings, Confidence::Suspicious),
            advisory: count_open(&findings, Confidence::Advisory),
            clean: report.clean_and_complete(),
        }
    }

    fn from_stored(scan: &StoredScan, allowlist: &[AllowEntry]) -> ScanView {
        let findings: Vec<FindingView> = scan
            .findings
            .iter()
            .map(|finding| FindingView::new(finding, allowlist))
            .collect();
        ScanView {
            scan_id: scan.scan_id.clone(),
            mode: scan.mode.clone(),
            finished_ms: scan.finished_ms,
            elapsed_ms: scan.elapsed_ms,
            cancelled: scan.cancelled,
            examined: scan.examined,
            cached: scan.cached,
            skipped: scan.skipped,
            coverage_complete: scan.coverage_complete,
            gaps: scan
                .gaps
                .iter()
                .map(|(path, reason)| GapView {
                    path: path.clone(),
                    reason: reason.clone(),
                })
                .collect(),
            confirmed: count_open(&findings, Confidence::Confirmed),
            suspicious: count_open(&findings, Confidence::Suspicious),
            advisory: count_open(&findings, Confidence::Advisory),
            findings,
            system_advisories: Vec::new(),
            clean: scan.findings.is_empty() && scan.coverage_complete && !scan.cancelled,
        }
    }
}

fn count_open(findings: &[FindingView], confidence: Confidence) -> usize {
    findings
        .iter()
        .filter(|finding| {
            finding.confidence == confidence
                && !finding.ignored
                && !finding.allowlisted
                && !finding.quarantined
        })
        .count()
}

/// The security half of the settings file, in the shape the UI binds to.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecuritySettingsView {
    scan_schedule: String,
    notify_on_findings: bool,
    auto_quarantine: bool,
    update_rules: bool,
    scan_exclusions: Vec<String>,
    ignored_findings: Vec<String>,
    allowlist: Vec<AllowEntry>,
    /// Absolute paths a scan covers, so the UI can promise only what it can show.
    quick_roots: Vec<String>,
    full_roots: Vec<String>,
    quarantined: Vec<String>,
}

impl SecuritySettingsView {
    fn current() -> SecuritySettingsView {
        let (settings, _) = Settings::load();
        let held = held_paths();
        SecuritySettingsView {
            scan_schedule: settings.security.scan_schedule.as_str().to_string(),
            notify_on_findings: settings.security.notify_on_findings,
            auto_quarantine: settings.security.auto_quarantine,
            update_rules: settings.security.update_rules,
            scan_exclusions: settings.security.scan_exclusions.clone(),
            ignored_findings: settings.security.ignored_findings.clone(),
            allowlist: settings.security.allowlist.clone(),
            quick_roots: display_list(&squid_guard::scan::quick_roots()),
            full_roots: display_list(&squid_guard::scan::full_roots()),
            quarantined: held,
        }
    }
}

fn display_list(roots: &[PathBuf]) -> Vec<String> {
    roots
        .iter()
        .filter(|root| root.exists())
        .map(|root| paths::pretty(root))
        .collect()
}

/// Original paths in the quarantine, used to grey out findings that are already
/// dealt with even when the stored scan predates the move.
fn held_paths() -> Vec<String> {
    let mut held = Quarantine::new();
    held.entries()
        .iter()
        .map(|entry| entry.original_path.to_string_lossy().into_owned())
        .collect()
}

/// What the quarantine holds, with the evidence flattened the same way a
/// finding's is. `original_path` stays in the payload because restore needs the
/// id, not the path — the path is only ever shown.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineView {
    id: String,
    original_path: String,
    display_path: String,
    size: u64,
    sha256: String,
    modified_ms: u64,
    quarantined_ms: u64,
    title: String,
    confidence: String,
    evidence: Vec<EvidenceView>,
}

impl From<&QuarantineEntry> for QuarantineView {
    fn from(entry: &QuarantineEntry) -> QuarantineView {
        QuarantineView {
            id: entry.id.clone(),
            original_path: entry.original_path.to_string_lossy().into_owned(),
            display_path: entry.display_path.clone(),
            size: entry.size,
            sha256: entry.sha256.clone(),
            modified_ms: entry.modified_ms,
            quarantined_ms: entry.quarantined_ms,
            title: entry.title.clone(),
            confidence: entry.confidence.clone(),
            evidence: entry.evidence.iter().map(EvidenceView::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatus {
    protection: ProtectionStatus,
    intel: IntelStatus,
    /// `None` until the first scan has run on this machine.
    scan: Option<ScanView>,
    quarantine: Vec<QuarantineView>,
    quarantine_size: u64,
    activity: Vec<ActivityEntry>,
    settings: SecuritySettingsView,
    permissions: crate::permissions::PermissionStatus,
    paths: SecurityPaths,
    /// Ready for the tray and the overview strip: something to act on right now.
    confirmed: usize,
    suspicious: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityPaths {
    quarantine: String,
    intel: String,
    startup_backups: String,
}

#[tauri::command]
pub async fn security_status() -> Result<SecurityStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let (settings, _) = Settings::load();
        let mut store = squid_guard::store::Store::new();
        let mut held = Quarantine::new();
        // The ledger, not the frontend, decides what counts as handled.
        let pairs: Vec<(String, PathBuf)> = held
            .entries()
            .iter()
            .map(|entry| (entry.id.clone(), entry.original_path.clone()))
            .collect();
        store.mark_quarantined(&pairs);
        let scan = store
            .scan()
            .map(|scan| ScanView::from_stored(scan, &settings.security.allowlist));
        let (confirmed, suspicious) = scan
            .as_ref()
            .map(|view| (view.confirmed, view.suspicious))
            .unwrap_or((0, 0));
        let quarantine = held.entries().iter().map(QuarantineView::from).collect();
        let size = held.total_size();
        SecurityStatus {
            protection: squid_platform::protection::status(),
            intel: squid_guard::intel::status(),
            scan,
            quarantine,
            quarantine_size: size,
            activity: store.recent_activity(20),
            settings: SecuritySettingsView::current(),
            permissions: crate::permissions::permission_status(),
            paths: SecurityPaths {
                quarantine: paths::pretty(&squid_guard::coverage::quarantine_root()),
                intel: paths::pretty(&squid_guard::intel::intel_dir()),
                startup_backups: paths::pretty(&squid_guard::coverage::startup_backup_root()),
            },
            confirmed,
            suspicious,
        }
    })
    .await
    .map_err(|error| format!("Could not read the security state: {error}"))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    /// `quick`, `full`, or `custom` together with `paths`.
    mode: String,
    #[serde(default)]
    paths: Vec<String>,
}

#[tauri::command]
pub async fn security_scan(
    app: AppHandle,
    job_id: String,
    request: ScanRequest,
) -> Result<ScanView, String> {
    let target = match target_for(&request.mode, &request.paths) {
        Ok(target) => target,
        Err(error) => {
            app.state::<AppState>().finish(&job_id);
            return Err(error);
        }
    };
    let token = app.state::<AppState>().begin(&job_id)?;
    mark_scanning(&app, true);
    let progress = Progress::new();
    let reporter =
        ProgressReporter::start(app.clone(), job_id.clone(), "security", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || run_scan(target, &progress, &token))
        .await
        .map_err(|error| format!("The security scan stopped unexpectedly: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    mark_scanning(&app, false);
    let view = task?;
    after_scan(app.clone(), view.clone());
    Ok(view)
}

/// Resolve the mode the frontend sends into a scope, or refuse. A custom scan is
/// capped at 64 paths so a mistyped pick cannot quietly become a disk sweep.
fn target_for(mode: &str, chosen: &[String]) -> Result<ScanTarget, String> {
    match mode {
        "quick" => Ok(ScanTarget::Quick),
        "full" => Ok(ScanTarget::Full),
        "custom" => {
            let paths: Vec<String> = chosen
                .iter()
                .map(|path| path.trim().to_string())
                .filter(|path| !path.is_empty())
                .take(64)
                .collect();
            if paths.is_empty() {
                return Err("Choose at least one file or folder to scan.".into());
            }
            Ok(ScanTarget::Paths(paths))
        }
        other => Err(format!("Unknown scan mode {other}.")),
    }
}

/// Flip the tray's "Scanning…" line, and redraw the tray so the change is visible.
fn mark_scanning(app: &AppHandle, scanning: bool) {
    if let Some(state) = app.try_state::<SecurityState>() {
        state.set_scanning(scanning);
        let (settings, _) = Settings::load();
        crate::sync_tray(app, squid_core::Locale::detect(&settings));
    }
}

pub(crate) fn scanning(app: &AppHandle) -> bool {
    app.try_state::<SecurityState>()
        .map(|state| state.scanning())
        .unwrap_or(false)
}

/// Start a scan that nobody waits for: the tray item and the scheduler. The
/// result still lands in the store, the tray, and a notification, so a scan that
/// finishes while the window is closed is not a scan that never happened.
pub(crate) fn scan_in_background(app: AppHandle, target: ScanTarget) {
    if scanning(&app) {
        return;
    }
    let job_id = format!("scan-{}", uuid::Uuid::new_v4());
    let Ok(token) = app.state::<AppState>().begin(&job_id) else {
        return;
    };
    mark_scanning(&app, true);
    let handle = app.clone();
    std::thread::spawn(move || {
        let progress = Progress::new();
        let reporter =
            ProgressReporter::start(handle.clone(), job_id.clone(), "security", progress.clone());
        let result = run_scan(target, &progress, &token);
        reporter.finish();
        handle.state::<AppState>().finish(&job_id);
        mark_scanning(&handle, false);
        match result {
            Ok(view) => after_scan(handle, view),
            Err(error) => {
                let mut store = squid_guard::store::Store::new();
                store.note("scan-failed", error, None);
            }
        }
    });
}

/// The scan itself, on the blocking thread: build the guard from the current
/// settings and rule pack, run, store what it found, and only then apply the
/// automatic quarantine.
///
/// Ordering matters twice over. The scan is recorded before anything is moved, so
/// an interrupted run still leaves finding ids a later command can act on. And
/// auto-quarantine happens against the report's own `Finding` values — the ones
/// with real evidence and paths — not against the flattened view the UI is sent.
fn run_scan(
    target: ScanTarget,
    progress: &Progress,
    token: &CancelToken,
) -> Result<ScanView, String> {
    let (settings, _) = Settings::load();
    let mut guard = Guard::new(settings.security.exclusion_paths());
    let report = guard
        .scan(&target, progress, token)
        .map_err(|error| error.to_string())?;
    let mut store = squid_guard::store::Store::new();
    store.set_scan(StoredScan::record(&report));
    store.note(
        "scan",
        format!(
            "{} · {} files · {} findings",
            report.mode.as_str(),
            report.examined,
            report.findings.len()
        ),
        None,
    );
    if settings.security.auto_quarantine {
        let mut held = Quarantine::new();
        for finding in &report.findings {
            if !finding.confidence.auto_quarantinable() || finding.quarantined {
                continue;
            }
            match held.quarantine(finding) {
                Ok(entry) => store.note(
                    "quarantine",
                    entry.title.clone(),
                    Some(entry.original_path.clone()),
                ),
                // A finding that could not be moved is reported, not retried
                // silently: "in use" and "permission denied" are the user's to
                // resolve, and pretending otherwise hides an unhandled threat.
                Err(error) => store.note("quarantine-failed", error, finding.path.clone()),
            }
        }
        let pairs: Vec<(String, PathBuf)> = held
            .entries()
            .iter()
            .map(|entry| (entry.id.clone(), entry.original_path.clone()))
            .collect();
        store.mark_quarantined(&pairs);
    }
    // The stored scan, re-read, is what the UI sees: it is the only version that
    // has the ledger folded into it.
    Ok(store
        .scan()
        .map(|scan| ScanView::from_stored(scan, &settings.security.allowlist))
        .unwrap_or_else(|| ScanView::new(&report, &settings.security.allowlist)))
}

/// Everything that follows a finished scan once the ledger has been folded in:
/// the tray line, the frontend event, and the notification if the user asked.
fn after_scan(app: AppHandle, view: ScanView) {
    let (settings, _) = Settings::load();
    // Nothing is copied into the tray: `sync_tray` asks `outstanding()`, which
    // reads the same store the Security page was built from.
    crate::sync_tray(&app, squid_core::Locale::detect(&settings));
    let _ = app.emit("squid://security", &view);
    if settings.security.notify_on_findings && view.confirmed > 0 {
        crate::notifications::notify(
            &app,
            &format!(
                "Squid found {} possible threat{}",
                view.confirmed,
                if view.confirmed == 1 { "" } else { "s" }
            ),
            // The unfinished case says so on the notification itself: a clean
            // result from a partial scan must never be read as an all-clear.
            if view.coverage_complete {
                "Open Security to review what was found."
            } else {
                "Open Security — part of this scan could not be checked."
            },
        );
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    /// What the operation did, in one line, for the toast.
    pub message: String,
    pub status: SecurityStatus,
}

/// Stop a running scan. The frontend may call this with the id of a scan it
/// started from the page or from the tray, and cancelling is always allowed: a
/// scan that cannot be stopped is a scan that owns the machine.
/// Bring the window forward for a notification click. The webview cannot unhide
/// itself, and a click that appeared to do nothing is worse than no notification.
#[tauri::command]
pub fn security_open(app: AppHandle) {
    crate::notifications::open_target(&app);
}

#[tauri::command]
pub fn security_cancel(app: AppHandle, job_id: String) -> bool {
    crate::app::cancel_job(app, job_id)
}

#[tauri::command]
pub async fn security_quarantine(finding_id: String) -> Result<ActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let finding = {
            let store = squid_guard::store::Store::new();
            store
                .finding(&finding_id)
                .cloned()
                .ok_or_else(|| "This finding is no longer in the last scan.".to_string())?
        };
        if finding.quarantined {
            return Err("This file is already in the quarantine.".into());
        }
        // The quarantine re-verifies hash, size and identity against what the
        // scan recorded, so a file replaced since the scan is refused here
        // rather than moved on the strength of a stale row.
        let mut held = Quarantine::new();
        let entry = held.quarantine(&finding)?;
        let mut store = squid_guard::store::Store::new();
        store.update_scan(|scan| {
            if let Some(tracked) = scan.findings.iter_mut().find(|f| f.id == finding_id) {
                tracked.quarantined = true;
            }
        });
        store.note(
            "quarantine",
            entry.title.clone(),
            Some(entry.original_path.clone()),
        );
        Ok(ActionResult {
            message: format!("Quarantined {}", entry.display_path),
            status: status_now(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn security_restore(id: String) -> Result<ActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut held = Quarantine::new();
        let restored = held.restore(&id)?;
        let mut store = squid_guard::store::Store::new();
        store.update_scan(|scan| {
            for finding in &mut scan.findings {
                if finding
                    .path
                    .as_ref()
                    .is_some_and(|candidate| paths::same_path(candidate, &restored))
                {
                    finding.quarantined = false;
                }
            }
        });
        store.note("restore", paths::pretty(&restored), Some(restored.clone()));
        Ok(ActionResult {
            message: format!("Put back {}", paths::pretty(&restored)),
            status: status_now(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn security_delete_quarantine(id: String) -> Result<ActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut held = Quarantine::new();
        let display = held
            .find(&id)
            .map(|entry| entry.display_path.clone())
            .unwrap_or_else(|| id.clone());
        held.purge(&id)?;
        let mut store = squid_guard::store::Store::new();
        store.note("purge", format!("Deleted {display} permanently"), None);
        Ok(ActionResult {
            message: format!("Deleted {display} for good"),
            status: status_now(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Ignore or un-ignore one finding. The fingerprint, not the id, is stored,
/// because a rescan issues new ids for the same file and the point of ignoring
/// something is that it stops coming back.
#[tauri::command]
pub async fn security_ignore(finding_id: String, ignored: bool) -> Result<ActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let fingerprint = {
            let store = squid_guard::store::Store::new();
            store
                .finding(&finding_id)
                .map(|finding| finding.fingerprint())
                .ok_or_else(|| "This finding is no longer in the last scan.".to_string())?
        };
        let (mut settings, _) = Settings::load();
        if ignored {
            settings.security.ignore(fingerprint.clone());
        } else {
            settings.security.unignore(&fingerprint);
        }
        settings
            .save()
            .map_err(|error| format!("Could not save settings: {error}"))?;
        let mut store = squid_guard::store::Store::new();
        store.update_scan(|scan| {
            if let Some(finding) = scan.findings.iter_mut().find(|f| f.id == finding_id) {
                finding.ignored = ignored;
            }
        });
        Ok(ActionResult {
            message: if ignored {
                "Ignored — future scans will hide this finding.".to_string()
            } else {
                "This finding will be reported again.".to_string()
            },
            status: status_now(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Trust one piece of software so its ordinary advisories stop asking for a
/// decision. The frontend names a finding and a scope; the path and publisher
/// come from the stored scan, not from the webview.
#[tauri::command]
pub async fn security_trust(finding_id: String, scope: String) -> Result<ActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let finding = {
            let store = squid_guard::store::Store::new();
            store
                .finding(&finding_id)
                .cloned()
                .ok_or_else(|| "This finding is no longer in the last scan.".to_string())?
        };
        let entry = trust_entry_for(&finding, &scope)?;
        let message = match entry.kind {
            AllowKind::Publisher => format!("Trusted publisher {}", entry.value),
            AllowKind::Path => format!("Trusted {}", paths::pretty(&paths::expand(&entry.value))),
            AllowKind::Evidence => format!("No longer reporting {} advisories", entry.value),
        };
        let (mut settings, _) = Settings::load();
        settings.security.allow(entry.kind, entry.value);
        settings
            .save()
            .map_err(|error| format!("Could not save settings: {error}"))?;
        let mut store = squid_guard::store::Store::new();
        store.note("trust", message.clone(), finding.path.clone());
        Ok(ActionResult {
            message,
            status: status_now(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Remove a trust rule. The next status is rebuilt from the stored scan, so the
/// findings it covered become actionable again without waiting for a rescan.
#[tauri::command]
pub async fn security_untrust(kind: String, value: String) -> Result<ActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let kind =
            AllowKind::parse(&kind).ok_or_else(|| format!("Unknown trust rule kind {kind}."))?;
        let (mut settings, _) = Settings::load();
        if !settings.security.remove_allow(kind, value.trim()) {
            return Err("That trust rule was not present.".into());
        }
        settings
            .save()
            .map_err(|error| format!("Could not save settings: {error}"))?;
        Ok(ActionResult {
            message: "This software will be reported again.".into(),
            status: status_now(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn trust_entry_for(finding: &Finding, scope: &str) -> Result<AllowEntry, String> {
    if finding.confidence == Confidence::Confirmed {
        return Err("Confirmed threats cannot be trusted.".into());
    }
    match scope {
        "publisher" => {
            let publisher = finding
                .signed
                .then_some(finding.publisher.as_deref())
                .flatten()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "This finding has no verified publisher to trust.".to_string())?;
            Ok(AllowEntry {
                kind: AllowKind::Publisher,
                value: publisher.to_string(),
            })
        }
        "folder" => {
            let path = finding
                .path
                .as_deref()
                .ok_or_else(|| "This finding has no file to trust.".to_string())?;
            let root = trust_root(path).ok_or_else(|| {
                "This file is too close to a broad system folder to trust by path.".to_string()
            })?;
            Ok(AllowEntry {
                kind: AllowKind::Path,
                value: root.to_string_lossy().into_owned(),
            })
        }
        "evidence" => {
            if finding.confidence != Confidence::Advisory {
                return Err("Only advisory notes can be hidden by type.".into());
            }
            let mut kinds: Vec<&str> = finding
                .evidence
                .iter()
                .filter_map(squid_guard::allowlist::evidence_kind)
                .collect();
            kinds.sort_unstable();
            kinds.dedup();
            match kinds.as_slice() {
                [kind] => Ok(AllowEntry {
                    kind: AllowKind::Evidence,
                    value: (*kind).to_string(),
                }),
                [] => Err("This finding has no trustable advisory class.".into()),
                _ => Err(
                    "This finding has several advisory classes; trust its folder or publisher instead."
                        .into(),
                ),
            }
        }
        other => Err(format!("Unknown trust scope {other}.")),
    }
}

/// The app bundle when there is one, otherwise the containing folder. Broad
/// folders are refused: one click must not trust every app on the machine.
fn trust_root(path: &std::path::Path) -> Option<PathBuf> {
    let mut bundle = None;
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        if component
            .as_os_str()
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".app")
        {
            bundle = Some(prefix.clone());
        }
    }
    let root = bundle.or_else(|| path.parent().map(std::path::Path::to_path_buf))?;
    if paths::is_root_like(&root) {
        return None;
    }
    let home = squid_platform::startup::home_dir();
    let mut broad = vec![
        home.clone(),
        home.join("Applications"),
        home.join("Desktop"),
        home.join("Documents"),
        home.join("Downloads"),
        home.join("Library"),
        home.join("Library/Application Support"),
        home.join("Library/Caches"),
        home.join("Library/LaunchAgents"),
        home.join("Library/LaunchDaemons"),
        home.join("Library/Preferences"),
        PathBuf::from("/Applications"),
        PathBuf::from("/Library"),
        PathBuf::from("/Library/Application Support"),
        PathBuf::from("/Library/LaunchAgents"),
        PathBuf::from("/Library/LaunchDaemons"),
        PathBuf::from("/Users/Shared"),
        PathBuf::from("/System"),
        PathBuf::from("/usr"),
        PathBuf::from("/private"),
        PathBuf::from("/tmp"),
        PathBuf::from("/var"),
    ];
    for key in [
        "LOCALAPPDATA",
        "APPDATA",
        "ProgramData",
        "ProgramFiles",
        "ProgramFiles(x86)",
    ] {
        if let Some(path) = std::env::var_os(key) {
            broad.push(PathBuf::from(path));
        }
    }
    if broad.iter().any(|broad| paths::same_path(broad, &root)) {
        return None;
    }
    Some(root)
}

/// Add or drop a scan exclusion. Separate from the cleanup whitelist on purpose,
/// and the UI says so: protecting a folder from *cleaning* must not stop a
/// malware finding from being reported there.
#[tauri::command]
pub async fn security_exclusions(
    action: String,
    path: String,
) -> Result<SecuritySettingsView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (mut settings, _) = Settings::load();
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err("Choose a folder to exclude.".into());
        }
        match action.as_str() {
            "add" => {
                if !paths::expand(trimmed).exists() {
                    return Err(format!("{trimmed} does not exist."));
                }
                settings.security.exclude(trimmed.to_string());
            }
            "remove" => {
                if !settings.security.remove_exclusion(trimmed) {
                    return Err(format!("{trimmed} was not excluded."));
                }
            }
            other => return Err(format!("Unknown exclusion action {other}.")),
        }
        settings
            .save()
            .map_err(|error| format!("Could not save settings: {error}"))?;
        Ok(SecuritySettingsView::current())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecurityPatch {
    pub scan_schedule: Option<String>,
    pub notify_on_findings: Option<bool>,
    pub auto_quarantine: Option<bool>,
    pub update_rules: Option<bool>,
}

#[tauri::command]
pub async fn security_settings_save(
    app: AppHandle,
    patch: SecurityPatch,
) -> Result<SecuritySettingsView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (mut settings, _) = Settings::load();
        if let Some(schedule) = patch.scan_schedule {
            settings.security.scan_schedule = squid_core::settings::ScanSchedule::parse(&schedule)
                .ok_or_else(|| format!("Unknown scan schedule {schedule}."))?;
        }
        if let Some(notify) = patch.notify_on_findings {
            settings.security.notify_on_findings = notify;
        }
        if let Some(auto) = patch.auto_quarantine {
            settings.security.auto_quarantine = auto;
        }
        if let Some(update) = patch.update_rules {
            settings.security.update_rules = update;
        }
        settings
            .save()
            .map_err(|error| format!("Could not save settings: {error}"))?;
        // A schedule that just changed must not wait for the deadline set by the
        // old one.
        if let Some(state) = app.try_state::<SecurityState>() {
            if let Ok(wake) = state.wake.lock() {
                if let Some(sender) = wake.as_ref() {
                    let _ = sender.send(());
                }
            }
        }
        Ok(SecuritySettingsView::current())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn status_now() -> SecurityStatus {
    tauri::async_runtime::block_on(security_status()).unwrap_or_else(|_| SecurityStatus {
        protection: ProtectionStatus::default(),
        intel: squid_guard::intel::status(),
        scan: None,
        quarantine: Vec::new(),
        quarantine_size: 0,
        activity: Vec::new(),
        settings: SecuritySettingsView::current(),
        permissions: crate::permissions::permission_status(),
        paths: SecurityPaths {
            quarantine: String::new(),
            intel: String::new(),
            startup_backups: String::new(),
        },
        confirmed: 0,
        suspicious: 0,
    })
}

/// The last time a scan finished, in milliseconds, or `None` when this machine
/// has never been scanned.
fn last_scan_ms() -> Option<u64> {
    squid_guard::store::Store::new()
        .scan()
        .map(|scan| scan.finished_ms)
}

/// Fetch and install a signed rule pack. Returns the new intel status, with a
/// message so the UI can say what actually happened rather than "done".
#[tauri::command]
pub async fn security_update_rules() -> Result<IntelStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let (settings, _) = Settings::load();
        if !settings.security.update_rules {
            return Err("Rule-pack updates are switched off in Settings.".into());
        }
        update_pack()?;
        Ok(squid_guard::intel::status())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn security_rollback_rules() -> Result<IntelStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        squid_guard::intel::rollback()?;
        // Verdicts recorded under the rejected pack are not answers the rolled
        // back pack gave, so they go with it.
        squid_guard::store::ScanCache::new().invalidate();
        Ok(squid_guard::intel::status())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// One update round-trip: manifest, signature, pack, then install. Every failure
/// leaves the previously installed pack in place, which is the whole point of
/// keeping `previous.json`.
pub(crate) fn update_pack() -> Result<IntelStatus, String> {
    let manifest_url = crate::update::intel_manifest_url()
        .ok_or_else(|| "No rule-pack feed is configured for this build.".to_string())?;
    let text = crate::update::fetch_text(&manifest_url).map_err(|error| {
        if error.contains("404") {
            "No rule pack has been published for this release yet. Squid looks for \
             squid-intel.json next to squid-update.json on the releases branch."
                .to_string()
        } else {
            error
        }
    })?;
    let signature = crate::update::intel_manifest_signature()?;
    let key = crate::update::intel_public_key()?;
    // Two answers, not one. A build that was given an intel key must verify
    // against it or the update is refused — that is the whole value of the key.
    // A build that was given none (any build from source, and the release build
    // until a key is baked) cannot verify, and refusing there would mean rule
    // packs never update for anyone who is not the release process. That case is
    // accepted with its provenance recorded, so the Security page can say the
    // rules were checked by hash and not by signature, instead of either failing
    // or implying a check that never happened.
    let provenance = match &key {
        Some(_) => {
            if !crate::update::verify_against(key.as_ref(), &text, signature.as_deref())? {
                return Err(
                    "The rule manifest signature does not match the key this build trusts.".into(),
                );
            }
            squid_guard::intel::Provenance::Verified
        }
        None => squid_guard::intel::Provenance::Unverified,
    };
    let manifest = squid_guard::intel::parse_manifest(&text)?;
    let current = squid_guard::intel::status();
    if current.revision >= manifest.revision {
        return Ok(current);
    }
    let directory = std::env::temp_dir().join(format!("squid-intel-{}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let result = (|| -> Result<IntelStatus, String> {
        let destination = directory.join(&manifest.pack);
        crate::update::download(
            &crate::update::raw_asset_url(&manifest.pack)?,
            &destination,
            std::time::Duration::from_secs(120),
            || {},
        )?;
        let pack = squid_guard::intel::read_pack(&directory, &manifest, provenance)?;
        squid_guard::intel::install_pack(&pack)?;
        squid_guard::store::ScanCache::new().invalidate();
        Ok(squid_guard::intel::status())
    })();
    let _ = std::fs::remove_dir_all(&directory);
    result
}

#[tauri::command]
pub fn open_quarantine_folder() -> Result<(), String> {
    let root = squid_guard::coverage::quarantine_root();
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    let program = "/usr/bin/open";
    #[cfg(windows)]
    let program = "explorer.exe";
    #[cfg(not(any(target_os = "macos", windows)))]
    let program = "xdg-open";
    let status = crate::process::command(program)
        .arg(&root)
        .status()
        .map_err(|error| format!("Could not open the quarantine folder: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Could not open the quarantine folder.".into())
    }
}

/// The timer thread behind the scan schedule. It does not poll: it waits on a
/// channel until the next scan is due, or until a settings change wakes it, so an
/// idle tray process costs one parked thread and no CPU. There is no daemon, and
/// this is not one — it exists only while the app is running, which is exactly
/// what the schedule in Settings promises.
pub fn spawn_scheduler(app: &AppHandle, settings: &Settings) {
    let handle = app.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    if let Some(state) = app.try_state::<SecurityState>() {
        if let Ok(mut wake) = state.wake.lock() {
            *wake = Some(sender.clone());
        }
    }
    // `Manual` still starts the thread: switching schedules from Settings must
    // take effect without restarting the app, and the thread decides on each wake.
    let _ = settings;
    std::thread::spawn(move || loop {
        let (current, _) = Settings::load();
        let interval = current.security.scan_schedule.interval_secs();
        let wait = match interval {
            // Nothing is scheduled; sleep an hour and re-read, at no measurable cost.
            None => std::time::Duration::from_secs(3_600),
            Some(secs) => match last_scan_ms() {
                // `OnLaunch` means "once per start", so the floor is a minute
                // after the last scan rather than zero on every wake.
                Some(finished) => {
                    let floor_ms = if secs == 0 {
                        60_000
                    } else {
                        secs as u64 * 1_000
                    };
                    let due = finished.saturating_add(floor_ms);
                    std::time::Duration::from_millis(due.saturating_sub(now_ms()))
                }
                None => std::time::Duration::ZERO,
            },
        };
        // A timeout means the deadline passed; a message means settings changed
        // and the wait has to be recomputed from the schedule now in the file.
        if receiver.recv_timeout(wait).is_err() && interval.is_some() {
            scan_in_background(handle.clone(), ScanTarget::Quick);
            // The scan runs on its own thread and only records a finish when it
            // ends, so re-checking immediately would find the same scan still due
            // and spin. Park for a minute instead, and stay wake-able.
            let _ = receiver.recv_timeout(std::time::Duration::from_secs(60));
        }
    });
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

/// When the last scan finished, formatted for the tray and the overview strip.
pub fn last_scan_label() -> Option<String> {
    let store = squid_guard::store::Store::new();
    let finished_ms = store.scan()?.finished_ms;
    let seconds = (finished_ms / 1_000) as i64;
    let stamp = DateTime::<Local>::from(chrono::DateTime::from_timestamp(seconds, 0)?);
    Some(stamp.format("%Y-%m-%d %H:%M").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use squid_guard::findings::Evidence;
    use std::path::{Path, PathBuf};

    fn finding(id: &str, confidence: Confidence, ignored: bool, quarantined: bool) -> Finding {
        Finding {
            id: id.into(),
            confidence,
            path: Some(PathBuf::from(format!("/tmp/{id}"))),
            display_path: Some(format!("/tmp/{id}")),
            name: id.into(),
            title: "test".into(),
            description: String::new(),
            evidence: vec![Evidence::Unsigned {
                path: PathBuf::from(format!("/tmp/{id}")),
                note: None,
            }],
            size: 1,
            sha256: None,
            signed: false,
            publisher: None,
            found_at_ms: 0,
            quarantined,
            ignored,
        }
    }

    fn scan_with(findings: Vec<Finding>) -> StoredScan {
        StoredScan {
            scan_id: "scan-1".into(),
            mode: "quick".into(),
            finished_ms: 1,
            elapsed_ms: 1,
            cancelled: false,
            examined: findings.len(),
            cached: 0,
            skipped: 0,
            coverage_complete: true,
            gaps: Vec::new(),
            findings,
        }
    }

    #[test]
    fn a_dealt_with_finding_stops_being_counted_as_outstanding() {
        // The tray badge and the page must agree, and both must stop counting a
        // file that has been quarantined or ignored. Otherwise "3 issues found"
        // follows a user who has already handled all three.
        let view = ScanView::from_stored(
            &scan_with(vec![
                finding("confirmed-open", Confidence::Confirmed, false, false),
                finding("confirmed-held", Confidence::Confirmed, false, true),
                finding("confirmed-dismissed", Confidence::Confirmed, true, false),
                finding("suspicious-open", Confidence::Suspicious, false, false),
            ]),
            &[],
        );
        assert_eq!(
            view.confirmed, 1,
            "only the unhandled confirmed finding counts"
        );
        assert_eq!(view.suspicious, 1);
        assert!(
            !view.clean,
            "a scan with findings is never reported as clean"
        );
    }

    #[test]
    fn a_scan_with_gaps_is_never_called_clean() {
        let mut stored = scan_with(Vec::new());
        assert!(ScanView::from_stored(&stored, &[]).clean);
        stored.coverage_complete = false;
        stored.gaps = vec![("/protected".into(), "permission denied".into())];
        let view = ScanView::from_stored(&stored, &[]);
        assert!(
            !view.clean,
            "an unreadable folder is not the same as a clean one"
        );
        assert_eq!(view.gaps.len(), 1);
        stored.cancelled = true;
        assert!(!ScanView::from_stored(&stored, &[]).clean);
    }

    #[test]
    fn a_trusted_finding_stops_counting_without_hiding_confirmed_ones() {
        let stored = scan_with(vec![
            finding("advisory-trusted", Confidence::Advisory, false, false),
            finding("confirmed-trusted", Confidence::Confirmed, false, false),
        ]);
        let allowlist = vec![AllowEntry {
            kind: AllowKind::Path,
            value: "/tmp".into(),
        }];
        let view = ScanView::from_stored(&stored, &allowlist);
        assert_eq!(view.advisory, 0, "the trusted advisory is handled");
        assert_eq!(
            view.confirmed, 1,
            "a confirmed finding is never hidden by trust"
        );
        assert_eq!(
            view.findings
                .iter()
                .filter(|finding| finding.allowlisted)
                .count(),
            1
        );
    }

    #[test]
    fn trust_by_path_stops_at_the_bundle_and_refuses_broad_folders() {
        assert_eq!(
            trust_root(Path::new(
                "/Applications/Example.app/Contents/MacOS/Example"
            )),
            Some(PathBuf::from("/Applications/Example.app"))
        );
        assert_eq!(
            trust_root(Path::new("/Applications")),
            None,
            "one click must not trust every app"
        );
        assert_eq!(trust_root(Path::new("/tmp/example")), None);
    }

    #[test]
    fn trust_refuses_confirmed_findings_and_suspicious_evidence_rules() {
        let confirmed = finding("confirmed", Confidence::Confirmed, false, false);
        assert!(trust_entry_for(&confirmed, "folder").is_err());
        let suspicious = finding("suspicious", Confidence::Suspicious, false, false);
        assert!(trust_entry_for(&suspicious, "evidence").is_err());
    }

    #[test]
    fn a_custom_scan_needs_a_path_and_is_capped() {
        assert!(target_for("quick", &[]).is_ok());
        assert!(target_for("full", &[]).is_ok());
        assert!(
            target_for("custom", &[]).is_err(),
            "an empty pick is not a scope"
        );
        assert!(
            target_for("custom", &["  ".into()]).is_err(),
            "blank is empty"
        );
        assert!(
            target_for("sideways", &[]).is_err(),
            "an unknown mode is a bug, not quick"
        );
        let many: Vec<String> = (0..200).map(|index| format!("/tmp/{index}")).collect();
        match target_for("custom", &many).unwrap() {
            ScanTarget::Paths(chosen) => assert_eq!(chosen.len(), 64, "a folder pick is bounded"),
            other => panic!("custom must stay custom, got {other:?}"),
        }
    }
}
