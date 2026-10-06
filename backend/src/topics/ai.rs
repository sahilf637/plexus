//! Naming topics with the connected AI provider (the local names in `names` are the fallback).
//!
//! One small call per topic that needs it. What the provider sees is bounded like every other
//! call: the titles and one-line summaries of the topic's most central notes, the names of its
//! sub-topics and siblings, and the local name. Never the notes' text, never a secret. Only the
//! top two levels are named this way, to keep the number of calls small.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::capture::secrets::find_secret;
use crate::db::items;
use crate::db::topics::{self as store, LabelBy};
use crate::db::graph;
use crate::enrich::{self, Lane};
use crate::error::{Error, Result};
use crate::llm::{LlmError, Request};
use crate::state::AppState;

/// Prefix of a naming job in the enrichment queue (the rest is the topic's id).
pub const JOB_PREFIX: &str = "topic:";
/// Only topics this high in the tree are named by the AI (0 is the top).
const MAX_LEVEL: i64 = 1;
/// Smaller topics keep their local names.
const MIN_ITEMS: usize = 5;
/// How many of a topic's notes the provider is shown.
const SAMPLE: usize = 12;
const SIBLINGS: usize = 10;
const CHILDREN: usize = 12;
const MAX_NAME_CHARS: usize = 40;
const MAX_SUMMARY_CHARS: usize = 220;
const MAX_ANSWER_TOKENS: u32 = 200;
/// An AI name is kept while at least this much of the topic is what it was named for.
pub const KEEP_NAME_OVERLAP: f64 = 0.6;

const SYSTEM: &str = "You name a group of notes from someone's personal knowledge base. \
Reply with JSON only. \
name: 1 to 4 words saying what the group is about, specific rather than generic (never 'Notes', 'Misc' or 'General'), in the language the notes are written in, with no quotation marks and no trailing punctuation. It must differ from the sibling names you are given. \
summary: one plain sentence, at most 25 words, saying what the group covers. \
Use only the notes shown; do not invent topics they do not contain.";

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "summary"],
        "properties": { "name": { "type": "string" }, "summary": { "type": "string" } }
    })
}

#[derive(Deserialize)]
struct Answer {
    name: String,
    summary: String,
}

/// The queue entry for naming a topic.
pub fn job(topic_id: &str) -> String {
    format!("{JOB_PREFIX}{topic_id}")
}

/// Queues naming for the topics that need it: ones still named locally, and ones the AI named
/// that have since changed a lot. Does nothing without a connected provider.
pub fn queue_names(state: &AppState) {
    let ai = state.settings.get().ai;
    if !ai.enabled || ai.active_connection().is_none() {
        return;
    }
    let wanted = state.db.with(|conn| {
        let topics = store::all(conn)?;
        let members = store::members_with_descendants(&topics, &store::memberships(conn)?);
        Ok(topics
            .into_iter()
            .filter(|t| t.id != crate::topics::LOOSE_ID && t.level <= MAX_LEVEL && t.label_by != LabelBy::You)
            .filter(|t| members.get(&t.id).is_some_and(|m| m.len() >= MIN_ITEMS))
            .filter(|t| match (&t.label_by, &t.named_members) {
                (LabelBy::Ai, Some(named)) => {
                    let named: HashSet<String> = named.iter().cloned().collect();
                    crate::topics::matching::overlap(&named, &members[&t.id]) < KEEP_NAME_OVERLAP
                }
                (LabelBy::Ai, None) => false,
                _ => true,
            })
            .map(|t| t.id)
            .collect::<Vec<_>>())
    });
    match wanted {
        Ok(ids) => ids.iter().for_each(|id| state.enricher.enqueue(&job(id), Lane::Topics)),
        Err(e) => tracing::warn!("couldn't find topics to name: {e}"),
    }
}

