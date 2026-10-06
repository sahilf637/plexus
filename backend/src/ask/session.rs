//! Running questions: starting one (within the day's limit), going on with one that a provider
//! error stopped, stopping one, and keeping the result. A run lives here while it can still be
//! resumed; its steps are not stored, only the answer and the sources that held up.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{SecondsFormat, Utc};

use super::cite;
use super::tools::{Finder, LibraryTools, Memory, searchable};
use super::{Event, Model, Run};
use crate::db::asks::{self, Ask, State};
use crate::db::{Db, chunks};
use crate::error::{Error, Result};

struct Active {
    run: Run,
    memory: Memory,
    scope: Option<String>,
}

/// The questions that are going, or stopped by an error and waiting to be resumed.
#[derive(Default)]
pub struct Asker {
    runs: Mutex<HashMap<String, Active>>,
    /// Kept apart from the runs so a run can be stopped while it is busy.
    stops: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Asker {
    /// Starts a question: counts it against the day's limit (`per_day`, 0 for none), looks up the
    /// notes to start from, and returns its id. `execute` does the work.
    pub fn begin(&self, db: &Db, finder: &dyn Finder, question: &str, scope: Option<&str>, per_day: u32) -> Result<String> {
        let question = question.trim();
        if question.is_empty() {
            return Err(Error::Invalid("Ask something first.".into()));
        }
        db.with(|conn| {
            let key = format!("asks_{}", Utc::now().format("%Y-%m-%d"));
            let used: u32 = chunks::get_meta(conn, &key)?.and_then(|v| v.parse().ok()).unwrap_or(0);
            if per_day > 0 && used >= per_day {
                return Err(Error::DailyLimit);
            }
            chunks::set_meta(conn, &key, &(used + 1).to_string())
        })?;
        let id = ulid::Ulid::new().to_string();
        db.with(|conn| asks::create(conn, &id, scope, question, &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)))?;
        let mut tools = LibraryTools::new(db, finder, scope);
        let start = tools.candidates(question);
        let run = Run::new(question, scope, start.cards, start.ids);
        self.stops.lock().expect("asker lock").insert(id.clone(), Arc::new(AtomicBool::new(false)));
        self.runs.lock().expect("asker lock").insert(id.clone(), Active { run, memory: tools.into_memory(), scope: scope.map(str::to_string) });
        Ok(id)
    }

    /// Runs a question to its end. If the provider fails, the question stays here as it was, is
    /// recorded as failed, and `execute` can be called for it again.
    pub fn execute(&self, db: &Db, finder: &dyn Finder, model: &dyn Model, id: &str, on_event: &mut dyn FnMut(&Event)) -> Result<Ask> {
        let mut active = self.runs.lock().expect("asker lock").remove(id).ok_or_else(|| Error::Invalid("That question isn't running.".into()))?;
        let stop = self.stops.lock().expect("asker lock").get(id).cloned().unwrap_or_default();
        stop.store(false, Ordering::Relaxed);
        db.with(|conn| asks::set_state(conn, id, State::Running))?;

        let mut tools = LibraryTools::resume(db, finder, active.scope.as_deref(), std::mem::take(&mut active.memory));
        let result = active.run.advance(model, &mut tools, &stop, on_event);
        active.memory = tools.into_memory();

        match result {
            Ok(outcome) => {
                let checked = db.with(|conn| {
                    let resolve = |name: &str| active.memory.resolve(name);
                    let seen: std::collections::HashSet<String> = active.run.shown.union(&active.run.read).cloned().collect();
                    Ok(cite::check(&outcome.text, &outcome.sources, &resolve, &seen, &|item| searchable(conn, item)))
                })?;
                db.with(|conn| asks::finish(conn, id, &checked.text, outcome.effort.as_str(), outcome.state, checked.unsupported, &checked.sources))?;
                self.stops.lock().expect("asker lock").remove(id);
                Ok(db.with(|conn| asks::get(conn, id))?.expect("the ask was just saved"))
            }
            Err(error) => {
                db.with(|conn| asks::set_state(conn, id, State::Failed))?;
                self.runs.lock().expect("asker lock").insert(id.to_string(), active);
                Err(error)
            }
        }
    }

