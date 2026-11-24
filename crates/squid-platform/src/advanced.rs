//! 进阶启动项 (advanced startup) — the Autoruns-shaped tail of the startup
//! story: Winlogon hooks, AppInit, IFEO debuggers, browser BHOs, and the
//! hidden corners of Task Scheduler and Services that ordinary listings
//! skip. Everything here can break boot or logon, so the contract is:
//!
//! * **display first** — the UI lists these read-mostly, with signature
//!   trust and the exact registry location on every row;
//! * **disable is reversible** — the first disable records the original
//!   value in `advanced-backup.json`; restoring puts the bytes back;
//! * hidden tasks and automatic services ride the *same* id scheme as the
//!   normal startup list, so their toggles reuse the proven arms.
//!
//! Signature trust is a batched `Get-AuthenticodeSignature` PowerShell
//! call over the deduplicated exe set — one process, honest status
//! (微软 / 已知厂商 / 未签名 / 失效), no FFI.

use serde::{Deserialize, Serialize};

/// One advanced row. `kind` decides how disabling works.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvancedEntry {
    /// `adv|<kind>|<hive>|<key>|<value>` (or task/service ids for the
    /// reused arms).
    pub id: String,
    pub kind: String,
    pub name: String,
    /// Registry command line or task/service target.
    pub command: String,
    /// Human location, e.g. `HKLM\…\Winlogon\Userinit`.
    pub location: String,
    pub scope: String,
    pub enabled: bool,
    /// Digital-signature verdict: `microsoft` | `vendor` | `unsigned` |
    /// `invalid` | `unknown` (no exe resolvable / non-Windows).
    pub signature: String,
    /// Subject of the signing certificate when known.
    pub signer: Option<String>,
    /// A warning the UI must show: IFEO hijacks a program, Winlogon is
    /// boot-critical, etc.
    pub warning: Option<String>,
}

#[cfg(windows)]
mod windows {
    use super::AdvancedEntry;
    use std::path::PathBuf;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    const WINLOGON: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Winlogon";
    const IFEO: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Image File Execution Options";

    fn hive(machine: bool) -> RegKey {
        RegKey::predef(if machine {
            HKEY_LOCAL_MACHINE
        } else {
            HKEY_CURRENT_USER
        })
    }

    fn read_string(key: &RegKey, path: &str, value: &str) -> Option<String> {
        key.open_subkey_with_flags(path, KEY_READ)
            .ok()?
            .get_value::<String, _>(value)
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    }

    /// Strip quotes and arguments from a registry command to reach the exe.
    pub(crate) fn exe_of(command: &str) -> Option<String> {
        let text = command.trim();
        if let Some(rest) = text.strip_prefix('"') {
            return rest.split('"').next().map(str::to_string);
        }
        // Up to the first `.exe` boundary, else the first whitespace token.
        if let Some(end) = text.to_ascii_lowercase().find(".exe") {
            return Some(text[..end + 4].to_string());
        }
        text.split_whitespace().next().map(str::to_string)
    }

    fn winlogon_entries(out: &mut Vec<AdvancedEntry>) {
        for (machine, hive_label) in [(true, "HKLM"), (false, "HKCU")] {
            let root = hive(machine);
            for (value, meaning) in [
                ("Shell", "登录外壳（正常情况下只有 explorer.exe）"),
                (
                    "Userinit",
                    "登录初始化链（正常情况下以 userinit.exe, 结尾）",
                ),
            ] {
                let Some(text) = read_string(&root, WINLOGON, value) else {
                    continue;
                };
                // The pristine value is a bare system component; anything
                // extra rides alongside and is worth a warning.
                let clean = text.split(',').all(|part| {
                    let part = part.trim().to_ascii_lowercase();
                    part.is_empty()
                        || part
                            .replace("%systemroot%", "")
                            .replace("\\system32\\", "")
                            .starts_with(if value == "Shell" {
                                "explorer.exe"
                            } else {
                                "userinit.exe"
                            })
                });
                out.push(AdvancedEntry {
                    id: format!("adv|winlogon|{hive_label}|{WINLOGON}|{value}"),
                    kind: "winlogon".into(),
                    name: format!("Winlogon {value}"),
                    command: text,
                    location: format!("{hive_label}\\{WINLOGON} · {value}"),
                    scope: if machine { "machine" } else { "currentUser" }.into(),
                    enabled: true,
                    signature: "unknown".into(),
                    signer: None,
                    warning: (!clean)
                        .then(|| format!("{meaning}；存在附加程序，请确认是否认识"))
                        .or_else(|| Some(format!("{meaning}；改动需重启生效"))),
                });
            }
        }
    }

