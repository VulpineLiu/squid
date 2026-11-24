//! Tauri commands for the 权限管理 page: startup items, context-menu handlers
//! and a popup blocker (list visible windows, register per-title rules, close
//! matching windows from a background watcher).
//!
//! Entries address themselves by their registry-level `id`, so a toggle can
//! never fire at a shifted list position — the same safety idea as the
//! index-only delete flows, expressed for a page that mutates continuously.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use squid_core::Settings;
use squid_platform::permissions::{self, PopupWindow, ShellExtEntry, StartupEntry};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// One popup rule: "whenever a window from `exe` shows `title`, close it".
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PopupRule {
    pub exe: String,
    pub title: String,
    /// `contains` (default) or `exact`.
    pub matcher: String,
    pub enabled: bool,
    /// Cumulative number of windows this rule has closed.
    pub closed: u64,
    /// `data:image/png;base64,…` of the owning exe, filled in by `manage_scan`
    /// only — never persisted into manage.json.
    #[serde(skip)]
    pub icon: Option<String>,
}

impl Default for PopupRule {
    fn default() -> Self {
        Self {
            exe: String::new(),
            title: String::new(),
            matcher: "contains".to_string(),
            enabled: true,
            closed: 0,
            icon: None,
        }
    }
}

impl PopupRule {
    fn matches(&self, window: &PopupWindow) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(exe) = window.exe.as_deref() else {
            return false;
        };
        if !exe.eq_ignore_ascii_case(&self.exe) {
            return false;
        }
        if self.matcher == "exact" {
            window.title == self.title
        } else if self.title.is_empty() {
            true
        } else {
            window.title.contains(&self.title)
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ManagePrefs {
    enabled: bool,
    interval_ms: u64,
    rules: Vec<PopupRule>,
    /// Handlers the user removed; kept so the list can offer "restore".
    removed_extensions: Vec<ShellExtEntry>,
}

pub struct ManageState {
    prefs: Mutex<ManagePrefs>,
    startup: Mutex<Vec<StartupEntry>>,
    extensions: Mutex<Vec<ShellExtEntry>>,
}

impl Default for ManageState {
    fn default() -> Self {
        Self {
            prefs: Mutex::new(load_prefs()),
            startup: Mutex::new(Vec::new()),
            extensions: Mutex::new(Vec::new()),
        }
    }
}

impl ManageState {
    fn lock<T>(value: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
        value
            .lock()
            .map_err(|_| "The application state is unavailable; restart Squid.".to_string())
    }
}

fn prefs_path() -> PathBuf {
    Settings::config_dir().join("manage.json")
}

fn load_prefs() -> ManagePrefs {
    fs::read_to_string(prefs_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_prefs(state: &ManageState) -> Result<(), String> {
    let prefs = ManageState::lock(&state.prefs)?.clone();
    let path = prefs_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(&prefs).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupView {
    id: String,
    name: String,
    display_name: String,
    /// `data:image/png;base64,…` of the binary's icon, absent on macOS/Linux.
    icon: Option<String>,
    command: String,
    source: String,
    source_label: String,
    scope: String,
    enabled: bool,
    /// Attributed to a real software product (drives the 其他软件 bucket).
    named: bool,
}

impl From<&StartupEntry> for StartupView {
    fn from(entry: &StartupEntry) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            display_name: entry.display_name.clone(),
            icon: entry.icon.as_deref().map(png_data_url),
            command: entry.command.clone(),
            source: entry.source.clone(),
            source_label: entry.source_label.clone(),
            scope: entry.scope.clone(),
            enabled: entry.enabled,
            named: entry.named,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionView {
    id: String,
    name: String,
    handler: String,
    extension_point: String,
    scope: String,
    friendly_name: Option<String>,
    module: Option<String>,
    /// Publisher of the owning dll — 电脑管家's 创建者 column.
    publisher: Option<String>,
    /// `data:image/png;base64,…` of the owning module's icon.
    icon: Option<String>,
    /// false = currently parked in the app's restore list.
    registered: bool,
}

impl ExtensionView {
    fn label(&self) -> &str {
        self.friendly_name.as_deref().unwrap_or(&self.name)
    }
}

fn png_data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", BASE64.encode(png))
}

impl From<&ShellExtEntry> for ExtensionView {
    fn from(entry: &ShellExtEntry) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            handler: entry.handler.clone(),
            extension_point: entry.extension_point.clone(),
            scope: entry.scope.clone(),
            friendly_name: entry.friendly_name.clone(),
            module: entry.module.clone(),
            publisher: entry.publisher.clone(),
            icon: entry.icon.as_deref().map(png_data_url),
            registered: true,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupView {
    hwnd: i64,
    title: String,
    process: String,
    exe: Option<String>,
    /// `data:image/png;base64,…` of the owning process's icon.
    icon: Option<String>,
}

impl From<&PopupWindow> for PopupView {
    fn from(window: &PopupWindow) -> Self {
        Self {
            hwnd: window.hwnd,
            title: window.title.clone(),
            process: window.process.clone(),
            exe: window.exe.clone(),
            icon: window.icon.as_deref().map(png_data_url),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManageView {
    watcher_enabled: bool,
    interval_ms: u64,
    rules: Vec<PopupRule>,
    windows: Vec<PopupView>,
    startup: Vec<StartupView>,
    extensions: Vec<ExtensionView>,
    startup_count: usize,
    startup_enabled_count: usize,
}

#[tauri::command]
pub async fn manage_scan(app: AppHandle) -> Result<ManageView, String> {
    let manage = app.state::<ManageState>();
    let (startup, extensions, windows) = tauri::async_runtime::spawn_blocking(|| {
        (
            permissions::scan_startup_entries(),
            permissions::scan_shell_extensions(),
            permissions::list_popup_windows(),
        )
    })
    .await
    .map_err(|error| format!("权限扫描失败：{error}"))?;

    let mut startup_view: Vec<StartupView> = startup.iter().map(StartupView::from).collect();
    startup_view.sort_by(|a, b| {
        a.enabled
            .cmp(&b.enabled)
            .reverse()
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let mut extension_view: Vec<ExtensionView> =
        extensions.iter().map(ExtensionView::from).collect();

    let (watcher_enabled, interval_ms, mut rules) = {
        let prefs = ManageState::lock(&manage.prefs)?;
        for parked in &prefs.removed_extensions {
            extension_view.push(ExtensionView {
                registered: false,
                ..ExtensionView::from(parked)
            });
        }
        extension_view.sort_by(|a, b| {
            a.registered
                .cmp(&b.registered)
                .reverse()
                .then_with(|| a.extension_point.cmp(&b.extension_point))
                .then_with(|| a.label().to_lowercase().cmp(&b.label().to_lowercase()))
        });
        (
            prefs.enabled,
            if prefs.interval_ms == 0 {
                1_500
            } else {
                prefs.interval_ms
            },
            prefs.rules.clone(),
        )
    };
    // Rule rows carry the owning software's icon (电脑管家 shows it too).
    let mut rule_icons: std::collections::HashMap<String, Option<String>> = Default::default();
    for rule in &mut rules {
        let icon = rule_icons
            .entry(rule.exe.clone())
            .or_insert_with(|| permissions::exe_icon(&rule.exe).map(|png| png_data_url(&png)))
            .clone();
        rule.icon = icon;
    }
    *ManageState::lock(&manage.startup)? = startup;
    *ManageState::lock(&manage.extensions)? = extensions;
    let startup_enabled_count = startup_view.iter().filter(|entry| entry.enabled).count();
    let startup_count = startup_view.len();
    Ok(ManageView {
        watcher_enabled,
        interval_ms,
        rules,
        windows: windows.iter().map(PopupView::from).collect(),
        startup: startup_view,
        extensions: extension_view,
        startup_count,
        startup_enabled_count,
    })
}

/// Flip one startup entry (registry / startup folder / scheduled task), then
/// rescan so a partially-applied toggle can never desync the UI.
#[tauri::command]
pub async fn startup_set_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<usize, String> {
    let manage = app.state::<ManageState>();
    let entry = ManageState::lock(&manage.startup)?
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
        .ok_or_else(|| "请先重新扫描启动项".to_string())?;
    tauri::async_runtime::spawn_blocking(move || permissions::set_startup_enabled(&entry, enabled))
        .await
        .map_err(|error| error.to_string())??;
    let fresh = tauri::async_runtime::spawn_blocking(permissions::scan_startup_entries)
        .await
        .map_err(|error| error.to_string())?;
    let enabled_count = fresh.iter().filter(|entry| entry.enabled).count();
    *ManageState::lock(&manage.startup)? = fresh;
    Ok(enabled_count)
}

// ---- 进阶启动项 (advanced startup) ----------------------------------------

/// One advanced row for the UI: registry hooks with signature trust, plus
/// the hidden-task / auto-service tails merged from the ordinary scan.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvancedView {
    id: String,
    kind: String,
    name: String,
    command: String,
    location: String,
    scope: String,
    enabled: bool,
    signature: String,
    signer: Option<String>,
    warning: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvancedScanView {
    entries: Vec<AdvancedView>,
    /// Hidden scheduled tasks and automatic services, in the ordinary
    /// startup shape so their toggles reuse `startup_set_enabled`.
    extras: Vec<StartupView>,
}

#[tauri::command]
pub async fn advanced_scan(app: AppHandle) -> Result<AdvancedScanView, String> {
    let manage = app.state::<ManageState>();
    let (entries, startup) = tauri::async_runtime::spawn_blocking(|| {
        (
            squid_platform::advanced::scan_advanced(),
            permissions::scan_startup_entries(),
        )
    })
    .await
    .map_err(|error| format!("进阶扫描失败：{error}"))?;
    // Cache the ordinary scan so the extras' toggles can find their entries.
    *ManageState::lock(&manage.startup)? = startup.clone();
    let extras: Vec<StartupView> = startup
        .iter()
        .filter(|entry| {
            // Only the corners the plain list does not already show:
            // hidden-by-default vendor tasks (source marks) and services.
            entry.source == "service"
        })
        .map(StartupView::from)
        .collect();
    Ok(AdvancedScanView {
        entries: entries
            .iter()
            .map(|entry| AdvancedView {
                id: entry.id.clone(),
                kind: entry.kind.clone(),
                name: entry.name.clone(),
                command: entry.command.clone(),
                location: entry.location.clone(),
                scope: entry.scope.clone(),
                enabled: entry.enabled,
                signature: entry.signature.clone(),
                signer: entry.signer.clone(),
                warning: entry.warning.clone(),
            })
            .collect(),
        extras,
    })
}

#[tauri::command]
pub async fn advanced_set_enabled(app: AppHandle, id: String, enabled: bool) -> Result<(), String> {
    let manage = app.state::<ManageState>();
    // Advanced rows carry their whole state in the id + a fresh scan match.
    let entry = tauri::async_runtime::spawn_blocking(squid_platform::advanced::scan_advanced)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| "该进阶项已不存在，请重新扫描".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        squid_platform::advanced::set_advanced_enabled(&entry, enabled)
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(())
}

/// Unregister / restore a context-menu handler. Removal keeps the entry in
/// `manage.json`, which is both the restore recipe and the audit trail.
#[tauri::command]
pub fn extension_set_registered(
    app: AppHandle,
    id: String,
    registered: bool,
) -> Result<(), String> {
    let manage = app.state::<ManageState>();
    let entry = if registered {
        ManageState::lock(&manage.prefs)?
            .removed_extensions
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
    } else {
        ManageState::lock(&manage.extensions)?
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
    }
    .ok_or_else(|| "请先重新扫描右键菜单".to_string())?;
    if registered {
        permissions::restore_shell_extension(&entry)?;
        ManageState::lock(&manage.prefs)?
            .removed_extensions
            .retain(|item| item.id != id);
    } else {
        permissions::remove_shell_extension(&entry)?;
        let mut prefs = ManageState::lock(&manage.prefs)?;
        if !prefs.removed_extensions.iter().any(|item| item.id == id) {
            prefs.removed_extensions.push(entry);
        }
    }
    save_prefs(&manage)
}

/// Live list of closeable top-level windows (popup blocker "抓一个窗口").
#[tauri::command]
pub fn popup_list() -> Vec<PopupView> {
    permissions::list_popup_windows()
        .iter()
        .map(PopupView::from)
        .collect()
}

/// Close one window manually from the picker.
#[tauri::command]
pub fn popup_close(hwnd: i64) -> Result<(), String> {
    if permissions::close_popup_window(hwnd) {
        Ok(())
    } else {
        Err("这个窗口已经关闭或拒绝了关闭请求".to_string())
    }
}

/// Teach the blocker a new rule from a live window: "this exe + this title".
#[tauri::command]
pub fn popup_add_rule(app: AppHandle, hwnd: i64, matcher: String) -> Result<(), String> {
    let manage = app.state::<ManageState>();
    let window = permissions::list_popup_windows_bare()
        .into_iter()
        .find(|window| window.hwnd == hwnd)
        .ok_or_else(|| "窗口已关闭，请重新选择".to_string())?;
    let exe = window.exe.ok_or("无法读取该窗口的程序路径".to_string())?;
    let matcher = if matcher == "exact" {
        "exact"
    } else {
        "contains"
    };
    {
        let mut prefs = ManageState::lock(&manage.prefs)?;
        if prefs
            .rules
            .iter()
            .any(|item| item.exe == exe && item.title == window.title)
        {
            return Err("这条拦截规则已存在".to_string());
        }
        prefs.rules.push(PopupRule {
            exe,
            title: window.title.clone(),
            matcher: matcher.to_string(),
            enabled: true,
            closed: 0,
            icon: None,
        });
        prefs.enabled = true;
    }
    // Adding a rule implies closing its window right now.
    let _ = permissions::close_popup_window(hwnd);
    save_prefs(&manage)
}

#[tauri::command]
pub fn popup_rule_update(
    app: AppHandle,
    index: usize,
    enabled: Option<bool>,
    reset_count: Option<bool>,
) -> Result<(), String> {
    let manage = app.state::<ManageState>();
    {
        let mut prefs = ManageState::lock(&manage.prefs)?;
        let rule = prefs
            .rules
            .get_mut(index)
            .ok_or("规则已变化，请刷新后重试")?;
        if let Some(enabled) = enabled {
            rule.enabled = enabled;
        }
        if reset_count.unwrap_or(false) {
            rule.closed = 0;
        }
    }
    save_prefs(&manage)
}

#[tauri::command]
pub fn popup_rule_remove(app: AppHandle, index: usize) -> Result<(), String> {
    let manage = app.state::<ManageState>();
    {
        let mut prefs = ManageState::lock(&manage.prefs)?;
        if index >= prefs.rules.len() {
            return Err("规则已变化，请刷新后重试".to_string());
        }
        prefs.rules.remove(index);
    }
    save_prefs(&manage)
}

#[tauri::command]
pub fn popup_watcher_set(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manage = app.state::<ManageState>();
    ManageState::lock(&manage.prefs)?.enabled = enabled;
    save_prefs(&manage)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClosedEvent {
    process: String,
    title: String,
    rule_index: usize,
}

/// The resident watcher: poll the desktop, close every window a rule claims.
/// Ticks only while enabled with armed rules; one `EnumWindows` per interval
/// is cheap enough for a 1.5 s heartbeat and stops itself when disarmed.
pub fn spawn_watcher(app: AppHandle) {
    std::thread::spawn(move || loop {
        let manage = match app.try_state::<ManageState>() {
            Some(manage) => manage,
            None => return,
        };
        let (interval, armed) = match ManageState::lock(&manage.prefs) {
            Ok(prefs) => (
                Duration::from_millis(if prefs.interval_ms == 0 {
                    1_500
                } else {
                    prefs.interval_ms.max(500)
                }),
                prefs.enabled && prefs.rules.iter().any(|rule| rule.enabled),
            ),
            Err(_) => return,
        };
        if armed {
            let windows = permissions::list_popup_windows_bare();
            let mut closed: Vec<ClosedEvent> = Vec::new();
            {
                let mut prefs = match ManageState::lock(&manage.prefs) {
                    Ok(prefs) => prefs,
                    Err(_) => return,
                };
                for window in &windows {
                    for (index, rule) in prefs.rules.iter_mut().enumerate() {
                        if rule.matches(window) {
                            if permissions::close_popup_window(window.hwnd) {
                                rule.closed += 1;
                                closed.push(ClosedEvent {
                                    process: window.process.clone(),
                                    title: window.title.clone(),
                                    rule_index: index,
                                });
                            }
                            break;
                        }
                    }
                }
            }
            if !closed.is_empty() {
                let _ = save_prefs(&manage);
                for event in closed {
                    let _ = app.emit("popup://closed", event);
                }
            }
        }
        std::thread::sleep(interval);
    });
}
