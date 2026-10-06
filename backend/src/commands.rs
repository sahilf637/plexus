//! The command surface the UI can call. Each command is a thin wrapper over a module function.

use std::borrow::Cow;
use std::path::PathBuf;

use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Emitter, State};

use crate::capture::detect::{self, Kind};
use crate::capture::{self, CaptureResult, Preview, clipboard, hotkey, save};
use crate::db::{concepts, graph};
use crate::enrich;
use crate::llm::keys;
use crate::db::items::{self, Item};
use crate::error::{Error, Result};
use crate::library::{links, trash};
use crate::settings::Settings;
use crate::state::AppState;
use crate::windows;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.get()
}

#[tauri::command]
pub fn set_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
    let previous = state.settings.get();
    state.settings.set(settings.clone())?;
    crate::sync_autostart(&app, &settings);
    if settings.hotkey != previous.hotkey {
        hotkey::register(&app);
    }
    if settings.ai != previous.ai {
        // A connection that was removed takes its key with it.
        for gone in previous.ai.connections.iter().filter(|c| !settings.ai.connections.iter().any(|n| n.id == c.id)) {
            if let Err(e) = keys::clear(&gone.id) {
                tracing::warn!(id = gone.id, "couldn't remove a deleted connection's key: {e}");
            }
        }
        enrich::settings_changed(&app, &previous.ai, &settings.ai);
    }
    Ok(settings)
}

/// Whether a key is stored for a connection.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiKeyStatus {
    has_key: bool,
    /// Set when the system keychain couldn't be read.
    error: Option<String>,
}

#[tauri::command]
pub fn ai_key_status(id: String) -> AiKeyStatus {
    match keys::get(&id) {
        Ok(key) => AiKeyStatus { has_key: key.is_some(), error: None },
        Err(e) => AiKeyStatus { has_key: false, error: Some(e.to_string()) },
    }
}

/// Stores a connection's key in the system keychain (an empty key removes it).
#[tauri::command]
pub fn set_ai_key(app: AppHandle, state: State<'_, AppState>, id: String, key: String) -> Result<()> {
    if key.trim().is_empty() {
        keys::clear(&id)?;
    } else {
        keys::set(&id, &key)?;
    }
    state.enricher.resume();
    enrich::requeue_missed(&state);
    let _ = app.emit("enrich-status", state.enricher.status(state.settings.get().ai.enabled));
    Ok(())
}

/// Checks a connection with a tiny request, before or after it's saved. `key` is the one just
/// typed; without it the saved key is used. Runs off the UI thread: a slow provider can take a
/// while.
#[tauri::command]
pub async fn test_ai(connection: crate::settings::AiConnection, key: Option<String>) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let _purpose = crate::usage::purpose(crate::usage::Purpose::Check);
        enrich::test_connection(&connection, key.as_deref())
    })
        .await
        .map_err(|e| Error::Invalid(e.to_string()))?
}

/// The chat models a connection's provider offers, for the model dropdown. Needs its key (just
/// typed, or saved) unless the server is local.
#[tauri::command]
pub async fn list_ai_models(connection: crate::settings::AiConnection, key: Option<String>) -> Result<Vec<crate::llm::models::Model>> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(crate::llm::models::list(&enrich::connection_config(&connection, key.as_deref())?)?)
    })
    .await
    .map_err(|e| Error::Invalid(e.to_string()))?
}

#[tauri::command]
pub fn enrich_status(state: State<'_, AppState>) -> enrich::QueueStatus {
    state.enricher.status(state.settings.get().ai.enabled)
}

/// Tries again after a pause (a wrong key fixed, a limit raised).
#[tauri::command]
pub fn enrich_retry(app: AppHandle, state: State<'_, AppState>) {
    state.enricher.resume();
    enrich::requeue_missed(&state);
    let _ = app.emit("enrich-status", state.enricher.status(state.settings.get().ai.enabled));
}

/// Puts back the title an item had before the AI renamed it (and keeps it from changing again).
#[tauri::command]
pub fn revert_title(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    if let Some(item) = enrich::apply::revert_title(&state.library, &state.db, &id)? {
        let _ = app.emit("item-updated", &item);
    }
    Ok(())
}

/// Removes a link the AI proposed; it won't come back.
#[tauri::command]
pub fn remove_ai_link(app: AppHandle, state: State<'_, AppState>, a: String, b: String) -> Result<()> {
    state.db.with(|conn| graph::reject_ai_edge(conn, &a, &b))?;
    announce_links(&app, [&a, &b], Vec::new(), &state)
}