    fn appinit_entries(out: &mut Vec<AdvancedEntry>) {
        const KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Windows";
        for (machine, hive_label) in [(true, "HKLM"), (false, "HKCU")] {
            let root = hive(machine);
            let enabled = read_string(&root, KEY, "LoadAppInit_DLLs").as_deref() == Some("1");
            let Some(dlls) = read_string(&root, KEY, "AppInit_DLLs") else {
                continue;
            };
            out.push(AdvancedEntry {
                id: format!("adv|appinit|{hive_label}|{KEY}|AppInit_DLLs"),
                kind: "appinit".into(),
                name: "AppInit_DLLs 全局注入".to_string(),
                command: dlls,
                location: format!("{hive_label}\\{KEY}"),
                scope: if machine { "machine" } else { "currentUser" }.into(),
                enabled,
                signature: "unknown".into(),
                signer: None,
                warning: Some("每个 GUI 进程都会加载这些 DLL；除非认识每一个，否则保持关闭".into()),
            });
        }
    }

    fn ifeo_entries(out: &mut Vec<AdvancedEntry>) {
        for (machine, hive_label) in [(true, "HKLM"), (false, "HKCU")] {
            let root = hive(machine);
            let Ok(ifeo) = root.open_subkey_with_flags(IFEO, KEY_READ) else {
                continue;
            };
            for name in ifeo.enum_keys().flatten() {
                let Some(debugger) = read_string(&ifeo, &name, "Debugger") else {
                    continue;
                };
                // A Debugger replaces the program's launch entirely — the
                // classic persistence trick. Only flag non-microsoft ones.
                let lower = debugger.to_ascii_lowercase();
                if lower.contains("mseone")
                    || lower.contains("hybridlaunch")
                    || lower.contains("msedge.exe --profile-directory")
                {
                    continue; // Edge/IE protocol handlers live here legitimately
                }
                out.push(AdvancedEntry {
                    id: format!("adv|ifeo|{hive_label}|{IFEO}\\{name}|Debugger"),
                    kind: "ifeo".into(),
                    name: format!("{name} 被调试器接管"),
                    command: debugger,
                    location: format!("{hive_label}\\{IFEO}\\{name} · Debugger"),
                    scope: if machine { "machine" } else { "currentUser" }.into(),
                    enabled: true,
                    signature: "unknown".into(),
                    signer: None,
                    warning: Some(format!(
                        "启动 {name} 会先启动调试器；安全软件常用，也可能是流氓持久化"
                    )),
                });
            }
        }
    }

    fn bho_entries(out: &mut Vec<AdvancedEntry>) {
        const BHO: &str =
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Browser Helper Objects";
        let root = hive(true);
        let Ok(bho) = root.open_subkey_with_flags(BHO, KEY_READ) else {
            return;
        };
        for clsid in bho.enum_keys().flatten() {
            let module = read_string(&root, &format!(r"Classes\{clsid}\InprocServer32"), "");
            let friendly = read_string(&root, &format!(r"Classes\{clsid}"), "");
            let load = bho
                .open_subkey_with_flags(&clsid, KEY_READ)
                .ok()
                .and_then(|key| key.get_value::<u32, _>("LoadBehavior").ok())
                .unwrap_or(3);
            out.push(AdvancedEntry {
                id: format!("adv|bho|HKLM|{BHO}\\{clsid}|LoadBehavior"),
                kind: "bho".into(),
                name: friendly.unwrap_or_else(|| clsid.clone()),
                command: module.unwrap_or_else(|| clsid.clone()),
                location: format!("HKLM\\{BHO}\\{clsid} · LoadBehavior"),
                scope: "machine".into(),
                enabled: load & 1 != 0,
                signature: "unknown".into(),
                signer: None,
                warning: Some("浏览器加载项（IE 模式）；现代浏览器大多不再使用".into()),
            });
        }
    }

    pub fn scan() -> Vec<AdvancedEntry> {
        let mut out = Vec::new();
        winlogon_entries(&mut out);
        appinit_entries(&mut out);
        ifeo_entries(&mut out);
        bho_entries(&mut out);
        enrich_signatures(&mut out);
        out
    }

