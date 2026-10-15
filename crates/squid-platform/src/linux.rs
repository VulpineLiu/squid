//! Linux adapter: `.desktop` entries, Flatpak/Snap awareness, XDG leftovers.

use crate::{AppSource, LeftoverRoot, LifecycleReport, Naming, Platform, PlatformId, TrashTarget};
use squid_core::progress::CancelToken;
use squid_core::{AppInfo, Bytes, Category, Result, Volume};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Default)]
pub struct Linux;

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/root"))
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// One parsed `.desktop` file.
#[derive(Default, Clone)]
struct DesktopEntry {
    name: String,
    exec: Option<String>,
    startup_wm_class: Option<String>,
    icon: Option<String>,
    no_display: bool,
}

fn parse_desktop(path: &Path) -> Option<DesktopEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut entry = DesktopEntry::default();
    let mut in_desktop = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_desktop = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Name" if entry.name.is_empty() => entry.name = value.trim().to_string(),
            "Exec" => entry.exec = Some(value.trim().to_string()),
            "StartupWMClass" => entry.startup_wm_class = Some(value.trim().to_string()),
            "Icon" => entry.icon = Some(value.trim().to_string()),
            "NoDisplay" => entry.no_display = value.trim().eq_ignore_ascii_case("true"),
            _ => {}
        }
    }
    if entry.name.is_empty() {
        return None;
    }
    Some(entry)
}

impl Platform for Linux {
    fn id(&self) -> PlatformId {
        PlatformId::Linux
    }

    fn display_name(&self) -> String {
        "Linux".to_string()
    }

