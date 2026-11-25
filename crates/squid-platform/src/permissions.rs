//! Permission-management data sources: startup entries, shell context-menu
//! handlers and top-level windows that a popup blocker can see and close.
//!
//! Unlike [`crate::Platform`], these are free functions rather than trait
//! methods: they belong to the desktop shell story (Tauri commands), not to
//! the clean/uninstall engines, and the CLI/TUI never call them. macOS and
//! Linux get useful startup listings; shell extensions and popup detection
//! are Windows-only for now and return empty lists elsewhere.

use serde::{Deserialize, Serialize};

/// One startup row. `id` encodes the exact source so [`set_startup_enabled`]
/// can toggle the same object Task Manager / 电脑管家 would.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupEntry {
    pub id: String,
    pub name: String,
    /// Human-facing software name resolved from the binary's version info
    /// (ProductName / FileDescription, preferring CJK — the 电脑管家 habit).
    /// Falls back to `name` when the PE carries nothing friendlier.
    pub display_name: String,
    pub command: String,
    /// `registry` | `startupFolder` | `scheduledTask` | `launchAgent` | `autostart` | `service`
    pub source: String,
    /// Human-readable source, e.g. `HKLM\...\CurrentVersion\Run`.
    pub source_label: String,
    /// `currentUser` | `machine`
    pub scope: String,
    pub enabled: bool,
    /// True once the entry was attributed to a real software product (the
    /// binary carries a human FileDescription/ProductName). 电脑管家 groups
    /// every entry it *cannot* attribute under 其他软件; this flag drives
    /// that bucket on the UI side.
    #[serde(default)]
    pub named: bool,
    /// PNG bytes of the binary's default icon, extracted on Windows only —
    /// the 电脑管家 habit of showing the software icon next to its name.
    #[serde(default)]
    pub icon: Option<Vec<u8>>,
    /// The executable this entry really runs (lnk targets resolved). Empty
    /// when the command holds no path at all.
    #[serde(default)]
    pub target: String,
    /// False when `target` does not exist on disk — the program was removed
    /// but its autostart config survived: a 残留项 (leftover), removable via
    /// [`remove_startup_entry`].
    #[serde(default)]
    pub target_exists: bool,
    /// When this fires: `boot` | `logon` | `background` | `unknown`, from the
    /// registry value / task triggers / service start type. Shown as a chip.
    #[serde(default)]
    pub trigger: String,
    /// Microsoft-signed or living under %SystemRoot% — the UI folds these
    /// into a 系统组件 group that 「显示系统项」 can hide.
    #[serde(default)]
    pub system_item: bool,
    /// Digital-signature verdict of `target`: `microsoft` | `vendor` |
    /// `unsigned` | `invalid` | `unknown` (nothing resolvable / non-Windows).
    /// Filled by the batch signature pass at the end of the scan.
    #[serde(default)]
    pub signature: String,
    /// Subject of the signing certificate when known.
    #[serde(default)]
    pub signer: Option<String>,
}

/// One registered context-menu ("右键菜单") handler.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellExtEntry {
    /// `shlext|<hive>|<point>|<name>` — doubles as the restore recipe while
    /// the handler sits in the app's own backup list.
    pub id: String,
    pub name: String,
    /// Registered payload: usually a `{CLSID}`, sometimes a legacy verb.
    pub handler: String,
    /// Menu surface, aligned with 电脑管家: 文件 / 文件夹 / 桌面 / 驱动器.
    pub extension_point: String,
    pub scope: String,
    /// Best human-facing name: the module's FileDescription (what 电脑管家
    /// shows), else the CLSID friendly name, else the raw key name.
    pub friendly_name: Option<String>,
    /// Implementation dll/exe behind the CLSID, when resolvable.
    pub module: Option<String>,
    /// Publisher (CompanyName) of the owning module — 电脑管家's 创建者 column.
    pub publisher: Option<String>,
    /// PNG bytes of the owning module's icon (Windows only).
    #[serde(default)]
    pub icon: Option<Vec<u8>>,
    /// True once the handler was attributed to a real software product (its
    /// module carries usable FileDescription/ProductName strings). Drives the
    /// same software-grouping rule the startup list uses; unattributed rows
    /// land in 其他软件.
    #[serde(default)]
    pub named: bool,
}

/// A visible top-level window the popup blocker could act on.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupWindow {
    pub hwnd: i64,
    pub title: String,
    pub process: String,
    pub exe: Option<String>,
    /// PNG bytes of the owning process's icon (Windows only).
    #[serde(default)]
    pub icon: Option<Vec<u8>>,
}

pub fn scan_startup_entries() -> Vec<StartupEntry> {
    #[cfg(windows)]
    {
        windows::scan_startup()
    }
    #[cfg(target_os = "macos")]
    {
        macos::scan_startup()
    }
    #[cfg(target_os = "linux")]
    {
        linux::scan_startup()
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

/// PNG bytes of an installed application's icon — Windows extracts it from
/// `DisplayIcon`/the install folder; other platforms return None for now.
pub fn app_icon(app: &squid_core::AppInfo) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        windows::app_icon(app)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

/// PNG bytes of any executable's default icon (Windows only) — used for
/// popup rules and context-menu rows that reference a raw path rather than
/// an installed [`squid_core::AppInfo`].
pub fn exe_icon(path: &str) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        windows::exe_icon_png(path.trim_matches('"'), 0)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

pub fn set_startup_enabled(entry: &StartupEntry, enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::set_startup_enabled(entry, enabled)
    }
    #[cfg(target_os = "macos")]
    {
        macos::set_startup_enabled(entry, enabled)
    }
    #[cfg(target_os = "linux")]
    {
        linux::set_startup_enabled(entry, enabled)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (entry, enabled);
        Err("startup management is not supported on this platform".into())
    }
}

/// Delete a leftover autostart configuration (the program it points at is
/// gone). Only the *registration* disappears — registry value, Startup-folder
/// shortcut, task definition — never a program's files. Services are disabled
/// rather than uninstalled because other software may still reference them.
pub fn remove_startup_entry(entry: &StartupEntry) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::remove_startup_entry(entry)
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
        Err("删除启动项仅在 Windows 提供".into())
    }
}

pub fn scan_shell_extensions() -> Vec<ShellExtEntry> {
    #[cfg(windows)]
    {
        windows::scan_shell_extensions()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Unregister a context-menu handler (it disappears from every app's right
/// click). The caller keeps [`ShellExtEntry`] itself as the restore recipe.
pub fn remove_shell_extension(entry: &ShellExtEntry) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::remove_shell_extension(entry)
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
        Err("context menu management is not supported on this platform".into())
    }
}

/// Re-register a handler previously saved by [`remove_shell_extension`].
pub fn restore_shell_extension(entry: &ShellExtEntry) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::restore_shell_extension(entry)
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
        Err("context menu management is not supported on this platform".into())
    }
}

