//! The tools the agent works with, over the library. Results are compact (titles, one-line
//! summaries, short snippets, never whole notes unless `read` pages through one) and capped in
//! size, so a question costs about the same however big the library is.
//!
//! The model never sees real ids: notes are `n1`, `n2`…, topics `t1`, `t2`…, in the order they are
//! first shown. A name it was not shown is refused, which also stops it citing things it made up.
//! Notes that look like secrets are invisible to every tool.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use regex::RegexBuilder;
use rusqlite::{Connection, OptionalExtension};

use super::prompt::{Call, Mode};
use super::{ToolOutput, Tools};
use crate::capture::detect::truncate;
use crate::capture::secrets::find_secret;
use crate::db::items::Item;
use crate::db::{Db, graph, items, topics};
use crate::error::Result;
use crate::state::AppState;

/// The most a tool result may be (characters; about 1,500 tokens).
const CAP_CHARS: usize = 6_000;
const RESULTS: usize = 8;
const SNIPPET: usize = 160;
const READ_DEFAULT: usize = 3_000;
const READ_MAX: usize = 4_000;
const NEIGHBOURS: usize = 12;
const SECOND_HOP: usize = 3;
const MEMBERS: usize = 10;
/// How many notes are looked at before a search is filtered by scope and secrets.
const OVERFETCH: usize = 40;
/// An exact search stops here, whatever it has covered.
const SCAN_TIME: Duration = Duration::from_millis(1_500);
const CANDIDATES: usize = 10;

/// Finding notes by words and by meaning (the app's search; a fixed list in tests).
pub trait Finder {
    fn hybrid(&self, query: &str, limit: usize) -> Result<Vec<String>>;
    fn semantic(&self, query: &str, limit: usize) -> Result<Vec<String>>;
}

pub struct AppFinder<'a>(pub &'a AppState);

impl Finder for AppFinder<'_> {
    fn hybrid(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        crate::pipeline::search::hybrid(self.0, query, limit)
    }

    fn semantic(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let mut found = crate::pipeline::search::semantic(self.0, query)?;
        found.truncate(limit);
        Ok(found)
    }
}

/// Short names for ids, handed out in the order things are shown.
struct Names {
    prefix: char,
    by_name: HashMap<String, String>,
    by_id: HashMap<String, String>,
}

impl Names {
    fn new(prefix: char) -> Self {
        Self { prefix, by_name: HashMap::new(), by_id: HashMap::new() }
    }

    fn name(&mut self, id: &str) -> String {
        if let Some(name) = self.by_id.get(id) {
            return name.clone();
        }
        let name = format!("{}{}", self.prefix, self.by_id.len() + 1);
        self.by_id.insert(id.to_string(), name.clone());
        self.by_name.insert(name.clone(), id.to_string());
        name
    }

    fn id(&self, name: &str) -> Option<&String> {
        self.by_name.get(name.trim())
    }
}

/// What the tools remember between steps (and between a failed run and its resumption): the names
/// handed out, so `n3` keeps meaning the same note.
pub struct Memory {
    notes: Names,
    topics: Names,
    /// Whether a note looks like a secret, worked out once.
    secret: HashMap<String, bool>,
    read: HashSet<String>,
}

impl Default for Memory {
    fn default() -> Self {
        Self { notes: Names::new('n'), topics: Names::new('t'), secret: HashMap::new(), read: HashSet::new() }
    }
}

impl Memory {
    /// The note a name stands for, if it was ever shown.
    pub fn resolve(&self, name: &str) -> Option<String> {
        self.notes.id(name).cloned()
    }
}

pub struct LibraryTools<'a> {
    db: &'a Db,
    finder: &'a dyn Finder,
    /// A topic or note the user limited the question to; the model can't widen it.
    scope: Option<String>,
    mem: Memory,
}

/// What the agent is given before its first step: the notes local search found for the question.
pub struct Candidates {
    pub cards: String,
    pub ids: Vec<String>,
}

