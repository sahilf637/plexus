//! Topics (M3): a tree of groups built from the links, so a big library can be read at a glance.
//!
//! Grouping is done here, locally, and never by the AI (which only names groups, see `ai`). A
//! new item joins an existing topic straight away; the whole tree is only rebuilt when the
//! library has grown a good deal, and a rebuild carries each topic's identity and name over, so
//! the map doesn't rearrange itself under the user.

pub mod ai;
pub mod cluster;
pub mod graph;
pub mod matching;
pub mod names;
#[cfg(test)]
pub(crate) mod tune;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::db::items;
use crate::db::topics::{self as store, LabelBy, Topic};
use crate::db::{chunks, concepts};
use crate::error::Result;
use crate::state::AppState;
use cluster::Params;
use matching::Candidate;
use names::{ItemWords, ToName};

/// Bumped when the grouping rules change; the tree is then built again.
pub const VERSION: i64 = 1;
/// Fewer items than this have no topics: they would only add noise.
pub const MIN_ITEMS: usize = 12;
/// The topic for items that relate to nothing yet.
pub const LOOSE_ID: &str = "loose";
const LOOSE_LABEL: &str = "Loose notes";

/// How much the library must grow, or change under the AI, before the tree is rebuilt.
const GROWTH_MIN: usize = 50;
const GROWTH_FRACTION: usize = 4; // a quarter
const GROWTH_EVERY: i64 = 3600;
const CHANGES_MIN: u32 = 25;
const CHANGES_EVERY: i64 = 1800;
/// A rebuild waits for things to go quiet this long (nothing new asking for one), so a burst of
/// notes being linked is one rebuild, but starts anyway after `MAX_WAIT`.
const QUIET: Duration = Duration::from_secs(8);
const MAX_WAIT: Duration = Duration::from_secs(120);
/// A rebuild asked for by the user starts after this.
const ASKED: Duration = Duration::from_secs(1);
/// Until the tree holds this many items, growth rebuilds may come this often (seconds).
const SMALL_TREE: usize = 100;
const SMALL_TREE_EVERY: i64 = 120;
/// An item's closest topic by meaning must be at least this alike, else it goes to Loose notes.
const MIN_CENTRE_SIMILARITY: f32 = 0.3;
/// Characters of an item's text its name-finding terms are taken from.
const TERM_CHARS: usize = 800;

const BUILT_ITEMS: &str = "topics_built_items";
const BUILT_AT: &str = "topics_built_at";
const BUILT_VERSION: &str = "topics_version";
const CHANGES: &str = "topics_changes";

pub struct Topics {
    /// A rebuild is waiting or running.
    busy: AtomicBool,
    /// When a rebuild was last asked for, and how long things must be quiet before it starts.
    asked: std::sync::Mutex<(std::time::Instant, Duration)>,
}

impl Default for Topics {
    fn default() -> Self {
        Self { busy: AtomicBool::new(false), asked: std::sync::Mutex::new((std::time::Instant::now(), QUIET)) }
    }
}

/// A topic as the map sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicInfo {
    pub id: String,
    pub parent_id: Option<String>,
    pub level: i64,
    pub label: String,
    pub label_by: LabelBy,
    pub summary: Option<String>,
    /// Items in it, counting those in its sub-topics.
    pub count: usize,
}

/// The whole tree and each item's lowest topic.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicsData {
    pub topics: Vec<TopicInfo>,
    pub item_topic: HashMap<String, String>,
}

/// Sent when an item has joined a topic.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemTopic {
    id: String,
    topic_id: String,
}

pub fn data(conn: &Connection) -> Result<TopicsData> {
    let topics = store::all(conn)?;
    let membership = store::memberships(conn)?;
    let members = store::members_with_descendants(&topics, &membership);
    let info = topics
        .into_iter()
        .map(|t| TopicInfo {
            count: members.get(&t.id).map_or(0, HashSet::len),
            id: t.id,
            parent_id: t.parent_id,
            level: t.level,
            label: t.label,
            label_by: t.label_by,
            summary: t.summary,
        })
        .filter(|t| t.count > 0)
        .collect();
    Ok(TopicsData { topics: info, item_topic: membership })
}

