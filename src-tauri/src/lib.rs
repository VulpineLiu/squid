mod app;
mod autostart;
pub mod notifications;
mod permissions;
mod process;
pub mod security;
mod startup;
mod update;

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
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;

    TrayIconBuilder::with_id("squid-tray")
        .icon(icon)
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
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .manage(app::AppState::default())
        .manage(update::UpdateState::default())
        .manage(security::SecurityState::default())
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
            startup::process_inventory,
            startup::process_detail,
            startup::process_stop,
        ])
        .setup(|app| {
            let (settings, _) = Settings::load();
            create_tray(app.handle(), Locale::detect(&settings))?;
            update::spawn_startup_check(app.handle());
            security::spawn_scheduler(app.handle(), &settings);
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
