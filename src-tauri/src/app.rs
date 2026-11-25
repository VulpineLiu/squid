//! Tauri command surface for the desktop application.
//!
//! The UI never invents a deletion target. Scans are held in this process and
//! a delete command can only select indices from the scan that produced them.
//! The engine still re-validates every path against the same safety policy and
//! whitelist used by the CLI and TUI.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use squid_core::ledger::{self, Session};
use squid_core::paths;
use squid_core::progress::{CancelToken, Event, Progress};
use squid_core::{
    AppInfo, Bytes, Category, CleanupItem, DeleteMode, ItemKind, Locale, LockReason, ScanReport,
    Settings, UninstallPlan,
};
use squid_engine::analyze::{self, FileEntry, Node};
use squid_engine::delete::{self, DeleteJob, DeleteOutcome};
use squid_engine::{clean, purge, uninstall, Engine};
use squid_platform::PlatformId;
use squid_stats::{ProcessSample, Sampler, SystemSnapshot};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct AppState {
    jobs: Mutex<HashMap<String, CancelToken>>,
    scans: Mutex<HashMap<String, ScanReport>>,
    /// Confirmed duplicate groups from the last `duplicates_scan`, keyed by
    /// scan id — deletes may only name (group, copy) indices from it.
    dup_scans: Mutex<HashMap<String, squid_engine::duplicates::DupReport>>,
    plans: Mutex<HashMap<String, UninstallPlan>>,
    apps: Mutex<Vec<AppInfo>>,
    app_icons: Mutex<HashMap<String, Option<String>>>,
    sampler: Mutex<Sampler>,
    /// Last light snapshot, so the widget window and the main window polling
    /// a beat apart share one refresh instead of paying for two.
    snapshot_cache: Mutex<Option<(Instant, squid_stats::SystemSnapshot)>>,
    /// Mole-style size cache (A⑥): explored storage trees answered from
    /// memory until a delete run bumps the epoch and invalidates them.
    /// Stamp is wall-clock so the age survives nicely for UI copy.
    storage_cache: Mutex<HashMap<String, (u64, std::time::SystemTime, analyze::Node)>>,
    clean_epoch: std::sync::atomic::AtomicU64,
}

impl AppState {
    fn lock<T>(value: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
        value
            .lock()
            .map_err(|_| "The application state is unavailable; restart Squid.".to_string())
    }

