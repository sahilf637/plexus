//! The map's view of the index: edges between items and where each item sits.

use std::collections::HashMap;

use chrono::{SecondsFormat, Utc};
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

/// Links made by hand (between items that both exist) read as edges: always strong.
const USER_EDGES: &str = "SELECT u.src, u.dst, 'user', 1.0, 'linked by you', 'strong'
    FROM user_links u JOIN items a ON a.id = u.src JOIN items b ON b.id = u.dst";

/// One edge per pair of items, strongest first. A pair can be linked in several ways; a link made
/// by hand wins, then an AI link (its type and reason, with the stronger of the two tiers and
/// weights), then the link found by meaning.
fn collect_edges(found: Vec<Edge>) -> Vec<Edge> {
    let mut pairs: HashMap<(String, String), Vec<Edge>> = HashMap::new();
    for edge in found {
        let key = if edge.src < edge.dst { (edge.src.clone(), edge.dst.clone()) } else { (edge.dst.clone(), edge.src.clone()) };
        pairs.entry(key).or_default().push(edge);
    }
    let mut merged: Vec<Edge> = pairs
        .into_values()
        .map(|group| {
            let best_by = |kind_ok: fn(&str) -> bool| {
                group.iter().filter(|e| kind_ok(&e.kind)).max_by(|a, b| a.weight.total_cmp(&b.weight)).cloned()
            };
            if let Some(user) = best_by(|k| k == "user") {
                return user;
            }
            let tier = group.iter().map(|e| e.tier).max().unwrap_or(Tier::Maybe);
            let weight = group.iter().map(|e| e.weight).fold(f64::MIN, f64::max);
            match best_by(is_ai_kind) {
                Some(ai) => Edge { tier, weight, ..ai },
                None => group.into_iter().max_by(|a, b| a.weight.total_cmp(&b.weight)).expect("a pair has an edge"),
            }
        })
        .collect();
    merged.sort_by(|a, b| b.weight.total_cmp(&a.weight).then_with(|| (&a.src, &a.dst).cmp(&(&b.src, &b.dst))));
    merged
}

/// Links the AI provider proposes: `src` is the item the statement is about ("src extends dst").
pub const AI_KINDS: [&str; 5] = ["supports", "contradicts", "extends", "example_of", "related"];

pub fn is_ai_kind(kind: &str) -> bool {
    AI_KINDS.contains(&kind)
}

/// A link proposed by the AI provider, already checked against the candidates it was shown.
#[derive(Clone, Debug, PartialEq)]
pub struct AiEdge {
    pub other: String,
    pub kind: String,
    pub reason: String,
    pub confidence: f64,
}

impl AiEdge {
    /// Weight and tier on the same scale as links found by meaning.
    fn strength(&self) -> (f64, Tier) {
        let tier = match self.confidence {
            c if c >= 0.8 => Tier::Strong,
            c if c >= 0.55 => Tier::Related,
            _ => Tier::Maybe,
        };
        (0.66 + 0.22 * self.confidence.clamp(0.0, 1.0), tier)
    }
}