/// The topic tree and each item's topic, as of now.
#[tauri::command]
pub fn get_topics(state: State<'_, AppState>) -> Result<crate::topics::TopicsData> {
    state.db.with(|conn| crate::topics::data(conn))
}

/// Names a topic yourself; it is then never renamed.
#[tauri::command]
pub fn rename_topic(app: AppHandle, state: State<'_, AppState>, id: String, name: String) -> Result<()> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > 60 {
        return Err(Error::Invalid("a topic name needs 1 to 60 characters".into()));
    }
    let found = state.db.with(|conn| {
        let Some(topic) = crate::db::topics::get(conn, &id)? else { return Ok(false) };
        crate::db::topics::set_label(conn, &id, &name, crate::db::topics::LabelBy::You, topic.summary.as_deref(), None)?;
        Ok(true)
    })?;
    if found {
        crate::topics::announce(&app, &state);
    }
    Ok(())
}

/// Goes back to the name Plexus made (a new one is worked out in the background).
#[tauri::command]
pub fn reset_topic_name(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    state.db.with(|conn| {
        if let Some(topic) = crate::db::topics::get(conn, &id)? {
            crate::db::topics::set_label(conn, &id, &topic.label, crate::db::topics::LabelBy::Auto, None, None)?;
        }
        Ok(())
    })?;
    crate::topics::schedule_now(&app);
    Ok(())
}

/// Builds the topics again now, instead of waiting for the library to grow.
#[tauri::command]
pub fn regroup_topics(app: AppHandle) {
    crate::topics::schedule_now(&app);
}

/// How the slow pass over older notes is going.
#[tauri::command]
pub fn older_status(state: State<'_, AppState>) -> Result<enrich::older::Progress> {
    state.db.with(|conn| enrich::older::progress(conn))
}

#[tauri::command]
pub fn hotkey_status(state: State<'_, AppState>) -> hotkey::HotkeyStatus {
    state.hotkey.status()
}

/// Opens the system's own dialog for changing the shortcut (Wayland desktops).
#[tauri::command]
pub fn configure_hotkey(app: AppHandle) {
    hotkey::configure(&app);
}

#[tauri::command]
pub fn show_capture(app: AppHandle) {
    windows::show_capture(&app);
}

#[tauri::command]
pub fn hide_capture(app: AppHandle) {
    windows::hide_capture(&app);
}

/// The clipboard image as PNG bytes (empty when there is none). The webview's paste event
/// doesn't carry images on Linux, so the capture window asks for them here.
#[tauri::command]
pub fn clipboard_image() -> Result<Response> {
    Ok(Response::new(clipboard::image_png()?.unwrap_or_default()))
}

/// Files copied in a file manager, read from the clipboard (the paste event comes up empty).
#[tauri::command]
pub fn clipboard_files() -> Result<Vec<PathBuf>> {
    clipboard::copied_files()
}

/// Pasted text that is only paths to existing files (copied in a file manager) → those paths.
#[tauri::command]
pub fn pasted_files(text: String) -> Vec<PathBuf> {
    clipboard::existing_files(&text)
}

#[tauri::command]
pub fn capture_preview(text: String) -> Preview {
    capture::preview_text(&text)
}

#[tauri::command]
pub fn capture_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    force: bool,
    html: Option<String>,
) -> Result<CaptureResult> {
    timed("text", || save::rich_text(&state.library, &state.db, &text, html.as_deref(), force))
        .map(|saved| capture::announce(&app, saved))
}

/// Files dropped onto a window, or copied in a file manager, by path, with an optional note.
#[tauri::command]
pub fn capture_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: PathBuf,
    note: Option<String>,
    force: Option<bool>,
) -> Result<CaptureResult> {
    let (note, force) = (note.unwrap_or_default(), force.unwrap_or(false));
    timed("file", || save::file_from_disk(&state.library, &state.db, &path, &note, force))
        .map(|saved| capture::announce(&app, saved))
}

