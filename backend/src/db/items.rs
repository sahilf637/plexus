//! Items in the index: one row per note file.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::capture::detect::Kind;
use crate::error::Result;

/// Where an item is in the background pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Waiting to be chunked, embedded and linked.
    Pending,
    Linked,
    /// Nothing to link by (e.g. a file with no text yet).
    Unlinkable,
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Linked => "linked",
            Status::Unlinkable => "unlinkable",
            Status::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Status {
        match text {
            "linked" => Status::Linked,
            "unlinkable" => Status::Unlinkable,
            "failed" => Status::Failed,
            _ => Status::Pending,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    /// Note file, relative to the library root.
    pub path: String,
    pub kind: Kind,
    pub title: String,
    pub source_url: Option<String>,
    pub language: Option<String>,
    pub captured_at: String,
    pub body_hash: String,
    /// Original file, relative to the library root.
    pub asset_path: Option<String>,
    pub asset_hash: Option<String>,
    pub mime: Option<String>,
    pub size: Option<i64>,
    #[serde(skip)]
    pub file_mtime: i64,
    #[serde(skip)]
    pub file_size: i64,
    pub status: Status,
}

/// What reconcile needs to know to skip unchanged note files.
#[derive(Clone, Debug, PartialEq)]
pub struct FileState {
    pub id: String,
    pub mtime: i64,
    pub size: i64,
}

const COLUMNS: &str = "id, path, kind, title, source_url, language, captured_at, body_hash, \
                       asset_path, asset_hash, mime, size, file_mtime, file_size, status";

fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Item> {
    let kind: String = row.get(2)?;
    let status: String = row.get(14)?;
    Ok(Item {
        id: row.get(0)?,
        path: row.get(1)?,
        kind: Kind::parse(&kind),
        title: row.get(3)?,
        source_url: row.get(4)?,
        language: row.get(5)?,
        captured_at: row.get(6)?,
        body_hash: row.get(7)?,
        asset_path: row.get(8)?,
        asset_hash: row.get(9)?,
        mime: row.get(10)?,
        size: row.get(11)?,
        file_mtime: row.get(12)?,
        file_size: row.get(13)?,
        status: Status::parse(&status),
    })
}

/// Inserts or replaces the item and its keyword-search entry. A changed item goes back to
/// `pending`, so the pipeline picks it up again.
pub fn upsert(conn: &mut Connection, item: &Item, body: &str) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        &format!(
            "INSERT INTO items ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
               path = excluded.path, kind = excluded.kind, title = excluded.title,
               source_url = excluded.source_url, language = excluded.language,
               captured_at = excluded.captured_at, body_hash = excluded.body_hash,
               asset_path = excluded.asset_path, asset_hash = excluded.asset_hash,
               mime = excluded.mime, size = excluded.size, file_mtime = excluded.file_mtime,
               file_size = excluded.file_size, status = excluded.status"
        ),
        params![
            item.id,
            item.path,
            item.kind.as_str(),
            item.title,
            item.source_url,
            item.language,
            item.captured_at,
            item.body_hash,
            item.asset_path,
            item.asset_hash,
            item.mime,
            item.size,
            item.file_mtime,
            item.file_size,
            item.status.as_str(),
        ],
    )?;
    tx.execute("DELETE FROM items_fts WHERE item_id = ?1", [&item.id])?;
    tx.execute(
        "INSERT INTO items_fts (item_id, title, body) VALUES (?1, ?2, ?3)",
        params![item.id, item.title, body],
    )?;
    tx.commit()?;
    Ok(())
}

/// Removes the item and everything derived from it.
pub fn delete(conn: &mut Connection, id: &str) -> Result<()> {
    let tx = conn.transaction()?;
    // vec_chunks is a virtual table, so foreign keys can't clean it up.
    tx.execute("DELETE FROM vec_chunks WHERE rowid IN (SELECT id FROM chunks WHERE item_id = ?1)", [id])?;
    tx.execute("DELETE FROM items_fts WHERE item_id = ?1", [id])?;
    tx.execute("DELETE FROM items WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(())
}

pub fn set_status(conn: &Connection, id: &str, status: Status, pipeline_version: i64) -> Result<()> {
    conn.execute(
        "UPDATE items SET status = ?2, pipeline_version = ?3 WHERE id = ?1",
        params![id, status.as_str(), pipeline_version],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Item>> {
    let sql = format!("SELECT {COLUMNS} FROM items WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], from_row).optional()?)
}

pub fn all(conn: &Connection) -> Result<Vec<Item>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM items ORDER BY captured_at"))?;
    let items = stmt.query_map([], from_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(items)
}

/// An existing item with the same text or the same original file.
pub fn find_duplicate(conn: &Connection, body_hash: &str, asset_hash: Option<&str>) -> Result<Option<Item>> {
    let sql = match asset_hash {
        Some(_) => format!("SELECT {COLUMNS} FROM items WHERE asset_hash = ?1 LIMIT 1"),
        None => format!("SELECT {COLUMNS} FROM items WHERE body_hash = ?1 AND asset_hash IS NULL LIMIT 1"),
    };
    Ok(conn.query_row(&sql, [asset_hash.unwrap_or(body_hash)], from_row).optional()?)
}

/// Note path → what the index knows about that file.
pub fn file_states(conn: &Connection) -> Result<HashMap<String, FileState>> {
    let mut stmt = conn.prepare("SELECT path, id, file_mtime, file_size FROM items")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, FileState { id: row.get(1)?, mtime: row.get(2)?, size: row.get(3)? }))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
pub fn count(conn: &Connection) -> Result<usize> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))?;
    Ok(count as usize)
}

#[cfg(test)]
pub(crate) fn sample(id: &str, path: &str, body_hash: &str) -> Item {
    Item {
        id: id.into(),
        path: path.into(),
        kind: Kind::Text,
        title: format!("Item {id}"),
        source_url: None,
        language: None,
        captured_at: "2026-10-05T10:00:00Z".into(),
        body_hash: body_hash.into(),
        asset_path: None,
        asset_hash: None,
        mime: None,
        size: None,
        file_mtime: 1,
        file_size: 10,
        status: Status::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_temp;

    #[test]
    fn upsert_get_find_delete() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            let item = sample("a", "items/a.md", "h1");
            upsert(conn, &item, "rust ownership rules")?;
            assert_eq!(get(conn, "a")?, Some(item.clone()));

            let moved = Item { path: "items/b.md".into(), title: "Renamed".into(), ..item };
            upsert(conn, &moved, "rust borrowing")?;
            assert_eq!(count(conn)?, 1);
            assert_eq!(get(conn, "a")?.unwrap().path, "items/b.md");
            let hits: i64 =
                conn.query_row("SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH 'borrowing'", [], |r| r.get(0))?;
            assert_eq!(hits, 1);

            assert_eq!(find_duplicate(conn, "h1", None)?.map(|i| i.id), Some("a".into()));
            assert!(find_duplicate(conn, "other", None)?.is_none());

            delete(conn, "a")?;
            assert_eq!(count(conn)?, 0);
            let fts: i64 = conn.query_row("SELECT COUNT(*) FROM items_fts", [], |r| r.get(0))?;
            assert_eq!(fts, 0);
            Ok(())
        })
        .unwrap();
    }
}
