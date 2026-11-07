//! The full process table, as opposed to the twelve busiest rows the meters
//! panel draws.
//!
//! `squid-stats` samples every couple of seconds and must stay cheap, so it
//! reports names and percentages. Answering "what is this thing, who started it
//! and did anybody sign it" is a different question, asked when a person opens
//! the Processes tab, and it tolerates a slower path: one refresh of the table
//! plus a signature lookup for the executables actually on screen.
//!
//! The table is deliberately read-only here. Deciding whether a pid may be
//! stopped is the job of [`crate::process`], which refuses squid itself and the
//! processes a boot rests on; this module only describes what is running.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, Users};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessRow {
    pub pid: u32,
    pub parent: Option<u32>,
    pub name: String,
    pub command: String,
    pub program: Option<PathBuf>,
    pub working_directory: Option<PathBuf>,
    pub user: String,
    pub cpu: f32,
    pub memory: u64,
    pub started_ms: Option<u64>,
    /// Set later by the desktop layer, which owns the startup inventory: a
    /// process started by a login item is a different kind of thing than one
    /// the person launched themselves.
    pub startup_item: Option<String>,
    pub signed: bool,
    pub publisher: Option<String>,
    /// False for processes owned by somebody else, which squid must not stop.
    pub owned: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessTable {
    pub rows: Vec<ProcessRow>,
    pub taken_at_ms: u64,
}

/// Snapshot the table. `with_trust` asks the operating system about each
/// distinct executable once, which is the expensive part and is only wanted
/// when the signature column is actually on screen.
pub fn snapshot(with_trust: bool) -> ProcessTable {
    let mut system =
        System::new_with_specifics(RefreshKind::nothing().with_processes(process_kind()));
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, process_kind());
    snapshot_from(&system, with_trust)
}

fn process_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cmd(sysinfo::UpdateKind::Always)
        .with_exe(sysinfo::UpdateKind::Always)
        .with_cwd(sysinfo::UpdateKind::Always)
        .with_memory()
        .with_cpu()
        .with_user(sysinfo::UpdateKind::Always)
}

fn snapshot_from(system: &System, with_trust: bool) -> ProcessTable {
    let users = Users::new_with_refreshed_list();
    let current_user = current_user_name();
    let mut cache: std::collections::HashMap<PathBuf, crate::trust::Trust> =
        std::collections::HashMap::new();
    let mut rows: Vec<ProcessRow> = system
        .processes()
        .values()
        .map(|process| {
            let program = process.exe().map(|path| path.to_path_buf());
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            let user = process
                .user_id()
                .and_then(|id| users.get_user_by_id(id))
                .map(|user| user.name().to_string())
                .unwrap_or_default();
            let mut row = ProcessRow {
                pid: process.pid().as_u32(),
                parent: process.parent().map(|pid| pid.as_u32()),
                name: name_of(process.name()),
                command,
                started_ms: start_time_ms(process.start_time()),
                program,
                working_directory: process.cwd().map(|path| path.to_path_buf()),
                user,
                cpu: process.cpu_usage().min(4_000.0),
                memory: process.memory(),
                startup_item: None,
                signed: false,
                publisher: None,
                owned: true,
            };
            row.owned = row.user.is_empty()
                || current_user.is_empty()
                || row.user.eq_ignore_ascii_case(&current_user);

            if with_trust {
                if let Some(program) = row.program.as_deref() {
                    let trust = cache.entry(program.to_path_buf()).or_insert_with(|| {
                        if crate::trust::might_be_executable(program) {
                            crate::trust::assess(program)
                        } else {
                            Default::default()
                        }
                    });
                    row.signed = trust.signed;
                    row.publisher = trust.signer.clone();
                }
            }
            row
        })
        .collect();

    rows.sort_by(|a, b| b.memory.cmp(&a.memory).then_with(|| a.pid.cmp(&b.pid)));
    ProcessTable {
        rows,
        taken_at_ms: now_ms(),
    }
}

/// One row, re-read right now. The desktop layer asks this before it stops
/// anything, so the name the person clicked is checked against the pid that
/// answers to that number today.
pub fn row_of(pid: u32) -> Option<ProcessRow> {
    let mut system = System::new_with_specifics(RefreshKind::nothing());
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
        process_kind(),
    );
    let process = system.process(sysinfo::Pid::from_u32(pid))?;
    Some(ProcessRow {
        pid,
        parent: process.parent().map(|pid| pid.as_u32()),
        name: name_of(process.name()),
        command: process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" "),
        program: process.exe().map(|path| path.to_path_buf()),
        working_directory: process.cwd().map(|path| path.to_path_buf()),
        user: String::new(),
        cpu: process.cpu_usage(),
        memory: process.memory(),
        started_ms: start_time_ms(process.start_time()),
        startup_item: None,
        signed: false,
        publisher: None,
        owned: true,
    })
}

fn name_of(name: &std::ffi::OsStr) -> String {
    name.to_string_lossy()
        .split(['/', '\\'])
        .next_back()
        .filter(|part| !part.is_empty())
        .unwrap_or("?")
        .to_string()
}

/// sysinfo reports Unix start times as seconds since boot; Windows and newer
/// sysinfo releases report seconds since the epoch. Anything that already looks
/// like an epoch passes through, so a wrong guess cannot turn into a date in
/// 1970 on every row.
fn start_time_ms(raw: u64) -> Option<u64> {
    if raw == 0 {
        return None;
    }
    let uptime = System::uptime();
    let now = now_ms() / 1_000;
    if raw > 1_000_000_000 && raw <= now {
        return Some(raw.saturating_mul(1_000));
    }
    if raw <= uptime {
        return Some(now.saturating_sub(raw).saturating_mul(1_000));
    }
    None
}

/// The account squid runs as. Ownership is compared by name because a process
/// reports a uid/SID and the user table is the only thing that turns either one
/// into the name a person recognises — the same source `HostInfo` prints.
fn current_user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
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

    #[test]
    fn a_snapshot_describes_the_test_process() {
        let table = snapshot(false);
        assert!(
            table.rows.iter().any(|row| row.pid == std::process::id()),
            "the process running this test is missing from the table"
        );
    }

    #[test]
    fn a_row_can_be_re_read_by_pid() {
        let here = std::process::id();
        let row = row_of(here).expect("this process is running");
        assert_eq!(row.pid, here);
        assert!(!row.name.is_empty());
        assert!(row_of(u32::MAX).is_none());
    }

    #[test]
    fn sysinfo_start_times_are_normalised_to_millis() {
        // Seconds since boot, which is what sysinfo gives on unix.
        assert!(start_time_ms(10).is_some());
        // Seconds since the epoch, which is what it gives on Windows.
        let epoch = 1_700_000_000;
        assert_eq!(start_time_ms(epoch), Some(epoch * 1_000));
        assert_eq!(start_time_ms(0), None);
    }
}