/// Pasted images and files, sent as raw bytes with `x-name` / `x-mime` headers, and the note
/// (URI-encoded) in `x-note`, `x-force: 1` to keep a note that looks like a secret.
#[tauri::command]
pub fn capture_bytes(app: AppHandle, state: State<'_, AppState>, request: Request<'_>) -> Result<CaptureResult> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(Error::Invalid("expected the file's bytes".into()));
    };
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    let name = clipboard::percent_decode(header("x-name"));
    let mime = match header("x-mime") {
        "" => capture::mime_from_name(&name).to_string(),
        mime => mime.to_string(),
    };
    let note = clipboard::percent_decode(header("x-note"));
    let force = header("x-force") == "1";
    timed("bytes", || save::file(&state.library, &state.db, &name, &mime, bytes, &note, force))
        .map(|saved| capture::announce(&app, saved))
}

/// Several images and files with one note, saved as one item. The bytes of pasted ones are
/// the raw body, one after another; `x-parts` (URI-encoded JSON) lists each part as
/// `{"name", "mime", "size"}` for bytes in the body, or `{"path"}` for a file on disk.
#[tauri::command]
pub fn capture_many(app: AppHandle, state: State<'_, AppState>, request: Request<'_>) -> Result<CaptureResult> {
    #[derive(serde::Deserialize)]
    struct Part {
        name: Option<String>,
        mime: Option<String>,
        size: Option<usize>,
        path: Option<PathBuf>,
    }
    let InvokeBody::Raw(body) = request.body() else {
        return Err(Error::Invalid("expected the files' bytes".into()));
    };
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    let parts: Vec<Part> = serde_json::from_str(&clipboard::percent_decode(header("x-parts")))?;
    let note = clipboard::percent_decode(header("x-note"));
    let force = header("x-force") == "1";

    // (name, mime, bytes): files on disk are read, pasted ones are sliced out of the body.
    let mut loaded: Vec<(String, String, Cow<[u8]>)> = Vec::with_capacity(parts.len());
    let mut offset = 0;
    for part in parts {
        if let Some(path) = part.path {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mime = capture::mime_from_name(&name).to_string();
            loaded.push((name, mime, Cow::Owned(std::fs::read(&path)?)));
        } else {
            let size = part.size.unwrap_or(0);
            let bytes = body.get(offset..offset + size).ok_or_else(|| Error::Invalid("file sizes don't add up".into()))?;
            offset += size;
            let name = part.name.unwrap_or_default();
            let mime = part.mime.filter(|m| !m.is_empty()).unwrap_or_else(|| capture::mime_from_name(&name).to_string());
            loaded.push((name, mime, Cow::Borrowed(bytes)));
        }
    }
    let originals: Vec<save::Original> =
        loaded.iter().map(|(name, mime, bytes)| save::Original { name, mime, bytes }).collect();
    timed("files", || save::files(&state.library, &state.db, &originals, &note, force))
        .map(|saved| capture::announce(&app, saved))
}

/// One item with its note body, for the detail panel.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetail {
    #[serde(flatten)]
    item: Item,
    body: String,
    /// Absolute path of the original file, if any, for opening or previewing it.
    asset_file: Option<PathBuf>,
    /// More originals saved with it, after the first.
    attachments: Vec<AttachmentDetail>,
    /// Text read out of the original (a PDF's pages, an image's words), apart from the note.
    extracted_text: Option<String>,
    concepts: Vec<concepts::Concept>,
    /// The title was written by the AI (and can be reverted to `title_was`).
    title_by_ai: bool,
    title_was: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDetail {
    name: String,
    file: PathBuf,
    kind: Kind,
    size: Option<u64>,
}

#[tauri::command]
pub fn get_item(state: State<'_, AppState>, id: String) -> Result<ItemDetail> {
    let item = find_item(&state, &id)?;
    let note_path = state.library.absolute(&item.path);
    let note = state.library.read_note(&note_path)?;
    let asset_file = item.asset_path.as_deref().map(|p| state.library.absolute(p));
    let attachments = note
        .meta
        .attachments
        .iter()
        .map(|a| {
            let name = a.original_name.clone().unwrap_or_else(|| a.file.clone());
            let kind = detect::detect_file(&name, a.mime.as_deref().unwrap_or("")).kind;
            AttachmentDetail { kind, size: a.size, file: note_path.with_file_name(&a.file), name }
        })
        .collect();
    let (own, extracted) = crate::extract::split_body(&note.body);
    let (body, extracted_text) = (own.to_string(), extracted.map(str::to_string));
    let concepts = state.db.with(|conn| concepts::of_item(conn, &id))?;
    let title_by_ai = note.meta.title_by.as_deref() == Some("ai");
    let title_was = note.meta.title_was.clone().filter(|_| title_by_ai);
    Ok(ItemDetail { item, body, asset_file, attachments, extracted_text, concepts, title_by_ai, title_was })
}

