//! squid-guard — the security half of squid.
//!
//! Position matters more than ambition here. XProtect (macOS) and Defender
//! (Windows) remain the real-time protection of this machine; squid does not
//! replace them, does not copy their rules, and does not install anything that
//! runs before execve. What this crate adds is the part those engines leave to
//! the owner of the machine: an inventory of *what* starts at login, an
//! evidence-grade scan of the files behind that inventory and the obvious
//! user-writable run locations, and a reversible quarantine that never deletes.
//!
//! Two rules keep the crate honest:
//!
//! 1. A scan never removes anything. It produces [`finding::Finding`]s with
//!    evidence, and only an explicit user decision (or the opt-in automatic
//!    quarantine, confirmed-level only) moves a file.
//! 2. Findings are addressed by backend-issued id, never by path. The caller
//!    cannot quarantine a file the scan never reported.
//!
//! Detection is layered so a weak layer can never masquerade as a strong one:
//!
//! - a SHA-256 blacklist answers "is this exact file known bad" (`confirmed`);
//! - YARA-X rules answer "does this file match a known family" (`confirmed` for
//!   high-confidence rules, `suspicious` otherwise);
//! - heuristics answer "would a careful person look at this twice"
//!   (`suspicious`/`advisory`) and never quarantine on their own.

pub mod coverage;
pub mod findings;
pub mod heuristics;
pub mod intel;
pub mod quarantine;
pub mod rules;
pub mod scan;
pub mod store;

pub use findings::{Confidence, Evidence, Finding};
pub use quarantine::{QuarantineEntry, QuarantineLedger};
pub use scan::{ScanMode, ScanReport as GuardReport, ScanTarget};
pub use store::Store;

/// Crate version tag written into the quarantine ledger, mirroring the
/// cleanup ledger's habit of stamping the format it was written by.
pub const LEDGER_TAG: &str = concat!("squid-guard/", env!("CARGO_PKG_VERSION"));

/// Tests that touch on-disk state share one lock, because the state directory is
/// chosen by a process-wide environment variable. Without it, a quarantine test
/// can reopen its ledger in whatever directory an intel test pointed the process
/// at a moment earlier.
#[cfg(test)]
pub(crate) fn state_guard(tag: &str) -> std::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = std::env::temp_dir().join(format!("squid-guard-state-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a test can create its own state directory");
    std::env::set_var("SQUID_STATE_DIR", &dir);
    guard
}
