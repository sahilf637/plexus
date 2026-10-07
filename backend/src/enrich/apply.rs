//! Checking the provider's answer and keeping what is good: the title (in the note), the summary,
//! concepts and links (in the index). Nothing the answer says is trusted: links may only point
//! at candidates that were shown, concepts are cleaned and merged with existing ones, and text
//! is cut to size.

use std::collections::HashSet;
use std::fs;
use std::sync::Mutex;

use serde::Deserialize;

use super::context::Context;
use super::VERSION;
use crate::capture::detect::{self, Kind, TITLE_MAX, truncate};
use crate::db::concepts::{self, Concept, Enriched};
use crate::db::graph::{self, AiEdge};
use crate::db::items::{self, Item};
use crate::db::Db;
use crate::error::{Error, Result};
use crate::extract::split_body;
use crate::library::Library;
use crate::library::note::Note;
use crate::library::reconcile::{item_from_note, modified_nanos};
use crate::pipeline::{embed::Embedder, link};

const MAX_CONCEPTS: usize = 7;
const MAX_CONCEPT_CHARS: usize = 40;
const MAX_LINKS: usize = 8;
const MAX_SUMMARY_CHARS: usize = 220;
const MAX_REASON_CHARS: usize = 160;
/// Links the model isn't at least this sure about are dropped.
const MIN_CONFIDENCE: f64 = 0.4;
/// A new concept this close to an existing one is the same idea, written differently.
const SAME_CONCEPT: f32 = 0.9;

#[derive(Debug, Default, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub concepts: Vec<ConceptPick>,
    #[serde(default)]
    pub links: Vec<LinkPick>,
}

