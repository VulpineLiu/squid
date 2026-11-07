//! System notifications, and the one thing they must never be relied on for.
//!
//! A notification is a convenience: it tells a person that something is waiting.
//! Every state it describes is also visible in the app and in the tray, because
//! macOS and Windows both let the user switch notifications off, and a security
//! tool that goes quiet when that happens has stopped reporting rather than
//! stopped finding.
//!
//! Clicking a notification navigates rather than acting: nothing is quarantined
//! on the strength of a click. The frontend registers the plugin's `onAction` and
//! calls back into [`open_target`], because closing the window only hides it and
//! a webview that is not there cannot bring itself back.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    let result = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .auto_cancel()
        .show();
    if let Err(error) = result {
        // Losing a notification is not worth a dialog, but it is worth a line in
        // the log a person sends us when "squid never told me".
        eprintln!("notification could not be shown: {error}");
    }
}

/// Where a notification click leads: the findings, and nothing else. Findings are
/// the only thing squid ever notifies about, so the tap needs no destination in its
/// payload, and the same path the tray menu uses brings the window forward.
pub fn open_target(app: &AppHandle) {
    crate::navigate_to(app, "security");
}
