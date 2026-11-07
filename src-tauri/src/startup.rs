//! `startup_*` and `process_*`: what runs by itself, and what is running now.
//!
//! Two lists that look alike and answer different questions. The startup list is
//! a claim about the next login, read from launchd plists, registry Run keys, the
//! startup folders, scheduled tasks and services; the process list is a fact
//! about this second. A row can appear in one and not the other for entirely
//! ordinary reasons, so the two are joined only by a label — "this process came
//! from that entry" — and never merged into one table.
//!
//! Reading is free. Writing is not: only entries the current user owns are
//! offered as modifiable, and the platform layer re-reads reality after a change
//! instead of trusting the caller to remember what it asked for.

use serde::Serialize;
use squid_platform::process_table::ProcessRow;
use squid_platform::startup::{StartupInventory, StartupItem};
use squid_platform::trust;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupView {
    items: Vec<StartupItem>,
    notes: Vec<String>,
    user_count: usize,
    system_count: usize,
    disabled_count: usize,
    /// Entries the user owns that are not vendor-owned: what this page may act on.
    actionable_count: usize,
    risky_count: usize,
    /// Which of the two platforms' surfaces were readable at all, so a page that
    /// shows nothing can say whether that means "none" or "could not look".
    sources: Vec<String>,
}

impl StartupView {
    fn of(inventory: StartupInventory) -> StartupView {
        let mut sources: Vec<String> = inventory
            .items
            .iter()
            .map(|item| item.source.as_str().to_string())
            .collect();
        sources.sort();
        sources.dedup();
        StartupView {
            actionable_count: inventory
                .items
                .iter()
                .filter(|item| item.modifiable && !item.system_vendor)
                .count(),
            risky_count: inventory
                .items
                .iter()
                .filter(|item| !item.risks.is_empty())
                .count(),
            user_count: inventory.user_count(),
            system_count: inventory.system_count(),
            disabled_count: inventory.disabled_count(),
            sources,
            items: inventory.items,
            notes: inventory.notes,
        }
    }
}

#[tauri::command]
pub async fn startup_items() -> Result<StartupView, String> {
    tauri::async_runtime::spawn_blocking(|| StartupView::of(squid_platform::startup::inventory()))
        .await
        .map_err(|error| format!("Could not read the startup items: {error}"))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupChange {
    item: StartupItem,
    message: String,
    view: StartupView,
}

/// Disable or enable one entry. The refreshed inventory comes back with it,
/// because the truth after a launchd reload or a registry write is whatever the
/// system now says, not what this function intended.
#[tauri::command]
pub async fn startup_set_enabled(id: String, enabled: bool) -> Result<StartupChange, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let item = squid_platform::startup::set_enabled(&id, enabled)?;
        let mut store = squid_guard::store::Store::new();
        store.note(
            if enabled { "startup-on" } else { "startup-off" },
            item.name.clone(),
            item.program.clone(),
        );
        let message = if item.disabled == !enabled {
            format!(
                "{} is now {}.",
                item.name,
                if item.disabled { "off" } else { "on" }
            )
        } else {
            // The system disagrees with what was asked. Say so, because the
            // common causes — a re-armed login item, a managed profile — are
            // things the user needs to know rather than to retry.
            format!(
                "{} did not change: the system reports it as {}.",
                item.name,
                if item.disabled { "off" } else { "on" }
            )
        };
        Ok(StartupChange {
            view: StartupView::of(squid_platform::startup::inventory()),
            item,
            message,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDetailView {
    row: ProcessRow,
    sha256: Option<String>,
    trust: trust::Trust,
    startup: Option<StartupItem>,
    parent: Option<ProcessRow>,
    /// Why squid would refuse to stop this one, in the user's terms.
    stop_note: Option<String>,
}

#[tauri::command]
pub async fn process_inventory(with_trust: Option<bool>) -> Result<Vec<ProcessRow>, String> {
    let with_trust = with_trust.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ProcessRow>, String> {
        Ok(squid_platform::process_table::snapshot(with_trust).rows)
    })
    .await
    .map_err(|error| format!("Could not read the process table: {error}"))?
}

#[tauri::command]
pub async fn process_detail(pid: u32) -> Result<ProcessDetailView, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<ProcessDetailView, String> {
        let row = squid_platform::process_table::row_of(pid)
            .ok_or_else(|| format!("Process {pid} has already exited."))?;
        let executable: Option<PathBuf> = row.program.clone();
        let trust = match &executable {
            Some(path) => trust::assess(path),
            None => trust::Trust::default(),
        };
        let sha256 = executable
            .as_ref()
            .and_then(|path| squid_guard::rules::sha256_of(path));
        let startup = row
            .program
            .as_ref()
            .and_then(|program| squid_platform::startup::entry_for(program));
        let parent = row.parent.and_then(squid_platform::process_table::row_of);
        let stop_note = refusal_note(&row);
        Ok(ProcessDetailView {
            trust,
            sha256,
            startup,
            parent,
            stop_note,
            row,
        })
    })
    .await
    .map_err(|error| format!("Could not read that process: {error}"))?
}

/// Why squid will not stop a process, or `None` when it will. Mirrors the checks
/// in `squid-stats`' sampler so the UI can explain before the user clicks, and
/// not promise something the stop path will refuse.
fn refusal_note(row: &ProcessRow) -> Option<String> {
    if let Some(rule) = squid_platform::process::pid_refusal(row.pid) {
        return Some(format!("Squid never stops {rule}.", rule = describe(rule)));
    }
    if let Some(rule) = squid_platform::process::refusal(row.pid, &row.name) {
        return Some(format!("Squid never stops {rule}.", rule = describe(rule)));
    }
    if !row.owned {
        return Some("This process belongs to another user or to the system.".into());
    }
    None
}

/// The rule's own wording, so a refusal shown before a click and the refusal the
/// stop path returns cannot drift into two different sentences.
fn describe(rule: squid_platform::process::OffLimits) -> &'static str {
    rule.reason()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStopResult {
    pub message: String,
}

/// Stop one process by pid, re-checked against the live table. The name that
/// comes back with the confirmation is the point: a pid that has been recycled
/// between the click and the kill belongs to something else, and the row read
/// here is the row whose name is compared.
#[tauri::command]
pub async fn process_stop(pid: u32, name: String) -> Result<ProcessStopResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let row = squid_platform::process_table::row_of(pid)
            .ok_or_else(|| format!("Process {pid} has already exited."))?;
        if row.name != name {
            return Err(format!(
                "Process {pid} is now {}, not {name}. Nothing was stopped.",
                row.name
            ));
        }
        if let Some(refusal) = refusal_note(&row) {
            return Err(refusal);
        }
        squid_platform::process::force_stop(pid)
            .map_err(|error| format!("Could not stop {name}: {error}"))?;
        Ok(ProcessStopResult {
            message: format!("Stopped {name} ({pid})."),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
