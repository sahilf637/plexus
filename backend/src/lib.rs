mod capture;
mod commands;
mod db;
mod enrich;
mod error;
mod extract;
mod fetch;
mod library;
mod llm;
mod pipeline;
mod settings;
mod state;
mod tray;
mod windows;

use std::sync::mpsc::Receiver;

use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use capture::hotkey::{self, Hotkey};
use db::Db;
use extract::ocr::{self, Ocr};
use extract::worker::Extractor;
use fetch::Fetcher;
use library::Library;
use pipeline::Pipeline;
use pipeline::embed::{self, Embedder};
use settings::{Settings, SettingsStore};
use state::AppState;

/// Opens the capture window instead of the map, e.g. from a desktop custom shortcut
/// (`plexus --capture`). Forwarded to the running instance if there is one.
const CAPTURE_FLAG: &str = "--capture";
/// Starts in the tray without showing a window. Used when launched at login.
const HIDDEN_FLAG: &str = "--hidden";

pub fn run() {
    init_tracing();

    tauri::Builder::default()
        // Must come first: a second launch hands its arguments to the running instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            handle_args(app, &args, true);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_FLAG]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let settings = SettingsStore::load(app.path().app_config_dir()?.join("settings.json"))?;
            sync_autostart(app.handle(), &settings.get());
            // `PLEXUS_LIBRARY` opens another library for this run, e.g. one made by `npm run seed`.
            let library_path = std::env::var_os("PLEXUS_LIBRARY").map_or(settings.get().library_path, Into::into);
            tracing::info!(path = %library_path.display(), "opening library");
            let library = Library::open(library_path)?;
            let db = Db::open(&library.index_path())?;
            // Lets the map show images straight from the library folder (and nothing else).
            app.asset_protocol_scope().allow_directory(library.root(), true)?;
            let (pipeline, link_queue) = Pipeline::new(Embedder::new(find_model(app.handle())));
            let (fetcher, fetch_queue) = Fetcher::new();
            let (extractor, extract_queue) = Extractor::new(Ocr::new(find_models(app.handle(), ocr::MODEL_DIR, Ocr::find)));
            app.manage(AppState { settings, hotkey: Hotkey::default(), library, db, pipeline, fetcher, extractor, enricher: Default::default() });
            start_background_work(app.handle(), link_queue, fetch_queue, extract_queue);
            tray::create(app.handle())?;
            hotkey::register(app.handle());
            capture::watch::start(app.handle());
            let args: Vec<String> = std::env::args().collect();
            handle_args(app.handle(), &args, false);
            Ok(())
        })
        // Closing a window only hides it; Plexus keeps running in the tray.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_settings,
            commands::hotkey_status,
            commands::configure_hotkey,
            commands::show_capture,
            commands::hide_capture,
            commands::clipboard_image,
            commands::pasted_files,
            commands::clipboard_files,
            commands::capture_preview,
            commands::capture_text,
            commands::capture_file,
            commands::capture_bytes,
            commands::capture_many,
            commands::list_items,
            commands::get_item,
            commands::get_map,
            commands::save_layout,
            commands::search,
            commands::open_item_file,
            commands::open_source,
            commands::delete_items,
            commands::restore_items,
            commands::link_items,
            commands::unlink_items,
            commands::ai_key_status,
            commands::set_ai_key,
            commands::test_ai,
            commands::list_ai_models,
            commands::enrich_status,
            commands::enrich_retry,
            commands::revert_title,
            commands::remove_ai_link,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Plexus");
}

/// Catches the index up with the library folder, then keeps reading links' pages and linking
/// items as they arrive. Runs on its own threads so startup doesn't wait for it.
fn start_background_work(app: &AppHandle, link_queue: Receiver<String>, fetch_queue: Receiver<String>, extract_queue: Receiver<String>) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let started = std::time::Instant::now();
        match library::reconcile::reconcile(&state.library, &state.db) {
            Ok(report) => {
                tracing::info!(?report, elapsed = ?started.elapsed(), "library reconciled");
                let _ = app.emit("library-changed", ());
                let purged = library::trash::purge(&state.library, library::trash::KEEP);
                if purged > 0 {
                    tracing::info!(purged, "emptied old deleted items");
                }
            }
            Err(e) => tracing::error!("couldn't read the library: {e}"),
        }
        fetch::run(&app, fetch_queue);
        extract::worker::run(&app, extract_queue);
        enrich::run(&app);
        pipeline::run(app.clone(), link_queue);
    });
}

/// The bundled embedding model.
fn find_model(app: &AppHandle) -> Option<std::path::PathBuf> {
    find_models(app, embed::MODEL_DIR, Embedder::find)
}

/// A bundled model folder: in the app's resources, or (in development) in the source tree.
fn find_models(
    app: &AppHandle,
    dir: &str,
    find: impl FnOnce(Vec<std::path::PathBuf>) -> Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let bundled = app.path().resource_dir().ok().map(|resources| resources.join(dir));
    let source_tree =
        cfg!(debug_assertions).then(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(dir));
    find(bundled.into_iter().chain(source_tree).collect())
}

/// Decides which window to show for a launch (or a second launch forwarded to us).
fn handle_args(app: &AppHandle, args: &[String], from_second_instance: bool) {
    if args.iter().any(|a| a == CAPTURE_FLAG) {
        windows::show_capture(app);
    } else if from_second_instance || !args.iter().any(|a| a == HIDDEN_FLAG) {
        windows::show_main(app);
    }
}

/// Registers or removes the launch-at-login entry to match the settings. Skipped in debug
/// builds so development runs don't register the dev binary.
pub(crate) fn sync_autostart(app: &AppHandle, settings: &Settings) {
    if cfg!(debug_assertions) {
        return;
    }
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    let result = if settings.start_at_login { autolaunch.enable() } else { autolaunch.disable() };
    if let Err(e) = result {
        tracing::warn!("couldn't update start at login: {e}");
    }
}

fn init_tracing() {
    // zbus logs harmless proxy-cache warnings for every portal request.
    let default = if cfg!(debug_assertions) { "info,zbus=error,html5ever=error,plexus_lib=debug" } else { "info,zbus=error,html5ever=error" };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
