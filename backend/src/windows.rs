//! Showing and hiding the two windows: the map (`main`) and the capture window.
//! Both are created at startup and only hidden, never closed, so showing them is instant.

use tauri::{AppHandle, Emitter, Manager};

pub const MAIN: &str = "main";
pub const CAPTURE: &str = "capture";

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn show_capture(app: &AppHandle) {
    show_capture_activated(app, None);
}

/// Shows the capture window with the keyboard focus in it. On Wayland the desktop only hands
/// focus to a window that presents the activation token from the event that triggered it
/// (here, the global shortcut); without one GNOME shows a "Plexus is ready" notice instead.
pub fn show_capture_activated(app: &AppHandle, activation_token: Option<String>) {
    let Some(window) = app.get_webview_window(CAPTURE) else { return };
    #[cfg(target_os = "linux")]
    if let Some(token) = activation_token {
        let window = window.clone();
        let _ = app.run_on_main_thread(move || {
            use gtk::prelude::GtkWindowExt;
            if let Ok(gtk_window) = window.gtk_window() {
                gtk_window.set_startup_id(&token);
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = activation_token;

    let _ = window.center();
    let _ = window.show();
    let _ = window.set_focus();
    // Lets the page reset itself and focus the paste zone.
    let _ = app.emit_to(CAPTURE, "capture-opened", ());
}

pub fn hide_capture(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(CAPTURE) {
        let _ = window.hide();
    }
}
