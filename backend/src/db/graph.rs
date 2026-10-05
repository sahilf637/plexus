//! The map's view of the index: edges between items and where each item sits.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::pipeline::link::Tier;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub src: String,
    pub dst: String,
    pub kind: String,
    pub weight: f64,
    /// How sure the link is: strong, related or maybe.
    pub tier: Tier,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub x: f64,
    pub y: f64,
}

fn edge_from_row(row: &rusqlite::Row) -> rusqlite::Result<Edge> {
    let tier: String = row.get(5)?;
    Ok(Edge {
        src: row.get(0)?,
        dst: row.get(1)?,
        kind: row.get(2)?,
        weight: row.get(3)?,
        reason: row.get(4)?,
        tier: Tier::parse(&tier),
    })
}

/// Every edge the user hasn't rejected.
pub fn edges(conn: &Connection) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT src, dst, kind, weight, reason, tier FROM edges WHERE status != 'rejected' ORDER BY weight DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        edge_from_row(row)
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The links touching one item.
pub fn edges_of(conn: &Connection, id: &str) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT src, dst, kind, weight, reason, tier FROM edges
         WHERE status != 'rejected' AND (src = ?1 OR dst = ?1) ORDER BY weight DESC",
    )?;
    let rows = stmt.query_map([id], |row| {
        edge_from_row(row)
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn positions(conn: &Connection) -> Result<Vec<Position>> {
    let mut stmt = conn.prepare("SELECT item_id, x, y FROM layout")?;
    let rows = stmt.query_map([], |row| Ok(Position { id: row.get(0)?, x: row.get(1)?, y: row.get(2)? }))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Stores positions for items that still exist (an item may be deleted while the map lays out).
pub fn save_positions(conn: &mut Connection, positions: &[Position]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO layout (item_id, x, y) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM items WHERE id = ?1)
             ON CONFLICT(item_id) DO UPDATE SET x = excluded.x, y = excluded.y",
        )?;
        for p in positions {
            stmt.execute(params![p.id, p.x, p.y])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Keyword search over titles and bodies, best matches first.
pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<String>> {
    let Some(fts_query) = fts_query(query) else { return Ok(Vec::new()) };
    let mut stmt = conn.prepare(
        "SELECT item_id FROM items_fts WHERE items_fts MATCH ?1 ORDER BY bm25(items_fts, 0.0, 4.0, 1.0) LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![fts_query, limit as i64], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Turns what the user typed into an FTS5 query: every word must match, the last one as a
/// prefix (so results appear while typing). Quotes keep FTS syntax characters literal.
fn fts_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\""))
        .collect();
    let (last, rest) = words.split_last()?;
    Some(rest.iter().cloned().chain([format!("{last}*")]).collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::items::{sample, upsert};
    use crate::db::open_temp;

    #[test]
    fn positions_and_search() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            upsert(conn, &sample("a", "items/a.md", "h1"), "Rust ownership and borrowing")?;
            upsert(conn, &sample("b", "items/b.md", "h2"), "Sourdough bread recipe")?;

            let wanted = vec![Position { id: "a".into(), x: 1.0, y: 2.0 }, Position { id: "gone".into(), x: 0.0, y: 0.0 }];
            save_positions(conn, &wanted)?;
            assert_eq!(positions(conn)?, vec![wanted[0].clone()]);

            assert_eq!(search(conn, "borrow", 10)?, vec!["a".to_string()]);
            assert_eq!(search(conn, "RUST own", 10)?, vec!["a".to_string()]);
            assert_eq!(search(conn, "bread\"*(:", 10)?, vec!["b".to_string()]);
            assert!(search(conn, "  ", 10)?.is_empty());
            assert!(edges(conn)?.is_empty());
            Ok(())
        })
        .unwrap();
    }
}
