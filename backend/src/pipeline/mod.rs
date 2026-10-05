//! The background pipeline ("T1"): after an item is saved, chunk it, embed it and link it to
//! related items, then tell the windows. One worker thread handles items in order, so the UI
//! never waits on it.

pub mod chunk;
pub mod embed;
pub mod link;
pub mod search;
#[cfg(test)]
mod tune;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use regex::Regex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::capture::detect::Kind;
use crate::db::items::{self, Item, Status};
use crate::db::{chunks, graph};
use crate::error::Result;
use crate::state::AppState;
use embed::Embedder;

/// Chunks embedded per item (~40k characters), so a book-length page doesn't hold up the queue.
const MAX_CHUNKS: usize = 32;

pub struct Pipeline {
    queue: Sender<String>,
    pub embedder: Arc<Embedder>,
}

/// Sent to the windows when an item has been linked.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemLinked {
    id: String,
    status: Status,
    /// Every link touching this item, now.
    edges: Vec<graph::Edge>,
    /// The linked items' titles, closest first (for the capture window).
    related: Vec<Related>,
}

#[derive(Clone, Debug, Serialize)]
struct Related {
    id: String,
    title: String,
    weight: f32,
}

impl Pipeline {
    /// The pipeline plus the receiving end of its queue, for [`run`].
    pub fn new(embedder: Embedder) -> (Self, Receiver<String>) {
        let (queue, receiver) = mpsc::channel();
        (Self { queue, embedder: Arc::new(embedder) }, receiver)
    }

    pub fn enqueue(&self, id: &str) {
        let _ = self.queue.send(id.to_string());
    }
}

