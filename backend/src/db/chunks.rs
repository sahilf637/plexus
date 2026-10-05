//! Chunks of notes and their embedding vectors, and nearest-neighbour search over them
//! (sqlite-vec, cosine distance).

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;

/// A chunk near a query vector: which item it belongs to and how similar it is (0..1).
#[derive(Clone, Debug, PartialEq)]
pub struct Neighbour {
    pub item_id: String,
    pub similarity: f32,
}

/// Replaces the item's chunks and vectors. Returns the new chunk ids, in order.
pub fn replace(conn: &mut Connection, item_id: &str, texts: &[String], vectors: &[Vec<f32>]) -> Result<Vec<i64>> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM vec_chunks WHERE rowid IN (SELECT id FROM chunks WHERE item_id = ?1)", [item_id])?;
    tx.execute("DELETE FROM chunks WHERE item_id = ?1", [item_id])?;
    let mut ids = Vec::with_capacity(texts.len());
    for (ord, (text, vector)) in texts.iter().zip(vectors).enumerate() {
        tx.execute("INSERT INTO chunks (item_id, ord, text) VALUES (?1, ?2, ?3)", params![item_id, ord as i64, text])?;
        let id = tx.last_insert_rowid();
        tx.execute("INSERT INTO vec_chunks (rowid, embedding) VALUES (?1, ?2)", params![id, to_blob(vector)])?;
        ids.push(id);
    }
    tx.commit()?;
    Ok(ids)
}

/// The `k` chunks nearest to `vector`, best first, with their items.
pub fn nearest(conn: &Connection, vector: &[f32], k: usize) -> Result<Vec<Neighbour>> {
    let mut stmt = conn.prepare(
        "SELECT c.item_id, v.distance
         FROM (SELECT rowid, distance FROM vec_chunks WHERE embedding MATCH ?1 AND k = ?2) v
         JOIN chunks c ON c.id = v.rowid
         ORDER BY v.distance",
    )?;
    let rows = stmt.query_map(params![to_blob(vector), k as i64], |row| {
        let distance: f64 = row.get(1)?;
        Ok(Neighbour { item_id: row.get(0)?, similarity: (1.0 - distance) as f32 })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Best similarity per item among `neighbours`, leaving out `exclude`.
pub fn best_per_item(neighbours: impl IntoIterator<Item = Neighbour>, exclude: &str) -> HashMap<String, f32> {
    let mut best: HashMap<String, f32> = HashMap::new();
    for n in neighbours.into_iter().filter(|n| n.item_id != exclude) {
        let entry = best.entry(n.item_id).or_insert(f32::MIN);
        *entry = entry.max(n.similarity);
    }
    best
}

pub fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0)).optional()?)
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// Forgets every vector and similarity link, e.g. after switching embedding models, so all
/// items are embedded again.
pub fn reset_embeddings(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM vec_chunks", [])?;
    tx.execute("DELETE FROM chunks", [])?;
    tx.execute("DELETE FROM edges WHERE provenance = 'embedding'", [])?;
    tx.execute("UPDATE items SET status = 'pending'", [])?;
    tx.commit()?;
    Ok(())
}

/// Drops the links found by meaning (except rejected ones, which stay remembered) so every
/// item is linked again, e.g. after the linking rules changed.
pub fn reset_links(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM edges WHERE provenance = 'embedding' AND status != 'rejected'", [])?;
    tx.execute("UPDATE items SET status = 'pending'", [])?;
    tx.commit()?;
    Ok(())
}

pub fn pending_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM items WHERE status = 'pending' ORDER BY captured_at")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[cfg(test)]
pub(crate) fn unit(values: &[f32]) -> Vec<f32> {
    let mut v = values.to_vec();
    v.resize(384, 0.0);
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / norm).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::items::{sample, upsert};
    use crate::db::open_temp;

    #[test]
    fn stores_and_finds_neighbours() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            for (id, hash) in [("a", "1"), ("b", "2"), ("c", "3")] {
                upsert(conn, &sample(id, &format!("items/{id}.md"), hash), "")?;
            }
            replace(conn, "a", &["a0".into(), "a1".into()], &[unit(&[1.0, 0.0]), unit(&[0.9, 0.1])])?;
            replace(conn, "b", &["b0".into()], &[unit(&[0.8, 0.2])])?;
            replace(conn, "c", &["c0".into()], &[unit(&[0.0, 1.0])])?;

            let near = nearest(conn, &unit(&[1.0, 0.0]), 10)?;
            assert_eq!(near[0].item_id, "a");
            assert!((near[0].similarity - 1.0).abs() < 1e-4);
            let best = best_per_item(near, "a");
            assert!(best["b"] > 0.9 && best["c"] < 0.1);

            // Replacing drops the old vectors.
            replace(conn, "a", &["a0".into()], &[unit(&[0.0, 1.0])])?;
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM vec_chunks", [], |r| r.get(0))?;
            assert_eq!(count, 3);
            assert_eq!(pending_ids(conn)?.len(), 3);

            set_meta(conn, "embed_model", "x")?;
            assert_eq!(get_meta(conn, "embed_model")?.as_deref(), Some("x"));
            reset_embeddings(conn)?;
            let left: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
            assert_eq!(left, 0);
            Ok(())
        })
        .unwrap();
    }
}
