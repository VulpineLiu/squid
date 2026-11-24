mod app;
mod manage;
mod permissions;
mod process;
mod update;
mod widget;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};

#[cfg(target_os = "macos")]
use tauri::RunEvent;

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        // The page stops sampling while parked in the tray; wake it up.
        let _ = app.emit("squid://visible", true);
    }
}

fn navigate_to(app: &AppHandle, page: &'static str) {
    show_main_window(app);
    let _ = app.emit("squid://navigate", page);
}

fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开 Squid", true, None::<&str>)?;
    let space = MenuItem::with_id(app, "space", "空间清理", true, None::<&str>)?;
    let apps = MenuItem::with_id(app, "applications", "应用卸载", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Squid", true, None::<&str>)?;
    let first_separator = PredefinedMenuItem::separator(app)?;
    let last_separator = PredefinedMenuItem::separator(app)?;
    // Flips the widget master switch; state is shown on the 桌面小工具 page.
    let widget_toggle =
        MenuItem::with_id(app, "widget", "显示/隐藏桌面小工具", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &first_separator,
            &space,
            &apps,
            &widget_toggle,
            &last_separator,
            &quit,
        ],
    )?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;

    TrayIconBuilder::with_id("squid-tray")
        .icon(icon)
        .tooltip("Squid")
        .menu(&menu)
        // 左键/双击 = 直接打开主界面，右键 = 菜单（电脑管家式交互）。
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "space" => navigate_to(app, "space"),
            "applications" => navigate_to(app, "applications"),
            "widget" => {
                if let Err(error) = widget::toggle_from_tray(app) {
                    eprintln!("widget toggle failed: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            let app = tray.app_handle();
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
                | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => show_main_window(app),
                _ => {}
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
        .manage(manage::ManageState::default())
        .manage(widget::WidgetState::default())
        .invoke_handler(tauri::generate_handler![
            app::bootstrap,
            app::cancel_job,
            app::cleanup_scan,
            app::cleanup_delete,
            app::cleanup_preflight,
            app::purge_scan,
            app::purge_delete,
            app::duplicates_scan,
            app::duplicates_delete,
            app::large_files_scan,
            app::large_files_delete,
            app::maintenance_tasks,
            app::maintenance_run,
            app::apps_scan,
            app::app_plan,
            app::app_uninstall,
            app::storage_explore,
            app::storage_insights,
            app::write_text_file,
            app::storage_largest_files,
            app::system_snapshot,
            app::stop_process,
            app::history_list,
            app::history_undo,
            app::settings_save,
            app::whitelist_add,
            app::whitelist_remove,
            app::whitelist_audit,
            app::reveal_path,
            app::open_trash,
            permissions::permission_status,
            permissions::open_full_disk_access_settings,
            manage::manage_scan,
            manage::startup_set_enabled,
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
        ])
        .setup(|app| {
            create_tray(app.handle())?;
            update::spawn_startup_check(app.handle());
            manage::spawn_watcher(app.handle().clone());
            widget::spawn_startup(app.handle());
            widget::spawn_desktop_watch(app.handle());
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