/// Everything the map draws: items, edges and saved positions.
#[derive(serde::Serialize)]
pub struct MapData {
    items: Vec<Item>,
    edges: Vec<graph::Edge>,
    positions: Vec<graph::Position>,
    concepts: Vec<concepts::ConceptItems>,
    topics: crate::topics::TopicsData,
}

#[tauri::command]
pub fn get_map(state: State<'_, AppState>) -> Result<MapData> {
    state.db.with(|conn| {
        Ok(MapData {
            items: items::all(conn)?,
            edges: graph::edges(conn)?,
            positions: graph::positions(conn)?,
            concepts: concepts::with_items(conn)?,
            topics: crate::topics::data(conn)?,
        })
    })
}

#[tauri::command]
pub fn save_layout(state: State<'_, AppState>, positions: Vec<graph::Position>) -> Result<()> {
    state.db.with(|conn| graph::save_positions(conn, &positions))
}

/// Item ids matching the query by words or by meaning, best first.
#[tauri::command]
pub fn search(state: State<'_, AppState>, query: String) -> Result<Vec<String>> {
    crate::pipeline::search::hybrid(&state, &query, 50)
}

/// Opens the item's original file (or, for notes without one, the note itself) in the
/// default app; `attachment` picks one of the originals saved after the first.
#[tauri::command]
pub fn open_item_file(app: AppHandle, state: State<'_, AppState>, id: String, attachment: Option<usize>) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let item = find_item(&state, &id)?;
    let path = match attachment {
        Some(index) => {
            let note_path = state.library.absolute(&item.path);
            let note = state.library.read_note(&note_path)?;
            let file = note.meta.attachments.get(index).ok_or_else(|| Error::Invalid("no such attachment".into()))?;
            note_path.with_file_name(&file.file)
        }
        None => state.library.absolute(item.asset_path.as_deref().unwrap_or(&item.path)),
    };
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| Error::Invalid(e.to_string()))
}

/// Opens a link item's web page in the browser.
#[tauri::command]
pub fn open_source(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let url = find_item(&state, &id)?.source_url.ok_or_else(|| Error::Invalid("this item has no link".into()))?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(Error::Invalid("only web links can be opened".into()));
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| Error::Invalid(e.to_string()))
}

/// Deletes items: their files go to the library's trash and the map drops them. The result
/// carries the batch to pass to `restore_items` for Undo.
#[tauri::command]
pub fn delete_items(app: AppHandle, state: State<'_, AppState>, ids: Vec<String>) -> Result<trash::Deleted> {
    let deleted = trash::delete_items(&state.library, &state.db, &ids)?;
    if !deleted.ids.is_empty() {
        crate::topics::forget(&state);
        let _ = app.emit("items-deleted", &deleted.ids);
    }
    Ok(deleted)
}

/// Undo of `delete_items`: puts the files back; the items are linked again as they return.
#[tauri::command]
pub fn restore_items(app: AppHandle, state: State<'_, AppState>, batch: String) -> Result<usize> {
    let restored = trash::restore_items(&state.library, &state.db, &batch)?;
    for item in &restored {
        let _ = app.emit("item-added", item);
        state.process_later(item);
    }
    Ok(restored.len())
}

/// What the map needs to redraw after a hand-made link changes: every link touching these items.
#[derive(Clone, serde::Serialize)]
struct LinksChanged {
    ids: Vec<String>,
    edges: Vec<graph::Edge>,
}

fn announce_links(app: &AppHandle, ids: [&str; 2], edges: Vec<graph::Edge>, state: &AppState) -> Result<()> {
    let mut all = edges;
    all.extend(state.db.with(|conn| graph::edges_of(conn, ids[1]))?);
    let mut seen = std::collections::HashSet::new();
    all.retain(|e| seen.insert((e.src.clone(), e.dst.clone(), e.kind.clone())));
    let _ = app.emit("links-changed", LinksChanged { ids: ids.map(String::from).to_vec(), edges: all });
    Ok(())
}

/// Links two items by hand (kept in the note of `from`). Hand-made links are always strong.
#[tauri::command]
pub fn link_items(app: AppHandle, state: State<'_, AppState>, from: String, to: String) -> Result<()> {
    let edges = links::add(&state.library, &state.db, &from, &to)?;
    announce_links(&app, [&from, &to], edges, &state)
}