// --- building the tree -------------------------------------------------------------------

/// What a rebuild reads from the index, so the computation itself can run without holding it.
struct Inputs {
    graph: graph::TopicGraph,
    vectors: HashMap<String, Vec<f32>>,
    words: HashMap<String, ItemWords>,
    old_topics: Vec<Topic>,
    old_membership: HashMap<String, String>,
}

fn read_inputs(conn: &Connection) -> Result<Inputs> {
    let graph = graph::build(conn)?;
    let vectors = chunks::item_vectors(conn)?;
    let titles: HashMap<String, String> = items::all(conn)?.into_iter().map(|i| (i.id, i.title)).collect();
    let mut words = HashMap::new();
    for id in &graph.ids {
        let text = chunks::text_of(conn, id, TERM_CHARS)?;
        let title = titles.get(id).map(String::as_str).unwrap_or_default();
        words.insert(id.clone(), ItemWords::new(title, &text, Vec::new()));
    }
    for (id, found) in concepts::of_items(conn, &graph.ids)? {
        if let Some(w) = words.get_mut(&id) {
            w.concepts = found.into_iter().map(|c| c.name).collect();
        }
    }
    Ok(Inputs { graph, vectors, words, old_topics: store::all(conn)?, old_membership: store::memberships(conn)? })
}

/// A topic before it has an id: where it sits and which items are in it.
struct Draft {
    parent: Option<usize>,
    level: i64,
    /// Items in it and its sub-topics.
    members: Vec<String>,
    /// Items directly in it (only a lowest topic has any).
    own: Vec<String>,
    /// The loose-notes topic.
    loose: bool,
}

fn drafts_of(tree: &cluster::Tree, ids: &[String]) -> Vec<Draft> {
    fn walk(group: &cluster::Group, parent: Option<usize>, level: i64, ids: &[String], out: &mut Vec<Draft>) {
        let at = out.len();
        let members: Vec<String> = group.all_items().into_iter().map(|i| ids[i].clone()).collect();
        let own = group.items.iter().map(|&i| ids[i].clone()).collect();
        out.push(Draft { parent, level, members, own, loose: false });
        for child in &group.children {
            walk(child, Some(at), level + 1, ids, out);
        }
    }
    let mut out = Vec::new();
    for group in &tree.groups {
        walk(group, None, 0, ids, &mut out);
    }
    if !tree.loose.is_empty() {
        let own: Vec<String> = tree.loose.iter().map(|&i| ids[i].clone()).collect();
        out.push(Draft { parent: None, level: 0, members: own.clone(), own, loose: true });
    }
    out
}

/// The tree for this input: topics (with ids carried over from the old ones) and each item's
/// lowest topic.
fn compute(inputs: &Inputs) -> (Vec<Topic>, Vec<(String, String)>) {
    compute_with(inputs, &Params::default())
}