    fn os_version(&self) -> String {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find(|l| l.starts_with("PRETTY_NAME="))
                    .map(|l| {
                        l.trim_end_matches('"')
                            .trim_start_matches("PRETTY_NAME=\"")
                            .to_string()
                    })
            })
            .unwrap_or_else(|| format!("Linux {}", std::env::consts::ARCH))
    }

    fn hardware_model(&self) -> String {
        let read = |p: &str| {
            std::fs::read_to_string(p)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let product = read("/sys/class/dmi/id/product_name").unwrap_or_default();
        let cpu = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|info| {
                info.lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| l.split_once(':'))
                    .map(|(_, v)| v.trim().to_string())
            })
            .unwrap_or_default();
        [product, cpu]
            .join(" · ")
            .trim_matches('·')
            .trim()
            .to_string()
    }

    fn app_sources(&self) -> Vec<AppSource> {
        [
            ("/usr/share/applications", "System", 1, true),
            ("/usr/local/share/applications", "Local", 1, true),
            ("~/.local/share/applications", "User", 1, false),
            (
                "/var/lib/flatpak/exports/share/applications",
                "Flatpak",
                1,
                true,
            ),
            (
                "~/.local/share/flatpak/exports/share/applications",
                "Flatpak (user)",
                1,
                false,
            ),
            ("/snap", "Snap", 2, true),
        ]
        .into_iter()
        .map(|(dir, label, depth, system)| AppSource {
            dir: squid_core::paths::expand(dir),
            label: label.into(),
            depth,
            system,
        })
        .filter(|s| s.dir.is_dir())
        .collect()
    }

    fn list_apps(&self, cancel: &CancelToken, _size_apps: bool) -> Result<Vec<AppInfo>> {
        let running = self.running_executables();
        let mut by_id: BTreeMap<String, AppInfo> = Default::default();
        for source in self.app_sources() {
            if source.label == "Snap" {
                continue; // handled below through `snap list`
            }
            let Ok(entries) = std::fs::read_dir(&source.dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if cancel.is_cancelled() {
                    break;
                }
                let path = entry.path();
                if path.extension().map(|e| e != "desktop").unwrap_or(true) {
                    continue;
                }
                let Some(desktop) = parse_desktop(&path) else {
                    continue;
                };
                if desktop.no_display {
                    continue;
                }
                let id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let exec_name = desktop
                    .exec
                    .as_deref()
                    .and_then(|exec| exec.split_whitespace().next())
                    .map(|exe| {
                        Path::new(exe)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    });
                let running_flag = exec_name
                    .as_deref()
                    .or(desktop.startup_wm_class.as_deref())
                    .map(|needle| {
                        let needle = needle.to_lowercase();
                        running.iter().any(|p| p.to_lowercase().ends_with(&needle))
                    })
                    .unwrap_or(false);
                by_id.entry(id.clone()).or_insert_with(|| AppInfo {
                    name: desktop.name.clone(),
                    bundle_id: Some(id.clone()),
                    path: path.clone(),
                    version: None,
                    size: Bytes::ZERO,
                    source: source.label.clone(),
                    running: running_flag,
                    has_sibling_install: false,
                    publisher: desktop.icon.clone(),
                    installed_ms: None,
                });
            }
        }

        if let Some(list) = run(
            "flatpak",
            &[
                "list",
                "--app",
                "--columns=application,name,size,installation",
            ],
        ) {
            for line in list.lines().skip(1) {
                let cols: Vec<&str> = line.split('\t').collect();
                if let Some(id) = cols.first() {
                    by_id.entry(id.to_string()).or_insert_with(|| AppInfo {
                        name: cols
                            .get(1)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| id.to_string()),
                        bundle_id: Some(id.to_string()),
                        path: PathBuf::from(format!("flatpak:{id}")),
                        version: None,
                        size: cols
                            .get(2)
                            .and_then(|s| s.trim().strip_suffix(" B"))
                            .and_then(|s| s.trim().parse::<u64>().ok())
                            .map(Bytes)
                            .unwrap_or_default(),
                        source: "Flatpak".into(),
                        running: false,
                        has_sibling_install: false,
                        publisher: None,
                        installed_ms: None,
                    });
                }
            }
        }
        Ok(by_id.into_values().collect())
    }

    fn leftover_roots(&self) -> Vec<LeftoverRoot> {
        let candidates = [
            ("~/.config", Naming::AppName),
            ("~/.cache", Naming::AppName),
            ("~/.local/share", Naming::AppName),
            ("~/.local/state", Naming::AppName),
            ("~/.var/app", Naming::BundleId),
            ("~/.pki", Naming::AppName),
        ];
        candidates
            .into_iter()
            .map(|(dir, naming)| LeftoverRoot {
                dir: squid_core::paths::expand(dir),
                naming,
                deep: false,
                category: Category::Leftovers,
            })
            .filter(|c| c.dir.is_dir())
            .collect()
    }

    fn prepare_uninstall(&self, app: &AppInfo) -> LifecycleReport {
        let mut report = LifecycleReport::default();
        if app.running {
            report
                .warnings
                .push(format!("{} is still running", app.name));
        }
        if app.source == "Flatpak" {
            report
                .notes
                .push("`flatpak uninstall --unused` is recommended afterwards".into());
        }
        report
    }

    fn finish_uninstall(&self, _app: &AppInfo, _removed: &[PathBuf]) -> LifecycleReport {
        LifecycleReport::default()
    }

    fn volumes(&self) -> Result<Vec<Volume>> {
        Ok(crate::volumes::volumes_from_mounts(
            &crate::volumes::mount_entries(),
        ))
    }

    fn protected_prefixes(&self) -> Vec<PathBuf> {
        ["/etc", "/usr", "/bin", "/sbin", "/boot", "/EFI"]
            .iter()
            .map(PathBuf::from)
            .collect()
    }

    fn running_executables(&self) -> Vec<String> {
        run("ps", &["-axo", "comm="])
            .map(|out| out.lines().map(|l| l.trim().to_string()).collect())
            .unwrap_or_default()
    }

    fn reveal_trash(&self) {
        let trash = home().join(".local/share/Trash/expunged");
        let _ = Command::new("xdg-open")
            .arg(home().join(".local/share/Trash/files"))
            .spawn();
        let _ = trash;
    }

    fn trash_target(&self) -> TrashTarget {
        // The XDG spec puts it under $XDG_DATA_HOME; in practice ~/.local/share.
        let root = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home().join(".local").join("share"));
        let trash = root.join("Trash");
        if trash.is_dir() {
            TrashTarget::Folder(trash)
        } else {
            TrashTarget::Unreachable
        }
    }
}
