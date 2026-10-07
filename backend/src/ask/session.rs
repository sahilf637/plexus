//! Running questions: starting one (within the day's limit), going on with one that a provider
//! error stopped, stopping one, and keeping the result. A run lives here while it can still be
//! resumed; its steps are not stored, only the answer and the sources that held up.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{SecondsFormat, Utc};

use super::budget::Budget;
use super::cite;
use super::thread;
use super::tools::{Finder, LibraryTools, Memory, searchable};
use super::{Event, Model, Run};
use crate::db::asks::{self, Ask, State};
use crate::db::{Db, chunks, items};
use crate::error::{Error, Result};

struct Active {
    run: Run,
    memory: Memory,
    scope: Option<String>,
    budget: Budget,
}

/// A question to start.
pub struct Start<'a> {
    pub question: &'a str,
    /// A topic or note to keep the question inside.
    pub scope: Option<&'a str>,
    /// The conversation this follows on from (any question id in it, or its thread id).
    pub thread: Option<&'a str>,
    /// Most questions per day (0 for no limit).
    pub per_day: u32,
    pub budget: Budget,
}

/// The questions that are going, or stopped by an error and waiting to be resumed.
#[derive(Default)]
pub struct Asker {
    runs: Mutex<HashMap<String, Active>>,
    /// Kept apart from the runs so a run can be stopped while it is busy.
    stops: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Asker {
    /// Starts a question: counts it against the day's limit, looks up the notes to start from
    /// (for a follow-up, the notes the conversation cited and the ones its latest question and
    /// this one find together), and returns its id. `execute` does the work.
    pub fn begin(&self, db: &Db, finder: &dyn Finder, start: &Start) -> Result<String> {
        let question = start.question.trim();
        if question.is_empty() {
            return Err(Error::Invalid("Ask something first.".into()));
        }
        // The conversation so far, if this follows on from one.
        let prior = match start.thread {
            Some(thread) => {
                let prior = db.with(|conn| {
                    let thread_id = asks::get(conn, thread)?.map_or_else(|| thread.to_string(), |a| a.thread_id);
                    asks::thread(conn, &thread_id)
                })?;
                if prior.is_empty() {
                    return Err(Error::Invalid("That conversation is gone; start a new question.".into()));
                }
                prior
            }
            None => Vec::new(),
        };
        // An empty scope is the user saying "everything"; none at all follows the conversation.
        let scope = match start.scope {
            Some("") => None,
            Some(id) => Some(id.to_string()),
            None => prior.last().and_then(|a| a.scope.clone()),
        };
        db.with(|conn| {
            let key = format!("asks_{}", Utc::now().format("%Y-%m-%d"));
            let used: u32 = chunks::get_meta(conn, &key)?.and_then(|v| v.parse().ok()).unwrap_or(0);
            if start.per_day > 0 && used >= start.per_day {
                return Err(Error::DailyLimit);
            }
            chunks::set_meta(conn, &key, &(used + 1).to_string())
        })?;
        let id = ulid::Ulid::new().to_string();
        let thread_id = prior.first().map_or_else(|| id.clone(), |a| a.thread_id.clone());
        db.with(|conn| asks::create(conn, &id, &thread_id, scope.as_deref(), question, &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)))?;

        let carried = db.with(|conn| {
            let title_of = |id: &str| items::get(conn, id).ok().flatten().map(|item| item.title);
            Ok(thread::carry(&prior, &title_of, start.budget.thread_tokens))
        })?;
        let mut tools = LibraryTools::new(db, finder, scope.as_deref(), start.budget.clone());
        // A short follow-up ("and in detail?") is a poor search on its own: the question before it helps.
        let query = match &carried.last_question {
            Some(before) => format!("{} {question}", before.chars().take(200).collect::<String>()),
            None => question.to_string(),
        };
        // What the user pointed at comes first, then what the conversation cited, then what
        // search found for the question.
        let anchor = tools.anchor();
        let cited = tools.introduce(&carried.cited);
        let found = tools.candidates(&query);
        let mut ids = Vec::new();
        let mut lines: Vec<String> = Vec::new();
        let mut add = |cards: &str, from: Vec<String>| {
            for (card, id) in cards.lines().zip(from) {
                if !lines.iter().any(|line| line == card) {
                    lines.push(card.to_string());
                    ids.push(id);
                }
            }
        };
        if let Some(anchor) = &anchor {
            add(&anchor.cards, anchor.ids.clone());
        }
        add(&cited.cards, cited.ids);
        add(&found.cards, found.ids);
        let mut run = Run::new(question, scope.as_deref(), lines.join("\n"), ids, carried.text, start.budget.clone());
        if let Some(anchor) = anchor {
            run = run.anchored(anchor.line);
        }
        self.stops.lock().expect("asker lock").insert(id.clone(), Arc::new(AtomicBool::new(false)));
        self.runs.lock().expect("asker lock").insert(id.clone(), Active { run, memory: tools.into_memory(), scope, budget: start.budget.clone() });
        Ok(id)
    }