    /// Asks a running question to stop; it ends with what it has found.
    pub fn stop(&self, id: &str) {
        if let Some(flag) = self.stops.lock().expect("asker lock").get(id) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Whether a failed question can still be resumed.
    pub fn can_resume(&self, id: &str) -> bool {
        self.runs.lock().expect("asker lock").contains_key(id)
    }

    /// Drops a question, e.g. when it is deleted from the history.
    pub fn forget(&self, id: &str) {
        self.runs.lock().expect("asker lock").remove(id);
        self.stops.lock().expect("asker lock").remove(id);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use serde_json::{Value, json};

    use super::*;
    use crate::ask::tools::Finder;
    use crate::capture::detect::Kind;
    use crate::db::items::{self, Item, Status};
    use crate::db::open_temp;
    use crate::llm::{LlmError, Request};

    struct Fixed;

    impl Finder for Fixed {
        fn hybrid(&self, _q: &str, _l: usize) -> Result<Vec<String>> {
            Ok(vec!["i1".into(), "i2".into()])
        }

        fn semantic(&self, q: &str, l: usize) -> Result<Vec<String>> {
            self.hybrid(q, l)
        }
    }

    struct Script(RefCell<VecDeque<std::result::Result<Value, LlmError>>>);

    impl Model for Script {
        fn complete(&self, _request: &Request) -> std::result::Result<Value, LlmError> {
            self.0.borrow_mut().pop_front().expect("the script ran out of replies")
        }
    }

    fn step(tool: &str) -> Value {
        json!({
            "effort": "explain", "learned": "", "tool": tool, "query": "", "mode": "hybrid", "scope": "", "item": "",
            "offset": 0, "limit": 0, "depth": 0, "topic": "", "text": "", "sources": []
        })
    }

    fn answer(text: &str, sources: Value) -> Value {
        let mut value = step("answer");
        value["text"] = json!(text);
        value["sources"] = sources;
        value
    }

    fn library() -> (tempfile::TempDir, Db) {
        let (dir, db) = open_temp();
        db.with(|conn| {
            for (id, title, body) in [("i1", "Ownership", "Each value has one owner."), ("i2", "Borrowing", "A borrow lends a value out.")] {
                let item = Item {
                    id: id.into(),
                    path: format!("{id}.md"),
                    kind: Kind::Text,
                    title: title.into(),
                    source_url: None,
                    language: None,
                    captured_at: "2026-10-06T00:00:00Z".into(),
                    body_hash: id.into(),
                    asset_path: None,
                    asset_hash: None,
                    mime: None,
                    size: None,
                    extracted: None,
                    summary: None,
                    file_mtime: 0,
                    file_size: 0,
                    status: Status::Linked,
                };
                items::upsert(conn, &item, body)?;
            }
            Ok(())
        })
        .unwrap();
        (dir, db)
    }

    fn script(replies: Vec<std::result::Result<Value, LlmError>>) -> Script {
        Script(RefCell::new(replies.into()))
    }

    #[test]
    fn an_answer_is_checked_and_kept_with_the_sources_that_hold_up() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, "Who owns a value?", None, 20).unwrap();
        let model = script(vec![Ok(answer(
            "One owner [1], and maybe more [2].",
            json!([{ "n": 1, "item": "n1", "quote": "has one owner" }, { "n": 2, "item": "n2", "quote": "never written anywhere" }]),
        ))]);
        let ask = asker.execute(&db, &Fixed, &model, &id, &mut |_| {}).unwrap();
        assert_eq!(ask.state, State::Done);
        assert_eq!(ask.answer, "One owner [1], and maybe more.");
        assert_eq!(ask.sources.len(), 1);
        assert_eq!(ask.sources[0].item_id, "i1");
        assert!(!ask.unsupported);
        assert!(!asker.can_resume(&id));
        assert_eq!(db.with(|conn| asks::get(conn, &id)).unwrap().unwrap().answer, ask.answer);
    }

    #[test]
    fn made_up_support_marks_the_answer_unsupported() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, "q", None, 20).unwrap();
        let model = script(vec![Ok(answer("Sure [1].", json!([{ "n": 1, "item": "n1", "quote": "a sentence nobody wrote" }])))]);
        let ask = asker.execute(&db, &Fixed, &model, &id, &mut |_| {}).unwrap();
        assert!(ask.unsupported);
        assert!(ask.sources.is_empty());
    }

    #[test]
    fn the_days_asks_are_counted_and_zero_means_no_limit() {
        let (_dir, db) = library();
        let asker = Asker::default();
        asker.begin(&db, &Fixed, "one", None, 2).unwrap();
        asker.begin(&db, &Fixed, "two", None, 2).unwrap();
        assert!(matches!(asker.begin(&db, &Fixed, "three", None, 2), Err(Error::DailyLimit)));
        assert!(asker.begin(&db, &Fixed, "three", None, 0).is_ok());
        assert!(matches!(asker.begin(&db, &Fixed, "  ", None, 0), Err(Error::Invalid(_))));
    }

    #[test]
    fn a_provider_error_can_be_resumed_and_the_names_still_mean_the_same_notes() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, "q", None, 20).unwrap();
        let model = script(vec![
            Ok({
                let mut value = step("read");
                value["item"] = json!("n2");
                value
            }),
            Err(LlmError::Unavailable("offline".into())),
            Ok(answer("A borrow lends [1].", json!([{ "n": 1, "item": "n2", "quote": "lends a value out" }]))),
        ]);
        assert!(asker.execute(&db, &Fixed, &model, &id, &mut |_| {}).is_err());
        assert_eq!(db.with(|conn| asks::get(conn, &id)).unwrap().unwrap().state, State::Failed);
        assert!(asker.can_resume(&id));
        let ask = asker.execute(&db, &Fixed, &model, &id, &mut |_| {}).unwrap();
        assert_eq!(ask.state, State::Done);
        assert_eq!(ask.sources[0].item_id, "i2");
    }

    #[test]
    fn stopping_keeps_what_was_found_and_ends_the_question() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, "q", None, 20).unwrap();
        let model = script(vec![Ok({
            let mut value = step("search");
            value["query"] = json!("borrow");
            value
        })]);
        let stopper = &asker;
        let id2 = id.clone();
        let ask = asker.execute(&db, &Fixed, &model, &id, &mut |_| stopper.stop(&id2)).unwrap();
        assert_eq!(ask.state, State::Stopped);
        assert!(!asker.can_resume(&id));
    }
}
