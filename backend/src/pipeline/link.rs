//! Finding which items an item is related to, by meaning, and storing those links.
//!
//! Like Atomic's `compute_semantic_edges_for_atom`: every chunk of the item looks up its nearest
//! chunks elsewhere, each other item counts with its best-matching chunk, and the closest few
//! become `similar` links, each with a tier (strong, related, maybe) saying how sure it is.

use std::collections::HashMap;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, params};

use serde::Serialize;

use crate::db::{chunks, items};
use crate::error::Result;

/// How sure a link is, from how similar the two items are. Tuned on a seeded library of 218
/// notes across 15 topics (`pipeline::tune`): strong links are 99% within a topic (~2 per note),
/// related ones 84% (~3; many of the rest are real, like Maillard reaction ↔ coffee roasting),
/// maybe ones 67%. Long encyclopedic notes score high against nearly everything, hence the high
/// cutoffs; short notes mostly land in the lower tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Maybe,
    Related,
    Strong,
}

impl Tier {
    pub const STRONG: f32 = 0.72;
    pub const RELATED: f32 = 0.67;
    pub const MAYBE: f32 = 0.62;

    pub fn of(similarity: f32) -> Option<Tier> {
        match similarity {
            s if s >= Self::STRONG => Some(Tier::Strong),
            s if s >= Self::RELATED => Some(Tier::Related),
            s if s >= Self::MAYBE => Some(Tier::Maybe),
            _ => None,
        }
    }

    /// Links an item starts with in this tier. Links other items made to it are kept while they
    /// still clear a tier, so these aren't hard caps.
    fn cap(self) -> usize {
        match self {
            Tier::Strong => 5,
            Tier::Related | Tier::Maybe => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Strong => "strong",
            Tier::Related => "related",
            Tier::Maybe => "maybe",
        }
    }

    pub fn parse(text: &str) -> Tier {
        match text {
            "strong" => Tier::Strong,
            "maybe" => Tier::Maybe,
            _ => Tier::Related,
        }
    }
}

/// Bumped when how links are computed changes; links from an older version are recomputed.
pub const PIPELINE_VERSION: i64 = 3;
const NEIGHBOURS_PER_CHUNK: usize = 40;

