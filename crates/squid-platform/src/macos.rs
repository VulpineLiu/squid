//! macOS adapter: `.app` bundles, sandbox containers, launch services.

use crate::apps::{app_from_bundle, discover_bundles};
use crate::{
    AppSource, Device, LeftoverRoot, LifecycleReport, Naming, Platform, PlatformId, TrashTarget,
};
use rayon::prelude::*;
use squid_core::progress::CancelToken;
use squid_core::{AppInfo, Category, Result, SquidError, Volume};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Default)]
pub struct MacOs;

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Versions/A/\
                          Frameworks/LaunchServices.framework/Versions/A/Support/lsregister";

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn quiet(program: &str, args: &[&str]) {
    let _ = Command::new(program).args(args).output();
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/Users/nobody"))
}

fn root(path: &str) -> PathBuf {
    PathBuf::from(path)
}

impl Platform for MacOs {
    fn id(&self) -> PlatformId {
        PlatformId::MacOS
    }

    fn display_name(&self) -> String {
        "macOS".to_string()
    }

    fn os_version(&self) -> String {
        let product = run("sw_vers", &["-productVersion"]).unwrap_or_default();
        let build = run("sw_vers", &["-buildVersion"]).unwrap_or_default();
        if build.is_empty() {
            format!("macOS {product}")
        } else {
            format!("macOS {product} ({build})")
        }
    }

    fn hardware_model(&self) -> String {
        let model = run("sysctl", &["-n", "hw.model"]).unwrap_or_default();
        let brand = run("sysctl", &["-n", "machdep.cpu.brand_string"]).unwrap_or_default();
        let chips = run("sysctl", &["-n", "hw.perflevel0.physicalcpu"])
            .and_then(|p| {
                run("sysctl", &["-n", "hw.perflevel1.physicalcpu"]).map(|e| format!("{p}P+{e}E"))
            })
            .unwrap_or_default();
        let mut parts = vec![model.clone()];
        if !brand.is_empty() && !model.contains(&brand) {
            parts.push(brand);
        }
        if !chips.is_empty() {
            parts.push(chips);
        }
        parts.join(" · ")
    }

    fn graphics_adapter(&self) -> String {
        run(
            "/usr/sbin/system_profiler",
            &["SPDisplaysDataType", "-detailLevel", "mini"],
        )
        .map(|text| {
            text.lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix("Chipset Model:")
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                })
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default()
    }

    fn app_sources(&self) -> Vec<AppSource> {
        let mut sources = vec![
            AppSource {
                dir: root("/Applications"),
                label: "Applications".into(),
                depth: 2,
                system: false,
            },
            AppSource {
                dir: home().join("Applications"),
                label: "User Applications".into(),
                depth: 2,
                system: false,
            },
            AppSource {
                dir: root("/System/Applications"),
                label: "macOS Apps".into(),
                depth: 2,
                system: true,
            },
            AppSource {
                dir: home().join("Library/Application Support/Setapp/Applications"),
                label: "Setapp".into(),
                depth: 1,
                system: false,
            },
            AppSource {
                dir: home().join("Library/Containers/io.playcover.PlayCover/Applications"),
                label: "PlayCover".into(),
                depth: 1,
                system: false,
            },
        ];
        sources.retain(|s| s.dir.is_dir());
        sources
    }

    fn list_apps(&self, cancel: &CancelToken, size_apps: bool) -> Result<Vec<AppInfo>> {
        let running = self.running_executables();
        let mut bundles: Vec<(PathBuf, String)> = Vec::new();
        for source in self.app_sources() {
            for path in discover_bundles(&source.dir, source.depth, cancel) {
                bundles.push((path, source.label.clone()));
            }
        }
        let mut apps: Vec<AppInfo> = bundles
            .par_iter()
            .filter_map(|(path, source)| app_from_bundle(path.clone(), source, &running, size_apps))
            .collect();
        apps.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        // Flag bundle ids that appear more than once: shared preference files
        // must survive when only one copy goes away.
        let mut counts: std::collections::HashMap<String, usize> = Default::default();
        for app in &apps {
            if let Some(id) = app.bundle_id.as_deref() {
                *counts.entry(id.to_lowercase()).or_insert(0) += 1;
            }
        }
        for app in &mut apps {
            if let Some(id) = app.bundle_id.as_deref() {
                app.has_sibling_install = counts.get(&id.to_lowercase()).copied().unwrap_or(0) > 1;
            }
        }
        Ok(apps)
    }

