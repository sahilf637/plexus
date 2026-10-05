//! Shared state handed to commands through Tauri's `State`.

use crate::capture::hotkey::Hotkey;
use crate::db::Db;
use crate::library::Library;
use crate::settings::SettingsStore;

pub struct AppState {
    pub settings: SettingsStore,
    pub hotkey: Hotkey,
    pub library: Library,
    pub db: Db,
}
