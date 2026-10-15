//! Windows adapter. Installed apps come from the uninstall registry hives
//! (HKLM 64-bit + 32-bit views, HKCU), leftovers from the well-known
//! AppData/ProgramData folders, volumes and processes from `sysinfo`.

use crate::{AppSource, LeftoverRoot, LifecycleReport, Naming, Platform, PlatformId};
use squid_core::progress::CancelToken;
use squid_core::{AppInfo, Bytes, Category, Result, Volume};
use std::path::{Path, PathBuf};
use std::process::Command;
use sysinfo::{Disks, System};
use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
};
use winreg::RegKey;

#[derive(Default)]
pub struct Windows;

const UNINSTALL_SUBKEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const NT_VERSION_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const BIOS_KEY: &str = r"HARDWARE\DESCRIPTION\System\BIOS";

fn open_uninstall_hives() -> Vec<(RegKey, &'static str)> {
    let mut hives = Vec::new();
    let views = [
        (KEY_WOW64_64KEY, "Registry (HKLM 64-bit)"),
        (KEY_WOW64_32KEY, "Registry (HKLM 32-bit)"),
    ];
    for (flag, label) in views {
        if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(UNINSTALL_SUBKEY, KEY_READ | flag)
        {
            hives.push((key, label));
        }
    }
    if let Ok(key) =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(UNINSTALL_SUBKEY, KEY_READ)
    {
        hives.push((key, "Registry (HKCU)"));
    }
    hives
}