fn compute_with(inputs: &Inputs, params: &Params) -> (Vec<Topic>, Vec<(String, String)>) {
    let tree = cluster::build(&inputs.graph.adj, params);
    let drafts = drafts_of(&tree, &inputs.graph.ids);

    // Which old topic each new one continues.
    let old_members = store::members_with_descendants(&inputs.old_topics, &inputs.old_membership);
    let old: Vec<Candidate<String>> = inputs
        .old_topics
        .iter()
        .filter(|t| t.id != LOOSE_ID)
        .map(|t| Candidate { key: t.id.clone(), level: t.level, members: &old_members[&t.id] })
        .collect();
    let member_sets: Vec<HashSet<String>> = drafts.iter().map(|d| d.members.iter().cloned().collect()).collect();
    let new: Vec<Candidate<usize>> =
        drafts.iter().enumerate().filter(|(_, d)| !d.loose).map(|(i, d)| Candidate { key: i, level: d.level, members: &member_sets[i] }).collect();
    let carried = matching::carry_over(&old, &new);
    let old_by_id: HashMap<&str, &Topic> = inputs.old_topics.iter().map(|t| (t.id.as_str(), t)).collect();

    let ids: Vec<String> = drafts
        .iter()
        .enumerate()
        .map(|(i, d)| if d.loose { LOOSE_ID.to_string() } else { carried.get(&i).cloned().unwrap_or_else(|| ulid::Ulid::new().to_string()) })
        .collect();

    // Names made from the notes' words, for every topic; a name the AI or the user gave is kept.
    let to_name: Vec<ToName> = drafts
        .iter()
        .enumerate()
        .filter(|(_, d)| !d.loose)
        .map(|(i, d)| {
            // Its sub-topics, biggest first.
            let mut children: Vec<usize> = (0..drafts.len()).filter(|&c| drafts[c].parent == Some(i)).collect();
            children.sort_by_key(|&c| (std::cmp::Reverse(drafts[c].members.len()), c));
            ToName {
                id: &ids[i],
                parent: d.parent.map(|p| ids[p].as_str()),
                level: d.level,
                members: &d.members,
                children: children.into_iter().map(|c| ids[c].as_str()).collect(),
            }
        })
        .collect();
    let auto = names::auto_names(&to_name, &inputs.words);

    let mut topics = Vec::new();
    for (i, d) in drafts.iter().enumerate() {
        let id = &ids[i];
        let previous = old_by_id.get(id.as_str());
        let (label, label_by, summary, named_members) = if d.loose {
            (LOOSE_LABEL.to_string(), LabelBy::Auto, None, None)
        } else {
            match previous {
                // Yours stays. The AI's stays too, unless the topic is now quite different.
                Some(p) if p.label_by == LabelBy::You => (p.label.clone(), p.label_by, p.summary.clone(), None),
                Some(p) if p.label_by == LabelBy::Ai && still_fits(p, &member_sets[i]) => {
                    (p.label.clone(), p.label_by, p.summary.clone(), p.named_members.clone())
                }
                _ => (auto.get(id).cloned().unwrap_or_else(|| "Notes".into()), LabelBy::Auto, None, None),
            }
        };
        topics.push(Topic {
            id: id.clone(),
            parent_id: d.parent.map(|p| ids[p].clone()),
            level: d.level,
            label,
            label_by,
            summary,
            named_members,
            centroid: centre(&d.members, &inputs.vectors),
            version: VERSION,
        });
    }
    let members = drafts.iter().enumerate().flat_map(|(i, d)| d.own.iter().map(|item| (item.clone(), ids[i].clone())).collect::<Vec<_>>()).collect();
    (topics, members)
}

/// The AI's name still fits if at least 60% of the topic is what it was named for.
fn still_fits(topic: &Topic, now: &HashSet<String>) -> bool {
    match &topic.named_members {
        Some(named) => matching::overlap(&named.iter().cloned().collect(), now) >= ai::KEEP_NAME_OVERLAP,
        None => true,
    }
}

/// The mean vector of some items, scaled to length 1.
fn centre(members: &[String], vectors: &HashMap<String, Vec<f32>>) -> Option<Vec<f32>> {
    let mut sum: Vec<f32> = Vec::new();
    for id in members {
        let Some(v) = vectors.get(id) else { continue };
        if sum.is_empty() {
            sum = vec![0.0; v.len()];
        }
        if v.len() == sum.len() {
            sum.iter_mut().zip(v).for_each(|(s, x)| *s += x);
        }
    }
    let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
    (norm > 0.0).then(|| sum.iter().map(|x| x / norm).collect())
}

/// Builds the tree again and stores it. Returns whether there are enough items for one.
pub fn rebuild(state: &AppState) -> Result<bool> {
    rebuild_in(&state.db)
}

