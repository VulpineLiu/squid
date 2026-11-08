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
use squid_platform::startup::{StartupCategory, StartupInventory, StartupItem, StartupNote};
use squid_platform::trust;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupView {
    items: Vec<StartupItem>,
    notes: Vec<StartupNote>,
    /// The two lists the page shows, counted the way they are counted on screen.
    login_count: usize,
    background_count: usize,
    /// Entries belonging to Apple or Microsoft, which are hidden by default.
    vendor_count: usize,
    /// Entries the user owns that are not vendor-owned: what this page may act on.
    actionable_count: usize,
    risky_count: usize,
}

impl StartupView {
    fn of(inventory: StartupInventory) -> StartupView {
        StartupView {
            login_count: inventory
                .items
                .iter()
                .filter(|item| item.category == StartupCategory::OpenAtLogin)
                .count(),
            background_count: inventory
                .items
                .iter()
                .filter(|item| item.category == StartupCategory::Background)
                .count(),
            vendor_count: inventory
                .items
                .iter()
                .filter(|item| item.system_vendor)
                .count(),
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

/// Open the pane where the entries squid only reads can actually be changed.
///
/// macOS owns the login-item list, and Windows owns scheduled tasks and services, so
/// for those the honest action is to hand the person to the operating system rather
/// than offer a toggle that would not hold. `open` reports success for a settings
/// pane it does not recognise, so the macOS version picks the URL instead of trying
/// one and hoping: Ventura renamed the login-item pane from a Users & Groups anchor
/// to its own extension.
#[tauri::command]
pub fn open_startup_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let target = {
        let version = crate::process::command("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default();
        let major: u32 = version
            .split('.')
            .next()
            .and_then(|head| head.parse().ok())
            .unwrap_or(12);
        if major >= 13 {
            "x-apple.systempreferences:com.apple.LoginItems-Settings.extension"
        } else {
            "x-apple.systempreferences:com.apple.preference.users?LoginItems"
        }
    };
    // The pane Task Manager's Startup tab writes to. `ms-settings:startup` is not a
    // URI and silently opens whatever was last shown.
    #[cfg(windows)]
    let target = "ms-settings:startupapps";

    #[cfg(target_os = "macos")]
    let opened = run("/usr/bin/open", &[target]);
    #[cfg(windows)]
    let opened = run("cmd", &["/C", "start", "", target]);
    #[cfg(not(any(target_os = "macos", windows)))]
    return Err("The system settings are not available here.".into());
    #[cfg(any(target_os = "macos", windows))]
    opened
}

/// The one shape the two platforms' settings openers differ in: a program, its
/// arguments, and whether the operating system accepted the request.
#[cfg(any(target_os = "macos", windows))]
fn run(program: &str, arguments: &[&str]) -> Result<(), String> {
    let status = crate::process::command(program)
        .args(arguments)
        .status()
        .map_err(|error| format!("Could not open the system settings: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Could not open the system settings.".into())
    }
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
        let mut rows = squid_platform::process_table::snapshot(with_trust).rows;
        mark_startup_entries(&mut rows, &squid_platform::startup::inventory());
        Ok(rows)
    })
    .await
    .map_err(|error| format!("Could not read the process table: {error}"))?
}

/// Name the startup entry behind each process, which is what the table's "started by
/// a startup item" marker and its filter read.
///
/// `ProcessRow::startup_item` says the desktop layer fills it in, and the marker has
/// been drawn from it since 0.1.6 — but nothing ever assigned it, so the icon never
/// appeared and "only started by an entry" returned nothing at all.
///
/// Measured on an M1 Pro, ~760 processes and 874 entries: costs 0.45 s on top of the
/// 5.7 s the table already spends resolving signatures, and marks 452 of the rows.
/// The same read dominates the cheap path that skips signatures — 26 ms becomes 390 ms.
/// See ROADMAP 13.9 for why that trade was taken without caching.
fn mark_startup_entries(rows: &mut [ProcessRow], inventory: &StartupInventory) {
    // Keyed once for the whole table. `entry_for` re-reads every root on every call,
    // so using it per row would cost one inventory per process, several hundred of them,
    // where the table already spends seconds reading signatures.
    let mut declared: HashMap<&Path, &str> = HashMap::new();
    for item in &inventory.items {
        // The inventory arrives user entries first, so keeping the first name seen
        // prefers the entry this user owns when a vendor installs the same program
        // twice (`com.google.keystone.agent` exists in both `~/Library` and `/Library`).
        if let Some(program) = item.program.as_deref() {
            declared.entry(program).or_insert(item.name.as_str());
        }
    }
    for row in rows {
        row.startup_item = row
            .program
            .as_deref()
            .and_then(|program| declared.get(program))
            .map(|name| (*name).to_string());
    }
}

#[cfg(test)]
mod join_tests {
    use super::mark_startup_entries;
    use squid_platform::startup::{StartupInventory, StartupItem, StartupScope, StartupSource};
    use std::path::Path;

    /// The join costs one inventory read per refresh. Measured here rather than
    /// asserted, because it is a latency decision, not a correctness one: the process
    /// table has a deliberately cheap path that skips signature reads, and a join that
    /// dwarfed it would be the wrong trade.
    #[test]
    #[ignore]
    fn what_the_join_costs() {
        use std::time::Instant;
        for with_trust in [true, false] {
            let start = Instant::now();
            let mut rows = squid_platform::process_table::snapshot(with_trust).rows;
            let table = start.elapsed();
            let start = Instant::now();
            mark_startup_entries(&mut rows, &squid_platform::startup::inventory());
            let join = start.elapsed();
            let marked = rows.iter().filter(|row| row.startup_item.is_some()).count();
            println!(
                "trust={with_trust}: table {table:.0?} + join {join:.0?} = {:.0?} · {marked}/{} marked",
                table + join,
                rows.len()
            );
        }
    }

