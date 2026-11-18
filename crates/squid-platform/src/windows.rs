//! Windows adapter. Installed apps come from the uninstall registry hives
//! (HKLM 64-bit + 32-bit views, HKCU), leftovers from the well-known
//! AppData/ProgramData folders, volumes and processes from `sysinfo`.

use crate::{
    AppSource, BinNoun, Device, LeftoverRoot, LifecycleReport, Naming, Platform, PlatformId,
    TrashTarget,
};
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

/// The registry's `ProductName` freezes at "Windows 10 …" on every machine
/// upgraded from 10 (Microsoft documents it as unreliable); Windows 11 keeps
/// reporting API version 10.0 too. The build number is the real signal:
/// 22000 shipped as Windows 11. Editions come from `EditionID`, the feature
/// label from `DisplayVersion`, e.g. `Windows 11 Pro 26H2`.
fn os_label(
    major: Option<u32>,
    build: Option<&str>,
    product: Option<&str>,
    edition: Option<&str>,
    display: Option<&str>,
) -> String {
    let build_no = build.and_then(|b| b.trim().parse::<u32>().ok());
    let Some(family) = (match (major.unwrap_or(10), build_no) {
        (10, Some(b)) if b >= 22000 => Some("Windows 11"),
        (10, Some(_)) => Some("Windows 10"),
        _ => None, // Windows 7/8 or a future major: use the registry's wording.
    }) else {
        let mut out = product.unwrap_or("Windows").to_string();
        if let Some(d) = display {
            out.push(' ');
            out.push_str(d);
        } else if let Some(b) = build {
            out.push_str(&format!(" (build {b})"));
        }
        return out;
    };
    let mut out = family.to_string();
    if let Some(edition) = edition_label(edition) {
        out.push(' ');
        out.push_str(edition);
    }
    match (display, build) {
        (Some(d), _) => {
            out.push(' ');
            out.push_str(d);
        }
        (None, Some(b)) => out.push_str(&format!(" (build {b})")),
        (None, None) => {}
    }
    out
}

