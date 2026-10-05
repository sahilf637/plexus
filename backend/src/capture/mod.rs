//! Getting things into Plexus: the global shortcut, working out what was pasted, and saving it.

pub mod clipboard;
#[cfg(target_os = "linux")]
mod desktop_entry;
pub mod detect;
pub mod hotkey;
pub mod save;
pub mod secrets;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::Result;
use crate::state::AppState;
use detect::{Detected, Kind};

/// What the capture window shows before saving.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    #[serde(flatten)]
    pub detected: Detected,
    /// Set when the text looks like a password or key: "a GitHub token", …
    pub secret: Option<&'static str>,
}

/// The outcome of a save, for the capture window.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum CaptureResult {
    Saved { id: String, title: String, kind: Kind },
    /// Not saved: the same thing was saved before.
    Duplicate { id: String, title: String },
    /// Not saved: it looks like a secret. Saving again with `force` stores it anyway.
    Secret { reason: &'static str },
}

pub fn preview_text(text: &str) -> Preview {
    Preview { detected: detect::detect_text(text), secret: secrets::find_secret(text) }
}

/// Tells every window about a newly saved item (the map adds it straight away) and hands the
/// result back for the caller.
pub fn announce(app: &AppHandle, saved: save::Saved) -> CaptureResult {
    if let Some(item) = &saved.item {
        let _ = app.emit("item-added", item);
    }
    saved.result
}

/// "Save clipboard directly": saves whatever is on the clipboard without opening a window, and
/// reports the outcome as a desktop notification.
pub fn save_clipboard(app: &AppHandle) {
    let state = app.state::<AppState>();
    let outcome = read_and_save_clipboard(&state).map(|saved| saved.map(|saved| announce(app, saved)));
    let message = match outcome {
        Ok(Some(CaptureResult::Saved { title, .. })) => format!("Saved “{title}”"),
        Ok(Some(CaptureResult::Duplicate { title, .. })) => format!("Already saved as “{title}”"),
        Ok(Some(CaptureResult::Secret { reason })) => {
            format!("Not saved: this looks like {reason}. Paste it into the Plexus window to keep it anyway.")
        }
        Ok(None) => "The clipboard is empty.".to_string(),
        Err(e) => format!("Couldn't save the clipboard: {e}"),
    };
    notify(app, &message);
}

fn read_and_save_clipboard(state: &AppState) -> Result<Option<save::Saved>> {
    let (library, db) = (&state.library, &state.db);
    if let Some(text) = clipboard::text()? {
        if let Some(path) = clipboard::existing_files(&text).first() {
            return save::file_from_disk(library, db, path).map(Some);
        }
        return save::text(library, db, &text, false).map(Some);
    }
    match clipboard::image_png()? {
        Some(png) => save::file(library, db, "Pasted image.png", "image/png", &png).map(Some),
        None => Ok(None),
    }
}

fn notify(app: &AppHandle, message: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title("Plexus").body(message).show() {
        tracing::warn!("couldn't show a notification: {e}");
    }
}

/// A best guess from the extension, for files dropped from disk.
pub fn mime_from_name(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "epub" => "application/epub+zip",
        "md" | "markdown" => "text/markdown",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}
