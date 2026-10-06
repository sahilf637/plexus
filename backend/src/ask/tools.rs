//! The tools the agent works with, over the library. Results are compact (titles, one-line
//! summaries, short snippets, never whole notes unless `read` pages through one) and capped in
//! size, so a question costs about the same however big the library is.
//!
//! The model never sees real ids: notes are `n1`, `n2`…, topics `t1`, `t2`…, in the order they are
//! first shown. A name it was not shown is refused, which also stops it citing things it made up.
//! Notes that look like secrets are invisible to every tool.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::time::{Duration, Instant};

use regex::RegexBuilder;
use rusqlite::{Connection, OptionalExtension};

use super::budget::Budget;
use super::prompt::{Call, Mode};
use super::{ToolOutput, Tools};
use crate::capture::detect::truncate;
use crate::capture::secrets::find_secret;
use crate::db::items::Item;
use crate::db::{Db, graph, items, topics};
use crate::error::Result;
use crate::state::AppState;

const SNIPPET: usize = 160;
const NEIGHBOURS: usize = 12;
const NEIGHBOURS_MAX: usize = 30;
/// A path longer than this many links is not a connection worth reporting.
const PATH_HOPS: usize = 6;
const SECOND_HOP: usize = 3;
const MEMBERS: usize = 10;
/// How many notes are looked at before a search is filtered by scope and secrets.
const OVERFETCH: usize = 40;
/// An exact search stops here, whatever it has covered.
const SCAN_TIME: Duration = Duration::from_millis(1_500);

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
    budget: Budget,
    mem: Memory,
}

/// What the agent is given before its first step: the notes local search found for the question.
pub struct Candidates {
    pub cards: String,
    pub ids: Vec<String>,
}

impl<'a> LibraryTools<'a> {
    pub fn new(db: &'a Db, finder: &'a dyn Finder, scope: Option<&str>, budget: Budget) -> Self {
        Self::resume(db, finder, scope, budget, Memory::default())
    }

    pub fn resume(db: &'a Db, finder: &'a dyn Finder, scope: Option<&str>, budget: Budget, mem: Memory) -> Self {
        Self { db, finder, scope: scope.map(str::to_string), budget, mem }
    }

    pub fn into_memory(self) -> Memory {
        self.mem
    }

