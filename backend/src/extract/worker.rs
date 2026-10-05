//! Reading the text out of files and images in the background. A dropped PDF or screenshot is
//! saved at once as a raw file; a moment later its note gets the text, and only then is it
//! linked, so it is related by what it says rather than by its name.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager};

use super::{NONE, Outcome, join_body, split_body};
use super::ocr::Ocr;
use crate::capture::detect::Kind;
use crate::db::Db;
use crate::db::items::{self, Item};
use crate::library::Library;
use crate::error::{Error, Result};
use crate::library::reconcile::{item_from_note, modified_nanos};
use crate::state::AppState;

/// How long the OCR models stay loaded after the last image.
const IDLE_UNLOAD: std::time::Duration = std::time::Duration::from_secs(120);

pub struct Extractor {
    queue: Sender<String>,
    pub ocr: Arc<Ocr>,
}

impl Extractor {
    /// The extractor plus the receiving end of its queue, for [`run`].
    pub fn new(ocr: Ocr) -> (Self, Receiver<String>) {
        let (queue, receiver) = mpsc::channel();
        (Self { queue, ocr: Arc::new(ocr) }, receiver)
    }

    pub fn enqueue(&self, id: &str) {
        let _ = self.queue.send(id.to_string());
    }
}

/// Whether the item is an original whose text hasn't been read yet (or was read by an older
/// version of its extractor).
pub fn needs_extraction(item: &Item) -> bool {
    matches!(item.kind, Kind::File | Kind::Image)
        && item.asset_path.is_some()
        && item.extracted.as_deref().is_none_or(|stamp| !super::is_current(stamp))
}

