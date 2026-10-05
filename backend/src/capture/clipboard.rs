//! Reading the system clipboard directly, for what the webview's paste event doesn't hand over:
//! on Linux, images (e.g. screenshots) never reach the page.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// The clipboard text, or `None` when it holds no (non-blank) text.
pub fn text() -> Result<Option<String>> {
    match arboard::Clipboard::new()?.get_text() {
        Ok(text) if !text.trim().is_empty() => Ok(Some(text)),
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The clipboard image as PNG bytes, or `None` when the clipboard holds no image.
pub fn image_png() -> Result<Option<Vec<u8>>> {
    let image = match arboard::Clipboard::new()?.get_image() {
        Ok(image) => image,
        Err(arboard::Error::ContentNotAvailable) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, image.width as u32, image.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.bytes)?;
    Ok(Some(png))
}

/// When pasted text is nothing but paths to existing files (what GNOME Files puts on the
/// clipboard when you copy files), returns those files. Otherwise an empty list.
pub fn existing_files(text: &str) -> Vec<PathBuf> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let paths: Vec<PathBuf> = lines.iter().filter_map(|line| as_path(line)).collect();
    if paths.is_empty() || paths.len() != lines.len() || !paths.iter().all(|p| p.is_file()) {
        return Vec::new();
    }
    paths
}

fn as_path(line: &str) -> Option<PathBuf> {
    if let Some(rest) = line.strip_prefix("file://") {
        return Some(PathBuf::from(percent_decode(rest)));
    }
    let path = Path::new(line);
    path.is_absolute().then(|| path.to_path_buf())
}

/// Decodes `%20`-style escapes, as used in file URIs and in header values.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_copied_files() {
        let dir = std::env::temp_dir().join(format!("plexus-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("My Report.pdf");
        std::fs::write(&file, b"x").unwrap();

        let path = file.to_string_lossy().into_owned();
        assert_eq!(existing_files(&path), vec![file.clone()]);
        let uri = format!("file://{}", path.replace(' ', "%20"));
        assert_eq!(existing_files(&uri), vec![file.clone()]);
        // Prose that happens to mention a path, missing files and directories stay text.
        assert!(existing_files(&format!("see {path}")).is_empty());
        assert!(existing_files("/no/such/file.txt").is_empty());
        assert!(existing_files(&dir.to_string_lossy()).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn percent_decodes() {
        assert_eq!(percent_decode("Q3%20Report%20%E2%9C%93.pdf"), "Q3 Report ✓.pdf");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
