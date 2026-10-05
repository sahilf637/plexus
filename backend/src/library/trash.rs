//! Deleting items, and undoing it. A deleted item's files (note, original, attachments) move to
//! `.plexus/trash/<batch>/` under the same relative paths, so putting them back is a move. The
//! trash is outside `items/`, so reconcile never sees it; batches are purged after [`KEEP`].

use std::fs;
use std::path::{Component, Path};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use super::Library;
use super::reconcile::{item_from_note, modified_nanos};
use crate::db::Db;
use crate::db::items::{self, Item};
use crate::error::{Error, Result};

/// How long deleted items can still be restored.
pub const KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Items deleted together, restorable as one batch.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Deleted {
    pub batch: String,
    pub ids: Vec<String>,
}

/// Moves the items' files to the trash and removes them from the index. Unknown ids are skipped.
pub fn delete_items(library: &Library, db: &Db, ids: &[String]) -> Result<Deleted> {
    let batch = ulid::Ulid::new().to_string();
    let mut deleted = Vec::new();
    for id in ids {
        let Some(item) = db.with(|conn| items::get(conn, id))? else { continue };
        move_in(library, &batch, &item_files(library, &item))?;
        db.with(|conn| items::delete(conn, id))?;
        deleted.push(id.clone());
    }
    Ok(Deleted { batch, ids: deleted })
}

/// Puts a deleted batch back and indexes its notes again (as `pending`, to be re-linked).
pub fn restore_items(library: &Library, db: &Db, batch: &str) -> Result<Vec<Item>> {
    let mut restored = Vec::new();
    for relative in restore_files(library, batch)? {
        let path = library.absolute(&relative);
        let Ok(note) = library.read_note(&path) else { continue };
        let stat = fs::metadata(&path)?;
        let item = item_from_note(library, &path, &note, modified_nanos(&stat), stat.len() as i64);
        db.with(|conn| {
            items::upsert(conn, &item, &note.body)?;
            crate::db::graph::set_user_links(conn, &item.id, &note.meta.links)
        })?;
        restored.push(item);
    }
    Ok(restored)
}

/// Removes batches older than `age`. Returns how many were removed.
pub fn purge(library: &Library, age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(library.trash_dir()) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|elapsed| elapsed > age);
        if old && fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Everything that belongs to an item, relative to the library: attachments and the original
/// first, the note last, so a failure part-way leaves a note that still describes what remains.
fn item_files(library: &Library, item: &Item) -> Vec<String> {
    let note_path = library.absolute(&item.path);
    let mut files = Vec::new();
    if let Ok(note) = library.read_note(&note_path) {
        files.extend(note.meta.attachments.iter().map(|a| library.relative(&note_path.with_file_name(&a.file))));
    }
    files.extend(item.asset_path.clone());
    files.push(item.path.clone());
    files
}

fn move_in(library: &Library, batch: &str, relative: &[String]) -> Result<()> {
    for path in relative {
        check_relative(path)?;
        let from = library.absolute(path);
        if !from.exists() {
            continue;
        }
        let to = library.trash_dir().join(batch).join(path);
        if let Some(dir) = to.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::rename(&from, &to)?;
    }
    Ok(())
}

/// Moves a batch's files back; returns the note files among them (relative paths). Nothing is
/// moved if a file's old place has been taken.
fn restore_files(library: &Library, batch: &str) -> Result<Vec<String>> {
    if batch.is_empty() || !batch.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Error::Invalid("not a deleted batch".into()));
    }
    let dir = library.trash_dir().join(batch);
    if !dir.is_dir() {
        return Err(Error::Invalid("this was deleted too long ago to restore".into()));
    }
    let mut files = Vec::new();
    collect(&dir, &mut files);
    let relative: Vec<String> = files.iter().map(|f| library.relative(f.strip_prefix(&dir).unwrap_or(f))).collect();
    for path in &relative {
        check_relative(path)?;
        if library.absolute(path).exists() {
            return Err(Error::Invalid(format!("can't restore: {path} already exists")));
        }
    }
    for (file, path) in files.iter().zip(&relative) {
        let to = library.absolute(path);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(file, to)?;
    }
    fs::remove_dir_all(&dir)?;
    Ok(relative.into_iter().filter(|p| p.ends_with(".md")).collect())
}

