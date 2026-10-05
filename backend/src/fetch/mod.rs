//! Reading the web pages behind saved links, in the background. A pasted link is saved at
//! once as a bare address; a moment later its note gets the page's title and article, and only
//! then is it linked, so it's related by what the page says rather than by its address.
//!
//! Offline or a server error: the link stays bare (and is linked by its address) and is tried
//! again at the next start. Gone or not a web page: marked unreadable and left alone.

pub mod web;

use std::fs;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager};

use crate::capture::detect::Kind;
use crate::db::items::{self, Item, Status};
use crate::error::Result;
use crate::library::Library;
use crate::library::reconcile::{item_from_note, modified_nanos};
use crate::state::AppState;
use web::{Failure, Page};

/// `extracted` for a link whose page was read into its note.
pub const READ: &str = "web@1";
/// `extracted` for a link whose page couldn't be read and won't be retried.
pub const UNREADABLE: &str = "web-unreadable@1";
/// Pages read at the same time, so one slow site doesn't hold up the others.
const WORKERS: usize = 3;

pub struct Fetcher {
    queue: Sender<String>,
}

impl Fetcher {
    /// The fetcher plus the receiving end of its queue, for [`run`].
    pub fn new() -> (Self, Receiver<String>) {
        let (queue, receiver) = mpsc::channel();
        (Self { queue }, receiver)
    }

    pub fn enqueue(&self, id: &str) {
        let _ = self.queue.send(id.to_string());
    }
}

/// Whether the item is a link whose page still has to be read.
pub fn is_unread_link(item: &Item) -> bool {
    item.kind == Kind::Url && item.extracted.is_none()
}

/// Starts the workers that read pages as links arrive.
pub fn run(app: &AppHandle, queue: Receiver<String>) {
    let queue = Arc::new(Mutex::new(queue));
    for n in 0..WORKERS {
        let (app, queue) = (app.clone(), queue.clone());
        let worker = move || {
            loop {
                let next = queue.lock().expect("fetch queue lock poisoned").recv();
                let Ok(id) = next else { break };
                if let Err(e) = process(&app, &id) {
                    tracing::warn!(id, "couldn't read the link's page: {e}");
                }
            }
        };
        if let Err(e) = std::thread::Builder::new().name(format!("fetch-{n}")).spawn(worker) {
            tracing::error!("couldn't start a page reader: {e}");
        }
    }
}

fn process(app: &AppHandle, id: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let Some(item) = state.db.with(|conn| items::get(conn, id))? else {
        return Ok(()); // Deleted meanwhile.
    };
    let url = item.source_url.clone().filter(|_| is_unread_link(&item) && state.settings.get().fetch_links);
    let Some(url) = url else {
        link_if_pending(&state, &item);
        return Ok(());
    };

    let started = Instant::now();
    match web::fetch(&url) {
        Ok(page) => {
            let updated = rewrite(&state.library, &state.db, &item, Some(page))?;
            tracing::debug!(id, title = updated.title, elapsed = ?started.elapsed(), "link's page read");
            let _ = app.emit("item-updated", &updated);
            state.pipeline.enqueue(id);
        }
        Err(Failure::Unreadable(reason)) => {
            tracing::info!(url, reason, "link's page is unreadable; keeping the bare link");
            rewrite(&state.library, &state.db, &item, None)?;
            state.pipeline.enqueue(id);
        }
        Err(Failure::Retry(reason)) => {
            tracing::info!(url, reason, "couldn't read the link's page; trying again at next start");
            link_if_pending(&state, &item);
        }
    }
    Ok(())
}

/// Links the item as it is, unless that's already been done.
fn link_if_pending(state: &AppState, item: &Item) {
    if item.status == Status::Pending {
        state.pipeline.enqueue(&item.id);
    }
}

/// Puts the page into the link's note (or marks it unreadable when `page` is `None`). A new
/// title renames the file to match. Returns the updated index row, which is `pending` again.
fn rewrite(library: &Library, db: &crate::db::Db, item: &Item, page: Option<Page>) -> Result<Item> {
    let old_path = library.absolute(&item.path);
    let mut note = library.read_note(&old_path)?;
    match page {
        Some(page) => {
            if !page.markdown.is_empty() {
                note.body = format!("{}\n", page.markdown);
            }
            if let Some(title) = page.title {
                note.meta.title = title;
            }
            note.meta.extracted = Some(READ.into());
        }
        None => note.meta.extracted = Some(UNREADABLE.into()),
    }

    let path = if note.meta.title == item.title { old_path.clone() } else { library.path_for(&note.meta) };
    library.write_note(&path, &note)?;
    if path != old_path {
        fs::remove_file(&old_path)?;
    }
    let stat = fs::metadata(&path)?;
    let updated = item_from_note(library, &path, &note, modified_nanos(&stat), stat.len() as i64);
    // Read back, so the AI's summary (kept in the index, not the note) isn't lost.
    db.with(|conn| {
        items::upsert(conn, &updated, &note.body)?;
        Ok(items::get(conn, &updated.id)?.unwrap_or(updated))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::save;
    use crate::db::Db;

    #[test]
    fn a_read_page_renames_the_note_and_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        let url = "https://example.com/posts/42";
        let item = save::text(&library, &db, url, false).unwrap().item.unwrap();
        assert!(is_unread_link(&item));
        assert!(item.path.contains("example-com-posts-42"));

        let page = Page { title: Some("Why steam matters".into()), markdown: "Steam keeps the crust soft.".into() };
        let updated = rewrite(&library, &db, &item, Some(page)).unwrap();
        assert!(updated.path.contains("/why-steam-matters-") && !library.absolute(&item.path).exists());
        assert_eq!((updated.title.as_str(), updated.status), ("Why steam matters", Status::Pending));
        assert!(!is_unread_link(&updated));

        let note = library.read_note(&library.absolute(&updated.path)).unwrap();
        assert_eq!(note.body, "Steam keeps the crust soft.\n");
        assert_eq!(note.meta.source.as_deref(), Some(url));
        assert_eq!(db.with(|conn| items::get(conn, &item.id)).unwrap(), Some(updated));
        assert_eq!(db.with(|conn| items::unread_links(conn)).unwrap(), Vec::<String>::new());

        // Pasting the address again finds the same item, though its body changed.
        let again = save::text(&library, &db, url, false).unwrap();
        assert!(matches!(again.result, crate::capture::CaptureResult::Duplicate { ref id, .. } if *id == item.id));
    }

    #[test]
    fn an_unreadable_page_keeps_the_bare_link() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path().to_path_buf()).unwrap();
        let db = Db::open(&library.index_path()).unwrap();
        let item = save::text(&library, &db, "https://example.com/gone", false).unwrap().item.unwrap();
        let updated = rewrite(&library, &db, &item, None).unwrap();
        assert_eq!((updated.path.as_str(), updated.extracted.as_deref()), (item.path.as_str(), Some(UNREADABLE)));
        let note = library.read_note(&library.absolute(&updated.path)).unwrap();
        assert_eq!(note.body, "https://example.com/gone\n");
    }
}
