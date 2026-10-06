//! The graph topics are found in: items as nodes, how strongly two items relate as the weight.
//!
//! Weights come from every kind of relation, strongest first: a link made by hand, a typed link
//! the AI proposed, a link found by meaning (by how sure it is), and items sharing a concept.
//! An item with hardly any links is also tied, weakly, to the items nearest to it by meaning, so
//! it can still find its group.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;

use super::cluster::Adjacency;
use crate::db::{chunks, concepts, graph};
use crate::error::Result;
use crate::pipeline::link::Tier;

/// A concept shared by more items than this says too little about any pair of them.
const MAX_CONCEPT_ITEMS: usize = 30;
/// Items with fewer links than this get their nearest neighbours added.
const MIN_LINKS: usize = 2;
const NEIGHBOURS: usize = 3;
/// The least similar a nearest neighbour may be to be tied to an item.
const MIN_NEIGHBOUR_SIMILARITY: f32 = 0.45;

pub struct TopicGraph {
    /// The item behind each node, sorted by id.
    pub ids: Vec<String>,
    pub adj: Adjacency,
}

/// How much a link counts for grouping.
pub fn edge_weight(kind: &str, tier: Tier) -> f64 {
    let base = match tier {
        Tier::Strong => 1.0,
        Tier::Related => 0.5,
        Tier::Maybe => 0.15,
    };
    match kind {
        "user" => 2.0,
        kind if graph::is_ai_kind(kind) => base * 1.3,
        _ => base,
    }
}

/// Builds the graph over the items that have been linked.
pub fn build(conn: &Connection) -> Result<TopicGraph> {
    let mut stmt = conn.prepare("SELECT id FROM items WHERE status = 'linked' ORDER BY id")?;
    let ids: Vec<String> = stmt.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let mut pairs: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    fn add(pairs: &mut BTreeMap<(usize, usize), f64>, a: usize, b: usize, weight: f64) {
        if a != b {
            *pairs.entry((a.min(b), a.max(b))).or_insert(0.0) += weight;
        }
    }

    for edge in graph::edges(conn)? {
        if let (Some(&a), Some(&b)) = (index.get(edge.src.as_str()), index.get(edge.dst.as_str())) {
            add(&mut pairs, a, b, edge_weight(&edge.kind, edge.tier));
        }
    }

    for concept in concepts::with_items(conn)? {
        let members: Vec<usize> = concept.items.iter().filter_map(|id| index.get(id.as_str()).copied()).collect();
        if members.len() < 2 || members.len() > MAX_CONCEPT_ITEMS {
            continue;
        }
        // A rarer concept ties its items together more tightly.
        let weight = 0.6 / (members.len() as f64).sqrt();
        for (i, &a) in members.iter().enumerate() {
            for &b in &members[i + 1..] {
                add(&mut pairs, a, b, weight);
            }
        }
    }

    let mut degree = vec![0usize; ids.len()];
    for &(a, b) in pairs.keys() {
        degree[a] += 1;
        degree[b] += 1;
    }
    for (i, id) in ids.iter().enumerate() {
        if degree[i] >= MIN_LINKS {
            continue;
        }
        let Some(vector) = chunks::item_vector(conn, id)? else { continue };
        let near = chunks::best_per_item(chunks::nearest(conn, &vector, 12)?, id);
        let mut near: Vec<(String, f32)> = near.into_iter().filter(|(_, s)| *s >= MIN_NEIGHBOUR_SIMILARITY).collect();
        near.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (other, similarity) in near.into_iter().take(NEIGHBOURS) {
            if let Some(&j) = index.get(other.as_str()) {
                add(&mut pairs, i, j, 0.3 * similarity as f64);
            }
        }
    }

    let mut adj: Adjacency = vec![Vec::new(); ids.len()];
    for ((a, b), weight) in pairs {
        adj[a].push((b, weight));
        adj[b].push((a, weight));
    }
    for edges in adj.iter_mut() {
        edges.sort_by_key(|&(other, _)| other);
    }
    Ok(TopicGraph { ids, adj })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::db::items::{self, Status, sample};

    fn add_item(conn: &mut Connection, id: &str) {
        let mut item = sample(id, &format!("items/{id}.md"), id);
        item.status = Status::Linked;
        items::upsert(conn, &item, "").unwrap();
        items::set_status(conn, id, Status::Linked, 1).unwrap();
    }

    fn add_edge(conn: &Connection, a: &str, b: &str, kind: &str, tier: &str) {
        conn.execute(
            "INSERT INTO edges (src, dst, kind, weight, tier, provenance, status, pipeline_version, created_at)
             VALUES (?1, ?2, ?3, 0.8, ?4, 'embedding', 'proposed', 1, 't')",
            [a, b, kind, tier],
        )
        .unwrap();
    }

    #[test]
    fn links_and_shared_concepts_become_weights() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        db.with(|conn| {
            for id in ["a", "b", "c", "d"] {
                add_item(conn, id);
            }
            add_edge(conn, "a", "b", "similar", "strong");
            add_edge(conn, "a", "c", "similar", "maybe");
            conn.execute("INSERT INTO user_links (src, dst) VALUES ('c', 'd')", [])?;
            let concept = concepts::create(conn, "Rust", &[1.0, 0.0])?;
            concepts::set_for_item(conn, "a", &[(concept.id, 1.0)])?;
            concepts::set_for_item(conn, "d", &[(concept.id, 1.0)])?;

            let graph = build(conn)?;
            assert_eq!(graph.ids, vec!["a", "b", "c", "d"]);
            let weight = |x: usize, y: usize| graph.adj[x].iter().find(|(o, _)| *o == y).map(|(_, w)| *w);
            assert_eq!(weight(0, 1), Some(1.0), "a strong similarity link");
            assert_eq!(weight(1, 0), Some(1.0), "undirected");
            assert_eq!(weight(0, 2), Some(0.15), "a maybe link counts little");
            assert_eq!(weight(2, 3), Some(2.0), "a link made by hand counts most");
            assert!(weight(0, 3).is_some_and(|w| w > 0.0 && w < 0.5), "a shared concept ties items together a little");
            assert_eq!(weight(1, 2), None);
            Ok(())
        })
        .unwrap();
    }
}
