//! AI enrichment ("T2"): after an item is saved and linked locally, a connected provider adds a
//! title, a one-line summary, concepts and typed links with reasons. It runs on its own thread,
//! one item at a time, and never blocks capture or local linking.
//!
//! New items go through the `New` lane; anything caught up later (items saved while the provider
//! was unreachable, and in future a deeper pass over the whole library) uses `Backfill`, which
//! only runs when `New` is empty. A bad key pauses everything until it's fixed; a run of network
//! failures pauses for a few minutes; a daily limit pauses until tomorrow.
//!
//! Only new items are enriched: an item older than the moment AI was switched on is left alone
//! unless it was enriched before.

pub mod apply;
pub mod context;
pub mod vision;

use std::collections::{HashSet, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::capture::secrets::find_secret;
use crate::db::concepts::{self, Concept};
use crate::db::items::{self, Item, Status};
use crate::db::{chunks, graph};
use crate::error::{Error, Result};
use crate::library::text_hash;
use crate::llm::{self, Config, LlmError, Request, keys};
use crate::pipeline::own_text;
use crate::settings::{AiConnection, AiSettings};
use crate::state::AppState;
use apply::Answer;

/// Bumped when the prompt or the answer's shape changes; items enriched under an older version
/// are done again when they next come up.
pub const VERSION: i64 = 1;
/// `meta` key: when AI was switched on (RFC 3339). Items saved before it aren't enriched.
const SINCE_KEY: &str = "ai_since";
/// Consecutive network failures before pausing.
const MAX_FAILURES: u32 = 3;
const NETWORK_PAUSE: Duration = Duration::from_secs(300);
const RATE_LIMIT_WAIT: Duration = Duration::from_secs(30);
const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(120);
const MAX_ANSWER_TOKENS: u32 = 700;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// Items just saved.
    New,
    /// Catching up: runs only when `New` is empty.
    Backfill,
}

struct Pause {
    message: String,
    /// `None`: until something changes (a new key, other settings).
    until: Option<Instant>,
}

#[derive(Default)]
struct Queue {
    new: VecDeque<String>,
    backfill: VecDeque<String>,
    /// Everything in either lane, so an item is queued once.
    queued: HashSet<String>,
    pause: Option<Pause>,
    busy: bool,
    failures: u32,
}

#[derive(Default)]
pub struct Enricher {
    queue: Mutex<Queue>,
    wake: Condvar,
}

/// Sent to the windows whenever the queue changes.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatus {
    /// "off", "idle", "working" or "paused".
    pub state: &'static str,
    /// Items waiting, including the one being read.
    pub pending: usize,
    /// Why it is paused.
    pub message: Option<String>,
}

impl Enricher {
    pub fn enqueue(&self, id: &str, lane: Lane) {
        let mut queue = self.queue.lock().expect("enrich queue lock poisoned");
        if !queue.queued.insert(id.to_string()) {
            return;
        }
        match lane {
            Lane::New => queue.new.push_back(id.to_string()),
            Lane::Backfill => queue.backfill.push_back(id.to_string()),
        }
        self.wake.notify_all();
    }

    /// Clears a pause (the key or settings changed, or the user asked to try again).
    pub fn resume(&self) {
        let mut queue = self.queue.lock().expect("enrich queue lock poisoned");
        queue.pause = None;
        queue.failures = 0;
        self.wake.notify_all();
    }

    fn pause(&self, message: String, until: Option<Instant>) {
        self.queue.lock().expect("enrich queue lock poisoned").pause = Some(Pause { message, until });
    }

    /// Forgets everything waiting (AI was switched off).
    pub fn clear(&self) {
        let mut queue = self.queue.lock().expect("enrich queue lock poisoned");
        queue.new.clear();
        queue.backfill.clear();
        queue.queued.clear();
    }

    /// Blocks until there is an item and the queue isn't paused.
    fn next(&self) -> String {
        let mut queue = self.queue.lock().expect("enrich queue lock poisoned");
        loop {
            if let Some(pause) = &queue.pause {
                match pause.until {
                    Some(until) if until <= Instant::now() => {
                        queue.pause = None;
                        queue.failures = 0;
                    }
                    Some(until) => {
                        queue = self.wake.wait_timeout(queue, until - Instant::now()).expect("enrich queue lock poisoned").0;
                        continue;
                    }
                    None => {
                        queue = self.wake.wait(queue).expect("enrich queue lock poisoned");
                        continue;
                    }
                }
            }
            if let Some(id) = queue.new.pop_front().or_else(|| queue.backfill.pop_front()) {
                queue.busy = true;
                return id;
            }
            queue = self.wake.wait(queue).expect("enrich queue lock poisoned");
        }
    }

