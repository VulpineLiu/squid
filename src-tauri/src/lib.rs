mod app;
mod autostart;
mod manage;
pub mod notifications;
mod permissions;
mod process;
pub mod security;
mod startup;
mod update;
mod widget;

use security::ScanTarget;
use squid_core::{Locale, Settings};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, WindowEvent,
};

#[cfg(target_os = "macos")]
use tauri::RunEvent;

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        // The page stops sampling while parked in the tray; wake it up.
        let _ = app.emit("squid://visible", true);
    }
}

pub(crate) fn navigate_to(app: &AppHandle, page: &str) {
    show_main_window(app);
    let _ = app.emit("squid://navigate", page);
}

fn tray_text(locale: Locale, en: &'static str, zh: &'static str) -> &'static str {
    if locale == Locale::Zh {
        zh
    } else {
        en
    }
}

fn tray_menu(app: &AppHandle, locale: Locale) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(
        app,
        "open",
        tray_text(locale, "Open Squid", "打开 Squid"),
        true,
        None::<&str>,
    )?;
    // A status line, not a button: it is disabled because reading it is the only
    // thing it is for, and the actions that follow are where the acting happens.
    let protection = MenuItem::with_id(
        app,
        "protection",
        protection_line(app, locale),
        false,
        None::<&str>,
    )?;
    let scan = MenuItem::with_id(
        app,
        "scan",
        if security::scanning(app) {
            tray_text(locale, "Scanning…", "正在扫描…")
        } else {
            tray_text(locale, "Scan for malware", "扫描恶意软件")
        },
        !security::scanning(app),
        None::<&str>,
    )?;
    let startup = MenuItem::with_id(
        app,
        "startup",
        tray_text(locale, "Manage startup items", "管理启动项"),
        true,
        None::<&str>,
    )?;
    let cleanup = MenuItem::with_id(
        app,
        "cleanup",
        tray_text(locale, "Daily cleanup", "日常清理"),
        true,
        None::<&str>,
    )?;
    let apps = MenuItem::with_id(
        app,
        "applications",
        tray_text(locale, "Uninstall apps", "应用卸载"),
        true,
        None::<&str>,
    )?;
    let purge = MenuItem::with_id(
        app,
        "purge",
        tray_text(locale, "Deep Clean", "深度清理"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        tray_text(locale, "Quit Squid", "退出 Squid"),
        true,
        None::<&str>,
    )?;
    let first_separator = PredefinedMenuItem::separator(app)?;
    let last_separator = PredefinedMenuItem::separator(app)?;
    // Flips the widget master switch; state is shown on the Real-time monitor.
    let widget_toggle = MenuItem::with_id(
        app,
        "widget",
        tray_text(locale, "Show / hide desktop widget", "显示/隐藏桌面小工具"),
        true,
        None::<&str>,
    )?;
    Menu::with_items(
        app,
        &[
            &open,
            &protection,
            &scan,
            &startup,
            &first_separator,
            &cleanup,
            &apps,
            &purge,
            &widget_toggle,
            &last_separator,
            &quit,
        ],
    )
}

/// The one line that tells a person whether to open the app. It reports findings
/// squid has actually confirmed, and says "not scanned" rather than "normal" when
/// nothing has been looked at — a security tool that defaults to green is a
/// security tool that lies.
fn protection_line(app: &AppHandle, locale: Locale) -> String {
    if security::scanning(app) {
        return tray_text(locale, "Protection: scanning…", "保护状态：正在扫描").to_string();
    }
    let (confirmed, suspicious, scanned) = security::outstanding();
    if confirmed > 0 {
        return format!(
            "{} {}",
            tray_text(locale, "Protection: issues found", "保护状态：发现问题"),
            confirmed
        );
    }
    if !scanned {
        return tray_text(locale, "Protection: not scanned yet", "保护状态：尚未扫描").to_string();
    }
    if suspicious > 0 {
        return format!(
            "{} {}",
            tray_text(locale, "Protection: needs a look", "保护状态：有待检查项"),
            suspicious
        );
    }
    tray_text(
        locale,
        "Protection: nothing outstanding",
        "保护状态：没有待处理项",
    )
    .to_string()
}