impl<'a> LibraryTools<'a> {
    pub fn new(db: &'a Db, finder: &'a dyn Finder, scope: Option<&str>) -> Self {
        Self::resume(db, finder, scope, Memory::default())
    }

    pub fn resume(db: &'a Db, finder: &'a dyn Finder, scope: Option<&str>, mem: Memory) -> Self {
        Self { db, finder, scope: scope.map(str::to_string), mem }
    }

    pub fn into_memory(self) -> Memory {
        self.mem
    }

    /// Cards (name | title | summary) for the notes closest to the question, to start from.
    pub fn candidates(&mut self, question: &str) -> Candidates {
        let found = self.finder.hybrid(question, OVERFETCH).unwrap_or_default();
        let db = self.db;
        let (cards, ids) = db
            .with(|conn| {
                let allowed = self.scope_set(conn, self.scope.clone())?;
                let ids = self.keep(conn, found, allowed.as_ref(), CANDIDATES);
                let mut cards = Vec::new();
                for id in &ids {
                    if let Some(line) = self.card(conn, id)? {
                        cards.push(line);
                    }
                }
                Ok((cards.join("\n"), ids))
            })
            .unwrap_or_default();
        Candidates { cards, ids }
    }

    // --- shared pieces ---------------------------------------------------------------------

    /// The notes a scope covers: a topic's (sub-topics included), or a note and what it links to.
    /// `None` is the whole library.
    fn scope_set(&self, conn: &Connection, scope: Option<String>) -> Result<Option<HashSet<String>>> {
        let Some(scope) = scope else { return Ok(None) };
        if topics::get(conn, &scope)?.is_some() {
            let all = topics::all(conn)?;
            let members = topics::memberships(conn)?;
            return Ok(Some(topics::members_with_descendants(&all, &members).remove(&scope).unwrap_or_default()));
        }
        if items::get(conn, &scope)?.is_some() {
            let mut set: HashSet<String> = graph::edges_of(conn, &scope)?
                .into_iter()
                .map(|e| if e.src == scope { e.dst } else { e.src })
                .collect();
            set.insert(scope);
            return Ok(Some(set));
        }
        Ok(Some(HashSet::new()))
    }

    /// The scope for a call: the user's if there is one, else the topic the model named.
    fn call_scope(&self, asked: &Option<String>) -> std::result::Result<Option<String>, String> {
        if self.scope.is_some() {
            return Ok(self.scope.clone());
        }
        match asked {
            None => Ok(None),
            Some(name) => match self.mem.topics.id(name) {
                Some(id) => Ok(Some(id.clone())),
                None => Err(format!("`{name}` isn't a topic you were shown.")),
            },
        }
    }

    fn is_secret(&mut self, conn: &Connection, id: &str) -> bool {
        if let Some(known) = self.mem.secret.get(id) {
            return *known;
        }
        let item = items::get(conn, id).ok().flatten();
        let secret = item.is_none_or(|item| {
            find_secret(&item.title).is_some()
                || item.summary.as_deref().is_some_and(|s| find_secret(s).is_some())
                || body_of(conn, id).is_ok_and(|body| find_secret(&body).is_some())
        });
        self.mem.secret.insert(id.to_string(), secret);
        secret
    }

    /// Keeps the notes inside the scope that aren't secrets, up to `limit`.
    fn keep(&mut self, conn: &Connection, ids: Vec<String>, allowed: Option<&HashSet<String>>, limit: usize) -> Vec<String> {
        let mut kept = Vec::new();
        for id in ids {
            if allowed.is_some_and(|set| !set.contains(&id)) || self.is_secret(conn, &id) {
                continue;
            }
            if !kept.contains(&id) {
                kept.push(id);
            }
            if kept.len() == limit {
                break;
            }
        }
        kept
    }

