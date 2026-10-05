//! The note file format: YAML frontmatter with the item's metadata, then the body. See
//! "Storage" in docs/OVERVIEW.md.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::capture::detect::Kind;
use crate::error::{Error, Result};

/// Version of the note format, written as `plexus:` so notes from future versions can be told
/// apart (and files that aren't Plexus notes at all are left alone).
pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteMeta {
    pub plexus: u32,
    pub id: String,
    pub kind: Kind,
    pub title: String,
    /// RFC 3339 timestamp.
    pub captured: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `sha256:<hex>` of the body as captured; used to spot duplicates.
    pub hash: String,
    /// The original file, next to the note (images, documents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// What produced the body from the original ("pdf-text@1"), or "edited" once the user
    /// changed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted: Option<String>,
    /// Links the user accepted or made: ids of other items.
    #[serde(default)]
    pub links: Vec<String>,
    /// Keys added by the user or a newer version, kept as they are.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_norway::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub meta: NoteMeta,
    pub body: String,
}

impl Note {
    /// Parses a note file. Errors for files without Plexus frontmatter.
    pub fn parse(text: &str) -> Result<Note> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let rest = text
            .strip_prefix("---\n")
            .or_else(|| text.strip_prefix("---\r\n"))
            .ok_or_else(|| not_a_note("no frontmatter"))?;
        let (yaml, body) = split_frontmatter(rest).ok_or_else(|| not_a_note("unclosed frontmatter"))?;
        let meta: NoteMeta = serde_norway::from_str(yaml).map_err(|e| not_a_note(&e.to_string()))?;
        if meta.plexus == 0 || meta.id.is_empty() {
            return Err(not_a_note("missing plexus version or id"));
        }
        // One blank line separates the frontmatter from the body when Plexus writes a note.
        let body = body.strip_prefix('\n').or_else(|| body.strip_prefix("\r\n")).unwrap_or(body);
        Ok(Note { meta, body: body.to_string() })
    }

    pub fn to_file(&self) -> Result<String> {
        let yaml = serde_norway::to_string(&self.meta).map_err(|e| Error::Invalid(e.to_string()))?;
        Ok(format!("---\n{yaml}---\n\n{}", self.body))
    }
}

/// Splits at the closing `---` line: (frontmatter, everything after that line).
fn split_frontmatter(rest: &str) -> Option<(&str, &str)> {
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

fn not_a_note(reason: &str) -> Error {
    Error::Invalid(format!("not a Plexus note: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Note {
        Note {
            meta: NoteMeta {
                plexus: FORMAT_VERSION,
                id: "01JA7QX3Z9K3F9".into(),
                kind: Kind::Code,
                title: "fn main(): \"hello\" # not a comment".into(),
                captured: "2026-10-05T10:00:00Z".into(),
                source: None,
                language: Some("rust".into()),
                hash: "sha256:abc".into(),
                asset: None,
                asset_hash: None,
                original_name: None,
                mime: None,
                size: None,
                extracted: None,
                links: vec![],
                extra: BTreeMap::new(),
            },
            body: "```rust\nfn main() {}\n```\n\n---\n\nA horizontal rule above.\n".into(),
        }
    }

    #[test]
    fn round_trips() {
        let note = sample();
        let file = note.to_file().unwrap();
        assert!(file.starts_with("---\nplexus: 1\nid: 01JA7QX3Z9K3F9\nkind: code\n"));
        assert!(!file.contains("asset:"), "unset fields are left out");
        assert_eq!(Note::parse(&file).unwrap(), note);
    }

    #[test]
    fn keeps_unknown_keys() {
        let file = "---\nplexus: 1\nid: x1\nkind: text\ntitle: T\ncaptured: now\nhash: h\nmood: happy\ntags: [a, b]\n---\n\nbody\n";
        let note = Note::parse(file).unwrap();
        assert_eq!(note.meta.extra.len(), 2);
        assert_eq!(note.body, "body\n");
        let again = Note::parse(&note.to_file().unwrap()).unwrap();
        assert_eq!(again, note);
    }

    #[test]
    fn rejects_other_markdown() {
        assert!(Note::parse("# Just a markdown file\n").is_err());
        assert!(Note::parse("---\ntitle: Obsidian note\n---\nhi").is_err());
        assert!(Note::parse("---\nplexus: 1\nid: x\n").is_err());
    }
}