    /// Cards (name | title | summary) for the notes closest to the question, to start from.
    pub fn candidates(&mut self, question: &str) -> Candidates {
        let found = self.finder.hybrid(question, OVERFETCH).unwrap_or_default();
        let candidates = self.budget.candidates;
        let db = self.db;
        let (cards, ids) = db
            .with(|conn| {
                let allowed = self.scope_set(conn, self.scope.clone())?;
                let ids = self.keep(conn, found, allowed.as_ref(), candidates);
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

    /// Cards for notes named by id (e.g. the ones a conversation cited before), so a follow-up can
    /// read them without searching again. Notes that are gone or look like secrets are skipped.
    pub fn introduce(&mut self, ids: &[String]) -> Candidates {
        let db = self.db;
        let (cards, shown) = db
            .with(|conn| {
                let allowed = self.scope_set(conn, self.scope.clone())?;
                let ids = self.keep(conn, ids.to_vec(), allowed.as_ref(), usize::MAX);
                let mut cards = Vec::new();
                let mut shown = Vec::new();
                for id in ids {
                    if let Some(line) = self.card(conn, &id)? {
                        cards.push(line);
                        shown.push(id);
                    }
                }
                Ok((cards.join("\n"), shown))
            })
            .unwrap_or_default();
        Candidates { cards, ids: shown }
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
        let results = self.budget.results;
        let cap_chars = self.budget.result_chars;
        let done = db.with(|conn| {
            let allowed = self.scope_set(conn, scope)?;
            let mut lines = Vec::new();
            let mut ids = Vec::new();
            let mut more = 0;
            if mode == Mode::Exact {
                for (id, snippet) in self.scan(conn, query, allowed.as_ref(), results)? {
                    let Some(card) = self.card(conn, &id)? else { continue };
                    let head = card.split(" | ").take(2).collect::<Vec<_>>().join(" | ");
                    lines.push(format!("{head} | …{snippet}…"));
                    ids.push(id);
                }
            } else {
                let matched = self.keep(conn, found, allowed.as_ref(), OVERFETCH);
                more = matched.len().saturating_sub(results);
                for id in matched.into_iter().take(results) {
                    if let Some(card) = self.card(conn, &id)? {
                        lines.push(card);
                        ids.push(id);
                    }
                }
            }
            Ok((lines, ids, more))
        });
        match done {
            Ok((lines, _, _)) if lines.is_empty() => refusal("No notes matched.".to_string()),
            Ok((mut lines, ids, more)) => {
                if more > 0 {
                    lines.push(format!("…and {more} more matches not shown (a narrower query or a topic scope shows others)."));
                }
                ToolOutput { text: cap(&lines.join("\n"), cap_chars), shown: ids, read: Vec::new(), label: None }
            }
            Err(e) => refusal(format!("The search failed: {e}")),
        }
    }

    /// Notes whose text matches the pattern (a regex, or the literal text if it isn't one), with
    /// a snippet around the first match. Bounded by count and time.
    fn scan(&mut self, conn: &Connection, pattern: &str, allowed: Option<&HashSet<String>>, most: usize) -> Result<Vec<(String, String)>> {
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
            if found.len() == most || started.elapsed() > SCAN_TIME {
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
        let limit = if limit == 0 { self.budget.read_page } else { limit.min(self.budget.read_max) };
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
        let label = Some(format!("read “{}”", one_line(&item.title)));
        ToolOutput { text, shown: vec![id.clone()], read: vec![id], label }
    }

    fn neighbors(&mut self, name: &str, depth: usize, limit: usize) -> ToolOutput {
        let id = match self.note(name) {
            Ok(id) => id,
            Err(message) => return refusal(message),
        };
        let db = self.db;
        let cap_chars = self.budget.result_chars;
        let first_hop = if limit == 0 { NEIGHBOURS } else { limit.min(NEIGHBOURS_MAX) };
        let done = db.with(|conn| {
            let mut lines = Vec::new();
            let mut shown = Vec::new();
            let mut hidden = 0;
            let mut visited = HashSet::from([id.clone()]);
            let mut front = vec![id.clone()];
            for hop in 1..=depth {
                let mut next = Vec::new();
                for from in &front {
                    let wanted = if hop == 1 { first_hop } else { SECOND_HOP };
                    let mut taken = 0;
                    let mut left_out = 0;
                    for edge in graph::edges_of(conn, from)? {
                        let other = if edge.src == *from { edge.dst.clone() } else { edge.src.clone() };
                        if visited.contains(&other) || self.is_secret(conn, &other) {
                            continue;
                        }
                        if taken == wanted {
                            left_out += 1;
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
                    if hop == 1 {
                        hidden = left_out;
                    }
                }
                front = next;
            }
            let label = items::get(conn, &id)?.map(|item| format!("looked at what “{}” links to", one_line(&item.title)));
            Ok((lines, shown, hidden, label))
        });
        match done {
            Ok((lines, ..)) if lines.is_empty() => refusal(format!("{name} has no links.")),
            Ok((mut lines, shown, hidden, label)) => {
                if hidden > 0 {
                    lines.push(format!("…and {hidden} more links not shown, the weakest ones (ask again with a bigger limit, up to {NEIGHBOURS_MAX}, to list them)."));
                }
                ToolOutput { text: cap(&lines.join("\n"), cap_chars), shown, read: Vec::new(), label }
            }
            Err(e) => refusal(format!("Looking up links failed: {e}")),
        }
    }

    /// How two notes connect: the chain of links with the best overall strength (strong links are
    /// worth more than weak ones, but a weak link beats no link). Weak links are included on
    /// purpose: they are often the only bridge between two areas.
    fn path(&mut self, from: &str, to: &str) -> ToolOutput {
        let (a, b) = match (self.note(from), self.note(to)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(message), _) | (_, Err(message)) => return refusal(message),
        };
        if a == b {
            return refusal("Those are the same note.".to_string());
        }
        let db = self.db;
        let cap_chars = self.budget.result_chars;
        let done = db.with(|conn| {
            let mut adjacent: HashMap<String, Vec<(String, graph::Edge)>> = HashMap::new();
            for edge in graph::edges(conn)? {
                adjacent.entry(edge.src.clone()).or_default().push((edge.dst.clone(), edge.clone()));
                adjacent.entry(edge.dst.clone()).or_default().push((edge.src.clone(), edge));
            }
            // Cheapest chain, where a link costs 1 plus how far it is from perfect: fewer links
            // and stronger links both win. Costs are whole numbers so they can be ordered.
            let cost = |edge: &graph::Edge| 1_000 + ((1.0 - edge.weight.clamp(0.0, 1.0)) * 1_000.0) as u64;
            let mut best: HashMap<String, (u64, usize)> = HashMap::from([(a.clone(), (0, 0))]);
            let mut came_from: HashMap<String, (String, graph::Edge)> = HashMap::new();
            let mut queue = BinaryHeap::from([Reverse((0u64, 0usize, a.clone()))]);
            while let Some(Reverse((spent, hops, here))) = queue.pop() {
                if here == b {
                    break;
                }
                if best.get(&here).is_some_and(|(known, _)| *known < spent) || hops == PATH_HOPS {
                    continue;
                }
                for (next, edge) in adjacent.get(&here).into_iter().flatten() {
                    if *next != b && self.is_secret(conn, next) {
                        continue;
                    }
                    let total = spent + cost(edge);
                    if best.get(next).is_none_or(|(known, _)| total < *known) {
                        best.insert(next.clone(), (total, hops + 1));
                        came_from.insert(next.clone(), (here.clone(), edge.clone()));
                        queue.push(Reverse((total, hops + 1, next.clone())));
                    }
                }
            }
            if !came_from.contains_key(&b) {
                return Ok(None);
            }
            let mut chain = Vec::new();
            let mut at = b.clone();
            while let Some((before, edge)) = came_from.get(&at) {
                chain.push((before.clone(), at.clone(), edge.clone()));
                at = before.clone();
            }
            chain.reverse();
            let name_of = |this: &mut Self, id: &str| -> Result<String> {
                let title = items::get(conn, id)?.map_or_else(|| "a deleted note".to_string(), |item| one_line(&item.title));
                Ok(format!("{} “{title}”", this.mem.notes.name(id)))
            };
            let mut lines = Vec::new();
            let mut shown = Vec::new();
            let mut weakest: Option<(String, graph::Edge)> = None;
            for (before, after, edge) in &chain {
                let (x, y) = (name_of(self, before)?, name_of(self, after)?);
                let reason = edge.reason.as_deref().map(one_line).unwrap_or_default();
                lines.push(format!("{x} — {y} [{} {:?}]: {reason}", edge.kind, edge.tier));
                shown.extend([before.clone(), after.clone()]);
                if weakest.as_ref().is_none_or(|(_, w)| edge.weight < w.weight) {
                    weakest = Some((format!("{x} — {y}"), edge.clone()));
                }
            }
            shown.dedup();
            let head = format!("{} links connect {} to {}:", chain.len(), name_of(self, &a)?, name_of(self, &b)?);
            let tail = weakest.map(|(link, edge)| format!("The weakest link is {link} ({:?}).", edge.tier)).unwrap_or_default();
            let label = format!("looked for how “{}” and “{}” connect", title_of(conn, &a), title_of(conn, &b));
            Ok(Some((format!("{head}\n{}\n{tail}", lines.join("\n")), shown, label)))
        });
        match done {
            Ok(Some((text, shown, label))) => ToolOutput { text: cap(&text, cap_chars), shown, read: Vec::new(), label: Some(label) },
            Ok(None) => refusal(format!("No chain of links of up to {PATH_HOPS} steps joins {from} and {to} (links through private notes don't count).")),
            Err(e) => refusal(format!("Looking for a path failed: {e}")),
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
        let cap_chars = self.budget.result_chars;
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
                return Ok((cap(&lines.join("\n"), cap_chars), Vec::new()));
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
            Ok((cap(&text, cap_chars), shown))
        });
        match done {
            Ok((text, shown)) => ToolOutput { text, shown, read: Vec::new(), label: None },
            Err(e) => refusal(format!("Looking up topics failed: {e}")),
        }
    }
}

impl Tools for LibraryTools<'_> {
    fn run(&mut self, call: &Call) -> ToolOutput {
        match call {
            Call::Search { query, mode, scope } => self.search(query, *mode, scope),
            Call::Read { item, offset, limit } => self.read(item, *offset, *limit),
            Call::Neighbors { item, depth, limit } => self.neighbors(item, *depth, *limit),
            Call::Path { from, to } => self.path(from, to),
            Call::Topics { topic } => self.topics(topic),
            Call::Answer { .. } => ToolOutput::default(),
        }
    }
}

fn title_of(conn: &Connection, id: &str) -> String {
    items::get(conn, id).ok().flatten().map_or_else(|| "a deleted note".to_string(), |item| one_line(&item.title))
}

fn refusal(message: String) -> ToolOutput {
    ToolOutput { text: message, ..ToolOutput::default() }
}

fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}\n[cut: too long to show]", truncate(text, max))
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
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
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
        let mut tools = LibraryTools::new(&db, &finder, Some("b-kitchen"), Budget::default());
        let out = tools.run(&search("anything", Mode::Hybrid));
        assert_eq!(out.shown, ["i3"]);
        assert!(out.text.contains("Soup"));
        assert!(!out.text.contains("Ownership"));
    }

