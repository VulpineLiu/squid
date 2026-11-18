//! The only code path in squid that destroys data.
//!
//! Every engine (clean / uninstall / purge) and the TUI funnel deletions here.
//! The rules it enforces are deliberately paranoid:
//!
//! * locked items are refused, always — a lock means "the scanner said no";
//! * every item is re-validated against [`squid_core::SafetyPolicy`] and the
//!   whitelist at delete time, because a scan can be minutes old and the disk
//!   may have moved underneath us;
//! * nested children are dropped before their parent is removed so the freed
//!   total is not double counted and children do not fail with ENOENT;
//! * Trash mode records the destination so `squid undo` can move things back.

use crate::{drop_nested_duplicates, Engine};
use squid_core::fs;
use squid_core::ledger::{self, LedgerEntry, Session};
use squid_core::progress::{CancelToken, Event, Progress};
use squid_core::{Bytes, CleanSummary, CleanupItem, DeleteMode, LockReason};
use std::path::PathBuf;
use std::time::Instant;

/// A batch of approved items plus the provenance needed for the ledger.
#[derive(Clone, Debug)]
pub struct DeleteJob {
    pub items: Vec<CleanupItem>,
    pub mode: DeleteMode,
    /// `squid clean`, `squid purge`, … — written to the history file.
    pub command: String,
}

impl DeleteJob {
    pub fn new(items: Vec<CleanupItem>, mode: DeleteMode, command: impl Into<String>) -> Self {
        DeleteJob {
            items,
            mode,
            command: command.into(),
        }
    }

    /// Items the user selected *and* the scanner allowed.
    pub fn removable(&self) -> Vec<CleanupItem> {
        self.items
            .iter()
            .filter(|item| item.selected && item.lock.is_none())
            .cloned()
            .collect()
    }
}

/// Result of a delete run, with the refusal reasons split out for the UI.
#[derive(Clone, Debug, Default)]
pub struct DeleteOutcome {
    pub summary: CleanSummary,
    /// Items that were never attempted, with the reason shown to the user.
    pub skipped: Vec<(PathBuf, String)>,
}

/// Delete every removable item of the job.
pub fn execute(
    engine: &Engine,
    job: &DeleteJob,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<DeleteOutcome> {
    let mut items = job.removable();
    if items.is_empty() {
        return Err(squid_core::SquidError::NothingToDo);
    }
    drop_nested_duplicates(&mut items);

    progress.phase("delete", "removing approved items");
    progress.tick(0, Some(items.len() as u64), None);

    let session_id = ledger::new_session_id();
    let started = Instant::now();
    let mut summary = CleanSummary {
        mode: Some(job.mode),
        session_id: Some(session_id.clone()),
        ..CleanSummary::default()
    };
    let mut entries: Vec<LedgerEntry> = Vec::with_capacity(items.len());
    let mut skipped: Vec<(PathBuf, String)> = Vec::new();

    for (index, item) in items.iter().enumerate() {
        if cancel.is_cancelled() {
            progress.warning("cancelled — the items already removed stay removed");
            break;
        }
        progress.tick(
            index as u64 + 1,
            Some(items.len() as u64),
            Some(item.path.clone()),
        );

        if let Some(lock) = item.lock {
            skipped.push((item.path.clone(), lock_reason(lock).to_string()));
            summary.record_error(format!("{}: {}", item.display_path(), lock_reason(lock)));
            continue;
        }
        if let Some(reason) = engine.block_reason(item) {
            skipped.push((item.path.clone(), reason.clone()));
            progress.warning(format!("{} — {}", item.display_path(), reason));
            continue;
        }

        // Resolve the Trash destination *before* moving so the ledger can
        // reverse the move later. `trash_target_for` only invents a name that
        // does not exist yet, so this matches what `remove_item_with` does.
        // On Windows the shell owns the Recycle Bin bookkeeping and returns
        // `None`; restore then falls back to the Recycle Bin UI.
        let trash_destination = match job.mode {
            DeleteMode::Trash => fs::trash_target_for(&item.path),
            DeleteMode::Permanent => None,
        };

        match fs::remove_item_with(item, job.mode, engine.safety()) {
            Ok(freed) => {
                summary.freed += item.size;
                summary.freed_allocated += item.allocated;
                summary.removed += 1;
                progress.publish(Event::Deleted {
                    path: item.path.clone(),
                    freed: freed.max(Bytes::ZERO),
                });
                entries.push(LedgerEntry {
                    path: item.path.clone(),
                    kind: item.kind,
                    logical: item.size.0,
                    allocated: item.allocated.0,
                    trash_path: trash_destination,
                    category: Some(item.category.id().to_string()),
                });
            }
            Err(err) => {
                let message = format!("{}: {}", item.display_path(), err);
                progress.warning(message.clone());
                summary.record_error(message);
            }
        }
    }

    if !entries.is_empty() {
        let session = Session {
            id: session_id.clone(),
            started: chrono::Local::now().to_rfc3339(),
            command: job.command.clone(),
            mode: job.mode,
            logical_freed: summary.freed.0,
            allocated_freed: summary.freed_allocated.0,
            removed: summary.removed,
            failed: summary.failed,
            entries,
            duration_ms: started.elapsed().as_millis() as u64,
            restored: false,
        };
        match ledger::write_session(&session) {
            Ok(path) => progress.note(format!(
                "session {} recorded ({})",
                session_id,
                squid_core::paths::pretty(&path)
            )),
            Err(err) => progress.warning(format!("could not write history: {err}")),
        }
    }

    Ok(DeleteOutcome { summary, skipped })
}

pub fn lock_reason(lock: LockReason) -> &'static str {
    match lock {
        LockReason::AppRunning => "app is running",
        LockReason::SharedWithOtherInstall => "shared with another install of the same app",
        LockReason::Whitelisted => "whitelisted",
        LockReason::NotOwned => "owned by another user — needs admin rights",
        LockReason::Protected => "protected by safety policy",
    }
}

