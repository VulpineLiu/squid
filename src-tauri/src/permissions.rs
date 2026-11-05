//! Optional macOS privacy access used by the guided first-run check and Settings.
//!
//! macOS has no public API that reports Full Disk Access. The app probes a small
//! set of TCC-protected locations that a system-care tool normally reads. The probe
//! only opens a file or lists a directory; it never reads their contents.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    /// `granted`, `denied`, `unknown`, or `not-required`.
    pub full_disk_access: String,
}

impl PermissionStatus {
    fn new(state: &str) -> Self {
        Self {
            full_disk_access: state.into(),
        }
    }
}

#[tauri::command]
pub fn permission_status() -> PermissionStatus {
    #[cfg(target_os = "macos")]
    {
        PermissionStatus::new(full_disk_access_state())
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionStatus::new("not-required")
    }
}

#[tauri::command]
pub fn open_full_disk_access_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = crate::process::command("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
            .status()
            .map_err(|error| format!("Could not open Full Disk Access settings: {error}"))?;
        if status.success() {
            return Ok(());
        }
        Err("Could not open Full Disk Access settings.".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Full Disk Access is macOS-specific.".into())
    }
}

#[cfg(target_os = "macos")]
fn full_disk_access_state() -> &'static str {
    let home = squid_core::paths::home_dir();
    let probes = [
        home.join("Library/Application Support/com.apple.TCC/TCC.db"),
        home.join("Library/Safari"),
        home.join("Library/Mail"),
        home.join("Library/Messages"),
    ];

    let mut readable = false;
    let mut denied = false;
    for path in probes {
        match probe(&path) {
            Probe::Readable => readable = true,
            Probe::Denied => denied = true,
            Probe::Missing | Probe::Unknown => {}
        }
    }
    if readable {
        "granted"
    } else if denied {
        "denied"
    } else {
        "unknown"
    }
}

#[cfg(target_os = "macos")]
enum Probe {
    Readable,
    Denied,
    Missing,
    Unknown,
}

#[cfg(target_os = "macos")]
fn probe(path: &std::path::Path) -> Probe {
    use std::io::ErrorKind;

    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Probe::Missing,
        Err(error) if error.kind() == ErrorKind::PermissionDenied => return Probe::Denied,
        Err(_) => return Probe::Unknown,
    };
    let result = if metadata.is_dir() {
        std::fs::read_dir(path).map(|_| ())
    } else {
        std::fs::File::open(path).map(|_| ())
    };
    match result {
        Ok(()) => Probe::Readable,
        Err(error) if error.kind() == ErrorKind::PermissionDenied => Probe::Denied,
        Err(_) => Probe::Unknown,
    }
}