/// The hover text. macOS hides the disabled menu row behind a click, so the
/// status has to be readable without opening anything.
fn tray_tooltip(app: &AppHandle, locale: Locale) -> String {
    format!("Squid — {}", protection_line(app, locale))
}

fn create_tray(app: &AppHandle, locale: Locale) -> tauri::Result<()> {
    let menu = tray_menu(app, locale)?;
    let tooltip = tray_tooltip(app, locale);
    // A transparent monochrome mark: macOS uses it as a template image so the
    // system tints it for light/dark menu bars; other platforms keep the gray.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?;

    TrayIconBuilder::with_id("squid-tray")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip(tooltip)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "scan" => security::scan_in_background(app.clone(), ScanTarget::Quick),
            "startup" => navigate_to(app, "startup"),
            "cleanup" => navigate_to(app, "cleanup"),
            "applications" => navigate_to(app, "applications"),
            "purge" => navigate_to(app, "purge"),
            "widget" => {
                if let Err(error) = widget::toggle_from_tray(app) {
                    eprintln!("widget toggle failed: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

pub(crate) fn sync_tray(app: &AppHandle, locale: Locale) {
    let Some(tray) = app.tray_by_id("squid-tray") else {
        return;
    };
    if let Ok(menu) = tray_menu(app, locale) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = tray.set_tooltip(Some(tray_tooltip(app, locale)));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                // A hidden window must not keep sampling the whole machine;
                // the page pauses its polling on this signal.
                let _ = window.emit("squid://visible", false);
            }
        })
        .manage(app::AppState::default())
        .manage(update::UpdateState::default())
        .manage(security::SecurityState::default())
        .manage(manage::ManageState::default())
        .manage(widget::WidgetState::default())
        .invoke_handler(tauri::generate_handler![
            app::bootstrap,
            app::cancel_job,
            app::cleanup_scan,
            app::cleanup_delete,
            app::purge_scan,
            app::purge_delete,
            app::apps_scan,
            app::app_plan,
            app::app_uninstall,
            app::app_icon,
            app::storage_explore,
            app::storage_largest_files,
            app::system_snapshot,
            app::stop_process,
            app::history_list,
            app::history_undo,
            app::settings_save,
            app::whitelist_add,
            app::whitelist_remove,
            autostart::autostart_status,
            autostart::set_autostart,
            app::reveal_path,
            app::open_trash,
            permissions::permission_status,
            permissions::open_full_disk_access_settings,
            manage::manage_scan,
            manage::managed_startup_set_enabled,
            manage::extension_set_registered,
            manage::popup_list,
            manage::popup_close,
            manage::popup_add_rule,
            manage::popup_rule_update,
            manage::popup_rule_remove,
            manage::popup_watcher_set,
            widget::widget_get,
            widget::widget_set,
            widget::widget_hide,
            update::update_check,
            update::update_status,
            update::update_download,
            update::update_install,
            update::update_clear,
            update::set_update_check_enabled,
            security::security_status,
            security::security_scan,
            security::security_cancel,
            security::security_open,
            security::security_quarantine,
            security::security_restore,
            security::security_delete_quarantine,
            security::security_ignore,
            security::security_exclusions,
            security::security_settings_save,
            security::security_update_rules,
            security::security_rollback_rules,
            security::open_quarantine_folder,
            startup::startup_items,
            startup::startup_set_enabled,
            startup::open_startup_settings,
            startup::process_inventory,
            startup::process_detail,
            startup::process_stop,
        ])
        .setup(|app| {
            let (settings, _) = Settings::load();
            create_tray(app.handle(), Locale::detect(&settings))?;
            update::spawn_startup_check(app.handle());
            security::spawn_scheduler(app.handle(), &settings);
            manage::spawn_watcher(app.handle().clone());
            widget::spawn_startup(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running Squid");

    app.run(|app_handle, event| {
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            show_main_window(app_handle);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app_handle, event);
    });
}
