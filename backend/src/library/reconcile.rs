//! Bringing the index in line with the library folder: notes added, edited, moved or deleted
//! outside Plexus (or while it wasn't running) are picked up, and a deleted index is rebuilt.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use super::{Library, text_hash};
use crate::db::{Db, graph};
use crate::db::items::{self, Item, Status};
use crate::error::Result;
use crate::library::note::Note;

#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    /// Markdown files that aren't Plexus notes, or couldn't be read.
    pub skipped: usize,
}

pub fn reconcile(library: &Library, db: &Db) -> Result<Report> {
    let mut report = Report::default();
    let known = db.with(|conn| items::file_states(conn))?;
    let mut seen_ids = HashSet::new();

    for path in note_files(&library.items_dir()) {
        let relative = library.relative(&path);
        let Ok(stat) = fs::metadata(&path) else { continue };
        let (mtime, size) = (modified_nanos(&stat), stat.len() as i64);

        if let Some(state) = known.get(&relative)
            && state.mtime == mtime
            && state.size == size
        {
            seen_ids.insert(state.id.clone());
            report.unchanged += 1;
            continue;
        }

        let note = match library.read_note(&path) {
            Ok(note) => note,
            Err(e) => {
                tracing::debug!(path = %relative, "skipped: {e}");
                report.skipped += 1;
                continue;
            }
        };
        if !seen_ids.insert(note.meta.id.clone()) {
            tracing::warn!(path = %relative, id = %note.meta.id, "skipped: another note has the same id");
            report.skipped += 1;
            continue;
        }

        let is_new = !known.values().any(|state| state.id == note.meta.id);
        let item = item_from_note(library, &path, &note, mtime, size);
        db.with(|conn| {
            items::upsert(conn, &item, &note.body)?;
            graph::set_user_links(conn, &item.id, &note.meta.links)
        })?;
        if is_new { report.added += 1 } else { report.updated += 1 }
    }

    for state in known.values().filter(|state| !seen_ids.contains(&state.id)) {
        db.with(|conn| items::delete(conn, &state.id))?;
        report.removed += 1;
    }
    Ok(report)
}

/// The index row for a note. It starts out `pending` so the pipeline (re)links it.
pub fn item_from_note(library: &Library, path: &Path, note: &Note, mtime: i64, size: i64) -> Item {
    let meta = &note.meta;
    // Originals live next to their note.
    let asset_path = meta.asset.as_ref().map(|name| library.relative(&path.with_file_name(name)));
    Item {
        id: meta.id.clone(),
        path: library.relative(path),
        kind: meta.kind,
        title: meta.title.clone(),
        source_url: meta.source.clone(),
        language: meta.language.clone(),
        captured_at: meta.captured.clone(),
        // Recomputed rather than read from the frontmatter, so edited notes dedupe correctly.
        body_hash: text_hash(&note.body),
        asset_path,
        asset_hash: meta.asset_hash.clone(),
        mime: meta.mime.clone(),
        size: meta.size.map(|s| s as i64),
        file_mtime: mtime,
        file_size: size,
        status: Status::Pending,
        extracted: meta.extracted.clone(),
        summary: None,
    }
}

pub fn modified_nanos(stat: &fs::Metadata) -> i64 {
    stat.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i64)
}

/// Every `.md` file under `dir`, skipping hidden folders.
fn note_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let hidden = entry.file_name().to_string_lossy().starts_with('.');
            match entry.file_type() {
                Ok(t) if t.is_dir() && !hidden => stack.push(path),
                Ok(t) if t.is_file() && path.extension().is_some_and(|e| e == "md") => files.push(path),
                _ => {}
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::capture::detect::Kind;
    use crate::library::note::{FORMAT_VERSION, NoteMeta};

    fn note(id: &str, body: &str) -> Note {
        Note {
            meta: NoteMeta {
                plexus: FORMAT_VERSION,
                id: id.into(),
                kind: Kind::Text,
                title: format!("Note {id}"),
                captured: "2026-10-05T10:00:00Z".into(),
                source: None,
                language: None,
                hash: text_hash(body),
                asset: None,
                asset_hash: None,
                original_name: None,
                mime: None,
                size: None,
                extracted: None,
                title_by: None,
                title_was: None,
                attachments: Vec::new(),
                links: vec![],
                extra: BTreeMap::new(),
            },
            body: body.into(),
        }
    }

    fn setup() -> (tempfile::TempDir, Library, Db) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        (dir, library, db)
    }

    fn write(library: &Library, relative: &str, note: &Note) -> PathBuf {
        let path = library.root().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        library.write_note(&path, note).unwrap();
        path
    }

    #[test]
    fn follows_the_folder() {
        let (_dir, library, db) = setup();
        let a = write(&library, "items/2026/10/a.md", &note("A", "first"));
        write(&library, "items/2026/10/b.md", &note("B", "second"));
        fs::write(library.root().join("items/readme.md"), "# not a note").unwrap();
        fs::create_dir_all(library.root().join("items/.trash")).unwrap();
        write(&library, "items/.trash/old.md", &note("OLD", "ignored"));

        let first = reconcile(&library, &db).unwrap();
        assert_eq!(first, Report { added: 2, skipped: 1, ..Default::default() });

        // Nothing changed.
        assert_eq!(reconcile(&library, &db).unwrap(), Report { unchanged: 2, skipped: 1, ..Default::default() });

        // Edited, moved and deleted outside Plexus.
        library.write_note(&a, &note("A", "first, edited")).unwrap();
        fs::rename(library.root().join("items/2026/10/b.md"), library.root().join("items/b-moved.md")).unwrap();
        let after = reconcile(&library, &db).unwrap();
        assert_eq!(after, Report { updated: 2, skipped: 1, ..Default::default() });
        db.with(|conn| {
            assert_eq!(items::get(conn, "B")?.unwrap().path, "items/b-moved.md");
            assert_eq!(items::get(conn, "A")?.unwrap().body_hash, text_hash("first, edited"));
            Ok(())
        })
        .unwrap();

        fs::remove_file(&a).unwrap();
        assert_eq!(reconcile(&library, &db).unwrap(), Report { removed: 1, unchanged: 1, skipped: 1, ..Default::default() });
    }

    #[test]
    fn rebuilds_a_deleted_index() {
        let (_dir, library, db) = setup();
        write(&library, "items/a.md", &note("A", "first"));
        reconcile(&library, &db).unwrap();
        drop(db);
        fs::remove_file(library.index_path()).unwrap();

        let db = Db::open(&library.index_path()).unwrap();
        assert_eq!(reconcile(&library, &db).unwrap().added, 1);
        assert_eq!(db.with(|conn| items::count(conn)).unwrap(), 1);
    }

    #[test]
    fn duplicate_ids_are_skipped() {
        let (_dir, library, db) = setup();
        write(&library, "items/a.md", &note("A", "first"));
        write(&library, "items/a-copy.md", &note("A", "first"));
        let report = reconcile(&library, &db).unwrap();
        assert_eq!((report.added, report.skipped), (1, 1));
    }
}