    fn leftover_roots(&self) -> Vec<LeftoverRoot> {
        let user = |rel: &str, naming: Naming| LeftoverRoot {
            dir: home().join(rel),
            naming,
            deep: false,
            category: Category::Leftovers,
        };
        let system = |abs: &str, naming: Naming| LeftoverRoot {
            dir: root(abs),
            naming,
            deep: false,
            category: Category::Leftovers,
        };
        let candidates = vec![
            user("Library/Application Support", Naming::Either),
            user("Library/Caches", Naming::Either),
            user("Library/Preferences", Naming::BundleId),
            user("Library/Preferences/ByHost", Naming::BundleId),
            user("Library/Saved Application State", Naming::BundleId),
            user("Library/HTTPStorages", Naming::BundleId),
            user("Library/WebKit", Naming::BundleId),
            user("Library/Cookies", Naming::BundleId),
            user("Library/Containers", Naming::BundleId),
            user("Library/Application Scripts", Naming::BundleId),
            user("Library/Group Containers", Naming::GroupContainer),
            user("Library/LaunchAgents", Naming::BundleId),
            user("Library/Logs", Naming::Either),
            user(
                "Library/Application Support/CrashReporter",
                Naming::BundleId,
            ),
            user("Library/PreferencePanes", Naming::AppName),
            user("Library/QuickLook", Naming::AppName),
            user("Library/Services", Naming::AppName),
            user("Library/ScriptingAdditions", Naming::AppName),
            user("Library/Input Methods", Naming::AppName),
            user("Library/Screen Savers", Naming::AppName),
            user("Library/PreferenceBundles", Naming::AppName),
            user("Library/Audio/Plug-Ins/AppleAudioMIDI", Naming::AppName),
            user(".config", Naming::AppName),
            user(".cache", Naming::AppName),
            user(".local/share", Naming::AppName),
            user(".local/state", Naming::AppName),
            system("/Library/LaunchAgents", Naming::BundleId),
            system("/Library/LaunchDaemons", Naming::BundleId),
            system("/Library/Application Support", Naming::Either),
            system("/Library/Caches", Naming::Either),
            system("/Library/Logs", Naming::Either),
            system("/Library/PreferencePanes", Naming::AppName),
            system("/Library/ScriptingAdditions", Naming::AppName),
            system("/Library/QuickLook", Naming::AppName),
            system("/Library/Audio/Plug-Ins/HAL", Naming::AppName),
            system("/Library/Screen Savers", Naming::AppName),
            system("/Library/Internet Plug-Ins", Naming::AppName),
        ];
        candidates.into_iter().filter(|c| c.dir.is_dir()).collect()
    }

