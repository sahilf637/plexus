//! Documents: PDF, Word (.docx) and EPUB. Each returns the text, or `None` when the file can't be
//! read. They run on files the user dropped in, so they must cope with anything.

use std::io::{Cursor, Read};

use roxmltree::{Document, Node};
use zip::ZipArchive;

/// Largest single file read out of a zip (a bomb guard).
const MAX_ENTRY: u64 = 50 * 1024 * 1024;

/// Text of each page, behind a `<!-- page N -->` marker (for citations later).
pub fn pdf(bytes: &[u8]) -> Option<String> {
    // The PDF reader panics on some malformed files; that mustn't take the app down.
    let pages = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes)).ok()?.ok()?;
    let text: Vec<String> = pages
        .iter()
        .enumerate()
        .filter(|(_, page)| !page.trim().is_empty())
        .map(|(i, page)| format!("<!-- page {} -->\n{}", i + 1, page.trim()))
        .collect();
    Some(text.join("\n\n"))
}

/// The paragraphs of the document body.
pub fn docx(bytes: &[u8]) -> Option<String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).ok()?;
    let xml = read_entry(&mut zip, "word/document.xml")?;
    let doc = Document::parse(&xml).ok()?;
    let paragraphs: Vec<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("p"))
        .map(paragraph_text)
        .filter(|p| !p.trim().is_empty())
        .collect();
    Some(paragraphs.join("\n\n"))
}

fn paragraph_text(paragraph: Node) -> String {
    let mut text = String::new();
    for node in paragraph.descendants() {
        match node.tag_name().name() {
            "t" => text.push_str(node.text().unwrap_or("")),
            "tab" => text.push('\t'),
            "br" | "cr" => text.push('\n'),
            _ => {}
        }
    }
    text
}

/// The book's chapters in reading order, as markdown.
pub fn epub(bytes: &[u8]) -> Option<String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).ok()?;
    let container = read_entry(&mut zip, "META-INF/container.xml")?;
    let container = Document::parse(&container).ok()?;
    let opf_path = container.descendants().find(|n| n.has_tag_name("rootfile"))?.attribute("full-path")?.to_string();
    let opf = read_entry(&mut zip, &opf_path)?;
    let opf = Document::parse(&opf).ok()?;
    let base = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir);

    let manifest: Vec<(&str, &str)> = opf
        .descendants()
        .filter(|n| n.has_tag_name("item"))
        .filter_map(|n| Some((n.attribute("id")?, n.attribute("href")?)))
        .collect();
    let mut chapters = Vec::new();
    for itemref in opf.descendants().filter(|n| n.has_tag_name("itemref")) {
        let Some(id) = itemref.attribute("idref") else { continue };
        let Some((_, href)) = manifest.iter().find(|(item, _)| *item == id) else { continue };
        let href = href.split('#').next().unwrap_or(href);
        let path = if base.is_empty() { href.to_string() } else { format!("{base}/{}", percent_decode(href)) };
        let Some(html) = read_entry(&mut zip, &path) else { continue };
        if let Some(markdown) = super::html_to_markdown(&html).filter(|m| !m.trim().is_empty()) {
            chapters.push(markdown.trim().to_string());
        }
    }
    Some(chapters.join("\n\n"))
}

fn read_entry(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let file = zip.by_name(name).ok()?;
    let mut text = String::new();
    file.take(MAX_ENTRY).read_to_string(&mut text).ok()?;
    Some(text)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            for (name, content) in files {
                zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[test]
    fn reads_word_paragraphs() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
            <w:p><w:r><w:t>Quarterly</w:t></w:r><w:r><w:t xml:space="preserve"> report</w:t></w:r></w:p>
            <w:p/><w:p><w:r><w:t>Revenue grew.</w:t></w:r></w:p></w:body></w:document>"#;
        let text = docx(&zip_of(&[("word/document.xml", xml)])).unwrap();
        assert_eq!(text, "Quarterly report\n\nRevenue grew.");
        assert!(docx(b"not a zip").is_none());
    }

    #[test]
    fn reads_epub_chapters_in_spine_order() {
        let container = r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;
        let opf = r#"<package><manifest><item id="b" href="two.xhtml"/><item id="a" href="one%201.xhtml"/></manifest>
            <spine><itemref idref="a"/><itemref idref="b"/></spine></package>"#;
        let book = zip_of(&[
            ("META-INF/container.xml", container),
            ("OEBPS/content.opf", opf),
            ("OEBPS/one 1.xhtml", "<html><body><h1>First</h1></body></html>"),
            ("OEBPS/two.xhtml", "<html><body><p>Second</p></body></html>"),
        ]);
        assert_eq!(epub(&book).unwrap(), "# First\n\nSecond");
    }

    #[test]
    fn garbage_pdf_is_none_not_a_crash() {
        assert!(pdf(b"%PDF-1.4 definitely broken").is_none());
    }
}
