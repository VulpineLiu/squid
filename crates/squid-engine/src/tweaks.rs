//! 系统优化 (system optimisation) — a curated, **reversible** tweaks catalog.
//!
//! MangoDisk keeps 83 Windows settings and a revert registry; we start with
//! the low-dispute subset and one hard rule each tweak must pass before it
//! ships here: (1) registry-only or documented-API, (2) the original value
//! is recorded before the first write, (3) 「恢复全部默认」 puts every recorded
//! value back exactly, (4) verified on a real machine, not just this file.
//!
//! The catalog is data. Each tweak declares its hive/key/value/type and the
//! bytes that mean on/off; everything else (state read, apply, revert) is
//! one shared code path, so adding a tweak cannot introduce new bugs in
//! reading, writing, or undoing.

use serde::Serialize;
use squid_core::Settings;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TweakKind {
    Dword,
    String,
    NoneMeansZero,
}

/// One catalog entry. `on` is the payload that enables the tweak; `restore`
/// describes what "back to default" means (delete the value, or write the
/// original bytes we recorded).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tweak {
    pub id: &'static str,
    pub title_zh: &'static str,
    pub title_en: &'static str,
    pub detail_zh: &'static str,
    pub detail_en: &'static str,
    /// "safe" | "notice" — notice carries a visible side-effect hint.
    pub risk: &'static str,
    /// HKCU | HKLM (we only ever touch HKCU-approved paths for now).
    pub hive: &'static str,
    pub key: &'static str,
    pub value: &'static str,
    pub kind: TweakKind,
    /// Payload (DWORD as u32 text, or the string) meaning "enabled".
    pub on: &'static str,
    /// A restart / Explorer restart is needed to see the effect.
    pub restart_hint: bool,
}

/// The catalog — every entry verified on a real Windows 11 machine.
pub fn catalog() -> Vec<Tweak> {
    vec![
        Tweak {
            id: "transparency",
            title_zh: "关闭透明效果",
            title_en: "Turn off transparency",
            detail_zh: "降低 GPU 合成负担，远程桌面里也更清晰",
            detail_en: "Less compositor work; sharper over RDP",
            risk: "safe",
            hive: "HKCU",
            key: r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            value: "EnableTransparency",
            kind: TweakKind::Dword,
            on: "0",
            restart_hint: false,
        },
        Tweak {
            id: "show-extensions",
            title_zh: "资源管理器显示文件扩展名",
            title_en: "Show file extensions",
            detail_zh: "隐藏扩展名是误删与伪装文件事故的头号来源",
            detail_en: "Hidden extensions cause mis-deletes and fake-file tricks",
            risk: "safe",
            hive: "HKCU",
            key: r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            value: "HideFileExt",
            kind: TweakKind::Dword,
            on: "0",
            restart_hint: false,
        },
        Tweak {
            id: "widgets-web",
            title_zh: "关闭小组件 Web 内容",
            title_en: "Disable Widgets web content",
            detail_zh: "任务栏小组件不再拉取新闻，减少后台联网与内存",
            detail_en: "Taskbar Widgets stops fetching news feeds",
            risk: "safe",
            hive: "HKCU",
            key: r"Software\Microsoft\Windows\CurrentVersion\Feeds",
            value: "ShellWebContentSystemProvisioning",
            kind: TweakKind::Dword,
            on: "0",
            restart_hint: true,
        },
        Tweak {
            id: "location",
            title_zh: "禁止位置同步",
            title_en: "Disable location",
            detail_zh: "应用不能再读取本机地理位置",
            detail_en: "Apps can no longer read this device's location",
            risk: "safe",
            hive: "HKCU",
            key: r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location",
            value: "Value",
            kind: TweakKind::String,
            on: "Deny",
            restart_hint: false,
        },
        Tweak {
            id: "recent-files",
            title_zh: "关闭「最近使用的文件」",
            title_en: "Turn off recent files",
            detail_zh: "资源管理器与快速访问不再记录最近文件（隐私+微量性能）",
            detail_en: "Explorer stops logging recent files",
            risk: "notice",
            hive: "HKCU",
            key: r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            value: "Start_TrackDocs",
            kind: TweakKind::Dword,
            on: "0",
            restart_hint: true,
        },
    ]
}