/// Items that have been linked: what topics are built from.
fn linked_count(conn: &Connection) -> Result<usize> {
    Ok(conn.query_row("SELECT COUNT(*) FROM items WHERE status = 'linked'", [], |row| row.get::<_, i64>(0))? as usize)
}

pub(crate) fn rebuild_in(db: &crate::db::Db) -> Result<bool> {
    let count = db.with(|conn| linked_count(conn))?;
    if count < MIN_ITEMS {
        db.with(|conn| store::clear(conn))?;
        return Ok(false);
    }
    let started = std::time::Instant::now();
    // Read, then work it out without holding the index, then write.
    let inputs = db.with(|conn| read_inputs(conn))?;
    let (topics, members) = compute(&inputs);
    let linked = inputs.graph.ids.len();
    db.with(|conn| {
        store::replace_all(conn, &topics, &members)?;
        chunks::set_meta(conn, BUILT_ITEMS, &count.to_string())?;
        chunks::set_meta(conn, BUILT_AT, &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))?;
        chunks::set_meta(conn, BUILT_VERSION, &VERSION.to_string())?;
        chunks::set_meta(conn, CHANGES, "0")?;
        // Anything linked or saved while this was computed.
        assign_missing(conn)?;
        Ok(())
    })?;
    tracing::info!(items = linked, topics = topics.len(), elapsed = ?started.elapsed(), "topics built");
    Ok(true)
}

// --- new items join a topic --------------------------------------------------------------

/// Puts every item that has no topic into one. Returns how many were placed.
fn assign_missing(conn: &mut Connection) -> Result<usize> {
    if store::all(conn)?.is_empty() {
        return Ok(0);
    }
    let mut stmt = conn.prepare(
        "SELECT id FROM items WHERE status IN ('linked', 'unlinkable') AND id NOT IN (SELECT item_id FROM topic_members) ORDER BY id",
    )?;
    let missing: Vec<String> = stmt.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    for id in &missing {
        assign(conn, id)?;
    }
    Ok(missing.len())
}

/// Puts one item in the topic most of the items it links to are in (weighted by how strong the
/// links are), else the one whose centre is nearest by meaning, else Loose notes. Does nothing
/// while there is no tree. Returns the topic.
pub fn assign(conn: &Connection, item_id: &str) -> Result<Option<String>> {
    let topics = store::all(conn)?;
    if topics.is_empty() {
        return Ok(None);
    }
    let membership = store::memberships(conn)?;
    let mut votes: HashMap<&str, f64> = HashMap::new();
    for edge in crate::db::graph::edges_of(conn, item_id)? {
        let other = if edge.src == item_id { &edge.dst } else { &edge.src };
        if let Some(topic) = membership.get(other) {
            *votes.entry(topic.as_str()).or_insert(0.0) += graph::edge_weight(&edge.kind, edge.tier);
        }
    }
    let mut best: Option<String> = votes
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(topic, _)| topic.to_string());

    if best.is_none()
        && let Some(vector) = chunks::item_vector(conn, item_id)?
    {
        let parents: HashSet<&str> = topics.iter().filter_map(|t| t.parent_id.as_deref()).collect();
        best = topics
            .iter()
            .filter(|t| !parents.contains(t.id.as_str()) && t.id != LOOSE_ID)
            .filter_map(|t| Some((t.id.clone(), concepts::similarity(&vector, t.centroid.as_ref()?))))
            .filter(|(_, s)| *s >= MIN_CENTRE_SIMILARITY)
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .map(|(id, _)| id);
    }
    let target = match best {
        Some(topic) => topic,
        None => {
            ensure_loose(conn, &topics)?;
            LOOSE_ID.to_string()
        }
    };
    store::set_member(conn, item_id, &target)?;
    Ok(Some(target))
}

fn ensure_loose(conn: &Connection, topics: &[Topic]) -> Result<()> {
    if topics.iter().any(|t| t.id == LOOSE_ID) {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO topics (id, parent_id, level, label, label_by, version) VALUES (?1, NULL, 0, ?2, 'auto', ?3)",
        rusqlite::params![LOOSE_ID, LOOSE_LABEL, VERSION],
    )?;
    Ok(())
}

