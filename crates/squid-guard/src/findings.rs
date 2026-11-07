//! What a scan decided about one file, and how it knows.
//!
//! The three levels are not a severity dial. They answer different questions,
//! and the difference is the contract the UI relies on:
//!
//! - [`Confidence::Confirmed`] means a specific published rule or hash says this
//!   exact file is bad. It may be quarantined automatically if the user opted in.
//! - [`Confidence::Suspicious`] means the file has properties that a careful
//!   person would check, with nothing published to point at. Alert, never move.
//! - [`Confidence::Advisory`] is not about the file at all — it is about the
//!   machine: protection switched off, definitions old, a directory that could
//!   not be read. Nothing to quarantine, something to say.
//!
//! Every finding carries [`Evidence`], and the UI shows that evidence rather
//! than a score. "SHA-256 is on the known-bad list" is checkable; "risk 72" is
//! not.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Advisory,
    Suspicious,
    Confirmed,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Advisory => "advisory",
            Confidence::Suspicious => "suspicious",
            Confidence::Confirmed => "confirmed",
        }
    }

    /// Only confirmed findings may be moved without asking again, and only this
    /// method decides that — the UI cannot widen it.
    pub fn auto_quarantinable(self) -> bool {
        self == Confidence::Confirmed
    }
}

/// One reason a file was reported. A finding may hold several: a binary that
/// matches a family rule *and* starts at login *and* is unsigned is three
/// statements, and a person reading the row deserves all three.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Evidence {
    /// The SHA-256 of the file is on the known-bad list.
    DeniedHash { sha256: String, threat: String },
    /// A YARA-X rule fired.
    YaraRule {
        rule: String,
        description: String,
        family: String,
    },
    /// A published allow-list entry does not cover a file that claims to be it.
    Impersonation { claimed: String, path: PathBuf },
    /// Where this entry is started from.
    StartsAtLogin { entry: String, source: String },
    /// No usable code signature.
    Unsigned { path: PathBuf, note: Option<String> },
    /// Runs from a location every normal user of this machine can rewrite.
    WritableLocation { path: PathBuf },
    /// The file the entry points at is gone.
    MissingTarget { path: PathBuf },
    /// An advisory about the machine rather than a file.
    SystemState { key: String, detail: String },
    /// Part of the scan could not be checked.
    Coverage { path: PathBuf, reason: String },
}

impl Evidence {
    /// Stable identity for de-duplication across scans, so an ignored finding
    /// stays ignored after a rescan rather than reappearing under a new row.
    pub fn key(&self) -> String {
        match self {
            Evidence::DeniedHash { sha256, threat } => format!("hash:{sha256}:{threat}"),
            Evidence::YaraRule { rule, .. } => format!("yara:{rule}"),
            Evidence::Impersonation { claimed, .. } => format!("impersonation:{claimed}"),
            Evidence::StartsAtLogin { entry, .. } => format!("startup:{entry}"),
            Evidence::Unsigned { path, .. } => format!("unsigned:{}", path.display()),
            Evidence::WritableLocation { path } => format!("writable:{}", path.display()),
            Evidence::MissingTarget { path } => format!("missing:{}", path.display()),
            Evidence::SystemState { key, .. } => format!("state:{key}"),
            Evidence::Coverage { path, reason } => {
                format!("coverage:{}:{reason}", path.display())
            }
        }
    }

    /// One line for a table cell. The full evidence stays available in details.
    pub fn summary(&self) -> String {
        match self {
            Evidence::DeniedHash { threat, .. } => format!("known-bad hash: {threat}"),
            Evidence::YaraRule { rule, family, .. } => {
                if family.is_empty() {
                    format!("matches rule {rule}")
                } else {
                    format!("matches rule {rule} ({family})")
                }
            }
            Evidence::Impersonation { claimed, .. } => {
                format!("claims to be {claimed} but is not where {claimed} lives")
            }
            Evidence::StartsAtLogin { entry, source } => {
                format!("starts at login via {source} ({entry})")
            }
            Evidence::Unsigned { note, .. } => match note {
                Some(note) => format!("not signed: {note}"),
                None => "not signed".into(),
            },
            Evidence::WritableLocation { path } => {
                format!("runs from a user-writable location: {}", path.display())
            }
            Evidence::MissingTarget { path } => {
                format!("target is gone: {}", path.display())
            }
            Evidence::SystemState { detail, .. } => detail.clone(),
            Evidence::Coverage { path, reason } => {
                format!("not checked ({}): {reason}", path.display())
            }
        }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            Evidence::Impersonation { path, .. }
            | Evidence::Unsigned { path, .. }
            | Evidence::WritableLocation { path }
            | Evidence::MissingTarget { path }
            | Evidence::Coverage { path, .. } => Some(path),
            _ => None,
        }
    }
}

/// One reported file or machine state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Backend-issued. The frontend sends this back to quarantine; it never
    /// sends a path.
    pub id: String,
    pub confidence: Confidence,
    /// The file the finding is about. `None` for machine-state advisories.
    pub path: Option<PathBuf>,
    pub display_path: Option<String>,
    /// What the thing is called in a list: file name, or the advisory subject.
    pub name: String,
    /// Human-readable statement of the threat or the reason.
    pub title: String,
    pub description: String,
    pub evidence: Vec<Evidence>,
    pub size: u64,
    pub sha256: Option<String>,
    pub signed: bool,
    pub publisher: Option<String>,
    pub found_at_ms: u64,
    /// Set once the file has been quarantined, from the ledger.
    pub quarantined: bool,
    /// The user dismissed this finding for this scan only.
    pub ignored: bool,
}

impl Finding {
    pub fn confirmed(&self) -> bool {
        self.confidence == Confidence::Confirmed
    }

    /// De-duplication identity: the same file reported by two detectors is one
    /// row with two pieces of evidence.
    pub fn fingerprint(&self) -> String {
        let path = self
            .path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "-".into());
        format!("{}|{}", self.confidence.as_str(), path)
    }
}