    /// Done with the item `next` returned.
    fn done(&self, id: &str, requeue: Option<Lane>) {
        let mut queue = self.queue.lock().expect("enrich queue lock poisoned");
        queue.busy = false;
        queue.queued.remove(id);
        drop(queue);
        if let Some(lane) = requeue {
            self.enqueue(id, lane);
        }
    }

    fn failed(&self) -> u32 {
        let mut queue = self.queue.lock().expect("enrich queue lock poisoned");
        queue.failures += 1;
        queue.failures
    }

    fn succeeded(&self) {
        self.queue.lock().expect("enrich queue lock poisoned").failures = 0;
    }

    pub fn status(&self, enabled: bool) -> QueueStatus {
        let queue = self.queue.lock().expect("enrich queue lock poisoned");
        let pending = queue.new.len() + queue.backfill.len() + usize::from(queue.busy);
        match (&queue.pause, enabled) {
            (_, false) => QueueStatus { state: "off", pending: 0, message: None },
            (Some(pause), _) => QueueStatus { state: "paused", pending, message: Some(pause.message.clone()) },
            (None, _) if queue.busy => QueueStatus { state: "working", pending, message: None },
            (None, _) => QueueStatus { state: "idle", pending, message: None },
        }
    }
}

/// Starts the worker thread.
pub fn run(app: &AppHandle) {
    let app = app.clone();
    let worker = move || {
        let state = app.state::<AppState>();
        loop {
            let id = state.enricher.next();
            emit_status(&app);
            let outcome = process(&app, &state, &id);
            let requeue = handle(&app, &state, &id, outcome);
            state.enricher.done(&id, requeue);
            emit_status(&app);
        }
    };
    if let Err(e) = std::thread::Builder::new().name("enrich".into()).spawn(worker) {
        tracing::error!("couldn't start the AI worker: {e}");
    }
}

/// Decides what a failure means for the queue. Returns the lane to put the item back in, if any.
fn handle(app: &AppHandle, state: &AppState, id: &str, outcome: Result<()>) -> Option<Lane> {
    let error = match outcome {
        Ok(()) => {
            state.enricher.succeeded();
            return None;
        }
        Err(e) => e,
    };
    tracing::warn!(id, "AI enrichment: {error}");
    match error {
        Error::Llm(LlmError::Auth(message)) => {
            state.enricher.pause(format!("The AI provider didn't accept the key ({message}). Check it in Settings."), None);
            Some(Lane::Backfill)
        }
        Error::Keychain(e) => {
            state.enricher.pause(format!("Couldn't read the AI key from your system keychain ({e})."), None);
            Some(Lane::Backfill)
        }
        Error::Llm(LlmError::RateLimited(wait)) => {
            let wait = wait.unwrap_or(RATE_LIMIT_WAIT).min(MAX_RATE_LIMIT_WAIT);
            if state.enricher.failed() >= MAX_FAILURES * 2 {
                state.enricher.pause("The AI provider keeps limiting requests; trying again in a few minutes.".into(), Some(Instant::now() + NETWORK_PAUSE));
            } else {
                state.enricher.pause("The AI provider is limiting requests; waiting a moment.".into(), Some(Instant::now() + wait));
            }
            emit_status(app);
            Some(Lane::New)
        }
        Error::Llm(LlmError::Unavailable(message)) => {
            if state.enricher.failed() >= MAX_FAILURES {
                state.enricher.pause(
                    format!("Couldn't reach the AI provider ({message}). Trying again in a few minutes."),
                    Some(Instant::now() + NETWORK_PAUSE),
                );
            }
            Some(Lane::Backfill)
        }
        Error::DailyLimit => {
            let until = Instant::now() + Duration::from_secs(seconds_to_midnight());
            state.enricher.pause("Reached today's AI limit. It continues tomorrow, or raise the limit in Settings.".into(), Some(until));
            Some(Lane::Backfill)
        }
        // This item's answer was unusable; the others may be fine.
        _ => None,
    }
}

fn emit_status(app: &AppHandle) {
    let state = app.state::<AppState>();
    let _ = app.emit("enrich-status", state.enricher.status(state.settings.get().ai.enabled));
}

// --- one item ----------------------------------------------------------------------------

/// Sent when an item's concepts changed.
#[derive(Clone, Debug, Serialize)]
struct ItemConcepts {
    id: String,
    concepts: Vec<Concept>,
}

/// Sent when AI links changed: every link touching these items, now.
#[derive(Clone, Debug, Serialize)]
struct LinksChanged {
    ids: Vec<String>,
    edges: Vec<graph::Edge>,
}