/// `EditionID` → marketing edition, mirroring the names Windows itself uses.
fn edition_label(edition: Option<&str>) -> Option<&'static str> {
    Some(match edition? {
        "Professional" | "ProfessionalN" => "Pro",
        "ProfessionalEducation" | "ProfessionalEducationN" => "Pro Education",
        "ProfessionalWorkstation" | "ProfessionalWorkstationN" => "Pro for Workstations",
        "Education" | "EducationN" => "Education",
        "Enterprise" | "EnterpriseN" | "EnterpriseS" | "EnterpriseEvaluation" => "Enterprise",
        "Core" | "CoreN" | "CoreCountrySpecific" | "CoreSingleLanguage" => "Home",
        "ServerRdsh" => "Enterprise Multi-Session",
        "ServerDatacenter" | "ServerDatacenterACor" => "Server Datacenter",
        "ServerStandard" | "ServerStandardACor" => "Server Standard",
        _ => return None,
    })
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
        let Some(key) = key else {
            return "Windows".into();
        };
        os_label(
            dword_value(&key, "CurrentMajorVersionNumber"),
            str_value(&key, "CurrentBuildNumber").as_deref(),
            str_value(&key, "ProductName").as_deref(),
            str_value(&key, "EditionID").as_deref(),
            str_value(&key, "DisplayVersion").as_deref(),
        )
    }
    fn hardware_model(&self) -> String {
        let bios = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(BIOS_KEY, KEY_READ)
            .ok();
        match bios {
            Some(key) => {
                let vendor = str_value(&key, "SystemManufacturer").filter(|v| !is_placeholder(v));
                let model = str_value(&key, "SystemProductName").filter(|v| !is_placeholder(v));
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
    fn list_apps(&self, cancel: &CancelToken, size: bool) -> Result<Vec<AppInfo>> {
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
                // Size: Windows installers report EstimatedSize (KB) for
                // Add/Remove Programs — the same number Settings shows.
                // Absent that, measure the install folder when asked.
                let size = match dword_value(&key, "EstimatedSize") {
                    Some(kb) => Bytes(u64::from(kb) * 1024),
                    None if size => install_location
                        .as_ref()
                        .map(|loc| {
                            squid_core::fs::measure(loc, squid_core::ItemKind::Path).allocated
                        })
                        .unwrap_or_default(),
                    None => Bytes::ZERO,
                };
                apps.push(AppInfo {
                    name: display_name,
                    bundle_id: None,
                    path,
                    version: str_value(&key, "DisplayVersion"),
                    size,
                    source: source.into(),
                    running: running_app,
                    has_sibling_install: false,
                    publisher: str_value(&key, "Publisher"),
                    installed_ms: install_date_ms(str_value(&key, "InstallDate")),
                    display_icon: str_value(&key, "DisplayIcon"),
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

    fn device(&self) -> Device {
        Device::Pc
    }

    fn bin_noun(&self) -> BinNoun {
        BinNoun::RecycleBin
    }

    fn trash_target(&self) -> TrashTarget {
        // Every interactive Windows desktop has a Recycle Bin for its drive;
        // squid writes it through `IFileOperation` rather than a folder, so
        // there is no path to report. `SHEmptyRecycleBinW` failing is the only
        // signal that the shell refused, and that is a per-deletion error.
        TrashTarget::RecycleBin
    }
}

// ---------------------------------------------------------------------------
// Static component facts for the overview spec panel: the BIOS HAL key, raw
// SMBIOS via `GetSystemFirmwareTable`, the storage stack through
// `IOCTL_STORAGE_QUERY_PROPERTY` and the display class registry. Native calls
// only — no spawned `wmic`/PowerShell helpers, which are deprecated outright.

pub(crate) fn hardware_info() -> crate::HardwareInfo {
    let mut system = System::new();
    system.refresh_cpu_list(sysinfo::CpuRefreshKind::everything());
    let mut info = crate::HardwareInfo {
        board: baseboard_name(),
        cpu_model: system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().trim().to_string())
            .unwrap_or_default(),
        ..Default::default()
    };
    info.memory_sticks = smbios_memory();
    if let Some((vendor, model, bus, size)) = system_disk_identity() {
        info.disk_vendor = vendor;
        info.disk_model = model;
        info.disk_bus = bus;
        info.disk_size_bytes = size;
    }
    if let Some((vendor, model, vram)) = best_gpu() {
        info.gpu_vendor = vendor;
        info.gpu_model = model;
        info.gpu_vram_bytes = vram;
    }
    info
}

/// Placeholder strings OEM firmware leaves behind instead of real names.
fn is_placeholder(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.is_empty()
        || lower.contains("default string")
        || lower.contains("system product name")
        || lower.contains("system manufacturer")
        || lower.contains("base board")
        || lower.contains("to be filled by o.e.m.")
}

fn baseboard_name() -> String {
    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(BIOS_KEY, KEY_READ)
    else {
        return String::new();
    };
    [
        str_value(&key, "BaseBoardProduct"),
        str_value(&key, "SystemProductName"),
    ]
    .into_iter()
    .flatten()
    .find(|text| !is_placeholder(text))
    .unwrap_or_default()
}

/// Type-17 (Memory Device) records from the raw SMBIOS blob: brand, size and
/// running speed per stick. `RSMB` lives in the SystemInformation provider.
fn smbios_memory() -> Vec<crate::MemoryStick> {
    use windows_sys::Win32::System::SystemInformation::GetSystemFirmwareTable;
    const RSMB: u32 = u32::from_le_bytes(*b"RSMB");
    // SAFETY: a null buffer with size 0 asks for the required size.
    let needed = unsafe { GetSystemFirmwareTable(RSMB, 0, std::ptr::null_mut(), 0) };
    if !(8..=4_000_000).contains(&needed) {
        return Vec::new();
    }
    let mut table = vec![0u8; needed as usize];
    // SAFETY: buffer sized exactly as requested by the first call.
    let written = unsafe { GetSystemFirmwareTable(RSMB, 0, table.as_mut_ptr(), needed) };
    if written < 8 {
        return Vec::new();
    }
    let declared = u32::from_le_bytes(table[4..8].try_into().unwrap_or([0; 4])) as usize;
    let end = (written as usize - 8)
        .min(declared.max(1))
        .min(table.len() - 8);
    let data = &table[8..8 + end];
    let mut sticks = Vec::new();
    let mut at = 0usize;
    while at + 4 <= data.len() {
        let kind = data[at];
        let formatted = usize::from(u16::from_le_bytes([data[at + 2], data[at + 3]]));
        if formatted < 5 || at + formatted > data.len() {
            break;
        }
        let strings_start = at + formatted;
        let strings_end = data[strings_start..]
            .windows(2)
            .position(|pair| pair == [0, 0])
            .map_or(data.len(), |offset| strings_start + offset + 2);
        if kind == 17 && formatted >= 0x1D {
            // DMTF DSP0134 layout: size @0x0E (KB, 0x7FFF = see extended
            // size @0x1F), speed @0x17, manufacturer string @0x19, and from
            // v3.2 the running clock @0x1D which beats the rated one.
            let raw = u16::from_le_bytes([data[at + 0x0E], data[at + 0x0F]]);
            let size_bytes = if raw == 0xFFFF {
                0 // slot empty
            } else if raw == 0x7FFF && formatted >= 0x27 {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&data[at + 0x1F..at + 0x27]);
                u64::from_le_bytes(bytes)
            } else if raw & 0x8000 != 0 {
                u64::from(raw & 0x7FFF) * 128 // kilobits per spec
            } else {
                u64::from(raw) * 1024
            };
            let rated = u16::from_le_bytes([data[at + 0x17], data[at + 0x18]]) as u32;
            let configured = if formatted >= 0x1F {
                u16::from_le_bytes([data[at + 0x1D], data[at + 0x1E]]) as u32
            } else {
                0
            };
            let manufacturer = smbios_string(
                &data[strings_start..strings_end],
                usize::from(data[at + 0x19]),
            );
            let speed = configured.max(rated);
            if size_bytes > 0 || speed > 0 || !manufacturer.is_empty() {
                sticks.push(crate::MemoryStick {
                    manufacturer: if is_placeholder(&manufacturer) {
                        String::new()
                    } else {
                        manufacturer
                    },
                    size_bytes,
                    speed_mhz: if speed > 100 { speed } else { 0 },
                });
            }
        }
        at = strings_end;
    }
    sticks
}

fn smbios_string(area: &[u8], index: usize) -> String {
    if index == 0 {
        return String::new();
    }
    let mut start = 0usize;
    for _ in 1..index {
        let Some(rel) = area[start..].iter().position(|byte| *byte == 0) else {
            return String::new();
        };
        start += rel + 1;
        if start > area.len() {
            return String::new();
        }
    }
    let end = area[start..]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(area.len() - start);
    String::from_utf8_lossy(&area[start..start + end])
        .trim()
        .to_string()
}

/// Vendor + model + bus of the disk behind the system volume. The query is
/// a buffered IOCTL that needs no data access, so the handle is opened with
/// `dwDesiredAccess = 0` — enough even for standard users (GENERIC_READ on
/// `\\.\C:` or the physical drive is elevation-gated, measured on this VDI).
fn system_disk_identity() -> Option<(String, String, String, u64)> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::CreateFileW;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
    #[repr(C)]
    struct StoragePropertyQuery {
        property_id: u32,
        query_type: u32,
        raw: [u8; 4],
    }
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let targets = [
        format!("\\\\.\\{}\\", drive.trim_end_matches('\\')),
        r"\\.\PhysicalDrive0".to_string(),
    ];
    for target in targets {
        let path: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated path, query-only open, existing object.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                0x0000_0003,
                std::ptr::null_mut(),
                3,
                0x0000_0080,
                std::ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            continue;
        }
        let query = StoragePropertyQuery {
            property_id: 0,
            query_type: 0,
            raw: [0; 4],
        };
        let mut out = vec![0u8; 2048];
        let mut returned = 0u32;
        // SAFETY: query struct and out buffer outlive the call; handle valid.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                &query as *const _ as *const c_void,
                std::mem::size_of::<StoragePropertyQuery>() as u32,
                out.as_mut_ptr() as *mut c_void,
                out.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || returned < 40 {
            // SAFETY: paired with CreateFileW above.
            unsafe { CloseHandle(handle) };
            continue;
        }
        let vendor = descriptor_string(&out, returned as usize, 12);
        let model = descriptor_string(&out, returned as usize, 16);
        if vendor.is_empty() && model.is_empty() {
            // SAFETY: paired with CreateFileW above.
            unsafe { CloseHandle(handle) };
            continue;
        }
        let bus = bus_label(u32::from_le_bytes(out[28..32].try_into().unwrap_or([0; 4])));
        // Whole-disk size: the volumes only show partitions, the overview
        // wants the physical capacity. `IOCTL_DISK_GET_LENGTH_INFO` demands
        // FILE_READ_ACCESS, which a query-only handle cannot service, so
        // reopen with GENERIC_READ — standard users get denied on physical
        // drives, and then the capacity simply stays unknown.
        let disk_path: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated path, query open of an existing object.
        let disk_handle = unsafe {
            CreateFileW(
                disk_path.as_ptr(),
                0x8000_0000,
                0x0000_0003,
                std::ptr::null_mut(),
                3,
                0x0000_0080,
                std::ptr::null_mut(),
            )
        };
        let mut size_bytes = 0u64;
        if !disk_handle.is_null() && disk_handle != INVALID_HANDLE_VALUE {
            let mut size_written = 0u32;
            // SAFETY: 8-byte out buffer, valid handle; paired CloseHandle.
            let sized = unsafe {
                DeviceIoControl(
                    disk_handle,
                    0x0007_405C, // IOCTL_DISK_GET_LENGTH_INFO
                    std::ptr::null(),
                    0,
                    &mut size_bytes as *mut _ as *mut std::ffi::c_void,
                    8,
                    &mut size_written,
                    std::ptr::null_mut(),
                )
            };
            unsafe { CloseHandle(disk_handle) };
            if sized == 0 || size_written < 8 {
                size_bytes = 0;
            }
        }
        // SAFETY: paired with CreateFileW above.
        let _ = unsafe { CloseHandle(handle) };
        return Some((vendor, model, bus, size_bytes));
    }
    None
}

/// ANSI string inside STORAGE_DEVICE_DESCRIPTOR at the u32 offset stored in
/// `offset_field` (12 = VendorId, 16 = ProductId).
fn descriptor_string(buf: &[u8], used: usize, offset_field: usize) -> String {
    let Ok(bytes) = buf[offset_field..offset_field + 4].try_into() else {
        return String::new();
    };
    let offset = u32::from_le_bytes(bytes) as usize;
    if offset == 0 || offset >= used || used > buf.len() {
        return String::new();
    }
    let text = &buf[offset..used];
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len());
    String::from_utf8_lossy(&text[..end]).trim().to_string()
}

