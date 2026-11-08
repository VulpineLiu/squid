//! OS abstraction. The engines stay OS-agnostic and ask a [`Platform`] for
//! everything that differs between macOS, Linux and Windows.
//!
//! Adding Windows support later means implementing this trait in
//! `windows.rs` (already stubbed) — no engine changes.

use serde::{Deserialize, Serialize};
use squid_core::error::Result;
use squid_core::progress::CancelToken;
use squid_core::{AppInfo, Bytes, Volume};
use std::path::{Path, PathBuf};

pub mod apps;
pub mod console;
pub mod currency;
pub mod leftovers;
pub mod process;
pub mod process_table;
pub mod protection;
pub mod startup;
pub mod trust;
#[cfg(unix)]
pub mod volumes;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlatformId {
    MacOS,
    Linux,
    Windows,
    Unknown,
}

impl PlatformId {
    pub fn as_str(self) -> &'static str {
        match self {
            PlatformId::MacOS => "macos",
            PlatformId::Linux => "linux",
            PlatformId::Windows => "windows",
            PlatformId::Unknown => "unknown",
        }
    }

    pub fn current() -> PlatformId {
        if cfg!(target_os = "macos") {
            PlatformId::MacOS
        } else if cfg!(target_os = "linux") {
            PlatformId::Linux
        } else if cfg!(windows) {
            PlatformId::Windows
        } else {
            PlatformId::Unknown
        }
    }
}

/// A directory that may contain installed applications.
#[derive(Clone, Debug)]
pub struct AppSource {
    pub dir: PathBuf,
    pub label: String,
    /// Scan depth: 1 for `/Applications/*.app`, 2 for nested vendor folders.
    pub depth: usize,
    pub system: bool,
}

/// A directory that may contain per-app leftovers.
#[derive(Clone, Debug)]
pub struct LeftoverRoot {
    pub dir: PathBuf,
    /// How entries in this directory are named.
    pub naming: Naming,
    /// Recursing is dangerous here (e.g. `Application Support`), so matching is
    /// always done against the direct children unless this is set.
    pub deep: bool,
    pub category: squid_core::Category,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Naming {
    /// `com.example.App.plist`, `com.example.App`
    BundleId,
    /// `Safari`, `Slack`
    AppName,
    /// Either of the two.
    Either,
    /// `group.com.example.App`
    GroupContainer,
}

/// Notes a platform wants the user to see before/after removal.
#[derive(Clone, Debug, Default)]
pub struct LifecycleReport {
    pub notes: Vec<String>,
    pub stopped_services: Vec<String>,
    pub warnings: Vec<String>,
}

pub trait Platform: Send + Sync {
    fn id(&self) -> PlatformId;
    fn display_name(&self) -> String;
    fn os_version(&self) -> String;
    fn hardware_model(&self) -> String;
    /// Display adapter(s), when the platform exposes a stable name.
    fn graphics_adapter(&self) -> String {
        String::new()
    }

    /// Directories to scan for installed applications.
    fn app_sources(&self) -> Vec<AppSource>;

    /// Full app inventory, sizes included. This is the slow one: `size_apps`
    /// lets the UI render names first.
    fn list_apps(&self, cancel: &CancelToken, size_apps: bool) -> Result<Vec<AppInfo>>;

    /// Directories that may hold leftovers for the given app.
    fn leftover_roots(&self) -> Vec<LeftoverRoot>;

    /// Platform hook before anything is deleted (stop daemons, unload login
    /// items, unregister with Launch Services).
    fn prepare_uninstall(&self, app: &AppInfo) -> LifecycleReport {
        let _ = app;
        LifecycleReport::default()
    }

    /// Platform hook after deletion (refresh caches, forget pkg receipts).
    fn finish_uninstall(&self, app: &AppInfo, removed: &[PathBuf]) -> LifecycleReport {
        let _ = (app, removed);
        LifecycleReport::default()
    }

    fn volumes(&self) -> Result<Vec<Volume>>;

