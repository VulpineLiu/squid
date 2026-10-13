//! # squid-core
//!
//! Shared vocabulary and safety machinery for squid. Everything that could
//! destroy user data lives behind [`safety::SafetyPolicy`] and
//! [`fs::remove_item`]; engines are expected to never call `std::fs::remove*`
//! directly.

pub mod bytes;
pub mod error;
pub mod fs;
pub mod ledger;
pub mod model;
pub mod paths;
pub mod progress;
pub mod safety;
pub mod settings;

pub use bytes::Bytes;
pub use error::{Result, SquidError};
pub use model::{
    AppInfo, Category, CleanSummary, CleanupItem, DeleteMode, ItemKind, LockReason, ScanReport,
    UninstallPlan, Volume,
};
pub use progress::{CancelToken, Event, Progress};
pub use safety::SafetyPolicy;
pub use settings::{Locale, LocaleSource, Settings};

/// Crate version, used by `squid --version` and the history format tag.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One-liner shown at the top of every TUI screen.
pub const TAGLINE: &str = "clean · uninstall · purge · watch";
