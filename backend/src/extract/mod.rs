//! Reading the text out of files and images, so they link by what they say instead of by their
//! name. Extractors are small functions picked by file type; each result is stamped
//! `extracted: <tool>@<version>` in the note, so a better version can run again later.
//!
//! The text goes after a marker line at the end of the note's body: everything before it is the
//! user's own note, everything after it was extracted and may be regenerated (until the user
//! edits it, which sets `extracted: edited`).

mod docs;
pub mod ocr;
pub mod worker;

use std::fs;
use std::path::Path;

use crate::error::Result;
use ocr::Ocr;

/// Line in a note's body that starts the extracted text.
pub const MARKER: &str = "<!-- plexus:extracted -->";
/// Longest extracted text kept in a note (characters).
const MAX_CHARS: usize = 1_000_000;
/// Largest original read for extraction.
const MAX_BYTES: u64 = 100 * 1024 * 1024;
/// Text a scan or image must yield to count as having text (letters).
const MIN_LETTERS: usize = 20;

pub const TEXT: &str = "text@1";
pub const HTML: &str = "html@1";
pub const PDF: &str = "pdf-text@1";
pub const DOCX: &str = "docx-text@1";
pub const EPUB: &str = "epub-text@1";
pub const OCR: &str = "ocr@1";
/// Stamp for originals no extractor reads (audio, archives, …), so they aren't tried again.
pub const NONE: &str = "none@1";

/// Newest version of each extractor's stamp, to tell old results from current ones.
const CURRENT: &[&str] = &[TEXT, HTML, PDF, DOCX, EPUB, OCR];

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Text was found (already trimmed and cut to a sane length).
    Text { tool: &'static str, text: String },
    /// The type is readable but holds no text (a photo, a scanned page).
    Empty(&'static str),
    /// No extractor reads this type.
    Unsupported,
    /// An extractor exists but can't run now (its model isn't installed).
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Format {
    Text,
    Html,
    Pdf,
    Docx,
    Epub,
    Image,
}

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "rst", "org", "csv", "tsv", "json", "yaml", "yml", "toml", "ini", "log", "xml", "rs", "py",
    "js", "mjs", "ts", "tsx", "jsx", "go", "java", "kt", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "swift", "sh",
    "bash", "zsh", "sql", "lua", "css", "scss", "svelte", "vue", "tex", "bib", "conf", "cfg",
];

fn format_of(name: &str, mime: &str) -> Option<Format> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" | "xhtml" => return Some(Format::Html),
        "pdf" => return Some(Format::Pdf),
        "docx" => return Some(Format::Docx),
        "epub" => return Some(Format::Epub),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => return Some(Format::Image),
        e if TEXT_EXTENSIONS.contains(&e) => return Some(Format::Text),
        _ => {}
    }
    match mime {
        "text/html" => Some(Format::Html),
        "application/pdf" => Some(Format::Pdf),
        "application/epub+zip" => Some(Format::Epub),
        m if m.ends_with("wordprocessingml.document") => Some(Format::Docx),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp" => Some(Format::Image),
        m if m.starts_with("text/") || m == "application/json" => Some(Format::Text),
        _ => None,
    }
}

/// Whether the stamp is from the newest version of its extractor (stamps of other kinds, such
/// as `web@1` or `edited`, are never replaced).
pub fn is_current(stamp: &str) -> bool {
    let Some((tool, _)) = stamp.split_once('@') else { return true };
    !CURRENT.iter().any(|c| c.split_once('@').is_some_and(|(t, _)| t == tool)) || CURRENT.contains(&stamp)
}

/// Reads text from the original at `path`.
pub fn extract(path: &Path, name: &str, mime: &str, ocr: &Ocr) -> Result<Outcome> {
    let Some(format) = format_of(name, mime) else { return Ok(Outcome::Unsupported) };
    if fs::metadata(path)?.len() > MAX_BYTES {
        return Ok(Outcome::Unsupported);
    }
    let bytes = fs::read(path)?;
    Ok(extract_bytes(format, &bytes, ocr))
}

fn extract_bytes(format: Format, bytes: &[u8], ocr: &Ocr) -> Outcome {
    let (tool, text) = match format {
        Format::Text => {
            if bytes.contains(&0) {
                return Outcome::Unsupported; // Binary data with a text-like name.
            }
            (TEXT, Some(String::from_utf8_lossy(bytes).into_owned()))
        }
        Format::Html => (HTML, html_to_markdown(&String::from_utf8_lossy(bytes))),
        Format::Pdf => (PDF, docs::pdf(bytes)),
        Format::Docx => (DOCX, docs::docx(bytes)),
        Format::Epub => (EPUB, docs::epub(bytes)),
        Format::Image => {
            if !ocr.available() {
                return Outcome::Unavailable;
            }
            (OCR, ocr.read(bytes))
        }
    };
    let text = text.map(|t| tidy(&t)).unwrap_or_default();
    // Text files are kept whatever they hold; scans and photos need real words to count.
    let enough = match format {
        Format::Text => !text.is_empty(),
        _ => text.chars().filter(|c| c.is_alphabetic()).count() >= MIN_LETTERS,
    };
    if enough { Outcome::Text { tool, text } } else { Outcome::Empty(tool) }
}

