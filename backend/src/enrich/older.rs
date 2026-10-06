//! The slow pass over older notes (saved before AI was switched on), when the user has asked for
//! it in Settings. It hands the enrichment queue one note at a time, a few minutes apart, only
//! when the queue is otherwise idle, at most a set number a day, and never past a share of the
//! day's call limit, so something saved this afternoon is never refused because of it.
//! Notes are taken most-linked first: those shape the topics.

use std::time::{Duration, Instant};

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use super::{Lane, SINCE_KEY, VERSION};
use crate::db::chunks;
use crate::error::Result;
use crate::settings::AiSettings;
use crate::state::AppState;

/// Time between one older note and the next.
const PACE: Duration = Duration::from_secs(180);
/// Wait before the first look after the app starts, and between looks.
const FIRST_WAIT: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(20);
/// Older notes stop once this share of the day's call limit is used.
const RESERVE: f64 = 0.8;

/// How the pass is going, for Settings.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// Older notes that have been looked at (improved, or found to have nothing to send).
    pub done: usize,
    /// All older notes.
    pub total: usize,
    /// Improved today.
    pub today: u32,
}

fn today_key() -> String {
    format!("ai_older_{}", Utc::now().format("%Y-%m-%d"))
}

fn calls_today(conn: &Connection) -> Result<u32> {
    let key = format!("ai_calls_{}", Utc::now().format("%Y-%m-%d"));
    Ok(chunks::get_meta(conn, &key)?.and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// The moment AI was switched on; notes saved before it are the older ones. With no record
/// every note counts as older.
fn since(conn: &Connection) -> Result<String> {
    Ok(chunks::get_meta(conn, SINCE_KEY)?.unwrap_or_else(|| "9999".to_string()))
}

pub fn progress(conn: &Connection) -> Result<Progress> {
    let since = since(conn)?;
    let total: usize = conn.query_row(
        "SELECT COUNT(*) FROM items WHERE status = 'linked' AND captured_at < ?1",
        [&since],
        |row| row.get::<_, i64>(0),
    )? as usize;
    let waiting: usize = conn.query_row(
        "SELECT COUNT(*) FROM items i LEFT JOIN enrichment e ON e.item_id = i.id
         WHERE i.status = 'linked' AND i.captured_at < ?1 AND (e.version IS NULL OR e.version < ?2)",
        params![since, VERSION],
        |row| row.get::<_, i64>(0),
    )? as usize;
    let today = chunks::get_meta(conn, &today_key())?.and_then(|v| v.parse().ok()).unwrap_or(0);
    Ok(Progress { done: total.saturating_sub(waiting), total, today })
}

/// The next older note to improve, if the settings, today's allowance and the call limit allow
/// one (counting it against today's allowance).
pub fn pick(conn: &Connection, ai: &AiSettings) -> Result<Option<String>> {
    if !ai.enabled || !ai.improve_old || ai.active_connection().is_none() {
        return Ok(None);
    }
    let today: u32 = chunks::get_meta(conn, &today_key())?.and_then(|v| v.parse().ok()).unwrap_or(0);
    if ai.improve_per_day > 0 && today >= ai.improve_per_day {
        return Ok(None);
    }
    if ai.daily_limit > 0 && f64::from(calls_today(conn)?) >= f64::from(ai.daily_limit) * RESERVE {
        return Ok(None);
    }
    let found = conn
        .query_row(
            "SELECT i.id FROM items i LEFT JOIN enrichment e ON e.item_id = i.id
             WHERE i.status = 'linked' AND i.captured_at < ?1 AND (e.version IS NULL OR e.version < ?2)
             ORDER BY (SELECT COUNT(*) FROM edges x WHERE (x.src = i.id OR x.dst = i.id) AND x.status != 'rejected') DESC,
                      i.captured_at DESC, i.id
             LIMIT 1",
            params![since(conn)?, VERSION],
            |row| row.get::<_, String>(0),
        )
        .ok();
    if found.is_some() {
        chunks::set_meta(conn, &today_key(), &(today + 1).to_string())?;
    }
    Ok(found)
}

/// Starts the thread that feeds older notes to the queue.
pub fn run(app: &AppHandle) {
    let app = app.clone();
    let worker = move || {
        std::thread::sleep(FIRST_WAIT);
        let state = app.state::<AppState>();
        let mut last: Option<Instant> = None;
        loop {
            std::thread::sleep(CHECK_EVERY);
            if last.is_some_and(|at| at.elapsed() < PACE) || !state.enricher.is_idle() {
                continue;
            }
            let ai = state.settings.get().ai;
            match state.db.with(|conn| pick(conn, &ai)) {
                Ok(Some(id)) => {
                    tracing::debug!(id, "improving an older note");
                    state.enricher.enqueue(&id, Lane::Backfill);
                    last = Some(Instant::now());
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("couldn't pick an older note: {e}"),
            }
        }
    };
    if let Err(e) = std::thread::Builder::new().name("enrich-older".into()).spawn(worker) {
        tracing::error!("couldn't start the pass over older notes: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::db::concepts::{Enriched, record_enriched};
    use crate::db::items::{self, Status, sample};
    use crate::settings::AiConnection;

    fn setup(db: &Db, n: usize) {
        db.with(|conn| {
            for i in 0..n {
                let id = format!("n{i}");
                let mut item = sample(&id, &format!("items/{id}.md"), &id);
                item.captured_at = format!("2026-01-{:02}T00:00:00Z", i + 1);
                item.status = Status::Linked;
                items::upsert(conn, &item, "")?;
                items::set_status(conn, &id, Status::Linked, 1)?;
            }
            chunks::set_meta(conn, SINCE_KEY, "2026-06-01T00:00:00Z")
        })
        .unwrap();
    }

    fn ai(per_day: u32) -> AiSettings {
        AiSettings {
            enabled: true,
            improve_old: true,
            improve_per_day: per_day,
            connections: vec![AiConnection { id: "c".into(), ..Default::default() }],
            active: "c".into(),
            daily_limit: 100,
            ..Default::default()
        }
    }

    #[test]
    fn picks_the_most_linked_first_and_stops_at_the_daily_number() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        setup(&db, 5);
        db.with(|conn| {
            // n1 has two links, n3 one.
            for (a, b) in [("n1", "n2"), ("n1", "n4"), ("n3", "n0")] {
                conn.execute(
                    "INSERT INTO edges (src, dst, kind, weight, tier, provenance, status, pipeline_version, created_at)
                     VALUES (?1, ?2, 'similar', 0.8, 'strong', 'embedding', 'proposed', 1, 't')",
                    [a, b],
                )?;
            }
            let settings = ai(2);
            assert_eq!(pick(conn, &settings)?.as_deref(), Some("n1"));
            // Done with it: the next most linked is next.
            record_enriched(conn, "n1", &Enriched { content_hash: "h".into(), version: VERSION, title: None })?;
            let second = pick(conn, &settings)?;
            assert!(second.is_some());
            // Two today: that's the day's number.
            assert_eq!(pick(conn, &settings)?, None);
            // 0 means no per-day number.
            assert!(pick(conn, &ai(0))?.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn leaves_headroom_and_respects_the_switch() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        setup(&db, 3);
        db.with(|conn| {
            let mut settings = ai(10);
            settings.improve_old = false;
            assert_eq!(pick(conn, &settings)?, None, "off unless switched on");
            settings.improve_old = true;
            // 80 of 100 calls used today: the rest is kept for new notes.
            let key = format!("ai_calls_{}", Utc::now().format("%Y-%m-%d"));
            chunks::set_meta(conn, &key, "80")?;
            assert_eq!(pick(conn, &settings)?, None);
            chunks::set_meta(conn, &key, "79")?;
            assert!(pick(conn, &settings)?.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn progress_counts_older_notes_only() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        setup(&db, 4);
        db.with(|conn| {
            // One note saved after AI was switched on isn't an older one.
            let mut newer = sample("new", "items/new.md", "new");
            newer.captured_at = "2026-08-01T00:00:00Z".into();
            newer.status = Status::Linked;
            items::upsert(conn, &newer, "")?;
            items::set_status(conn, "new", Status::Linked, 1)?;
            assert_eq!(progress(conn)?, Progress { done: 0, total: 4, today: 0 });
            record_enriched(conn, "n0", &Enriched { content_hash: "skipped".into(), version: VERSION, title: None })?;
            record_enriched(conn, "n1", &Enriched { content_hash: "h".into(), version: VERSION, title: None })?;
            assert_eq!(progress(conn)?.done, 2);
            Ok(())
        })
        .unwrap();
    }
}