/// Asks the provider to name one topic and stores the answer (unless the user named it).
pub fn name(app: &AppHandle, state: &AppState, topic_id: &str) -> Result<()> {
    let ai = state.settings.get().ai;
    if !ai.enabled {
        return Ok(());
    }
    let Some(prompt) = state.db.with(|conn| prompt_for(conn, topic_id))? else { return Ok(()) };
    let config = enrich::config_for(&ai)?;
    let schema = schema();
    let request = Request { system: SYSTEM, user: &prompt.user, images: &[], schema: &schema, max_tokens: MAX_ANSWER_TOKENS };
    let reply = enrich::call(state, &ai, &config, &request)?;
    let answer: Answer = serde_json::from_value(reply).map_err(|e| LlmError::Bad(format!("the answer didn't fit: {e}")))?;
    let Some((label, summary)) = clean(&answer) else {
        return Err(Error::Llm(LlmError::Bad("the topic name wasn't usable".into())));
    };

    let changed = state.db.with(|conn| {
        // The user may have named it, or it may have gone, while the provider thought.
        match store::get(conn, topic_id)? {
            Some(topic) if topic.label_by != LabelBy::You => {
                store::set_label(conn, topic_id, &label, LabelBy::Ai, summary.as_deref(), Some(&prompt.members))?;
                Ok(true)
            }
            _ => Ok(false),
        }
    })?;
    if changed {
        tracing::debug!(topic = topic_id, label, "topic named");
        if let Ok(data) = state.db.with(|conn| crate::topics::data(conn)) {
            let _ = app.emit("topics-changed", data);
        }
    }
    Ok(())
}

/// A usable name and summary from the provider's answer: short, on one line.
fn clean(answer: &Answer) -> Option<(String, Option<String>)> {
    let line = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = line(&answer.name).trim_matches(|c: char| c == '"' || c == '\'' || c == '.').trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return None;
    }
    let summary = line(&answer.summary);
    let summary = (!summary.is_empty()).then(|| summary.chars().take(MAX_SUMMARY_CHARS).collect());
    Some((name, summary))
}

struct Prompt {
    user: String,
    /// The topic's items now, kept to tell later how much it has changed.
    members: Vec<String>,
}

fn prompt_for(conn: &mut rusqlite::Connection, topic_id: &str) -> Result<Option<Prompt>> {
    let topics = store::all(conn)?;
    let Some(topic) = topics.iter().find(|t| t.id == topic_id).cloned() else { return Ok(None) };
    if topic.label_by == LabelBy::You {
        return Ok(None);
    }
    let membership = store::memberships(conn)?;
    let all_members = store::members_with_descendants(&topics, &membership);
    let members: Vec<String> = {
        let mut m: Vec<String> = all_members.get(topic_id).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        m.sort();
        m
    };
    if members.len() < MIN_ITEMS {
        return Ok(None);
    }
    // Already named for (nearly) these notes: nothing to ask.
    if topic.label_by == LabelBy::Ai
        && let Some(named) = &topic.named_members
    {
        let named: HashSet<String> = named.iter().cloned().collect();
        let now: HashSet<String> = members.iter().cloned().collect();
        if crate::topics::matching::overlap(&named, &now) >= KEEP_NAME_OVERLAP {
            return Ok(None);
        }
    }

    // The most central notes: the ones with the most links to others in the topic.
    let inside: HashSet<&str> = members.iter().map(String::as_str).collect();
    let mut links: HashMap<String, f64> = HashMap::new();
    for edge in graph::edges(conn)? {
        if inside.contains(edge.src.as_str()) && inside.contains(edge.dst.as_str()) {
            let w = crate::topics::graph::edge_weight(&edge.kind, edge.tier);
            *links.entry(edge.src.clone()).or_insert(0.0) += w;
            *links.entry(edge.dst).or_insert(0.0) += w;
        }
    }
    let mut ranked: Vec<&String> = members.iter().collect();
    ranked.sort_by(|a, b| {
        links.get(b.as_str()).unwrap_or(&0.0).total_cmp(links.get(a.as_str()).unwrap_or(&0.0)).then_with(|| a.cmp(b))
    });

    let mut lines = Vec::new();
    for id in ranked {
        if lines.len() == SAMPLE {
            break;
        }
        let Some(item) = items::get(conn, id)? else { continue };
        let summary = item.summary.clone().unwrap_or_default();
        // A secret someone chose to keep never goes out inside a card.
        if find_secret(&item.title).is_some() || find_secret(&summary).is_some() {
            continue;
        }
        let line = if summary.is_empty() { cut(&item.title, 90) } else { format!("{} — {}", cut(&item.title, 90), cut(&summary, 140)) };
        lines.push(format!("- {line}"));
    }

    let names_of = |filter: &dyn Fn(&&store::Topic) -> bool, limit: usize| -> Vec<String> {
        topics.iter().filter(filter).take(limit).map(|t| t.label.clone()).collect()
    };
    let children = names_of(&|t| t.parent_id.as_deref() == Some(topic_id), CHILDREN);
    let siblings = names_of(&|t| t.id != topic_id && t.parent_id == topic.parent_id && t.id != crate::topics::LOOSE_ID, SIBLINGS);

    let mut user = format!("The group has {} notes. Its working name is \"{}\".\n\nSome of its notes:\n{}\n", members.len(), topic.label, lines.join("\n"));
    if !children.is_empty() {
        user.push_str(&format!("\nIts sub-groups are called: {}\n", children.join("; ")));
    }
    if !siblings.is_empty() {
        user.push_str(&format!("\nOther groups at this level (do not reuse their names): {}\n", siblings.join("; ")));
    }
    Ok(Some(Prompt { user, members }))
}

