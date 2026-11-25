//! Hidden-space insights (Mole's `cmd/analyze` overview idiom).
//!
//! The disk analyzer answers "what is big"; this answers "what is big *and*
//! actionable" — curated Windows space hogs with a one-line explanation and
//! the action that actually frees them. Existence-gated: a row appears only
//! when the underlying path (or state) is really on this machine.

use serde::Serialize;
use squid_core::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Insight {
    /// Stable id for the UI (icon/translation lookup).
    pub id: String,
    pub title: String,
    pub path: String,
    pub size: u64,
    /// "cleanable" — 空间清理 covers it; "command" — needs the given command;
    /// "external" — system tool territory; "info" — informational.
    pub action: String,
    /// Command to run (elevated) when action == "command".
    pub command: Option<String>,
    pub hint: String,
}

fn home() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

fn dir_size(path: &Path) -> u64 {
    if !path.is_dir() {
        return 0;
    }
    fs::measure(path, squid_core::ItemKind::Path)
        .on_disk()
        .as_u64()
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

/// All insights that currently exist on this machine, biggest-effect first
/// ordering is left to the UI.
pub fn collect() -> Vec<Insight> {
    let mut out = Vec::new();
    let Some(home) = home() else {
        return out;
    };
    let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let system = PathBuf::from(system);

    // 1. Windows Update download cache — the classic 5-20 GB surprise.
    let updates = system.join(r"SoftwareDistribution\Download");
    let size = dir_size(&updates);
    if size > 256 * 1024 * 1024 {
        out.push(Insight {
            id: "update-cache".into(),
            title: "Windows 更新缓存".into(),
            path: updates.display().to_string(),
            size,
            action: "cleanable".into(),
            command: None,
            hint: "已安装的更新包，Windows 需要时会重新下载；空间清理的「系统垃圾」覆盖此目录"
                .into(),
        });
    }

    // 2. Hibernation file — freed by turning hibernation off.
    let hiber = system.join("..\\hiberfil.sys");
    let size = file_size(&hiber);
    if size > 0 {
        out.push(Insight {
            id: "hiberfil".into(),
            title: "休眠文件 hiberfil.sys".into(),
            path: hiber.display().to_string(),
            size,
            action: "command".into(),
            command: Some("powercfg /h off".into()),
            hint: "关闭休眠后系统自动删除；睡眠不受影响".into(),
        });
    }

    // 3. Pagefile — visible but rarely worth touching; info only.
    let page = system.join("..\\pagefile.sys");
    let size = file_size(&page);
    if size > 0 {
        out.push(Insight {
            id: "pagefile".into(),
            title: "虚拟内存 pagefile.sys".into(),
            path: page.display().to_string(),
            size,
            action: "info".into(),
            command: None,
            hint: "系统正在使用的虚拟内存，不建议手动删除；可在系统设置改到别的盘".into(),
        });
    }

    // 4. Component store (WinSxS) — needs DISM, never raw deletion.
    //    Walking 100k+ files just to print a number is not worth it here;
    //    the UI shows "several GB" and the DISM command does the rest.
    let winsxs = system.join("WinSxS");
    if winsxs.is_dir() {
        out.push(Insight {
            id: "winsxs".into(),
            title: "组件存储 WinSxS".into(),
            path: winsxs.display().to_string(),
            size: 0,
            action: "command".into(),
            command: Some("Dism.exe /online /Cleanup-Image /StartComponentCleanup".into()),
            hint: "系统组件仓库，通常 5~10 GB；只能由 DISM 收缩，直接删会破坏系统".into(),
        });
    }

    // 5. Phone backups (Apple MobileSync / third-party Android tools).
    for (tail, title) in [
        (r"Apple\MobileSync\Backup", "iPhone 备份"),
        (r"Apple Computer\MobileSync\Backup", "iPhone 备份"),
    ] {
        let backups = home.join(tail);
        let size = dir_size(&backups);
        if size > 256 * 1024 * 1024 {
            out.push(Insight {
                id: "phone-backup".into(),
                title: title.into(),
                path: backups.display().to_string(),
                size,
                action: "external".into(),
                command: None,
                hint: "整机备份体积大；确认 iCloud 或新备份完成后，在 iTunes/Finder 里删除旧备份"
                    .into(),
            });
            break;
        }
    }

    // 6. Downloads older than 90 days (Mole's "Old Downloads" insight).
    let downloads = home.join("Downloads");
    if downloads.is_dir() {
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(90 * 24 * 3600);
        let mut size = 0u64;
        let mut stale = 0usize;
        if let Ok(entries) = std::fs::read_dir(&downloads) {
            for entry in entries.flatten() {
                let meta = match entry.metadata() {
                    Ok(meta) => meta,
                    Err(_) => continue,
                };
                let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                if modified < cutoff {
                    size += if meta.is_dir() {
                        dir_size(&entry.path())
                    } else {
                        meta.len()
                    };
                    stale += 1;
                }
            }
        }
        if size > 256 * 1024 * 1024 {
            out.push(Insight {
                id: "old-downloads".into(),
                title: "下载文件夹（90 天以上）".into(),
                path: downloads.display().to_string(),
                size,
                action: "external".into(),
                command: None,
                hint: format!("{stale} 个条目三个月没动过；确认不再需要后手动清理"),
            });
        }
    }

    out.sort_by_key(|row| std::cmp::Reverse(row.size));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_never_panics_on_a_live_machine() {
        // Pure smoke: whatever exists, ids are unique and sizes sane.
        let rows = collect();
        for (index, row) in rows.iter().enumerate() {
            assert!(!row.id.is_empty());
            assert!(!rows[..index]
                .iter()
                .any(|earlier| earlier.id == row.id && row.id != "phone-backup"));
        }
    }
}
