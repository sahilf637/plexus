//! Links made by hand. They live in the `links:` list of the note they were made from (the
//! source of truth) and are mirrored in the index's `user_links` table.

use std::fs;

use super::Library;
use super::reconcile::modified_nanos;
use crate::db::items;
use crate::db::{Db, graph};
use crate::error::{Error, Result};

/// Links `from` to `to`. Returns the links touching `from` afterwards.
pub fn add(library: &Library, db: &Db, from: &str, to: &str) -> Result<Vec<graph::Edge>> {
    if from == to {
        return Err(Error::Invalid("an item can't link to itself".into()));
    }
    let exists = db.with(|conn| items::get(conn, to))?.is_some();
    if !exists {
        return Err(Error::Invalid(format!("no item {to}")));
    }
    edit(library, db, from, |links| {
        let missing = !links.iter().any(|l| l == to);
        if missing {
            links.push(to.to_string());
        }
        missing
    })?;
    db.with(|conn| graph::edges_of(conn, from))
}

/// Removes a hand-made link between the two, whichever of them holds it. Returns the links
/// touching `a` afterwards.
pub fn remove(library: &Library, db: &Db, a: &str, b: &str) -> Result<Vec<graph::Edge>> {
    for (holder, other) in [(a, b), (b, a)] {
        // The other item may be gone; nothing to remove then.
        if db.with(|conn| items::get(conn, holder))?.is_none() {
            continue;
        }
        edit(library, db, holder, |links| {
            let before = links.len();
            links.retain(|l| l != other);
            links.len() != before
        })?;
    }
    db.with(|conn| graph::edges_of(conn, a))
}

/// Applies `change` to the note's `links`; it says whether it changed anything. The note is
/// only rewritten if so.
fn edit(library: &Library, db: &Db, id: &str, change: impl FnOnce(&mut Vec<String>) -> bool) -> Result<()> {
    let item = db.with(|conn| items::get(conn, id))?.ok_or_else(|| Error::Invalid(format!("no item {id}")))?;
    let path = library.absolute(&item.path);
    let mut note = library.read_note(&path)?;
    if !change(&mut note.meta.links) {
        return Ok(());
    }
    library.write_note(&path, &note)?;
    let stat = fs::metadata(&path)?;
    db.with(|conn| {
        items::set_file_state(conn, id, modified_nanos(&stat), stat.len() as i64)?;
        graph::set_user_links(conn, id, &note.meta.links)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::save;
    use crate::library::reconcile::reconcile;
    use crate::library::trash;

    fn setup() -> (tempfile::TempDir, Library, Db) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        (dir, library, db)
    }

    fn item(library: &Library, db: &Db, text: &str) -> items::Item {
        save::text(library, db, text, false).unwrap().item.unwrap()
    }

    #[test]
    fn a_hand_made_link_is_a_strong_edge_kept_in_the_note() {
        let (_dir, library, db) = setup();
        let (a, b) = (item(&library, &db, "Sourdough starter"), item(&library, &db, "Rust ownership"));
        let edges = add(&library, &db, &a.id, &b.id).unwrap();
        assert_eq!((edges.len(), edges[0].kind.as_str(), edges[0].tier), (1, "user", crate::pipeline::link::Tier::Strong));
        assert_eq!(edges[0].reason.as_deref(), Some("linked by you"));
        // Seen from the other side too, and adding it twice changes nothing.
        assert_eq!(db.with(|conn| graph::edges_of(conn, &b.id)).unwrap().len(), 1);
        add(&library, &db, &a.id, &b.id).unwrap();
        assert_eq!(library.read_note(&library.absolute(&a.path)).unwrap().meta.links, vec![b.id.clone()]);
        // Rewriting the note is not an edit that reconcile re-indexes.
        assert_eq!(reconcile(&library, &db).unwrap().updated, 0);
        assert!(add(&library, &db, &a.id, &a.id).is_err() && add(&library, &db, &a.id, "nope").is_err());
    }

    #[test]
    fn links_survive_a_rebuilt_index_and_a_restore() {
        let (_dir, library, db) = setup();
        let (a, b) = (item(&library, &db, "Sourdough starter"), item(&library, &db, "Rust ownership"));
        add(&library, &db, &a.id, &b.id).unwrap();

        // Deleting the index and rebuilding it brings the link back from the notes.
        drop(db);
        fs::remove_file(library.index_path()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        reconcile(&library, &db).unwrap();
        assert_eq!(db.with(|conn| graph::edges(conn)).unwrap().len(), 1);

        // Deleting the target hides the link; restoring it brings the link back.
        let deleted = trash::delete_items(&library, &db, std::slice::from_ref(&b.id)).unwrap();
        assert!(db.with(|conn| graph::edges(conn)).unwrap().is_empty());
        trash::restore_items(&library, &db, &deleted.batch).unwrap();
        assert_eq!(db.with(|conn| graph::edges(conn)).unwrap().len(), 1);

        // Deleting the holder and restoring it too.
        let deleted = trash::delete_items(&library, &db, std::slice::from_ref(&a.id)).unwrap();
        assert!(db.with(|conn| graph::edges(conn)).unwrap().is_empty());
        trash::restore_items(&library, &db, &deleted.batch).unwrap();
        assert_eq!(db.with(|conn| graph::edges(conn)).unwrap().len(), 1);
    }

    #[test]
    fn removing_works_from_either_side() {
        let (_dir, library, db) = setup();
        let (a, b) = (item(&library, &db, "Sourdough starter"), item(&library, &db, "Rust ownership"));
        add(&library, &db, &a.id, &b.id).unwrap();
        assert!(remove(&library, &db, &b.id, &a.id).unwrap().is_empty());
        assert!(library.read_note(&library.absolute(&a.path)).unwrap().meta.links.is_empty());
        assert!(db.with(|conn| graph::edges(conn)).unwrap().is_empty());
    }
}
