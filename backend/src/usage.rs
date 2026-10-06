//! What the AI has been asked to do, by day and purpose, for the usage page in Settings: calls,
//! the ones that failed or were rate-limited, and tokens when the provider says how many.
//!
//! Every provider call passes through `llm::complete_json`, which reports here; the app installs
//! a sink that stores it. A call is tagged with the purpose set by a `purpose` guard on its thread.

use std::cell::Cell;
use std::sync::OnceLock;

use chrono::{Duration, Utc};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::Value;

use crate::db::chunks;
use crate::error::Result;
use crate::llm::LlmError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Purpose {
    /// Titles, summaries, concepts and links for notes.
    Notes,
    /// Describing pictures.
    Images,
    /// Naming topics.
    Topics,
    /// Answering questions.
    Ask,
    /// Trying a connection or a model.
    Check,
    #[default]
    Other,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Notes => "notes",
            Purpose::Images => "images",
            Purpose::Topics => "topics",
            Purpose::Ask => "ask",
            Purpose::Check => "check",
            Purpose::Other => "other",
        }
    }
}

thread_local! {
    static PURPOSE: Cell<Purpose> = const { Cell::new(Purpose::Other) };
}

/// Calls made on this thread until the guard is dropped are for `purpose`.
pub fn purpose(purpose: Purpose) -> Guard {
    Guard(PURPOSE.replace(purpose))
}

pub struct Guard(Purpose);

impl Drop for Guard {
    fn drop(&mut self) {
        PURPOSE.set(self.0);
    }
}

/// One finished provider call.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub purpose: Purpose,
    pub failed: bool,
    pub limited: bool,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

type Sink = Box<dyn Fn(&Call) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();