/// Starts the worker that reads originals as they arrive. One worker: extraction is CPU-bound.
pub fn run(app: &AppHandle, queue: Receiver<String>) {
    let app = app.clone();
    let worker = move || {
        loop {
            match queue.recv_timeout(IDLE_UNLOAD) {
                Ok(id) => {
                    if let Err(e) = process(&app, &id) {
                        tracing::warn!(id, "couldn't read the file's text: {e}");
                        // Link it as it is rather than leave it waiting.
                        app.state::<AppState>().pipeline.enqueue(&id);
                    }
                }
                Err(RecvTimeoutError::Timeout) => app.state::<AppState>().extractor.ocr.unload(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    };
    if let Err(e) = std::thread::Builder::new().name("extract".into()).spawn(worker) {
        tracing::error!("couldn't start the text reader: {e}");
    }
}

fn process(app: &AppHandle, id: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let started = Instant::now();
    // Not read (gone, nothing to do, or no extractor now): it is linked as it is.
    if let Some(updated) = extract_item(&state.library, &state.db, &state.extractor.ocr, id)? {
        tracing::debug!(id, stamp = ?updated.extracted, elapsed = ?started.elapsed(), "text read from a file");
        let _ = app.emit("item-updated", &updated);
    }
    state.pipeline.enqueue(id);
    Ok(())
}

/// Reads the item's original into its note and the index. Returns the updated row, or `None`
/// when there was nothing to do: the item is gone or already read, or the extractor can't run
/// yet (its models are missing), in which case it is tried again at the next start.
pub fn extract_item(library: &Library, db: &Db, ocr: &Ocr, id: &str) -> Result<Option<Item>> {
    let Some(item) = db.with(|conn| items::get(conn, id))? else { return Ok(None) };
    if !needs_extraction(&item) {
        return Ok(None);
    }
    let note_path = library.absolute(&item.path);
    let mut note = library.read_note(&note_path)?;
    let asset = library.absolute(item.asset_path.as_deref().unwrap_or_default());
    let name = note
        .meta
        .original_name
        .clone()
        .or_else(|| asset.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let mime = note.meta.mime.clone().unwrap_or_default();

    let (stamp, text) = match super::extract(&asset, &name, &mime, ocr)? {
        Outcome::Text { tool, text } => (tool, Some(text)),
        Outcome::Empty(tool) => (tool, None),
        Outcome::Unsupported => (NONE, None),
        Outcome::Unavailable => return Ok(None),
    };
    let (own, _) = split_body(&note.body);
    note.body = join_body(own, text.as_deref());
    note.meta.extracted = Some(stamp.into());
    library.write_note(&note_path, &note).map_err(|e| Error::Invalid(format!("couldn't save the text: {e}")))?;

    let stat = std::fs::metadata(&note_path)?;
    let updated = item_from_note(library, &note_path, &note, modified_nanos(&stat), stat.len() as i64);
    db.with(|conn| items::upsert(conn, &updated, &note.body))?;
    Ok(Some(updated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::save;
    use crate::db::items::sample;
    use crate::extract::MARKER;

    #[test]
    fn only_unread_originals_need_extraction() {
        let mut item = sample("a", "items/a.md", "h");
        item.kind = Kind::File;
        assert!(!needs_extraction(&item), "no original");
        item.asset_path = Some("items/a.pdf".into());
        assert!(needs_extraction(&item));
        for stamp in ["pdf-text@1", "none@1", "edited"] {
            item.extracted = Some(stamp.into());
            assert!(!needs_extraction(&item), "{stamp}");
        }
        item.extracted = Some("ocr@0".into());
        assert!(needs_extraction(&item), "older extractor");
        item.kind = Kind::Text;
        assert!(!needs_extraction(&item));
    }

    fn setup() -> (tempfile::TempDir, Library, Db) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        (dir, library, db)
    }

    fn save_file(library: &Library, db: &Db, name: &str, mime: &str, bytes: &[u8], note: &str) -> Item {
        save::file(library, db, name, mime, bytes, note, false).unwrap().item.unwrap()
    }

    #[test]
    fn a_text_file_gets_its_text_after_the_users_note() {
        let (_dir, library, db) = setup();
        let item = save_file(&library, &db, "plan.md", "text/markdown", b"# Q4 plan\n\nShip the importer.\n", "my reminder");
        assert!(needs_extraction(&item));
        let ocr = Ocr::new(None);
        let updated = extract_item(&library, &db, &ocr, &item.id).unwrap().unwrap();
        assert_eq!(updated.extracted.as_deref(), Some("text@1"));

        let note = library.read_note(&library.absolute(&updated.path)).unwrap();
        assert_eq!(note.body, format!("my reminder\n\n{MARKER}\n# Q4 plan\n\nShip the importer.\n"));
        // It is searchable by what the file says, and not read twice.
        assert_eq!(db.with(|conn| crate::db::graph::search(conn, "importer", 5)).unwrap(), vec![item.id.clone()]);
        assert!(extract_item(&library, &db, &ocr, &item.id).unwrap().is_none());
        // Regenerating after a newer extractor keeps the user's note.
        let mut stale = updated.clone();
        stale.extracted = Some("text@0".into());
        db.with(|conn| items::upsert(conn, &stale, &note.body)).unwrap();
        let again = extract_item(&library, &db, &ocr, &item.id).unwrap().unwrap();
        assert_eq!(library.read_note(&library.absolute(&again.path)).unwrap().body, note.body);
    }

    #[test]
    fn unsupported_files_are_marked_and_images_wait_for_ocr() {
        let (_dir, library, db) = setup();
        let song = save_file(&library, &db, "song.mp3", "audio/mpeg", b"ID3....", "");
        let updated = extract_item(&library, &db, &Ocr::new(None), &song.id).unwrap().unwrap();
        assert_eq!(updated.extracted.as_deref(), Some("none@1"));
        assert!(!needs_extraction(&updated));

        let png = save_file(&library, &db, "shot.png", "image/png", b"not really a png", "");
        // No OCR models: left alone, to be read at a later start.
        assert!(extract_item(&library, &db, &Ocr::new(None), &png.id).unwrap().is_none());
        assert!(db.with(|conn| items::get(conn, &png.id)).unwrap().unwrap().extracted.is_none());
    }

    #[test]
    fn a_pdf_gets_its_pages() {
        let (_dir, library, db) = setup();
        let item = save_file(&library, &db, "report.pdf", "application/pdf", &tiny_pdf("Quarterly revenue grew eighteen percent"), "");
        let updated = extract_item(&library, &db, &Ocr::new(None), &item.id).unwrap().unwrap();
        let note = library.read_note(&library.absolute(&updated.path)).unwrap();
        assert_eq!(updated.extracted.as_deref(), Some("pdf-text@1"), "{}", note.body);
        assert!(note.body.contains("<!-- page 1 -->") && note.body.contains("Quarterly revenue"), "{}", note.body);
    }

    /// A one-page PDF with a line of text, written by hand (the reader rebuilds its table).
    fn tiny_pdf(text: &str) -> Vec<u8> {
        let content = format!("BT /F1 18 Tf 72 700 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
        }
        let xref = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1));
        for offset in offsets {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1));
        pdf.into_bytes()
    }
}