#[derive(Debug, Deserialize)]
pub struct ConceptPick {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct LinkPick {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub relation: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub confidence: f64,
}

/// What changed, for the windows.
pub struct Applied {
    /// The item as it is now.
    pub item: Item,
    pub title_changed: bool,
    pub concepts: Vec<Concept>,
    /// Items whose links changed (this one and the ones it links to).
    pub linked: Vec<String>,
}

pub fn apply(
    library: &Library,
    db: &Db,
    embedder: &Embedder,
    item: &Item,
    context: &Context,
    answer: Answer,
    content_hash: String,
) -> Result<Applied> {
    let previous = db.with(|conn| concepts::enriched(conn, &item.id))?;
    let last_ai_title = previous.and_then(|p| p.title);

    let title = clean_title(&answer.title);
    let renamed = match &title {
        Some(title) => retitle(library, db, item, title, last_ai_title.as_deref())?,
        None => None,
    };
    let title_changed = renamed.is_some();

    let summary = clean_line(&answer.summary, MAX_SUMMARY_CHARS).filter(|s| !s.is_empty());
    let picked = resolve_concepts(db, embedder, context, &answer.concepts)?;
    let edges = valid_links(context, &answer.links);

    let mut linked = db.with(|conn| {
        concepts::set_summary(conn, &item.id, summary.as_deref())?;
        concepts::set_for_item(conn, &item.id, &picked.iter().enumerate().map(|(i, c)| (c.id, 1.0 - 0.1 * i as f32)).collect::<Vec<_>>())?;
        let touched = graph::replace_ai_edges(conn, &item.id, &edges, VERSION)?;
        concepts::record_enriched(
            conn,
            &item.id,
            &Enriched { content_hash, version: VERSION, title: title.clone().or(last_ai_title) },
        )?;
        Ok(touched)
    })?;
    linked.push(item.id.clone());
    linked.sort();
    linked.dedup();

    let item = db.with(|conn| items::get(conn, &item.id))?.unwrap_or_else(|| item.clone());
    Ok(Applied { item, title_changed, concepts: picked, linked })
}

// --- title -------------------------------------------------------------------------------

/// Held while a title is read, changed and written, so the AI and the user can't overwrite each
/// other: whichever comes second sees the first one's result, and the user's always stays.
static TITLE_LOCK: Mutex<()> = Mutex::new(());

/// Replaces the item's title with the AI's when it is Plexus's own guess (see
/// [`may_replace_title`]); the old one is kept in the note. Returns the updated item when it
/// changed. The item is looked up again under the lock: the user may have renamed it (and so
/// moved its file) while the AI was thinking.
fn retitle(library: &Library, db: &Db, item: &Item, title: &str, last_ai_title: Option<&str>) -> Result<Option<Item>> {
    let _held = TITLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(item) = db.with(|conn| items::get(conn, &item.id))? else { return Ok(None) };
    let mut note = library.read_note(&library.absolute(&item.path))?;
    if title == note.meta.title || !may_replace_title(&note, last_ai_title) {
        return Ok(None);
    }
    if note.meta.title_by.as_deref() != Some("ai") {
        note.meta.title_was = Some(note.meta.title.clone());
    }
    note.meta.title_by = Some("ai".into());
    note.meta.title = title.to_string();
    write_title(library, db, &item, note).map(Some)
}

/// Whether the title is still one Plexus made: not changed by the user, and not reverted.
fn may_replace_title(note: &Note, last_ai_title: Option<&str>) -> bool {
    match note.meta.title_by.as_deref() {
        Some("you") => false,
        // An AI title the user then edited by hand stays.
        Some("ai") => last_ai_title == Some(note.meta.title.as_str()),
        _ => match note.meta.kind {
            // A pasted note's own title is its first line; anything else was typed by hand.
            Kind::Text => note.meta.title == detect::detect_text(split_body(&note.body).0).title,
            _ => true,
        },
    }
}

/// Sets the title by hand. From then on it is the user's: the AI never replaces it, even if its
/// answer was already on the way. Returns the updated item when the title changed.
pub fn set_title(library: &Library, db: &Db, id: &str, title: &str) -> Result<Option<Item>> {
    let title = truncate(&title.split_whitespace().collect::<Vec<_>>().join(" "), TITLE_MAX);
    if title.is_empty() {
        return Err(Error::Invalid("a title needs at least one character".into()));
    }
    let _held = TITLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(item) = db.with(|conn| items::get(conn, id))? else { return Ok(None) };
    let mut note = library.read_note(&library.absolute(&item.path))?;
    let already = note.meta.title_by.as_deref() == Some("you");
    if title == note.meta.title && already {
        return Ok(None);
    }
    note.meta.title = title;
    note.meta.title_by = Some("you".into());
    note.meta.title_was = None;
    write_title(library, db, &item, note).map(Some)
}

/// Writes the note with its new title and keeps the index in step. The file is renamed to match
/// unless it has originals next to it, which share its name.
fn write_title(library: &Library, db: &Db, item: &Item, note: Note) -> Result<Item> {
    let old_path = library.absolute(&item.path);
    let path = if note.meta.asset.is_some() || !note.meta.attachments.is_empty() {
        old_path.clone()
    } else {
        library.path_for(&note.meta)
    };
    library.write_note(&path, &note)?;
    if path != old_path {
        fs::remove_file(&old_path)?;
    }
    let stat = fs::metadata(&path)?;
    let updated = item_from_note(library, &path, &note, modified_nanos(&stat), stat.len() as i64);
    db.with(|conn| {
        let status = items::get(conn, &item.id)?.map_or(item.status, |current| current.status);
        items::upsert(conn, &updated, &note.body)?;
        // The body is the same, so it needn't be linked again.
        items::set_status(conn, &item.id, status, link::PIPELINE_VERSION)?;
        Ok(items::get(conn, &item.id)?.unwrap_or(updated))
    })
}

/// One line, no quotes or label, cut to the title length. `None` when nothing usable is left.
fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.strip_prefix("Title:").unwrap_or(line);
    let line = line.trim().trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '“' | '”' | '*' | '#')).trim();
    let line = line.trim_end_matches('.');
    (line.chars().filter(|c| c.is_alphanumeric()).count() >= 3).then(|| truncate(line, TITLE_MAX))
}

fn clean_line(raw: &str, max: usize) -> Option<String> {
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (!line.is_empty()).then(|| truncate(&line, max))
}

// --- concepts ----------------------------------------------------------------------------

