//! Windows adapter (planned). The shape of the implementation is here so the
//! engines already work: apps come from the uninstall registry hive via
//! PowerShell, leftovers from the well known app-data folders.

use crate::{AppSource, LeftoverRoot, LifecycleReport, Naming, Platform, PlatformId};
use squid_core::progress::CancelToken;
use squid_core::{AppInfo, Bytes, Category, Result, Volume};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Default)]
pub struct Windows;

fn ps(script: &str) -> Option<String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        None
    }
}

/// `HKLM\...\Uninstall` + `HKCU\...\Uninstall` + the WOW64 view.
const HARVEST_SCRIPT: &str = r#"
$paths = @(
 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
)
Get-ItemProperty $paths -ErrorAction SilentlyContinue |
  Where-Object { $_.DisplayName -and -not $_.SystemComponent } |
  ForEach-Object { '{0}|{1}|{2}|{3}' -f $_.DisplayName, $_.DisplayVersion, $_.InstallLocation, $_.UninstallString }
"#;

impl Platform for Windows {
    fn id(&self) -> PlatformId {
        PlatformId::Windows
    }
    fn display_name(&self) -> String {
        "Windows".into()
    }
    fn os_version(&self) -> String {
        ps("(Get-CimInstance Win32_OperatingSystem).Caption")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Windows".into())
    }
    fn hardware_model(&self) -> String {
        ps("(Get-CimInstance Win32_ComputerSystem).Model")
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }
    fn app_sources(&self) -> Vec<AppSource> {
        ["C:\\Program Files", "C:\\Program Files (x86)"]
            .iter()
            .map(|dir| AppSource {
                dir: PathBuf::from(dir),
                label: "Program Files".into(),
                depth: 1,
                system: true,
            })
            .filter(|s| s.dir.is_dir())
            .collect()
    }
    fn list_apps(&self, cancel: &CancelToken, _size: bool) -> Result<Vec<AppInfo>> {
        let mut apps = Vec::new();
        let Some(text) = ps(HARVEST_SCRIPT) else {
            return Ok(apps);
        };
        for line in text.lines() {
            if cancel.is_cancelled() {
                break;
            }
            let cols: Vec<&str> = line.split('|').collect();
            let Some(name) = cols.first().map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                continue;
            };
            apps.push(AppInfo {
                name: name.to_string(),
                bundle_id: None,
                path: cols
                    .get(2)
                    .map(|p| PathBuf::from(p.trim()))
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_default(),
                version: cols
                    .get(1)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                size: Bytes::ZERO,
                source: "Registry".into(),
                running: false,
                has_sibling_install: false,
                publisher: None,
                installed_ms: None,
            });
        }
        Ok(apps)
    }
    fn leftover_roots(&self) -> Vec<LeftoverRoot> {
        ["APPDATA", "LOCALAPPDATA", "ProgramData"]
            .iter()
            .filter_map(|key| std::env::var_os(key).map(PathBuf::from))
            .map(|dir| LeftoverRoot {
                dir,
                naming: Naming::AppName,
                deep: false,
                category: Category::Leftovers,
            })
            .filter(|c| c.dir.is_dir())
            .collect()
    }
    fn prepare_uninstall(&self, app: &AppInfo) -> LifecycleReport {
        let mut report = LifecycleReport::default();
        report.warnings.push(format!(
            "{} ships its own uninstaller; squid only removes leftover data on Windows",
            app.name
        ));
        report
    }
    fn finish_uninstall(&self, _app: &AppInfo, _removed: &[PathBuf]) -> LifecycleReport {
        LifecycleReport::default()
    }
    fn volumes(&self) -> Result<Vec<Volume>> {
        let mut out = Vec::new();
        let Some(text) = ps(
            "Get-PSDrive -PSProvider FileSystem | ForEach-Object { '{0}|{1}|{2}' -f $_.Root, $_.Used, $_.Free }",
        ) else {
            return Ok(out);
        };
        for line in text.lines() {
            let cols: Vec<&str> = line.split('|').collect();
            let (Some(root), Some(used), Some(free)) = (cols.first(), cols.get(1), cols.get(2))
            else {
                continue;
            };
            let (Ok(used), Ok(free)) = (used.trim().parse::<u64>(), free.trim().parse::<u64>())
            else {
                continue;
            };
            let mount = Path::new(root.trim());
            out.push(Volume {
                name: root.trim().to_string(),
                mount_point: mount.to_path_buf(),
                total: Bytes(used + free),
                free: Bytes(free),
                is_root: root.trim().eq_ignore_ascii_case("C:\\"),
                is_removable: false,
                filesystem: "ntfs".into(),
            });
        }
        Ok(out)
    }
    fn protected_prefixes(&self) -> Vec<PathBuf> {
        ["C:\\Windows", "C:\\Program Files"]
            .iter()
            .map(PathBuf::from)
            .collect()
    }
    fn running_executables(&self) -> Vec<String> {
        ps("Get-Process | ForEach-Object { $_.Path }")
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }
    fn inspect_path(&self, _path: &Path) -> Option<String> {
        None
    }
    fn reveal_trash(&self) {
        let _ = Command::new("explorer")
            .arg("shell:RecycleBinFolder")
            .spawn();
    }
}
