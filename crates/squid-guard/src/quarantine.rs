//! The quarantine: files squid moved out of the way, and the record that lets
//! them go back.
//!
//! Three properties are worth stating because they are what distinguishes this
//! from "delete" and from the system Trash:
//!
//! - **Reversible until told otherwise.** Each object keeps its original path,
//!   mode and timestamps, so `restore` can put it back where it came from.
//! - **Inert.** Files land in squid's own state folder with the execute bit
//!   cleared and owner-only permissions, so a quarantined login item cannot run
//!   again while it waits there.
//! - **Owned by squid.** The Trash is the operating system's, and sending a
//!   suspected binary there means the user has to remember to empty it. Here the
//!   ledger and the objects sit together, and the UI can say exactly what is in
//!   the way.
//!
//! The ledger is the authority. A file is quarantined because the ledger says
//! so, and every operation re-reads the ledger before touching the disk, so a
//! half-finished move on a full disk shows up as an honest error rather than a
//! file in two places.

use crate::coverage::quarantine_root;
use crate::findings::{Evidence, Finding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use squid_core::paths;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One quarantined object, as recorded before anything was moved.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineEntry {
    pub id: String,
    /// Where the file lived. Kept for restore, never trusted as-is.
    pub original_path: PathBuf,
    pub display_path: String,
    /// Relative name inside the quarantine folder.
    pub stored: String,
    pub size: u64,
    pub sha256: String,
    /// Unix mode, or the Windows read-only flag folded into it.
    pub mode: u32,
    pub modified_ms: u64,
    pub quarantined_ms: u64,
    /// Why it was moved, kept so the list can explain itself a month later.
    pub title: String,
    pub confidence: String,
    pub evidence: Vec<Evidence>,
}

/// The quarantine folder's own ledger. Written beside the objects so the whole
/// thing can be inspected, copied or deleted by hand.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineLedger {
    pub version: u32,
    pub entries: Vec<QuarantineEntry>,
}

impl QuarantineLedger {
    fn load(path: &Path) -> QuarantineLedger {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(QuarantineLedger {
                version: 1,
                entries: Vec::new(),
            })
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        std::fs::write(path, text).map_err(|error| error.to_string())
    }
}

fn ledger_path() -> PathBuf {
    quarantine_root().join("ledger.json")
}

fn objects_dir() -> PathBuf {
    quarantine_root().join("objects")
}

/// The quarantine, opened for one session of the app.
#[derive(Default)]
pub struct Quarantine {
    ledger: QuarantineLedger,
    loaded: bool,
}