/// The concepts the answer picked, as stored ones: reused by id when the model named an existing
/// concept, matched by name or alias, merged when a new name means the same as one we have, and
/// created otherwise.
fn resolve_concepts(db: &Db, embedder: &Embedder, context: &Context, picks: &[ConceptPick]) -> Result<Vec<Concept>> {
    // Names to look up: (the pick's id if it named one we showed, the cleaned name).
    let mut wanted: Vec<(Option<i64>, String)> = Vec::new();
    let mut seen = HashSet::new();
    for pick in picks {
        let shown = pick.id.as_deref().and_then(|id| context.concepts.get(id)).copied();
        let name = clean_concept(&pick.name);
        if shown.is_none() && name.is_none() {
            continue;
        }
        let key = shown.map(|id| format!("#{id}")).or_else(|| name.as_ref().map(|n| n.to_lowercase())).unwrap_or_default();
        if seen.insert(key) {
            wanted.push((shown, name.unwrap_or_default()));
        }
        if wanted.len() >= MAX_CONCEPTS {
            break;
        }
    }

    // New names are embedded (outside the database lock) to spot ones we already have.
    let unknown: Vec<String> = db.with(|conn| {
        let mut unknown = Vec::new();
        for (shown, name) in &wanted {
            if shown.is_none() && concepts::find(conn, name)?.is_none() {
                unknown.push(name.clone());
            }
        }
        Ok(unknown)
    })?;
    let vectors: Vec<Vec<f32>> = match embedder.available() && !unknown.is_empty() {
        true => embedder.embed_passages(&unknown)?,
        false => Vec::new(),
    };

    db.with(|conn| {
        let existing = if vectors.is_empty() { Vec::new() } else { concepts::all_embedded(conn)? };
        let mut out: Vec<Concept> = Vec::new();
        for (shown, name) in wanted {
            let concept = match shown {
                Some(id) => {
                    let Some(concept) = concepts::get(conn, id)? else { continue };
                    if !name.is_empty() && !name.eq_ignore_ascii_case(&concept.name) {
                        concepts::add_alias(conn, concept.id, &name)?;
                    }
                    concept
                }
                None => match concepts::find(conn, &name)? {
                    Some(concept) => concept,
                    None => {
                        let vector = unknown.iter().position(|n| *n == name).and_then(|i| vectors.get(i));
                        let same = vector.and_then(|v| {
                            existing
                                .iter()
                                .map(|(c, e)| (c, concepts::similarity(v, e)))
                                .filter(|(_, s)| *s >= SAME_CONCEPT)
                                .max_by(|a, b| a.1.total_cmp(&b.1))
                        });
                        match same {
                            Some((concept, _)) => {
                                concepts::add_alias(conn, concept.id, &name)?;
                                concept.clone()
                            }
                            None => concepts::create(conn, &name, vector.map_or(&[][..], Vec::as_slice))?,
                        }
                    }
                },
            };
            if !out.iter().any(|c| c.id == concept.id) {
                out.push(concept);
            }
        }
        Ok(out)
    })
}

/// A short noun phrase: whitespace collapsed, surrounding punctuation gone, cut to size.
fn clean_concept(raw: &str) -> Option<String> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.trim_matches(|c: char| !c.is_alphanumeric() && c != '+' && c != '#');
    (name.chars().any(char::is_alphanumeric) && name.chars().count() <= MAX_CONCEPT_CHARS).then(|| name.to_string())
}

// --- links -------------------------------------------------------------------------------

