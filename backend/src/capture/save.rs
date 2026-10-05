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
use crate::error::{Error, Result};
use crate::library::note::{Attachment, FORMAT_VERSION, Note, NoteMeta};
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
    rich_text(library, db, text, None, force)
}

/// Pasted text that may also come as HTML (copied from a web page or a document): when it has
/// real structure (headings, lists, links, tables) the note keeps it as markdown. Code and
/// links are saved from the plain text as usual.
pub fn rich_text(library: &Library, db: &Db, text: &str, html: Option<&str>, force: bool) -> Result<Saved> {
    if !force && let Some(reason) = secrets::find_secret(text) {
        return Ok(Saved::not_saved(CaptureResult::Secret { reason }));
    }
    let mut detected = detect::detect_text(text);
    let markdown = html.filter(|_| detected.kind == Kind::Text).and_then(markdown_from_html);
    let body = match &markdown {
        Some(markdown) => {
            detected = Detected { kind: Kind::Text, language: None, title: detect::detect_text(markdown).title };
            format!("{markdown}\n")
        }
        None => body_for_text(text, &detected),
    };
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
    write(library, db, meta, body, &[])
}

/// Markdown for clipboard HTML, when it has structure worth keeping (otherwise `None`, and the
/// plain text is used). Browsers wrap the copied part in fragment comments.
fn markdown_from_html(html: &str) -> Option<String> {
    static STRUCTURE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)<(h[1-6]|ul|ol|li|a|table|pre|blockquote|strong|b|em|i)[\s>]").unwrap()
    });
    let fragment = match (html.find("<!--StartFragment-->"), html.find("<!--EndFragment-->")) {
        (Some(start), Some(end)) if start < end => &html[start + "<!--StartFragment-->".len()..end],
        _ => html,
    };
    if !STRUCTURE.is_match(fragment) {
        return None;
    }
    let markdown = crate::extract::html_to_markdown(fragment)?;
    let markdown = markdown.trim();
    (markdown.chars().filter(|c| c.is_alphabetic()).count() >= 3).then(|| markdown.to_string())
}

/// An image or file to save, as captured.
pub struct Original<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    pub bytes: &'a [u8],
}

/// An image or file, from bytes (pasted) or from disk (dropped or copied in a file manager),
/// with an optional note about it from the user. The note becomes the body (below the image)
/// and its first line the title.
pub fn file(library: &Library, db: &Db, name: &str, mime: &str, bytes: &[u8], note: &str, force: bool) -> Result<Saved> {
    files(library, db, &[Original { name, mime, bytes }], note, force)
}

pub fn file_from_disk(library: &Library, db: &Db, path: &Path, note: &str, force: bool) -> Result<Saved> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = fs::read(path)?;
    file(library, db, &name, super::mime_from_name(&name), &bytes, note, force)
}

/// Several images and files captured together with one note about them: one item, with every
/// original next to its note. The first one decides the item's kind. Only a single original is
/// checked for duplicates; a set saved with a note is a new item each time.
pub fn files(library: &Library, db: &Db, originals: &[Original], note: &str, force: bool) -> Result<Saved> {
    let Some(first) = originals.first() else {
        return Err(Error::Invalid("nothing to save".into()));
    };
    let note = note.replace("\r\n", "\n");
    let note = note.trim();
    if !force && let Some(reason) = secrets::find_secret(note) {
        return Ok(Saved::not_saved(CaptureResult::Secret { reason }));
    }
    let asset_hash = library::hash(first.bytes);
    if originals.len() == 1
        && let Some(existing) = db.with(|conn| items::find_duplicate(conn, "", Some(&asset_hash)))?
    {
        return Ok(Saved::not_saved(CaptureResult::Duplicate { id: existing.id, title: existing.title }));
    }
    let mut detected = detect::detect_file(first.name, first.mime);
    if !note.is_empty() {
        detected.title = detect::detect_text(note).title;
    } else if originals.len() > 1 {
        detected.title = format!("{} and {} more", detected.title, originals.len() - 1);
    }
    // The text of the files is read in the background afterwards (`extract`), so the body starts
    // as just the user's note.
    let body = if note.is_empty() { String::new() } else { format!("{note}\n") };
    let mut meta = new_meta(&detected, library::text_hash(&body), None);
    meta.asset_hash = Some(asset_hash);
    meta.original_name = Some(first.name.to_string()).filter(|n| !n.is_empty());
    meta.mime = Some(first.mime.to_string());
    meta.size = Some(first.bytes.len() as u64);
    write(library, db, meta, body, originals)
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
        title_by: None,
        title_was: None,
        attachments: Vec::new(),
        links: Vec::new(),
        extra: Default::default(),
    }
}

