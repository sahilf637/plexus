//! Topics: the tree of groups items live in, and which topic each item is in (only the lowest;
//! its ancestors follow the parent chain). Derived data: it can be built again from the links.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::chunks::{from_blob, to_blob};
use crate::error::Result;

/// Who gave a topic its name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelBy {
    /// Made from the notes' own words.
    #[default]
    Auto,
    /// Named by the AI provider.
    Ai,
    /// Named by the user; never replaced.
    You,
}

impl LabelBy {
    pub fn as_str(self) -> &'static str {
        match self {
            LabelBy::Auto => "auto",
            LabelBy::Ai => "ai",
            LabelBy::You => "you",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "ai" => LabelBy::Ai,
            "you" => LabelBy::You,
            _ => LabelBy::Auto,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Topic {
    pub id: String,
    pub parent_id: Option<String>,
    /// 0 for the top level.
    pub level: i64,
    pub label: String,
    pub label_by: LabelBy,
    pub summary: Option<String>,
    /// The members when the AI named it (see the migration).
    pub named_members: Option<Vec<String>>,
    pub centroid: Option<Vec<f32>>,
    pub version: i64,
}

const COLUMNS: &str = "id, parent_id, level, label, label_by, summary, named_members, centroid, version";

fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Topic> {
    let label_by: String = row.get(4)?;
    let named: Option<String> = row.get(6)?;
    let centroid: Option<Vec<u8>> = row.get(7)?;
    Ok(Topic {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        level: row.get(2)?,
        label: row.get(3)?,
        label_by: LabelBy::parse(&label_by),
        summary: row.get(5)?,
        named_members: named.and_then(|json| serde_json::from_str(&json).ok()),
        centroid: centroid.map(|blob| from_blob(&blob)),
        version: row.get(8)?,
    })
}

/// Every topic, parents before their children.
pub fn all(conn: &Connection) -> Result<Vec<Topic>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM topics ORDER BY level, id"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Topic>> {
    Ok(conn.query_row(&format!("SELECT {COLUMNS} FROM topics WHERE id = ?1"), [id], from_row).optional()?)
}

/// The lowest topic of every item that has one.
pub fn memberships(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT item_id, topic_id FROM topic_members")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Replaces the whole tree and every membership.
pub fn replace_all(conn: &mut Connection, topics: &[Topic], members: &[(String, String)]) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM topic_members", [])?;
    tx.execute("DELETE FROM topics", [])?;
    {
        // Parents first, so a child's parent already exists.
        let mut ordered: Vec<&Topic> = topics.iter().collect();
        ordered.sort_by_key(|t| t.level);
        let mut insert = tx.prepare(&format!("INSERT INTO topics ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"))?;
        for t in ordered {
            insert.execute(params![
                t.id,
                t.parent_id,
                t.level,
                t.label,
                t.label_by.as_str(),
                t.summary,
                t.named_members.as_ref().and_then(|m| serde_json::to_string(m).ok()),
                t.centroid.as_deref().map(to_blob),
                t.version,
            ])?;
        }
        let mut member = tx.prepare("INSERT OR REPLACE INTO topic_members (item_id, topic_id) VALUES (?1, ?2)")?;
        for (item, topic) in members {
            member.execute([item, topic])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Puts an item in a topic (its lowest one), replacing where it was.
pub fn set_member(conn: &Connection, item_id: &str, topic_id: &str) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO topic_members (item_id, topic_id) VALUES (?1, ?2)", [item_id, topic_id])?;
    Ok(())
}

/// A topic's name and summary, and who made them.
pub fn set_label(conn: &Connection, id: &str, label: &str, by: LabelBy, summary: Option<&str>, named_members: Option<&[String]>) -> Result<()> {
    conn.execute(
        "UPDATE topics SET label = ?2, label_by = ?3, summary = ?4, named_members = ?5 WHERE id = ?1",
        params![id, label, by.as_str(), summary, named_members.and_then(|m| serde_json::to_string(m).ok())],
    )?;
    Ok(())
}

/// Removes topics with no items and no sub-topics (after items were deleted), and then any
/// parent that this leaves empty.
pub fn prune(conn: &Connection) -> Result<usize> {
    let mut removed = 0;
    loop {
        let gone = conn.execute(
            "DELETE FROM topics WHERE id NOT IN (SELECT topic_id FROM topic_members)
               AND id NOT IN (SELECT parent_id FROM topics WHERE parent_id IS NOT NULL)",
            [],
        )?;
        if gone == 0 {
            return Ok(removed);
        }
        removed += gone;
    }
}

pub fn clear(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM topic_members", [])?;
    conn.execute("DELETE FROM topics", [])?;
    Ok(())
}

/// The members of every topic, counting the ones in its sub-topics: topic id → item ids.
pub fn members_with_descendants(topics: &[Topic], membership: &HashMap<String, String>) -> HashMap<String, HashSet<String>> {
    let parent: HashMap<&str, Option<&str>> = topics.iter().map(|t| (t.id.as_str(), t.parent_id.as_deref())).collect();
    let mut found: HashMap<String, HashSet<String>> = topics.iter().map(|t| (t.id.clone(), HashSet::new())).collect();
    for (item, leaf) in membership {
        let mut at = Some(leaf.as_str());
        while let Some(id) = at {
            if let Some(set) = found.get_mut(id) {
                set.insert(item.clone());
            }
            at = parent.get(id).copied().flatten();
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn topic(id: &str, parent: Option<&str>, level: i64) -> Topic {
        Topic {
            id: id.into(),
            parent_id: parent.map(String::from),
            level,
            label: id.into(),
            label_by: LabelBy::Auto,
            summary: None,
            named_members: None,
            centroid: None,
            version: 1,
        }
    }

    #[test]
    fn descendants_count_in_their_parents_and_empty_topics_are_pruned() {
        let topics = [topic("a", None, 0), topic("a1", Some("a"), 1), topic("a2", Some("a"), 1), topic("b", None, 0)];
        let membership: HashMap<String, String> = [("x", "a1"), ("y", "a1"), ("z", "a2"), ("w", "b")].map(|(i, t)| (i.into(), t.into())).into();
        let found = members_with_descendants(&topics, &membership);
        assert_eq!(found["a"].len(), 3);
        assert_eq!(found["a1"].len(), 2);
        assert_eq!(found["b"].len(), 1);
    }

    #[test]
    fn replace_all_prune_and_labels() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        db.with(|conn| {
            for id in ["x", "y"] {
                crate::db::items::upsert(conn, &crate::db::items::sample(id, &format!("items/{id}.md"), id), "")?;
            }
            let topics = [topic("a", None, 0), topic("a1", Some("a"), 1), topic("a2", Some("a"), 1)];
            replace_all(conn, &topics, &[("x".into(), "a1".into()), ("y".into(), "a1".into())])?;
            assert_eq!(all(conn)?.len(), 3);
            assert_eq!(memberships(conn)?["x"], "a1");
            // a2 has nothing in it: pruned. a is kept, a1 still has its children link.
            assert_eq!(prune(conn)?, 1);
            assert_eq!(all(conn)?.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["a", "a1"]);
            // Deleting its items empties a1, and then a.
            conn.execute("DELETE FROM items", [])?;
            assert_eq!(prune(conn)?, 2);

            replace_all(conn, &[topic("t", None, 0)], &[])?;
            set_label(conn, "t", "Rust", LabelBy::You, Some("About Rust"), Some(&["x".to_string()]))?;
            let t = get(conn, "t")?.unwrap();
            assert_eq!((t.label.as_str(), t.label_by, t.summary.as_deref()), ("Rust", LabelBy::You, Some("About Rust")));
            assert_eq!(t.named_members, Some(vec!["x".to_string()]));
            Ok(())
        })
        .unwrap();
    }
}
