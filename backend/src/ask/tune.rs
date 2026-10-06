//! Measuring Ask on a seeded library (`npm run seed -- <folder>`) with a real provider: runs
//! questions whose answers are known to be in one seeded topic, and reports how often the cited
//! notes are the right ones, how many steps and calls it took, and how long the answers were.
//! Questions the notes can't answer should come back without sources.
//!
//!   PLEXUS_SEED=<folder> PLEXUS_ASK_URL=https://api.openai.com/v1 PLEXUS_ASK_MODEL=gpt-4o-mini \
//!   PLEXUS_ASK_KEY=… cargo test tune_ask -- --ignored --nocapture
//!
//! `PLEXUS_ASK_KIND` is `openai` (the default; also Groq, Ollama…), `anthropic` or `gemini`.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::session::Asker;
use super::tools::Finder;
use super::{Event, Model};
use crate::db::{Db, asks};
use crate::error::Result;
use crate::llm::{Config, LlmError, ProviderKind, Request};
use crate::pipeline::embed::{Embedder, MODEL_DIR};
use crate::pipeline::search;
use crate::topics::tune::load;

/// (question, the seeded topic its answer is in; `None` where the notes have no answer).
const QUESTIONS: &[(&str, Option<&str>)] = &[
    ("What does the borrow checker prevent?", Some("rust")),
    ("Why does sourdough bread need steam in the oven?", Some("baking")),
    ("How does spaced repetition help memory?", Some("memory")),
    ("What is a pulsar?", Some("astronomy")),
    ("What is the Paris Agreement?", Some("climate")),
    ("Who was Miles Davis and what is Kind of Blue?", Some("jazz")),
    ("What is the Sicilian Defence in chess?", Some("chess")),
    ("How does backpropagation train a neural network?", Some("ml")),
    ("What were the Punic Wars?", Some("rome")),
    ("What is companion planting?", Some("gardening")),
    ("How does espresso differ from a French press?", Some("coffee")),
    ("What is VO2 max?", Some("running")),
    ("Compare the memory techniques I have saved notes about.", Some("memory")),
    ("How are Rust's ownership and garbage collection related?", Some("rust")),
    ("What is the capital of Mongolia?", None),
    ("How do I change a flat tyre on a car?", None),
];

/// Counts the calls a run makes.
struct Counting<'a> {
    inner: &'a Config,
    calls: Cell<usize>,
}

impl Model for Counting<'_> {
    fn complete(&self, request: &Request) -> std::result::Result<Value, LlmError> {
        self.calls.set(self.calls.get() + 1);
        self.inner.complete(request)
    }
}

/// The app's search, over the scratch index, without the app around it.
struct Direct<'a> {
    db: &'a Db,
    embedder: &'a Embedder,
}

impl Finder for Direct<'_> {
    fn hybrid(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        search::hybrid_with(self.db, self.embedder, query, limit)
    }

    fn semantic(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let mut found = search::semantic_with(self.db, self.embedder, query)?;
        found.truncate(limit);
        Ok(found)
    }
}

fn provider() -> Config {
    let url = std::env::var("PLEXUS_ASK_URL").expect("set PLEXUS_ASK_URL (the provider's API address)");
    let model = std::env::var("PLEXUS_ASK_MODEL").expect("set PLEXUS_ASK_MODEL");
    let kind = match std::env::var("PLEXUS_ASK_KIND").as_deref() {
        Ok("anthropic") => ProviderKind::Anthropic,
        Ok("gemini") => ProviderKind::Gemini,
        _ => ProviderKind::OpenAi,
    };
    Config { kind, base_url: url, model, api_key: std::env::var("PLEXUS_ASK_KEY").ok().filter(|k| !k.is_empty()) }
}

#[test]
#[ignore = "needs the model files, a seeded library and a real AI provider; prints a report"]
fn tune_ask() {
    let Some(seed) = std::env::var_os("PLEXUS_SEED").map(PathBuf::from) else {
        panic!("set PLEXUS_SEED to a library made by `npm run seed -- <folder>`");
    };
    let config = provider();
    let (db, _scratch, seed_topic) = load(seed);
    let model_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(MODEL_DIR);
    let embedder = Embedder::new(Embedder::find(vec![model_dir]));
    let finder = Direct { db: &db, embedder: &embedder };
    let asker = Asker::default();

    println!("{} notes; asking {} questions with {}\n", seed_topic.len(), QUESTIONS.len(), config.model);
    println!("{:<58} {:<8} {:<8} {:>5} {:>5} {:>6} {:>4}  topics cited", "question", "effort", "state", "steps", "calls", "chars", "src");

    let (mut answerable, mut right, mut unanswerable, mut quiet) = (0, 0, 0, 0);
    let (mut steps_total, mut calls_total) = (0, 0);
    for (question, expected) in QUESTIONS {
        let model = Counting { inner: &config, calls: Cell::new(0) };
        let mut steps = 0;
        let result = asker
            .begin(&db, &finder, question, None, 0)
            .and_then(|id| asker.execute(&db, &finder, &model, &id, &mut |_: &Event| steps += 1));
        let ask = match result {
            Ok(ask) => ask,
            Err(error) => {
                println!("{question:<58} failed: {error}");
                continue;
            }
        };
        let cited: Vec<&str> = ask.sources.iter().filter_map(|s| seed_topic.get(&s.item_id).map(String::as_str)).collect();
        let mut distinct = cited.clone();
        distinct.sort_unstable();
        distinct.dedup();
        match expected {
            Some(topic) => {
                answerable += 1;
                // Right when something is cited and everything cited is on the expected topic.
                if !cited.is_empty() && cited.iter().all(|t| t == topic) {
                    right += 1;
                }
            }
            None => {
                unanswerable += 1;
                if ask.sources.is_empty() {
                    quiet += 1;
                }
            }
        }
        steps_total += steps;
        calls_total += model.calls.get();
        let state = format!("{:?}{}", ask.state, if ask.unsupported { "!" } else { "" }).to_lowercase();
        println!(
            "{:<58} {:<8} {:<8} {:>5} {:>5} {:>6} {:>4}  {}",
            question.chars().take(57).collect::<String>(),
            ask.effort,
            state,
            steps,
            model.calls.get(),
            ask.answer.chars().count(),
            ask.sources.len(),
            distinct.join(", "),
        );
        if ask.state != asks::State::Done {
            println!("    ↳ {}", ask.answer.lines().next().unwrap_or(""));
        }
    }
    let n = QUESTIONS.len().max(1);
    println!("\nanswerable: {right}/{answerable} cited only the right topic");
    println!("unanswerable: {quiet}/{unanswerable} answered without inventing sources");
    println!("average: {:.1} tool steps and {:.1} model calls per question", steps_total as f64 / n as f64, calls_total as f64 / n as f64);
}