    /// `name | title | one line`, naming the note as it is shown.
    fn card(&mut self, conn: &Connection, id: &str) -> Result<Option<String>> {
        let Some(item) = items::get(conn, id)? else { return Ok(None) };
        let line = blurb(&item, &body_of(conn, id)?);
        let name = self.mem.notes.name(id);
        let seen = if self.mem.read.contains(id) { " (already read)" } else { "" };
        Ok(Some(format!("{name} | {} | {line}{seen}", one_line(&item.title))))
    }

    /// The note a name stands for, or the message to give the model.
    fn note(&self, name: &str) -> std::result::Result<String, String> {
        self.mem.notes.id(name).cloned().ok_or_else(|| format!("`{name}` isn't a note you were shown. Use names from earlier results."))
    }

    // --- the tools -------------------------------------------------------------------------

    fn search(&mut self, query: &str, mode: Mode, asked: &Option<String>) -> ToolOutput {
        let scope = match self.call_scope(asked) {
            Ok(scope) => scope,
            Err(message) => return refusal(message),
        };
        // Search first (it takes the database itself), then filter and describe.
        let found = match mode {
            Mode::Hybrid => self.finder.hybrid(query, OVERFETCH),
            Mode::Semantic => self.finder.semantic(query, OVERFETCH),
            Mode::Keyword => self.db.with(|conn| graph::search(conn, query, OVERFETCH)),
            Mode::Exact => Ok(Vec::new()),
        };
        let found = match found {
            Ok(found) => found,
            Err(e) => return refusal(format!("The search failed: {e}")),
        };
        let db = self.db;
        let done = db.with(|conn| {
            let allowed = self.scope_set(conn, scope)?;
            let mut lines = Vec::new();
            let mut ids = Vec::new();
            if mode == Mode::Exact {
                for (id, snippet) in self.scan(conn, query, allowed.as_ref())? {
                    let Some(card) = self.card(conn, &id)? else { continue };
                    let head = card.split(" | ").take(2).collect::<Vec<_>>().join(" | ");
                    lines.push(format!("{head} | …{snippet}…"));
                    ids.push(id);
                }
            } else {
                for id in self.keep(conn, found, allowed.as_ref(), RESULTS) {
                    if let Some(card) = self.card(conn, &id)? {
                        lines.push(card);
                        ids.push(id);
                    }
                }
            }
            Ok((lines, ids))
        });
        match done {
            Ok((lines, _)) if lines.is_empty() => refusal("No notes matched.".to_string()),
            Ok((lines, ids)) => ToolOutput { text: cap(&lines.join("\n")), shown: ids, read: Vec::new() },
            Err(e) => refusal(format!("The search failed: {e}")),
        }
    }

    /// Notes whose text matches the pattern (a regex, or the literal text if it isn't one), with
    /// a snippet around the first match. Bounded by count and time.
    fn scan(&mut self, conn: &Connection, pattern: &str, allowed: Option<&HashSet<String>>) -> Result<Vec<(String, String)>> {
        let regex = RegexBuilder::new(pattern)
            .case_insensitive(true)
            .size_limit(1 << 20)
            .build()
            .or_else(|_| RegexBuilder::new(&regex::escape(pattern)).case_insensitive(true).build());
        let Ok(regex) = regex else { return Ok(Vec::new()) };
        let started = Instant::now();
        let mut stmt = conn.prepare("SELECT item_id, body FROM items_fts")?;
        let mut rows = stmt.query([])?;
        let mut found = Vec::new();
        while let Some(row) = rows.next()? {
            if found.len() == RESULTS || started.elapsed() > SCAN_TIME {
                break;
            }
            let (id, body): (String, String) = (row.get(0)?, row.get(1)?);
            if allowed.is_some_and(|set| !set.contains(&id)) {
                continue;
            }
            let Some(hit) = regex.find(&body) else { continue };
            if self.is_secret(conn, &id) {
                continue;
            }
            found.push((id, around(&body, hit.start(), hit.end())));
        }
        Ok(found)
    }