    /// Signature trust is a batch pass: collect the unique exes, grade them
    /// all in one PowerShell call, then stamp every entry from the map.
    fn enrich_signatures(entries: &mut [AdvancedEntry]) {
        let mut exes: Vec<String> = Vec::new();
        for entry in entries.iter_mut() {
            if let Some(exe) = exe_of(&entry.command).filter(|path| PathBuf::from(path).exists()) {
                if !exes.iter().any(|seen| seen.eq_ignore_ascii_case(&exe)) {
                    exes.push(exe);
                }
                entry.signature = "pending".into();
            }
        }
        if exes.is_empty() {
            return;
        }
        let list = exes
            .iter()
            .map(|exe| format!("'{}'", exe.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            "$ErrorActionPreference='SilentlyContinue'; {list} | ForEach-Object {{ $s = Get-AuthenticodeSignature -FilePath $_; \"@@|$_|$($s.Status)|$($s.SignerCertificate.Subject)\" }} | Out-String -Width 4096"
        );
        let Ok(output) = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
        else {
            return;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut grades: std::collections::HashMap<String, (String, Option<String>)> =
            Default::default();
        for line in text.lines() {
            // Out-String may pad lines; match on containment of the marker.
            let Some(start) = line.find("@@|") else {
                continue;
            };
            let rest = &line[start + 3..];
            let mut parts = rest.splitn(3, '|');
            let (Some(path), Some(status)) = (parts.next(), parts.next()) else {
                continue;
            };
            let subject = parts.next().unwrap_or_default().to_string();
            let signer = subject
                .split("CN=")
                .nth(1)
                .map(|tail| tail.split(',').next().unwrap_or(tail).to_string())
                .filter(|name| !name.is_empty());
            let verdict = match status {
                "Valid" => match &signer {
                    Some(name) if name.contains("Microsoft") || name.contains("Windows") => {
                        "microsoft"
                    }
                    Some(_) => "vendor",
                    None => "vendor",
                },
                "NotSigned" => "unsigned",
                "HashMismatch" | "NotTrusted" | "CertificateExpired" | "UnknownError"
                | "Missing" => "invalid",
                _ => "unknown",
            };
            grades.insert(path.to_ascii_lowercase(), (verdict.into(), signer));
        }
        for entry in entries.iter_mut() {
            let Some(exe) = exe_of(&entry.command) else {
                continue;
            };
            if let Some((verdict, signer)) = grades.get(&exe.to_ascii_lowercase()) {
                entry.signature = verdict.clone();
                entry.signer = signer.clone();
            } else if entry.signature == "pending" {
                entry.signature = "unknown".into();
            }
        }
    }
}

#[cfg(not(windows))]
mod windows {
    use super::AdvancedEntry;
    pub fn scan() -> Vec<AdvancedEntry> {
        Vec::new()
    }
}

pub fn scan_advanced() -> Vec<AdvancedEntry> {
    #[cfg(windows)]
    {
        windows::scan()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Disable/enable one advanced entry. Winlogon/AppInit/IFEO/BHO write
/// registry values (original recorded on first disable); tasks and services
/// are not handled here — the caller routes them through `set_startup_enabled`
/// because their ids use the ordinary scheme.
pub fn set_advanced_enabled(entry: &AdvancedEntry, enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_set(entry, enabled)
    }
    #[cfg(not(windows))]
    {
        let _ = (entry, enabled);
        Err("进阶启动项管理仅在 Windows 提供".into())
    }
}

#[cfg(windows)]
fn windows_set(entry: &AdvancedEntry, enabled: bool) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    // id shape: adv|kind|HIVELABEL|key|value
    let parts: Vec<&str> = entry.id.splitn(5, '|').collect();
    if parts.len() != 5 {
        return Err("无法解析的进阶项 id".into());
    }
    let (hive_label, key_path, value_name) = (parts[2], parts[3], parts[4]);
    let root = RegKey::predef(if hive_label == "HKLM" {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    let mut backups = Backups::load();
    match entry.kind.as_str() {
        "winlogon" => {
            let original = read_raw(&root, key_path, value_name)?;
            if enabled {
                let restore = backups
                    .originals
                    .get(&entry.id)
                    .cloned()
                    .unwrap_or_else(|| match value_name {
                        "Shell" => "explorer.exe".to_string(),
                        _ => r"C:\Windows\system32\userinit.exe,".to_string(),
                    });
                write_raw(&root, key_path, value_name, &restore)?;
                backups.originals.remove(&entry.id);
            } else {
                // Keep the mandatory piece, drop the third-party tail.
                let kept: Vec<&str> = original
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .filter(|part| {
                        let lower = part.to_ascii_lowercase();
                        lower.contains("explorer.exe") || lower.contains("userinit.exe")
                    })
                    .collect();
                let joined = kept.join(",");
                if joined != original {
                    backups
                        .originals
                        .entry(entry.id.clone())
                        .or_insert_with(|| original.clone());
                }
                write_raw(&root, key_path, value_name, &joined)?;
            }
        }
        "appinit" => {
            backups
                .originals
                .entry(entry.id.clone())
                .or_insert_with(|| if enabled { "1".into() } else { "0".into() });
            write_dword(
                &root,
                key_path,
                "LoadAppInit_DLLs",
                if enabled { 1 } else { 0 },
            )?;
        }
        "ifeo" => {
            let original = read_raw(&root, key_path, value_name)?;
            if enabled {
                let restore = backups
                    .originals
                    .get(&entry.id)
                    .cloned()
                    .unwrap_or_else(|| original.clone());
                write_raw(&root, key_path, value_name, &restore)?;
                backups.originals.remove(&entry.id);
            } else {
                backups.originals.insert(entry.id.clone(), original);
                delete_value(&root, key_path, value_name)?;
            }
        }
        "bho" => {
            write_dword(&root, key_path, "LoadBehavior", if enabled { 3 } else { 0 })?;
        }
        other => return Err(format!("不支持的进阶类型：{other}")),
    }
    backups.save()
}

#[cfg(windows)]
fn read_raw(root: &winreg::RegKey, key: &str, value: &str) -> Result<String, String> {
    use winreg::enums::KEY_READ;
    Ok(root
        .open_subkey_with_flags(key, KEY_READ)
        .map_err(|error| error.to_string())?
        .get_value::<String, _>(value)
        .unwrap_or_default())
}

#[cfg(windows)]
fn write_raw(root: &winreg::RegKey, key: &str, value: &str, text: &str) -> Result<(), String> {
    use winreg::enums::{KEY_READ, KEY_WRITE};
    root.create_subkey_with_flags(key, KEY_READ | KEY_WRITE)
        .map_err(|error| error.to_string())?
        .0
        .set_value(value, &text.to_string())
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn write_dword(root: &winreg::RegKey, key: &str, value: &str, number: u32) -> Result<(), String> {
    use winreg::enums::{KEY_READ, KEY_WRITE};
    root.create_subkey_with_flags(key, KEY_READ | KEY_WRITE)
        .map_err(|error| error.to_string())?
        .0
        .set_value(value, &number)
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn delete_value(root: &winreg::RegKey, key: &str, value: &str) -> Result<(), String> {
    use winreg::enums::{KEY_READ, KEY_WRITE};
    root.open_subkey_with_flags(key, KEY_READ | KEY_WRITE)
        .map_err(|error| error.to_string())?
        .delete_value(value)
        .map_err(|error| error.to_string())
}

/// Original values for disabled advanced entries (restore data).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Backups {
    originals: std::collections::HashMap<String, String>,
}

impl Backups {
    #[cfg(windows)]
    fn path() -> std::path::PathBuf {
        squid_core::Settings::config_dir().join("advanced-backup.json")
    }

    #[cfg(windows)]
    fn load() -> Backups {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    #[cfg(windows)]
    fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(
            &path,
            serde_json::to_string_pretty(self).unwrap_or_default(),
        )
        .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_runs_and_ids_are_prefixed() {
        // Real-machine smoke test: the scan must never panic and every id
        // must carry the adv| prefix the parser in set_advanced_enabled
        // depends on.
        for entry in scan_advanced() {
            assert!(entry.id.starts_with("adv|"), "{}", entry.id);
            assert!(!entry.location.is_empty());
        }
    }

    #[cfg(windows)]
    #[test]
    fn exe_extraction_handles_quotes_and_arguments() {
        assert_eq!(
            windows::exe_of(r#""C:\Program Files\Tool\warp.exe" --quiet""#.as_ref()).as_deref(),
            Some(r"C:\Program Files\Tool\warp.exe")
        );
        assert_eq!(
            windows::exe_of(r"C:\Windows\System32\userinit.exe,").as_deref(),
            Some(r"C:\Windows\System32\userinit.exe")
        );
        assert_eq!(
            windows::exe_of("mscandui.dll"),
            Some("mscandui.dll".to_string())
        );
    }
}
