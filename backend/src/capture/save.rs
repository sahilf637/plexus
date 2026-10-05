//! Turning a capture into a note file (plus its original, for images and files) and an index
//! row. This is the "T0" step: it has to be fast, so anything slow (embedding, linking,
//! fetching) happens later in the background.

use std::fs;
use std::path::Path;

use chrono::{SecondsFormat, Utc};

use super::detect::{self, Detected, Kind};
use super::{CaptureResult, secrets};
use crate::db::Db;
use crate::db::items::{self, Item};
use crate::error::Result;
use crate::library::note::{FORMAT_VERSION, Note, NoteMeta};
use crate::library::reconcile::{item_from_note, modified_nanos};
use crate::library::{self, Library};

/// A saved capture: what the UI is told, and the new index row (when something was saved).
pub struct Saved {
    pub result: CaptureResult,
    pub item: Option<Item>,
}

impl Saved {
    fn not_saved(result: CaptureResult) -> Self {
        Self { result, item: None }
    }
}

pub fn text(library: &Library, db: &Db, text: &str, force: bool) -> Result<Saved> {
    if !force && let Some(reason) = secrets::find_secret(text) {
        return Ok(Saved::not_saved(CaptureResult::Secret { reason }));
    }
    let detected = detect::detect_text(text);
    let body = body_for_text(text, &detected);
    let body_hash = library::text_hash(&body);
    let source = (detected.kind == Kind::Url).then(|| text.trim().to_string());
    let existing = db.with(|conn| match &source {
        Some(url) => items::find_link(conn, url),
        None => items::find_duplicate(conn, &body_hash, None),
    })?;
    if let Some(existing) = existing {
        return Ok(Saved::not_saved(CaptureResult::Duplicate { id: existing.id, title: existing.title }));
    }

    let meta = new_meta(&detected, body_hash, source);
    write(library, db, meta, body, None)
}

/// An image or file, from bytes (pasted) or from disk (dropped or copied in a file manager).
pub fn file(library: &Library, db: &Db, name: &str, mime: &str, bytes: &[u8]) -> Result<Saved> {
    let asset_hash = library::hash(bytes);
    if let Some(existing) = db.with(|conn| items::find_duplicate(conn, "", Some(&asset_hash)))? {
        return Ok(Saved::not_saved(CaptureResult::Duplicate { id: existing.id, title: existing.title }));
    }
    let detected = detect::detect_file(name, mime);
    // No text is extracted yet (OCR, PDF text come in M1), so images embed themselves and
    // other files have an empty body.
    let mut meta = new_meta(&detected, library::text_hash(""), None);
    meta.asset_hash = Some(asset_hash);
    meta.original_name = Some(name.to_string());
    meta.mime = Some(mime.to_string());
    meta.size = Some(bytes.len() as u64);
    write(library, db, meta, String::new(), Some((extension(name, mime), bytes)))
}

pub fn file_from_disk(library: &Library, db: &Db, path: &Path) -> Result<Saved> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = fs::read(path)?;
    file(library, db, &name, super::mime_from_name(&name), &bytes)
}

fn new_meta(detected: &Detected, hash: String, source: Option<String>) -> NoteMeta {
    NoteMeta {
        plexus: FORMAT_VERSION,
        id: ulid::Ulid::new().to_string(),
        kind: detected.kind,
        title: detected.title.clone(),
        captured: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        source,
        language: detected.language.clone(),
        hash,
        asset: None,
        asset_hash: None,
        original_name: None,
        mime: None,
        size: None,
        extracted: None,
        links: Vec::new(),
        extra: Default::default(),
    }
}

/// Writes the original (if any), then the note, then the index row. The note is the source of
/// truth, so a crash after writing it is repaired by the next reconcile.
fn write(library: &Library, db: &Db, mut meta: NoteMeta, mut body: String, asset: Option<(String, &[u8])>) -> Result<Saved> {
    let path = library.path_for(&meta);

    if let Some((ext, bytes)) = asset {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let name = format!("{stem}.{ext}");
        crate::settings::write_atomic(&path.with_file_name(&name), bytes)?;
        if meta.kind == Kind::Image {
            body = format!("![{}]({name})\n", meta.title.replace(['[', ']'], ""));
        }
        meta.asset = Some(name);
    }

    let note = Note { meta, body };
    library.write_note(&path, &note)?;
    let stat = fs::metadata(&path)?;
    let item = item_from_note(library, &path, &note, modified_nanos(&stat), stat.len() as i64);
    db.with(|conn| items::upsert(conn, &item, &note.body))?;

    let result = CaptureResult::Saved { id: item.id.clone(), title: item.title.clone(), kind: item.kind };
    Ok(Saved { result, item: Some(item) })
}

