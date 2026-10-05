//! Finding which items an item is related to, by meaning, and storing those links.
//!
//! Like Atomic's `compute_semantic_edges_for_atom`: every chunk of the item looks up its nearest
//! chunks elsewhere, each other item counts with its best-matching chunk, and the closest few
//! above a threshold become `similar` links.

use std::collections::HashMap;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, params};

use crate::db::chunks;
use crate::error::Result;

/// Measured on short snippets (`cargo test similarity_report -- --ignored --nocapture`): pairs on
/// the same topic score 0.44–0.78 (median 0.61), different topics 0.37–0.59 (median 0.46).
/// Longer notes score higher. Tuned again against a larger sample in M0 step 8.
pub const THRESHOLD: f32 = 0.58;
/// Links an item starts with. Existing links to it from other items are kept while they still
/// clear the threshold, so this isn't a hard cap.
pub const MAX_LINKS: usize = 8;
/// Bumped when how links are computed changes, so old links can be told apart.
pub const PIPELINE_VERSION: i64 = 1;
const NEIGHBOURS_PER_CHUNK: usize = 40;

/// Recomputes the item's `similar` links from its chunk vectors. Returns (other item, similarity).
pub fn link(conn: &mut Connection, item_id: &str, vectors: &[Vec<f32>]) -> Result<Vec<(String, f32)>> {
    let mut neighbours = Vec::new();
    for vector in vectors {
        neighbours.extend(chunks::nearest(conn, vector, NEIGHBOURS_PER_CHUNK)?);
    }
    let mut candidates: Vec<(String, f32)> = chunks::best_per_item(neighbours, item_id)
        .into_iter()
        .filter(|(_, similarity)| *similarity >= THRESHOLD)
        .collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let existing = existing_links(conn, item_id)?;
    let keep: Vec<(String, f32)> = candidates
        .into_iter()
        .filter(|(other, _)| existing.get(other).map(String::as_str) != Some("rejected"))
        .enumerate()
        .filter(|(rank, (other, _))| *rank < MAX_LINKS || existing.contains_key(other))
        .map(|(_, candidate)| candidate)
        .collect();

    let tx = conn.transaction()?;
    // Rejected links stay, so they're remembered and never come back.
    tx.execute(
        "DELETE FROM edges WHERE kind = 'similar' AND provenance = 'embedding' AND status != 'rejected'
           AND (src = ?1 OR dst = ?1)",
        [item_id],
    )?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    for (other, similarity) in &keep {
        let (src, dst) = if item_id < other.as_str() { (item_id, other.as_str()) } else { (other.as_str(), item_id) };
        tx.execute(
            "INSERT INTO edges (src, dst, kind, weight, provenance, status, pipeline_version, created_at)
             VALUES (?1, ?2, 'similar', ?3, 'embedding', 'proposed', ?4, ?5)
             ON CONFLICT(src, dst, kind) DO UPDATE SET weight = excluded.weight",
            params![src, dst, *similarity as f64, PIPELINE_VERSION, now],
        )?;
    }
    tx.commit()?;
    Ok(keep)
}

/// Other item → status of its `similar` link with this item.
fn existing_links(conn: &Connection, item_id: &str) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare(
        "SELECT CASE WHEN src = ?1 THEN dst ELSE src END, status FROM edges
         WHERE kind = 'similar' AND (src = ?1 OR dst = ?1)",
    )?;
    let rows = stmt.query_map([item_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Items that have no links at all.
#[cfg(test)]
fn unlinked(conn: &Connection) -> Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare("SELECT id FROM items WHERE id NOT IN (SELECT src FROM edges UNION SELECT dst FROM edges)")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::db::chunks::{replace, unit};
    use crate::db::items::{sample, upsert};
    use crate::db::open_temp;

    #[test]
    fn links_similar_items_and_respects_rejections() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            let vectors = [
                ("rust-1", unit(&[1.0, 0.1, 0.0])),
                ("rust-2", unit(&[0.95, 0.2, 0.0])),
                ("bread", unit(&[0.0, 0.1, 1.0])),
            ];
            for (i, (id, v)) in vectors.iter().enumerate() {
                upsert(conn, &sample(id, &format!("items/{id}.md"), &i.to_string()), "")?;
                replace(conn, id, &["x".into()], std::slice::from_ref(v))?;
            }

            let links = link(conn, "rust-1", &[vectors[0].1.clone()])?;
            assert_eq!(links.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), vec!["rust-2"]);
            assert_eq!(unlinked(conn)?, HashSet::from(["bread".to_string()]));

            // Linking the other side finds the same pair, stored once.
            link(conn, "rust-2", &[vectors[1].1.clone()])?;
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
            assert_eq!(count, 1);

            // A rejected link is never re-created.
            conn.execute("UPDATE edges SET status = 'rejected'", [])?;
            assert!(link(conn, "rust-1", &[vectors[0].1.clone()])?.is_empty());
            let status: String = conn.query_row("SELECT status FROM edges", [], |r| r.get(0))?;
            assert_eq!(status, "rejected");
            Ok(())
        })
        .unwrap();
    }
}