fn collect(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => collect(&path, files),
            Ok(t) if t.is_file() => files.push(path),
            _ => {}
        }
    }
}

/// Paths come from notes the user can edit: only plain relative ones are moved.
fn check_relative(path: &str) -> Result<()> {
    if Path::new(path).components().all(|c| matches!(c, Component::Normal(_))) {
        Ok(())
    } else {
        Err(Error::Invalid(format!("unsafe path in a note: {path}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::save;
    use crate::library::reconcile::reconcile;

    fn setup() -> (tempfile::TempDir, Library, Db) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        (dir, library, db)
    }

    #[test]
    fn delete_then_restore_round_trips_note_and_originals() {
        let (_dir, library, db) = setup();
        let originals = [
            save::Original { name: "a.png", mime: "image/png", bytes: b"one" },
            save::Original { name: "b.pdf", mime: "application/pdf", bytes: b"two" },
        ];
        let item = save::files(&library, &db, &originals, "Two files about boats", false).unwrap().item.unwrap();
        let note_path = library.absolute(&item.path);
        let asset_path = library.absolute(item.asset_path.as_deref().unwrap());
        let extra = fs::read_dir(note_path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.to_string_lossy().contains("_2."))
            .unwrap();

        let deleted = delete_items(&library, &db, &[item.id.clone(), "unknown".into()]).unwrap();
        assert_eq!(deleted.ids, vec![item.id.clone()]);
        assert!(!note_path.exists() && !asset_path.exists() && !extra.exists());
        assert_eq!(db.with(|conn| items::count(conn)).unwrap(), 0);
        // The trash is not part of the library: nothing comes back by itself.
        assert_eq!(reconcile(&library, &db).unwrap().added, 0);

        let restored = restore_items(&library, &db, &deleted.batch).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, item.id);
        assert!(note_path.exists() && asset_path.exists() && extra.exists());
        assert_eq!(db.with(|conn| items::count(conn)).unwrap(), 1);
        assert!(!library.trash_dir().join(&deleted.batch).exists());
    }

    #[test]
    fn deleting_removes_links_to_the_item() {
        let (_dir, library, db) = setup();
        let a = save::text(&library, &db, "Rust ownership", false).unwrap().item.unwrap();
        let b = save::text(&library, &db, "Rust borrowing", false).unwrap().item.unwrap();
        db.with(|conn| {
            conn.execute(
                "INSERT INTO edges (src, dst, kind, weight, provenance, created_at) VALUES (?1, ?2, 'similar', 0.8, 'embedding', '')",
                [&a.id, &b.id],
            )?;
            conn.execute("INSERT INTO layout (item_id, x, y) VALUES (?1, 1, 1)", [&a.id])?;
            Ok(())
        })
        .unwrap();
        delete_items(&library, &db, &[a.id]).unwrap();
        db.with(|conn| {
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get::<_, i64>(0))?, 0);
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM layout", [], |r| r.get::<_, i64>(0))?, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn restore_refuses_when_the_place_is_taken_and_unknown_batches() {
        let (_dir, library, db) = setup();
        let item = save::text(&library, &db, "Some note", false).unwrap().item.unwrap();
        let deleted = delete_items(&library, &db, std::slice::from_ref(&item.id)).unwrap();
        let path = library.absolute(&item.path);
        fs::write(&path, "taken").unwrap();
        assert!(restore_items(&library, &db, &deleted.batch).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "taken");
        assert!(restore_items(&library, &db, "../etc").is_err());
        assert!(restore_items(&library, &db, "NOSUCHBATCH").is_err());
    }

    #[test]
    fn purge_removes_only_old_batches() {
        let (_dir, library, db) = setup();
        let item = save::text(&library, &db, "Some note", false).unwrap().item.unwrap();
        let deleted = delete_items(&library, &db, &[item.id]).unwrap();
        assert_eq!(purge(&library, KEEP), 0);
        assert!(library.trash_dir().join(&deleted.batch).exists());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(purge(&library, Duration::from_millis(1)), 1);
        assert!(!library.trash_dir().join(&deleted.batch).exists());
    }

    #[test]
    fn paths_that_leave_the_library_are_refused() {
        assert!(check_relative("items/2026/10/a.md").is_ok());
        assert!(check_relative("../outside.md").is_err());
        assert!(check_relative("/etc/passwd").is_err());
    }
}