    fn epoch(&self) -> u64 {
        self.clean_epoch.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Any successful delete invalidates cached storage sizes (Mole bumps its
    /// cache schema whenever deletion semantics change what totals mean).
    fn invalidate_storage_cache(&self) {
        self.clean_epoch
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn begin(&self, job_id: &str) -> Result<CancelToken, String> {
        if job_id.trim().is_empty() {
            return Err("A job identifier is required.".into());
        }
        let token = CancelToken::new();
        let mut jobs = Self::lock(&self.jobs)?;
        jobs.insert(job_id.to_string(), token.clone());
        Ok(token)
    }

    pub(crate) fn finish(&self, job_id: &str) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(job_id);
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressView {
    job_id: String,
    kind: String,
    event: String,
    phase: Option<String>,
    detail: Option<String>,
    done: Option<u64>,
    total: Option<u64>,
    subject: Option<String>,
    freed: Option<u64>,
}

pub(crate) struct ProgressReporter {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ProgressReporter {
    pub(crate) fn start(
        app: AppHandle,
        job_id: String,
        kind: &'static str,
        progress: Progress,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handle = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                if let Some(event) = progress.recv(Duration::from_millis(80)) {
                    emit_progress(&app, &job_id, kind, event);
                }
            }
            for event in progress.drain() {
                emit_progress(&app, &job_id, kind, event);
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    pub(crate) fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ProgressReporter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn emit_progress(app: &AppHandle, job_id: &str, kind: &str, event: Event) {
    let payload = match event {
        Event::Phase { phase, detail } => ProgressView {
            job_id: job_id.into(),
            kind: kind.into(),
            event: "phase".into(),
            phase: Some(phase.into()),
            detail: Some(detail),
            done: None,
            total: None,
            subject: None,
            freed: None,
        },
        Event::Tick {
            done,
            total,
            subject,
        } => ProgressView {
            job_id: job_id.into(),
            kind: kind.into(),
            event: "tick".into(),
            phase: None,
            detail: None,
            done: Some(done),
            total,
            subject: subject.map(|path| paths::pretty(&path)),
            freed: None,
        },
        Event::Note(note) => ProgressView {
            job_id: job_id.into(),
            kind: kind.into(),
            event: "note".into(),
            phase: None,
            detail: Some(note),
            done: None,
            total: None,
            subject: None,
            freed: None,
        },
        Event::Warning(warning) => ProgressView {
            job_id: job_id.into(),
            kind: kind.into(),
            event: "warning".into(),
            phase: None,
            detail: Some(warning),
            done: None,
            total: None,
            subject: None,
            freed: None,
        },
        Event::Deleted { path, freed } => ProgressView {
            job_id: job_id.into(),
            kind: kind.into(),
            event: "deleted".into(),
            phase: None,
            detail: Some(paths::pretty(&path)),
            done: None,
            total: None,
            subject: None,
            freed: Some(freed.as_u64()),
        },
        Event::Bucket {
            category,
            total,
            count,
        } => ProgressView {
            job_id: job_id.into(),
            kind: kind.into(),
            event: "bucket".into(),
            phase: None,
            detail: Some(format!("{}:{}:{}", category.id(), count, total.as_u64())),
            done: None,
            total: None,
            subject: None,
            freed: None,
        },
        Event::Metered { files, bytes } => ProgressView {
            job_id: job_id.into(),
            kind: kind.into(),
            event: "metered".into(),
            phase: None,
            detail: None,
            done: Some(files),
            total: Some(bytes.as_u64()),
            subject: None,
            freed: None,
        },
        Event::Group {
            kind: unit,
            payload,
        } => ProgressView {
            job_id: job_id.into(),
            kind: kind.into(),
            event: "group".into(),
            phase: None,
            // `dup:{json}` — one confirmed work unit, streamed live.
            detail: Some(format!("{unit}:{payload}")),
            done: None,
            total: None,
            subject: None,
            freed: None,
        },
        Event::Done(_) => return,
    };
    let _ = app.emit("squid://progress", payload);
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItemView {
    path: String,
    display_path: String,
    kind: ItemKind,
    size: u64,
    allocated: u64,
    on_disk: u64,
    category: Category,
    label: String,
    rule_id: String,
    selected: bool,
    lock: Option<LockReason>,
    note: Option<String>,
    last_write_secs: Option<u64>,
    file_count: u64,
    evidence: Vec<String>,
}

impl From<&CleanupItem> for CleanupItemView {
    fn from(item: &CleanupItem) -> Self {
        Self {
            path: item.path.to_string_lossy().to_string(),
            display_path: item.display_path(),
            kind: item.kind,
            size: item.size.as_u64(),
            allocated: item.allocated.as_u64(),
            on_disk: item.on_disk().as_u64(),
            category: item.category,
            label: item.label.clone(),
            rule_id: item.rule_id.clone(),
            selected: item.selected,
            lock: item.lock,
            note: item.note.clone(),
            last_write_secs: item.last_write_secs,
            file_count: item.file_count,
            evidence: item.evidence.clone(),
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanView {
    scan_id: String,
    items: Vec<CleanupItemView>,
    total: u64,
    total_selected: u64,
    selected_count: usize,
    unreadable: u32,
    elapsed_ms: u64,
}

fn scan_view(scan_id: String, report: &ScanReport, elapsed_ms: u64) -> ScanView {
    let selected_total = report
        .items
        .iter()
        .filter(|item| item.selected && item.lock.is_none())
        .map(|item| item.on_disk().as_u64())
        .sum();
    ScanView {
        scan_id,
        items: report.items.iter().map(CleanupItemView::from).collect(),
        total: report.total_all().as_u64(),
        total_selected: selected_total,
        selected_count: report
            .items
            .iter()
            .filter(|item| item.selected && item.lock.is_none())
            .count(),
        unreadable: report.unreadable,
        elapsed_ms,
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedView {
    path: String,
    reason: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResultView {
    freed: u64,
    freed_allocated: u64,
    removed: usize,
    failed: usize,
    errors: Vec<String>,
    mode: Option<DeleteMode>,
    session_id: Option<String>,
    skipped: Vec<SkippedView>,
}

impl DeleteResultView {
    fn from_outcome(outcome: DeleteOutcome) -> Self {
        Self {
            freed: outcome.summary.freed.as_u64(),
            freed_allocated: outcome.summary.freed_allocated.as_u64(),
            removed: outcome.summary.removed,
            failed: outcome.summary.failed,
            errors: outcome.summary.errors,
            mode: outcome.summary.mode,
            session_id: outcome.summary.session_id,
            skipped: outcome
                .skipped
                .into_iter()
                .map(|(path, reason)| SkippedView {
                    path: paths::pretty(&path),
                    reason,
                })
                .collect(),
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    delete_mode: DeleteMode,
    whitelist: Vec<String>,
    purge_roots: Vec<String>,
    purge_min_age_days: u64,
    show_allocated: bool,
    language: Option<String>,
    effective_language: String,
    history_keep: usize,
    setup_reviewed: bool,
    errors: Vec<String>,
}

impl SettingsView {
    fn from_settings(settings: &Settings, errors: Vec<String>) -> Self {
        Self {
            delete_mode: settings.delete_mode,
            whitelist: settings.whitelist.clone(),
            purge_roots: settings.purge_roots.clone(),
            purge_min_age_days: settings.purge_min_age_days,
            show_allocated: settings.show_allocated,
            language: settings.language.clone(),
            effective_language: Locale::detect(settings).code().into(),
            history_keep: settings.history_keep,
            setup_reviewed: settings.desktop_setup_reviewed,
            errors,
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapView {
    version: String,
    platform: String,
    os_name: String,
    os_version: String,
    hardware: String,
    /// Static component facts (board/RAM/disk/GPU) for the overview panel.
    hardware_info: squid_platform::HardwareInfo,
    arch: String,
    device: String,
    language: String,
    bin_noun: String,
    config_path: String,
    history_path: String,
    update_feed_url: String,
    update_check_enabled: bool,
    settings: SettingsView,
}

#[tauri::command]
pub fn bootstrap() -> BootstrapView {
    let (settings, errors) = Settings::load();
    let engine = Engine::new();
    let locale = squid_core::Locale::detect(&settings);
    let prefs = crate::update::load_prefs();
    BootstrapView {
        version: env!("CARGO_PKG_VERSION").into(),
        platform: PlatformId::current().as_str().into(),
        os_name: engine.platform().display_name(),
        os_version: engine.platform().os_version(),
        hardware: engine.platform().hardware_model(),
        hardware_info: squid_platform::hardware_info(),
        arch: std::env::consts::ARCH.into(),
        device: match engine.platform().device() {
            squid_platform::Device::Mac => "mac".into(),
            squid_platform::Device::Pc => "pc".into(),
            squid_platform::Device::Machine => "machine".into(),
        },
        language: locale.code().into(),
        bin_noun: match engine.platform().bin_noun() {
            squid_platform::BinNoun::Trash => "trash".into(),
            squid_platform::BinNoun::RecycleBin => "recycle-bin".into(),
        },
        config_path: Settings::path().to_string_lossy().to_string(),
        history_path: ledger::history_dir().to_string_lossy().to_string(),
        update_feed_url: crate::update::manifest_url(),
        update_check_enabled: prefs.update_check_enabled,
        settings: SettingsView::from_settings(&settings, errors),
    }
}

#[tauri::command]
pub fn cancel_job(app: AppHandle, job_id: String) -> bool {
    let state = app.state::<AppState>();
    let Ok(jobs) = AppState::lock(&state.jobs) else {
        return false;
    };
    let Some(token) = jobs.get(&job_id) else {
        return false;
    };
    token.cancel();
    true
}

fn parse_categories(values: &[String]) -> Result<Vec<Category>, String> {
    let mut categories = Vec::new();
    for value in values {
        let category =
            Category::parse(value).ok_or_else(|| format!("Unknown cleanup category: {value}"))?;
        if !categories.contains(&category) {
            categories.push(category);
        }
    }
    Ok(categories)
}

#[tauri::command]
pub async fn cleanup_scan(
    app: AppHandle,
    job_id: String,
    categories: Vec<String>,
    min_age_days: Option<u64>,
    orphans: bool,
) -> Result<ScanView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let categories = parse_categories(&categories)?;
    let progress = Progress::new();
    let reporter =
        ProgressReporter::start(app.clone(), job_id.clone(), "cleanup", progress.clone());
    let started = Instant::now();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        let options = clean::CleanOptions {
            categories,
            skip_admin_rules: true,
            min_age_days,
            orphans,
            fast: false,
        };
        let report = clean::scan(&engine, &options, &progress, &token)?;
        clean::publish_buckets(&report, &progress);
        Ok::<ScanReport, squid_core::SquidError>(clean::sort_for_display(report))
    })
    .await
    .map_err(|error| format!("Cleanup scan failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let report = task.map_err(|error| error.to_string())?;
    let view = scan_view(
        job_id.clone(),
        &report,
        started.elapsed().as_millis() as u64,
    );
    app.state::<AppState>()
        .scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .insert(job_id, report);
    Ok(view)
}

fn select_indices(report: &mut ScanReport, selected: &[usize]) -> Result<(), String> {
    let selected: HashSet<usize> = selected.iter().copied().collect();
    if let Some(index) = selected.iter().find(|index| **index >= report.items.len()) {
        return Err(format!("Selected item {index} is not in this scan."));
    }
    for (index, item) in report.items.iter_mut().enumerate() {
        item.selected = selected.contains(&index) && item.lock.is_none();
    }
    Ok(())
}

#[tauri::command]
pub async fn cleanup_delete(
    app: AppHandle,
    job_id: String,
    scan_id: String,
    selected: Vec<usize>,
    mode: DeleteMode,
) -> Result<DeleteResultView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let mut report = app
        .state::<AppState>()
        .scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .get(&scan_id)
        .cloned()
        .ok_or_else(|| "This scan has expired. Run it again.".to_string())?;
    select_indices(&mut report, &selected)?;
    let items = report.items;
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "delete", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        let job = DeleteJob::new(items, mode, "Squid · Cleanup");
        delete::execute(&engine, &job, &progress, &token)
    })
    .await
    .map_err(|error| format!("Cleanup failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let outcome = task.map_err(|error| error.to_string())?;
    if let Ok(mut scans) = app.state::<AppState>().scans.lock() {
        scans.remove(&scan_id);
    }
    app.state::<AppState>().invalidate_storage_cache();
    Ok(DeleteResultView::from_outcome(outcome))
}

#[tauri::command]
pub async fn purge_scan(
    app: AppHandle,
    job_id: String,
    roots: Vec<String>,
    min_age_days: Option<u64>,
    force: bool,
) -> Result<ScanView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "purge", progress.clone());
    let started = Instant::now();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        let options = purge::PurgeOptions {
            roots: roots.into_iter().map(|root| paths::expand(&root)).collect(),
            min_age_days,
            force,
            max_depth: None,
        };
        let mut report = purge::scan(&engine, &options, &progress, &token)?;
        report.items.sort_by(|a, b| {
            b.on_disk()
                .cmp(&a.on_disk())
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok::<ScanReport, squid_core::SquidError>(report)
    })
    .await
    .map_err(|error| format!("Deep clean scan failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let report = task.map_err(|error| error.to_string())?;
    let view = scan_view(
        job_id.clone(),
        &report,
        started.elapsed().as_millis() as u64,
    );
    app.state::<AppState>()
        .scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .insert(job_id, report);
    Ok(view)
}

#[tauri::command]
pub async fn purge_delete(
    app: AppHandle,
    job_id: String,
    scan_id: String,
    selected: Vec<usize>,
    mode: DeleteMode,
) -> Result<DeleteResultView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let mut report = app
        .state::<AppState>()
        .scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .get(&scan_id)
        .cloned()
        .ok_or_else(|| "This scan has expired. Run it again.".to_string())?;
    select_indices(&mut report, &selected)?;
    let items = report.items;
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "delete", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        let job = DeleteJob::new(items, mode, "Squid · Deep Clean");
        delete::execute(&engine, &job, &progress, &token)
    })
    .await
    .map_err(|error| format!("Deep clean failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let outcome = task.map_err(|error| error.to_string())?;
    if let Ok(mut scans) = app.state::<AppState>().scans.lock() {
        scans.remove(&scan_id);
    }
    app.state::<AppState>().invalidate_storage_cache();
    Ok(DeleteResultView::from_outcome(outcome))
}

/// One duplicate-file scan: three-stage funnel, groups streamed live.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupsView {
    scan_id: String,
    report: squid_engine::duplicates::DupReport,
    elapsed_ms: u64,
}

#[tauri::command]
pub async fn duplicates_scan(
    app: AppHandle,
    job_id: String,
    roots: Vec<String>,
    min_size: Option<u64>,
) -> Result<DupsView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let progress = Progress::new();
    let reporter =
        ProgressReporter::start(app.clone(), job_id.clone(), "duplicates", progress.clone());
    let started = Instant::now();
    let roots: Vec<PathBuf> = if roots.is_empty() {
        squid_engine::duplicates::default_roots()
    } else {
        roots.into_iter().map(|root| paths::expand(&root)).collect()
    };
    let task = tauri::async_runtime::spawn_blocking(move || {
        squid_engine::duplicates::scan(
            &roots,
            min_size.unwrap_or(squid_engine::duplicates::DEFAULT_MIN_SIZE),
            &progress,
            &token,
        )
    })
    .await
    .map_err(|error| format!("Duplicate scan failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let report = task.map_err(|error| error.to_string())?;
    let view = DupsView {
        scan_id: job_id.clone(),
        report,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    app.state::<AppState>()
        .dup_scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .insert(job_id, view.report.clone());
    Ok(view)
}

/// Delete chosen copies of confirmed duplicate groups — Recycle Bin mode by
/// default, so a wrong click is a restore away, never a loss.
#[tauri::command]
pub async fn duplicates_delete(
    app: AppHandle,
    job_id: String,
    scan_id: String,
    selections: Vec<(usize, Vec<usize>)>,
    mode: DeleteMode,
) -> Result<DeleteResultView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let report = app
        .state::<AppState>()
        .dup_scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .get(&scan_id)
        .cloned()
        .ok_or_else(|| "This scan has expired. Run it again.".to_string())?;
    let items = report.items_for(&selections);
    if items.is_empty() {
        app.state::<AppState>().finish(&job_id);
        return Err("没有可删除的副本（每组至少保留一份）".into());
    }
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "delete", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new().with_volume_wide_safety();
        let job = DeleteJob::new(items, mode, "Squid · Duplicates");
        delete::execute(&engine, &job, &progress, &token)
    })
    .await
    .map_err(|error| format!("Duplicate delete failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let outcome = task.map_err(|error| error.to_string())?;
    app.state::<AppState>().invalidate_storage_cache();
    Ok(DeleteResultView::from_outcome(outcome))
}

/// Large-file sweep across drives with a threshold and optional type class.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFileView {
    path: String,
    display_path: String,
    size: u64,
    modified_ms: u64,
    class: &'static str,
    /// A cloud-sync folder sits above this file: 「网盘可能有副本？」.
    cloud_hint: bool,
    /// Disk-image extension that something (Hyper-V/WSL) may hold open.
    maybe_mounted: bool,
}

fn cloud_hint(path: &str) -> bool {
    let lower = path.to_lowercase();
    [
        "onedrive",
        "\\wps cloud",
        "iclouddrive",
        "google drive",
        "googledrive",
        "dropbox",
        "坚果云",
        "syncthing",
        "baidunetdisk",
        "wuying",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[tauri::command]
pub async fn large_files_scan(
    app: AppHandle,
    job_id: String,
    roots: Vec<String>,
    min_bytes: Option<u64>,
    class: Option<String>,
) -> Result<Vec<LargeFileView>, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let progress = Progress::new();
    let reporter =
        ProgressReporter::start(app.clone(), job_id.clone(), "bigfiles", progress.clone());
    let mut roots: Vec<PathBuf> = roots.into_iter().map(|root| paths::expand(&root)).collect();
    if roots.is_empty() {
        roots = squid_engine::duplicates::default_roots();
    }
    let class = class.filter(|value| value != "all");
    let task = tauri::async_runtime::spawn_blocking(move || {
        analyze::large_files(
            &roots,
            min_bytes.unwrap_or(100 * 1024 * 1024),
            class.as_deref(),
            &token,
            &progress,
        )
    })
    .await
    .map_err(|error| format!("Large file scan failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let files = task.map_err(|error| error.to_string())?;
    Ok(files
        .iter()
        .map(|file| {
            let display = file.path.display().to_string();
            let image = matches!(squid_engine::analyze::file_class(&file.path), "image");
            LargeFileView {
                path: display.clone(),
                display_path: paths::pretty(&file.path),
                size: file.size().as_u64(),
                modified_ms: file.modified_ms.unwrap_or(0),
                class: squid_engine::analyze::file_class(&file.path),
                cloud_hint: cloud_hint(&display),
                maybe_mounted: image,
            }
        })
        .collect())
}

/// Delete chosen large files (paths re-validated, Recycle Bin by default).
#[tauri::command]
pub async fn large_files_delete(
    app: AppHandle,
    job_id: String,
    paths: Vec<String>,
    mode: DeleteMode,
) -> Result<DeleteResultView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    if paths.is_empty() {
        app.state::<AppState>().finish(&job_id);
        return Err("没有选中任何文件".into());
    }
    let items: Vec<CleanupItem> = paths
        .iter()
        .map(|raw| {
            let path = PathBuf::from(raw);
            let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            let anchor = path
                .ancestors()
                .nth(2)
                .map(|dir| dir.to_path_buf())
                .unwrap_or_else(paths::home_dir);
            let mut item = CleanupItem::new(
                path.clone(),
                ItemKind::File,
                Category::SystemJunk,
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
            .with_rule("large-files")
            .with_anchors(std::iter::once(anchor));
            item.size = Bytes(size);
            item.allocated = Bytes(size);
            item.file_count = 1;
            item
        })
        .collect();
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "delete", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new().with_volume_wide_safety();
        let job = DeleteJob::new(items, mode, "Squid · Large Files");
        delete::execute(&engine, &job, &progress, &token)
    })
    .await
    .map_err(|error| format!("Large file delete failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let outcome = task.map_err(|error| error.to_string())?;
    app.state::<AppState>().invalidate_storage_cache();
    Ok(DeleteResultView::from_outcome(outcome))
}

/// Maintenance tasks live behind a single global lock key in the jobs map:
/// two repairs racing (SFC while the audio service restarts) can invalidate
/// each other's assumptions, so the second click is refused, not queued.
const MAINTENANCE_LOCK: &str = "maintenance";

#[tauri::command]
pub async fn maintenance_tasks() -> Result<Vec<squid_engine::maintenance::MaintenanceTask>, String>
{
    Ok(squid_engine::maintenance::tasks())
}

#[tauri::command]
pub async fn maintenance_run(
    app: AppHandle,
    job_id: String,
    task: String,
) -> Result<squid_engine::maintenance::MaintenanceResult, String> {
    let state = app.state::<AppState>();
    if job_id.trim().is_empty() {
        return Err("A job identifier is required.".into());
    }
    // One token under both keys: the mutual-exclusion lock and the caller's
    // cancel handle always point at the same flag.
    let token = CancelToken::new();
    {
        let mut jobs = AppState::lock(&state.jobs)?;
        if jobs.contains_key(MAINTENANCE_LOCK) {
            return Err("另一个维护任务正在进行，请先等待它结束。".into());
        }
        jobs.insert(MAINTENANCE_LOCK.to_string(), token.clone());
        jobs.insert(job_id.clone(), token.clone());
    }
    let progress = Progress::new();
    let reporter =
        ProgressReporter::start(app.clone(), job_id.clone(), "maintenance", progress.clone());
    let task_id = task.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        squid_engine::maintenance::run(&task_id, &progress, &token)
    })
    .await
    .map_err(|error| format!("维护任务中断：{error}"));
    reporter.finish();
    if let Ok(mut jobs) = state.jobs.lock() {
        jobs.remove(MAINTENANCE_LOCK);
        jobs.remove(&job_id);
    }
    let outcome = result?.map_err(|error| error.to_string())?;
    Ok(outcome)
}

/// Privacy traces scan: rows never pre-selected, running browsers locked.
#[tauri::command]
pub async fn privacy_scan(app: AppHandle, job_id: String) -> Result<ScanView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let progress = Progress::new();
    let reporter =
        ProgressReporter::start(app.clone(), job_id.clone(), "privacy", progress.clone());
    let started = Instant::now();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        let items = squid_engine::privacy::collect(&engine, &token);
        Ok::<ScanReport, squid_core::SquidError>(ScanReport {
            items,
            scanned_at_ms: 0,
            ..Default::default()
        })
    })
    .await
    .map_err(|error| format!("Privacy scan failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let report = task.map_err(|error| error.to_string())?;
    let view = scan_view(
        job_id.clone(),
        &report,
        started.elapsed().as_millis() as u64,
    );
    app.state::<AppState>()
        .scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .insert(job_id, report);
    Ok(view)
}

/// Privacy deletes are permanent by design (a bin-restored browser profile
/// is broken worse than a clean one) but land in the same history ledger.
#[tauri::command]
pub async fn privacy_delete(
    app: AppHandle,
    job_id: String,
    scan_id: String,
    selected: Vec<usize>,
) -> Result<DeleteResultView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let mut report = app
        .state::<AppState>()
        .scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .get(&scan_id)
        .cloned()
        .ok_or_else(|| "This scan has expired. Run it again.".to_string())?;
    select_indices(&mut report, &selected)?;
    let items = report.items;
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "delete", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        let job = DeleteJob::new(items, DeleteMode::Permanent, "Squid · Privacy");
        delete::execute(&engine, &job, &progress, &token)
    })
    .await
    .map_err(|error| format!("Privacy clean failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let outcome = task.map_err(|error| error.to_string())?;
    if let Ok(mut scans) = app.state::<AppState>().scans.lock() {
        scans.remove(&scan_id);
    }
    Ok(DeleteResultView::from_outcome(outcome))
}

/// Optimisation tweaks: catalog + live state; writes record the original.
#[tauri::command]
pub async fn tweaks_states() -> Result<Vec<squid_engine::tweaks::TweakState>, String> {
    tauri::async_runtime::spawn_blocking(squid_engine::tweaks::states)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn tweaks_set(
    id: String,
    enabled: bool,
) -> Result<squid_engine::tweaks::TweakState, String> {
    tauri::async_runtime::spawn_blocking(move || squid_engine::tweaks::set(&id, enabled))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn tweaks_restore_all() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(squid_engine::tweaks::restore_all)
        .await
        .map_err(|error| error.to_string())
}

#[derive(Clone, serde::Serialize)]
pub struct AppEntry {
    #[serde(flatten)]
    app: AppInfo,
    /// `data:image/png;base64,…` of the app's icon (Windows only).
    icon: Option<String>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppsView {
    apps: Vec<AppEntry>,
    total: u64,
}

#[tauri::command]
pub async fn apps_scan(app: AppHandle, job_id: String) -> Result<AppsView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "apps", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        let apps = uninstall::scan_apps(&engine, true, &progress, &token)
            .map_err(|error| error.to_string())?;
        let total = apps.iter().map(|app| app.size.as_u64()).sum();
        // Icon extraction touches every binary: run it in this blocking task,
        // once per install path.
        let mut icons: std::collections::HashMap<String, Option<String>> = Default::default();
        let entries = apps
            .iter()
            .map(|entry| {
                let icon = icons
                    .entry(entry.key())
                    .or_insert_with(|| {
                        squid_platform::permissions::app_icon(entry).map(|png| {
                            use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
                            format!("data:image/png;base64,{}", B64.encode(&png))
                        })
                    })
                    .clone();
                AppEntry {
                    app: entry.clone(),
                    icon,
                }
            })
            .collect::<Vec<_>>();
        Ok::<_, String>((apps, entries, total))
    })
    .await
    .map_err(|error| format!("Application scan failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let (apps, entries, total) = task?;
    if let Ok(mut cache) = app.state::<AppState>().apps.lock() {
        *cache = apps;
    }
    if let Ok(mut icons) = app.state::<AppState>().app_icons.lock() {
        icons.clear();
    }
    Ok(AppsView {
        apps: entries,
        total,
    })
}

/// Resolve one app's icon lazily so scanning does not pay for artwork the user
/// never opens. The result is cached; a missing icon is cached as `None` too.
#[tauri::command]
pub async fn app_icon(app: AppHandle, app_key: String) -> Result<Option<String>, String> {
    if let Some(cached) = AppState::lock(&app.state::<AppState>().app_icons)?
        .get(&app_key)
        .cloned()
    {
        return Ok(cached);
    }
    let chosen = AppState::lock(&app.state::<AppState>().apps)?
        .iter()
        .find(|candidate| candidate.key() == app_key)
        .cloned();
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    let png =
        tauri::async_runtime::spawn_blocking(move || squid_platform::apps::icon_png(&chosen, 64))
            .await
            .map_err(|error| format!("Could not read the application icon: {error}"))?;
    let icon = png.map(|bytes| format!("data:image/png;base64,{}", STANDARD.encode(bytes)));
    AppState::lock(&app.state::<AppState>().app_icons)?.insert(app_key, icon.clone());
    Ok(icon)
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    plan_id: String,
    app: AppInfo,
    items: Vec<CleanupItemView>,
    warnings: Vec<String>,
    total: u64,
    selected_count: usize,
    selected_total: u64,
}

fn plan_view(plan_id: String, plan: &UninstallPlan) -> PlanView {
    PlanView {
        plan_id,
        app: plan.app.clone(),
        items: plan.items.iter().map(CleanupItemView::from).collect(),
        warnings: plan.warnings.clone(),
        total: plan.total().as_u64(),
        selected_count: plan
            .items
            .iter()
            .filter(|item| item.selected && item.lock.is_none())
            .count(),
        selected_total: plan
            .items
            .iter()
            .filter(|item| item.selected && item.lock.is_none())
            .map(|item| item.on_disk().as_u64())
            .sum(),
    }
}

#[tauri::command]
pub async fn app_plan(app: AppHandle, job_id: String, app_key: String) -> Result<PlanView, String> {
    let chosen = app
        .state::<AppState>()
        .apps
        .lock()
        .map_err(|_| "The application inventory is unavailable.".to_string())?
        .iter()
        .find(|candidate| candidate.key() == app_key)
        .cloned()
        .ok_or_else(|| "That application is no longer in the inventory.".to_string())?;
    let token = app.state::<AppState>().begin(&job_id)?;
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "plan", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        uninstall::build_plan(&engine, &chosen, &progress, &token)
    })
    .await
    .map_err(|error| format!("Removal plan failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let plan = task.map_err(|error| error.to_string())?;
    let view = plan_view(job_id.clone(), &plan);
    app.state::<AppState>()
        .plans
        .lock()
        .map_err(|_| "The removal plan cache is unavailable.".to_string())?
        .insert(job_id, plan);
    Ok(view)
}

#[tauri::command]
pub async fn app_uninstall(
    app: AppHandle,
    job_id: String,
    plan_id: String,
    selected: Vec<usize>,
    mode: DeleteMode,
) -> Result<DeleteResultView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let mut plan = app
        .state::<AppState>()
        .plans
        .lock()
        .map_err(|_| "The removal plan cache is unavailable.".to_string())?
        .get(&plan_id)
        .cloned()
        .ok_or_else(|| "This removal plan has expired. Build it again.".to_string())?;
    let chosen: HashSet<usize> = selected.into_iter().collect();
    if let Some(index) = chosen.iter().find(|index| **index >= plan.items.len()) {
        return Err(format!("Selected item {index} is not in this plan."));
    }
    for (index, item) in plan.items.iter_mut().enumerate() {
        item.selected = chosen.contains(&index) && item.lock.is_none();
    }
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "delete", progress.clone());
    let task = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        uninstall::remove_plan(&engine, &plan, mode, &progress, &token)
    })
    .await
    .map_err(|error| format!("Uninstall failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let outcome = task.map_err(|error| error.to_string())?;
    if let Ok(mut plans) = app.state::<AppState>().plans.lock() {
        plans.remove(&plan_id);
    }
    app.state::<AppState>().invalidate_storage_cache();
    Ok(DeleteResultView::from_outcome(outcome))
}

/// 卸载增强: drop the 安装不完整 registry record after its files are gone.
/// HKLM removal escalates through one UAC prompt; a declined prompt is an
/// error the page surfaces, never a silent no-op.
#[tauri::command]
pub async fn app_remove_record(_app: AppHandle, reg_key: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            squid_platform::windows::remove_uninstall_record(&reg_key)
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = reg_key;
        Err("此系统没有注册表记录".to_string())
    }
}

/// What `storage_explore` answers: the tree plus how fresh it is (A⑥).
/// `cached` = served from the in-memory size cache; `measuredAtMs` = age of
/// the measurement so the page can say "12 分钟前" honestly. `Node` itself
/// serializes camelCase, same as before for the frontend contract.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreView {
    node: Node,
    cached: bool,
    measured_at_ms: Option<u64>,
}

const STORAGE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

#[tauri::command]
pub async fn storage_explore(
    app: AppHandle,
    job_id: String,
    path: Option<String>,
    depth: Option<usize>,
    top: Option<usize>,
    force: Option<bool>,
) -> Result<ExploreView, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let progress = Progress::new();
    let reporter =
        ProgressReporter::start(app.clone(), job_id.clone(), "storage", progress.clone());
    let root = path
        .filter(|path| !path.trim().is_empty())
        .map(|path| paths::expand(&path))
        .unwrap_or_else(paths::home_dir);
    if !root.is_dir() {
        app.state::<AppState>().finish(&job_id);
        reporter.finish();
        return Err(format!("{} is not a folder.", root.display()));
    }
    let depth = depth.unwrap_or(1).clamp(1, 3);
    let top = top.unwrap_or(16);
    let state = app.state::<AppState>();
    let key = format!("{}|{depth}|{top}", root.to_string_lossy().to_lowercase());
    let epoch = state.epoch();
    if !force.unwrap_or(false) {
        if let Ok(cache) = state.storage_cache.lock() {
            if let Some((cached_epoch, at, node)) = cache.get(&key) {
                let age = std::time::SystemTime::now()
                    .duration_since(*at)
                    .unwrap_or_default();
                let fresh = epoch == *cached_epoch && age < STORAGE_TTL;
                if fresh {
                    app.state::<AppState>().finish(&job_id);
                    reporter.finish();
                    return Ok(ExploreView {
                        node: node.clone(),
                        cached: true,
                        measured_at_ms: Some(u64::try_from(age.as_millis()).unwrap_or(u64::MAX)),
                    });
                }
            }
        }
    }
    let task = tauri::async_runtime::spawn_blocking(move || {
        analyze::explore(&root, depth, top, &token, &progress)
    })
    .await
    .map_err(|error| format!("Storage analysis failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    let node = task.map_err(|error| error.to_string())?;
    if let Ok(mut cache) = state.storage_cache.lock() {
        // Tiny memory cap: the epoch clears everything after a delete run.
        if cache.len() > 48 {
            cache.clear();
        }
        cache.insert(key, (epoch, std::time::SystemTime::now(), node.clone()));
    }
    Ok(ExploreView {
        node,
        cached: false,
        measured_at_ms: Some(0),
    })
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsightView {
    id: String,
    title: String,
    path: String,
    size: u64,
    action: String,
    command: Option<String>,
    hint: String,
}

/// Hidden-space insights for the storage page's curated section (A⑤).
#[tauri::command]
pub async fn storage_insights() -> Result<Vec<InsightView>, String> {
    let rows = tauri::async_runtime::spawn_blocking(squid_engine::insights::collect)
        .await
        .map_err(|error| format!("Insight scan failed: {error}"))?;
    Ok(rows
        .into_iter()
        .map(|row| InsightView {
            id: row.id,
            title: row.title,
            path: row.path,
            size: row.size,
            action: row.action,
            command: row.command,
            hint: row.hint,
        })
        .collect())
}

/// Mole-style last-second re-check surfaced before deletion (B1).
#[tauri::command]
pub async fn cleanup_preflight(
    app: AppHandle,
    scan_id: String,
    selected: Vec<usize>,
) -> Result<serde_json::Value, String> {
    let report = app
        .state::<AppState>()
        .scans
        .lock()
        .map_err(|_| "The scan cache is unavailable.".to_string())?
        .get(&scan_id)
        .cloned()
        .ok_or_else(|| "This scan has expired. Run it again.".to_string())?;
    let view = tauri::async_runtime::spawn_blocking(move || {
        let engine = Engine::new();
        let preflight = engine.preflight(&report, &selected);
        serde_json::json!({
            "checked": preflight.checked,
            "blocked": preflight
                .blocked
                .into_iter()
                .map(|(path, reason)| serde_json::json!({ "path": path, "reason": reason }))
                .collect::<Vec<_>>(),
        })
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(view)
}

#[tauri::command]
pub async fn storage_largest_files(
    app: AppHandle,
    job_id: String,
    path: String,
    limit: Option<usize>,
) -> Result<Vec<FileEntry>, String> {
    let token = app.state::<AppState>().begin(&job_id)?;
    let progress = Progress::new();
    let reporter = ProgressReporter::start(app.clone(), job_id.clone(), "files", progress.clone());
    let root = paths::expand(&path);
    let task = tauri::async_runtime::spawn_blocking(move || {
        analyze::largest_files(
            &root,
            limit.unwrap_or(40).clamp(1, 200),
            Bytes(1_000_000),
            &token,
            &progress,
        )
    })
    .await
    .map_err(|error| format!("Large-file scan failed: {error}"))?;
    reporter.finish();
    app.state::<AppState>().finish(&job_id);
    task.map_err(|error| error.to_string())
}

#[tauri::command]
pub fn system_snapshot(app: AppHandle, full: Option<bool>) -> Result<SystemSnapshot, String> {
    // `full` costs a per-pid CPU/memory walk of every process on the machine
    // (hundreds of syscalls) plus the top-12 sort — the 实时监控 page needs
    // it; the desktop widget, overview tiles and every background poll pass
    // false and ride the light refresh. Light answers are cached for two
    // seconds so windows polling in step pay for one refresh, not several.
    let state = app.state::<AppState>();
    let full = full.unwrap_or(false);
    // Check and fill the cache under the sampler lock: two windows polling a
    // beat apart then share one refresh only if the second can not slip in
    // between the first cache miss and the sampler call.
    let mut sampler = AppState::lock(&state.sampler)?;
    if !full {
        if let Ok(cache) = AppState::lock(&state.snapshot_cache) {
            if let Some((at, snapshot)) = cache.as_ref() {
                if at.elapsed() < Duration::from_secs(2) {
                    return Ok(snapshot.clone());
                }
            }
        }
    }
    let snapshot = if full {
        sampler.refresh()
    } else {
        sampler.refresh_with(false)
    };
    if let Ok(mut cache) = AppState::lock(&state.snapshot_cache) {
        *cache = Some((Instant::now(), snapshot.clone()));
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn stop_process(app: AppHandle, process: ProcessSample) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut sampler = AppState::lock(&state.sampler)?;
    sampler.stop(&process).map_err(|error| error.to_string())
}

/// Write a UTF-8 text file (B5 clean-list export; the UI picked the path
/// through the save dialog, so this is a plain, bounded write).
#[tauri::command]
pub async fn write_text_file(path: String, contents: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = paths::expand(&path);
        std::fs::write(&target, contents.as_bytes())
            .map_err(|error| format!("{}: {error}", target.display()))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// B2: which protection patterns actually hit something on this machine.
#[tauri::command]
pub async fn whitelist_audit() -> Result<serde_json::Value, String> {
    let rows: Vec<serde_json::Value> = tauri::async_runtime::spawn_blocking(|| {
        Engine::new()
            .audit_whitelist()
            .into_iter()
            .map(|(pattern, live)| serde_json::json!({ "pattern": pattern, "live": live }))
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(serde_json::Value::Array(rows))
}

#[tauri::command]
pub fn history_list(limit: Option<usize>) -> Vec<Session> {
    ledger::load_sessions(limit.unwrap_or(60).clamp(1, 500))
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResultView {
    restored: usize,
    skipped: usize,
}

#[tauri::command]
pub async fn history_undo(session_id: String) -> Result<UndoResultView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (restored, skipped) =
            delete::restore_session(&session_id).map_err(|e| e.to_string())?;
        Ok(UndoResultView { restored, skipped })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    delete_mode: Option<DeleteMode>,
    language: Option<String>,
    show_allocated: Option<bool>,
    purge_min_age_days: Option<u64>,
    purge_roots: Option<Vec<String>>,
    history_keep: Option<usize>,
    setup_reviewed: Option<bool>,
}

#[tauri::command]
pub async fn settings_save(app: AppHandle, patch: SettingsPatch) -> Result<SettingsView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (mut settings, mut errors) = Settings::load();
        if let Some(mode) = patch.delete_mode {
            settings.delete_mode = mode;
        }
        if let Some(language) = patch.language {
            let trimmed = language.trim();
            settings.language = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        if let Some(show) = patch.show_allocated {
            settings.show_allocated = show;
        }
        if let Some(days) = patch.purge_min_age_days {
            settings.purge_min_age_days = days.min(3650);
        }
        if let Some(roots) = patch.purge_roots {
            // Empty list = "back to the built-in defaults", mirroring Mole's
            // leave-the-file-blank-to-reset habit.
            settings.purge_roots = if roots.is_empty() {
                Settings::default().purge_roots
            } else {
                roots
            };
        }
        if let Some(keep) = patch.history_keep {
            settings.history_keep = keep.clamp(5, 500);
        }
        if let Some(reviewed) = patch.setup_reviewed {
            settings.desktop_setup_reviewed = reviewed;
        }
        settings
            .save()
            .map_err(|error| format!("Could not save settings: {error}"))?;
        crate::sync_tray(&app, Locale::detect(&settings));
        errors.retain(|error| !error.contains("config.toml"));
        Ok(SettingsView::from_settings(&settings, errors))
    })
    .await
    .map_err(|error| error.to_string())?
}

fn escape_glob(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '*' | '?' | '[' | ']' | '{' | '}' | '!') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[tauri::command]
pub async fn whitelist_add(path: String) -> Result<SettingsView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (mut settings, errors) = Settings::load();
        let expanded = paths::expand(&path);
        if expanded.as_os_str().is_empty() {
            return Err("Choose a path to protect.".into());
        }
        settings.add_whitelist(escape_glob(&paths::pretty(&expanded)));
        settings
            .save()
            .map_err(|error| format!("Could not save settings: {error}"))?;
        Ok(SettingsView::from_settings(&settings, errors))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn whitelist_remove(pattern: String) -> Result<SettingsView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (mut settings, errors) = Settings::load();
        settings.remove_whitelist(&pattern);
        settings
            .save()
            .map_err(|error| format!("Could not save settings: {error}"))?;
        Ok(SettingsView::from_settings(&settings, errors))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    let target = PathBuf::from(path);
    if !target.exists() {
        return Err(format!("{} no longer exists.", target.display()));
    }
    #[cfg(target_os = "macos")]
    {
        crate::process::command("/usr/bin/open")
            .arg("-R")
            .arg(&target)
            .status()
            .map_err(|error| error.to_string())?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer only understands `/select,"C:\path with space\x"` — the
        // quotes must sit *inside* the argument, around the path. Handing
        // the whole `/select,<path>` as one argument let Rust wrap it in
        // one pair of quotes, Explorer failed to parse it and opened its
        // default folder instead (定位 jumped to the wrong directory as
        // soon as the path contained a space, e.g. Package Cache).
        // `raw_arg` writes the argument verbatim, producing the exact
        // command line Explorer documents.
        let mut cmd = crate::process::command("explorer.exe");
        cmd.raw_arg(format!("/select,\"{}\"", target.display()));
        cmd.status().map_err(|error| error.to_string())?;
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        crate::process::command("xdg-open")
            .arg(target.parent().unwrap_or(Path::new("/")))
            .status()
            .map_err(|error| error.to_string())?;
    }
    let _ = app;
    Ok(())
}

#[tauri::command]
pub fn open_trash() -> Result<(), String> {
    let platform = squid_platform::current();
    platform.reveal_trash();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selected_index_can_never_select_a_locked_item() {
        let mut report = ScanReport::default();
        report.items.push(CleanupItem::new(
            PathBuf::from("/tmp/keep"),
            ItemKind::File,
            Category::Logs,
            "keep",
        ));
        report.items.push(
            CleanupItem::new(
                PathBuf::from("/tmp/locked"),
                ItemKind::File,
                Category::Logs,
                "locked",
            )
            .locked(LockReason::Protected),
        );
        select_indices(&mut report, &[0, 1]).unwrap();
        assert!(report.items[0].selected);
        assert!(!report.items[1].selected);
    }

    #[test]
    fn an_out_of_range_selection_is_refused() {
        let mut report = ScanReport::default();
        assert!(select_indices(&mut report, &[0]).is_err());
    }

    #[test]
    fn whitelist_paths_are_taken_literally() {
        assert_eq!(escape_glob("~/Caches/[demo]*"), "~/Caches/\\[demo\\]\\*");
    }
}
