//! The command surface the UI can call. Each command is a thin wrapper over a module function.

use tauri::{AppHandle, State};

use crate::error::Result;
use crate::settings::Settings;
use crate::state::AppState;
use crate::windows;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.get()
}

#[tauri::command]
pub fn set_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
    state.settings.set(settings.clone())?;
    crate::sync_autostart(&app, &settings);
    Ok(settings)
}

#[tauri::command]
pub fn show_capture(app: AppHandle) {
    windows::show_capture(&app);
}

#[tauri::command]
pub fn hide_capture(app: AppHandle) {
    windows::hide_capture(&app);
}
