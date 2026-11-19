//! Desktop widget (桌面小工具): a borderless desktop-layer window showing
//! live system stats, styled after the fnOS desktop monitor. It sits above
//! the wallpaper and icons but below ordinary app windows, and lives next to
//! the tray, so it keeps running while the main window is hidden.
//!
//! Settings persist in `widget.json` beside `manage.json`; the widget window
//! and the main window each load this module's commands, and every change is
//! broadcast on `widget://settings` so both views stay in sync.

use serde::{Deserialize, Serialize};
use squid_core::Settings;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const WIDGET_LABEL: &str = "widget";

/// Desktop residency (Windows). The widget is a borderless top-level window
/// parked at `HWND_BOTTOM`: above the wallpaper and icons, below every
/// ordinary app window. Pulsar dodges Win+D by living *inside* the taskbar
/// (SetParent into Shell_TrayWnd — the bar is immune to Show Desktop, and a
/// watchdog re-attaches when the parent drifts). The WebView2 widget cannot
/// be re-parented into the desktop tree like Pulsar's raw window: on this
/// machine the shell ejects it again within seconds, and every crossing
/// strands or hides the web content. So we keep Pulsar's *watchdog* idea
/// instead of its parent trick: the shell hides top-level windows on Win+D
/// by minimising or DWM-cloaking them, and a 1.5 s heartbeat puts any
/// shell-hidden-but-wanted widget back (unminimise → show uncloaks → re-park
/// on the desktop layer → one repaint). Ways the app hid the panel itself
/// (master switch off, all panels off, folded away via hide) are respected.
#[cfg(windows)]
mod desktop_layer {
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        fn SetWindowPos(
            hwnd: *mut c_void,
            insert_after: *mut c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        fn IsIconic(hwnd: *mut c_void) -> i32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
    }
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmGetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *mut i32,
            size: u32,
        ) -> i32;
    }
    const HWND_BOTTOM: *mut c_void = 1usize as *mut c_void;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const DWMWA_CLOAKED: u32 = 14;

    pub fn lower(hwnd: *mut c_void) {
        // SAFETY: hwnd comes from Tauri's own window handle; SetWindowPos with
        // NOMOVE|NOSIZE|NOACTIVATE only edits the z-order.
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    /// Whether the shell (Win+D / 显示桌面) currently has this window put
    /// away while we want it shown: minimised, DWM-cloaked, or plain hidden.
    /// `minimised_or_cloaked` refines the answer for callers that only want
    /// to force a repaint when the shell *actively* took the window down
    /// (a window still hidden mid-startup just needs showing, not reloading).
    pub fn shell_hidden(hwnd: *mut c_void, minimised_or_cloaked: &mut bool) -> bool {
        // SAFETY: plain queries on a live handle.
        *minimised_or_cloaked = unsafe { IsIconic(hwnd) != 0 };
        if *minimised_or_cloaked {
            return true;
        }
        let mut cloaked = 0i32;
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked,
                std::mem::size_of::<i32>() as u32,
            );
        }
        if cloaked != 0 {
            *minimised_or_cloaked = true;
            return true;
        }
        unsafe { IsWindowVisible(hwnd) == 0 }
    }
}

/// Park the widget at the bottom of the window stack (desktop layer).
/// macOS/Linux keep plain floating behaviour for now (no equivalent need:
/// the widget story is Windows-first).
pub fn lower_window(app: &AppHandle) {
    #[cfg(windows)]
    {
        let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
            return;
        };
        if let Ok(hwnd) = window.hwnd() {
            desktop_layer::lower(hwnd.0.cast());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
    }
}