/// Visible top-level windows, each with its owning process icon extracted.
/// Used by the picker UI where every row shows the software icon.
pub fn list_popup_windows() -> Vec<PopupWindow> {
    #[cfg(windows)]
    {
        windows::list_popup_windows(true)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Same enumeration without icon extraction — the resident popup watcher only
/// matches on exe/title, so skipping the per-window GDI work keeps each tick
/// cheap.
pub fn list_popup_windows_bare() -> Vec<PopupWindow> {
    #[cfg(windows)]
    {
        windows::list_popup_windows(false)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Ask one window to close (`WM_CLOSE`, the same message the ✕ button posts).
pub fn close_popup_window(hwnd: i64) -> bool {
    #[cfg(windows)]
    {
        windows::close_window(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

#[cfg(windows)]
mod windows {
    use super::{PopupWindow, ShellExtEntry, StartupEntry};
    use std::path::{Path, PathBuf};
    use winreg::enums::{
        RegType, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE,
    };
    use winreg::RegKey;

    const CURRENT_VERSION: &str = r"Software\Microsoft\Windows\CurrentVersion";
    const WOW_CURRENT_VERSION: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion";
    const SERVICES_KEY: &str = r"SYSTEM\CurrentControlSet\Services";

    fn registry_error(error: std::io::Error) -> String {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            "需要以管理员身份运行 Squid 才能修改这个位置".to_string()
        } else {
            format!("{error}")
        }
    }

    /// Task Manager's own enable/disable mechanism: a 12-byte marker under
    /// `Explorer\StartupApproved\…`; byte 0 = 2 means enabled, 3 = disabled.
    fn approved_marker(enabled: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; 12];
        bytes[0] = if enabled { 2 } else { 3 };
        bytes
    }

    fn approved_key(hive: &RegKey, base: &str, list: &str) -> Option<RegKey> {
        hive.open_subkey_with_flags(format!(r"{base}\Explorer\StartupApproved\{list}"), KEY_READ)
            .ok()
    }

    fn approved_enabled(hive: &RegKey, base: &str, list: &str, value_name: &str) -> bool {
        approved_key(hive, base, list)
            .and_then(|key| key.get_raw_value(value_name).ok())
            .map_or(true, |value| {
                !value.bytes.first().is_some_and(|head| *head == 3)
            })
    }

    fn write_approved(
        machine: bool,
        base: &str,
        list: &str,
        value_name: &str,
        enabled: bool,
    ) -> std::io::Result<()> {
        let hive = RegKey::predef(if machine {
            HKEY_LOCAL_MACHINE
        } else {
            HKEY_CURRENT_USER
        });
        // Minimal rights only: several security suites (and locked-down
        // profiles) deny KEY_ALL_ACCESS-style opens on the startup trees even
        // for the key's own owner, which used to make every toggle fail.
        let (key, _) = hive.create_subkey_with_flags(
            format!(r"{base}\Explorer\StartupApproved\{list}"),
            KEY_READ | KEY_WRITE,
        )?;
        let marker = winreg::RegValue {
            bytes: approved_marker(enabled),
            vtype: RegType::REG_BINARY,
        };
        key.set_raw_value(value_name, &marker)
    }

    /// Task Manager's marker rendered for `reg.exe` (12 bytes, hex).
    fn approved_marker_hex(enabled: bool) -> String {
        let mut hex = String::from(if enabled { "02" } else { "03" });
        hex.push_str(&"0".repeat(22));
        hex
    }

    /// Flip an approval marker, escalating through a one-shot UAC prompt when
    /// the direct write is denied (machine-scope keys, protected hives). The
    /// result is verified against the registry itself, so a cancelled prompt
    /// reports failure instead of a silently dead switch.
    fn flip_approved(
        machine: bool,
        base: &str,
        list: &str,
        value_name: &str,
        enabled: bool,
    ) -> Result<(), String> {
        if let Err(why) = write_approved(machine, base, list, value_name, enabled) {
            if why.kind() != std::io::ErrorKind::PermissionDenied {
                return Err(registry_error(why));
            }
            let hive_label = if machine { "HKLM" } else { "HKCU" };
            run_elevated(
                "reg.exe",
                &[
                    "add".to_string(),
                    format!(r"{hive_label}\{base}\Explorer\StartupApproved\{list}"),
                    "/v".to_string(),
                    value_name.to_string(),
                    "/t".to_string(),
                    "REG_BINARY".to_string(),
                    "/d".to_string(),
                    approved_marker_hex(enabled),
                    "/f".to_string(),
                ],
            )?;
            let hive = RegKey::predef(if machine {
                HKEY_LOCAL_MACHINE
            } else {
                HKEY_CURRENT_USER
            });
            if approved_enabled(&hive, base, list, value_name) != enabled {
                return Err("提权写入后状态仍未改变".to_string());
            }
        }
        Ok(())
    }

    /// Single-quote one argument for a PowerShell -ArgumentList list.
    fn quote_ps(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    /// Run `program args…` elevated through `Start-Process -Verb RunAs`.
    /// PowerShell's own exit code reports a cancelled or refused prompt.
    fn run_elevated(program: &str, args: &[String]) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        let arglist = args
            .iter()
            .map(|arg| quote_ps(arg))
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            "$ErrorActionPreference='Stop'; Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -WindowStyle Hidden -Wait",
            quote_ps(program),
            arglist
        );
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x0800_0000)
            .status()
            .map_err(|error| format!("无法启动提权助手：{error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("管理员授权被取消，未做修改".to_string())
        }
    }

    fn display_name(command: &str) -> String {
        let trimmed = command.trim();
        let path_part = if let Some(rest) = trimmed.strip_prefix('"') {
            rest.split('"').next().unwrap_or(rest)
        } else {
            trimmed.split_whitespace().next().unwrap_or(trimmed)
        };
        Path::new(path_part)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| trimmed.to_string())
    }

    /// Locate the executable inside a launch command: quoted head, the path
    /// up to the first `.exe`/`.lnk`/`.com`, or a bare token ending in one.
    /// Anything else (wscript scripts, `cmd /c`, bare verbs) yields None.
    fn exe_from_command(command: &str) -> Option<String> {
        const EXTENSIONS: [&str; 3] = [".exe", ".lnk", ".com"];
        let text = command.trim();
        if text.is_empty() {
            return None;
        }
        if let Some(rest) = text.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                return Some(rest[..end].to_string());
            }
        }
        let lower = text.to_lowercase();
        let hit = EXTENSIONS
            .iter()
            .filter_map(|ext| lower.find(ext).map(|idx| (idx, ext.len())))
            .min()
            .map(|(idx, len)| text[..idx + len].trim().to_string())
            .filter(|path| !path.is_empty());
        if hit.is_some() {
            return hit;
        }
        let token = text.split_whitespace().next()?;
        token
            .to_lowercase()
            .ends_with(EXTENSIONS[0])
            .then(|| token.to_string())
    }

    fn has_cjk(text: &str) -> bool {
        text.chars().any(|ch| {
            matches!(
                ch as u32,
                0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff
            )
        })
    }

    /// (FileDescription, ProductName, CompanyName) from a PE's version resource.
    fn version_strings(path: &str) -> (Option<String>, Option<String>, Option<String>) {
        use std::ffi::c_void;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
        };
        let empty = (None, None, None);
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated path buffer; the handle out-param is optional.
        let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut()) };
        if size == 0 {
            return empty;
        }
        let mut block = vec![0u8; size as usize];
        // SAFETY: block is `size` bytes long, matching the request.
        let ok = unsafe {
            GetFileVersionInfoW(wide.as_ptr(), 0, size, block.as_mut_ptr() as *mut c_void)
        };
        if ok == 0 {
            return empty;
        }
        let translation = {
            let key: Vec<u16> = "\\VarFileInfo\\Translation"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut value: *mut c_void = std::ptr::null_mut();
            let mut length = 0u32;
            // SAFETY: pointers into `block` and out-params we own.
            let found = unsafe {
                VerQueryValueW(
                    block.as_ptr() as *const c_void,
                    key.as_ptr(),
                    &mut value,
                    &mut length,
                )
            };
            if found != 0 && !value.is_null() && length >= 4 {
                let pair = unsafe { std::slice::from_raw_parts(value as *const u8, 4) };
                format!(
                    "{:04x}{:04x}",
                    u16::from_le_bytes([pair[0], pair[1]]),
                    u16::from_le_bytes([pair[2], pair[3]])
                )
            } else {
                "040904b0".to_string()
            }
        };
        let query = |field: &str| -> Option<String> {
            let sub = format!("\\StringFileInfo\\{translation}\\{field}");
            let subw: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
            let mut pointer: *mut c_void = std::ptr::null_mut();
            let mut chars = 0u32;
            // SAFETY: VerQueryValue returns pointers into the version block
            // we own; `chars` counts UTF-16 units including a NUL.
            let found = unsafe {
                VerQueryValueW(
                    block.as_ptr() as *const c_void,
                    subw.as_ptr(),
                    &mut pointer,
                    &mut chars,
                )
            };
            if found == 0 || pointer.is_null() || chars <= 1 {
                return None;
            }
            let units =
                unsafe { std::slice::from_raw_parts(pointer as *const u16, chars as usize - 1) };
            let text = String::from_utf16_lossy(units).trim().to_string();
            (!text.is_empty()).then_some(text)
        };
        (
            query("FileDescription"),
            query("ProductName"),
            query("CompanyName"),
        )
    }

    /// 电脑管家's naming habit: show the Chinese product name when the binary
    /// carries one, English descriptions next, the technical name last.
    fn friendly_program_name(
        technical: &str,
        description: Option<&str>,
        product: Option<&str>,
    ) -> String {
        if product.is_some_and(has_cjk) {
            return clip_software_name(product.expect("checked"));
        }
        if description.is_some_and(has_cjk) {
            return clip_software_name(description.expect("checked"));
        }
        description
            .or(product)
            .filter(|text| !text.trim().is_empty() && !text.to_lowercase().contains("program"))
            .map_or_else(|| technical.to_string(), str::to_string)
    }

    /// Per-binary cache payload: (FileDescription, ProductName, icon PNG).
    type SoftwareInfo = (Option<String>, Option<String>, Option<Vec<u8>>);

    /// Whether a launch path is concrete enough to judge: a drive-qualified or
    /// UNC path. `foo.exe` alone resolves against *our* working directory, so
    /// answering "missing" for it would invent false 残留项.
    fn is_absolute_path(path: &str) -> bool {
        let trimmed = path.trim_matches('"');
        let bytes = trimmed.as_bytes();
        (bytes.len() > 2 && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/'))
            || trimmed.starts_with(r"\\")
    }

    /// Resolve `display_name` (and the software icon) for every scanned
    /// entry, once per binary. Also settles `target` / `target_exists`: the
    /// binary the row really runs, and whether it is still on disk (a missing
    /// one is a 残留项).
    fn decorate_display_names(entries: &mut [StartupEntry]) {
        let vendors = vendor_icon_map();
        let mut cache: std::collections::HashMap<String, SoftwareInfo> = Default::default();
        for entry in entries {
            let Some(exe) = exe_from_command(&entry.command) else {
                entry.display_name = entry.name.clone();
                continue;
            };
            let probed = exe_to_existing_path(&exe);
            // Startup-folder rows carry the .lnk; its target owns the icon.
            let probed = if probed.to_lowercase().ends_with(".lnk") {
                lnk_target(&probed).unwrap_or(probed)
            } else {
                probed
            };
            entry.target = probed.clone();
            entry.target_exists = !is_absolute_path(&probed) || Path::new(&probed).exists();
            let (description, product, icon) = cache
                .entry(probed.clone())
                .or_insert_with(|| {
                    let (description, product, _company) = version_strings(&probed);
                    let usable = |text: Option<String>| text.filter(|t| !is_placeholder_label(t));
                    (
                        usable(description),
                        usable(product),
                        software_icon(&probed, &vendors),
                    )
                })
                .clone();
            entry.display_name =
                friendly_program_name(&entry.name, description.as_deref(), product.as_deref());
            // 电脑管家's grouping rule: an entry is only attributed to a
            // software product when the binary carries real human strings;
            // everything else falls into the 其他软件 bucket.
            entry.named = description.is_some() || product.is_some();
            entry.icon = icon;
        }
    }

    /// Whether a PE/ICO file ships *real* icon resources. Service stubs such
    /// as `qmbsrv.exe` or Edge's `elevation_service.exe` embed none, and
    /// `ExtractAssociatedIconW` then silently returns the generic Windows
    /// app tile — the cause of whole columns of 权限管理 rows looking alike.
    fn has_icon_resources(path: &str) -> bool {
        use windows_sys::Win32::UI::Shell::ExtractIconExW;
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated buffer; null handle arrays only ask for the
        // count of icons the module carries.
        (unsafe {
            ExtractIconExW(
                wide.as_ptr(),
                -1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        }) > 0
    }

    /// `App Paths` is the canonical "where does this tool really live" answer:
    /// `SpaceAgent.exe` registers the product binary even when the row points
    /// at a stripped-down service stub. Returns an (icon path, index) pair
    /// that differs from `except` and exists on disk.
    fn app_paths_icon_spec(exe_name: &str, except: &str) -> Option<(String, u16)> {
        const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
        const APP_PATHS_WOW: &str =
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths";
        let candidates = [
            (HKEY_LOCAL_MACHINE, APP_PATHS),
            (HKEY_LOCAL_MACHINE, APP_PATHS_WOW),
            (HKEY_CURRENT_USER, APP_PATHS),
        ];
        for (hive, base) in candidates {
            let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(base, KEY_READ) else {
                continue;
            };
            let Ok(app) = root.open_subkey_with_flags(exe_name, KEY_READ) else {
                continue;
            };
            let default: String = app.get_value("").unwrap_or_default();
            let icon: String = app
                .open_subkey_with_flags("DefaultIcon", KEY_READ)
                .and_then(|key| key.get_value(""))
                .unwrap_or_default();
            let spec = if icon.trim().is_empty() {
                &default
            } else {
                &icon
            };
            let (path, index) = parse_display_icon(spec)?;
            if path.eq_ignore_ascii_case(except.trim_matches('"')) {
                continue;
            }
            if Path::new(&path).exists() {
                return Some((path, index));
            }
        }
        None
    }

    /// A sibling in the same folder that does ship an icon: vendors park
    /// `Workspace.ico` next to the stub (H3C) or the full binary sits one
    /// directory entry away (Edge). `.ico` files win; otherwise the largest
    /// `.exe` with real icon resources — the main product binary.
    fn sibling_icon_spec(dir: &Path) -> Option<(String, u16)> {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return None;
        };
        let mut files: Vec<PathBuf> = read_dir.filter_map(Result::ok).map(|f| f.path()).collect();
        files.sort();
        for path in &files {
            let is_ico = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ico"));
            if is_ico {
                return Some((path.to_string_lossy().into_owned(), 0));
            }
        }
        let mut best: Option<(u64, String)> = None;
        for path in &files {
            let is_exe = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
            if !is_exe || !has_icon_resources(&path.to_string_lossy()) {
                continue;
            }
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if best.as_ref().map_or(true, |(known, _)| size > *known) {
                best = Some((size, path.to_string_lossy().into_owned()));
            }
        }
        best.map(|(_, path)| (path, 0))
    }

    /// Directory → `DisplayIcon` from the uninstall registry — the authoritative
    /// "this folder belongs to product X, and X's icon is here" table. Edge,
    /// 电脑管家 and SpaceAgent all register one whose parent is exactly the
    /// folder holding their icon-less service stubs. Built once per scan.
    fn vendor_icon_map() -> std::collections::HashMap<String, (String, u16)> {
        const UNINSTALLS: [&str; 3] = [
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        ];
        let mut map: std::collections::HashMap<String, (String, u16)> = Default::default();
        for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            for base in UNINSTALLS {
                let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(base, KEY_READ) else {
                    continue;
                };
                for name in root.enum_keys().filter_map(Result::ok) {
                    let Ok(app) = root.open_subkey_with_flags(&name, KEY_READ) else {
                        continue;
                    };
                    let Some(spec): Option<String> = app.get_value("DisplayIcon").ok() else {
                        continue;
                    };
                    let Some((path, index)) = parse_display_icon(&spec) else {
                        continue;
                    };
                    let Some(dir) = Path::new(&path).parent() else {
                        continue;
                    };
                    let dir = dir
                        .to_string_lossy()
                        .to_lowercase()
                        .trim_end_matches('\\')
                        .to_string();
                    if !dir.is_empty() && !map.contains_key(&dir) && Path::new(&path).exists() {
                        map.insert(dir, (path, index));
                    }
                }
            }
        }
        map
    }

    /// Startup-list icon with the vendor fallbacks the generic extraction
    /// lacks: real resources first, then App Paths, then the uninstall
    /// registry's DisplayIcon for the same folder, then a sibling file.
    /// `None` (a neutral placeholder in the UI) beats a misleading tile.
    fn software_icon(
        path: &str,
        vendors: &std::collections::HashMap<String, (String, u16)>,
    ) -> Option<Vec<u8>> {
        if has_icon_resources(path) {
            return exe_icon_png(path, 0);
        }
        let file = Path::new(path);
        let name = file.file_name()?.to_string_lossy().to_string();
        if let Some((spec, index)) = app_paths_icon_spec(&name, path) {
            if let Some(icon) = exe_icon_png(&spec, index) {
                return Some(icon);
            }
        }
        let dir_key = file
            .parent()
            .map(|dir| dir.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if let Some((spec, index)) = vendors.get(dir_key.trim_end_matches('\\')) {
            if let Some(icon) = exe_icon_png(spec, *index) {
                return Some(icon);
            }
        }
        if let Some((spec, index)) = file.parent().and_then(sibling_icon_spec) {
            if let Some(icon) = exe_icon_png(&spec, index) {
                return Some(icon);
            }
        }
        None
    }

    /// ImagePath entries sometimes carry `"` around paths with spaces even
    /// mid-command; strip and resolve to something `GetFileVersionInfo` likes.
    fn exe_to_existing_path(exe: &str) -> String {
        exe.trim_matches('"').to_string()
    }

    /// Follow a Start-Menu `.lnk` to the binary it launches, so startup-folder
    /// entries get the same icon + product name as registry ones. Minimal MS-
    /// SHLNFF parse: shell item id list skipped, LOCAL base path ANSI-decoded.
    fn lnk_target(path: &str) -> Option<String> {
        let bytes = std::fs::read(path).ok()?;
        let head = |at: usize, len: usize| bytes.get(at..at + len);
        if head(0, 4)? != b"L\0\0\0" {
            return None;
        }
        let flags = u32::from_le_bytes(head(0x14, 4)?.try_into().ok()?);
        let mut block = 0x4Cusize;
        if flags & 0x01 != 0 {
            // Walk the ShellItemID list to its 0x0000 terminator (each item's
            // size counts its own header). Real-world .lnk files ship a bogus
            // ItemIDSize in the header — RustDesk's reports 10352 for a
            // 713-byte file — so the field is deliberately not trusted.
            for _ in 0..4096 {
                let item = u16::from_le_bytes(head(block, 2)?.try_into().ok()?);
                block += if item == 0 { 2 } else { usize::from(item) };
                if item == 0 {
                    break;
                }
            }
        }
        // LINKINFO present? and does it carry a volume-id + local base path?
        if flags & 0x20 == 0 {
            return None;
        }
        let link_flags = u32::from_le_bytes(head(block + 8, 4)?.try_into().ok()?);
        if link_flags & 0x01 == 0 {
            return None;
        }
        let base_off = u32::from_le_bytes(head(block + 0x10, 4)?.try_into().ok()?) as usize;
        let start = block.checked_add(base_off)?;
        let tail = bytes.get(start..)?;
        let end = tail.iter().position(|byte| *byte == 0)?;
        let target = decode_ansi(tail.get(..end)?).trim().to_string();
        (!target.is_empty() && Path::new(&target).exists()).then_some(target)
    }

    /// Trim a PE description to the product part: cut at Chinese punctuation
    /// ("WPS服务程序，提供账号登录、云存储等服务" → "WPS服务程序").
    fn clip_software_name(text: &str) -> String {
        let head = text
            .split(['，', '、', '；', '。', '（'])
            .next()
            .unwrap_or(text)
            .trim();
        let mut chars = head.chars();
        let shortened: String = chars.by_ref().take(24).collect();
        if chars.next().is_some() {
            format!("{shortened}…")
        } else {
            shortened.to_string()
        }
    }

    /// Visual-Studio template leftovers and one-size-fits-all strings shipped
    /// in the wild (`TODO:  <文件说明>` — Sogou; `Windows Shell Context Menu
    /// Handler` — mCloud). 电脑管家 never shows text like this.
    fn is_placeholder_label(text: &str) -> bool {
        let lower = text.trim().to_lowercase();
        lower.is_empty()
            || lower.starts_with("todo")
            || (lower.contains('<') && lower.contains('>'))
            || lower == "windows shell context menu handler"
            || lower == "shell extension"
            || lower == "context menu"
    }

    /// The last human-settled directory of a module path, skipping generic
    /// containers and version folders: `…\BaiduNetdisk\YunShellExtV1.dll` →
    /// `BaiduNetdisk`; `…\mcloud\processes\1.2.3\x.dll` → `mcloud`.
    fn module_folder_name(path: &str) -> Option<String> {
        let generic = [
            "plugins",
            "office6",
            "components",
            "processes",
            "application",
            "app",
            "bin",
            "exe",
            "addins",
            "shell",
            "contextmenu",
            "x64",
            "x86",
            "program files",
            "program files (x86)",
            "windowsapps",
        ];
        Path::new(path.trim_matches('"'))
            .parent()?
            .ancestors()
            .find_map(|dir| {
                let name = dir.file_name()?.to_string_lossy().trim().to_string();
                let lower = name.to_lowercase();
                let version_like =
                    !name.is_empty() && name.chars().all(|ch| ch.is_ascii_digit() || ch == '.');
                let usable = name.chars().count() >= 3
                    && !version_like
                    && !name.contains('_')
                    && !generic.contains(&lower.as_str());
                usable.then_some(name)
            })
    }

    /// PNG rendering of a binary's default icon at system icon size — the
    /// 电脑管家 habit of showing the product logo next to its name.
    pub(super) fn exe_icon_png(path: &str, icon_index: u16) -> Option<Vec<u8>> {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetObjectW, SelectObject,
            BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        };
        use windows_sys::Win32::UI::Shell::ExtractAssociatedIconW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DestroyIcon, DrawIconEx, GetIconInfo, DI_NORMAL, ICONINFO,
        };

        let mut wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut index = icon_index;
        // SAFETY: `wide` is a live, mutable, NUL-terminated path buffer the
        // call may rewrite; the instance handle is documented as unused.
        let icon =
            unsafe { ExtractAssociatedIconW(std::ptr::null_mut(), wide.as_mut_ptr(), &mut index) };
        if icon.is_null() {
            return None;
        }
        // SAFETY: POD out-param; `icon` is a live handle.
        let mut info: ICONINFO = unsafe { std::mem::zeroed() };
        if unsafe { GetIconInfo(icon, &mut info) } == 0 {
            unsafe { DestroyIcon(icon) };
            return None;
        }
        // SAFETY: POD out-param sized for the icon's color bitmap.
        let mut bitmap: BITMAP = unsafe { std::mem::zeroed() };
        let sized = unsafe {
            GetObjectW(
                info.hbmColor,
                std::mem::size_of::<BITMAP>() as i32,
                &mut bitmap as *mut BITMAP as *mut std::ffi::c_void,
            )
        };
        let (width, height) = if sized != 0 && bitmap.bmWidth > 0 && bitmap.bmHeight > 0 {
            (bitmap.bmWidth, bitmap.bmHeight)
        } else {
            (32, 32)
        };
        let mut result: Option<Vec<u8>> = None;
        // SAFETY: every DC/bitmap/icon handle created here is destroyed on
        // each exit path; the DIB pixel pointer is only read while `dib` is
        // selected into `dc` and before `DeleteObject(dib)`.
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if !dc.is_null() {
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height, // top-down rows match PNG order
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
                let dib =
                    CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
                if !dib.is_null() && !bits.is_null() {
                    let previous = SelectObject(dc, dib);
                    DrawIconEx(
                        dc,
                        0,
                        0,
                        icon,
                        width,
                        height,
                        0,
                        std::ptr::null_mut(),
                        DI_NORMAL,
                    );
                    let pixels = (width * height) as usize;
                    let drawn = std::slice::from_raw_parts(bits as *const u8, pixels * 4);
                    let mut rgba = vec![0u8; pixels * 4];
                    for (src, dst) in drawn.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                        dst[0] = src[2];
                        dst[1] = src[1];
                        dst[2] = src[0];
                        dst[3] = src[3];
                    }
                    if !drawn.chunks_exact(4).any(|pixel| pixel[3] != 0) {
                        mask_to_alpha(info.hbmMask, width, height, &mut rgba);
                    }
                    result = encode_png(&rgba, width as u32, height as u32);
                    SelectObject(dc, previous);
                    DeleteObject(dib);
                }
                DeleteDC(dc);
            }
            if !info.hbmColor.is_null() {
                DeleteObject(info.hbmColor);
            }
            if !info.hbmMask.is_null() {
                DeleteObject(info.hbmMask);
            }
            DestroyIcon(icon);
        }
        result
    }

    /// Pre-alpha icons have no transparency in their pixels; the 1bpp AND
    /// mask from [`ICONINFO`] marks transparent bits instead. `GetBitmapBits`
    /// hands back bottom-up rows, the PNG buffer is top-down.
    unsafe fn mask_to_alpha(
        mask: windows_sys::Win32::Graphics::Gdi::HBITMAP,
        width: i32,
        height: i32,
        rgba: &mut [u8],
    ) {
        use windows_sys::Win32::Graphics::Gdi::GetBitmapBits;
        if mask.is_null() || width <= 0 || height <= 0 {
            return;
        }
        let stride = ((width + 31) / 32) * 4;
        let mut bits = vec![0u8; (stride * height) as usize];
        if GetBitmapBits(
            mask,
            bits.len() as i32,
            bits.as_mut_ptr() as *mut std::ffi::c_void,
        ) <= 0
        {
            for pixel in rgba.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            return;
        }
        for y in 0..height as usize {
            let stored = (height as usize - 1 - y) * stride as usize;
            for x in 0..width as usize {
                let transparent = (bits[stored + x / 8] >> (7 - x % 8)) & 1 == 1;
                rgba[(y * width as usize + x) * 4 + 3] = if transparent { 0 } else { 255 };
            }
        }
    }

    fn encode_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(rgba).ok()?;
        }
        (!out.is_empty()).then_some(out)
    }

    /// Resolve an installed application's icon the way Explorer does: the
    /// `DisplayIcon` value (path + optional `,index`), else the uninstall
    /// exe, else the install folder's main executable.
    pub(crate) fn app_icon(app: &squid_core::AppInfo) -> Option<Vec<u8>> {
        if let Some(spec) = app.display_icon.as_deref() {
            if let Some((path, index)) = parse_display_icon(spec) {
                if let Some(icon) = exe_icon_png(&path, index) {
                    return Some(icon);
                }
            }
        }
        let target = primary_executable(app)?;
        exe_icon_png(&target, 0)
    }

    fn parse_display_icon(spec: &str) -> Option<(String, u16)> {
        let text = expand_environment(spec.trim().trim_matches('"'));
        // `,N` only counts as an index when a drive colon precedes it, so
        // company names like `Foo, Inc\app.exe` survive.
        if let Some((head, tail)) = text.rsplit_once(',') {
            if let Ok(index) = tail.trim().parse::<u16>() {
                if head.contains(':') {
                    let path = head.trim().trim_matches('"').to_string();
                    return (!path.is_empty()).then_some((path, index));
                }
            }
        }
        (!text.is_empty()).then_some((text, 0))
    }

    /// The exe that represents an app: its install path when that is a file,
    /// otherwise the best-matching `.exe` directly inside the folder.
    fn primary_executable(app: &squid_core::AppInfo) -> Option<String> {
        let path = &app.path;
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
        if !path.is_dir() {
            return None;
        }
        let wanted: String = app
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
        let mut best: Option<(u64, String)> = None;
        let Ok(entries) = std::fs::read_dir(path) else {
            return None;
        };
        for entry in entries.flatten().take(128) {
            let candidate = entry.path();
            let is_exe = candidate
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
            if !is_exe {
                continue;
            }
            let stem: String = candidate
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(|c| c.to_lowercase())
                .collect();
            let text = candidate.to_string_lossy().into_owned();
            if stem == wanted {
                return Some(text);
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if best.as_ref().map_or(true, |(s, _)| size > *s) {
                best = Some((size, text));
            }
        }
        best.map(|(_, p)| p)
    }

    fn decode_value(value: &winreg::RegValue) -> String {
        match value.vtype {
            RegType::REG_SZ | RegType::REG_EXPAND_SZ => String::from_utf16_lossy(
                &value
                    .bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<u16>>(),
            )
            .trim_end_matches('\0')
            .to_string(),
            _ => String::new(),
        }
    }

    struct RunSource {
        machine: bool,
        key_path: String,
        base: String,
        list: &'static str,
        label: &'static str,
        /// Task Manager prefixes 32-bit entries with "32|" in the approved list.
        prefix32: bool,
    }

    fn run_entries(source: &RunSource) -> Vec<StartupEntry> {
        let hive = RegKey::predef(if source.machine {
            HKEY_LOCAL_MACHINE
        } else {
            HKEY_CURRENT_USER
        });
        let Ok(key) = hive.open_subkey_with_flags(&source.key_path, KEY_READ) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (name, value) in key.enum_values().filter_map(Result::ok) {
            let command = decode_value(&value);
            if command.is_empty() {
                continue;
            }
            let marker = if source.prefix32 {
                format!("32|{name}")
            } else {
                name.clone()
            };
            let enabled = approved_enabled(&hive, &source.base, source.list, &marker);
            let hive_label = if source.machine { "HKLM" } else { "HKCU" };
            out.push(StartupEntry {
                id: format!("reg|{hive_label}|{}|{name}", source.key_path),
                name: display_name(&command),
                command,
                target: String::new(),
                target_exists: true,
                trigger: "logon".to_string(),
                system_item: false,
                signature: String::new(),
                signer: None,
                display_name: String::new(),
                icon: None,
                source: "registry".to_string(),
                source_label: source.label.to_string(),
                scope: if source.machine {
                    "machine".to_string()
                } else {
                    "currentUser".to_string()
                },
                enabled,
                named: false,
            });
        }
        out
    }

    fn startup_folder_entries(dir: &Path, machine: bool) -> Vec<StartupEntry> {
        let mut out = Vec::new();
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return out;
        };
        let hive = RegKey::predef(if machine {
            HKEY_LOCAL_MACHINE
        } else {
            HKEY_CURRENT_USER
        });
        for file in read_dir.filter_map(Result::ok) {
            let path = file.path();
            let Some(file_name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            let extension = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !matches!(extension.as_str(), "lnk" | "exe" | "bat" | "cmd") {
                continue;
            }
            let enabled = approved_enabled(&hive, CURRENT_VERSION, "StartupFolder", &file_name);
            out.push(StartupEntry {
                id: format!("folder|{}", path.display()),
                name: path
                    .file_stem()
                    .map_or_else(|| file_name.clone(), |s| s.to_string_lossy().to_string()),
                command: path.display().to_string(),
                target: String::new(),
                target_exists: true,
                trigger: "logon".to_string(),
                system_item: false,
                signature: String::new(),
                signer: None,
                display_name: String::new(),
                icon: None,
                source: "startupFolder".to_string(),
                source_label: if machine {
                    "启动文件夹（所有用户）".to_string()
                } else {
                    "启动文件夹（当前用户）".to_string()
                },
                scope: if machine {
                    "machine".to_string()
                } else {
                    "currentUser".to_string()
                },
                enabled,
                named: false,
            });
        }
        out
    }

    fn scheduled_task_entries() -> Vec<StartupEntry> {
        let root = PathBuf::from(
            std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string()),
        )
        .join(r"System32\Tasks");
        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(read_dir) = std::fs::read_dir(&dir) else {
                continue;
            };
            for file in read_dir.filter_map(Result::ok) {
                let path = file.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = match path.strip_prefix(&root) {
                    Ok(rel) => rel.to_path_buf(),
                    Err(_) => continue,
                };
                // The \Microsoft\Windows tree is the OS's own schedule —
                // 电脑管家 counts every other vendor folder (\Microsoft\Office
                // included) and does not require a logon trigger.
                if relative.starts_with(r"Microsoft\Windows") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let task_name = format!("\\{}", relative.display());
                out.push(StartupEntry {
                    id: format!("task|{task_name}"),
                    name: relative
                        .file_name()
                        .map_or_else(|| task_name.clone(), |n| n.to_string_lossy().to_string()),
                    command: task_actions(&text).unwrap_or_else(|| task_name.clone()),
                    target: String::new(),
                    target_exists: true,
                    trigger: task_trigger(&text),
                    system_item: false,
                    signature: String::new(),
                    signer: None,
                    display_name: String::new(),
                    icon: None,
                    source: "scheduledTask".to_string(),
                    source_label: "计划任务".to_string(),
                    scope: "machine".to_string(),
                    enabled: task_settings_enabled(&text),
                    named: false,
                });
            }
        }
        if out.is_empty() {
            // Some machines deny plain users read on the task XML files even
            // when enumeration half-works; the Scheduler service answers for
            // us instead of silently listing zero tasks.
            return schtasks_fallback();
        }
        out
    }

    fn schtasks_fallback() -> Vec<StartupEntry> {
        use std::os::windows::process::CommandExt;
        // schtasks.exe refuses piped stdout when the caller has no real
        // console (GUI app under Task Scheduler), so round-trip a temp file
        // the way 电脑管家 itself does.
        let dump = std::env::temp_dir().join(format!("squid-schtasks-{}.csv", std::process::id()));
        let quoted = format!("\"{}\"", dump.display());
        let command = format!("chcp 65001 >nul && schtasks /Query /FO CSV /NH > {quoted} 2>nul");
        let spawned = std::process::Command::new("cmd.exe")
            .args(["/C", &command])
            .creation_flags(0x0800_0000)
            .output();
        let text = match spawned {
            Ok(_) => {
                let bytes = std::fs::read(&dump).unwrap_or_default();
                // chcp 65001 makes schtasks emit UTF-8; if the console code
                // page fight back, decode the ANSI bytes instead.
                match String::from_utf8(bytes.clone()) {
                    Ok(text) => text,
                    Err(_) => decode_ansi(&bytes),
                }
            }
            Err(_) => String::new(),
        };
        let _ = std::fs::remove_file(&dump);
        let mut seen: std::collections::HashMap<String, usize> = Default::default();
        let mut out: Vec<StartupEntry> = Vec::new();
        for line in text.lines() {
            let fields = parse_csv_line(line);
            let (Some(task_name), Some(status)) = (fields.first(), fields.get(2)) else {
                continue;
            };
            if task_name.starts_with(r"\Microsoft\Windows") {
                continue;
            }
            // One row per trigger; keep a single entry, enabled if any row is.
            if let Some(index) = seen.get(task_name) {
                if status.eq_ignore_ascii_case("Ready")
                    || status == "就绪"
                    || status == "正在等待"
                    || status == "正在运行"
                {
                    out[*index].enabled = true;
                }
                continue;
            }
            let enabled = status.eq_ignore_ascii_case("Ready")
                || status.eq_ignore_ascii_case("Running")
                || status == "就绪"
                || status == "正在等待"
                || status == "正在运行";
            seen.insert(task_name.clone(), out.len());
            out.push(StartupEntry {
                id: format!("task|{task_name}"),
                name: task_name
                    .rsplit_once('\\')
                    .map_or_else(|| task_name.clone(), |(_, leaf)| leaf.to_string()),
                command: String::new(),
                target: String::new(),
                // Nothing was resolved here, so "missing" would be a lie.
                target_exists: true,
                trigger: "unknown".to_string(),
                system_item: false,
                signature: String::new(),
                signer: None,
                display_name: String::new(),
                icon: None,
                source: "scheduledTask".to_string(),
                source_label: "计划任务".to_string(),
                scope: "machine".to_string(),
                enabled,
                named: false,
            });
        }
        out
    }

    /// Decode console output in the system ANSI codepage (GBK on zh-CN).
    fn decode_ansi(bytes: &[u8]) -> String {
        use windows_sys::Win32::Globalization::MultiByteToWideChar;
        if bytes.is_empty() {
            return String::new();
        }
        let flags: u32 = 0; // MULTI_BYTE_TO_WIDE_CHAR_FLAGS is a u32 alias
        let count = unsafe {
            // SAFETY: CP_ACP with valid buffer/length; null out-buffer asks
            // for the required wchar count.
            MultiByteToWideChar(
                0,
                flags,
                bytes.as_ptr(),
                bytes.len() as i32,
                std::ptr::null_mut(),
                0,
            )
        };
        if count <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut wide = vec![0u16; count as usize];
        let written = unsafe {
            // SAFETY: wide holds `count` slots as returned above.
            MultiByteToWideChar(
                0,
                flags,
                bytes.as_ptr(),
                bytes.len() as i32,
                wide.as_mut_ptr(),
                count,
            )
        };
        if written <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        String::from_utf16_lossy(&wide[..written as usize])
    }

    fn parse_csv_line(line: &str) -> Vec<String> {
        let mut fields = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '"' if in_quotes && chars.peek() == Some(&'"') => {
                    current.push('"');
                    chars.next();
                }
                '"' => in_quotes = !in_quotes,
                ',' if !in_quotes => {
                    fields.push(std::mem::take(&mut current));
                }
                other => current.push(other),
            }
        }
        fields.push(current);
        fields
    }

    /// 管家's 软件启动项 tab also folds in Win32 services that live outside
    /// `%SystemRoot%` — automatic *and* manual ones (百度网盘's "部分开启"
    /// rows are exactly the manual services). `Start = 2` is auto, 3 manual,
    /// 4 disabled; all three are listed, only auto counts as enabled.
    fn auto_service_entries() -> Vec<StartupEntry> {
        let Ok(root) =
            RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(SERVICES_KEY, KEY_READ)
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for name in root.enum_keys().filter_map(Result::ok) {
            let Ok(service) = root.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            // SERVICE_WIN32_OWN_PROCESS | SERVICE_WIN32_SHARE_PROCESS
            if service.get_value::<u32, _>("Type").unwrap_or(0) & 0x30 == 0 {
                continue;
            }
            let start = service.get_value::<u32, _>("Start").unwrap_or(3);
            if !matches!(start, 2..=4) {
                continue;
            }
            let image = service
                .get_value::<String, _>("ImagePath")
                .map(|image| expand_environment(&image))
                .unwrap_or_default();
            let lowered = image.to_lowercase().replace('/', "\\");
            if lowered.contains("\\system32\\")
                || lowered.contains("\\syswow64\\")
                || lowered.starts_with("\\??\\")
            {
                continue;
            }
            let mut display = service
                .get_value::<String, _>("DisplayName")
                .map(|display| expand_environment(&display))
                .unwrap_or_else(|_| name.clone());
            // MUI indirection ("foo.exe,-1001" / "@dll,-id") — fall back to
            // the service key, which is always readable and unique.
            let indirect = display.starts_with('@')
                || (display.contains(',')
                    && (display.contains(".dll") || display.contains(".exe")));
            if indirect {
                display = name.clone();
            }
            out.push(StartupEntry {
                id: format!("svc|{name}"),
                name: display,
                command: image,
                target: String::new(),
                target_exists: true,
                // Automatic services are started by the SCM during boot;
                // manual/disabled ones only run on demand.
                trigger: if start == 2 { "boot" } else { "background" }.to_string(),
                system_item: false,
                signature: String::new(),
                signer: None,
                display_name: String::new(),
                icon: None,
                source: "service".to_string(),
                source_label: match start {
                    2 => "服务（自动启动）",
                    3 => "服务（手动启动）",
                    _ => "服务（已禁用）",
                }
                .to_string(),
                scope: "machine".to_string(),
                enabled: start == 2,
                named: false,
            });
        }
        out
    }

    /// Expand `%VAR%` pairs the way the SCM does, best effort.
    fn expand_environment(text: &str) -> String {
        let mut out = text.to_string();
        for _ in 0..8 {
            let Some(start) = out.find('%') else { break };
            let Some(end) = out[start + 1..].find('%') else {
                break;
            };
            let key = &out[start + 1..start + 1 + end];
            let replacement = std::env::var(key).unwrap_or_default();
            out = format!(
                "{}{}{}",
                &out[..start],
                replacement,
                &out[start + end + 2..]
            );
        }
        out
    }

    /// The task-level `<Enabled>` flag lives inside `<Settings>`.
    fn task_settings_enabled(text: &str) -> bool {
        let Some(start) = text.find("<Settings") else {
            return true;
        };
        let tail = &text[start..];
        let end = tail.find("</Settings>").unwrap_or(tail.len());
        !tail[..end].contains("<Enabled>false")
    }

    /// When a task fires: a BootTrigger runs before anyone logs on, a
    /// LogonTrigger at logon, anything else (calendar, event, maintenance) is
    /// a background schedule the user does not experience as "startup".
    fn task_trigger(text: &str) -> String {
        if text.contains("<BootTrigger") {
            "boot".to_string()
        } else if text.contains("<LogonTrigger") {
            "logon".to_string()
        } else if text.contains("<Triggers>") {
            "background".to_string()
        } else {
            "unknown".to_string()
        }
    }

    fn task_actions(text: &str) -> Option<String> {
        let start = text.find("<Command>")? + "<Command>".len();
        let end = text[start..].find("</Command>")? + start;
        Some(text[start..end].trim().to_string())
    }

    pub(super) fn scan_startup() -> Vec<StartupEntry> {
        let mut entries = Vec::new();
        let sources = [
            RunSource {
                machine: false,
                key_path: format!(r"{CURRENT_VERSION}\Run"),
                base: CURRENT_VERSION.to_string(),
                list: "Run",
                label: r"HKCU\...\CurrentVersion\Run",
                prefix32: false,
            },
            RunSource {
                machine: false,
                key_path: format!(r"{CURRENT_VERSION}\RunOnce"),
                base: CURRENT_VERSION.to_string(),
                list: "RunOnce",
                label: r"HKCU\...\CurrentVersion\RunOnce",
                prefix32: false,
            },
            RunSource {
                machine: true,
                key_path: format!(r"{CURRENT_VERSION}\Run"),
                base: CURRENT_VERSION.to_string(),
                list: "Run",
                label: r"HKLM\...\CurrentVersion\Run",
                prefix32: false,
            },
            RunSource {
                machine: true,
                key_path: format!(r"{WOW_CURRENT_VERSION}\Run"),
                base: WOW_CURRENT_VERSION.to_string(),
                list: "Run",
                label: r"HKLM\...\WOW6432Node (32 位)",
                prefix32: true,
            },
        ];
        for source in &sources {
            entries.extend(run_entries(source));
        }
        let user_startup = std::env::var("APPDATA")
            .map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
            .unwrap_or_default();
        let common_startup = std::env::var("ProgramData")
            .map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
            .unwrap_or_default();
        entries.extend(startup_folder_entries(&user_startup, false));
        entries.extend(startup_folder_entries(&common_startup, true));
        entries.extend(scheduled_task_entries());
        entries.extend(auto_service_entries());
        decorate_display_names(&mut entries);
        enrich_trust(&mut entries);
        entries
    }

    /// One batched signature grade over every resolved target, then stamp the
    /// verdict and fold Microsoft-signed / %SystemRoot% rows into 系统组件 —
    /// the same trust table the 进阶启动项 tab shows, shared so a scan pays
    /// for one PowerShell process, not one per row.
    fn enrich_trust(entries: &mut [StartupEntry]) {
        let system_root = std::env::var("SystemRoot")
            .unwrap_or_else(|_| r"C:\Windows".to_string())
            .to_lowercase();
        let mut targets: Vec<String> = Vec::new();
        for entry in entries.iter_mut() {
            entry.system_item = entry.target.to_lowercase().starts_with(&system_root);
            if entry.target_exists
                && !entry.target.is_empty()
                && !targets
                    .iter()
                    .any(|seen| seen.eq_ignore_ascii_case(&entry.target))
            {
                targets.push(entry.target.clone());
            }
        }
        let grades = crate::advanced::windows::signature_grades(&targets);
        for entry in entries.iter_mut() {
            if let Some((verdict, signer)) = grades.get(&entry.target.to_ascii_lowercase()) {
                entry.signature = verdict.clone();
                entry.signer = signer.clone();
                if verdict == "microsoft" {
                    entry.system_item = true;
                }
            } else {
                entry.signature = "unknown".into();
            }
        }
    }

    pub(super) fn set_startup_enabled(entry: &StartupEntry, enabled: bool) -> Result<(), String> {
        let Some((kind, rest)) = entry.id.split_once('|') else {
            return Err("未知的启动项来源".to_string());
        };
        match kind {
            "reg" => {
                let pieces: Vec<&str> = rest.splitn(3, '|').collect();
                let [hive_label, key_path, name] = pieces.as_slice() else {
                    return Err("未知的启动项来源".to_string());
                };
                let (machine, base, list, prefix32) = split_run_path(hive_label, key_path);
                let marker = if prefix32 {
                    format!("32|{name}")
                } else {
                    name.to_string()
                };
                flip_approved(machine, &base, list, &marker, enabled)
            }
            "folder" => {
                let file = Path::new(rest)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .ok_or_else(|| "启动文件夹项目路径无效".to_string())?;
                let machine = rest.contains("ProgramData");
                flip_approved(machine, CURRENT_VERSION, "StartupFolder", &file, enabled)
            }
            "task" => {
                use std::os::windows::process::CommandExt;
                let output = std::process::Command::new("schtasks.exe")
                    .args([
                        "/Change",
                        "/TN",
                        rest,
                        if enabled { "/Enable" } else { "/Disable" },
                    ])
                    .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                    .output()
                    .map_err(|error| format!("无法调用 schtasks：{error}"))?;
                if output.status.success() {
                    return Ok(());
                }
                let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
                // schtasks reports "task not ready", missing tasks, etc. —
                // only a plain access denial justifies a UAC round-trip.
                let denied = stderr.contains("access is denied")
                    || stderr.contains("access denied")
                    || stderr.contains("拒绝访问");
                if !denied {
                    return Err(format!(
                        "计划任务修改失败：{}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                run_elevated(
                    "schtasks.exe",
                    &[
                        "/Change".to_string(),
                        "/TN".to_string(),
                        rest.to_string(),
                        if enabled { "/Enable" } else { "/Disable" }.to_string(),
                    ],
                )?;
                Ok(())
            }
            "svc" => {
                // 开 = Automatic(2)，关 = Manual(3)：保留按需启动能力，
                // 与管家的“禁止开机自启”语义一致；HKLM 写入需要管理员。
                let start = if enabled { 2u32 } else { 3u32 };
                let key_path = format!(r"{SERVICES_KEY}\{rest}");
                let direct = (|| -> std::io::Result<()> {
                    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
                        .open_subkey_with_flags(&key_path, KEY_READ | KEY_WRITE)?;
                    key.set_value("Start", &start)
                })();
                if let Err(why) = direct {
                    if why.kind() != std::io::ErrorKind::PermissionDenied {
                        return Err(registry_error(why));
                    }
                    run_elevated(
                        "reg.exe",
                        &[
                            "add".to_string(),
                            format!(r"HKLM\{key_path}"),
                            "/v".to_string(),
                            "Start".to_string(),
                            "/t".to_string(),
                            "REG_DWORD".to_string(),
                            "/d".to_string(),
                            start.to_string(),
                            "/f".to_string(),
                        ],
                    )?;
                    let live = RegKey::predef(HKEY_LOCAL_MACHINE)
                        .open_subkey_with_flags(&key_path, KEY_READ)
                        .map(|key| key.get_value::<u32, _>("Start").ok())
                        .ok()
                        .flatten();
                    if live != Some(start) {
                        return Err("提权写入后服务状态仍未改变".to_string());
                    }
                }
                if service_control(rest, if enabled { "auto" } else { "demand" }).is_err() {
                    let _ = run_elevated(
                        "sc.exe",
                        &[
                            "config".to_string(),
                            rest.to_string(),
                            "start=".to_string(),
                            if enabled { "auto" } else { "demand" }.to_string(),
                        ],
                    );
                }
                Ok(())
            }
            _ => Err("未知的启动项来源".to_string()),
        }
    }

    /// Best-effort `sc config` mirror of the registry edit (the SCM caches
    /// live state; failure here is not fatal because Start is authoritative).
    fn service_control(name: &str, start: &str) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("sc.exe")
            .args(["config", name, "start=", start])
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|error| format!("sc 调用失败：{error}"))
            .map(|_| ())
    }

    /// Remove one autostart registration; see the module-level
    /// `remove_startup_entry` for the semantics per source kind.
    pub(super) fn remove_startup_entry(entry: &StartupEntry) -> Result<(), String> {
        let Some((kind, rest)) = entry.id.split_once('|') else {
            return Err("未知的启动项来源".to_string());
        };
        match kind {
            "reg" => {
                let pieces: Vec<&str> = rest.splitn(3, '|').collect();
                let [hive_label, key_path, name] = pieces.as_slice() else {
                    return Err("未知的启动项来源".to_string());
                };
                let (machine, base, list, prefix32) = split_run_path(hive_label, key_path);
                let hive = RegKey::predef(if machine {
                    HKEY_LOCAL_MACHINE
                } else {
                    HKEY_CURRENT_USER
                });
                let direct = (|| -> std::io::Result<()> {
                    let key = hive.open_subkey_with_flags(key_path, KEY_READ | KEY_WRITE)?;
                    key.delete_value(*name)
                })();
                if let Err(why) = direct {
                    if why.kind() != std::io::ErrorKind::PermissionDenied {
                        return Err(registry_error(why));
                    }
                    run_elevated(
                        "reg.exe",
                        &[
                            "delete".to_string(),
                            format!(r"{hive_label}\{key_path}"),
                            "/v".to_string(),
                            name.to_string(),
                            "/f".to_string(),
                        ],
                    )?;
                    if hive
                        .open_subkey_with_flags(key_path, KEY_READ)
                        .map(|key| key.get_raw_value(*name).is_ok())
                        .unwrap_or(false)
                    {
                        return Err("提权删除后启动项仍然存在".to_string());
                    }
                }
                // Task Manager keeps its own approval marker next to the row;
                // drop it so the removed name cannot resurface as a ghost.
                let marker = if prefix32 {
                    format!("32|{name}")
                } else {
                    name.to_string()
                };
                let _ = hive
                    .open_subkey_with_flags(
                        format!(r"{base}\Explorer\StartupApproved\{list}"),
                        KEY_READ | KEY_WRITE,
                    )
                    .and_then(|key| key.delete_value(&marker));
                Ok(())
            }
            "folder" => {
                let file = Path::new(rest);
                match std::fs::remove_file(file) {
                    Ok(()) => Ok(()),
                    Err(why) if why.kind() == std::io::ErrorKind::PermissionDenied => {
                        run_elevated(
                            "cmd.exe",
                            &["/C".to_string(), "del".to_string(), format!("/F /Q {rest}")],
                        )?;
                        if file.exists() {
                            return Err("提权删除后快捷方式仍然存在".to_string());
                        }
                        Ok(())
                    }
                    Err(why) => Err(format!("删除快捷方式失败：{why}")),
                }
            }
            "task" => {
                use std::os::windows::process::CommandExt;
                let output = std::process::Command::new("schtasks.exe")
                    .args(["/Delete", "/TN", rest, "/F"])
                    .creation_flags(0x0800_0000)
                    .output()
                    .map_err(|error| format!("无法调用 schtasks：{error}"))?;
                if output.status.success() {
                    return Ok(());
                }
                let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
                if stderr.contains("access is denied")
                    || stderr.contains("access denied")
                    || stderr.contains("拒绝访问")
                {
                    run_elevated(
                        "schtasks.exe",
                        &[
                            "/Delete".to_string(),
                            "/TN".to_string(),
                            rest.to_string(),
                            "/F".to_string(),
                        ],
                    )?;
                    return Ok(());
                }
                Err(format!(
                    "删除计划任务失败：{}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ))
            }
            "svc" => {
                // Uninstalling a service outright breaks anything that still
                // opens it, so 残留服务 is disabled instead (Start = 4).
                set_startup_enabled(entry, false)?;
                let key_path = format!(r"{SERVICES_KEY}\{rest}");
                let direct = (|| -> std::io::Result<()> {
                    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
                        .open_subkey_with_flags(&key_path, KEY_READ | KEY_WRITE)?;
                    key.set_value("Start", &4u32)
                })();
                if let Err(why) = direct {
                    if why.kind() != std::io::ErrorKind::PermissionDenied {
                        return Err(registry_error(why));
                    }
                    run_elevated(
                        "reg.exe",
                        &[
                            "add".to_string(),
                            format!(r"HKLM\{key_path}"),
                            "/v".to_string(),
                            "Start".to_string(),
                            "/t".to_string(),
                            "REG_DWORD".to_string(),
                            "/d".to_string(),
                            "4".to_string(),
                            "/f".to_string(),
                        ],
                    )?;
                }
                let _ = service_control(rest, "disabled");
                Ok(())
            }
            _ => Err("这类启动项暂不支持删除".to_string()),
        }
    }

    fn split_run_path(hive_label: &str, key_path: &str) -> (bool, String, &'static str, bool) {
        let machine = hive_label == "HKLM";
        let wow = key_path.starts_with(WOW_CURRENT_VERSION);
        let base = if wow {
            WOW_CURRENT_VERSION.to_string()
        } else {
            CURRENT_VERSION.to_string()
        };
        let list = if key_path.ends_with("RunOnce") {
            "RunOnce"
        } else {
            "Run"
        };
        (machine, base, list, wow)
    }

    // ---- context menu handlers ------------------------------------------
    //
    // Category scheme mirrors 电脑管家's four surfaces. Directory\Background
    // is what its UI calls 桌面 (the empty spot on the desktop is the same
    // shell object); Folder handlers belong under 文件夹 like Directory's.

    const EXTENSION_POINTS: [(&str, &str); 6] = [
        (r"SOFTWARE\Classes\*\shellex\ContextMenuHandlers", "文件"),
        (
            r"SOFTWARE\Classes\Directory\shellex\ContextMenuHandlers",
            "文件夹",
        ),
        (
            r"SOFTWARE\Classes\Folder\shellex\ContextMenuHandlers",
            "文件夹",
        ),
        (
            r"SOFTWARE\Classes\Directory\Background\shellex\ContextMenuHandlers",
            "桌面",
        ),
        (
            r"SOFTWARE\Classes\Drive\shellex\ContextMenuHandlers",
            "驱动器",
        ),
        (
            r"SOFTWARE\Classes\AllFileSystemObjects\shellex\ContextMenuHandlers",
            "所有对象",
        ),
    ];

    /// Registry key names that are pure Windows plumbing — 电脑管家 never
    /// offers to switch these off.
    const BUILTIN_HANDLERS: [&str; 8] = [
        "New",
        "Sharing",
        "WorkFolders",
        "Offline Files",
        "Library Location",
        "EnhancedStorageShell",
        "EncryptionMenu",
        "SendTo",
    ];

    fn is_builtin_handler(name: &str, friendly: Option<&str>, module: Option<&str>) -> bool {
        if BUILTIN_HANDLERS
            .iter()
            .any(|builtin| name.eq_ignore_ascii_case(builtin))
        {
            return true;
        }
        // System binaries (shell32/windowsui/shellcore) are not "managed".
        module.is_some_and(|path| path.to_lowercase().contains("\\system32\\"))
            || friendly.is_none()
    }

    /// Trim an InprocServer32 default value down to the binary itself:
    /// `"C:\app\bndshell.dll" -Embedding` → `C:\app\bndshell.dll`.
    fn clean_module_path(raw: &str) -> String {
        let raw = raw.trim();
        if let Some(rest) = raw.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                return rest[..end].trim().to_string();
            }
        }
        let lower = raw.to_lowercase();
        for suffix in [" -embedding", "/embedding", " -server"] {
            if let Some(head) = lower.strip_suffix(suffix) {
                return raw[..head.len()].trim_end().to_string();
            }
        }
        raw.to_string()
    }

    fn point_entries(
        hive: &RegKey,
        path: &str,
        point: &str,
        machine: bool,
        id_hive: &str,
    ) -> Vec<ShellExtEntry> {
        let mut out = Vec::new();
        let Ok(root) = hive.open_subkey_with_flags(path, KEY_READ) else {
            return out;
        };
        for name in root.enum_keys().filter_map(|entry| entry.ok()) {
            let Ok(handler) = root.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            let clsid = handler
                .get_value::<String, _>("")
                .unwrap_or_default()
                .trim()
                .to_string();
            let (clsid_name, module) = resolve_clsid(&clsid);
            // Friendly naming the 电脑管家 way: the owning binary's own
            // description beats the raw CLSID class name users can't parse;
            // VS-template placeholders fall through to the install folder.
            let mut module_label = None;
            let mut publisher = None;
            let mut icon = None;
            let mut cleaned = None;
            if let Some(dll) = module.as_deref() {
                let clean = clean_module_path(dll);
                let (description, product, company) = version_strings(&clean);
                let usable = |text: Option<String>| text.filter(|text| !is_placeholder_label(text));
                let label = friendly_program_name(
                    &name,
                    usable(description).as_deref(),
                    usable(product).as_deref(),
                );
                module_label = (label.trim() != name.trim() && !is_placeholder_label(&label))
                    .then(|| label.trim().to_string());
                publisher = usable(company);
                icon = exe_icon_png(&clean, 0);
                cleaned = Some(clean);
            }
            // Attribution for the UI's software grouping: a row belongs to a
            // product when its module carries real strings or sits in a
            // recognisable install folder — same rule the startup list uses.
            let folder_label = cleaned.as_deref().and_then(module_folder_name);
            let named = module_label.is_some() || folder_label.is_some();
            let friendly_name = module_label.or(folder_label).or(clsid_name);
            if is_builtin_handler(&name, friendly_name.as_deref(), cleaned.as_deref()) {
                continue;
            }
            out.push(ShellExtEntry {
                id: format!("shlext|{id_hive}|{path}|{name}"),
                name: name.trim().to_string(),
                handler: clsid,
                extension_point: point.to_string(),
                scope: if machine {
                    "machine".to_string()
                } else {
                    "currentUser".to_string()
                },
                friendly_name,
                module: cleaned.or(module),
                publisher,
                icon,
                named,
            });
        }
        out
    }

    pub(super) fn scan_shell_extensions() -> Vec<ShellExtEntry> {
        let machine = RegKey::predef(HKEY_LOCAL_MACHINE);
        let user = RegKey::predef(HKEY_CURRENT_USER);
        let mut out = Vec::new();
        for (path, point) in EXTENSION_POINTS {
            out.extend(point_entries(&machine, path, point, true, "HKLM"));
            let user_path = path.replace(r"SOFTWARE\Classes", r"Software\Classes");
            out.extend(point_entries(&user, &user_path, point, false, "HKCU"));
        }
        // One row per module per category (电脑管家 dedups the same way —
        // Bandizip registers into every surface from one dll). Machine-scope
        // rows win; the parked HKCU twin keeps its own id for restore.
        let point_rank = |point: &str| {
            EXTENSION_POINTS
                .iter()
                .position(|(_, name)| *name == point)
                .unwrap_or(usize::MAX)
        };
        out.sort_by(|a, b| {
            let display = |entry: &ShellExtEntry| {
                entry
                    .friendly_name
                    .as_deref()
                    .unwrap_or(&entry.name)
                    .to_lowercase()
            };
            point_rank(&a.extension_point)
                .cmp(&point_rank(&b.extension_point))
                .then_with(|| display(a).cmp(&display(b)))
                .then_with(|| (b.scope == "machine").cmp(&(a.scope == "machine")))
        });
        let mut seen: std::collections::HashSet<(String, String)> = Default::default();
        out.retain(|entry| {
            let key = (
                entry.extension_point.clone(),
                entry
                    .module
                    .clone()
                    .unwrap_or_else(|| entry.handler.to_lowercase())
                    .to_lowercase(),
            );
            seen.insert(key)
        });
        out
    }

    fn resolve_clsid(clsid: &str) -> (Option<String>, Option<String>) {
        let clean = clsid.trim_matches('{').trim_matches('}');
        if clean.len() != 36 {
            return (None, None);
        }
        let guid = format!("{{{clean}}}");
        let Ok(hkcr) = RegKey::predef(HKEY_CLASSES_ROOT)
            .open_subkey_with_flags(format!(r"CLSID\{guid}"), KEY_READ)
        else {
            return (None, None);
        };
        let friendly = hkcr
            .get_value::<String, _>("")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let module = ["InprocServer32", "LocalServer32"].iter().find_map(|kind| {
            hkcr.open_subkey_with_flags(kind, KEY_READ)
                .ok()?
                .get_value::<String, _>("")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
        (friendly, module)
    }

    fn split_shlext_id(id: &str) -> Option<(RegKey, String, String)> {
        let mut pieces = id.splitn(4, '|');
        let kind = pieces.next()?;
        let hive = pieces.next()?;
        let path = pieces.next()?.to_string();
        let name = pieces.next()?.to_string();
        if kind != "shlext" {
            return None;
        }
        let root = match hive {
            "HKLM" => RegKey::predef(HKEY_LOCAL_MACHINE),
            "HKCU" => RegKey::predef(HKEY_CURRENT_USER),
            _ => return None,
        };
        Some((root, path, name))
    }

    pub(super) fn remove_shell_extension(entry: &ShellExtEntry) -> Result<(), String> {
        let (root, path, name) =
            split_shlext_id(&entry.id).ok_or_else(|| "未知的右键菜单项".to_string())?;
        const DELETE: u32 = 0x0001_0000;
        let parent = root
            .open_subkey_with_flags(&path, KEY_READ | DELETE)
            .map_err(registry_error)?;
        parent.delete_subkey_all(&name).map_err(registry_error)
    }

    pub(super) fn restore_shell_extension(entry: &ShellExtEntry) -> Result<(), String> {
        let (root, path, name) =
            split_shlext_id(&entry.id).ok_or_else(|| "未知的右键菜单项".to_string())?;
        let parent = root
            .open_subkey_with_flags(&path, KEY_READ | KEY_WRITE)
            .map_err(registry_error)?;
        let (handler, _) = parent.create_subkey(&name).map_err(registry_error)?;
        if !entry.handler.is_empty() {
            handler
                .set_value("", &entry.handler)
                .map_err(registry_error)?;
        }
        Ok(())
    }

    // ---- popup windows ----------------------------------------------------

    use windows_sys::Win32::Foundation::{CloseHandle, HWND};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible, PostMessageW, GA_ROOTOWNER, WM_CLOSE,
    };

    fn window_text(hwnd: HWND) -> String {
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; (length as usize) + 1];
        // SAFETY: the buffer is length+1 u16 cells, matching GetWindowTextW's
        // contract for the cchMax argument.
        let written = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), length + 1) };
        if written <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..written as usize])
    }

    fn process_image(pid: u32) -> Option<String> {
        unsafe {
            // SAFETY: the handle is closed on every path and the buffer size
            // is passed alongside the pointer.
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut buffer = vec![0u16; 4096];
            let mut size = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size);
            CloseHandle(handle);
            if ok == 0 {
                return None;
            }
            Some(String::from_utf16_lossy(&buffer[..size as usize]))
        }
    }

    pub(super) fn list_popup_windows(with_icons: bool) -> Vec<PopupWindow> {
        let mut icon_cache: std::collections::HashMap<String, Option<Vec<u8>>> = Default::default();
        let mut icon_for = |exe: &Option<String>| -> Option<Vec<u8>> {
            if !with_icons {
                return None;
            }
            let path = exe.as_deref()?.trim_matches('"').to_string();
            if let Some(cached) = icon_cache.get(&path) {
                return cached.clone();
            }
            let icon = exe_icon_png(&path, 0);
            icon_cache.insert(path, icon.clone());
            icon
        };
        unsafe extern "system" fn callback(hwnd: HWND, lparam: isize) -> i32 {
            // SAFETY: EnumWindows is synchronous and the Vec behind this
            // pointer outlives every callback for the call.
            let collected = &mut *(lparam as *mut Vec<(HWND, String, String, Option<String>)>);
            if IsWindowVisible(hwnd) == 0 || GetAncestor(hwnd, GA_ROOTOWNER) != hwnd {
                return 1;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid as *mut u32);
            if pid == 0 || pid == GetCurrentProcessId() {
                return 1;
            }
            let title = window_text(hwnd);
            if title.trim().is_empty() {
                return 1;
            }
            let exe = process_image(pid);
            let process = exe
                .as_deref()
                .and_then(|path| {
                    Path::new(path)
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| format!("pid {pid}"));
            collected.push((hwnd, title, process, exe));
            1
        }

        // Icons extract via GDI; EnumWindows must run on the thread that will
        // pump messages next, so collect raw rows first, then decorate.
        let mut raw: Vec<(HWND, String, String, Option<String>)> = Vec::new();
        // SAFETY: `raw` outlives the synchronous enumeration; the callback
        // only appends through the pointer passed as its LPARAM.
        unsafe {
            EnumWindows(Some(callback), &mut raw as *mut Vec<_> as isize);
        }
        let mut windows: Vec<PopupWindow> = raw
            .into_iter()
            .map(|(hwnd, title, process, exe)| {
                let icon = icon_for(&exe);
                PopupWindow {
                    hwnd: hwnd as i64,
                    title,
                    process,
                    exe,
                    icon,
                }
            })
            .collect();
        windows.sort_by(|a, b| {
            a.process
                .to_lowercase()
                .cmp(&b.process.to_lowercase())
                .then_with(|| a.title.cmp(&b.title))
        });
        windows
    }

    pub(super) fn close_window(hwnd: i64) -> bool {
        // SAFETY: posting WM_CLOSE is the standard ✕ simulation; a stale
        // handle simply makes PostMessage fail, which we report as false.
        unsafe { PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0) != 0 }
    }

    #[cfg(test)]
    mod tests {
        use super::{
            approved_enabled, clean_module_path, exe_from_command, friendly_program_name, has_cjk,
            is_placeholder_label, module_folder_name, parse_csv_line, split_run_path,
            task_settings_enabled, write_approved, CURRENT_VERSION,
        };
        use winreg::RegKey;

        #[test]
        fn toggle_round_trips_through_the_approved_marker() {
            // The exact pair the UI toggle relies on: write_approved stores a
            // marker, the rescan reads it back. If these ever drift, every
            // startup switch silently snaps back — the "button is dead" bug.
            let list = format!("squidTest{}", std::process::id());
            let hive = RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
            let key_path = format!(r"{CURRENT_VERSION}\Explorer\StartupApproved\{list}");
            for name in ["squid-off", "squid-on"] {
                let want = name.ends_with("on");
                if let Err(why) = write_approved(false, CURRENT_VERSION, &list, name, want) {
                    // Sandboxed CI runners deny registry writes; the marker
                    // round-trip cannot be proven here, so say so instead of
                    // failing a correct build.
                    eprintln!("skipping marker round-trip: registry write denied ({why})");
                    return;
                }
                assert_eq!(
                    approved_enabled(&hive, CURRENT_VERSION, &list, name),
                    want,
                    "{name} must read back as written"
                );
            }
            let _ = hive.delete_subkey_all(&key_path);
        }

        #[test]
        fn run_path_split_round_trips_hive_and_list() {
            // set_startup_enabled re-derives (machine, base, list) from the
            // id scan produced; a mismatch writes the marker to the wrong key
            // and the rescan then shows the old state.
            let wow_run = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run".to_string();
            let (machine, base, list, prefix32) = split_run_path("HKLM", &wow_run);
            assert!(machine);
            assert_eq!(list, "Run");
            assert!(prefix32);
            assert!(base.contains("WOW6432Node"));
            let (machine, base, list, prefix32) =
                split_run_path("HKCU", &format!(r"{CURRENT_VERSION}\RunOnce"));
            assert!(!machine);
            assert_eq!(list, "RunOnce");
            assert!(!prefix32);
            assert_eq!(base, CURRENT_VERSION);
        }

        #[test]
        fn command_paths_survive_quotes_and_arguments() {
            assert_eq!(
                exe_from_command(r#""C:\Program Files\WPS\wpscenter.exe" /update"#).as_deref(),
                Some(r"C:\Program Files\WPS\wpscenter.exe")
            );
            assert_eq!(
                exe_from_command(r"C:\Program Files\Everything\Everything.exe -startup").as_deref(),
                Some(r"C:\Program Files\Everything\Everything.exe")
            );
            assert_eq!(exe_from_command("cmd /c hello.bat"), None);
            assert_eq!(
                exe_from_command(r"C:\ProgramData\Start Menu\Programs\Startup\RustDesk Tray.lnk")
                    .as_deref(),
                Some(r"C:\ProgramData\Start Menu\Programs\Startup\RustDesk Tray.lnk")
            );
        }

        #[test]
        fn chinese_product_names_win_the_naming_priority() {
            assert!(has_cjk("腾讯电脑管家"));
            assert!(!has_cjk("Everything"));
            assert_eq!(
                friendly_program_name(
                    "QQPCTray",
                    Some("QQPCTray Application"),
                    Some("腾讯电脑管家")
                ),
                "腾讯电脑管家"
            );
            assert_eq!(
                friendly_program_name("mCloud", Some("和彩云同步盘"), None),
                "和彩云同步盘"
            );
            assert_eq!(
                friendly_program_name("edgeupdate", Some("Microsoft Edge Update"), None),
                "Microsoft Edge Update"
            );
            assert_eq!(
                friendly_program_name("weird", Some("应用框架宿主"), None),
                "应用框架宿主"
            );
        }

        #[test]
        fn csv_rows_survive_quoted_commas() {
            let fields = parse_csv_line(r#""\Vendor\Task, long","N/A","Ready","Interactive only""#);
            assert_eq!(fields[0], r"\Vendor\Task, long");
            assert_eq!(fields[2], "Ready");
        }

        #[test]
        fn task_enabled_lives_in_settings_not_triggers() {
            // A trigger-level <Enabled>false must not dim the task itself.
            let xml = "<Task><Triggers><LogonTrigger><Enabled>false</Enabled>\
                       </LogonTrigger></Triggers><Settings><Enabled>true</Enabled>\
                       </Settings></Task>";
            assert!(task_settings_enabled(xml));
            let off = "<Task><Settings><Enabled>false</Enabled></Settings></Task>";
            assert!(!task_settings_enabled(off));
        }

        #[test]
        fn long_cjk_descriptions_are_cut_to_the_product() {
            assert_eq!(
                friendly_program_name(
                    "wpscloudsvr",
                    Some("WPS服务程序，提供账号登录、云存储等服务"),
                    Some("WPS Office")
                ),
                "WPS服务程序"
            );
            assert_eq!(
                super::clip_software_name("百度网盘下载基础服务（用于快速申请磁盘空间）"),
                "百度网盘下载基础服务"
            );
            assert_eq!(super::clip_software_name("腾讯电脑管家"), "腾讯电脑管家");
            let long = "中".repeat(30);
            assert_eq!(super::clip_software_name(&long).chars().count(), 25);
        }

        #[test]
        fn template_placeholders_and_generic_strings_are_rejected() {
            assert!(is_placeholder_label("TODO:  <文件说明>"));
            assert!(is_placeholder_label("TODO: <公司名>"));
            assert!(is_placeholder_label("Windows Shell Context Menu Handler"));
            assert!(is_placeholder_label("  "));
            assert!(!is_placeholder_label("Bandizip shell menu dll"));
            assert!(!is_placeholder_label("腾讯电脑管家"));
        }

        #[test]
        fn module_fallback_names_come_from_the_product_folder() {
            assert_eq!(
                module_folder_name(r"C:\Program Files (x86)\SogouInput\Components\biz_center\1.0.0.3434\biz_shellext64.dll").as_deref(),
                Some("SogouInput")
            );
            assert_eq!(
                module_folder_name(
                    r"D:\Program Files (x86)\mcloud\processes\MSContextMenu_x64.dll"
                )
                .as_deref(),
                Some("mcloud")
            );
            assert_eq!(
                module_folder_name(r"C:\Program Files\Bandizip\bdzshl64.dll").as_deref(),
                Some("Bandizip")
            );
            assert_eq!(
                clean_module_path(r#""C:\app\bndshell.dll" -Embedding"#).as_str(),
                r"C:\app\bndshell.dll"
            );
        }

        #[test]
        fn shortcut_files_point_at_the_real_binary() {
            // The link parser checks that its target exists on disk, so this
            // needs a real file. Managed dev boxes with an endpoint protector
            // sometimes refuse scratch writes from a freshly built unsigned
            // test binary (%TEMP% and target/tmp alike, error 5); skip then
            // rather than reporting a code failure on a pure-parse test.
            let base = std::env::var_os("CARGO_TARGET_TMPDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let dir = base.join(format!("squid-lnk-{}", std::process::id()));
            if let Err(error) = std::fs::create_dir_all(&dir) {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    eprintln!("skipping: scratch dir write denied ({error})");
                    return;
                }
                panic!("scratch dir: {error}");
            }
            let target = dir.join("app.exe");
            if let Err(error) = std::fs::write(&target, b"MZ") {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    eprintln!("skipping: scratch write denied ({error})");
                    return;
                }
                panic!("target: {error}");
            }
            // Minimal MS-SHLNFF link: one idlist item (with the bogus header
            // ItemIDSize real files carry), LINKINFO with a local base path.
            let target_text = target.to_string_lossy().into_owned();
            let path_bytes = target_text.as_bytes();
            let mut lnk = vec![0u8; 0x4C];
            lnk[0..4].copy_from_slice(b"L\0\0\0");
            lnk[0x14..0x18].copy_from_slice(&0x21u32.to_le_bytes()); // idlist + linkinfo
            lnk[0x1C..0x1E].copy_from_slice(&9999u16.to_le_bytes()); // bogus ItemIDSize
            lnk.extend_from_slice(&6u16.to_le_bytes()); // one 4-byte shell id
            lnk.extend_from_slice(b"MYID");
            lnk.extend_from_slice(&0u16.to_le_bytes()); // end-of-idlist
            lnk.extend_from_slice(&((0x1C + path_bytes.len() + 1) as u32).to_le_bytes());
            lnk.extend_from_slice(&0x1Cu32.to_le_bytes()); // LinkHeaderSize
            lnk.extend_from_slice(&1u32.to_le_bytes()); // VolumeIDAndLocalBasePath
            lnk.extend_from_slice(&0u32.to_le_bytes()); // VolumeIDOffset
            lnk.extend_from_slice(&0x1Cu32.to_le_bytes()); // LocalBasePathOffset
            lnk.extend_from_slice(&[0u8; 8]); // net relative + cmdline offsets
            lnk.extend_from_slice(path_bytes);
            lnk.push(0);
            let lnk_path = dir.join("App.lnk");
            std::fs::write(&lnk_path, &lnk).expect("lnk");
            assert_eq!(
                super::lnk_target(&lnk_path.to_string_lossy()).as_deref(),
                Some(target_text.as_str())
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::StartupEntry;
    use std::path::{Path, PathBuf};

    const DISABLED_SUFFIX: &str = ".squid-disabled";

    fn agents(dir: &Path, scope: &str, label: &str) -> Vec<StartupEntry> {
        let mut out = Vec::new();
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return out;
        };
        for file in read_dir.filter_map(Result::ok) {
            let path = file.path();
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            let (stem, enabled) = if let Some(base) = name.strip_suffix(DISABLED_SUFFIX) {
                (base.to_string(), false)
            } else if name.ends_with(".plist") {
                (name.clone(), true)
            } else {
                continue;
            };
            let readout_path = if enabled {
                path.clone()
            } else {
                PathBuf::from(name.strip_suffix(DISABLED_SUFFIX).unwrap_or(&name))
            };
            let label_text = plist::Value::from_file(readout_path)
                .ok()
                .and_then(|value| {
                    let dict = value.as_dictionary()?;
                    dict.get("Label")
                        .or_else(|| dict.get("CFBundleIdentifier"))
                        .and_then(|v| v.as_string())
                        .map(String::from)
                })
                .unwrap_or(stem);
            out.push(StartupEntry {
                id: format!("launchAgent|{}", path.display()),
                name: label_text,
                command: path.display().to_string(),
                target: String::new(),
                target_exists: true,
                trigger: "logon".to_string(),
                system_item: false,
                signature: String::new(),
                signer: None,
                display_name: String::new(),
                icon: None,
                source: "launchAgent".to_string(),
                source_label: label.to_string(),
                scope: scope.to_string(),
                enabled,
                named: true,
            });
        }
        out
    }

    pub(super) fn scan_startup() -> Vec<StartupEntry> {
        let home = dirs::home_dir().unwrap_or_default();
        let mut out = agents(
            &home.join("Library/LaunchAgents"),
            "currentUser",
            "~/Library/LaunchAgents",
        );
        out.extend(agents(
            &PathBuf::from("/Library/LaunchAgents"),
            "machine",
            "/Library/LaunchAgents",
        ));
        out.extend(agents(
            &PathBuf::from("/Library/LaunchDaemons"),
            "machine",
            "/Library/LaunchDaemons",
        ));
        for entry in &mut out {
            entry.display_name = entry.name.clone();
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    pub(super) fn set_startup_enabled(entry: &StartupEntry, enabled: bool) -> Result<(), String> {
        let path = entry
            .id
            .strip_prefix("launchAgent|")
            .ok_or("未知的启动项来源")?;
        let current = PathBuf::from(path);
        let target = if enabled {
            let name = current
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(DISABLED_SUFFIX))
                .ok_or("当前已是启用状态")?;
            current.with_file_name(name)
        } else {
            PathBuf::from(format!("{path}{DISABLED_SUFFIX}"))
        };
        std::fs::rename(&current, &target).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                "需要管理员权限才能修改系统级启动项".to_string()
            } else {
                format!("{error}")
            }
        })
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::StartupEntry;
    use std::path::PathBuf;

    fn autostart_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("autostart")
    }

    pub(super) fn scan_startup() -> Vec<StartupEntry> {
        let mut out = Vec::new();
        let Ok(read_dir) = std::fs::read_dir(autostart_dir()) else {
            return out;
        };
        for file in read_dir.filter_map(Result::ok) {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut exec = String::new();
            let mut enabled = true;
            for line in text.lines() {
                if let Some(value) = line.strip_prefix("Name=") {
                    name = value.to_string();
                } else if let Some(value) = line.strip_prefix("Exec=") {
                    exec = value.to_string();
                } else if line == "X-GNOME-Autostart-enabled=false" {
                    enabled = false;
                }
            }
            out.push(StartupEntry {
                id: format!("autostart|{}", path.display()),
                name,
                command: exec,
                target: String::new(),
                target_exists: true,
                trigger: "logon".to_string(),
                system_item: false,
                signature: String::new(),
                signer: None,
                display_name: String::new(),
                icon: None,
                source: "autostart".to_string(),
                source_label: "~/.config/autostart".to_string(),
                scope: "currentUser".to_string(),
                enabled,
                named: true,
            });
        }
        for entry in &mut out {
            entry.display_name = entry.name.clone();
        }
        out
    }

    pub(super) fn set_startup_enabled(entry: &StartupEntry, enabled: bool) -> Result<(), String> {
        let path = entry
            .id
            .strip_prefix("autostart|")
            .ok_or("未知的启动项来源")?;
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let mut lines: Vec<String> = text
            .lines()
            .filter(|line| !line.starts_with("X-GNOME-Autostart-enabled="))
            .map(str::to_string)
            .collect();
        if !enabled {
            lines.push("X-GNOME-Autostart-enabled=false".to_string());
        }
        std::fs::write(path, lines.join("\n")).map_err(|error| error.to_string())
    }
}
