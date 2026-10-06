//! Questions asked of the library, with their answers and the sources the answers cite. The
//! steps of a run are not kept, only what the user can reopen later.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::Result;

/// How a run ended (or that it still is going).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    #[default]
    Running,
    Done,
    /// Out of steps or tokens: the answer is the best so far.
    Partial,
    Stopped,
    /// A provider error ended it; it can be resumed while the app is open.
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Running => "running",
            State::Done => "done",
            State::Partial => "partial",
            State::Stopped => "stopped",
            State::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "done" => State::Done,
            "partial" => State::Partial,
            "stopped" => State::Stopped,
            "failed" => State::Failed,
            _ => State::Running,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub n: i64,
    pub item_id: String,
    pub quote: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub id: String,
    pub scope: Option<String>,
    pub question: String,
    pub answer: String,
    pub effort: String,
    pub state: State,
    /// It cited sources, and none held up.
    pub unsupported: bool,
    pub created_at: String,
    pub sources: Vec<Source>,
}

pub fn create(conn: &Connection, id: &str, scope: Option<&str>, question: &str, created_at: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO asks (id, scope, question, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, scope, question, created_at],
    )?;
    Ok(())
}

/// Records how a run ended, with its answer and the sources that survived checking.
pub fn finish(conn: &mut Connection, id: &str, answer: &str, effort: &str, state: State, unsupported: bool, sources: &[Source]) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE asks SET answer = ?2, effort = ?3, state = ?4, unsupported = ?5 WHERE id = ?1",
        params![id, answer, effort, state.as_str(), unsupported],
    )?;
    tx.execute("DELETE FROM ask_sources WHERE ask_id = ?1", [id])?;
    for source in sources {
        tx.execute(
            "INSERT INTO ask_sources (ask_id, n, item_id, quote) VALUES (?1, ?2, ?3, ?4)",
            params![id, source.n, source.item_id, source.quote],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn sources_of(conn: &Connection, id: &str) -> Result<Vec<Source>> {
    let mut stmt = conn.prepare("SELECT n, item_id, quote FROM ask_sources WHERE ask_id = ?1 ORDER BY n")?;
    let rows = stmt.query_map([id], |row| Ok(Source { n: row.get(0)?, item_id: row.get(1)?, quote: row.get(2)? }))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Ask> {
    let state: String = row.get(5)?;
    Ok(Ask {
        id: row.get(0)?,
        scope: row.get(1)?,
        question: row.get(2)?,
        answer: row.get(3)?,
        effort: row.get(4)?,
        state: State::parse(&state),
        unsupported: row.get(7)?,
        created_at: row.get(6)?,
        sources: Vec::new(),
    })
}

const COLUMNS: &str = "id, scope, question, answer, effort, state, created_at, unsupported";

pub fn get(conn: &Connection, id: &str) -> Result<Option<Ask>> {
    let ask = conn.query_row(&format!("SELECT {COLUMNS} FROM asks WHERE id = ?1"), [id], from_row).optional()?;
    match ask {
        Some(mut ask) => {
            ask.sources = sources_of(conn, id)?;
            Ok(Some(ask))
        }
        None => Ok(None),
    }
}

/// The most recent asks first.
pub fn recent(conn: &Connection, limit: usize) -> Result<Vec<Ask>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM asks ORDER BY created_at DESC, id DESC LIMIT ?1"))?;
    let mut asks: Vec<Ask> = stmt.query_map([limit as i64], from_row)?.collect::<rusqlite::Result<_>>()?;
    for ask in &mut asks {
        ask.sources = sources_of(conn, &ask.id)?;
    }
    Ok(asks)
}

pub fn set_state(conn: &Connection, id: &str, state: State) -> Result<()> {
    conn.execute("UPDATE asks SET state = ?2 WHERE id = ?1", params![id, state.as_str()])?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM asks WHERE id = ?1", [id])?;
    Ok(())
}

/// A run that was going when the app closed can't go on: it is marked failed.
pub fn fail_unfinished(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE asks SET state = 'failed' WHERE state = 'running'", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_temp;

    #[test]
    fn an_ask_keeps_its_answer_and_sources() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            create(conn, "a1", Some("topic1"), "What is borrowing?", "2026-10-06T10:00:00Z")?;
            assert_eq!(get(conn, "a1")?.unwrap().state, State::Running);
            let sources = vec![Source { n: 1, item_id: "i1".into(), quote: "one owner".into() }];
            finish(conn, "a1", "Each value has one owner [1].", "lookup", State::Done, false, &sources)?;
            let ask = get(conn, "a1")?.unwrap();
            assert_eq!(ask.answer, "Each value has one owner [1].");
            assert_eq!(ask.state, State::Done);
            assert_eq!(ask.sources, sources);
            assert_eq!(ask.scope.as_deref(), Some("topic1"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn recent_comes_newest_first_and_delete_takes_the_sources() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            create(conn, "a1", None, "old", "2026-10-06T09:00:00Z")?;
            create(conn, "a2", None, "new", "2026-10-06T10:00:00Z")?;
            let source = Source { n: 1, item_id: "i1".into(), quote: "q".into() };
            finish(conn, "a2", "x", "lookup", State::Done, false, &[source])?;
            let ids: Vec<_> = recent(conn, 10)?.into_iter().map(|a| a.id).collect();
            assert_eq!(ids, ["a2", "a1"]);
            delete(conn, "a2")?;
            let left: i64 = conn.query_row("SELECT COUNT(*) FROM ask_sources", [], |r| r.get(0))?;
            assert_eq!(left, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_run_left_going_is_marked_failed() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            create(conn, "a1", None, "q", "2026-10-06T09:00:00Z")?;
            create(conn, "a2", None, "q", "2026-10-06T09:01:00Z")?;
            finish(&mut *conn, "a2", "x", "lookup", State::Done, false, &[])?;
            fail_unfinished(conn)?;
            assert_eq!(get(conn, "a1")?.unwrap().state, State::Failed);
            assert_eq!(get(conn, "a2")?.unwrap().state, State::Done);
            Ok(())
        })
        .unwrap();
    }
}