    fn read(&mut self, name: &str, offset: usize, limit: usize) -> ToolOutput {
        let id = match self.note(name) {
            Ok(id) => id,
            Err(message) => return refusal(message),
        };
        let limit = if limit == 0 { READ_DEFAULT } else { limit.min(READ_MAX) };
        let db = self.db;
        let done = db.with(|conn| {
            if self.is_secret(conn, &id) {
                return Ok(None);
            }
            let Some(item) = items::get(conn, &id)? else { return Ok(None) };
            Ok(Some((item, body_of(conn, &id)?)))
        });
        let (item, body) = match done {
            Ok(Some(found)) => found,
            Ok(None) => return refusal(format!("`{name}` can't be read.")),
            Err(e) => return refusal(format!("Reading failed: {e}")),
        };
        let total = body.chars().count();
        let page: String = body.chars().skip(offset).take(limit).collect();
        let end = (offset + page.chars().count()).min(total);
        let mut text = format!("{name}: {}", one_line(&item.title));
        if let Some(url) = &item.source_url {
            text.push_str(&format!(" ({url})"));
        }
        text.push_str(&format!("\nCharacters {offset}-{end} of {total}.\n\n{page}"));
        if end < total {
            text.push_str(&format!("\n\n[more: read {name} again with offset {end}]"));
        }
        self.mem.read.insert(id.clone());
        ToolOutput { text, shown: vec![id.clone()], read: vec![id] }
    }

    fn neighbors(&mut self, name: &str, depth: usize) -> ToolOutput {
        let id = match self.note(name) {
            Ok(id) => id,
            Err(message) => return refusal(message),
        };
        let db = self.db;
        let done = db.with(|conn| {
            let mut lines = Vec::new();
            let mut shown = Vec::new();
            let mut visited = HashSet::from([id.clone()]);
            let mut front = vec![id.clone()];
            for hop in 1..=depth {
                let mut next = Vec::new();
                for from in &front {
                    let limit = if hop == 1 { NEIGHBOURS } else { SECOND_HOP };
                    let mut taken = 0;
                    for edge in graph::edges_of(conn, from)? {
                        let other = if edge.src == *from { edge.dst.clone() } else { edge.src.clone() };
                        if taken == limit {
                            break;
                        }
                        if visited.contains(&other) || self.is_secret(conn, &other) {
                            continue;
                        }
                        let Some(card) = self.card(conn, &other)? else { continue };
                        let head = card.split(" | ").take(2).collect::<Vec<_>>().join(" | ");
                        let reason = edge.reason.as_deref().map(one_line).unwrap_or_default();
                        let via = if hop == 1 { String::new() } else { format!(" (via {})", self.mem.notes.name(from)) };
                        let direction = if edge.src == *from { "→" } else { "←" };
                        lines.push(format!("{head} | {} {direction} {:?}{via}: {reason}", edge.kind, edge.tier));
                        visited.insert(other.clone());
                        shown.push(other.clone());
                        next.push(other);
                        taken += 1;
                    }
                }
                front = next;
            }
            Ok((lines, shown))
        });
        match done {
            Ok((lines, _)) if lines.is_empty() => refusal(format!("{name} has no links.")),
            Ok((lines, shown)) => ToolOutput { text: cap(&lines.join("\n")), shown, read: Vec::new() },
            Err(e) => refusal(format!("Looking up links failed: {e}")),
        }
    }