    #[test]
    fn a_search_says_how_many_more_matches_there_are() {
        let (_dir, db) = library();
        let many: Vec<String> = (0..12).map(|n| format!("m{n}")).collect();
        db.with(|conn| {
            for id in &many {
                items::upsert(conn, &item(id, &format!("Note {id}"), None), "plain text")?;
            }
            Ok(())
        })
        .unwrap();
        let finder = Fixed(many);
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
        let out = tools.run(&search("note", Mode::Hybrid));
        assert_eq!(out.shown.len(), Budget::default().results);
        assert!(out.text.contains("and 4 more matches not shown"));
    }

    #[test]
    fn a_topic_the_model_was_shown_narrows_the_search() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
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
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
        let out = tools.run(&Call::Read { item: "n7".into(), offset: 0, limit: 0 });
        assert!(out.text.contains("isn't a note you were shown"));
        assert!(out.read.is_empty());
        let out = tools.run(&Call::Neighbors { item: "i1".into(), depth: 1, limit: 0 });
        assert!(out.text.contains("isn't a note you were shown"));
    }

    #[test]
    fn a_secret_cannot_be_found_read_or_walked_to() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
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
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
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
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
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
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
        tools.run(&search("owner", Mode::Hybrid));
        let own = tools.mem.notes.name("i1");
        let out = tools.run(&Call::Neighbors { item: own, depth: 1, limit: 0 });
        assert_eq!(out.shown, ["i2"]);
        assert!(out.text.contains("the checker applies the rules"));
        assert!(out.text.contains("extends"));
    }

    /// Notes `a`, `b`… joined by the given (from, to, weight) links.
    fn chain(db: &Db, ids: &[&str], links: &[(&str, &str, f64)]) {
        db.with(|conn| {
            for id in ids {
                items::upsert(conn, &item(id, &format!("Note {id}"), None), "plain text")?;
            }
            for (a, b, weight) in links {
                let tier = if *weight >= 0.72 { "strong" } else if *weight >= 0.67 { "related" } else { "maybe" };
                conn.execute(
                    "INSERT INTO edges (src, dst, kind, weight, reason, provenance, created_at, tier)
                     VALUES (?1, ?2, 'similar', ?3, ?4, 'embedding', '2026-10-06T00:00:00Z', ?5)",
                    rusqlite::params![a, b, weight, format!("{a} and {b} are alike"), tier],
                )?;
            }
            Ok(())
        })
        .unwrap();
    }

    fn show(tools: &mut LibraryTools, ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| tools.mem.notes.name(id)).collect()
    }

    #[test]
    fn a_path_finds_the_chain_even_when_its_first_link_is_weak() {
        let (_dir, db) = library();
        chain(&db, &["bread", "jazz", "swing", "other"], &[("bread", "jazz", 0.62), ("jazz", "swing", 0.82), ("bread", "other", 0.70)]);
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
        let names = show(&mut tools, &["bread", "swing"]);
        let out = tools.run(&Call::Path { from: names[0].clone(), to: names[1].clone() });
        assert!(out.text.contains("2 links connect"), "{}", out.text);
        assert!(out.text.contains("Note bread") && out.text.contains("Note jazz") && out.text.contains("Note swing"));
        assert!(out.text.contains("bread and jazz are alike"));
        assert!(out.text.contains("The weakest link is") && out.text.contains("Maybe"));
        assert_eq!(out.shown, ["bread", "jazz", "swing"]);
        assert_eq!(out.label.as_deref(), Some("looked for how “Note bread” and “Note swing” connect"));
    }

    #[test]
    fn a_stronger_longer_chain_can_beat_a_weak_shortcut_but_not_by_much() {
        let (_dir, db) = library();
        // A direct weak link, or two strong ones: the direct link is one step, so it wins.
        chain(&db, &["p", "q", "r"], &[("p", "r", 0.62), ("p", "q", 0.90), ("q", "r", 0.90)]);
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
        let names = show(&mut tools, &["p", "r"]);
        let out = tools.run(&Call::Path { from: names[0].clone(), to: names[1].clone() });
        assert!(out.text.contains("1 links connect"), "{}", out.text);
    }

    #[test]
    fn notes_with_no_chain_between_them_or_only_one_through_a_secret_are_reported_as_unconnected() {
        let (_dir, db) = library();
        chain(&db, &["m", "n", "lonely"], &[("m", "n", 0.8)]);
        db.with(|conn| {
            items::upsert(conn, &item("leaky", "Deploy keys", None), "token = ghp_abcdefghijklmnopqrstuvwxyz0123456789")?;
            items::upsert(conn, &item("end", "Far end", None), "plain")?;
            conn.execute(
                "INSERT INTO edges (src, dst, kind, weight, reason, provenance, created_at, tier) VALUES
                 ('n', 'leaky', 'similar', 0.8, 'x', 'embedding', '2026-10-06T00:00:00Z', 'strong'),
                 ('leaky', 'end', 'similar', 0.8, 'y', 'embedding', '2026-10-06T00:00:00Z', 'strong')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
        let names = show(&mut tools, &["m", "lonely", "end"]);
        let apart = tools.run(&Call::Path { from: names[0].clone(), to: names[1].clone() });
        assert!(apart.text.contains("No chain of links"), "{}", apart.text);
        let through_a_secret = tools.run(&Call::Path { from: names[0].clone(), to: names[2].clone() });
        assert!(through_a_secret.text.contains("No chain of links"), "{}", through_a_secret.text);
        assert!(!through_a_secret.text.contains("Deploy keys"));
        let unknown = tools.run(&Call::Path { from: "n99".into(), to: names[0].clone() });
        assert!(unknown.text.contains("isn't a note you were shown"));
    }

    #[test]
    fn neighbors_says_how_many_links_it_left_out_and_can_list_more() {
        let (_dir, db) = library();
        let spokes: Vec<String> = (0..15).map(|n| format!("s{n:02}")).collect();
        let ids: Vec<&str> = std::iter::once("hub").chain(spokes.iter().map(String::as_str)).collect();
        let links: Vec<(&str, &str, f64)> = spokes.iter().enumerate().map(|(n, s)| ("hub", s.as_str(), 0.90 - n as f64 * 0.01)).collect();
        chain(&db, &ids, &links);
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
        let hub = show(&mut tools, &["hub"]).remove(0);
        let few = tools.run(&Call::Neighbors { item: hub.clone(), depth: 1, limit: 0 });
        assert_eq!(few.shown.len(), 12);
        assert!(few.text.contains("and 3 more links not shown"), "{}", few.text);
        let all = tools.run(&Call::Neighbors { item: hub, depth: 1, limit: 30 });
        assert_eq!(all.shown.len(), 15);
        assert!(!all.text.contains("more links not shown"));
    }

    #[test]
    fn topics_list_the_top_level_then_open_up() {
        let (_dir, db) = library();
        let finder = all_ids();
        let mut tools = LibraryTools::new(&db, &finder, None, Budget::default());
        let top = tools.run(&Call::Topics { topic: None });
        assert!(top.text.contains("Rust (2 notes): About Rust"));
        assert!(top.text.contains("Cooking (1 notes)"));
        let inside = tools.run(&Call::Topics { topic: Some("t1".into()) });
        assert_eq!(inside.shown, ["i1", "i2"]);
        assert!(inside.text.contains("n1 | Ownership"));
    }

    #[test]
    fn nothing_comes_back_larger_than_the_cap() {
        let big = "x".repeat(12_000);
        assert!(cap(&big, 6_000).chars().count() < 6_050);
        assert_eq!(cap("short", 6_000), "short");
    }
}