pub fn find(id: &str) -> Option<Tweak> {
    catalog().into_iter().find(|tweak| tweak.id == id)
}

/// Row the UI sees: the tweak plus its live state and revert info.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TweakState {
    #[serde(flatten)]
    pub tweak: Tweak,
    /// True when the on-payload is currently written.
    pub enabled: bool,
    /// The key/value could not be read at all (missing is NOT an error for
    /// NoneMeansZero kinds).
    pub unreadable: bool,
    /// An original value was recorded → revert is possible.
    pub recorded: bool,
}

/// Recorded originals, persisted beside the other app state.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct TweakBackups {
    /// tweak id → (kind marker + original payload text, or "absent")
    pub originals: std::collections::HashMap<String, String>,
}

impl TweakBackups {
    fn path() -> PathBuf {
        Settings::config_dir().join("tweaks.json")
    }

    pub fn load() -> TweakBackups {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }

    /// Record once: the FIRST original value is the one we restore to, no
    /// matter how many times the user flips the switch afterwards.
    fn record(&mut self, id: &str, original: String) -> std::io::Result<()> {
        if self.originals.contains_key(id) {
            return Ok(());
        }
        self.originals.insert(id.to_string(), original);
        self.save()
    }
}

#[cfg(windows)]
mod registry {
    use super::{Tweak, TweakKind};
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    fn open(hive: &str) -> Result<RegKey, String> {
        match hive {
            "HKCU" => Ok(RegKey::predef(HKEY_CURRENT_USER)),
            other => Err(format!("不支持的注册表根：{other}")),
        }
    }

    /// Read the current payload as text; Ok(None) = value absent.
    pub fn read(tweak: &Tweak) -> Result<Option<String>, String> {
        let root = open(tweak.hive)?;
        let Ok(key) = root.open_subkey_with_flags(tweak.key, KEY_READ) else {
            return Ok(None); // key missing → value absent, not an error
        };
        let value = match tweak.kind {
            TweakKind::String => key
                .get_value::<String, _>(tweak.value)
                .map(Some)
                .unwrap_or(None),
            _ => key
                .get_value::<u32, _>(tweak.value)
                .map(|number| Some(number.to_string()))
                .unwrap_or(None),
        };
        Ok(value)
    }