    fn topics(&mut self, asked: &Option<String>) -> ToolOutput {
        let asked_id = match asked {
            None => None,
            Some(name) => match self.mem.topics.id(name) {
                Some(id) => Some(id.clone()),
                None => return refusal(format!("`{name}` isn't a topic you were shown.")),
            },
        };
        let db = self.db;
        let done = db.with(|conn| {
            let all = topics::all(conn)?;
            if all.is_empty() {
                return Ok((String::from("There are no topics yet."), Vec::new()));
            }
            let members = topics::members_with_descendants(&all, &topics::memberships(conn)?);
            let size = |id: &str| members.get(id).map_or(0, HashSet::len);
            let describe = |this: &mut Self, topic: &topics::Topic| {
                let summary = topic.summary.as_deref().map(one_line).unwrap_or_default();
                format!("{} | {} ({} notes): {summary}", this.mem.topics.name(&topic.id), one_line(&topic.label), size(&topic.id))
            };
            let Some(id) = asked_id else {
                let lines: Vec<String> = all.iter().filter(|t| t.parent_id.is_none() && size(&t.id) > 0).map(|t| describe(self, t)).collect();
                return Ok((cap(&lines.join("\n")), Vec::new()));
            };
            let Some(topic) = all.iter().find(|t| t.id == id) else { return Ok(("That topic no longer exists.".to_string(), Vec::new())) };
            let mut text = describe(self, topic);
            let subs: Vec<String> = all.iter().filter(|t| t.parent_id.as_deref() == Some(&id) && size(&t.id) > 0).map(|t| describe(self, t)).collect();
            if !subs.is_empty() {
                text.push_str(&format!("\nSub-topics:\n{}", subs.join("\n")));
            }
            // The notes directly in it (those in sub-topics are reached through them).
            let direct: Vec<String> = topics::memberships(conn)?.into_iter().filter(|(_, leaf)| *leaf == id).map(|(item, _)| item).collect();
            let mut direct = self.keep(conn, direct, None, usize::MAX);
            direct.sort();
            let more = direct.len().saturating_sub(MEMBERS);
            let mut shown = Vec::new();
            let mut cards = Vec::new();
            for item in direct.into_iter().take(MEMBERS) {
                if let Some(card) = self.card(conn, &item)? {
                    cards.push(card);
                    shown.push(item);
                }
            }
            if !cards.is_empty() {
                text.push_str(&format!("\nNotes:\n{}", cards.join("\n")));
            }
            if more > 0 {
                text.push_str(&format!("\n…and {more} more; search with scope {} to find them.", self.mem.topics.name(&id)));
            }
            Ok((cap(&text), shown))
        });
        match done {
            Ok((text, shown)) => ToolOutput { text, shown, read: Vec::new() },
            Err(e) => refusal(format!("Looking up topics failed: {e}")),
        }
    }
}

impl Tools for LibraryTools<'_> {
    fn run(&mut self, call: &Call) -> ToolOutput {
        match call {
            Call::Search { query, mode, scope } => self.search(query, *mode, scope),
            Call::Read { item, offset, limit } => self.read(item, *offset, *limit),
            Call::Neighbors { item, depth } => self.neighbors(item, *depth),
            Call::Topics { topic } => self.topics(topic),
            Call::Answer { .. } => ToolOutput::default(),
        }
    }
}

fn refusal(message: String) -> ToolOutput {
    ToolOutput { text: message, ..ToolOutput::default() }
}

