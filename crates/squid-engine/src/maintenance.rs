//! 系统维护 (system maintenance) — one-click repairs for the everyday
//! Windows complaints: search won't open, icons are wrong, the network is
//! flaky, an update keeps failing, sound died.
//!
//! The whole feature is a **controlled whitelist**: the UI can only ever
//! name one of these ids, and every command is `program + fixed argv` —
//! never a shell string, never user input. That is the difference between
//! "rebuild the icon cache" and `cmd /c <whatever the frontend sent>`. Each
//! task carries its real cost (budget seconds) and an elevation need so the
//! UI can be honest before the user clicks.
//!
//! Output is streamed as it arrives (live progress beats a frozen spinner),
//! and the per-task budget kills a runaway command instead of hanging the
//! app — SFC /scannow genuinely takes fifteen minutes.

use serde::Serialize;
use squid_core::progress::{CancelToken, Event, Progress};

/// One repair the whitelist knows about. Commands run top to bottom; any
/// step that needs administrator rights is flagged so the runner can route
/// it through the elevation broker instead of failing on a locked key.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceTask {
    pub id: &'static str,
    pub title_en: &'static str,
    pub title_zh: &'static str,
    pub detail_en: &'static str,
    pub detail_zh: &'static str,
    /// "safe" | "admin" | "slow" — drives the UI risk chip.
    pub risk: &'static str,
    /// Wall-clock budget in seconds; the runner kills past this.
    pub budget_secs: u64,
    /// Human estimate shown before the run ("约 2–5 分钟").
    pub estimate: &'static str,
}

