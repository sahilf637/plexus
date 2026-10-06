//! The index: a SQLite database built from the library folder (items, chunks, vectors, keyword
//! search, edges, layout). Everything here can be rebuilt from the note files, so the file may
//! be deleted at any time.

pub mod asks;
pub mod chunks;
pub mod concepts;
pub mod graph;
pub mod items;
pub mod topics;

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
"#, r#"
    -- What produced the note's body (from the frontmatter). Links without it still need their
    -- web page read.
    ALTER TABLE items ADD COLUMN extracted TEXT;
    CREATE INDEX items_source_url ON items(source_url);
"#, r#"
    -- How sure a link is: strong | related | maybe.
    ALTER TABLE edges ADD COLUMN tier TEXT NOT NULL DEFAULT 'related';
    -- Set when an item could only be linked by its name (a file with no text yet).
    ALTER TABLE items ADD COLUMN name_only INTEGER NOT NULL DEFAULT 0;
"#, r#"
    -- Links made by hand, copied from the `links:` of the note that holds them (`src`). No
    -- foreign keys: a link to a deleted item comes back if the item is restored.
    CREATE TABLE user_links (
        src TEXT NOT NULL,
        dst TEXT NOT NULL,
        PRIMARY KEY (src, dst)
    );
    CREATE INDEX user_links_dst ON user_links(dst);
"#, r#"
    -- AI enrichment (M2). All of it is derived and can be made again.
    ALTER TABLE items ADD COLUMN summary TEXT;

    -- A small shared vocabulary of ideas. The embedding of the name finds the ones relevant to
    -- an item and merges near-duplicates.
    CREATE TABLE concepts (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
        embedding  BLOB NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE TABLE concept_aliases (
        alias      TEXT PRIMARY KEY COLLATE NOCASE,
        concept_id INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE
    );
    CREATE TABLE item_concepts (
        item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
        concept_id INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
        weight     REAL NOT NULL DEFAULT 1,
        PRIMARY KEY (item_id, concept_id)
    );
    CREATE INDEX item_concepts_concept ON item_concepts(concept_id);

    -- What was last sent to the provider for an item: the same content (and enrichment version)
    -- is never sent twice. `title` is the AI's title, to tell it from one the user changed.
    CREATE TABLE enrichment (
        item_id      TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
        content_hash TEXT NOT NULL,
        version      INTEGER NOT NULL,
        title        TEXT,
        enriched_at  TEXT NOT NULL
    );
"#, r#"
    -- Topics (M3): a tree of groups, built from the links. All derived; rebuilt when the
    -- grouping rules change. Each item sits in one lowest topic; its ancestors follow the
    -- parent chain. `label_by` says who named it (auto, ai or you); only a name the user set
    -- can't be made again.
    CREATE TABLE topics (
        id            TEXT PRIMARY KEY,
        parent_id     TEXT REFERENCES topics(id) ON DELETE CASCADE,
        level         INTEGER NOT NULL,
        label         TEXT NOT NULL,
        label_by      TEXT NOT NULL DEFAULT 'auto',
        summary       TEXT,
        -- The members (a JSON list of item ids) when the AI named it, to tell later how much
        -- the topic has changed since.
        named_members TEXT,
        -- The mean of its members' vectors, for placing new items.
        centroid      BLOB,
        version       INTEGER NOT NULL
    );
    CREATE INDEX topics_parent ON topics(parent_id);
    CREATE TABLE topic_members (
        item_id  TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
        topic_id TEXT NOT NULL REFERENCES topics(id) ON DELETE CASCADE
    );
    CREATE INDEX topic_members_topic ON topic_members(topic_id);
"#, r#"
    -- Questions asked of the library and their answers (M4). Derived data, like the rest.
    CREATE TABLE asks (
        id         TEXT PRIMARY KEY,
        scope      TEXT,                       -- a topic or item id, or NULL for the whole library
        question   TEXT NOT NULL,
        answer     TEXT NOT NULL DEFAULT '',
        effort     TEXT NOT NULL DEFAULT 'explain',
        state      TEXT NOT NULL DEFAULT 'running',
        created_at TEXT NOT NULL
    );
    CREATE INDEX asks_created ON asks(created_at);
    CREATE TABLE ask_sources (
        ask_id  TEXT NOT NULL REFERENCES asks(id) ON DELETE CASCADE,
        n       INTEGER NOT NULL,
        item_id TEXT NOT NULL,                 -- not a foreign key: the note may be deleted later
        quote   TEXT NOT NULL,
        PRIMARY KEY (ask_id, n)
    );
"#, r#"
    -- The answer listed sources but none could be found in the notes it named.
    ALTER TABLE asks ADD COLUMN unsupported INTEGER NOT NULL DEFAULT 0;
"#, r#"
    -- A follow-up joins the thread of the question it follows; a first question starts its own.
    ALTER TABLE asks ADD COLUMN thread_id TEXT;
    UPDATE asks SET thread_id = id;
    CREATE INDEX asks_thread ON asks(thread_id);
    -- What the agent found along the way (a JSON list of short lines), carried into follow-ups.
    ALTER TABLE asks ADD COLUMN found TEXT NOT NULL DEFAULT '[]';
"#, r#"
    -- For a partial answer: what ran out ('steps' or 'tokens').
    ALTER TABLE asks ADD COLUMN limit_hit TEXT;
    -- What AI calls were made, by day and purpose, for the usage page in Settings.
    CREATE TABLE ai_usage (
        day        TEXT NOT NULL,
        purpose    TEXT NOT NULL,
        calls      INTEGER NOT NULL DEFAULT 0,
        failed     INTEGER NOT NULL DEFAULT 0,
        limited    INTEGER NOT NULL DEFAULT 0,
        tokens_in  INTEGER NOT NULL DEFAULT 0,
        tokens_out INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (day, purpose)
    );
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