fn cap(text: &str) -> String {
    if text.chars().count() <= CAP_CHARS {
        return text.to_string();
    }
    format!("{}\n[cut: too long to show]", truncate(text, CAP_CHARS))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn body_of(conn: &Connection, id: &str) -> Result<String> {
    let body: Option<String> = conn.query_row("SELECT body FROM items_fts WHERE item_id = ?1", [id], |row| row.get(0)).optional()?;
    Ok(body.unwrap_or_default())
}

/// The summary if there is one, else the start of the text.
fn blurb(item: &Item, body: &str) -> String {
    match item.summary.as_deref().map(one_line).filter(|s| !s.is_empty()) {
        Some(summary) => truncate(&summary, SNIPPET),
        None => truncate(&one_line(body), SNIPPET),
    }
}

/// Everything a quote may be found in: the note's title, summary and text.
pub(super) fn searchable(conn: &Connection, id: &str) -> Option<String> {
    let item = items::get(conn, id).ok().flatten()?;
    let body = body_of(conn, id).ok()?;
    Some(format!("{}\n{}\n{body}", item.title, item.summary.unwrap_or_default()))
}

/// The text around a match, cut on character boundaries.
fn around(body: &str, start: usize, end: usize) -> String {
    let before: Vec<char> = body[..start].chars().rev().take(60).collect();
    let before: String = before.into_iter().rev().collect();
    let after: String = body[end..].chars().take(60).collect();
    one_line(&format!("{before}{}{after}", &body[start..end]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::detect::Kind;
    use crate::db::items::{Item, Status};
    use crate::db::open_temp;
    use crate::db::topics::{LabelBy, Topic};

    struct Fixed(Vec<String>);

    impl Finder for Fixed {
        fn hybrid(&self, _query: &str, _limit: usize) -> Result<Vec<String>> {
            Ok(self.0.clone())
        }

        fn semantic(&self, _query: &str, _limit: usize) -> Result<Vec<String>> {
            Ok(self.0.clone())
        }
    }

    fn item(id: &str, title: &str, summary: Option<&str>) -> Item {
        Item {
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
            summary: summary.map(str::to_string),
            file_mtime: 0,
            file_size: 0,
            status: Status::Linked,
        }
    }

    fn library() -> (tempfile::TempDir, Db) {
        let (dir, db) = open_temp();
        db.with(|conn| {
            items::upsert(conn, &item("i1", "Ownership", Some("Each value has one owner")), "Each value has one owner. Borrowing lends it out.")?;
            items::upsert(conn, &item("i2", "Borrow checker", None), "The borrow checker enforces the rules at compile time.")?;
            items::upsert(conn, &item("i3", "Soup", None), "Boil the water, add salt.")?;
            items::upsert(conn, &item("i4", "Deploy notes", None), "token = ghp_abcdefghijklmnopqrstuvwxyz0123456789 for the deploy.")?;
            items::upsert(conn, &item("i5", "Long", None), &"word ".repeat(2_000))?;
            conn.execute(
                "INSERT INTO edges (src, dst, kind, weight, reason, provenance, created_at)
                 VALUES ('i1', 'i2', 'extends', 0.9, 'the checker applies the rules', 'llm', '2026-10-06T00:00:00Z')",
                [],
            )?;
            let topic = |id: &str, parent: Option<&str>, level: i64, label: &str| Topic {
                id: id.into(),
                parent_id: parent.map(str::to_string),
                level,
                label: label.into(),
                label_by: LabelBy::Auto,
                summary: Some(format!("About {label}")),
                named_members: None,
                centroid: None,
                version: 1,
            };
            topics::replace_all(
                conn,
                &[topic("a-top", None, 0, "Rust"), topic("b-kitchen", None, 0, "Cooking")],
                &[("i1".into(), "a-top".into()), ("i2".into(), "a-top".into()), ("i3".into(), "b-kitchen".into())],
            )?;
            Ok(())
        })
        .unwrap();
        (dir, db)
    }

    fn all_ids() -> Fixed {
        Fixed(["i1", "i2", "i3", "i4", "i5"].map(String::from).to_vec())
    }

    fn search(query: &str, mode: Mode) -> Call {
        Call::Search { query: query.into(), mode, scope: None }
    }

    #[test]
    fn candidates_are_named_in_order_and_secrets_are_left_out() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None);
        let found = tools.candidates("borrowing");
        let lines: Vec<&str> = found.cards.lines().collect();
        assert!(lines[0].starts_with("n1 | Ownership | Each value has one owner"));
        assert!(!found.cards.contains("Deploy notes"));
        assert!(!found.ids.contains(&"i4".to_string()));
        assert_eq!(tools.mem.resolve("n1").as_deref(), Some("i1"));
    }

    #[test]
    fn search_results_are_capped_and_only_name_notes_inside_scope() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, Some("b-kitchen"));
        let out = tools.run(&search("anything", Mode::Hybrid));
        assert_eq!(out.shown, ["i3"]);
        assert!(out.text.contains("Soup"));
        assert!(!out.text.contains("Ownership"));
    }

    #[test]
    fn a_topic_the_model_was_shown_narrows_the_search() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None);
        let listing = tools.run(&Call::Topics { topic: None });
        assert!(listing.text.contains("t1 | Rust (2 notes)"));
        let narrowed = tools.run(&Call::Search { query: "x".into(), mode: Mode::Hybrid, scope: Some("t1".into()) });
        assert_eq!(narrowed.shown, ["i1", "i2"]);
        let unknown = tools.run(&Call::Search { query: "x".into(), mode: Mode::Hybrid, scope: Some("t9".into()) });
        assert!(unknown.text.contains("isn't a topic you were shown"));
    }

    #[test]
    fn a_name_that_was_not_shown_is_refused() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None);
        let out = tools.run(&Call::Read { item: "n7".into(), offset: 0, limit: 0 });
        assert!(out.text.contains("isn't a note you were shown"));
        assert!(out.read.is_empty());
        let out = tools.run(&Call::Neighbors { item: "i1".into(), depth: 1 });
        assert!(out.text.contains("isn't a note you were shown"));
    }

    #[test]
    fn a_secret_cannot_be_found_read_or_walked_to() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None);
        assert!(!tools.run(&search("deploy", Mode::Hybrid)).text.contains("Deploy notes"));
        assert!(!tools.run(&search("ghp_", Mode::Exact)).text.contains("Deploy"));
        // Even a name handed out before the note was recognised can't be read.
        let name = tools.mem.notes.name("i4");
        let out = tools.run(&Call::Read { item: name, offset: 0, limit: 0 });
        assert!(out.read.is_empty());
        assert!(!out.text.contains("ghp_"));
    }

    #[test]
    fn reading_pages_through_a_long_note_and_marks_it_read() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None);
        tools.run(&search("long", Mode::Hybrid));
        let long = tools.mem.notes.name("i5");
        let first = tools.run(&Call::Read { item: long.clone(), offset: 0, limit: 0 });
        assert_eq!(first.read, ["i5"]);
        assert!(first.text.contains("Characters 0-3000 of 10000"));
        assert!(first.text.contains("offset 3000"));
        assert!(first.text.chars().count() < 3_400);
        let huge = tools.run(&Call::Read { item: long, offset: 9_000, limit: 100_000 });
        assert!(huge.text.contains("Characters 9000-10000 of 10000"));
        assert!(!huge.text.contains("[more"));
        let again = tools.run(&search("long", Mode::Hybrid));
        assert!(again.text.contains("(already read)"));
    }

    #[test]
    fn exact_search_matches_text_and_a_bad_pattern_counts_as_plain_text() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None);
        let out = tools.run(&search(r"compile\s+time", Mode::Exact));
        assert_eq!(out.shown, ["i2"]);
        assert!(out.text.contains("compile time"));
        // An unclosed bracket isn't a regex; it's searched for as written.
        let out = tools.run(&search("salt.", Mode::Exact));
        assert_eq!(out.shown, ["i3"]);
        let out = tools.run(&search("[unclosed", Mode::Exact));
        assert!(out.text.contains("No notes matched"));
    }

    #[test]
    fn neighbours_say_why_they_are_linked() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None);
        tools.run(&search("owner", Mode::Hybrid));
        let own = tools.mem.notes.name("i1");
        let out = tools.run(&Call::Neighbors { item: own, depth: 1 });
        assert_eq!(out.shown, ["i2"]);
        assert!(out.text.contains("the checker applies the rules"));
        assert!(out.text.contains("extends"));
    }

    #[test]
    fn topics_list_the_top_level_then_open_up() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None);
        let top = tools.run(&Call::Topics { topic: None });
        assert!(top.text.contains("Rust (2 notes): About Rust"));
        assert!(top.text.contains("Cooking (1 notes)"));
        let inside = tools.run(&Call::Topics { topic: Some("t1".into()) });
        assert_eq!(inside.shown, ["i1", "i2"]);
        assert!(inside.text.contains("n1 | Ownership"));
    }

    #[test]
    fn nothing_comes_back_larger_than_the_cap() {
        let big = "x".repeat(CAP_CHARS * 2);
        assert!(cap(&big).chars().count() < CAP_CHARS + 50);
        assert_eq!(cap("short"), "short");
    }
}