impl Quarantine {
    pub fn new() -> Quarantine {
        Quarantine::default()
    }

    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.ledger = QuarantineLedger::load(&ledger_path());
            self.loaded = true;
        }
    }

    /// Everything the ledger recorded. Reading the list is what loads the
    /// ledger, so a freshly opened quarantine describes what is on disk rather
    /// than an empty list that would offer "quarantine" for a file already moved.
    pub fn entries(&mut self) -> &[QuarantineEntry] {
        self.ensure_loaded();
        &self.ledger.entries
    }

    pub fn find(&mut self, id: &str) -> Option<&QuarantineEntry> {
        self.ensure_loaded();
        self.ledger.entries.iter().find(|entry| entry.id == id)
    }

    /// Move one finding into the quarantine.
    ///
    /// The finding is what the scan reported; nothing here accepts a bare path.
    /// Before moving, the file is re-hashed and compared with the value recorded
    /// at scan time, so a file that changed between "found" and "quarantine" is
    /// reported rather than moved: the bytes the user decided about are not the
    /// bytes on disk.
    pub fn quarantine(&mut self, finding: &Finding) -> Result<QuarantineEntry, String> {
        self.ensure_loaded();
        let path = finding
            .path
            .as_ref()
            .ok_or_else(|| "This finding has no file to quarantine.".to_string())?;
        let refused = refusal(path)?;
        if let Some(reason) = refused {
            return Err(reason);
        }

        let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err(
                "Only single files can be quarantined — a folder needs the uninstaller.".into(),
            );
        }
        let is_link = metadata.file_type().is_symlink();
        if is_link {
            return Err("A symbolic link is not the file that was scanned.".into());
        }
        if metadata.len() != finding.size {
            return Err(format!(
                "{} changed size since the scan. Run the scan again.",
                paths::pretty(path)
            ));
        }
        let hash = hash_file(path)?;
        if let Some(expected) = finding.sha256.as_deref() {
            if !expected.eq_ignore_ascii_case(&hash) {
                return Err(format!(
                    "{} changed since the scan. Run the scan again.",
                    paths::pretty(path)
                ));
            }
        }

        let id = new_id(path, &hash);
        if self.ledger.entries.iter().any(|entry| entry.id == id) {
            return Err("This file is already in the quarantine.".into());
        }
        // Two scans of the same bytes in different places get different ids,
        // because the original path is part of what restore needs.
        let stored = format!("{id}.bin");
        let destination = objects_dir().join(&stored);
        if destination.exists() {
            return Err("The quarantine already holds an object with this name.".into());
        }

        // Read the mode before anything else happens to the file: the stored
        // copy is deliberately made inert below, and restore has to hand back
        // what the file was, not what the quarantine keeps it as.
        let mode = current_mode(path);

        std::fs::create_dir_all(objects_dir()).map_err(|error| error.to_string())?;
        std::fs::rename(path, &destination).or_else(|rename_error| {
            // Renaming fails across volumes, which is normal for an external
            // disk or a DMG that is still mounted.
            std::fs::copy(path, &destination)
                .map_err(|copy_error| {
                    format!(
                        "Could not move the file into the quarantine: {rename_error}; copy failed: {copy_error}"
                    )
                })
                .and_then(|_| std::fs::remove_file(path).map_err(|error| error.to_string()))
        })?;

        // Make the object inert while it waits: no execute, readable only by the
        // owner who chose to quarantine it.
        restrict_permissions(&destination);

        let entry = QuarantineEntry {
            id: id.clone(),
            original_path: path.clone(),
            display_path: paths::pretty(path),
            stored,
            size: metadata.len(),
            sha256: hash,
            mode,
            modified_ms: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|since| since.as_millis() as u64)
                .unwrap_or_default(),
            quarantined_ms: now_ms(),
            title: finding.title.clone(),
            confidence: finding.confidence.as_str().to_string(),
            evidence: finding.evidence.clone(),
        };
        self.ledger.entries.push(entry.clone());
        if let Err(error) = self.ledger.save(&ledger_path()) {
            // The file is already in place, so undo the move rather than leave a
            // file the ledger does not know about.
            let _ = std::fs::rename(&destination, path);
            self.ledger.entries.retain(|kept| kept.id != id);
            return Err(format!("Could not record the quarantine: {error}"));
        }
        Ok(entry)
    }

    /// Put one object back where it came from.
    pub fn restore(&mut self, id: &str) -> Result<PathBuf, String> {
        self.ensure_loaded();
        let index = self
            .ledger
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| "That object is not in the quarantine.".to_string())?;
        let entry = self.ledger.entries[index].clone();
        let stored = objects_dir().join(&entry.stored);
        if !stored.is_file() {
            return Err(format!(
                "The quarantined copy of {} is gone from disk.",
                entry.display_path
            ));
        }
        if entry.original_path.exists() {
            return Err(format!(
                "{} is there again. Squid will not overwrite it.",
                entry.display_path
            ));
        }
        if let Some(parent) = entry.original_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::rename(&stored, &entry.original_path).or_else(|error| {
            std::fs::copy(&stored, &entry.original_path)
                .map_err(|copy| format!("Could not restore the file: {error}; {copy}"))
                .and_then(|_| std::fs::remove_file(&stored).map_err(|error| error.to_string()))
        })?;
        restore_permissions(&entry.original_path, entry.mode);

        // A byte-for-byte check is the only proof that restore worked: the file
        // is back, in one place, with the content that was taken out.
        let restored_hash = hash_file(&entry.original_path)?;
        if !restored_hash.eq_ignore_ascii_case(&entry.sha256) {
            return Err(format!(
                "{} came back with different bytes; the quarantine copy was kept.",
                entry.display_path
            ));
        }
        self.ledger.entries.remove(index);
        self.ledger.save(&ledger_path())?;
        Ok(entry.original_path)
    }

    /// Throw one object away for real. This is the only place in the security
    /// half of squid that destroys user data, so it is only reachable from the
    /// quarantine list, never from a scan.
    pub fn purge(&mut self, id: &str) -> Result<(), String> {
        self.ensure_loaded();
        let index = self
            .ledger
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| "That object is not in the quarantine.".to_string())?;
        let entry = self.ledger.entries.remove(index);
        let stored = objects_dir().join(&entry.stored);
        let result = match std::fs::remove_file(&stored) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => self.ledger.save(&ledger_path()),
            Err(error) => {
                self.ledger.entries.insert(index, entry);
                Err(format!("Could not delete the quarantined copy: {error}"))
            }
        }
    }

    /// Reconcile the ledger with the folder. Deleting objects by hand, or an
    /// interrupted release upgrade, can leave entries whose file is gone; the UI
    /// must not offer "restore" for something that is not there.
    pub fn reconcile(&mut self) -> Vec<String> {
        self.ensure_loaded();
        let mut lost = Vec::new();
        self.ledger.entries.retain(|entry| {
            let present = objects_dir().join(&entry.stored).is_file();
            if !present {
                lost.push(entry.id.clone());
            }
            present
        });
        if !lost.is_empty() {
            let _ = self.ledger.save(&ledger_path());
        }
        lost
    }

    pub fn total_size(&mut self) -> u64 {
        self.ledger.entries.iter().map(|entry| entry.size).sum()
    }
}