/// Called when an item has been linked (or found unlinkable): it joins a topic, and the tree is
/// rebuilt if it is time.
pub fn on_linked(app: &AppHandle, state: &AppState, id: &str) {
    match state.db.with(|conn| assign(conn, id)) {
        Ok(Some(topic_id)) => {
            let _ = app.emit("item-topic", ItemTopic { id: id.to_string(), topic_id });
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(id, "couldn't place the item in a topic: {e}"),
    }
    note_change(state);
    maybe_rebuild(app, state);
}

/// Counts something that changed what the topics rest on (an item enriched, say).
pub fn note_change(state: &AppState) {
    let result = state.db.with(|conn| {
        let count: u32 = chunks::get_meta(conn, CHANGES)?.and_then(|v| v.parse().ok()).unwrap_or(0);
        chunks::set_meta(conn, CHANGES, &(count + 1).to_string())
    });
    if let Err(e) = result {
        tracing::debug!("couldn't count a change for topics: {e}");
    }
}

/// Removes topics left empty by deleted items.
pub fn forget(state: &AppState) {
    if let Err(e) = state.db.with(|conn| store::prune(conn)) {
        tracing::warn!("couldn't tidy topics: {e}");
    }
}

// --- when to rebuild ---------------------------------------------------------------------

fn seconds_since(conn: &Connection, key: &str) -> Result<Option<i64>> {
    Ok(chunks::get_meta(conn, key)?
        .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
        .map(|at| (Utc::now() - at.with_timezone(&Utc)).num_seconds()))
}

/// Whether the tree should be built again now: it doesn't exist, the rules changed, the library
/// has grown a good deal, or much has changed since (older notes improved, say).
fn due(conn: &Connection) -> Result<bool> {
    let count = linked_count(conn)?;
    if count < MIN_ITEMS {
        return Ok(false);
    }
    let built: Option<usize> = chunks::get_meta(conn, BUILT_ITEMS)?.and_then(|v| v.parse().ok());
    let Some(built) = built else { return Ok(true) };
    if chunks::get_meta(conn, BUILT_VERSION)?.as_deref() != Some(VERSION.to_string().as_str()) {
        return Ok(true);
    }
    let age = seconds_since(conn, BUILT_AT)?.unwrap_or(i64::MAX);
    // A small tree is rebuilt as it grows fairly often; a big one at most once an hour.
    let every = if built < SMALL_TREE { SMALL_TREE_EVERY } else { GROWTH_EVERY };
    if count >= built + GROWTH_MIN.min(built).max(built / GROWTH_FRACTION).max(MIN_ITEMS) && age >= every {
        return Ok(true);
    }
    let changes: u32 = chunks::get_meta(conn, CHANGES)?.and_then(|v| v.parse().ok()).unwrap_or(0);
    Ok(changes >= CHANGES_MIN && age >= CHANGES_EVERY)
}

/// Starts a rebuild in the background if one is due.
pub fn maybe_rebuild(app: &AppHandle, state: &AppState) {
    if state.db.with(|conn| due(conn)).unwrap_or(false) {
        schedule(app);
    }
}

/// Rebuilds in the background once things have been quiet for a while (one at a time), then
/// tells the windows.
pub fn schedule(app: &AppHandle) {
    ask(app, QUIET);
}

/// A rebuild the user asked for: it starts almost at once.
pub fn schedule_now(app: &AppHandle) {
    ask(app, ASKED);
}

fn ask(app: &AppHandle, quiet: Duration) {
    let state = app.state::<AppState>();
    *state.topics.asked.lock().expect("topics lock poisoned") = (std::time::Instant::now(), quiet);
    if state.topics.busy.swap(true, Ordering::AcqRel) {
        return; // Already waiting: it will see this request and wait on.
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let first = std::time::Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let (at, quiet) = *state.topics.asked.lock().expect("topics lock poisoned");
            if at.elapsed() >= quiet || first.elapsed() >= MAX_WAIT {
                break;
            }
        }
        match rebuild(&state) {
            Ok(_) => announce(&app, &state),
            Err(e) => tracing::warn!("couldn't build topics: {e}"),
        }
        state.topics.busy.store(false, Ordering::Release);
    });
}