/// The note body for pasted text. Code goes in a fenced block (with its language) so the note
/// renders properly in any markdown app.
fn body_for_text(text: &str, detected: &Detected) -> String {
    let text = text.replace("\r\n", "\n");
    let trimmed = text.trim_matches('\n');
    let body = match detected.kind {
        Kind::Code if !trimmed.trim_start().starts_with("```") => {
            let fence = if trimmed.contains("```") { "````" } else { "```" };
            format!("{fence}{}\n{trimmed}\n{fence}", detected.language.as_deref().unwrap_or(""))
        }
        Kind::Url => trimmed.trim().to_string(),
        _ => trimmed.to_string(),
    };
    format!("{body}\n")
}

/// File extension for the stored original: from the name, else from the MIME type.
fn extension(name: &str, mime: &str) -> String {
    let from_name = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()));
    from_name.unwrap_or_else(|| {
        match mime {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "application/pdf" => "pdf",
            _ => "bin",
        }
        .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Library, Db) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        (dir, library, db)
    }

    fn saved_item(saved: Saved) -> Item {
        assert!(matches!(saved.result, CaptureResult::Saved { .. }));
        saved.item.unwrap()
    }

    #[test]
    fn saves_text_as_a_note_and_indexes_it() {
        let (_dir, library, db) = setup();
        let item = saved_item(text(&library, &db, "Ownership rules\n\nEach value has one owner.\r\n", false).unwrap());
        assert_eq!(item.title, "Ownership rules");
        assert!(item.path.starts_with("items/") && item.path.contains("/ownership-rules-"));

        let note = library.read_note(&library.absolute(&item.path)).unwrap();
        assert_eq!(note.body, "Ownership rules\n\nEach value has one owner.\n");
        assert_eq!(note.meta.id, item.id);
        assert_eq!(db.with(|conn| items::get(conn, &item.id)).unwrap(), Some(item));
    }

    #[test]
    fn code_is_fenced_and_links_keep_their_source() {
        let (_dir, library, db) = setup();
        let code = saved_item(text(&library, &db, "fn main() {\n    let mut x = 5;\n}", false).unwrap());
        let note = library.read_note(&library.absolute(&code.path)).unwrap();
        assert_eq!(note.body, "```rust\nfn main() {\n    let mut x = 5;\n}\n```\n");
        assert_eq!(note.meta.language.as_deref(), Some("rust"));

        let link = saved_item(text(&library, &db, " https://example.com/post ", false).unwrap());
        assert_eq!(link.source_url.as_deref(), Some("https://example.com/post"));
    }

    #[test]
    fn duplicates_and_secrets_are_not_saved() {
        let (_dir, library, db) = setup();
        let first = saved_item(text(&library, &db, "same thing", false).unwrap());
        let again = text(&library, &db, "  same thing\r\n", false).unwrap();
        assert!(matches!(again.result, CaptureResult::Duplicate { ref id, .. } if *id == first.id));

        let secret = text(&library, &db, "token = ghp_abcdefghijklmnopqrstuvwxyz0123456789", false).unwrap();
        assert!(matches!(secret.result, CaptureResult::Secret { .. }) && secret.item.is_none());
        assert!(text(&library, &db, "token = ghp_abcdefghijklmnopqrstuvwxyz0123456789", true).unwrap().item.is_some());
        assert_eq!(db.with(|conn| items::count(conn)).unwrap(), 2);
    }

    #[test]
    fn files_are_stored_next_to_their_note() {
        let (_dir, library, db) = setup();
        let png = b"\x89PNG fake image bytes";
        let item = saved_item(file(&library, &db, "Whiteboard sketch.png", "image/png", png).unwrap());
        assert_eq!(item.kind, Kind::Image);
        let asset = item.asset_path.clone().unwrap();
        assert_eq!(asset.trim_end_matches(".png"), item.path.trim_end_matches(".md"));
        assert_eq!(fs::read(library.absolute(&asset)).unwrap(), png);

        let note = library.read_note(&library.absolute(&item.path)).unwrap();
        assert_eq!(note.meta.original_name.as_deref(), Some("Whiteboard sketch.png"));
        assert!(note.body.starts_with("![Whiteboard sketch]("));

        let again = file(&library, &db, "copy.png", "image/png", png).unwrap();
        assert!(matches!(again.result, CaptureResult::Duplicate { .. }));
        let pasted = saved_item(file(&library, &db, "", "image/png", b"other").unwrap());
        assert!(pasted.asset_path.unwrap().ends_with(".png"));
    }

    #[test]
    fn a_rebuilt_index_matches_what_was_saved() {
        let (_dir, library, db) = setup();
        let item = saved_item(file(&library, &db, "report.pdf", "application/pdf", b"%PDF").unwrap());
        drop(db);
        fs::remove_file(library.index_path()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        crate::library::reconcile::reconcile(&library, &db).unwrap();
        assert_eq!(db.with(|conn| items::get(conn, &item.id)).unwrap(), Some(item));
    }
}