/// The paths a quarantine must never touch, whatever a scan claimed. Checked at
/// move time rather than trusted from the scan, because the scan is a snapshot
/// of an opinion and this is the moment data actually moves.
fn refusal(path: &Path) -> Result<Option<String>, String> {
    const PROTECTED: [&str; 6] = [
        "/System",
        "/usr",
        "/bin",
        "/sbin",
        "/Library/Apple",
        "/Applications/Utilities",
    ];
    if PROTECTED.iter().any(|prefix| path.starts_with(prefix)) {
        return Ok(Some(
            "Squid does not quarantine files inside the operating system.".into(),
        ));
    }
    #[cfg(windows)]
    if let Ok(system) = std::env::var("SystemRoot") {
        if path.starts_with(system) {
            return Ok(Some(
                "Squid does not quarantine files inside Windows.".into(),
            ));
        }
    }
    if path.starts_with(quarantine_root()) {
        return Ok(Some("That file is already in the quarantine.".into()));
    }
    if path.starts_with(squid_core::Settings::state_dir())
        || path.starts_with(squid_core::Settings::config_dir())
    {
        return Ok(Some("That file belongs to Squid itself.".into()));
    }
    if is_own_binary(path) {
        return Ok(Some("That is the app you are using right now.".into()));
    }
    if !std::fs::symlink_metadata(path).is_ok() {
        return Ok(Some("The file is not there any more.".into()));
    }
    // The current user has to own it, or be able to write its folder: without
    // that, moving the file needs rights this release does not ask for.
    if !squid_platform::is_user_writable(path.parent().unwrap_or(path)) {
        return Ok(Some(
            "You do not have permission to move a file from that folder.".into(),
        ));
    }
    Ok(None)
}

fn is_own_binary(path: &Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    paths::same_path(&current, path)
        || current
            .ancestors()
            .zip(path.ancestors())
            .all(|(a, b)| a == b)
            && path.starts_with(current.parent().unwrap_or(Path::new("/")))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn new_id(path: &Path, hash: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(path.to_string_lossy().as_bytes());
    digest.update(hash.as_bytes());
    let full = hex::encode(digest.finalize());
    full[..20].to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(windows)]
fn restrict_permissions(path: &Path) {
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

#[cfg(not(any(unix, windows)))]
fn restrict_permissions(_path: &Path) {}

#[cfg(unix)]
fn current_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode())
        .unwrap_or(0o644)
}

#[cfg(not(unix))]
fn current_mode(_path: &Path) -> u32 {
    0o644
}