/// Rich HTML (a pasted page, an `.html` file) as markdown, without scripts and styles.
pub fn html_to_markdown(html: &str) -> Option<String> {
    htmd::HtmlToMarkdown::builder().skip_tags(vec!["script", "style", "head", "noscript"]).build().convert(html).ok()
}

/// Trimmed, CRLF → LF, runs of blank lines squeezed, cut to [`MAX_CHARS`].
fn tidy(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\u{0}', "");
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let trimmed = out.trim();
    trimmed.chars().take(MAX_CHARS).collect()
}

/// The user's own note and the extracted text of a note body.
pub fn split_body(body: &str) -> (&str, Option<&str>) {
    match body.split_once(MARKER) {
        Some((note, extracted)) => (note.trim_end(), Some(extracted.trim())),
        None => (body, None),
    }
}

/// A body with `extracted` after the marker (none: just the user's note).
pub fn join_body(note: &str, extracted: Option<&str>) -> String {
    let note = note.trim_end();
    match extracted.filter(|t| !t.is_empty()) {
        Some(text) if note.is_empty() => format!("{MARKER}\n{text}\n"),
        Some(text) => format!("{note}\n\n{MARKER}\n{text}\n"),
        None if note.is_empty() => String::new(),
        None => format!("{note}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_extractors_by_name_then_type() {
        assert_eq!(format_of("Notes.MD", ""), Some(Format::Text));
        assert_eq!(format_of("main.rs", "application/octet-stream"), Some(Format::Text));
        assert_eq!(format_of("report", "application/pdf"), Some(Format::Pdf));
        assert_eq!(format_of("scan.JPG", ""), Some(Format::Image));
        assert_eq!(format_of("song.mp3", "audio/mpeg"), None);
        assert!(format_of("a.docx", "").is_some() && format_of("a.zip", "application/zip").is_none());
    }

    #[test]
    fn old_stamps_are_not_current_but_other_stamps_are() {
        assert!(is_current("ocr@1") && is_current("web@1") && is_current("edited") && is_current("none@1"));
        assert!(!is_current("ocr@0") && !is_current("pdf-text@0"));
    }

    #[test]
    fn body_split_and_join_round_trip() {
        let body = join_body("My note about it", Some("Line one\nLine two"));
        assert_eq!(body, format!("My note about it\n\n{MARKER}\nLine one\nLine two\n"));
        assert_eq!(split_body(&body), ("My note about it", Some("Line one\nLine two")));
        assert_eq!(split_body("plain note\n"), ("plain note\n", None));
        assert_eq!(join_body("", Some("text")), format!("{MARKER}\ntext\n"));
        assert_eq!(join_body("note", None), "note\n");
        // Regenerating keeps the user's note.
        let again = join_body(split_body(&body).0, Some("New text"));
        assert!(again.starts_with("My note about it\n\n") && again.ends_with("New text\n"));
    }

    #[test]
    fn text_files_html_and_binary() {
        let ocr = Ocr::new(None);
        assert_eq!(
            extract_bytes(Format::Text, b"fn main() {}\r\n\r\n\r\n\r\nx", &ocr),
            Outcome::Text { tool: TEXT, text: "fn main() {}\n\nx".into() }
        );
        assert_eq!(extract_bytes(Format::Text, b"\0\x01\x02", &ocr), Outcome::Unsupported);
        let html = b"<html><head><title>t</title><style>p{}</style></head><body><h1>Hello world</h1><p>Some <b>bold</b> words appear here today.</p><script>x()</script></body></html>";
        match extract_bytes(Format::Html, html, &ocr) {
            Outcome::Text { tool, text } => {
                assert_eq!(tool, HTML);
                assert!(text.contains("# Hello world") && text.contains("**bold**"), "{text}");
                assert!(!text.contains("x()") && !text.contains("p{}"), "{text}");
            }
            other => panic!("{other:?}"),
        }
        // A few words on a page isn't text worth linking by.
        assert_eq!(extract_bytes(Format::Html, b"<p>hi</p>", &ocr), Outcome::Empty(HTML));
        assert_eq!(extract_bytes(Format::Image, b"x", &ocr), Outcome::Unavailable);
    }
}