/// Prepares the index for the current model, queues everything still waiting to be linked, then
/// works through the queue until the app exits. Call on its own thread.
pub fn run(app: AppHandle, queue: Receiver<String>) {
    let state = app.state::<AppState>();
    if let Err(e) = prepare(&state) {
        tracing::error!("couldn't prepare the index for linking: {e}");
    }
    loop {
        match queue.recv_timeout(embed::IDLE_UNLOAD) {
            Ok(first) => {
                // Take everything already waiting, once each.
                let mut batch = vec![first];
                batch.extend(queue.try_iter());
                batch.dedup();
                for id in batch {
                    if let Err(e) = process(&app, &state, &id) {
                        tracing::warn!(id, "couldn't link item: {e}");
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => state.pipeline.embedder.unload_if_idle(),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn prepare(state: &AppState) -> Result<()> {
    let (pending, unread) = state.db.with(|conn| {
        if chunks::get_meta(conn, "embed_model")?.as_deref() != Some(embed::MODEL_ID) {
            tracing::info!(model = embed::MODEL_ID, "embedding model changed; re-linking everything");
            chunks::reset_embeddings(conn)?;
            chunks::set_meta(conn, "embed_model", embed::MODEL_ID)?;
        }
        let version = link::PIPELINE_VERSION.to_string();
        if chunks::get_meta(conn, "pipeline_version")?.as_deref() != Some(version.as_str()) {
            tracing::info!(version, "linking rules changed; re-linking everything");
            chunks::reset_links(conn)?;
            chunks::set_meta(conn, "pipeline_version", &version)?;
        }
        Ok((chunks::pending_ids(conn)?, items::unread_links(conn)?))
    })?;
    if !pending.is_empty() {
        tracing::info!(count = pending.len(), unread_links = unread.len(), "linking items");
    }
    // Links whose page hasn't been read go to the fetcher, which links them afterwards.
    for id in pending.iter().filter(|id| !unread.contains(id)) {
        state.pipeline.enqueue(id);
    }
    for id in &unread {
        state.fetcher.enqueue(id);
    }
    Ok(())
}

fn process(app: &AppHandle, state: &AppState, id: &str) -> Result<()> {
    if !state.pipeline.embedder.available() {
        return Ok(()); // Stays pending until a build with the model runs.
    }
    let started = Instant::now();
    let Some(item) = state.db.with(|conn| items::get(conn, id))? else {
        return Ok(()); // Deleted meanwhile.
    };
    let body = state.library.read_note(&state.library.absolute(&item.path))?.body;
    let mut texts = chunk::chunk(&embedding_text(&item, &body));
    // A long page is linked by its first few thousand words; keyword search still has it all.
    texts.truncate(MAX_CHUNKS);

    let (status, links) = if texts.is_empty() {
        (Status::Unlinkable, Vec::new())
    } else {
        let vectors = state.pipeline.embedder.embed_passages(&texts)?;
        let links = state.db.with(|conn| {
            chunks::replace(conn, id, &texts, &vectors)?;
            link::link(conn, id, &vectors)
        })?;
        (Status::Linked, links)
    };

    let event = state.db.with(|conn| {
        items::set_status(conn, id, status, link::PIPELINE_VERSION)?;
        let related = links
            .iter()
            .filter_map(|(other, weight)| {
                let title = items::get(conn, other).ok().flatten()?.title;
                Some(Related { id: other.clone(), title, weight: *weight })
            })
            .collect();
        Ok(ItemLinked { id: id.to_string(), status, edges: graph::edges_of(conn, id)?, related })
    })?;
    tracing::debug!(id, chunks = texts.len(), links = event.related.len(), elapsed = ?started.elapsed(), "item linked");
    let _ = app.emit("item-linked", event);
    Ok(())
}

/// The text that stands for an item when finding related ones.
fn embedding_text(item: &Item, body: &str) -> String {
    static IMAGE_EMBED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap());
    let body = IMAGE_EMBED.replace_all(body, "");
    let body = body.trim();
    match item.kind {
        // Until text is extracted from them (M1), images and files have the user's note, or
        // else only their names.
        Kind::Image | Kind::File if !body.is_empty() => body.to_string(),
        Kind::Image | Kind::File => {
            if item.title.chars().filter(|c| c.is_alphabetic()).count() < 3 || item.title.starts_with("Pasted ") {
                String::new()
            } else {
                item.title.clone()
            }
        }
        _ if body.is_empty() => item.title.clone(),
        _ => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::items::sample;

    #[test]
    fn what_gets_embedded() {
        let mut item = sample("a", "items/a.md", "h");
        assert_eq!(embedding_text(&item, "Some text\n"), "Some text");
        assert_eq!(embedding_text(&item, "  "), item.title);

        item.kind = Kind::Image;
        item.title = "Whiteboard sketch".into();
        assert_eq!(embedding_text(&item, "![Whiteboard sketch](w.png)\n"), "Whiteboard sketch");
        item.title = "Pasted image".into();
        assert_eq!(embedding_text(&item, "![Pasted image](p.png)\n"), "");
        assert_eq!(embedding_text(&item, "![Pasted image](p.png)\n\nOur sprint board\n"), "Our sprint board");
    }

    /// Save → chunk → embed → link with the real model, in a scratch library.
    #[test]
    #[ignore = "needs the model files; run with --ignored"]
    fn links_related_notes_end_to_end() {
        use crate::capture::save;
        use crate::db::Db;
        use crate::library::Library;

        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        let model = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(embed::MODEL_DIR);
        let embedder = Embedder::new(Embedder::find([model]));

        let notes = [
            "Rust ownership: every value has exactly one owner, and the value is dropped when its owner goes out of scope.",
            "The Rust borrow checker ensures references never outlive the data they point to.",
            "Feed the sourdough starter equal parts flour and water every twelve hours.",
            "Bake sourdough bread with steam for the first twenty minutes so the crust stays soft and the loaf rises.",
        ];
        let mut ids = Vec::new();
        for note in notes {
            let item = save::text(&library, &db, note, false).unwrap().item.unwrap();
            let started = Instant::now();
            let body = library.read_note(&library.absolute(&item.path)).unwrap().body;
            let texts = chunk::chunk(&embedding_text(&item, &body));
            let vectors = embedder.embed_passages(&texts).unwrap();
            let links = db
                .with(|conn| {
                    chunks::replace(conn, &item.id, &texts, &vectors)?;
                    link::link(conn, &item.id, &vectors)
                })
                .unwrap();
            println!("{:<60} → {} link(s) in {:?}", &note[..40], links.len(), started.elapsed());
            ids.push(item.id);
        }
        let linked = |a: usize, b: usize| {
            let (src, dst) = if ids[a] < ids[b] { (&ids[a], &ids[b]) } else { (&ids[b], &ids[a]) };
            db.with(|conn| Ok(graph::edges(conn)?.iter().any(|e| &e.src == src && &e.dst == dst))).unwrap()
        };
        assert!(linked(0, 1), "the two Rust notes should be linked");
        assert!(linked(2, 3), "the two bread notes should be linked");
        assert!(!linked(0, 2) && !linked(1, 3), "Rust and bread shouldn't be linked");
    }
}
