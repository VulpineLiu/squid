//! User-level launch-at-login integration.
//!
//! Every platform writes only the current user's startup configuration. Failing
//! to create or remove it is reported to Settings so the toggle can roll back.

use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::process::Command;

#[cfg(any(
    target_os = "windows",
    not(any(target_os = "macos", target_os = "windows"))
))]
const APP_NAME: &str = "Squid";

#[tauri::command]
pub fn autostart_status() -> Result<bool, String> {
    status().map_err(|error| format!("Could not read launch-at-login state: {error}"))
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<bool, String> {
    set(enabled).map_err(|error| format!("Could not update launch-at-login state: {error}"))
}

fn set(enabled: bool) -> Result<bool, String> {
    if enabled {
        enable()?;
    } else {
        disable()?;
    }
    status()
}

fn executable() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn registration_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("The home directory is unavailable.")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join("app.nex.squid.plist"))
}

#[cfg(target_os = "macos")]
fn status() -> Result<bool, String> {
    Ok(registration_path()?.is_file())
}

#[cfg(target_os = "macos")]
fn enable() -> Result<(), String> {
    let path = registration_path()?;
    let parent = path
        .parent()
        .ok_or("The LaunchAgents directory is unavailable.")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    std::fs::write(&path, launch_agent_plist(&executable()?)).map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn disable() -> Result<(), String> {
    let path = registration_path()?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn launch_agent_plist(exe: &Path) -> String {
    let target = exe
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .unwrap_or(exe);
    let arguments = if target == exe {
        format!("<string>{}</string>", xml_escape(&target.to_string_lossy()))
    } else {
        format!(
            "<string>/usr/bin/open</string><string>{}</string>",
            xml_escape(&target.to_string_lossy())
        )
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>app.nex.squid</string>
  <key>ProgramArguments</key>
  <array>{arguments}</array>
  <key>RunAtLoad</key>
  <true/>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(target_os = "windows")]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(target_os = "windows")]
const RUN_VALUE: &str = APP_NAME;

#[cfg(target_os = "windows")]
fn status() -> Result<bool, String> {
    let output = hidden_command("reg.exe")
        .args(["query", RUN_KEY, "/v", RUN_VALUE])
        .output()
        .map_err(|error| error.to_string())?;
    Ok(output.status.success())
}

#[cfg(target_os = "windows")]
fn enable() -> Result<(), String> {
    let exe = executable()?;
    let value = format!("\"{}\"", exe.display());
    let output = hidden_command("reg.exe")
        .args([
            "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &value, "/f",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    command_result(output)
}

#[cfg(target_os = "windows")]
fn disable() -> Result<(), String> {
    if !status()? {
        return Ok(());
    }
    let output = hidden_command("reg.exe")
        .args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])
        .output()
        .map_err(|error| error.to_string())?;
    command_result(output)
}

#[cfg(target_os = "windows")]
fn hidden_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(target_os = "windows")]
fn command_result(output: std::process::Output) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("reg.exe exited with {}", output.status)
    } else {
        stderr
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn registration_path() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|directory| directory.join("autostart").join("squid.desktop"))
        .ok_or_else(|| "The user configuration directory is unavailable.".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn status() -> Result<bool, String> {
    Ok(registration_path()?.is_file())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn enable() -> Result<(), String> {
    let path = registration_path()?;
    let parent = path
        .parent()
        .ok_or("The autostart directory is unavailable.")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    std::fs::write(&path, desktop_entry(&executable()?)).map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn disable() -> Result<(), String> {
    let path = registration_path()?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn desktop_entry(exe: &Path) -> String {
    let executable = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$");
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={APP_NAME}\n\
         Comment=System care for your computer\n\
         Exec=\"{executable}\"\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn launch_agent_opens_the_app_bundle() {
        let plist = launch_agent_plist(Path::new(
            "/Applications/Squid & Tools.app/Contents/MacOS/squid-desktop",
        ));
        assert!(plist.contains("<string>/usr/bin/open</string>"));
        assert!(plist.contains("/Applications/Squid &amp; Tools.app"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn desktop_entry_quotes_the_executable() {
        let entry = desktop_entry(Path::new("/home/me/Squid Tools/squid-desktop"));
        assert!(entry.contains("Exec=\"/home/me/Squid Tools/squid-desktop\""));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
    }
}
