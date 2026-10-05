mod commands;
mod error;
mod settings;
mod state;
mod tray;
mod windows;

use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

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
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let settings = SettingsStore::load(app.path().app_config_dir()?.join("settings.json"))?;
            sync_autostart(app.handle(), &settings.get());
            app.manage(AppState { settings });
            tray::create(app.handle())?;
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
            commands::show_capture,
            commands::hide_capture,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Plexus");
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
    let default = if cfg!(debug_assertions) { "info,plexus_lib=debug" } else { "info" };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