fn cut(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max { flat } else { format!("{}…", flat.chars().take(max).collect::<String>()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::db::items::{Status, sample};
    use crate::db::topics::Topic;

    #[test]
    fn answers_are_cleaned_and_bad_ones_refused() {
        let ok = clean(&Answer { name: "  \"Rust ownership\".\n".into(), summary: "How values are\nowned.".into() });
        assert_eq!(ok, Some(("Rust ownership".to_string(), Some("How values are owned.".to_string()))));
        assert_eq!(clean(&Answer { name: "".into(), summary: "x".into() }), None);
        assert_eq!(clean(&Answer { name: "x".repeat(60), summary: "x".into() }), None);
        assert_eq!(clean(&Answer { name: "Fine".into(), summary: "".into() }), Some(("Fine".to_string(), None)));
    }

    fn topic(id: &str, parent: Option<&str>, level: i64, by: LabelBy) -> Topic {
        Topic {
            id: id.into(),
            parent_id: parent.map(String::from),
            level,
            label: format!("Local {id}"),
            label_by: by,
            summary: None,
            named_members: None,
            centroid: None,
            version: 1,
        }
    }

    #[test]
    fn the_request_is_bounded_and_leaves_secrets_out() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        db.with(|conn| {
            let mut members = Vec::new();
            for i in 0..30 {
                let id = format!("i{i:02}");
                let mut item = sample(&id, &format!("items/{id}.md"), &id);
                item.title = if i == 3 { "AKIAIOSFODNN7EXAMPLE key".to_string() } else { format!("Ownership note {i}") };
                item.status = Status::Linked;
                items::upsert(conn, &item, "")?;
                members.push((id, "t1".to_string()));
            }
            let topics = [topic("t1", None, 0, LabelBy::Auto), topic("t2", None, 0, LabelBy::Auto), topic("t1a", Some("t1"), 1, LabelBy::Auto)];
            // t1a holds no items of its own here; t1 holds all 30.
            store::replace_all(conn, &topics, &members)?;

            let prompt = prompt_for(conn, "t1")?.expect("a topic this big gets a request");
            assert!(prompt.user.contains("The group has 30 notes"));
            assert_eq!(prompt.user.matches("\n- ").count(), SAMPLE, "a fixed number of notes is shown");
            assert!(!prompt.user.contains("AKIAIOSFODNN7EXAMPLE"), "a secret never goes out");
            assert!(prompt.user.contains("Local t2"), "sibling names are given so they aren't reused");
            assert!(prompt.user.contains("Local t1a"), "sub-group names are given");
            assert_eq!(prompt.members.len(), 30);

            // A name you gave is never asked about again; a small topic isn't asked either.
            store::set_label(conn, "t1", "Mine", LabelBy::You, None, None)?;
            assert!(prompt_for(conn, "t1")?.is_none());
            assert!(prompt_for(conn, "t2")?.is_none());
            Ok(())
        })
        .unwrap();
    }
}