    /// Absolute prefixes that must never be touched.
    fn protected_prefixes(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Process names currently running (cheap snapshot, no full table).
    fn running_executables(&self) -> Vec<String>;

    /// Human readable "why is this not removable" for a path, if any.
    fn inspect_path(&self, path: &Path) -> Option<String> {
        let _ = path;
        None
    }

    /// Free space that macOS reports as "purgeable" (caches the OS would drop
    /// anyway). 0 where the concept does not exist.
    fn purgeable_space(&self, mount: &Path) -> Bytes {
        let _ = mount;
        Bytes::ZERO
    }

    /// Open the system Trash (Finder / explorer / xdg-open).
    fn reveal_trash(&self) {}

    /// What would happen to a file sent to the Trash *right now*. Cheap: one
    /// metadata check at most, since the status line calls it per render.
    fn trash_target(&self) -> TrashTarget {
        TrashTarget::Unreachable
    }

    /// What to call the machine when squid talks to a person about it. This is
    /// a property of the platform, not of the locale: the Chinese UI on a
    /// Windows box must not inherit "这台 Mac" from the macOS build either.
    fn device(&self) -> Device {
        Device::Machine
    }

    /// What the operating system calls the place deleted files wait in.
    ///
    /// Explorer says 回收站 / "Recycle Bin", and that is the only name a Windows
    /// user can act on: there is no Trash folder to open, so copy that promises
    /// "empty the Trash" points at something the user cannot find.
    fn bin_noun(&self) -> BinNoun {
        BinNoun::Trash
    }
}

/// What the operating system calls the place deleted files wait in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinNoun {
    /// macOS and the freedesktop trash: 「废纸篓」.
    Trash,
    /// Windows: 回收站.
    RecycleBin,
}

impl BinNoun {
    /// The name as an English speaker of the interface would see it.
    pub fn en(self) -> &'static str {
        match self {
            BinNoun::Trash => "Trash",
            BinNoun::RecycleBin => "Recycle Bin",
        }
    }

    /// The name as a Chinese speaker of the interface would see it.
    pub fn zh(self) -> &'static str {
        match self {
            BinNoun::Trash => "废纸篓",
            BinNoun::RecycleBin => "回收站",
        }
    }
}

/// What the machine squid runs on is called when squid addresses a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Mac,
    Pc,
    Machine,
}

/// Where a file sent to the Trash actually lands, as far as the user can tell.
///
/// `doctor` used to probe `~/.Trash` unconditionally and conclude on Windows
/// that "deletions fall back to permanent", which was wrong: the shell Recycle
/// Bin is there, it is just not a folder squid can name or list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashTarget {
    /// The shell Recycle Bin. Reachable and restorable, but the shell owns the
    /// bookkeeping, so there is no folder path to show.
    RecycleBin,
    /// A real trash folder that exists right now.
    Folder(PathBuf),
    /// Nothing reachable — sending to the Trash would end up permanent.
    Unreachable,
}

/// Concrete adapter for the host we are running on.
pub fn current() -> Box<dyn Platform> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOs)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::Linux::default())
    }
    #[cfg(windows)]
    {
        Box::new(windows::Windows)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        Box::new(Unsupported)
    }
}

/// Adapter used when the OS is not implemented yet: reports volumes through
/// `df`-style data and refuses app management.
pub struct Unsupported;

impl Platform for Unsupported {
    fn id(&self) -> PlatformId {
        PlatformId::Unknown
    }
    fn display_name(&self) -> String {
        std::env::consts::OS.to_string()
    }
    fn os_version(&self) -> String {
        std::env::consts::ARCH.to_string()
    }
    fn hardware_model(&self) -> String {
        "unknown".into()
    }
    fn app_sources(&self) -> Vec<AppSource> {
        Vec::new()
    }
    fn list_apps(&self, _cancel: &CancelToken, _size: bool) -> Result<Vec<AppInfo>> {
        Err(squid_core::SquidError::Unsupported {
            platform: std::env::consts::OS,
            what: "application inventory",
        })
    }
    fn leftover_roots(&self) -> Vec<LeftoverRoot> {
        Vec::new()
    }
    fn volumes(&self) -> Result<Vec<Volume>> {
        Ok(Vec::new())
    }
    fn running_executables(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Convenience: can the current user write this path?
#[cfg(not(windows))]
pub fn is_user_writable(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match std::fs::metadata(path) {
        Ok(meta) => {
            let uid = unsafe { libc::getuid() };
            if uid == 0 {
                return true;
            }
            meta.uid() == uid && meta.mode() & 0o200 != 0
        }
        Err(_) => false,
    }
}

/// Windows has no uid; the read-only attribute is the portable signal and
/// ACL checks would need a token walk we cannot afford per path.
#[cfg(windows)]
pub fn is_user_writable(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => !meta.permissions().readonly(),
        Err(_) => false,
    }
}

/// Nearest existing ancestor that exists (for ownership checks on paths that
/// are about to disappear).
pub fn closest_existing(path: &Path) -> PathBuf {
    let mut cursor = path.to_path_buf();
    loop {
        if cursor.exists() {
            return cursor;
        }
        if !cursor.pop() {
            return PathBuf::from("/");
        }
    }
}