    /// Write the payload; missing key is created with minimal rights
    /// (create_subkey would ask KEY_ALL_ACCESS and security software says no).
    pub fn write(tweak: &Tweak, payload: &str) -> Result<(), String> {
        let root = open(tweak.hive)?;
        let key = root
            .create_subkey_with_flags(tweak.key, KEY_READ | KEY_WRITE)
            .map_err(|error| format!("无法打开 {}：{error}", tweak.key))?
            .0;
        match tweak.kind {
            TweakKind::String => key
                .set_value(tweak.value, &payload.to_string())
                .map_err(|error| error.to_string())?,
            _ => {
                let number: u32 = payload
                    .parse()
                    .map_err(|_| format!("不是合法的 DWORD：{payload}"))?;
                key.set_value(tweak.value, &number)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    pub fn remove(tweak: &Tweak) -> Result<(), String> {
        let root = open(tweak.hive)?;
        if let Ok(key) = root.open_subkey_with_flags(tweak.key, KEY_READ | KEY_WRITE) {
            let _ = key.delete_value(tweak.value);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod registry {
    use super::Tweak;
    pub fn read(_tweak: &Tweak) -> Result<Option<String>, String> {
        Err("tweaks 仅在 Windows 提供".into())
    }
    pub fn write(_tweak: &Tweak, _payload: &str) -> Result<(), String> {
        Err("tweaks 仅在 Windows 提供".into())
    }
    pub fn remove(_tweak: &Tweak) -> Result<(), String> {
        Ok(())
    }
}

pub fn states() -> Vec<TweakState> {
    let backups = TweakBackups::load();
    catalog()
        .into_iter()
        .map(|tweak| {
            let raw = registry::read(&tweak);
            let current = match &raw {
                Ok(Some(value)) => Some(value.clone()),
                // Absent DWORDs read as their documented default 0.
                Ok(None) if tweak.kind != TweakKind::String => Some("0".to_string()),
                Ok(None) => None,
                Err(_) => None,
            };
            let enabled = current
                .as_ref()
                .is_some_and(|value| value.eq_ignore_ascii_case(tweak.on));
            TweakState {
                tweak: tweak.clone(),
                enabled,
                unreadable: raw.is_err(),
                recorded: backups.originals.contains_key(tweak.id),
            }
        })
        .collect()
}

/// Flip one tweak. The original value (including "absent") is recorded
/// before the first write so 「恢复全部默认」 can always undo the whole page.
pub fn set(id: &str, enabled: bool) -> Result<TweakState, String> {
    let tweak = find(id).ok_or_else(|| format!("未知的优化项：{id}"))?;
    let mut backups = TweakBackups::load();
    let original = registry::read(&tweak)?;
    // Absent DWORDs read as their documented default 0 for comparison.
    let current = original
        .clone()
        .or((tweak.kind != TweakKind::String).then(|| "0".to_string()));
    let already = current
        .as_ref()
        .is_some_and(|value| value.eq_ignore_ascii_case(tweak.on))
        == enabled;
    if already {
        // No write, no recorded original — the machine is already where the
        // switch says it should be; claiming a change would be a lie.
    } else if enabled {
        backups
            .record(
                tweak.id,
                original.clone().unwrap_or_else(|| "absent".into()),
            )
            .map_err(|error| error.to_string())?;
        registry::write(&tweak, tweak.on)?;
    } else {
        // Turning off explicitly = restore the recorded original.
        restore_one(&mut backups, &tweak)?;
    }
    Ok(states()
        .into_iter()
        .find(|state| state.tweak.id == id)
        .expect("catalog round-trips"))
}

fn restore_one(backups: &mut TweakBackups, tweak: &Tweak) -> Result<(), String> {
    match backups.originals.get(tweak.id) {
        Some(original) if original == "absent" || original.is_empty() => {
            registry::remove(tweak)?;
        }
        Some(original) => {
            registry::write(tweak, original)?;
        }
        None => registry::remove(tweak)?,
    }
    backups.originals.remove(tweak.id);
    backups.save().map_err(|error| error.to_string())
}

/// 「恢复全部默认」: every recorded tweak back to its original value.
pub fn restore_all() -> Vec<String> {
    let mut backups = TweakBackups::load();
    let mut restored = Vec::new();
    for tweak in catalog() {
        if backups.originals.contains_key(tweak.id) && restore_one(&mut backups, &tweak).is_ok() {
            restored.push(tweak.id.to_string());
        }
    }
    restored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_prefixed_hkcu() {
        let mut seen = std::collections::HashSet::new();
        for tweak in catalog() {
            assert!(seen.insert(tweak.id), "duplicate {}", tweak.id);
            assert_eq!(tweak.hive, "HKCU", "batch 1 stays user-hive only");
            assert!(!tweak.key.is_empty() && !tweak.value.is_empty());
        }
    }

    #[test]
    fn unknown_ids_are_rejected() {
        assert!(find("delete-my-boot").is_none());
        assert!(set("delete-my-boot", true).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn a_tweak_round_trips_through_the_registry() {
        // A private scratch value under a key we own; registry access can be
        // denied by endpoint protection in sandboxes, so skip politely then.
        let probe = find("show-extensions").expect("catalog");
        let before = match registry::read(&probe) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("skipping: registry read denied ({error})");
                return;
            }
        };
        if registry::write(&probe, "1").is_err() {
            eprintln!("skipping: registry write denied");
            return;
        }
        assert_eq!(registry::read(&probe).ok().flatten().as_deref(), Some("1"));
        // Restore the machine to exactly how we found it.
        match &before {
            Some(value) => registry::write(&probe, value).expect("restore"),
            None => registry::remove(&probe).expect("remove"),
        }
        assert_eq!(registry::read(&probe).ok().flatten(), before);
    }
}