/// Tells the windows the tree changed, and asks the AI (if connected) to name what needs it.
pub fn announce(app: &AppHandle, state: &AppState) {
    match state.db.with(|conn| data(conn)) {
        Ok(data) => {
            let _ = app.emit("topics-changed", data);
        }
        Err(e) => tracing::warn!("couldn't read topics: {e}"),
    }
    ai::queue_names(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::db::items::{Status, sample};
    use crate::db::chunks::unit;

    /// Two groups of notes: items `a0..` and `b0..`, each linked within its group.
    fn library(conn: &mut Connection, per_group: usize) {
        for group in ["a", "b"] {
            for i in 0..per_group {
                let id = format!("{group}{i}");
                let mut item = sample(&id, &format!("items/{id}.md"), &id);
                item.title = format!("{} note {i}", if group == "a" { "Rust" } else { "Bread" });
                item.status = Status::Linked;
                items::upsert(conn, &item, "").unwrap();
                items::set_status(conn, &id, Status::Linked, 1).unwrap();
                // Each group lies along its own direction.
                let axis = if group == "a" { 0 } else { 1 };
                let mut v = vec![0.0; 3];
                v[axis] = 1.0;
                v[2] = 0.05 * i as f32;
                chunks::replace(conn, &id, std::slice::from_ref(&id), &[unit(&v)]).unwrap();
            }
            for i in 0..per_group {
                for j in (i + 1)..per_group {
                    conn.execute(
                        "INSERT INTO edges (src, dst, kind, weight, tier, provenance, status, pipeline_version, created_at)
                         VALUES (?1, ?2, 'similar', 0.8, 'strong', 'embedding', 'proposed', 1, 't')",
                        [format!("{group}{i}"), format!("{group}{j}")],
                    )
                    .unwrap();
                }
            }
        }
    }

    #[test]
    fn builds_a_tree_and_places_new_items() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        db.with(|conn| {
            library(conn, 8);
            let inputs = read_inputs(conn)?;
            let (topics, members) = compute(&inputs);
            store::replace_all(conn, &topics, &members)?;

            let data = data(conn)?;
            assert_eq!(data.topics.iter().filter(|t| t.level == 0).count(), 2, "two groups, two topics: {:?}", data.topics);
            assert_eq!(data.item_topic.len(), 16);
            let of = |id: &str| data.item_topic[id].clone();
            assert!((1..8).all(|i| of(&format!("a{i}")) == of("a0")), "the first group stays together");
            assert_ne!(of("a0"), of("b0"));
            let rust = data.topics.iter().find(|t| t.id == of("a0")).unwrap();
            assert!(rust.label.to_lowercase().contains("rust"), "named from its notes: {}", rust.label);

            // A new note linked to the second group joins it, one linked to nothing but near the
            // first group by meaning joins that, and an unrelated one goes to Loose notes.
            for id in ["n1", "n2", "n3"] {
                let mut item = sample(id, &format!("items/{id}.md"), id);
                item.status = Status::Linked;
                items::upsert(conn, &item, "")?;
                items::set_status(conn, id, Status::Linked, 1)?;
            }
            conn.execute(
                "INSERT INTO edges (src, dst, kind, weight, tier, provenance, status, pipeline_version, created_at)
                 VALUES ('b0', 'n1', 'similar', 0.8, 'strong', 'embedding', 'proposed', 1, 't')",
                [],
            )?;
            chunks::replace(conn, "n2", &["n2".into()], &[unit(&[0.99, 0.0, 0.1])])?;
            chunks::replace(conn, "n3", &["n3".into()], &[unit(&[0.0, 0.0, 1.0])])?;
            assert_eq!(assign(conn, "n1")?, Some(of("b0")));
            assert_eq!(assign(conn, "n2")?, Some(of("a0")));
            assert_eq!(assign(conn, "n3")?.as_deref(), Some(LOOSE_ID));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_rebuild_keeps_ids_and_names_you_gave() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        db.with(|conn| {
            library(conn, 8);
            let (topics, members) = compute(&read_inputs(conn)?);
            store::replace_all(conn, &topics, &members)?;
            let first = data(conn)?;
            let rust_id = first.item_topic["a0"].clone();
            store::set_label(conn, &rust_id, "My Rust notes", LabelBy::You, None, None)?;

            let (topics, members) = compute(&read_inputs(conn)?);
            store::replace_all(conn, &topics, &members)?;
            let again = data(conn)?;
            assert_eq!(again.item_topic["a0"], rust_id, "the topic keeps its id");
            let topic = again.topics.iter().find(|t| t.id == rust_id).unwrap();
            assert_eq!((topic.label.as_str(), topic.label_by), ("My Rust notes", LabelBy::You));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn rebuilds_when_it_is_time_and_not_before() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        db.with(|conn| {
            // Too few items: no tree, and nothing due.
            library(conn, 3);
            assert!(!due(conn)?);
            Ok(())
        })
        .unwrap();
        assert!(!rebuild_in(&db).unwrap(), "six items are too few for topics");
        db.with(|conn| {
            for i in 3..8 {
                for group in ["a", "b"] {
                    let id = format!("{group}{i}");
                    let mut item = sample(&id, &format!("items/{id}.md"), &id);
                    item.status = Status::Linked;
                    items::upsert(conn, &item, "")?;
                    items::set_status(conn, &id, Status::Linked, 1)?;
                }
            }
            assert!(due(conn)?, "enough items and no tree yet");
            Ok(())
        })
        .unwrap();

        assert!(rebuild_in(&db).unwrap());
        db.with(|conn| {
            assert!(!due(conn)?, "just built");
            let built = chunks::get_meta(conn, BUILT_ITEMS)?;
            assert_eq!(built.as_deref(), Some("16"));
            // Grown a lot, but built a moment ago: wait (growth rebuilds are an hour apart).
            for i in 8..80 {
                let id = format!("x{i}");
                let mut item = sample(&id, &format!("items/{id}.md"), &id);
                item.status = Status::Linked;
                items::upsert(conn, &item, "")?;
            }
            assert!(!due(conn)?);
            // An hour later, it is time.
            let hour_ago = (Utc::now() - chrono::Duration::seconds(3700)).to_rfc3339_opts(SecondsFormat::Secs, true);
            chunks::set_meta(conn, BUILT_AT, &hour_ago)?;
            assert!(due(conn)?);
            // Or when the rules changed.
            chunks::set_meta(conn, BUILT_AT, &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))?;
            conn.execute("DELETE FROM items WHERE id LIKE 'x%'", [])?;
            assert!(!due(conn)?);
            chunks::set_meta(conn, BUILT_VERSION, "0")?;
            assert!(due(conn)?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn many_changes_make_a_rebuild_due_after_half_an_hour() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("index.db")).unwrap();
        db.with(|conn| {
            library(conn, 8);
            Ok(())
        })
        .unwrap();
        assert!(rebuild_in(&db).unwrap());
        db.with(|conn| {
            chunks::set_meta(conn, CHANGES, "30")?;
            assert!(!due(conn)?, "built a moment ago");
            let long_ago = (Utc::now() - chrono::Duration::seconds(2000)).to_rfc3339_opts(SecondsFormat::Secs, true);
            chunks::set_meta(conn, BUILT_AT, &long_ago)?;
            assert!(due(conn)?);
            chunks::set_meta(conn, CHANGES, "3")?;
            assert!(!due(conn)?, "few changes: not worth it");
            Ok(())
        })
        .unwrap();
    }
}