    /// Runs a question to its end. If the provider fails, the question stays here as it was, is
    /// recorded as failed, and `execute` can be called for it again.
    pub fn execute(&self, db: &Db, finder: &dyn Finder, model: &dyn Model, id: &str, on_event: &mut dyn FnMut(&Event)) -> Result<Ask> {
        let mut active = self.runs.lock().expect("asker lock").remove(id).ok_or_else(|| Error::Invalid("That question isn't running.".into()))?;
        let stop = self.stops.lock().expect("asker lock").get(id).cloned().unwrap_or_default();
        stop.store(false, Ordering::Relaxed);
        db.with(|conn| asks::set_state(conn, id, State::Running))?;

        let mut tools = LibraryTools::resume(db, finder, active.scope.as_deref(), active.budget.clone(), std::mem::take(&mut active.memory));
        let result = active.run.advance(model, &mut tools, &stop, on_event);
        active.memory = tools.into_memory();

        match result {
            Ok(outcome) => {
                let checked = db.with(|conn| {
                    let resolve = |name: &str| active.memory.resolve(name);
                    let seen: std::collections::HashSet<String> = active.run.shown.union(&active.run.read).cloned().collect();
                    Ok(cite::check(&outcome.text, &outcome.sources, &resolve, &seen, &|item| searchable(conn, item)))
                })?;
                // An answer given on the last turn that stands on checked notes is a finished answer;
                // only one with nothing to stand on is incomplete. Both can still be taken further.
                let ran_out = outcome.state == State::Partial;
                let state = if ran_out && !checked.sources.is_empty() && !checked.unsupported { State::Done } else { outcome.state };
                db.with(|conn| asks::finish(conn, id, &checked.text, outcome.effort.as_str(), state, checked.unsupported, &outcome.found, outcome.limit_hit, &checked.sources))?;
                if ran_out {
                    // Out of steps or tokens: kept, so "keep looking" can go on from here.
                    self.runs.lock().expect("asker lock").insert(id.to_string(), active);
                } else {
                    self.stops.lock().expect("asker lock").remove(id);
                }
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

    /// Gives a partial answer's run more steps to go on with (`execute` then continues it).
    pub fn extend(&self, id: &str) -> Result<()> {
        match self.runs.lock().expect("asker lock").get_mut(id) {
            Some(active) => {
                active.run.extend();
                Ok(())
            }
            None => Err(Error::Invalid("That question can't be continued any more; ask it again.".into())),
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

    /// Finds exactly these notes, whatever is asked.
    struct Listed(Vec<&'static str>);

    impl Finder for Listed {
        fn hybrid(&self, _q: &str, _l: usize) -> Result<Vec<String>> {
            Ok(self.0.iter().map(|s| s.to_string()).collect())
        }

        fn semantic(&self, q: &str, l: usize) -> Result<Vec<String>> {
            self.hybrid(q, l)
        }
    }

    struct Script(RefCell<VecDeque<std::result::Result<Value, LlmError>>>, RefCell<Vec<String>>);

    impl Model for Script {
        fn complete(&self, request: &Request) -> std::result::Result<Value, LlmError> {
            self.1.borrow_mut().push(request.user.to_string());
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

    fn start<'a>(question: &'a str, per_day: u32) -> Start<'a> {
        Start { question, scope: None, thread: None, per_day, budget: Budget::default() }
    }

    fn script(replies: Vec<std::result::Result<Value, LlmError>>) -> Script {
        Script(RefCell::new(replies.into()), RefCell::default())
    }

    #[test]
    fn an_answer_is_checked_and_kept_with_the_sources_that_hold_up() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, &start("Who owns a value?", 20)).unwrap();
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
    fn a_follow_up_keeps_the_scope_unless_the_user_widens_it() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let scoped = |question, thread, scope| Start { question, scope, thread, per_day: 20, budget: Budget::default() };
        let first = asker.begin(&db, &Fixed, &scoped("What is this?", None, Some("i1"))).unwrap();
        let scope_of = |id: &str| db.with(|conn| asks::get(conn, id)).unwrap().unwrap().scope;
        assert_eq!(scope_of(&first).as_deref(), Some("i1"));

        // Nothing said: the conversation's scope goes on.
        let same = asker.begin(&db, &Fixed, &scoped("And more?", Some(&first), None)).unwrap();
        assert_eq!(scope_of(&same).as_deref(), Some("i1"));
        // "Everything" said: the follow-up isn't limited any more.
        let wide = asker.begin(&db, &Fixed, &scoped("And elsewhere?", Some(&first), Some(""))).unwrap();
        assert_eq!(scope_of(&wide), None);
    }

    #[test]
    fn a_follow_up_carries_the_conversation_and_starts_from_the_notes_it_cited() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let first = asker.begin(&db, &Fixed, &start("Who owns a value?", 20)).unwrap();
        let model = script(vec![Ok({
            let mut reply = answer("One owner [1].", json!([{ "n": 1, "item": "n1", "quote": "has one owner" }]));
            reply["learned"] = json!("ownership is one owner per value");
            reply
        })]);
        let before = asker.execute(&db, &Fixed, &model, &first, &mut |_| {}).unwrap();
        assert_eq!(before.thread_id, first);

        // Search finds nothing for the follow-up; the note the answer cited is still on offer.
        let nothing = Listed(vec![]);
        let follow = Start { thread: Some(&first), ..start("and explain it in detail?", 20) };
        let second = asker.begin(&db, &nothing, &follow).unwrap();
        let model = script(vec![Ok(answer("In detail [1].", json!([{ "n": 1, "item": "n1", "quote": "has one owner" }])))]);
        let after = asker.execute(&db, &nothing, &model, &second, &mut |_| {}).unwrap();
        assert_eq!(after.thread_id, first);
        assert_eq!(after.sources[0].item_id, "i1");
        let prompt = model.1.borrow()[0].clone();
        assert!(prompt.starts_with("The conversation so far"));
        assert!(prompt.contains("Q: Who owns a value?\nA: One owner [1]."));
        assert!(prompt.contains("[1] “Ownership” — “has one owner”"));
        assert!(prompt.contains("n1 | Ownership"));
        assert!(prompt.contains("Question: and explain it in detail?"));
        let thread = db.with(|conn| asks::thread(conn, &first)).unwrap();
        assert_eq!(thread.len(), 2);
        assert_eq!(thread[0].found, Vec::<String>::new());
    }

    #[test]
    fn what_the_agent_found_is_saved_for_the_next_question() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, &start("q", 20)).unwrap();
        let model = script(vec![
            Ok({
                let mut reply = step("search");
                reply["query"] = json!("owner");
                reply
            }),
            Ok({
                let mut reply = answer("Yes [1].", json!([{ "n": 1, "item": "n1", "quote": "has one owner" }]));
                reply["learned"] = json!("each value has one owner");
                reply
            }),
        ]);
        asker.execute(&db, &Fixed, &model, &id, &mut |_| {}).unwrap();
        let saved = db.with(|conn| asks::get(conn, &id)).unwrap().unwrap();
        assert_eq!(saved.found, ["searched for \"owner\": each value has one owner"]);
    }

    #[test]
    fn a_follow_up_to_a_conversation_that_is_gone_is_refused() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let follow = Start { thread: Some("nothing"), ..start("and?", 20) };
        assert!(matches!(asker.begin(&db, &Fixed, &follow), Err(Error::Invalid(_))));
    }

