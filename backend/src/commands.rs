//! The command surface the UI can call. Each command is a thin wrapper over a module function.

use std::path::PathBuf;

use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, State};

use crate::capture::{self, CaptureResult, Preview, clipboard, hotkey, save};
use crate::db::items::{self, Item};
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
pub fn capture_text(app: AppHandle, state: State<'_, AppState>, text: String, force: bool) -> Result<CaptureResult> {
    timed("text", || save::text(&state.library, &state.db, &text, force)).map(|saved| capture::announce(&app, saved))
}

/// Files dropped onto a window, or copied in a file manager, by path.
#[tauri::command]
pub fn capture_file(app: AppHandle, state: State<'_, AppState>, path: PathBuf) -> Result<CaptureResult> {
    timed("file", || save::file_from_disk(&state.library, &state.db, &path)).map(|saved| capture::announce(&app, saved))
}

/// Pasted images and files, sent as raw bytes with `x-name` / `x-mime` headers.
#[tauri::command]
pub fn capture_bytes(app: AppHandle, state: State<'_, AppState>, request: Request<'_>) -> Result<CaptureResult> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(Error::Invalid("expected the file's bytes".into()));
    };
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    let name = clipboard::percent_decode(header("x-name"));
    let mime = match header("x-mime") {
        "" => capture::mime_from_name(&name).to_string(),
        mime => mime.to_string(),
    };
    timed("bytes", || save::file(&state.library, &state.db, &name, &mime, bytes)).map(|saved| capture::announce(&app, saved))
}

/// One item with its note body, for the detail panel.
#[derive(serde::Serialize)]
pub struct ItemDetail {
    #[serde(flatten)]
    item: Item,
    body: String,
    /// Absolute path of the original file, if any, for opening or previewing it.
    asset_file: Option<PathBuf>,
}

#[tauri::command]
pub fn get_item(state: State<'_, AppState>, id: String) -> Result<ItemDetail> {
    let item = state.db.with(|conn| items::get(conn, &id))?.ok_or_else(|| Error::Invalid(format!("no item {id}")))?;
    let body = state.library.read_note(&state.library.absolute(&item.path))?.body;
    let asset_file = item.asset_path.as_deref().map(|p| state.library.absolute(p));
    Ok(ItemDetail { item, body, asset_file })
}

/// Everything in the index, for the map.
#[tauri::command]
pub fn list_items(state: State<'_, AppState>) -> Result<Vec<Item>> {
    state.db.with(|conn| items::all(conn))
}

/// Saving is meant to stay well under 50 ms; log how long it takes.
fn timed<T>(what: &str, save: impl FnOnce() -> Result<T>) -> Result<T> {
    let started = std::time::Instant::now();
    let result = save();
    tracing::debug!(what, elapsed = ?started.elapsed(), ok = result.is_ok(), "capture saved");
    result
}
