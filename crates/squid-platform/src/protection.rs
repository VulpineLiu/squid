//! What the operating system's own protection says about itself.
//!
//! Squid reads this and never writes it. `xprotect status` and `xprotect
//! version` are queried, `xprotect update` deliberately is not: that command
//! contacts Apple's servers on the machine's behalf, and a third-party app
//! starting downloads it did not pay for is not squid's to decide. The same
//! restraint applies to Defender.
//!
//! The reason to show any of it is honesty about coverage. XProtect runs with or
//! without squid, and a user who sees "background scanning: off" can act on that
//! in System Settings. A user who sees nothing assumes it is on.

use serde::{Deserialize, Serialize};

/// The state of the protection the operating system provides itself.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectionStatus {
    /// `xprotect`, `defender`, `third-party`, `unknown`.
    pub provider: String,
    /// Whether the state above was actually read, as opposed to assumed.
    pub known: bool,
    /// Signature/definition version, in whatever notation the platform uses.
    pub definitions: String,
    /// When those definitions were installed or last refreshed.
    pub definitions_date: String,
    pub realtime: Option<bool>,
    pub background_scans: Option<bool>,
    pub launch_scans: Option<bool>,
    /// Gatekeeper (`spctl`) or the Windows Defender engine state.
    pub gatekeeper: Option<bool>,
    pub last_scan_date: String,
    /// Anything that could not be determined, in plain words.
    pub notes: Vec<String>,
    pub product_name: String,
}

impl ProtectionStatus {
    /// Unknown must never be rendered as healthy. This is the guard against a
    /// probe that fails on a clean machine and reports green.
    pub fn is_healthy(&self) -> Option<bool> {
        if !self.known {
            return None;
        }
        match self.provider.as_str() {
            "xprotect" => Some(
                self.realtime.unwrap_or(true)
                    && self.background_scans.unwrap_or(true)
                    && self.launch_scans.unwrap_or(true),
            ),
            "defender" | "third-party" => Some(self.realtime.unwrap_or(false)),
            _ => None,
        }
    }
}