    fn row(pid: u32, program: Option<&str>) -> super::ProcessRow {
        super::ProcessRow {
            pid,
            parent: None,
            name: "thing".into(),
            command: String::new(),
            program: program.map(Path::new).map(|p| p.to_path_buf()),
            working_directory: None,
            user: String::new(),
            cpu: 0.0,
            memory: 0,
            started_ms: None,
            startup_item: None,
            signed: false,
            publisher: None,
            owned: true,
        }
    }

    fn entry(name: &str, program: &str) -> StartupItem {
        let mut item = StartupItem::new(
            format!("/Library/LaunchAgents/{name}.plist"),
            name,
            StartupSource::LaunchAgent,
            StartupScope::User,
        );
        item.program = Some(Path::new(program).to_path_buf());
        item
    }

    /// The table has drawn a marker from this field since 0.1.6, and a filter has
    /// keyed off it, but nothing ever wrote it: the icon never appeared and
    /// "only started by an entry" returned nothing at all.
    #[test]
    fn a_process_is_matched_to_the_entry_that_declares_it() {
        let inventory = StartupInventory {
            items: vec![
                entry("com.user.keystone", "/opt/keystone/agent"),
                entry("com.system.keystone", "/opt/keystone/agent"),
                entry("com.other.thing", "/opt/other/thing"),
            ],
            notes: vec![],
        };
        let mut rows = vec![
            row(1, Some("/opt/keystone/agent")),
            row(2, Some("/opt/unrelated/app")),
            row(3, None),
        ];
        mark_startup_entries(&mut rows, &inventory);

        assert_eq!(
            rows[0].startup_item.as_deref(),
            Some("com.user.keystone"),
            "an entry is named, and the user-owned one wins a duplicate program"
        );
        assert_eq!(
            rows[1].startup_item, None,
            "a process nothing declared stays plain"
        );
        assert_eq!(
            rows[2].startup_item, None,
            "a process with no path cannot be matched"
        );
    }

    /// Disabling an entry does not necessarily stop the process it started, and that
    /// is exactly when the link is worth seeing rather than losing. The detail drawer's
    /// `entry_for` has never filtered on `disabled` either, so the two views agree.
    #[test]
    fn a_disabled_entry_still_names_the_process_it_left_running() {
        let mut off = entry("com.user.updater", "/opt/updater/agent");
        off.disabled = true;
        let inventory = StartupInventory {
            items: vec![off],
            notes: vec![],
        };
        let mut rows = vec![row(9, Some("/opt/updater/agent"))];
        mark_startup_entries(&mut rows, &inventory);
        assert_eq!(
            rows[0].startup_item.as_deref(),
            Some("com.user.updater"),
            "the row that explains why this is still running must not disappear"
        );
    }
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

#[cfg(test)]
mod wire_tests {
    use super::StartupView;
    use squid_platform::startup::{
        StartupInventory, StartupItem, StartupOwner, StartupScope, StartupSource,
    };

    /// Every name the page reads has to be a name on the wire.
    ///
    /// `StartupView` renames itself to camelCase and `StartupItem` did not, so the
    /// page's `item.systemVendor` was `undefined` for every entry: the filter that
    /// hides 845 Apple items silently hid nothing, and TypeScript could not see it
    /// because the interface says what was intended, not what is sent.
    #[test]
    fn startup_entries_arrive_in_the_names_the_page_reads() {
        let mut item = StartupItem::new(
            "/Library/LaunchDaemons/com.docker.vmnetd.plist",
            "com.docker.vmnetd",
            StartupSource::LaunchDaemon,
            StartupScope::System,
        );
        item.owner = Some(StartupOwner {
            name: "Docker".into(),
            bundle_path: Some("/Applications/Docker.app".into()),
            vendor: false,
        });
        let json = serde_json::to_value(StartupView::of(StartupInventory {
            items: vec![item],
            notes: vec![],
        }))
        .expect("a view serializes");

        for key in ["systemVendor", "workingDirectory", "modifiedMs"] {
            assert!(
                json["items"][0].get(key).is_some(),
                "the page reads StartupItem.{key}; the wire does not carry it"
            );
        }
        assert!(
            json["items"][0]["owner"].get("bundlePath").is_some(),
            "the page reads owner.bundlePath"
        );
        assert!(
            json["items"][0].get("system_vendor").is_none(),
            "one casing per boundary, not two spellings of the same field"
        );
    }

    /// The process table renames its own rows the same way, and reads three fields
    /// that only exist if it does.
    #[test]
    fn process_rows_arrive_in_the_names_the_table_reads() {
        let row = squid_platform::process_table::ProcessRow {
            pid: 4_321,
            parent: None,
            name: "squid".into(),
            command: String::new(),
            program: None,
            working_directory: None,
            user: String::new(),
            cpu: 0.0,
            memory: 0,
            started_ms: None,
            startup_item: None,
            signed: false,
            publisher: None,
            owned: true,
        };
        let json = serde_json::to_value(&row).expect("a row serializes");
        for key in ["startedMs", "startupItem", "workingDirectory"] {
            assert!(
                json.get(key).is_some(),
                "the table reads ProcessRow.{key}; the wire does not carry it"
            );
        }
        let trust = serde_json::to_value(squid_platform::trust::Trust::default())
            .expect("a trust serializes");
        assert!(
            trust.get("adHoc").is_some(),
            "the table reads Trust.adHoc; the wire says {:?}",
            trust.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );
    }
}