fn str_value(key: &RegKey, name: &str) -> Option<String> {
    key.get_value::<String, _>(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn dword_value(key: &RegKey, name: &str) -> Option<u32> {
    key.get_value::<u32, _>(name).ok()
}

/// `2024-08-13`-ish epoch millis from the registry's `YYYYMMDD` string.
fn install_date_ms(raw: Option<String>) -> Option<u64> {
    let raw = raw?.replace('-', "");
    let year: i32 = raw.get(0..4)?.parse().ok()?;
    let month: u32 = raw.get(4..6)?.parse().ok()?;
    let day: u32 = raw.get(6..8)?.parse().ok()?;
    let days = days_since_epoch(year, month, day)?;
    Some(days.saturating_mul(86_400_000))
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_since_epoch(y: i32, m: u32, d: u32) -> Option<u64> {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u64;
    let mp = (m + 9) % 12;
    let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146097 + doe as i32 - 719468) as u64)
}

/// Strip surrounding quotes and `cmd /c` wrappers from an uninstall string.
fn uninstall_program(uninstall_string: &str) -> Option<PathBuf> {
    let s = uninstall_string.trim();
    let path_part = if s.starts_with('"') {
        s.strip_prefix('"')?.split('"').next()?
    } else {
        // msiexec style: `MsiExec.exe /X{GUID}` — take the first token only if
        // it looks like a file; `msiexec` alone resolves via PATH anyway.
        s.split_whitespace().next()?
    };
    let candidate = PathBuf::from(path_part);
    candidate.exists().then_some(candidate)
}

impl Platform for Windows {
    fn id(&self) -> PlatformId {
        PlatformId::Windows
    }
    fn display_name(&self) -> String {
        "Windows".into()
    }
    fn os_version(&self) -> String {
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(NT_VERSION_KEY, KEY_READ)
            .ok();
        match key {
            Some(key) => {
                let product = str_value(&key, "ProductName").unwrap_or_else(|| "Windows".into());
                let display = str_value(&key, "DisplayVersion");
                let build = str_value(&key, "CurrentBuildNumber");
                let mut out = product;
                if let Some(d) = display {
                    out.push(' ');
                    out.push_str(&d);
                } else if let Some(b) = build {
                    out.push_str(" (build ");
                    out.push_str(&b);
                    out.push(')');
                }
                out
            }
            None => "Windows".into(),
        }
    }
    fn hardware_model(&self) -> String {
        let bios = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(BIOS_KEY, KEY_READ)
            .ok();
        match bios {
            Some(key) => {
                let vendor = str_value(&key, "SystemManufacturer");
                let model = str_value(&key, "SystemProductName");
                match (vendor, model) {
                    (Some(v), Some(m)) if v != m => format!("{v} {m}"),
                    (_, Some(m)) => m,
                    (Some(v), _) => v,
                    (None, None) => String::new(),
                }
            }
            None => String::new(),
        }
    }
    fn app_sources(&self) -> Vec<AppSource> {
        let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
        let mut dirs: Vec<PathBuf> = vec![
            env("ProgramFiles").unwrap_or_else(|| PathBuf::from(r"C:\Program Files")),
            env("ProgramFiles(x86)").unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)")),
        ];
        dirs.dedup();
        dirs.into_iter()
            .filter(|dir| dir.is_dir())
            .map(|dir| AppSource {
                dir,
                label: "Program Files".into(),
                depth: 1,
                system: true,
            })
            .collect()
    }
    fn list_apps(&self, cancel: &CancelToken, _size: bool) -> Result<Vec<AppInfo>> {
        let running: Vec<String> = self
            .running_executables()
            .into_iter()
            .map(|p| p.to_lowercase())
            .collect();
        let mut apps = Vec::new();
        for (hive, source) in open_uninstall_hives() {
            let names = hive
                .enum_keys()
                .filter_map(|name| name.ok())
                .collect::<Vec<_>>();
            for name in names {
                if cancel.is_cancelled() {
                    return Ok(apps);
                }
                let Ok(key) = hive.open_subkey(&name) else {
                    continue;
                };
                let Some(display_name) = str_value(&key, "DisplayName") else {
                    continue;
                };
                if dword_value(&key, "SystemComponent") == Some(1) {
                    continue;
                }
                // Servicing entries (patches, update rollups) are not apps.
                if matches!(
                    str_value(&key, "ReleaseType").as_deref(),
                    Some("Security Update") | Some("Update Rollup") | Some("Hotfix")
                ) {
                    continue;
                }
                let install_location = str_value(&key, "InstallLocation")
                    .map(|l| l.trim_matches('"').to_string())
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir());
                let uninstall_string = str_value(&key, "UninstallString");
                let path = install_location
                    .clone()
                    .or_else(|| uninstall_string.as_deref().and_then(uninstall_program))
                    .unwrap_or_default();
                let running_app = install_location
                    .as_ref()
                    .map(|loc| {
                        let loc = loc.to_string_lossy().to_lowercase();
                        running.iter().any(|exe| exe.starts_with(&loc))
                    })
                    .unwrap_or(false);
                apps.push(AppInfo {
                    name: display_name,
                    bundle_id: None,
                    path,
                    version: str_value(&key, "DisplayVersion"),
                    size: Bytes::ZERO,
                    source: source.into(),
                    running: running_app,
                    has_sibling_install: false,
                    publisher: str_value(&key, "Publisher"),
                    installed_ms: install_date_ms(str_value(&key, "InstallDate")),
                });
            }
        }
        apps.sort_by_key(|app| app.name.to_lowercase());
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
        let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let mut out = Vec::new();
        let disks = Disks::new_with_refreshed_list();
        for disk in &disks {
            let mount = disk.mount_point().to_path_buf();
            let mount_name = mount.to_string_lossy().trim_end_matches('\\').to_string();
            out.push(Volume {
                name: mount_name.clone(),
                mount_point: mount,
                total: Bytes(disk.total_space()),
                free: Bytes(disk.available_space()),
                is_root: mount_name.eq_ignore_ascii_case(&system_drive),
                is_removable: disk.is_removable(),
                filesystem: disk.file_system().to_string_lossy().into_owned(),
            });
        }
        Ok(out)
    }
    fn protected_prefixes(&self) -> Vec<PathBuf> {
        let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
        let mut prefixes: Vec<PathBuf> = [
            env("SystemRoot"),
            env("windir"),
            env("ProgramFiles"),
            env("ProgramFiles(x86)"),
        ]
        .into_iter()
        .flatten()
        .chain(std::iter::once(PathBuf::from(r"C:\Windows")))
        .collect();
        prefixes.sort();
        prefixes.dedup();
        prefixes
    }
    fn running_executables(&self) -> Vec<String> {
        let mut system = System::new_with_specifics(
            sysinfo::RefreshKind::nothing()
                .with_processes(sysinfo::ProcessRefreshKind::everything()),
        );
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        system
            .processes()
            .values()
            .filter_map(|process| process.exe())
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
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