/// The links that point at a candidate that was shown, are sure enough, and have a reason.
fn valid_links(context: &Context, picks: &[LinkPick]) -> Vec<AiEdge> {
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    for pick in picks {
        let Some(other) = context.candidates.get(pick.id.trim()) else { continue };
        let Some(reason) = clean_line(&pick.reason, MAX_REASON_CHARS) else { continue };
        if pick.confidence < MIN_CONFIDENCE || !seen.insert(other.clone()) {
            continue;
        }
        let kind = if graph::is_ai_kind(&pick.relation) { pick.relation.clone() } else { "related".to_string() };
        edges.push(AiEdge { other: other.clone(), kind, reason, confidence: pick.confidence.min(1.0) });
        if edges.len() >= MAX_LINKS {
            break;
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::capture::save;

    fn setup() -> (tempfile::TempDir, Library, Db) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        (dir, library, db)
    }

    fn context(candidates: &[(&str, &str)], concepts: &[(&str, i64)]) -> Context {
        Context {
            user: String::new(),
            candidates: candidates.iter().map(|(a, id)| (a.to_string(), id.to_string())).collect(),
            concepts: concepts.iter().map(|(a, id)| (a.to_string(), *id)).collect::<HashMap<_, _>>(),
        }
    }

    fn answer(json: serde_json::Value) -> Answer {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean_title("\"Why steam matters.\"").as_deref(), Some("Why steam matters"));
        assert_eq!(clean_title("Title: Rust ownership\nextra").as_deref(), Some("Rust ownership"));
        assert_eq!(clean_title("**  **"), None);
        assert!(clean_title(&"word ".repeat(40)).unwrap().chars().count() <= TITLE_MAX + 1);
        assert_eq!(clean_concept("  Borrow   checker. ").as_deref(), Some("Borrow   checker".split_whitespace().collect::<Vec<_>>().join(" ").as_str()));
        assert_eq!(clean_concept("!!!"), None);
        assert_eq!(clean_concept(&"x".repeat(60)), None);
    }

    #[test]
    fn enriches_an_item_and_keeps_only_valid_answers() {
        let (_dir, library, db) = setup();
        let embedder = Embedder::new(None);
        let other = save::text(&library, &db, "Each value in Rust has one owner.", false).unwrap().item.unwrap();
        let item = save::text(&library, &db, "meeting notes about the borrow checker and lifetimes", false).unwrap().item.unwrap();
        db.with(|conn| items::set_status(conn, &item.id, items::Status::Linked, 4)).unwrap();
        let existing = db.with(|conn| concepts::create(conn, "ownership", &[])).unwrap();

        let context = context(&[("c1", &other.id)], &[("k1", existing.id)]);
        let reply = answer(serde_json::json!({
            "title": "How the borrow checker works",
            "summary": "  Notes on   borrowing and lifetimes in Rust. ",
            "concepts": [
                { "id": "k1", "name": "Ownership model" },
                { "id": null, "name": "lifetimes" },
                { "id": "k9", "name": "" },
                { "id": null, "name": "Lifetimes" }
            ],
            "links": [
                { "id": "c1", "relation": "extends", "reason": "Goes deeper into ownership.", "confidence": 0.9 },
                { "id": "c7", "relation": "supports", "reason": "made up", "confidence": 0.9 },
                { "id": "c1", "relation": "related", "reason": "duplicate", "confidence": 0.9 }
            ]
        }));
        let applied = apply(&library, &db, &embedder, &item, &context, reply, "hash-1".into()).unwrap();

        assert!(applied.title_changed);
        assert_eq!(applied.item.title, "How the borrow checker works");
        assert_eq!(applied.item.summary.as_deref(), Some("Notes on borrowing and lifetimes in Rust."));
        assert_eq!(applied.item.status, items::Status::Linked, "an AI title doesn't send the item back to be linked");
        let names: Vec<&str> = applied.concepts.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["ownership", "lifetimes"], "reused by id (with the new wording kept as an alias), de-duplicated");
        assert_eq!(db.with(|conn| concepts::find(conn, "Ownership model")).unwrap().map(|c| c.id), Some(existing.id));
        assert_eq!(applied.linked, {
            let mut ids = vec![item.id.clone(), other.id.clone()];
            ids.sort();
            ids
        });

        // Only the link to a candidate that was shown survives, with its type and reason.
        let edges = db.with(|conn| graph::edges_of(conn, &item.id)).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!((edges[0].kind.as_str(), edges[0].src.as_str()), ("extends", item.id.as_str()));
        assert_eq!(edges[0].reason.as_deref(), Some("Goes deeper into ownership."));

        // The note holds the new title (and the old one), and was renamed to match.
        let note = library.read_note(&library.absolute(&applied.item.path)).unwrap();
        assert_eq!((note.meta.title_by.as_deref(), note.meta.title_was.as_deref()), (Some("ai"), Some("meeting notes about the borrow checker and lifetimes")));
        assert!(applied.item.path.contains("how-the-borrow-checker-works") && !library.absolute(&item.path).exists());
        assert_eq!(db.with(|conn| concepts::enriched(conn, &item.id)).unwrap().unwrap().title.as_deref(), Some("How the borrow checker works"));

        // Setting the title by hand keeps the AI from changing it again.
        let edited = set_title(&library, &db, &item.id, "  My   own title ").unwrap().unwrap();
        assert_eq!(edited.title, "My own title");
        let again = answer(serde_json::json!({ "title": "Another title", "summary": "x", "concepts": [], "links": [] }));
        let applied = apply(&library, &db, &embedder, &item, &context, again, "hash-2".into()).unwrap();
        assert!(!applied.title_changed);
        assert_eq!(applied.item.title, "My own title");
        assert!(applied.concepts.is_empty());
        assert_eq!(db.with(|conn| concepts::of_item(conn, &item.id)).unwrap(), vec![]);
    }

    #[test]
    fn a_title_the_user_wrote_is_left_alone() {
        let (_dir, library, db) = setup();
        let embedder = Embedder::new(None);
        let item = save::text(&library, &db, "first line of a pasted note\nand more", false).unwrap().item.unwrap();
        // Editing the title in the note file by hand.
        let path = library.absolute(&item.path);
        let mut note = library.read_note(&path).unwrap();
        note.meta.title = "My own title".into();
        library.write_note(&path, &note).unwrap();

        let reply = answer(serde_json::json!({ "title": "An AI title", "summary": "s", "concepts": [], "links": [] }));
        let applied = apply(&library, &db, &embedder, &item, &context(&[], &[]), reply, "h".into()).unwrap();
        assert!(!applied.title_changed);
        assert_eq!(library.read_note(&path).unwrap().meta.title, "My own title");
    }
}
