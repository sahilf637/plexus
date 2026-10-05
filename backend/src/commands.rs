//! The command surface the UI can call. Each command is a thin wrapper over a module function.

use std::path::PathBuf;

use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, State};

use crate::capture::{self, CaptureResult, Preview, clipboard, hotkey};
use crate::error::{Error, Result};
use crate::settings::Settings;
use crate::state::AppState;
use crate::windows;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.get()
}

#[tauri::command]
pub fn set_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
    let previous = state.settings.get();
    state.settings.set(settings.clone())?;
    crate::sync_autostart(&app, &settings);
    if settings.hotkey != previous.hotkey {
        hotkey::register(&app);
    }
    Ok(settings)
}

#[tauri::command]
pub fn hotkey_status(state: State<'_, AppState>) -> hotkey::HotkeyStatus {
    state.hotkey.status()
}

/// Opens the system's own dialog for changing the shortcut (Wayland desktops).
#[tauri::command]
pub fn configure_hotkey(app: AppHandle) {
    hotkey::configure(&app);
}

#[tauri::command]
pub fn show_capture(app: AppHandle) {
    windows::show_capture(&app);
}

#[tauri::command]
pub fn hide_capture(app: AppHandle) {
    windows::hide_capture(&app);
}

/// The clipboard image as PNG bytes (empty when there is none). The webview's paste event
/// doesn't carry images on Linux, so the capture window asks for them here.
#[tauri::command]
pub fn clipboard_image() -> Result<Response> {
    Ok(Response::new(clipboard::image_png()?.unwrap_or_default()))
}

/// Pasted text that is only paths to existing files (copied in a file manager) → those paths.
#[tauri::command]
pub fn pasted_files(text: String) -> Vec<PathBuf> {
    clipboard::existing_files(&text)
}

#[tauri::command]
pub fn capture_preview(text: String) -> Preview {
    capture::preview_text(&text)
}

#[tauri::command]
pub fn capture_text(text: String, force: bool) -> CaptureResult {
    capture::save_text(&text, force)
}

/// Files dropped onto a window, by path.
#[tauri::command]
pub fn capture_file(path: PathBuf) -> CaptureResult {
    capture::save_file(&path)
}

/// Pasted images and files, sent as raw bytes with `x-name` / `x-mime` headers.
#[tauri::command]
pub fn capture_bytes(request: Request<'_>) -> Result<CaptureResult> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(Error::Invalid("expected the file's bytes".into()));
    };
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    let name = clipboard::percent_decode(header("x-name"));
    let mime = match header("x-mime") {
        "" => capture::mime_from_name(&name).to_string(),
        mime => mime.to_string(),
    };
    Ok(capture::save_bytes(&name, &mime, bytes))
}
