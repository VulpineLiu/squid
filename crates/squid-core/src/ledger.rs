//! Session history: what was removed, how much it freed, and how to undo it.
//!
//! Every destructive run writes one JSON file under the state directory. That
//! gives `squid history` a timeline and `squid undo` a Trash manifest.

use crate::bytes::Bytes;
use crate::error::{Result, SquidError};
use crate::model::{DeleteMode, ItemKind};
use crate::settings::Settings;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub path: PathBuf,
    pub kind: ItemKind,
    pub logical: u64,
    pub allocated: u64,
    pub trash_path: Option<PathBuf>,
    pub category: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub started: String,
    pub command: String,
    pub mode: DeleteMode,
    pub logical_freed: u64,
    pub allocated_freed: u64,
    pub removed: usize,
    pub failed: usize,
    pub entries: Vec<LedgerEntry>,
    pub duration_ms: u64,
    /// Whether the user already put this run back with `u`.
    ///
    /// `restore` renamed the files out of ~/.Trash and used to leave the record
    /// untouched, so the ledger went on reporting 24 GB as reclaimed after every
    /// byte of it had come back. `serde(default)` keeps the history written
    /// before this flag existed readable.
    #[serde(default)]
    pub restored: bool,
}

impl Session {
    pub fn started_at(&self) -> Option<DateTime<Local>> {
        DateTime::parse_from_rfc3339(&self.started)
            .map(|d| d.with_timezone(&Local))
            .ok()
    }

    pub fn freed(&self) -> Bytes {
        let allocated = Bytes(self.allocated_freed);
        if allocated.is_zero() {
            Bytes(self.logical_freed)
        } else {
            allocated
        }
    }

    /// What the run took off the volume *and left off*.
    ///
    /// Every total the interface prints is this, not [`Session::freed`]: an
    /// undone run gave the bytes back, so counting it as reclaimed is how the
    /// header comes to promise 27 GB that `df` has never seen. The row itself
    /// still shows `freed()` — the record of what happened is true even after
    /// the undo.
    pub fn kept(&self) -> Bytes {
        if self.restored {
            Bytes::ZERO
        } else {
            self.freed()
        }
    }
}

pub fn history_dir() -> PathBuf {
    Settings::state_dir().join("history")
}

fn session_path(id: &str) -> PathBuf {
    history_dir().join(format!("{id}.json"))
}

pub fn new_session_id() -> String {
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let rand: u16 = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        std::time::SystemTime::now().hash(&mut hasher);
        (hasher.finish() & 0xffff) as u16
    };
    format!("{stamp}-{rand:04x}")
}

pub fn write_session(session: &Session) -> Result<PathBuf> {
    let dir = history_dir();
    std::fs::create_dir_all(&dir).map_err(|e| SquidError::io(&dir, e))?;
    let path = session_path(&session.id);
    let text =
        serde_json::to_string_pretty(session).map_err(|e| SquidError::Other(e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| SquidError::io(&path, e))?;
    prune(dir.clone(), Settings::load().0.history_keep);
    Ok(path)
}

fn prune(dir: PathBuf, keep: usize) {
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .collect();
    files.sort();
    if files.len() > keep {
        let stale: Vec<PathBuf> = files.drain(..files.len() - keep).collect();
        for path in stale {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn load_sessions(limit: usize) -> Vec<Session> {
    let dir = history_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<Session> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|text| serde_json::from_str::<Session>(&text).ok())
        .collect();
    sessions.sort_by(|a, b| b.id.cmp(&a.id));
    sessions.truncate(limit);
    sessions
}

pub fn load_session(id: &str) -> Option<Session> {
    let text = std::fs::read_to_string(session_path(id)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Move everything that is still in the Trash back to its original location.
/// Returns `(restored, skipped)`.
pub fn restore(session: &Session) -> (usize, usize) {
    let mut restored = 0;
    let mut skipped = 0;
    for entry in &session.entries {
        let Some(trash_path) = entry.trash_path.as_ref() else {
            skipped += 1;
            continue;
        };
        if !trash_path.exists() {
            skipped += 1;
            continue;
        }
        if entry.path.exists() {
            skipped += 1;
            continue;
        }
        if let Some(parent) = entry.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(trash_path, &entry.path).is_ok() {
            restored += 1;
        } else {
            skipped += 1;
        }
    }
    // Mark the record once the bytes are really back. Without this the ledger
    // cannot tell "cleaned 24 GB" from "cleaned 24 GB and put it back", and
    // every total built from it is a lie by the amount restored.
    if restored > 0 {
        let mut record = session.clone();
        record.restored = true;
        let _ = write_session(&record);
    }
    (restored, skipped)
}

pub fn path_exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ledger written by an older build has no `restored` key. Refusing to
    /// read it would empty the history panel on upgrade — the opposite of an
    /// honesty fix.
    #[test]
    fn yesterday_records_still_read() {
        let json = r#"{
            "id": "20260101-000000-aaaa",
            "started": "2026-01-01T00:00:00+08:00",
            "command": "squid clean",
            "mode": "trash",
            "logical_freed": 2048,
            "allocated_freed": 1024,
            "removed": 1,
            "failed": 0,
            "entries": [],
            "duration_ms": 12
        }"#;
        let session: Session = serde_json::from_str(json).expect("old record parses");
        assert!(!session.restored);
        assert_eq!(session.kept(), Bytes(1024));
    }

    /// An undone run is history, but it is not space.
    #[test]
    fn an_undone_run_stops_counting_as_space() {
        let mut session: Session = serde_json::from_str(
            r#"{
            "id": "x", "started": "2026-01-01T00:00:00+08:00", "command": "squid clean",
            "mode": "trash", "logical_freed": 2048, "allocated_freed": 1024,
            "removed": 1, "failed": 0, "entries": [], "duration_ms": 12
        }"#,
        )
        .unwrap();
        assert_eq!(session.kept(), Bytes(1024));
        session.restored = true;
        assert_eq!(session.kept(), Bytes::ZERO);
        assert_eq!(session.freed(), Bytes(1024), "the record stays true");
    }
}