fn bus_label(bus: u32) -> String {
    match bus {
        0x01 => "SCSI",
        0x02 | 0x03 => "ATA",
        0x07 => "USB",
        0x08 => "RAID",
        0x09 => "iSCSI",
        0x0A => "SAS",
        0x0B => "SATA",
        0x11 | 0x12 => "NVMe",
        _ => return String::new(),
    }
    .to_string()
}

/// The GPU with the most video memory (a discrete card when one exists),
/// read from the display class registry keys every driver writes.
fn best_gpu() -> Option<(String, String, u64)> {
    let class = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}",
            KEY_READ,
        )
        .ok()?;
    let mut best: Option<(u64, String, String)> = None;
    for name in class.enum_keys().filter_map(|entry| entry.ok()) {
        if name.is_empty() || !name.chars().all(|digit| digit.is_ascii_digit()) {
            continue;
        }
        let Ok(sub) = class.open_subkey_with_flags(&name, KEY_READ) else {
            continue;
        };
        let Some(desc) = str_value(&sub, "DriverDesc") else {
            continue;
        };
        let lower = desc.to_lowercase();
        if lower.contains("basic render") || lower.contains("rdp driver") {
            continue;
        }
        let vram = sub
            .get_value::<u64, _>("HardwareInformation.qwMemorySize")
            .unwrap_or(0);
        // Garbage/negative QWORDs show up as huge unsigned values.
        let vram = if vram < 64 * 1024 * 1024 * 1024 {
            vram
        } else {
            0
        };
        if best.as_ref().is_some_and(|(known, _, _)| *known >= vram) {
            continue;
        }
        best = Some((
            vram,
            desc,
            str_value(&sub, "ProviderName").unwrap_or_default(),
        ));
    }
    let (vram, desc, provider) = best?;
    Some((gpu_vendor(&provider, &desc).to_string(), desc, vram))
}