/// Where finished calls go (once, at startup).
pub fn install(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Called by the provider layer after each call.
pub(crate) fn note(result: &std::result::Result<Value, LlmError>, tokens: Option<(u64, u64)>) {
    let Some(sink) = SINK.get() else { return };
    let (tokens_in, tokens_out) = tokens.unwrap_or_default();
    sink(&Call {
        purpose: PURPOSE.get(),
        failed: matches!(result, Err(e) if !matches!(e, LlmError::RateLimited(_))),
        limited: matches!(result, Err(LlmError::RateLimited(_))),
        tokens_in,
        tokens_out,
    });
}

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

pub fn store(conn: &Connection, call: &Call) -> Result<()> {
    conn.execute(
        "INSERT INTO ai_usage (day, purpose, calls, failed, limited, tokens_in, tokens_out) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6)
         ON CONFLICT(day, purpose) DO UPDATE SET calls = calls + 1, failed = failed + ?3, limited = limited + ?4,
                                                  tokens_in = tokens_in + ?5, tokens_out = tokens_out + ?6",
        params![today(), call.purpose.as_str(), call.failed as i64, call.limited as i64, call.tokens_in as i64, call.tokens_out as i64],
    )?;
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub calls: u64,
    pub failed: u64,
    pub limited: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayRow {
    pub day: String,
    pub calls: u64,
    pub tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurposeRow {
    pub purpose: String,
    #[serde(flatten)]
    pub totals: Totals,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub today: Totals,
    /// The last seven days, oldest first (days with nothing are there with zeros).
    pub days: Vec<DayRow>,
    /// Today, by what the calls were for, the busiest first.
    pub by_purpose: Vec<PurposeRow>,
    /// Calls counted against the daily limit today (notes, pictures and topic names).
    pub counted_today: u32,
    /// Questions asked today (against the daily question limit).
    pub asks_today: u32,
}

pub fn report(conn: &Connection) -> Result<Report> {
    let today = today();
    let mut stmt = conn.prepare("SELECT purpose, calls, failed, limited, tokens_in, tokens_out FROM ai_usage WHERE day = ?1")?;
    let mut by_purpose: Vec<PurposeRow> = stmt
        .query_map([&today], |row| {
            Ok(PurposeRow {
                purpose: row.get(0)?,
                totals: Totals {
                    calls: row.get::<_, i64>(1)? as u64,
                    failed: row.get::<_, i64>(2)? as u64,
                    limited: row.get::<_, i64>(3)? as u64,
                    tokens_in: row.get::<_, i64>(4)? as u64,
                    tokens_out: row.get::<_, i64>(5)? as u64,
                },
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    by_purpose.sort_by(|a, b| b.totals.calls.cmp(&a.totals.calls).then_with(|| a.purpose.cmp(&b.purpose)));
    let mut total = Totals::default();
    for row in &by_purpose {
        total.calls += row.totals.calls;
        total.failed += row.totals.failed;
        total.limited += row.totals.limited;
        total.tokens_in += row.totals.tokens_in;
        total.tokens_out += row.totals.tokens_out;
    }

    let mut days = Vec::new();
    let mut stmt = conn.prepare("SELECT COALESCE(SUM(calls), 0), COALESCE(SUM(tokens_in + tokens_out), 0) FROM ai_usage WHERE day = ?1")?;
    for back in (0..7).rev() {
        let day = (Utc::now() - Duration::days(back)).format("%Y-%m-%d").to_string();
        let (calls, tokens): (i64, i64) = stmt.query_row([&day], |row| Ok((row.get(0)?, row.get(1)?)))?;
        days.push(DayRow { day, calls: calls as u64, tokens: tokens as u64 });
    }

    let counter = |prefix: &str| -> Result<u32> { Ok(chunks::get_meta(conn, &format!("{prefix}_{today}"))?.and_then(|v| v.parse().ok()).unwrap_or(0)) };
    Ok(Report { today: total, days, by_purpose, counted_today: counter("ai_calls")?, asks_today: counter("asks")? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_temp;

    fn call(purpose: Purpose, failed: bool, limited: bool, tokens: (u64, u64)) -> Call {
        Call { purpose, failed, limited, tokens_in: tokens.0, tokens_out: tokens.1 }
    }

    #[test]
    fn calls_add_up_by_purpose_and_the_report_orders_them() {
        let (_dir, db) = open_temp();
        db.with(|conn| {
            store(conn, &call(Purpose::Notes, false, false, (1000, 200)))?;
            store(conn, &call(Purpose::Notes, false, false, (800, 100)))?;
            store(conn, &call(Purpose::Notes, true, false, (0, 0)))?;
            store(conn, &call(Purpose::Ask, false, true, (0, 0)))?;
            store(conn, &call(Purpose::Ask, false, false, (3000, 400)))?;
            chunks::set_meta(conn, &format!("ai_calls_{}", today()), "42")?;
            chunks::set_meta(conn, &format!("asks_{}", today()), "3")?;
            let report = report(conn)?;
            assert_eq!(report.today, Totals { calls: 5, failed: 1, limited: 1, tokens_in: 4800, tokens_out: 700 });
            assert_eq!(report.by_purpose.iter().map(|p| (p.purpose.as_str(), p.totals.calls)).collect::<Vec<_>>(), [("notes", 3), ("ask", 2)]);
            assert_eq!((report.counted_today, report.asks_today), (42, 3));
            // Seven days, today last, the rest zero.
            assert_eq!(report.days.len(), 7);
            assert_eq!(report.days[6].calls, 5);
            assert_eq!(report.days[6].tokens, 5500);
            assert!(report.days[..6].iter().all(|d| d.calls == 0));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_call_is_tagged_with_the_purpose_of_its_thread_until_the_guard_drops() {
        assert_eq!(PURPOSE.get(), Purpose::Other);
        {
            let _ask = purpose(Purpose::Ask);
            assert_eq!(PURPOSE.get(), Purpose::Ask);
            {
                let _images = purpose(Purpose::Images);
                assert_eq!(PURPOSE.get(), Purpose::Images);
            }
            assert_eq!(PURPOSE.get(), Purpose::Ask);
        }
        assert_eq!(PURPOSE.get(), Purpose::Other);
    }

    #[test]
    fn an_empty_day_reports_zeros() {
        let (_dir, db) = open_temp();
        let report = db.with(|conn| report(conn)).unwrap();
        assert_eq!(report.today, Totals::default());
        assert!(report.by_purpose.is_empty());
        assert_eq!(report.days.iter().map(|d| d.calls).sum::<u64>(), 0);
    }
}
