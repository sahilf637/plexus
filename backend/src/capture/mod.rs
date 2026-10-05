//! Getting things into Plexus: the global shortcut, working out what was pasted, and saving it.

#[cfg(target_os = "linux")]
mod desktop_entry;
pub mod clipboard;
pub mod detect;
pub mod hotkey;
pub mod secrets;

use std::path::Path;

use serde::Serialize;

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
    /// Not saved: it looks like a secret. Saving again with `force` stores it anyway.
    Secret { reason: &'static str },
}

pub fn preview_text(text: &str) -> Preview {
    Preview { detected: detect::detect_text(text), secret: secrets::find_secret(text) }
}

// Saving is a stand-in until the library and index exist (M0 step 4): it reports what would be
// saved without writing anything.

pub fn save_text(text: &str, force: bool) -> CaptureResult {
    if !force && let Some(reason) = secrets::find_secret(text) {
        return CaptureResult::Secret { reason };
    }
    let detected = detect::detect_text(text);
    tracing::info!(kind = ?detected.kind, title = %detected.title, "capture (not stored yet)");
    CaptureResult::Saved { id: String::new(), title: detected.title, kind: detected.kind }
}

pub fn save_file(path: &Path) -> CaptureResult {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    save_bytes_named(&name, mime_from_name(&name), None)
}

pub fn save_bytes(name: &str, mime: &str, bytes: &[u8]) -> CaptureResult {
    save_bytes_named(name, mime, Some(bytes.len()))
}

fn save_bytes_named(name: &str, mime: &str, size: Option<usize>) -> CaptureResult {
    let detected = detect::detect_file(name, mime);
    tracing::info!(kind = ?detected.kind, name, mime, size, "capture (not stored yet)");
    CaptureResult::Saved { id: String::new(), title: detected.title, kind: detected.kind }
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
