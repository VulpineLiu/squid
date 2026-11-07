//! Which parts of the machine a scan actually looked at.
//!
//! A cleaner can be vague about coverage because missing a cache costs nothing.
//! A security scan cannot: "no threats found" that quietly skipped
//! `~/Library/Mail` for lack of Full Disk Access is worse than no answer at all.
//! So every root records what happened to it, and the report is only allowed to
//! say "clean" for the roots that were walked.
//!
//! Nothing here asks the operating system for permission and nothing here
//! retries a refusal. A denied directory is a fact about this run, reported as
//! one, and the person using squid decides whether to grant access and rescan.

use serde::{Deserialize, Serialize};
use squid_core::paths;
use std::path::{Path, PathBuf};

/// What happened to one root during a scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RootOutcome {
    /// Every readable entry beneath it was offered to the detectors.
    Walked,
    /// The root itself could not be opened — usually a privacy permission.
    Denied,
    /// It was not there, which is not a gap in coverage.
    Absent,
    /// The user or a rule excluded it on purpose.
    Excluded,
    /// Deliberately not part of this scan mode.
    OutOfScope,
    /// Stopped early: cancelled, or the entry budget ran out.
    Partial,
}

impl RootOutcome {
    pub fn counts_as_gap(self) -> bool {
        matches!(self, RootOutcome::Denied | RootOutcome::Partial)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RootOutcome::Walked => "walked",
            RootOutcome::Denied => "denied",
            RootOutcome::Absent => "absent",
            RootOutcome::Excluded => "excluded",
            RootOutcome::OutOfScope => "out-of-scope",
            RootOutcome::Partial => "partial",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootRecord {
    pub path: PathBuf,
    pub display_path: String,
    pub outcome: RootOutcome,
    pub reason: Option<String>,
    pub files: usize,
    pub scanned: usize,
    pub skipped: usize,
}

/// The whole coverage statement for one scan.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub roots: Vec<RootRecord>,
}

impl Coverage {
    pub fn push(&mut self, mut record: RootRecord) {
        if record.display_path.is_empty() {
            record.display_path = paths::pretty(&record.path);
        }
        self.roots.push(record);
    }

    /// True when the scan saw everything it meant to see. This is the only
    /// condition under which the UI is allowed to say "nothing found" without a
    /// qualifier next to it.
    pub fn complete(&self) -> bool {
        !self.roots.iter().any(|root| root.outcome.counts_as_gap())
    }

    pub fn gaps(&self) -> Vec<&RootRecord> {
        self.roots
            .iter()
            .filter(|root| root.outcome.counts_as_gap())
            .collect()
    }

    pub fn walked_files(&self) -> usize {
        self.roots.iter().map(|root| root.scanned).sum()
    }

    pub fn skipped_files(&self) -> usize {
        self.roots.iter().map(|root| root.skipped).sum()
    }

    /// Record the outcome of walking one root, folding in the entry count.
    pub fn record_walk(
        &mut self,
        root: &Path,
        outcome: RootOutcome,
        reason: Option<String>,
        files: usize,
        scanned: usize,
    ) {
        self.push(RootRecord {
            path: root.to_path_buf(),
            display_path: String::new(),
            outcome,
            reason,
            files,
            scanned,
            skipped: files.saturating_sub(scanned),
        });
    }
}

/// Decide what happened to a root from the error reading it, so a missing
/// `~/Applications` is not reported as a coverage gap while a refused
/// `~/Library/Mail` is.
pub fn outcome_for_error(error: &std::io::Error) -> RootOutcome {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => RootOutcome::Absent,
        ErrorKind::PermissionDenied => RootOutcome::Denied,
        _ => RootOutcome::Denied,
    }
}

/// A path the scan should never descend into, wherever it appears.
pub fn is_never_scanned(path: &Path) -> bool {
    const SKIP_DIRS: [&str; 8] = [
        ".git",
        "node_modules",
        ".Trash",
        ".Trashes",
        "$RECYCLE.BIN",
        "System Volume Information",
        ".Spotlight-V100",
        ".fseventsd",
    ];
    // The quarantine holds files this crate moved deliberately; scanning it
    // would report the same finding forever.
    if path.starts_with(quarantine_root()) {
        return true;
    }
    path.ancestors()
        .filter_map(|ancestor| ancestor.file_name())
        .any(|name| SKIP_DIRS.iter().any(|skip| name.eq(*skip)))
}

/// Where quarantined files live. One place, inside the state directory, so the
/// user can always find and empty it.
pub fn quarantine_root() -> PathBuf {
    squid_core::Settings::state_dir()
        .join("security")
        .join("quarantine")
}

/// Where rule packs and the hash cache live.
pub fn intel_root() -> PathBuf {
    squid_core::Settings::state_dir()
        .join("security")
        .join("intel")
}

/// Where startup-item backups are kept before a change.
pub fn startup_backup_root() -> PathBuf {
    squid_core::Settings::state_dir()
        .join("security")
        .join("startup-backups")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_folder_is_not_a_coverage_gap() {
        let error = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(outcome_for_error(&error), RootOutcome::Absent);
        assert!(!RootOutcome::Absent.counts_as_gap());
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(outcome_for_error(&denied), RootOutcome::Denied);
        assert!(RootOutcome::Denied.counts_as_gap());
    }

    #[test]
    fn coverage_is_only_complete_without_denials() {
        let mut coverage = Coverage::default();
        coverage.record_walk(Path::new("/a"), RootOutcome::Walked, None, 10, 10);
        coverage.record_walk(Path::new("/b"), RootOutcome::Absent, None, 0, 0);
        assert!(coverage.complete());
        coverage.record_walk(
            Path::new("/c"),
            RootOutcome::Denied,
            Some("Full Disk Access".into()),
            0,
            0,
        );
        assert!(!coverage.complete());
        assert_eq!(coverage.gaps().len(), 1);
        assert_eq!(coverage.walked_files(), 10);
        assert_eq!(coverage.skipped_files(), 0);
    }

    #[test]
    fn vendored_and_recycle_directories_are_never_walked() {
        assert!(is_never_scanned(Path::new(
            "/Users/x/proj/node_modules/left-pad"
        )));
        assert!(is_never_scanned(Path::new(
            "/Volumes/Data/.Trash/thing.app"
        )));
        assert!(is_never_scanned(&quarantine_root().join("payload.bin")));
        assert!(!is_never_scanned(Path::new("/Applications/Calculator.app")));
    }

    #[test]
    fn quarantine_lives_in_the_state_directory() {
        assert!(
            quarantine_root().ends_with("security/quarantine")
                || quarantine_root()
                    .to_string_lossy()
                    .replace('\\', "/")
                    .ends_with("security/quarantine")
        );
    }
}
