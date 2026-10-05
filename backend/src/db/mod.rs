//! The index: a SQLite database built from the library folder (items, chunks, vectors, keyword
//! search, edges, layout). Everything here can be rebuilt from the note files, so the file may
//! be deleted at any time.

pub mod chunks;
pub mod graph;
pub mod items;

use std::path::Path;
use std::sync::{Mutex, Once};

use rusqlite::Connection;

use crate::error::Result;

/// Each entry upgrades the schema by one version (`PRAGMA user_version`).
const MIGRATIONS: &[&str] = &[r#"
    CREATE TABLE items (
        id               TEXT PRIMARY KEY,
        path             TEXT NOT NULL UNIQUE,   -- note file, relative to the library root
        kind             TEXT NOT NULL,
        title            TEXT NOT NULL,
        source_url       TEXT,
        language         TEXT,
        captured_at      TEXT NOT NULL,
        body_hash        TEXT NOT NULL,
        asset_path       TEXT,                   -- original file, relative to the library root
        asset_hash       TEXT,
        mime             TEXT,
        size             INTEGER,
        file_mtime       INTEGER NOT NULL,       -- of the note file, to skip unchanged ones
        file_size        INTEGER NOT NULL,
        status           TEXT NOT NULL DEFAULT 'pending',
        pipeline_version INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX items_body_hash ON items(body_hash);
    CREATE INDEX items_asset_hash ON items(asset_hash);
    CREATE INDEX items_status ON items(status);

    CREATE TABLE chunks (
        id      INTEGER PRIMARY KEY,             -- also the rowid in vec_chunks
        item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
        ord     INTEGER NOT NULL,
        text    TEXT NOT NULL
    );
    CREATE INDEX chunks_item ON chunks(item_id);
    CREATE VIRTUAL TABLE vec_chunks USING vec0(embedding float[384]);

    CREATE VIRTUAL TABLE items_fts USING fts5(
        item_id UNINDEXED, title, body, tokenize = 'porter unicode61'
    );

    CREATE TABLE edges (
        src              TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
        dst              TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
        kind             TEXT NOT NULL,          -- similar | supports | contradicts | … | user
        weight           REAL NOT NULL,
        reason           TEXT,
        confidence       REAL,
        provenance       TEXT NOT NULL,          -- embedding | llm | user
        status           TEXT NOT NULL DEFAULT 'proposed',
        pipeline_version INTEGER NOT NULL DEFAULT 0,
        created_at       TEXT NOT NULL,
        PRIMARY KEY (src, dst, kind)
    );
    CREATE INDEX edges_dst ON edges(dst);

    CREATE TABLE layout (
        item_id TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
        x       REAL NOT NULL,
        y       REAL NOT NULL
    );

    CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
"#, r#"
    -- Vectors are compared by cosine distance (similarity = 1 - distance). Nothing was embedded
    -- under version 1, so the table can simply be recreated.
    DROP TABLE vec_chunks;
    DELETE FROM chunks;
    CREATE VIRTUAL TABLE vec_chunks USING vec0(embedding float[384] distance_metric=cosine);
    UPDATE items SET status = 'pending';
"#];

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        register_sqlite_vec();
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        migrate(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Runs `f` with the connection. Keep the work short: other callers wait meanwhile.
    pub fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().expect("db lock poisoned");
        f(&mut conn)
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        conn.execute_batch(&format!("BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;", i + 1))?;
    }
    Ok(())
}

/// Makes the sqlite-vec extension (vector search) available to every connection.
fn register_sqlite_vec() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: `sqlite3_vec_init` is a valid SQLite extension entry point; SQLite calls it
        // with the documented arguments for each new connection.
        unsafe {
            type EntryPoint = unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::ffi::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::ffi::c_int;
            let entry = std::mem::transmute::<*const (), EntryPoint>(sqlite_vec::sqlite3_vec_init as *const ());
            rusqlite::ffi::sqlite3_auto_extension(Some(entry));
        }
    });
}

#[cfg(test)]
pub(crate) fn open_temp() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("index.db")).unwrap();
    (dir, db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_loads_sqlite_vec() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            let version: String = conn.query_row("SELECT vec_version()", [], |r| r.get(0))?;
            assert!(version.starts_with('v'));
            let schema: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            assert_eq!(schema as usize, MIGRATIONS.len());
            Ok(())
        })
        .unwrap();
    }
}