/// Replaces the AI links that are about `item_id` with `edges`. Pairs the user removed
/// (`rejected`) stay removed. Returns the other items touched, old and new.
pub fn replace_ai_edges(conn: &mut Connection, item_id: &str, edges: &[AiEdge], version: i64) -> Result<Vec<String>> {
    let tx = conn.transaction()?;
    let mut touched: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT dst FROM edges WHERE src = ?1 AND provenance = 'llm' AND status != 'rejected'",
        )?;
        stmt.query_map([item_id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    tx.execute("DELETE FROM edges WHERE src = ?1 AND provenance = 'llm' AND status != 'rejected'", [item_id])?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    for edge in edges {
        let removed: i64 = tx.query_row(
            "SELECT COUNT(*) FROM edges WHERE provenance = 'llm' AND status = 'rejected'
               AND ((src = ?1 AND dst = ?2) OR (src = ?2 AND dst = ?1))",
            params![item_id, edge.other],
            |row| row.get(0),
        )?;
        if removed > 0 {
            continue;
        }
        let (weight, tier) = edge.strength();
        tx.execute(
            "INSERT OR IGNORE INTO edges (src, dst, kind, weight, reason, confidence, tier, provenance, status, pipeline_version, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'llm', 'proposed', ?8, ?9)",
            params![item_id, edge.other, edge.kind, weight, edge.reason, edge.confidence, tier.as_str(), version, now],
        )?;
        touched.push(edge.other.clone());
    }
    tx.commit()?;
    touched.sort();
    touched.dedup();
    Ok(touched)
}

/// The user removes an AI link: it is hidden, and enrichment won't bring it back.
pub fn reject_ai_edge(conn: &Connection, a: &str, b: &str) -> Result<()> {
    conn.execute(
        "UPDATE edges SET status = 'rejected' WHERE provenance = 'llm' AND status != 'rejected'
           AND ((src = ?1 AND dst = ?2) OR (src = ?2 AND dst = ?1))",
        params![a, b],
    )?;
    Ok(())
}

/// Every edge the user hasn't rejected, plus the links they made, strongest first.
pub fn edges(conn: &Connection) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT src, dst, kind, weight, reason, tier FROM edges WHERE status != 'rejected'",
    )?;
    let mut found: Vec<Edge> = stmt.query_map([], edge_from_row)?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare(USER_EDGES)?;
    found.extend(stmt.query_map([], edge_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?);
    Ok(collect_edges(found))
}

/// The links touching one item.
pub fn edges_of(conn: &Connection, id: &str) -> Result<Vec<Edge>> {
    let mut stmt = conn.prepare(
        "SELECT src, dst, kind, weight, reason, tier FROM edges
         WHERE status != 'rejected' AND (src = ?1 OR dst = ?1)",
    )?;
    let mut found: Vec<Edge> = stmt.query_map([id], edge_from_row)?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare(&format!("{USER_EDGES} WHERE u.src = ?1 OR u.dst = ?1"))?;
    found.extend(stmt.query_map([id], edge_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?);
    Ok(collect_edges(found))
}

/// Replaces the links made by hand that the note of `src` holds.
pub fn set_user_links(conn: &mut Connection, src: &str, dst: &[String]) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM user_links WHERE src = ?1", [src])?;
    for id in dst.iter().filter(|id| id.as_str() != src) {
        tx.execute("INSERT OR IGNORE INTO user_links (src, dst) VALUES (?1, ?2)", [src, id])?;
    }
    tx.commit()?;
    Ok(())
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

    #[test]
    fn ai_links_merge_with_similar_ones_and_can_be_removed() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            for (id, hash) in [("a", "1"), ("b", "2"), ("c", "3")] {
                upsert(conn, &sample(id, &format!("items/{id}.md"), hash), "")?;
            }
            // a–b is found by meaning (maybe); the AI says b extends a, and links c too.
            conn.execute(
                "INSERT INTO edges (src, dst, kind, weight, tier, provenance, created_at)
                 VALUES ('a', 'b', 'similar', 0.64, 'maybe', 'embedding', 'now')",
                [],
            )?;
            let proposed = [
                AiEdge { other: "a".into(), kind: "extends".into(), reason: "adds the borrow rules".into(), confidence: 0.9 },
                AiEdge { other: "c".into(), kind: "related".into(), reason: "same author".into(), confidence: 0.3 },
            ];
            assert_eq!(replace_ai_edges(conn, "b", &proposed, 1)?, vec!["a".to_string(), "c".to_string()]);

            let all = edges(conn)?;
            assert_eq!(all.len(), 2, "one edge per pair: {all:?}");
            let ab = all.iter().find(|e| e.dst == "b" || e.src == "b").filter(|e| e.kind == "extends").unwrap();
            assert_eq!((ab.src.as_str(), ab.tier, ab.reason.as_deref()), ("b", Tier::Strong, Some("adds the borrow rules")));
            let bc = all.iter().find(|e| e.kind == "related").unwrap();
            assert_eq!(bc.tier, Tier::Maybe);

            // Removing an AI link keeps the local one and survives the next enrichment.
            reject_ai_edge(conn, "a", "b")?;
            let left = edges(conn)?;
            assert!(left.iter().any(|e| e.kind == "similar"));
            replace_ai_edges(conn, "b", &proposed, 1)?;
            assert!(!edges(conn)?.iter().any(|e| e.kind == "extends"));
            Ok(())
        })
        .unwrap();
    }
}