/// Win+D proof, Pulsar-style heartbeat: if the shell put the wanted widget
/// away, bring it back within a beat and repaint it once (the transparent
/// webview can stay blank after a cloak round-trip).
#[cfg(windows)]
pub fn spawn_desktop_watch(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(1_500));
        let Some(state) = app.try_state::<WidgetState>() else {
            continue;
        };
        let prefs = state.current();
        // A panel the user switched off (master switch, or every panel off —
        // sync_window hides the empty shell then) is never resurrected; the
        // watchdog only ever counters the shell's doing.
        let wanted = prefs.enabled
            && (prefs.panel_run
                || prefs.panel_cpu
                || prefs.panel_network
                || prefs.panel_disk
                || prefs.panel_storage);
        if !wanted {
            continue;
        }
        let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
            continue;
        };
        let Ok(hwnd) = window.hwnd() else { continue };
        let raw: *mut std::ffi::c_void = hwnd.0.cast();
        let mut took_down = false;
        if !desktop_layer::shell_hidden(raw, &mut took_down) {
            continue;
        }
        let _ = window.unminimize();
        let _ = window.show();
        desktop_layer::lower(raw);
        // When the shell actively took the window down, Show() clears the
        // cloak but the composed page can come back blank; one cheap reload
        // makes it honest again. A plain hidden window (mid-startup) just
        // needed showing.
        if took_down {
            let _ = window.eval("location.reload()");
        }
    });
}

#[cfg(not(windows))]
pub fn spawn_desktop_watch(_app: &AppHandle) {}

fn default_interval_ms() -> u64 {
    2_000
}

fn default_corner() -> String {
    "top-right".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WidgetPrefs {
    /// Master switch: hides the window entirely when off.
    pub enabled: bool,
    pub panel_run: bool,
    pub panel_cpu: bool,
    pub panel_network: bool,
    pub panel_disk: bool,
    pub panel_storage: bool,
    /// 90–100 (%): window opacity; at most 10% of the desktop may bleed
    /// through so text stays readable over busy wallpapers.
    pub opacity: u8,
    /// Sampling cadence for the widget window.
    pub interval_ms: u64,
    /// `top-right` | `top-left` | `bottom-right` | `bottom-left`.
    pub corner: String,
    /// Remember the collapsed header-only state across restarts.
    pub remember_collapsed: bool,
    pub collapsed: bool,
    /// Last user-dragged position; None re-docks to `corner`.
    pub position: Option<(i32, i32)>,
}

impl Default for WidgetPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            panel_run: true,
            panel_cpu: true,
            panel_network: true,
            panel_disk: true,
            panel_storage: true,
            opacity: 100,
            interval_ms: default_interval_ms(),
            corner: default_corner(),
            remember_collapsed: true,
            collapsed: false,
            position: None,
        }
    }
}

pub struct WidgetState {
    prefs: Mutex<WidgetPrefs>,
}

impl Default for WidgetState {
    fn default() -> Self {
        Self {
            prefs: Mutex::new(load_prefs()),
        }
    }
}

impl WidgetState {
    fn lock<T>(value: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
        value
            .lock()
            .map_err(|_| "The application state is unavailable; restart Squid.".to_string())
    }