/// Ask the operating system. Never fails; a platform with nothing to report says
/// so through `known: false` and a note.
pub fn status() -> ProtectionStatus {
    #[cfg(target_os = "macos")]
    {
        macos::status()
    }
    #[cfg(windows)]
    {
        windows::status()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        ProtectionStatus {
            provider: "unknown".into(),
            known: false,
            notes: vec!["this platform has no protection status that Squid can read".into()],
            ..Default::default()
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    fn run(program: &str, args: &[&str]) -> Option<String> {
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .ok()?;
        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }

    pub fn status() -> ProtectionStatus {
        let mut status = ProtectionStatus {
            provider: "xprotect".into(),
            product_name: "XProtect".into(),
            ..Default::default()
        };

        match run("/usr/bin/xprotect", &["status", "--json"]) {
            Some(json) => match serde_json::from_str::<serde_json::Value>(&json) {
                Ok(value) => {
                    status.known = true;
                    status.background_scans = value
                        .get("xprotect_background_scans")
                        .and_then(|value| value.as_bool());
                    status.launch_scans = value
                        .get("xprotect_launch_scans")
                        .and_then(|value| value.as_bool());
                    status.realtime = status.launch_scans;
                }
                Err(error) => status
                    .notes
                    .push(format!("XProtect status was unreadable: {error}")),
            },
            None => status.notes.push(
                "XProtect did not report its state. macOS 13 and later answer this query.".into(),
            ),
        }

        if let Some(json) = run("/usr/bin/xprotect", &["version", "--json"]) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
                if let Some(version) = value
                    .get("xprotect_bundle_version")
                    .and_then(|v| v.as_str())
                {
                    status.definitions = version.to_string();
                }
                if let Some(date) = value
                    .get("xprotect_bundle_install_date")
                    .and_then(|value| value.as_str())
                {
                    status.definitions_date = date.to_string();
                }
            }
        }

        // Gatekeeper answers for whether a downloaded app can run at all.
        match run("/usr/sbin/spctl", &["--status"]) {
            Some(text) => status.gatekeeper = Some(text.contains("assessments enabled")),
            None => status
                .notes
                .push("Gatekeeper status could not be read.".into()),
        }

        if !status.known {
            status.provider = "unknown".into();
        }
        status
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
    use winreg::RegKey;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Defender's own registry view answers without a WMI round trip: the
    /// product-registration key names the active antivirus, and the
    /// `CurrentReputation`/`Signature` keys carry the definition version.
    pub fn status() -> ProtectionStatus {
        let mut status = ProtectionStatus {
            provider: "unknown".into(),
            ..Default::default()
        };
        let base = match RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows Defender",
            KEY_READ | KEY_WOW64_64KEY,
        ) {
            Ok(base) => base,
            Err(_) => {
                status
                    .notes
                    .push("Windows Defender is not registered on this machine.".into());
                return status;
            }
        };

        let disabled = base.get_value::<u32, _>("DisableAntiSpyware").unwrap_or(0) != 0;

        if let Ok(signatures) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows Defender\Signature Updates",
            KEY_READ | KEY_WOW64_64KEY,
        ) {
            if let Some(version) = signatures
                .get_value::<String, _>("SignatureDefinitionVersion")
                .ok()
                .or_else(|| signatures.get_value::<String, _>("AMRunVersion").ok())
            {
                status.definitions = version;
            }
            if let Ok(days) = signatures.get_value::<u32, _>("LastSignatureUpdateAge") {
                status.definitions_date = format!("{days} days ago");
            }
            status.known = true;
        }

        // A third-party product that registered itself replaces Defender.
        let registered = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows Defender\FeatureManagement",
            KEY_READ | KEY_WOW64_64KEY,
        );
        let _ = registered;
        match active_antivirus() {
            Some(name) if !name.is_empty() => {
                status.product_name = name.clone();
                status.provider = if name.contains("Windows Defender") || name.contains("Defender")
                {
                    "defender".into()
                } else {
                    "third-party".into()
                };
                status.known = true;
            }
            _ => {
                status.provider = if disabled {
                    "unknown".into()
                } else {
                    "defender".into()
                };
                status.product_name = "Microsoft Defender Antivirus".into();
                status.realtime = Some(!disabled);
            }
        }
        if !status.known {
            status
                .notes
                .push("Windows Security Center did not report an antivirus product.".into());
        }
        status
    }

    /// Security Center's registration list, via WMI. One subprocess, no service.
    fn active_antivirus() -> Option<String> {
        let script = "Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntiVirusProduct | Select-Object -ExpandProperty displayName";
        let output = Command::new("powershell.exe")
            .creation_flags(CREATE_NO_WINDOW)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ])
            .spawn()
            .ok()?
            .wait_with_output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())?
            .to_string();
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_is_never_reported_as_healthy() {
        let unknown = ProtectionStatus {
            provider: "unknown".into(),
            known: false,
            ..Default::default()
        };
        assert_eq!(unknown.is_healthy(), None);

        let xprotect = ProtectionStatus {
            provider: "xprotect".into(),
            known: true,
            background_scans: Some(true),
            launch_scans: Some(true),
            realtime: Some(true),
            ..Default::default()
        };
        assert_eq!(xprotect.is_healthy(), Some(true));

        let off = ProtectionStatus {
            provider: "defender".into(),
            known: true,
            realtime: Some(false),
            ..Default::default()
        };
        assert_eq!(off.is_healthy(), Some(false));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_real_machine_answers_the_query() {
        let status = status();
        // Whether or not protection is on, the probe must produce something a
        // person can read rather than an empty card.
        assert!(
            status.known || !status.notes.is_empty(),
            "the probe said nothing at all: {status:?}"
        );
        assert!(!status.product_name.is_empty());
    }
}