fn gpu_vendor(provider: &str, desc: &str) -> &'static str {
    let text = format!("{provider} {desc}").to_lowercase();
    if text.contains("nvidia")
        || text.contains("geforce")
        || text.contains("quadro")
        || text.contains("rtx")
        || text.contains("gtx")
    {
        "NVIDIA"
    } else if text.contains("advanced micro devices")
        || text.contains("radeon")
        || text.contains(" amd")
    {
        "AMD"
    } else if text.contains("intel") || text.contains("uhd graphics") || text.contains("iris") {
        "Intel"
    } else if text.contains("aspeed") {
        "ASPEED"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::os_label;

    #[test]
    fn hardware_probe_runs_on_this_machine() {
        // Local smoke: prints what the component query found here.
        let info = super::hardware_info();
        println!(
            "board={:?} cpu={:?} sticks={} disk={:?}/{:?}/{:?}/{} gpu={:?}/{:?} vram={}",
            info.board,
            info.cpu_model,
            info.memory_sticks.len(),
            info.disk_vendor,
            info.disk_model,
            info.disk_bus,
            info.disk_size_bytes,
            info.gpu_vendor,
            info.gpu_model,
            info.gpu_vram_bytes
        );
        assert!(!info.cpu_model.is_empty() || info.board.is_empty());
    }

    #[test]
    fn upgraded_windows_10_builds_report_eleven() {
        // The exact shape seen on 26H2 machines: ProductName still says 10.
        assert_eq!(
            os_label(
                Some(10),
                Some("26300"),
                Some("Windows 10 Pro"),
                Some("Professional"),
                Some("26H2"),
            ),
            "Windows 11 Pro 26H2"
        );
        assert_eq!(
            os_label(Some(10), Some("22000"), Some("Windows 10 Pro"), None, None),
            "Windows 11 (build 22000)"
        );
    }

    #[test]
    fn real_windows_10_builds_stay_ten() {
        assert_eq!(
            os_label(
                Some(10),
                Some("19045"),
                Some("Windows 10 Pro"),
                Some("Professional"),
                Some("22H2"),
            ),
            "Windows 10 Pro 22H2"
        );
    }

    #[test]
    fn server_and_home_editions_map_to_marketing_names() {
        assert_eq!(
            os_label(
                Some(10),
                Some("25398"),
                None,
                Some("ServerDatacenterACor"),
                None
            ),
            "Windows 11 Server Datacenter (build 25398)"
        );
        assert_eq!(
            os_label(
                Some(10),
                Some("26100"),
                None,
                Some("CoreSingleLanguage"),
                Some("24H2")
            ),
            "Windows 11 Home 24H2"
        );
    }

    #[test]
    fn legacy_and_future_versions_keep_the_registry_wording() {
        assert_eq!(
            os_label(
                Some(6),
                Some("7601"),
                Some("Windows 7 Ultimate"),
                None,
                None
            ),
            "Windows 7 Ultimate (build 7601)"
        );
        assert_eq!(
            os_label(
                Some(11),
                Some("30000"),
                Some("Windows Next"),
                None,
                Some("1H3")
            ),
            "Windows Next 1H3"
        );
    }
}