    pub fn current(&self) -> WidgetPrefs {
        self.prefs.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

fn prefs_path() -> PathBuf {
    Settings::config_dir().join("widget.json")
}

fn load_prefs() -> WidgetPrefs {
    let mut prefs = fs::read_to_string(prefs_path())
        .ok()
        .and_then(|text| serde_json::from_str::<WidgetPrefs>(&text).ok())
        .unwrap_or_default();
    // Migration: older builds allowed 60–100; the 10%-translucency cap now
    // floors everything below 90 (including the old 92 default).
    prefs.opacity = prefs.opacity.clamp(90, 100);
    // 折叠记忆 off: always boot expanded (save_prefs already stops persisting
    // the fold state, this covers configs written while it was on).
    if !prefs.remember_collapsed {
        prefs.collapsed = false;
    }
    prefs
}

fn save_prefs(state: &WidgetState) -> Result<(), String> {
    let prefs = WidgetState::lock(&state.prefs)?.clone();
    let path = prefs_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    // 折叠记忆 off: the live fold state still applies this session, but the
    // next launch always starts expanded.
    let stored = WidgetPrefs {
        collapsed: if prefs.remember_collapsed {
            prefs.collapsed
        } else {
            false
        },
        ..prefs
    };
    let text = serde_json::to_string_pretty(&stored).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

/// Dock to a screen corner (widget width ~350px) or restore the remembered
/// drag position. Called on creation and whenever the corner changes. All
/// math is in physical pixels — the webview reports `outerPosition` there.
fn place_window(app: &AppHandle, prefs: &WidgetPrefs) {
    let Some(window) = app.get_webview_window(WIDGET_LABEL) else {
        return;
    };
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };
    let size = monitor.size();
    let outer = window
        .outer_size()
        .map(|s| (s.width as i32, s.height as i32))
        .unwrap_or((350, 620));
    let margin = 16;
    let pos = if let Some((x, y)) = prefs.position {
        // Clamp into the monitor so an unplugged-secondary position can not
        // hide the widget off-screen.
        (
            x.clamp(0, (size.width as i32 - outer.0).max(0)),
            y.clamp(0, (size.height as i32 - outer.1).max(0)),
        )
    } else {
        let x = match prefs.corner.as_str() {
            "top-left" | "bottom-left" => margin,
            _ => size.width as i32 - outer.0 - margin,
        };
        let y = match prefs.corner.as_str() {
            "bottom-left" | "bottom-right" => size.height as i32 - outer.1 - margin,
            _ => margin,
        };
        (x, y)
    };
    let _ = window.set_position(tauri::PhysicalPosition::new(pos.0, pos.1));
}

/// Bring the widget window into the state `prefs` describes (create/show/hide).
/// The window is deliberately *not* always-on-top: it is parked at the bottom
/// of the z-order (desktop layer), above wallpaper/icons but under every
/// ordinary app window, so it can never cover the user's work.
fn sync_window(app: &AppHandle, prefs: &WidgetPrefs) {
    if !prefs.enabled
        || !(prefs.panel_run
            || prefs.panel_cpu
            || prefs.panel_network
            || prefs.panel_disk
            || prefs.panel_storage)
    {
        if let Some(window) = app.get_webview_window(WIDGET_LABEL) {
            let _ = window.hide();
        }
        return;
    }
    if app.get_webview_window(WIDGET_LABEL).is_some() {
        if let Some(window) = app.get_webview_window(WIDGET_LABEL) {
            let _ = window.show();
        }
        place_window(app, prefs);
        lower_window(app);
        return;
    }
    let builder = WebviewWindowBuilder::new(
        app,
        WIDGET_LABEL,
        WebviewUrl::App("index.html?view=widget".into()),
    )
    .title("Squid 小工具")
    .inner_size(350.0, 640.0)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(false)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .shadow(false);
    match builder.build() {
        Ok(window) => {
            let _ = window.set_ignore_cursor_events(false);
            place_window(app, prefs);
            let _ = window.show();
            lower_window(app);
        }
        Err(error) => {
            eprintln!("widget window failed to open: {error}");
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetView {
    prefs: WidgetPrefs,
}

#[tauri::command]
pub fn widget_get(app: AppHandle) -> WidgetView {
    WidgetView {
        prefs: app.state::<WidgetState>().current(),
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPatch {
    pub enabled: Option<bool>,
    pub panel_run: Option<bool>,
    pub panel_cpu: Option<bool>,
    pub panel_network: Option<bool>,
    pub panel_disk: Option<bool>,
    pub panel_storage: Option<bool>,
    pub opacity: Option<u8>,
    pub interval_ms: Option<u64>,
    pub corner: Option<String>,
    pub remember_collapsed: Option<bool>,
    pub collapsed: Option<bool>,
    pub position: Option<Option<(i32, i32)>>,
    /// `position_changed` marks a drag-persist (no re-docking after set).
    #[serde(default)]
    pub keep_position: Option<bool>,
    /// Re-run corner placement (used after the window auto-resizes and the
    /// user has not dragged it anywhere).
    #[serde(default)]
    pub redock: Option<bool>,
}

#[tauri::command]
pub fn widget_set(app: AppHandle, patch: WidgetPatch) -> Result<WidgetPrefs, String> {
    let state = app.state::<WidgetState>();
    let (prefs, corner_changed) = {
        let mut prefs = WidgetState::lock(&state.prefs)?;
        if let Some(value) = patch.enabled {
            prefs.enabled = value;
        }
        if let Some(value) = patch.panel_run {
            prefs.panel_run = value;
        }
        if let Some(value) = patch.panel_cpu {
            prefs.panel_cpu = value;
        }
        if let Some(value) = patch.panel_network {
            prefs.panel_network = value;
        }
        if let Some(value) = patch.panel_disk {
            prefs.panel_disk = value;
        }
        if let Some(value) = patch.panel_storage {
            prefs.panel_storage = value;
        }
        if let Some(value) = patch.opacity {
            // At most 10% translucent: anything more reads as "see-through"
            // over busy wallpapers.
            prefs.opacity = value.clamp(90, 100);
        }
        if let Some(value) = patch.interval_ms {
            prefs.interval_ms = value.clamp(1_000, 10_000);
        }
        let corner_changed = patch.corner.is_some();
        if let Some(value) = patch.corner {
            if matches!(
                value.as_str(),
                "top-right" | "top-left" | "bottom-right" | "bottom-left"
            ) {
                prefs.corner = value;
                // Choosing a corner overrides any manual drag position.
                prefs.position = None;
            }
        }
        if let Some(value) = patch.remember_collapsed {
            prefs.remember_collapsed = value;
        }
        if let Some(value) = patch.collapsed {
            prefs.collapsed = value;
        }
        if let Some(value) = patch.position {
            prefs.position = value;
        }
        (prefs.clone(), corner_changed)
    };
    save_prefs(&state)?;
    let keep = patch.keep_position.unwrap_or(false);
    let redock = patch.redock.unwrap_or(false);
    if redock || (!keep && (corner_changed || patch.enabled == Some(true))) {
        place_window(&app, &prefs);
    }
    sync_window(&app, &prefs);
    if keep {
        // The drag click raised the window; settle it back onto the desktop
        // layer so it keeps covering nothing but the wallpaper.
        lower_window(&app);
    }
    let _ = app.emit("widget://settings", &prefs);
    Ok(prefs)
}

/// Hide the widget window (header button) without flipping the master switch:
/// a transient fold-away. Remembered only when 折叠记忆 is on.
#[tauri::command]
pub fn widget_hide(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WIDGET_LABEL) {
        window.hide().map_err(|error| error.to_string())
    } else {
        Ok(())
    }
}

/// Create the window at startup when enabled; the main page can also open it
/// later through widget_set.
pub fn spawn_startup(app: &AppHandle) {
    let prefs = app.state::<WidgetState>().current();
    sync_window(app, &prefs);
}

/// Tray shortcut: flip the master switch and broadcast, same path as the
/// settings page so every view stays in sync.
pub fn toggle_from_tray(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<WidgetState>();
    let target = !state.current().enabled;
    widget_set(
        app.clone(),
        WidgetPatch {
            enabled: Some(target),
            ..Default::default()
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{WidgetPatch, WidgetPrefs};

    #[test]
    fn prefs_round_trip_uses_camel_case_keys() {
        let prefs = WidgetPrefs {
            opacity: 88,
            corner: "bottom-left".into(),
            position: Some((100, 200)),
            ..WidgetPrefs::default()
        };
        let text = serde_json::to_string(&prefs).unwrap();
        // keys the frontend types.ts promises must appear serialized
        for key in [
            "\"panelRun\"",
            "\"panelCpu\"",
            "\"intervalMs\"",
            "\"rememberCollapsed\"",
            "\"collapsed\"",
        ] {
            assert!(text.contains(key), "missing {key} in {text}");
        }
        let back: WidgetPrefs = serde_json::from_str(&text).unwrap();
        assert_eq!(back.opacity, 88);
        assert_eq!(back.corner, "bottom-left");
        assert_eq!(back.position, Some((100, 200)));
    }

    #[test]
    fn patch_accepts_sparse_frontend_payloads() {
        let patch: WidgetPatch = serde_json::from_str(r#"{"opacity":92,"redock":true}"#).unwrap();
        assert_eq!(patch.opacity, Some(92));
        assert_eq!(patch.redock, Some(true));
        assert!(patch.enabled.is_none());
        assert!(patch.position.is_none());
    }

    #[test]
    fn legacy_configs_without_new_fields_still_load() {
        let prefs: WidgetPrefs = serde_json::from_str("{}").unwrap();
        assert!(prefs.enabled);
        assert_eq!(prefs.interval_ms, 2_000);
        assert!(prefs.position.is_none());
    }
}
