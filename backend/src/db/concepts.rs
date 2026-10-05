//! Concepts (a small shared vocabulary of ideas), which items they tag, and the record of what
//! was already sent to the AI provider for each item. All derived: it can be rebuilt by
//! enriching the library again.

use std::collections::HashMap;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::Result;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Concept {
    pub id: i64,
    pub name: String,
}

/// A concept with the items it tags, for the map.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConceptItems {
    pub id: i64,
    pub name: String,
    pub items: Vec<String>,
}

/// Every concept with its embedding, to find the ones relevant to an item.
pub fn all_embedded(conn: &Connection) -> Result<Vec<(Concept, Vec<f32>)>> {
    let mut stmt = conn.prepare("SELECT id, name, embedding FROM concepts")?;
    let rows = stmt.query_map([], |row| {
        let blob: Vec<u8> = row.get(2)?;
        Ok((Concept { id: row.get(0)?, name: row.get(1)? }, from_blob(&blob)))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The concept with this name or alias (any capitalisation).
pub fn find(conn: &Connection, name: &str) -> Result<Option<Concept>> {
    let by_name = conn
        .query_row("SELECT id, name FROM concepts WHERE name = ?1", [name], |row| {
            Ok(Concept { id: row.get(0)?, name: row.get(1)? })
        })
        .optional()?;
    if by_name.is_some() {
        return Ok(by_name);
    }
    Ok(conn
        .query_row(
            "SELECT c.id, c.name FROM concept_aliases a JOIN concepts c ON c.id = a.concept_id WHERE a.alias = ?1",
            [name],
            |row| Ok(Concept { id: row.get(0)?, name: row.get(1)? }),
        )
        .optional()?)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Concept>> {
    Ok(conn
        .query_row("SELECT id, name FROM concepts WHERE id = ?1", [id], |row| {
            Ok(Concept { id: row.get(0)?, name: row.get(1)? })
        })
        .optional()?)
}

pub fn create(conn: &Connection, name: &str, embedding: &[f32]) -> Result<Concept> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    conn.execute(
        "INSERT INTO concepts (name, embedding, created_at) VALUES (?1, ?2, ?3)",
        params![name, to_blob(embedding), now],
    )?;
    Ok(Concept { id: conn.last_insert_rowid(), name: name.to_string() })
}

/// Another way of writing the concept, so it is recognised next time without asking.
pub fn add_alias(conn: &Connection, concept_id: i64, alias: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO concept_aliases (alias, concept_id)
         SELECT ?1, ?2 WHERE NOT EXISTS (SELECT 1 FROM concepts WHERE name = ?1)",
        params![alias, concept_id],
    )?;
    Ok(())
}

/// Replaces the concepts of an item, then forgets concepts nothing uses any more.
pub fn set_for_item(conn: &Connection, item_id: &str, concepts: &[(i64, f32)]) -> Result<()> {
    conn.execute("DELETE FROM item_concepts WHERE item_id = ?1", [item_id])?;
    for (concept_id, weight) in concepts {
        conn.execute(
            "INSERT OR REPLACE INTO item_concepts (item_id, concept_id, weight) VALUES (?1, ?2, ?3)",
            params![item_id, concept_id, *weight as f64],
        )?;
    }
    gc(conn)
}

/// Deletes concepts that tag no item.
pub fn gc(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM concepts WHERE id NOT IN (SELECT concept_id FROM item_concepts)", [])?;
    Ok(())
}

pub fn of_item(conn: &Connection, item_id: &str) -> Result<Vec<Concept>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name FROM item_concepts ic JOIN concepts c ON c.id = ic.concept_id
         WHERE ic.item_id = ?1 ORDER BY ic.weight DESC, c.name",
    )?;
    let rows = stmt.query_map([item_id], |row| Ok(Concept { id: row.get(0)?, name: row.get(1)? }))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The concepts of each of these items.
pub fn of_items(conn: &Connection, ids: &[String]) -> Result<HashMap<String, Vec<Concept>>> {
    let mut found: HashMap<String, Vec<Concept>> = HashMap::new();
    for id in ids {
        let concepts = of_item(conn, id)?;
        if !concepts.is_empty() {
            found.insert(id.clone(), concepts);
        }
    }
    Ok(found)
}

/// Every concept with its items, most widely used first.
pub fn with_items(conn: &Connection) -> Result<Vec<ConceptItems>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, ic.item_id FROM concepts c JOIN item_concepts ic ON ic.concept_id = c.id
         ORDER BY c.id",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?;
    let mut found: Vec<ConceptItems> = Vec::new();
    for row in rows {
        let (id, name, item) = row?;
        match found.last_mut() {
            Some(last) if last.id == id => last.items.push(item),
            _ => found.push(ConceptItems { id, name, items: vec![item] }),
        }
    }
    found.sort_by(|a, b| b.items.len().cmp(&a.items.len()).then_with(|| a.name.cmp(&b.name)));
    Ok(found)
}