    /// Unloading login items and clearing the Launch Services database entry
    /// keeps macOS from resurrecting the app's icon or restarting its helpers.
    fn prepare_uninstall(&self, app: &AppInfo) -> LifecycleReport {
        let mut report = LifecycleReport::default();
        if app.running {
            report
                .warnings
                .push(format!("{} is still running", app.name));
        }

        let Some(bundle_id) = app.bundle_id.as_deref() else {
            return report;
        };
        let uid = unsafe { libc::getuid() };
        let launch_roots = [
            home().join("Library/LaunchAgents"),
            PathBuf::from("/Library/LaunchAgents"),
            PathBuf::from("/Library/LaunchDaemons"),
        ];
        for dir in launch_roots {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                    continue;
                };
                if !name.ends_with(".plist")
                    || !name.to_lowercase().contains(&bundle_id.to_lowercase())
                {
                    continue;
                }
                let label = name.trim_end_matches(".plist");
                let domain = if path.starts_with("/Library/LaunchDaemons") {
                    format!("system/{label}")
                } else {
                    format!("gui/{uid}/{label}")
                };
                if Command::new("launchctl")
                    .args(["bootout", &domain])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    report.stopped_services.push(path.display().to_string());
                }
            }
        }

        if Path::new(LSREGISTER).exists() {
            quiet(LSREGISTER, &["-u", &app.path.to_string_lossy()]);
            report
                .notes
                .push("removed the Launch Services registration".into());
        }
        report
    }

    fn finish_uninstall(&self, app: &AppInfo, _removed: &[PathBuf]) -> LifecycleReport {
        let mut report = LifecycleReport::default();
        // Homebrew-owned apps must be told they are gone, or `brew list` keeps
        // a phantom entry forever.
        if app.path.starts_with("/Applications") || app.path.starts_with("/opt/homebrew") {
            if let Some(cask) = cask_for_bundle(&app.path) {
                quiet("brew", &["uninstall", "--cask", "--force", &cask]);
                report.notes.push(format!("told Homebrew about {cask}"));
            }
        }
        // Receipts make the installer think the app is still present.
        if let Some(bundle_id) = app.bundle_id.as_deref() {
            if let Some(receipts) = run("pkgutil", &["--pkgs"]) {
                for receipt in receipts.lines() {
                    if receipt
                        .to_lowercase()
                        .starts_with(&bundle_id.to_lowercase())
                    {
                        quiet("pkgutil", &["--forget", receipt]);
                        report.notes.push(format!("forgot receipt {receipt}"));
                    }
                }
            }
        }
        quiet("killall", &["-HUP", "cfprefsd"]);
        report
    }

    fn volumes(&self) -> Result<Vec<Volume>> {
        let volumes = crate::volumes::volumes_from_mounts(&crate::volumes::mount_entries());
        if volumes.is_empty() {
            return Err(SquidError::Other("could not read the mount table".into()));
        }
        Ok(volumes)
    }

    fn protected_prefixes(&self) -> Vec<PathBuf> {
        vec![
            PathBuf::from("/System"),
            PathBuf::from("/Library/Apple"),
            home().join("Library/Keychains"),
            home().join("Library/Mail"),
            home().join("Library/Messages"),
            home().join("Library/Photos"),
            home().join("Library/Application Support/MobileSync"),
            home().join("Library/Application Support/CloudDocs"),
            home().join("Library/Containers/com.apple.Safari"),
        ]
    }

    fn running_executables(&self) -> Vec<String> {
        run("ps", &["-axo", "comm="])
            .map(|out| out.lines().map(|l| l.trim().to_string()).collect())
            .unwrap_or_default()
    }

    fn reveal_trash(&self) {
        quiet("open", &[&home().join(".Trash").to_string_lossy()]);
    }

    fn device(&self) -> Device {
        Device::Mac
    }

    fn trash_target(&self) -> TrashTarget {
        let trash = home().join(".Trash");
        if trash.is_dir() {
            TrashTarget::Folder(trash)
        } else {
            TrashTarget::Unreachable
        }
    }

    fn inspect_path(&self, path: &Path) -> Option<String> {
        // Sandboxed containers are owned by containermanagerd; deleting the
        // folder without the metadata file upsets the sandbox service.
        if path
            .join(".com.apple.containermanagerd.metadata.plist")
            .exists()
        {
            return Some("sandboxed container (removing the whole folder is safe)".into());
        }
        if path.starts_with(home().join("Library/Containers")) {
            return Some("inside a sandboxed container".into());
        }
        None
    }
}

/// Ask Homebrew which cask installed this bundle, if any.
fn cask_for_bundle(app_path: &Path) -> Option<String> {
    let out = run("brew", &["list", "--cask", "--versions"])
        .or_else(|| run("/opt/homebrew/bin/brew", &["list", "--cask", "--versions"]))?;
    let stem = app_path
        .file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(".app").to_lowercase())
        .unwrap_or_default();
    for line in out.lines() {
        let name = line.split_whitespace().next()?.to_lowercase();
        if name == stem || stem.contains(&name) || name.contains(&stem) {
            return Some(line.split_whitespace().next()?.to_string());
        }
    }
    None
}