/// Convenience used by `squid undo`: restore a Trash session, newest first.
pub fn restore_session(id: &str) -> squid_core::Result<(usize, usize)> {
    let session = ledger::load_session(id)
        .ok_or_else(|| squid_core::SquidError::Other(format!("no history session named {id}")))?;
    Ok(ledger::restore(&session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use squid_core::{Category, ItemKind};
    use std::path::PathBuf;

    /// Managed dev boxes with an endpoint protector can refuse scratch writes
    /// from a freshly built unsigned test binary (error 5); helpers return
    /// None then and the test skips instead of reporting a code failure.
    fn denied(error: &std::io::Error) -> bool {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!("skipping: scratch write denied ({error})");
            return true;
        }
        false
    }

    /// The safety policy only allows removing things inside the user's home,
    /// so the deletable fixture has to live there too.
    fn scratch_dir(tag: &str) -> Option<PathBuf> {
        let dir = squid_core::paths::home_dir()
            .join("Library/Caches")
            .join(format!(
                "squid-delete-test-{}-{tag}-{}",
                std::process::id(),
                squid_core::ledger::new_session_id()
            ));
        match std::fs::create_dir_all(&dir) {
            Ok(()) => Some(dir),
            Err(error) if denied(&error) => None,
            Err(error) => panic!("scratch dir: {error}"),
        }
    }

    /// Ledger entries are user data: keep test sessions out of the real
    /// `~/…/squid/history` so `squid history` only shows real cleanups.
    /// Returns false when the protector refused the isolated state dir.
    fn isolate_state() -> bool {
        use std::sync::atomic::{AtomicBool, Ordering};
        static ONCE: std::sync::Once = std::sync::Once::new();
        static READY: AtomicBool = AtomicBool::new(false);
        ONCE.call_once(|| {
            let dir = std::env::temp_dir().join(format!("squid-state-{}", std::process::id()));
            match std::fs::create_dir_all(&dir) {
                Ok(()) => {
                    std::env::set_var("SQUID_STATE_DIR", &dir);
                    READY.store(true, Ordering::SeqCst);
                }
                Err(error) if denied(&error) => {}
                Err(error) => panic!("scratch state dir: {error}"),
            }
        });
        READY.load(Ordering::SeqCst)
    }

    fn temp_dir(tag: &str) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!(
            "squid-delete-test-{}-{tag}-{}",
            std::process::id(),
            squid_core::ledger::new_session_id()
        ));
        match std::fs::create_dir_all(&dir) {
            Ok(()) => Some(dir),
            Err(error) if denied(&error) => None,
            Err(error) => panic!("scratch dir: {error}"),
        }
    }

    #[test]
    fn protected_paths_are_refused_and_reported() {
        if !isolate_state() {
            return;
        }
        let engine = Engine::new();
        let item = CleanupItem::new(
            PathBuf::from("/"),
            ItemKind::Path,
            Category::SystemJunk,
            "root",
        );
        let job = DeleteJob::new(vec![item], DeleteMode::Permanent, "squid test");
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let outcome = execute(&engine, &job, &progress, &cancel).expect("run");
        assert_eq!(outcome.summary.removed, 0);
        assert_eq!(outcome.skipped.len(), 1);
        assert!(
            outcome.skipped[0].1.contains("protected") || outcome.skipped[0].1.contains("safety"),
            "{:?}",
            outcome.skipped
        );
    }

    #[test]
    fn a_temporary_file_can_be_deleted_and_ledgered() {
        if !isolate_state() {
            return;
        }
        let Some(dir) = scratch_dir("delete") else {
            return;
        };
        let file = dir.join("junk.bin");
        std::fs::write(&file, vec![7u8; 4096]).unwrap();
        let engine = Engine::new();
        let mut item = CleanupItem::new(file.clone(), ItemKind::File, Category::SystemJunk, "junk");
        item.size = Bytes(4096);
        item.allocated = Bytes(4096);
        let job = DeleteJob::new(vec![item], DeleteMode::Permanent, "squid test");
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let outcome = execute(&engine, &job, &progress, &cancel).expect("delete");
        assert_eq!(outcome.summary.removed, 1, "{:?}", outcome.summary.errors);
        assert!(!file.exists());
        let session = ledger::load_sessions(10)
            .into_iter()
            .find(|session| session.id == outcome.summary.session_id.clone().unwrap_or_default());
        assert!(session.is_some(), "session should be written to history");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Trash is the default mode, so the undo path has to work end to end:
    /// delete → the file leaves its folder and lands in the Trash → `undo`
    /// puts it back with its bytes intact.
    ///
    /// Unix-only: on Windows the shell Recycle Bin owns the destination, so
    /// the ledger records no physical path and restore goes through the
    /// Recycle Bin UI instead of `ledger::restore_session`.
    #[test]
    #[cfg(unix)]
    fn trashing_then_restoring_round_trips_the_file() {
        if !isolate_state() {
            return;
        }
        let Some(dir) = scratch_dir("undo") else {
            return;
        };
        let file = dir.join("precious.txt");
        let payload = b"the squid gnawed here, and left this behind".to_vec();
        std::fs::write(&file, &payload).unwrap();

        let engine = Engine::new();
        let mut item = CleanupItem::new(file.clone(), ItemKind::File, Category::Logs, "precious");
        item.size = Bytes(payload.len() as u64);
        item.allocated = Bytes(payload.len() as u64);
        let job = DeleteJob::new(vec![item], DeleteMode::Trash, "squid test");
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let outcome = execute(&engine, &job, &progress, &cancel).expect("trash");
        assert_eq!(outcome.summary.removed, 1, "{:?}", outcome.summary.errors);
        assert!(!file.exists(), "the file should be in the Trash now");

        let session_id = outcome.summary.session_id.clone().expect("session id");
        let session = ledger::load_session(&session_id).expect("trashed session recorded");
        let entry = session.entries.first().expect("one entry");
        let trash_path = entry.trash_path.clone().expect("trash destination");
        assert!(trash_path.exists(), "ledger points at the trashed file");

        let (restored, skipped) = restore_session(&session_id).expect("restore");
        assert_eq!((restored, skipped), (1, 0));
        assert!(file.exists(), "undo should bring the file back");
        assert_eq!(std::fs::read(&file).unwrap(), payload, "bytes intact");
        assert!(!trash_path.exists(), "the Trash copy should be gone");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locked_items_are_never_attempted() {
        if !isolate_state() {
            return;
        }
        let Some(dir) = temp_dir("locked") else {
            return;
        };
        let file = dir.join("keep.bin");
        std::fs::write(&file, b"keep").unwrap();
        let engine = Engine::new();
        let item = CleanupItem::new(file.clone(), ItemKind::File, Category::Logs, "keep")
            .locked(LockReason::Whitelisted);
        let job = DeleteJob::new(vec![item], DeleteMode::Permanent, "squid test");
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let err = execute(&engine, &job, &progress, &cancel).unwrap_err();
        assert!(matches!(err, squid_core::SquidError::NothingToDo));
        assert!(file.exists(), "locked items must survive");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
