//! Battery state without pulling in IOKit bindings.
//!
//! macOS: `pmset -g batt` (plus `ioreg` once per refresh for wear/cycles).
//! Linux: `/sys/class/power_supply/BAT*/`. Windows: the caller gets `None`
//! until the Win32 power status API is wired up — the TUI simply omits the
//! battery card.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    pub on_ac: bool,
    pub present: bool,
    /// Minutes until fully charged, when the OS is willing to estimate it.
    pub minutes_to_full: Option<u64>,
    /// Minutes of runtime left on battery.
    pub minutes_to_empty: Option<u64>,
    /// Maximum capacity versus design capacity (iStat's "Health").
    pub health_percent: Option<u8>,
    pub cycle_count: Option<u32>,
    pub temperature_c: Option<f32>,
}

impl Battery {
    pub fn current() -> Option<Battery> {
        #[cfg(target_os = "macos")]
        {
            macos_battery()
        }
        #[cfg(target_os = "linux")]
        {
            return linux_battery();
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }

    pub fn label(&self) -> String {
        if self.charging {
            format!("{}% charging", self.percent)
        } else if self.on_ac {
            format!("{}% charged", self.percent)
        } else {
            format!("{}% on battery", self.percent)
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_battery() -> Option<Battery> {
    let output = std::process::Command::new("/usr/bin/pmset")
        .arg("-g")
        .arg("batt")
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut battery = parse_pmset(&text)?;
    if let Some((health, cycles, temp)) = battery_details() {
        battery.health_percent = health;
        battery.cycle_count = cycles;
        battery.temperature_c = temp;
    }
    Some(battery)
}

/// Example input:
///
/// ```text
/// Now drawing from 'Battery Power'
///  -InternalBattery-0 (id=6226083) 87%; discharging; 4:12 remaining present: true
/// ```
#[cfg(target_os = "macos")]
pub fn parse_pmset(text: &str) -> Option<Battery> {
    let mut battery = Battery {
        present: false,
        percent: 0,
        charging: false,
        on_ac: false,
        minutes_to_full: None,
        minutes_to_empty: None,
        health_percent: None,
        cycle_count: None,
        temperature_c: None,
    };
    let source_line = text
        .lines()
        .find(|line| line.trim_start().starts_with("Now drawing from"))?;
    battery.on_ac = source_line.contains("AC Power");
    let detail = text
        .lines()
        .find(|line| line.contains("Battery-") || line.contains('%'))
        .filter(|line| !line.starts_with("Now drawing"))?;
    if !detail.contains("Battery-") {
        // Desktops report ` -AC Power  100%; charged; 0:00 remaining present: true`
        // and have no battery to show.
        return None;
    }
    battery.present = true;
    let body = detail.split('\t').next_back().unwrap_or(detail).trim();
    let mut parts = body.split(';');
    let first = parts.next()?.trim();
    battery.percent = first.split('%').next()?.trim().parse().unwrap_or(0);
    let state = parts.next().unwrap_or("").trim().to_lowercase();
    battery.charging = state.contains("charging") && !state.contains("discharging");
    if state.contains("discharging") && !battery.on_ac {
        battery.on_ac = false;
    }
    let remaining = parts.next().unwrap_or("").trim();
    if let Some(minutes) = parse_hhmm(remaining.split_whitespace().next().unwrap_or("")) {
        if battery.charging {
            battery.minutes_to_full = Some(minutes);
        } else if state.contains("discharging") {
            battery.minutes_to_empty = Some(minutes);
        }
    }
    Some(battery)
}

#[cfg(target_os = "macos")]
fn battery_details() -> Option<(Option<u8>, Option<u32>, Option<f32>)> {
    let output = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-rn", "AppleSmartBattery"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let design = find_number(&text, "DesignCapacity");
    let max = find_number(&text, "MaxCapacity");
    let health = match (design, max) {
        (Some(design), Some(max)) if design > 0 => {
            Some(((max as f64 / design as f64) * 100.0).round() as u8)
        }
        _ => None,
    };
    let cycles = find_number(&text, "CycleCount").map(|value| value as u32);
    let temp = find_number(&text, "Temperature").map(|tenths| tenths as f32 / 100.0);
    Some((health, cycles, temp))
}

#[cfg(target_os = "macos")]
fn find_number(text: &str, key: &str) -> Option<u64> {
    let (_, after) = text.split_once(&format!("\"{key}\""))?;
    let after = after.split('=').nth(1)?.trim_start();
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(target_os = "linux")]
fn linux_battery() -> Option<Battery> {
    let root = std::path::Path::new("/sys/class/power_supply");
    let entries = std::fs::read_dir(root).ok()?;
    let dir = entries.flatten().map(|entry| entry.path()).find(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().starts_with("BAT"))
            .unwrap_or(false)
    })?;
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .map(|value| value.trim().to_string())
    };
    let percent: u8 = read("capacity")?.parse().ok()?;
    let status = read("status").unwrap_or_default().to_lowercase();
    let charging = status.contains("charging");
    let full = || {
        let now = read("energy_now")?.parse::<f64>().ok()?;
        let rate = read("power_now")?.parse::<f64>().ok()?;
        if rate <= 0.0 {
            return None;
        }
        Some((now / rate * 60.0) as u64)
    };
    Some(Battery {
        percent,
        charging,
        on_ac: status.contains("full") || status.contains("not charging"),
        present: true,
        minutes_to_full: if charging { full() } else { None },
        minutes_to_empty: if !charging && status.contains("discharging") {
            full()
        } else {
            None
        },
        health_percent: None,
        cycle_count: read("cycle_count")?.parse().ok(),
        temperature_c: None,
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn parse_hhmm(text: &str) -> Option<u64> {
    let (hours, minutes) = text.split_once(':')?;
    let hours: u64 = hours.parse().ok()?;
    let minutes: u64 = minutes.parse().ok()?;
    Some(hours * 60 + minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn parses_pmset_output() {
        let text = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=6226083)\t87%; discharging; 4:12 remaining present: true\n";
        let battery = parse_pmset(text).expect("battery");
        assert_eq!(battery.percent, 87);
        assert!(!battery.charging);
        assert!(!battery.on_ac);
        assert_eq!(battery.minutes_to_empty, Some(252));

        let charging = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=6226083)\t42%; charging; (no estimate) present: true\n";
        let battery = parse_pmset(charging).expect("charging");
        assert!(battery.charging);
        assert!(battery.on_ac);
        assert_eq!(battery.minutes_to_full, None);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn desktops_report_no_battery() {
        let text = "Now drawing from 'AC Power'\n -AC Power  100%; charged; 0:00 remaining present: true\n";
        assert!(parse_pmset(text).is_none());
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn hhmm_parsing() {
        assert_eq!(parse_hhmm("4:12"), Some(252));
        assert_eq!(parse_hhmm("0:00"), Some(0));
        assert_eq!(parse_hhmm("no"), None);
    }

    #[test]
    fn current_matches_platform() {
        // Must never panic; laptops return Some, desktops None.
        let _ = Battery::current();
    }
}