/// Recomputes the item's `similar` links from its chunk vectors. `name_only` items (a file known
/// only by its name) and links to them are never more than `maybe`. Returns (other item,
/// similarity, tier), closest first.
pub fn link(conn: &mut Connection, item_id: &str, vectors: &[Vec<f32>], name_only: bool) -> Result<Vec<(String, f32, Tier)>> {
    let mut neighbours = Vec::new();
    for vector in vectors {
        neighbours.extend(chunks::nearest(conn, vector, NEIGHBOURS_PER_CHUNK)?);
    }
    let existing = existing_links(conn, item_id)?;
    let named_only = items::name_only_ids(conn)?;
    let mut candidates: Vec<(String, f32, Tier)> = chunks::best_per_item(neighbours, item_id)
        .into_iter()
        .filter(|(other, _)| existing.get(other).map(String::as_str) != Some("rejected"))
        .filter_map(|(other, similarity)| {
            let tier = Tier::of(similarity)?;
            let tier = if name_only || named_only.contains(&other) { tier.min(Tier::Maybe) } else { tier };
            Some((other, similarity, tier))
        })
        .collect();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    // The closest few in each tier, plus links other items made to this one.
    let mut taken: HashMap<Tier, usize> = HashMap::new();
    let keep: Vec<(String, f32, Tier)> = candidates
        .into_iter()
        .filter(|(other, _, tier)| {
            let count = taken.entry(*tier).or_default();
            *count += 1;
            *count <= tier.cap() || existing.contains_key(other)
        })
        .collect();

    let tx = conn.transaction()?;
    // Rejected links stay, so they're remembered and never come back.
    tx.execute(
        "DELETE FROM edges WHERE kind = 'similar' AND provenance = 'embedding' AND status != 'rejected'
           AND (src = ?1 OR dst = ?1)",
        [item_id],
    )?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    for (other, similarity, tier) in &keep {
        let (src, dst) = if item_id < other.as_str() { (item_id, other.as_str()) } else { (other.as_str(), item_id) };
        tx.execute(
            "INSERT INTO edges (src, dst, kind, weight, tier, provenance, status, pipeline_version, created_at)
             VALUES (?1, ?2, 'similar', ?3, ?4, 'embedding', 'proposed', ?5, ?6)
             ON CONFLICT(src, dst, kind) DO UPDATE SET weight = excluded.weight, tier = excluded.tier",
            params![src, dst, *similarity as f64, tier.as_str(), PIPELINE_VERSION, now],
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

    /// A unit vector at the given similarity to `[1, 0, …]`, leaning along `axis`.
    fn at(similarity: f32, axis: usize) -> Vec<f32> {
        let mut v = vec![similarity, 0.0, 0.0, 0.0, 0.0];
        v[axis] = (1.0 - similarity * similarity).sqrt();
        unit(&v)
    }

    fn setup(conn: &mut Connection, vectors: &[(&str, Vec<f32>)]) -> Result<()> {
        for (i, (id, v)) in vectors.iter().enumerate() {
            upsert(conn, &sample(id, &format!("items/{id}.md"), &i.to_string()), "")?;
            replace(conn, id, &["x".into()], std::slice::from_ref(v))?;
        }
        Ok(())
    }

    fn tiers(links: &[(String, f32, Tier)]) -> Vec<(&str, Tier)> {
        links.iter().map(|(id, _, tier)| (id.as_str(), *tier)).collect()
    }

    #[test]
    fn links_fall_into_tiers_by_similarity() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            let note = unit(&[1.0, 0.0, 0.0, 0.0, 0.0]);
            setup(conn, &[("note", note.clone()), ("strong", at(0.8, 1)), ("related", at(0.69, 2)), ("maybe", at(0.64, 3)), ("far", at(0.5, 4))])?;
            let links = link(conn, "note", std::slice::from_ref(&note), false)?;
            assert_eq!(tiers(&links), vec![("strong", Tier::Strong), ("related", Tier::Related), ("maybe", Tier::Maybe)]);
            let stored: String = conn.query_row("SELECT tier FROM edges WHERE src = 'note' AND dst = 'related'", [], |r| r.get(0))?;
            assert_eq!(stored, "related");

            // Known only by its name: nothing better than maybe, either way round.
            let links = link(conn, "note", &[note], true)?;
            assert!(links.iter().all(|(_, _, tier)| *tier == Tier::Maybe));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn links_similar_items_and_respects_rejections() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            let vectors = [
                ("rust-1", unit(&[1.0, 0.1, 0.0])),
                ("rust-2", unit(&[0.95, 0.2, 0.0])),
                ("bread", unit(&[0.0, 0.1, 1.0])),
            ];
            setup(conn, &vectors)?;

            let links = link(conn, "rust-1", &[vectors[0].1.clone()], false)?;
            assert_eq!(tiers(&links), vec![("rust-2", Tier::Strong)]);
            assert_eq!(unlinked(conn)?, HashSet::from(["bread".to_string()]));

            // Linking the other side finds the same pair, stored once.
            link(conn, "rust-2", &[vectors[1].1.clone()], false)?;
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
            assert_eq!(count, 1);

            // A rejected link is never re-created.
            conn.execute("UPDATE edges SET status = 'rejected'", [])?;
            assert!(link(conn, "rust-1", &[vectors[0].1.clone()], false)?.is_empty());
            let status: String = conn.query_row("SELECT status FROM edges", [], |r| r.get(0))?;
            assert_eq!(status, "rejected");
            Ok(())
        })
        .unwrap();
    }
}
