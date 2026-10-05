//! The library folder: one markdown note per item, with originals (images, documents) next to
//! their notes. It is the source of truth; the index in `.plexus/` is rebuilt from it.

pub mod note;
pub mod reconcile;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Utc};
use sha2::{Digest, Sha256};

use crate::error::Result;
use note::Note;

const ITEMS_DIR: &str = "items";
const INDEX_DIR: &str = ".plexus";
const SLUG_MAX: usize = 60;

#[derive(Clone, Debug)]
pub struct Library {
    root: PathBuf,
}

impl Library {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join(ITEMS_DIR))?;
        fs::create_dir_all(root.join(INDEX_DIR))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn items_dir(&self) -> PathBuf {
        self.root.join(ITEMS_DIR)
    }

    pub fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_DIR).join("index.db")
    }

    /// Paths are stored relative to the library root, with `/` separators, so the library can be
    /// moved or synced between machines.
    pub fn relative(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
    }

    pub fn absolute(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// A free path for a new note: `items/YYYY/MM/<title-slug>-<last 4 of id>.md`.
    pub fn new_note_path(&self, title: &str, id: &str, captured: DateTime<Utc>) -> PathBuf {
        let dir = self
            .items_dir()
            .join(format!("{:04}", captured.year()))
            .join(format!("{:02}", captured.month()));
        let suffix = id[id.len().saturating_sub(4)..].to_ascii_lowercase();
        let stem = match slug(title) {
            s if s.is_empty() => suffix,
            s => format!("{s}-{suffix}"),
        };
        let mut path = dir.join(format!("{stem}.md"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{stem}-{n}.md"));
            n += 1;
        }
        path
    }

    pub fn write_note(&self, path: &Path, note: &Note) -> Result<()> {
        crate::settings::write_atomic(path, note.to_file()?.as_bytes())
    }

    pub fn read_note(&self, path: &Path) -> Result<Note> {
        Note::parse(&fs::read_to_string(path)?)
    }
}

/// "Rust's Ownership Rules!" → "rusts-ownership-rules"
pub fn slug(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if (c.is_whitespace() || "-_/.:".contains(c)) && !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.chars().count() >= SLUG_MAX {
            break;
        }
    }
    slug.trim_end_matches('-').to_string()
}

/// `sha256:<hex>`
pub fn hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Hash of text with line endings and surrounding whitespace normalised, so the same snippet
/// copied from different apps is recognised as a duplicate.
pub fn text_hash(text: &str) -> String {
    hash(text.replace("\r\n", "\n").trim().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Rust's Ownership Rules!"), "rusts-ownership-rules");
        assert_eq!(slug("  example.com/blog/post  "), "example-com-blog-post");
        assert_eq!(slug("Ünïcödé — names"), "ünïcödé-names");
        assert_eq!(slug("!!!"), "");
        assert!(slug(&"word ".repeat(30)).chars().count() <= SLUG_MAX);
    }

    #[test]
    fn note_paths_are_unique_and_dated() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let when = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z").unwrap().with_timezone(&Utc);
        let path = library.new_note_path("Hello World", "01JA7QX3Z9K3F9", when);
        assert_eq!(library.relative(&path), "items/2026/10/hello-world-k3f9.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "x").unwrap();
        let next = library.new_note_path("Hello World", "01JA7QX3Z9K3F9", when);
        assert_eq!(library.relative(&next), "items/2026/10/hello-world-k3f9-2.md");
        assert!(library.relative(&library.new_note_path("!!!", "ABCD", when)).ends_with("/abcd.md"));
    }

    #[test]
    fn text_hash_ignores_line_endings_and_edges() {
        assert_eq!(text_hash("a\r\nb\n"), text_hash("  a\nb"));
        assert_ne!(text_hash("a b"), text_hash("a  b"));
    }
}