/// Writes the original (if any), then the note, then the index row. The note is the source of
/// truth, so a crash after writing it is repaired by the next reconcile.
fn write(library: &Library, db: &Db, mut meta: NoteMeta, mut body: String, originals: &[Original]) -> Result<Saved> {
    let path = library.path_for(&meta);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();

    // The originals first, then the note about them, as the user saw them in the capture window.
    let mut embeds = Vec::new();
    for (i, original) in originals.iter().enumerate() {
        let ext = extension(original.name, original.mime);
        // Slugs never contain "_", so these names can't clash with another note's.
        let file = if i == 0 { format!("{stem}.{ext}") } else { format!("{stem}_{}.{ext}", i + 1) };
        crate::settings::write_atomic(&path.with_file_name(&file), original.bytes)?;
        let label = if i == 0 || original.name.is_empty() { meta.title.as_str() } else { original.name };
        let label = label.replace(['[', ']'], "");
        if detect::detect_file(original.name, original.mime).kind == Kind::Image {
            embeds.push(format!("![{label}]({file})"));
        } else if originals.len() > 1 {
            embeds.push(format!("[{label}]({file})"));
        }
        if i == 0 {
            meta.asset = Some(file);
        } else {
            meta.attachments.push(Attachment {
                file,
                hash: library::hash(original.bytes),
                original_name: Some(original.name.to_string()).filter(|n| !n.is_empty()),
                mime: Some(original.mime.to_string()),
                size: Some(original.bytes.len() as u64),
            });
        }
    }
    if !embeds.is_empty() {
        let embeds = embeds.join("\n") + "\n";
        body = if body.is_empty() { embeds } else { format!("{embeds}\n{body}") };
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

/// File extension for the stored original: from the name, else from the MIME type. Never `md`:
/// the original sits next to its note, which is `<same name>.md`, and the library treats every
/// `.md` file as a note.
fn extension(name: &str, mime: &str) -> String {
    let from_name = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()));
    let ext = from_name.unwrap_or_else(|| {
        match mime {
            "text/markdown" => "markdown",
            "text/plain" => "txt",
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "application/pdf" => "pdf",
            _ => "bin",
        }
        .to_string()
    });
    if ext == "md" { "markdown".to_string() } else { ext }
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
    fn rich_html_keeps_its_structure_as_markdown() {
        let (_dir, library, db) = setup();
        let html = "<meta charset='utf-8'><!--StartFragment--><h2>Sourdough basics</h2><ul><li>Flour</li><li><a href=\"https://example.com\">Starter</a></li></ul><!--EndFragment-->";
        let item = saved_item(rich_text(&library, &db, "Sourdough basics\nFlour\nStarter", Some(html), false).unwrap());
        assert_eq!((item.kind, item.title.as_str()), (Kind::Text, "Sourdough basics"));
        let note = library.read_note(&library.absolute(&item.path)).unwrap();
        assert!(note.body.contains("## Sourdough basics") && note.body.contains("[Starter](https://example.com)"), "{}", note.body);

        // HTML with no structure, or for code, is ignored: the plain text is what was copied.
        let plain = saved_item(rich_text(&library, &db, "just some words", Some("<div><span>just some words</span></div>"), false).unwrap());
        assert_eq!(library.read_note(&library.absolute(&plain.path)).unwrap().body, "just some words\n");
        let code = saved_item(rich_text(&library, &db, "fn main() {\n    let x = 5;\n}", Some("<pre><b>fn</b> main</pre>"), false).unwrap());
        assert_eq!(code.kind, Kind::Code);
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
        let item = saved_item(file(&library, &db, "Whiteboard sketch.png", "image/png", png, "", false).unwrap());
        assert_eq!(item.kind, Kind::Image);
        let asset = item.asset_path.clone().unwrap();
        assert_eq!(asset.trim_end_matches(".png"), item.path.trim_end_matches(".md"));
        assert_eq!(fs::read(library.absolute(&asset)).unwrap(), png);

        let note = library.read_note(&library.absolute(&item.path)).unwrap();
        assert_eq!(note.meta.original_name.as_deref(), Some("Whiteboard sketch.png"));
        assert!(note.body.starts_with("![Whiteboard sketch]("));

        let again = file(&library, &db, "copy.png", "image/png", png, "", false).unwrap();
        assert!(matches!(again.result, CaptureResult::Duplicate { .. }));
        let pasted = saved_item(file(&library, &db, "", "image/png", b"other", "", false).unwrap());
        assert!(pasted.asset_path.unwrap().ends_with(".png"));
    }

    #[test]
    fn a_note_about_an_image_goes_below_it_and_names_it() {
        let (_dir, library, db) = setup();
        let note = "Sprint board after planning\r\nThe API work slipped a week.";
        let item = saved_item(file(&library, &db, "Screenshot 2026.png", "image/png", b"png", note, false).unwrap());
        assert_eq!(item.title, "Sprint board after planning");
        let saved = library.read_note(&library.absolute(&item.path)).unwrap();
        let asset = saved.meta.asset.unwrap();
        assert_eq!(
            saved.body,
            format!("![Sprint board after planning]({asset})\n\nSprint board after planning\nThe API work slipped a week.\n")
        );
        assert_eq!(saved.meta.original_name.as_deref(), Some("Screenshot 2026.png"));

        // A secret in the note is caught like in pasted text.
        let secret = file(&library, &db, "x.png", "image/png", b"x", "key ghp_abcdefghijklmnopqrstuvwxyz0123456789", false).unwrap();
        assert!(matches!(secret.result, CaptureResult::Secret { .. }));
    }

    #[test]
    fn several_originals_with_a_note_make_one_item() {
        let (_dir, library, db) = setup();
        let originals = [
            Original { name: "board.png", mime: "image/png", bytes: b"one" },
            Original { name: "Screenshot 2.png", mime: "image/png", bytes: b"two" },
            Original { name: "Q3 report.pdf", mime: "application/pdf", bytes: b"%PDF" },
        ];
        let item = saved_item(files(&library, &db, &originals, "Sprint review\nAPI slipped.", false).unwrap());
        assert_eq!((item.title.as_str(), item.kind), ("Sprint review", Kind::Image));

        let note = library.read_note(&library.absolute(&item.path)).unwrap();
        let primary = note.meta.asset.clone().unwrap();
        let stem = primary.trim_end_matches(".png");
        let extra: Vec<_> = note.meta.attachments.iter().map(|a| a.file.as_str()).collect();
        assert_eq!(extra, vec![format!("{stem}_2.png"), format!("{stem}_3.pdf")]);
        assert_eq!(note.meta.attachments[1].original_name.as_deref(), Some("Q3 report.pdf"));
        assert_eq!(
            note.body,
            format!(
                "![Sprint review]({primary})\n![Screenshot 2.png]({stem}_2.png)\n[Q3 report.pdf]({stem}_3.pdf)\n\nSprint review\nAPI slipped.\n"
            )
        );
        let folder = library.absolute(&item.path).with_file_name("");
        assert_eq!(fs::read(folder.join(format!("{stem}_3.pdf"))).unwrap(), b"%PDF");

        // The note's metadata survives a rebuild of the index.
        let again = Note::parse(&fs::read_to_string(library.absolute(&item.path)).unwrap()).unwrap();
        assert_eq!(again.meta.attachments, note.meta.attachments);
    }

    #[test]
    fn a_rebuilt_index_matches_what_was_saved() {
        let (_dir, library, db) = setup();
        let item = saved_item(file(&library, &db, "report.pdf", "application/pdf", b"%PDF", "Q3 numbers", false).unwrap());
        drop(db);
        fs::remove_file(library.index_path()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        crate::library::reconcile::reconcile(&library, &db).unwrap();
        assert_eq!(db.with(|conn| items::get(conn, &item.id)).unwrap(), Some(item));
    }
}