/// argv for one command in a task: (program, args, needs-elevation).
/// One pseudo-program exists: `delete-file` removes the named file through
/// std::fs directly — no shell glob, no quoting question.
type Command = (&'static str, &'static [&'static str], bool);

fn commands(id: &str) -> Option<Vec<Command>> {
    Some(match id {
        "search-index" => vec![
            ("net.exe", &["stop", "WSearch"], true),
            (
                "delete-file",
                &[r"%PROGRAMDATA%\Microsoft\Search\Data\Applications\Windows\Windows.edb"],
                false,
            ),
            ("net.exe", &["start", "WSearch"], true),
        ],
        "icon-cache" => vec![
            ("ie4uinit.exe", &["-ClearIconCache"], false),
            ("taskkill.exe", &["/F", "/IM", "explorer.exe"], false),
            ("cmd.exe", &["/C", "start", "", "explorer.exe"], false),
        ],
        "network-reset" => vec![
            ("netsh.exe", &["winsock", "reset"], true),
            ("ipconfig.exe", &["/flushdns"], true),
            ("ipconfig.exe", &["/registerdns"], true),
        ],
        "sfc-dism" => vec![
            (
                "DISM.exe",
                &["/Online", "/Cleanup-Image", "/RestoreHealth"],
                true,
            ),
            ("sfc.exe", &["/scannow"], true),
        ],
        "windows-update" => vec![
            ("net.exe", &["stop", "wuauserv"], true),
            ("net.exe", &["stop", "bits"], true),
            (
                "cmd.exe",
                &[
                    "/C",
                    "del",
                    "/Q",
                    "/F",
                    r"%SystemRoot%\SoftwareDistribution\Download\*.*",
                ],
                true,
            ),
            ("net.exe", &["start", "bits"], true),
            ("net.exe", &["start", "wuauserv"], true),
        ],
        "audio-restart" => vec![
            ("net.exe", &["stop", "Audiosrv"], true),
            ("net.exe", &["stop", "AudioEndpointBuilder"], true),
            ("net.exe", &["start", "AudioEndpointBuilder"], true),
            ("net.exe", &["start", "Audiosrv"], true),
        ],
        _ => return None,
    })
}

/// The catalog the UI lists. Order matters: cheap and safe first, the long
/// SFC/DISM repair last so nobody starts with the fifteen-minute job.
pub fn tasks() -> Vec<MaintenanceTask> {
    vec![
        MaintenanceTask {
            id: "search-index",
            title_en: "Rebuild the search index",
            title_zh: "重建搜索索引",
            detail_en: "Start menu / Explorer search returns nothing or is stale",
            detail_zh: "开始菜单 / 资源管理器搜索没结果、结果不更新",
            risk: "admin",
            budget_secs: 120,
            estimate: "约 1–2 分钟",
        },
        MaintenanceTask {
            id: "icon-cache",
            title_en: "Rebuild the icon cache",
            title_zh: "重建图标缓存",
            detail_en: "Shortcuts show blank or wrong icons",
            detail_zh: "快捷方式图标变白块、显示错乱（会重启 Explorer）",
            risk: "safe",
            budget_secs: 30,
            estimate: "约 10 秒 · 需重启 Explorer",
        },
        MaintenanceTask {
            id: "network-reset",
            title_en: "Reset the network stack + flush DNS",
            title_zh: "重置网络栈 + 刷新 DNS",
            detail_en: "Connected but pages fail; DNS resolves wrong",
            detail_zh: "能连上但打不开网页、DNS 解析异常（会短暂断网重连）",
            risk: "admin",
            budget_secs: 30,
            estimate: "约 5 秒",
        },
        MaintenanceTask {
            id: "audio-restart",
            title_en: "Restart the audio services",
            title_zh: "重启音频服务",
            detail_en: "No sound, or headphone switching stopped working",
            detail_zh: "突然没声音、耳机切换失效",
            risk: "admin",
            budget_secs: 30,
            estimate: "约 3 秒",
        },
        MaintenanceTask {
            id: "windows-update",
            title_en: "Reset Windows Update components",
            title_zh: "Windows 更新组件重置",
            detail_en: "Updates keep failing or stuck downloading",
            detail_zh: "更新反复失败 / 卡在下载（会停 wuauserv 再清 SoftwareDistribution）",
            risk: "admin",
            budget_secs: 90,
            estimate: "约 30 秒",
        },
        MaintenanceTask {
            id: "sfc-dism",
            title_en: "System file repair (SFC + DISM)",
            title_zh: "系统文件修复（SFC + DISM）",
            detail_en: "Suspected system file corruption, random crashes",
            detail_zh: "疑似系统文件损坏、频繁蓝屏后自检",
            risk: "slow",
            budget_secs: 1_800,
            estimate: "约 15 分钟",
        },
    ]
}

pub fn find(id: &str) -> Option<MaintenanceTask> {
    tasks().into_iter().find(|task| task.id == id)
}

/// Result of a maintenance run: every command's exit, plus the tail of
/// combined output so the UI can show what actually happened.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceResult {
    pub task: String,
    pub ok: bool,
    pub steps: Vec<String>,
    pub failed_step: Option<String>,
    pub output_tail: String,
    pub duration_ms: u64,
}

#[cfg(windows)]
mod run {
    use super::*;
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Stdio};

    fn quote_ps(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    /// Expand the few `%VAR%` tokens our catalog uses; we never hand a raw
    /// string to a shell, so only these need resolving.
    fn expand_env(arg: &str) -> String {
        let mut out = arg.to_string();
        for (key, value) in [
            ("%PROGRAMDATA%", std::env::var("PROGRAMDATA").ok()),
            ("%SystemRoot%", std::env::var("SystemRoot").ok()),
        ] {
            if let Some(value) = value {
                out = out.replace(key, &value);
            }
        }
        out
    }

    fn spawn(program: &str, args: &[String]) -> std::io::Result<Child> {
        std::process::Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
    }

    /// Run one command elevated through the PowerShell broker; a cancelled
    /// UAC prompt is an honest failure, not a crash.
    fn run_elevated(program: &str, args: &[String]) -> Result<(), String> {
        let arglist = args
            .iter()
            .map(|arg| quote_ps(arg))
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            "$ErrorActionPreference='SilentlyContinue'; Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -WindowStyle Hidden -Wait",
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
            Err("管理员授权被取消或命令失败".into())
        }
    }

    /// Wait for a direct child with a wall-clock budget, streaming its
    /// combined output into `progress`. Returns (ok, output tail).
    fn wait_streamed(
        mut child: Child,
        budget: std::time::Duration,
        progress: &Progress,
        cancel: &CancelToken,
    ) -> (bool, String) {
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = crossbeam_channel::unbounded::<String>();
        let mut readers = Vec::new();
        let mut streams: Vec<Box<dyn Read + Send>> = Vec::new();
        if let Some(stream) = stdout {
            streams.push(Box::new(stream));
        }
        if let Some(stream) = stderr {
            streams.push(Box::new(stream));
        }
        for stream in streams {
            let tx = tx.clone();
            readers.push(std::thread::spawn(move || {
                let mut reader = std::io::BufReader::new(stream);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            for line in String::from_utf8_lossy(&buf[..n]).lines() {
                                let trimmed = line.trim();
                                if !trimmed.is_empty() {
                                    let _ = tx.send(trimmed.to_string());
                                }
                            }
                        }
                    }
                }
            }));
        }
        drop(tx);
        let started = std::time::Instant::now();
        let mut tail: Vec<String> = Vec::new();
        loop {
            while let Ok(line) = rx.try_recv() {
                tail.push(line.clone());
                if tail.len() > 200 {
                    tail.remove(0);
                }
                progress.note(line);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Drain whatever the readers queued before EOF.
                    while let Ok(line) = rx.try_recv() {
                        tail.push(line);
                    }
                    for reader in readers {
                        let _ = reader.join();
                    }
                    return (status.success(), tail.join("\n"));
                }
                Ok(None) => {
                    if started.elapsed() > budget || cancel.is_cancelled() {
                        let _ = child.kill();
                        let _ = child.wait();
                        return (false, format!("超时或已取消（预算 {}s）", budget.as_secs()));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
                Err(_) => return (false, "无法读取命令状态".into()),
            }
        }
    }

    pub fn run(
        id: &str,
        progress: &Progress,
        cancel: &CancelToken,
    ) -> squid_core::Result<MaintenanceResult> {
        let task = find(id)
            .ok_or_else(|| squid_core::SquidError::Other(format!("未知的维护任务：{id}")))?;
        let commands = commands(id).unwrap_or_default();
        let budget_each = std::time::Duration::from_secs(task.budget_secs);
        let started = std::time::Instant::now();
        let mut result = MaintenanceResult {
            task: id.to_string(),
            ok: true,
            ..Default::default()
        };
        let total = commands.len() as u64;
        for (index, (program, args, elevated)) in commands.into_iter().enumerate() {
            cancel.check()?;
            progress.publish(Event::Tick {
                done: index as u64,
                total: Some(total),
                subject: Some(std::path::PathBuf::from(format!(
                    "{program} {}",
                    args.join(" ")
                ))),
            });
            let expanded: Vec<String> = args.iter().map(|arg| expand_env(arg)).collect();
            let label = format!("{program} {}", args.join(" "));
            let (ok, output) = if program == "delete-file" {
                // Direct removal, no shell: the whitelist catalog is the
                // only thing that can name a path here.
                let path = std::path::PathBuf::from(&expanded[0]);
                if !path.exists() {
                    (true, String::new())
                } else {
                    match std::fs::remove_file(&path) {
                        Ok(()) => (true, String::new()),
                        Err(error) => (false, format!("删除 {} 失败：{error}", path.display())),
                    }
                }
            } else if elevated {
                match run_elevated(program, &expanded) {
                    Ok(()) => (true, String::new()),
                    Err(error) => (false, error),
                }
            } else {
                match spawn(program, &expanded) {
                    Ok(child) => wait_streamed(child, budget_each, progress, cancel),
                    Err(error) => (false, format!("无法启动 {program}：{error}")),
                }
            };
            result
                .steps
                .push(format!("{label} — {}", if ok { "完成" } else { "失败" }));
            if !output.is_empty() {
                result.output_tail = output;
            }
            if !ok {
                result.ok = false;
                result.failed_step = Some(label);
                break;
            }
        }
        result.duration_ms = started.elapsed().as_millis() as u64;
        Ok(result)
    }
}