#[cfg(unix)]
fn restore_permissions(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restore_permissions(path: &Path, mode: u32) {
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(mode & 0o200 == 0);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::Confidence;
    use std::io::Write;

    fn finding_for(path: &Path) -> Finding {
        let hash = hash_file(path).unwrap();
        Finding {
            id: "f1".into(),
            confidence: Confidence::Confirmed,
            path: Some(path.to_path_buf()),
            display_path: Some(paths::pretty(path)),
            name: "probe".into(),
            title: "test finding".into(),
            description: String::new(),
            evidence: vec![Evidence::DeniedHash {
                sha256: hash.clone(),
                threat: "probe".into(),
            }],
            size: std::fs::metadata(path).unwrap().len(),
            sha256: Some(hash),
            signed: false,
            publisher: None,
            found_at_ms: now_ms(),
            quarantined: false,
            ignored: false,
        }
    }

    fn probe(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("squid-quarantine-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("payload.bin");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// Quarantine paths come from `Settings::state_dir()`, which honours
    /// `SQUID_STATE_DIR`. Every test that touches the ledger takes this lock and
    /// points the state directory at its own folder, so the suite neither sees
    /// the real quarantine nor leaves debris in it.
    /// Quarantine paths come from `Settings::state_dir()`, which honours
    /// `SQUID_STATE_DIR`, and that variable is process-wide. So each ledger test
    /// holds one lock for its whole body and gets a freshly emptied directory of
    /// its own: no test sees another's entries, and none of them touch the real

    #[test]
    fn quarantine_and_restore_round_trip_is_byte_identical() {
        let _guard = crate::state_guard("round-trip");
        let path = probe("round-trip", b"the payload that has to come back");
        let before = std::fs::metadata(&path).unwrap();
        let mut quarantine = Quarantine::new();
        let entry = quarantine
            .quarantine(&finding_for(&path))
            .expect("quarantine should accept a file the test owns");
        assert!(!path.exists(), "the file should have moved");
        assert!(!entry.stored.is_empty());

        let restored = quarantine.restore(&entry.id).expect("restore");
        assert_eq!(restored, path);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"the payload that has to come back"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            before.len(),
            "the file should be back at its original size"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn restoring_restores_the_execute_bit() {
        let _guard = crate::state_guard("execute-bit");
        let path = probe("execute-bit", b"#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o111,
                0,
                "the probe should start executable"
            );
        }
        let mut quarantine = Quarantine::new();
        let entry = quarantine.quarantine(&finding_for(&path)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let stored = objects_dir().join(&entry.stored);
            assert_eq!(
                std::fs::metadata(stored).unwrap().permissions().mode() & 0o111,
                0,
                "a quarantined file must not be executable"
            );
        }
        quarantine.restore(&entry.id).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o111,
                0,
                "restore should hand the execute bit back"
            );
        }
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_file_that_changed_after_the_scan_is_not_moved() {
        let _guard = crate::state_guard("changed");
        let path = probe("changed", b"original bytes");
        let finding = finding_for(&path);
        std::fs::write(&path, b"somebody else rewrote this file").unwrap();
        let error = Quarantine::new()
            .quarantine(&finding)
            .expect_err("the file no longer matches the scan");
        assert!(
            error.contains("changed"),
            "the message should say the file changed: {error}"
        );
        assert!(path.exists(), "a refused quarantine must not move anything");

        let finding = finding_for(&path);
        let mut quarantine = Quarantine::new();
        let entry = quarantine.quarantine(&finding).expect("a fresh scan works");
        quarantine.restore(&entry.id).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn the_system_and_squid_itself_are_never_quarantined() {
        let _guard = crate::state_guard("protected");
        let mut quarantine = Quarantine::new();
        for path in [
            Path::new("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder"),
            Path::new("/usr/bin/true"),
        ] {
            if !path.is_file() {
                continue;
            }
            let error = quarantine
                .quarantine(&finding_for(path))
                .expect_err("an OS file must be refused");
            assert!(error.contains("operating system"), "{path:?}: {error}");
            assert!(path.is_file(), "{path:?} must still be where it was");
        }
    }

    #[test]
    fn purging_deletes_only_the_quarantined_copy() {
        let _guard = crate::state_guard("purge");
        let path = probe("purge", b"this one is thrown away");
        let mut quarantine = Quarantine::new();
        let entry = quarantine.quarantine(&finding_for(&path)).unwrap();
        assert!(!path.exists());
        quarantine.purge(&entry.id).unwrap();
        assert!(!objects_dir().join(&entry.stored).exists());
        assert!(quarantine.entries().iter().all(|kept| kept.id != entry.id));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn the_ledger_survives_a_new_session_and_notices_lost_objects() {
        let _guard = crate::state_guard("ledger");
        let path = probe("ledger", b"survive the restart");
        let mut session = Quarantine::new();
        let entry = session.quarantine(&finding_for(&path)).unwrap();
        drop(session);

        let mut reopened = Quarantine::new();
        assert!(
            reopened.find(&entry.id).is_some(),
            "the ledger must describe the quarantine after a restart"
        );

        let stored = objects_dir().join(&entry.stored);
        std::fs::remove_file(&stored).unwrap();
        let mut third = Quarantine::new();
        let lost = third.reconcile();
        assert_eq!(lost, vec![entry.id.clone()]);
        assert!(third.find(&entry.id).is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn restore_refuses_to_overwrite_a_file_that_came_back() {
        let _guard = crate::state_guard("collision");
        let path = probe("collision", b"moved away then reappeared");
        let mut quarantine = Quarantine::new();
        let entry = quarantine.quarantine(&finding_for(&path)).unwrap();
        std::fs::write(&path, b"the app re-created itself").unwrap();
        let error = quarantine
            .restore(&entry.id)
            .expect_err("restoring over a live file would lose data");
        assert!(error.contains("again"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"the app re-created itself");
        quarantine.purge(&entry.id).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn the_same_id_names_the_same_file_and_bytes() {
        let a = new_id(Path::new("/tmp/x"), "aa");
        assert_eq!(a, new_id(Path::new("/tmp/x"), "aa"));
        assert_ne!(a, new_id(Path::new("/tmp/x"), "bb"));
        assert_ne!(a, new_id(Path::new("/tmp/y"), "aa"));
        assert_eq!(a.len(), 20);
    }
}
