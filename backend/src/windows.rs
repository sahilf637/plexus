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
    if let Some(window) = app.get_webview_window(CAPTURE) {
        let _ = window.center();
        let _ = window.show();
        let _ = window.set_focus();
        // Lets the page reset itself and focus the paste zone.
        let _ = app.emit_to(CAPTURE, "capture-opened", ());
    }
}

pub fn hide_capture(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(CAPTURE) {
        let _ = window.hide();
    }
}