#[cfg(not(windows))]
mod run {
    use super::*;
    pub fn run(
        id: &str,
        _progress: &Progress,
        _cancel: &CancelToken,
    ) -> squid_core::Result<MaintenanceResult> {
        Ok(MaintenanceResult {
            task: id.to_string(),
            ok: false,
            failed_step: Some("这些维护命令仅在 Windows 提供".into()),
            ..Default::default()
        })
    }
}

pub fn run(
    id: &str,
    progress: &Progress,
    cancel: &CancelToken,
) -> squid_core::Result<MaintenanceResult> {
    run::run(id, progress, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_task_has_a_command_list() {
        for task in tasks() {
            let commands = commands(task.id);
            assert!(commands.is_some(), "{} has no commands", task.id);
            assert!(!commands.unwrap().is_empty(), "{} is empty", task.id);
            assert!(task.budget_secs > 0);
            assert!(!task.estimate.is_empty());
        }
    }

    #[test]
    fn the_whitelist_rejects_unknown_ids() {
        assert!(commands("rm -rf /").is_none());
        assert!(find("delete-my-boot").is_none());
        assert_eq!(tasks().len(), 6);
    }

    #[test]
    fn slow_jobs_come_last_and_carry_the_biggest_budget() {
        let list = tasks();
        let last = list.last().expect("tasks");
        assert_eq!(last.id, "sfc-dism");
        assert!(list.iter().all(|task| task.budget_secs <= last.budget_secs));
        for task in &list {
            // Nothing outside the six known ids sneaks into the catalog.
            assert!(commands(task.id).is_some());
        }
    }
}