fn process(app: &AppHandle, state: &AppState, id: &str) -> Result<()> {
    let ai = state.settings.get().ai;
    if !ai.enabled {
        return Ok(());
    }
    let Some(item) = state.db.with(|conn| items::get(conn, id))? else { return Ok(()) };
    // Waiting to be linked: it comes back here once it is.
    if item.status == Status::Pending || item.status == Status::Failed {
        return Ok(());
    }
    if !eligible(state, &item)? {
        return Ok(());
    }
    let note = state.library.read_note(&state.library.absolute(&item.path))?;
    let text = own_text(&note.body);
    // Secrets never leave the machine.
    if find_secret(&text).is_some() || find_secret(&item.title).is_some() {
        tracing::debug!(id, "left out of AI: looks like a secret");
        return Ok(());
    }
    let config = config_for(&ai)?;

    // A picture is described first; once that's in its note it is linked again and comes back.
    if ai.active_connection().is_some_and(|c| c.vision) && vision::needs_description(&item) && describe_image(app, state, &config, &ai, &item)? {
        return Ok(());
    }
    if text.is_empty() {
        return Ok(()); // Nothing to say about it.
    }

    let hash = text_hash(&format!("{}\n{text}", item.kind.as_str()));
    let cached = state.db.with(|conn| concepts::enriched(conn, id))?;
    if cached.as_ref().is_some_and(|c| c.content_hash == hash && c.version == VERSION) {
        return Ok(()); // This content was already sent.
    }

    let context = context::build(&state.db, &state.pipeline.embedder, &item, &text)?;
    let schema = context::schema();
    let request = Request { system: context::SYSTEM, user: &context.user, images: &[], schema: &schema, max_tokens: MAX_ANSWER_TOKENS };
    let reply = call(state, &ai, &config, &request)?;
    let answer: Answer = serde_json::from_value(reply).map_err(|e| LlmError::Bad(format!("the answer didn't fit: {e}")))?;

    let applied = apply::apply(&state.library, &state.db, &state.pipeline.embedder, &item, &context, answer, hash)?;
    tracing::debug!(id, title = applied.item.title, retitled = applied.title_changed, concepts = applied.concepts.len(), "item enriched");
    announce(app, state, &applied)
}

/// Whether to enrich the item: it was saved since AI was switched on, or was enriched before
/// (its content changed).
fn eligible(state: &AppState, item: &Item) -> Result<bool> {
    state.db.with(|conn| {
        if concepts::enriched(conn, &item.id)?.is_some() {
            return Ok(true);
        }
        Ok(chunks::get_meta(conn, SINCE_KEY)?.is_some_and(|since| item.captured_at.as_str() >= since.as_str()))
    })
}

/// Describes the image into its note. `false` when it couldn't be read as a picture, in which
/// case it is treated like any other item.
fn describe_image(app: &AppHandle, state: &AppState, config: &Config, ai: &AiSettings, item: &Item) -> Result<bool> {
    spend(state, ai)?;
    let Some(description) = vision::describe(config, &state.library, item)? else { return Ok(false) };
    let updated = vision::write_description(&state.library, &state.db, item, &description)?;
    let _ = app.emit("item-updated", &updated);
    // Linked again by what it shows; that comes back here for the rest.
    state.pipeline.enqueue(&item.id);
    Ok(true)
}

/// One provider call, counted against the daily limit.
fn call(state: &AppState, ai: &AiSettings, config: &Config, request: &Request) -> Result<serde_json::Value> {
    spend(state, ai)?;
    Ok(llm::complete_json(config, request)?)
}

fn spend(state: &AppState, ai: &AiSettings) -> Result<()> {
    state.db.with(|conn| {
        let key = format!("ai_calls_{}", Utc::now().format("%Y-%m-%d"));
        let used: u32 = chunks::get_meta(conn, &key)?.and_then(|v| v.parse().ok()).unwrap_or(0);
        if ai.daily_limit > 0 && used >= ai.daily_limit {
            return Err(Error::DailyLimit);
        }
        chunks::set_meta(conn, &key, &(used + 1).to_string())
    })
}

fn seconds_to_midnight() -> u64 {
    86_400 - (Utc::now().timestamp().rem_euclid(86_400)) as u64
}