    #[test]
    fn a_partial_answer_can_keep_looking_but_a_finished_one_cannot() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, &start("q", 20)).unwrap();
        let searching = |learned: &str| {
            let mut reply = step("search");
            reply["query"] = json!("owner");
            reply["learned"] = json!(learned);
            Ok(reply)
        };
        let mut replies = vec![searching("")];
        replies.extend((0..8).map(|_| searching("more")));
        replies.push(Ok(answer("A guess.", json!([]))));
        replies.push(searching(""));
        replies.push(Ok(answer("Found [1].", json!([{ "n": 1, "item": "n1", "quote": "has one owner" }]))));
        let model = script(replies);
        let first = asker.execute(&db, &Fixed, &model, &id, &mut |_| {}).unwrap();
        assert_eq!(first.state, State::Partial);
        assert_eq!(first.limit_hit.as_deref(), Some("steps"));
        assert!(asker.can_resume(&id));
        asker.extend(&id).unwrap();
        let second = asker.execute(&db, &Fixed, &model, &id, &mut |_| {}).unwrap();
        assert_eq!(second.state, State::Done);
        assert_eq!(second.limit_hit, None);
        assert!(asker.extend(&id).is_err());
    }

    #[test]
    fn a_last_turn_answer_resting_on_notes_is_finished_but_can_go_deeper() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, &start("q", 20)).unwrap();
        let searching = |learned: &str| {
            let mut reply = step("search");
            reply["query"] = json!("owner");
            reply["learned"] = json!(learned);
            Ok(reply)
        };
        let mut replies = vec![searching("")];
        replies.extend((0..7).map(|_| searching("more")));
        replies.push(Ok(answer("A full answer [1].", json!([{ "n": 1, "item": "n1", "quote": "has one owner" }]))));
        let ask = asker.execute(&db, &Fixed, &script(replies), &id, &mut |_| {}).unwrap();
        assert_eq!(ask.state, State::Done);
        assert_eq!(ask.limit_hit.as_deref(), Some("steps"));
        assert!(asker.extend(&id).is_ok());
    }

    #[test]
    fn made_up_support_marks_the_answer_unsupported() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, &start("q", 20)).unwrap();
        let model = script(vec![Ok(answer("Sure [1].", json!([{ "n": 1, "item": "n1", "quote": "a sentence nobody wrote" }])))]);
        let ask = asker.execute(&db, &Fixed, &model, &id, &mut |_| {}).unwrap();
        assert!(ask.unsupported);
        assert!(ask.sources.is_empty());
    }

    #[test]
    fn the_days_asks_are_counted_and_zero_means_no_limit() {
        let (_dir, db) = library();
        let asker = Asker::default();
        asker.begin(&db, &Fixed, &start("one", 2)).unwrap();
        asker.begin(&db, &Fixed, &start("two", 2)).unwrap();
        assert!(matches!(asker.begin(&db, &Fixed, &start("three", 2)), Err(Error::DailyLimit)));
        assert!(asker.begin(&db, &Fixed, &start("three", 0)).is_ok());
        assert!(matches!(asker.begin(&db, &Fixed, &start("  ", 0)), Err(Error::Invalid(_))));
    }

    #[test]
    fn a_provider_error_can_be_resumed_and_the_names_still_mean_the_same_notes() {
        let (_dir, db) = library();
        let asker = Asker::default();
        let id = asker.begin(&db, &Fixed, &start("q", 20)).unwrap();
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
        let id = asker.begin(&db, &Fixed, &start("q", 20)).unwrap();
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