// --- the record of what was sent ---------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Enriched {
    pub content_hash: String,
    pub version: i64,
    /// The title the AI gave, when it did.
    pub title: Option<String>,
}

pub fn enriched(conn: &Connection, item_id: &str) -> Result<Option<Enriched>> {
    Ok(conn
        .query_row("SELECT content_hash, version, title FROM enrichment WHERE item_id = ?1", [item_id], |row| {
            Ok(Enriched { content_hash: row.get(0)?, version: row.get(1)?, title: row.get(2)? })
        })
        .optional()?)
}

pub fn record_enriched(conn: &Connection, item_id: &str, record: &Enriched) -> Result<()> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    conn.execute(
        "INSERT INTO enrichment (item_id, content_hash, version, title, enriched_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(item_id) DO UPDATE SET content_hash = excluded.content_hash, version = excluded.version,
           title = excluded.title, enriched_at = excluded.enriched_at",
        params![item_id, record.content_hash, record.version, record.title, now],
    )?;
    Ok(())
}

pub fn set_summary(conn: &Connection, item_id: &str, summary: Option<&str>) -> Result<()> {
    conn.execute("UPDATE items SET summary = ?2 WHERE id = ?1", params![item_id, summary])?;
    Ok(())
}

/// Items that were captured since `since` and haven't been enriched at `version`, oldest first.
pub fn unenriched_since(conn: &Connection, since: &str, version: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT i.id FROM items i LEFT JOIN enrichment e ON e.item_id = i.id
         WHERE i.captured_at >= ?1 AND i.status = 'linked' AND (e.version IS NULL OR e.version < ?2)
         ORDER BY i.captured_at",
    )?;
    let rows = stmt.query_map(params![since, version], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Forgets everything the AI wrote (summaries, concepts, what was sent), so it can be redone.
#[cfg(test)]
pub fn reset(conn: &Connection) -> Result<()> {
    conn.execute_batch("DELETE FROM item_concepts; DELETE FROM concepts; DELETE FROM enrichment; UPDATE items SET summary = NULL;")?;
    Ok(())
}

pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(blob: &[u8]) -> Vec<f32> {
    blob.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::chunks::unit;
    use crate::db::items::{sample, upsert};
    use crate::db::open_temp;

    #[test]
    fn concepts_aliases_and_cleanup() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            upsert(conn, &sample("a", "items/a.md", "1"), "")?;
            upsert(conn, &sample("b", "items/b.md", "2"), "")?;
            let borrow = create(conn, "borrow checker", &unit(&[1.0, 0.0]))?;
            let own = create(conn, "ownership", &unit(&[0.0, 1.0]))?;
            add_alias(conn, borrow.id, "borrowck")?;
            assert_eq!(find(conn, "BORROW CHECKER")?, Some(borrow.clone()));
            assert_eq!(find(conn, "Borrowck")?, Some(borrow.clone()));
            assert_eq!(find(conn, "lifetimes")?, None);

            set_for_item(conn, "a", &[(borrow.id, 1.0), (own.id, 0.5)])?;
            set_for_item(conn, "b", &[(borrow.id, 1.0)])?;
            assert_eq!(of_item(conn, "a")?, vec![borrow.clone(), own.clone()]);
            let all = with_items(conn)?;
            assert_eq!((all[0].name.as_str(), all[0].items.len()), ("borrow checker", 2));

            // Re-tagging drops concepts nothing uses; deleting an item does too.
            set_for_item(conn, "a", &[(borrow.id, 1.0)])?;
            assert_eq!(find(conn, "ownership")?, None);
            crate::db::items::delete(conn, "a")?;
            crate::db::items::delete(conn, "b")?;
            assert!(all_embedded(conn)?.is_empty());
            let aliases: i64 = conn.query_row("SELECT COUNT(*) FROM concept_aliases", [], |r| r.get(0))?;
            assert_eq!(aliases, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn remembers_what_was_sent() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            upsert(conn, &sample("a", "items/a.md", "1"), "")?;
            assert_eq!(enriched(conn, "a")?, None);
            let record = Enriched { content_hash: "h".into(), version: 1, title: Some("T".into()) };
            record_enriched(conn, "a", &record)?;
            assert_eq!(enriched(conn, "a")?, Some(record));
            set_summary(conn, "a", Some("About a"))?;
            assert_eq!(crate::db::items::get(conn, "a")?.unwrap().summary.as_deref(), Some("About a"));
            // Not linked yet, so not due.
            assert!(unenriched_since(conn, "2000-01-01", 1)?.is_empty());
            crate::db::items::set_status(conn, "a", crate::db::items::Status::Linked, 4)?;
            assert!(unenriched_since(conn, "2000-01-01", 1)?.is_empty(), "already enriched at version 1");
            assert_eq!(unenriched_since(conn, "2000-01-01", 2)?, vec!["a".to_string()]);
            assert!(unenriched_since(conn, "2030-01-01", 2)?.is_empty());
            reset(conn)?;
            assert_eq!(enriched(conn, "a")?, None);
            Ok(())
        })
        .unwrap();
    }
}