fn announce(app: &AppHandle, state: &AppState, applied: &apply::Applied) -> Result<()> {
    let _ = app.emit("item-updated", &applied.item);
    let _ = app.emit("item-concepts", ItemConcepts { id: applied.item.id.clone(), concepts: applied.concepts.clone() });
    let edges = state.db.with(|conn| {
        let mut all = Vec::new();
        for id in &applied.linked {
            all.extend(graph::edges_of(conn, id)?);
        }
        let mut seen = HashSet::new();
        all.retain(|e| seen.insert((e.src.clone(), e.dst.clone(), e.kind.clone())));
        Ok(all)
    })?;
    let _ = app.emit("links-changed", LinksChanged { ids: applied.linked.clone(), edges });
    Ok(())
}

// --- settings, keys, catching up -----------------------------------------------------------

/// What a call needs for the connection in use: its settings plus its key from the keychain.
pub fn config_for(ai: &AiSettings) -> Result<Config> {
    let connection = ai.active_connection().ok_or_else(|| LlmError::Auth("no AI connection chosen".into()))?;
    connection_config(connection, None)
}

/// The same for any connection, e.g. one being added whose key isn't saved yet: `key` stands in
/// for the keychain's when given.
pub fn connection_config(connection: &AiConnection, key: Option<&str>) -> Result<Config> {
    let api_key = match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => Some(key.to_string()),
        None => keys::get(&connection.id)?,
    };
    if api_key.is_none() && connection.kind != llm::ProviderKind::OpenAi {
        return Err(LlmError::Auth("no key set".into()).into());
    }
    Ok(Config { kind: connection.kind, base_url: connection.base_url.clone(), model: connection.model.clone(), api_key })
}

/// Called when the AI settings change: remembers when AI was switched on, wakes a paused queue,
/// and catches up on items saved while it couldn't run.
pub fn settings_changed(app: &AppHandle, before: &AiSettings, after: &AiSettings) {
    let state = app.state::<AppState>();
    if after.enabled && !before.enabled {
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        if let Err(e) = state.db.with(|conn| chunks::set_meta(conn, SINCE_KEY, &now)) {
            tracing::warn!("couldn't note when AI was switched on: {e}");
        }
    }
    if !after.enabled {
        state.enricher.clear();
    }
    state.enricher.resume();
    requeue_missed(&state);
    emit_status(app);
}

/// Queues items saved since AI was switched on that haven't been enriched (the provider was
/// unreachable, or the app was closed first).
pub fn requeue_missed(state: &AppState) {
    if !state.settings.get().ai.enabled {
        return;
    }
    let missed = state.db.with(|conn| match chunks::get_meta(conn, SINCE_KEY)? {
        Some(since) => concepts::unenriched_since(conn, &since, VERSION),
        None => Ok(Vec::new()),
    });
    match missed {
        Ok(ids) => ids.iter().for_each(|id| state.enricher.enqueue(id, Lane::Backfill)),
        Err(e) => tracing::warn!("couldn't find items waiting for AI: {e}"),
    }
}

/// Asks the provider for a tiny answer, to check the settings and key work together.
pub fn test_connection(connection: &AiConnection, key: Option<&str>) -> Result<()> {
    Ok(llm::test(&connection_config(connection, key)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_items_go_before_catching_up_and_each_is_queued_once() {
        let enricher = Enricher::default();
        enricher.enqueue("old", Lane::Backfill);
        enricher.enqueue("fresh", Lane::New);
        enricher.enqueue("fresh", Lane::New);
        assert_eq!(enricher.status(true).pending, 2);
        assert_eq!(enricher.next(), "fresh");
        enricher.done("fresh", None);
        assert_eq!(enricher.next(), "old");
        // Put back after a failure, it can be queued again.
        enricher.done("old", Some(Lane::Backfill));
        assert_eq!(enricher.status(true).pending, 1);
        assert_eq!(enricher.status(false).state, "off");
    }

    #[test]
    fn a_pause_holds_the_queue_until_it_ends() {
        let enricher = Enricher::default();
        enricher.enqueue("a", Lane::New);
        enricher.pause("slow down".into(), Some(Instant::now() + Duration::from_millis(60)));
        let status = enricher.status(true);
        assert_eq!((status.state, status.message.as_deref()), ("paused", Some("slow down")));
        let started = Instant::now();
        assert_eq!(enricher.next(), "a");
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert_eq!(enricher.status(true).state, "working");
    }

    #[test]
    fn resume_clears_an_open_ended_pause() {
        let enricher = std::sync::Arc::new(Enricher::default());
        enricher.pause("bad key".into(), None);
        enricher.enqueue("a", Lane::New);
        let waiting = enricher.clone();
        let worker = std::thread::spawn(move || waiting.next());
        std::thread::sleep(Duration::from_millis(30));
        enricher.resume();
        assert_eq!(worker.join().unwrap(), "a");
    }
}