/// Removes a link made by hand.
#[tauri::command]
pub fn unlink_items(app: AppHandle, state: State<'_, AppState>, a: String, b: String) -> Result<()> {
    let edges = links::remove(&state.library, &state.db, &a, &b)?;
    announce_links(&app, [&a, &b], edges, &state)
}

fn find_item(state: &AppState, id: &str) -> Result<Item> {
    state.db.with(|conn| items::get(conn, id))?.ok_or_else(|| Error::Invalid(format!("no item {id}")))
}

/// Everything in the index, for the map.
#[tauri::command]
pub fn list_items(state: State<'_, AppState>) -> Result<Vec<Item>> {
    state.db.with(|conn| items::all(conn))
}

/// Saving is meant to stay well under 50 ms; log how long it takes.
fn timed<T>(what: &str, save: impl FnOnce() -> Result<T>) -> Result<T> {
    let started = std::time::Instant::now();
    let result = save();
    tracing::debug!(what, elapsed = ?started.elapsed(), ok = result.is_ok(), "capture saved");
    result
}

// --- Ask --------------------------------------------------------------------------------

/// Starts a question about the library (or about one topic or note); `thread` is the
/// conversation it follows on from. Returns it at once; the
/// answer arrives as `ask-done` once the agent has worked through it.
#[tauri::command]
pub fn ask_start(app: AppHandle, state: State<'_, AppState>, question: String, scope: Option<String>, thread: Option<String>) -> Result<crate::db::asks::Ask> {
    let ai = state.settings.get().ai;
    // Fails early, with a plain message, when there is no connection to ask through.
    enrich::config_for(&ai).map_err(|_| Error::Invalid("Connect an AI in Settings to get written answers.".into()))?;
    let finder = crate::ask::tools::AppFinder(&state);
    let start = crate::ask::session::Start {
        question: &question,
        scope: scope.as_deref(),
        thread: thread.as_deref(),
        per_day: ai.ask_per_day,
        budget: crate::ask::budget::Budget::for_profile(ai.ask_profile),
    };
    let id = state.asker.begin(&state.db, &finder, &start)?;
    crate::ask::spawn(&app, id.clone());
    state.db.with(|conn| crate::db::asks::get(conn, &id))?.ok_or_else(|| Error::Invalid("the question wasn't saved".into()))
}

/// Stops a question that is going; it ends with what it has found so far.
#[tauri::command]
pub fn ask_stop(state: State<'_, AppState>, id: String) {
    state.asker.stop(&id);
}

/// Goes on with a question that a provider error stopped.
#[tauri::command]
pub fn ask_resume(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    if !state.asker.can_resume(&id) {
        return Err(Error::Invalid("That question can't be continued; ask it again.".into()));
    }
    crate::ask::spawn(&app, id);
    Ok(())
}

/// Lets a partial answer (one that ran out of steps) go on looking where it stopped.
#[tauri::command]
pub fn ask_continue(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    state.asker.extend(&id)?;
    crate::ask::spawn(&app, id);
    Ok(())
}

/// Past questions with their answers, newest first.
#[tauri::command]
pub fn ask_history(state: State<'_, AppState>) -> Result<Vec<crate::db::asks::Ask>> {
    state.db.with(|conn| crate::db::asks::recent(conn, 100))
}

#[tauri::command]
pub fn ask_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    state.asker.stop(&id);
    state.asker.forget(&id);
    state.db.with(|conn| crate::db::asks::delete(conn, &id))
}

/// Tries a model on a small made-up question and says whether it can drive Ask, and whether it
/// calls tools or has to be asked for JSON. Like `test_ai`, for a connection before or after it is
/// saved.
#[tauri::command]
pub async fn ask_check(connection: crate::settings::AiConnection, key: Option<String>) -> Result<crate::ask::check::AskCheck> {
    tauri::async_runtime::spawn_blocking(move || {
        let _purpose = crate::usage::purpose(crate::usage::Purpose::Check);
        let config = enrich::connection_config(&connection, key.as_deref())?;
        Ok(crate::ask::check::check(&config))
    })
    .await
    .map_err(|e| Error::Invalid(e.to_string()))?
}

/// What the AI has been used for: today, the last week, and by purpose. For the usage page.
#[tauri::command]
pub fn ai_usage(state: State<'_, AppState>) -> Result<crate::usage::Report> {
    state.db.with(|conn| crate::usage::report(conn))
}
