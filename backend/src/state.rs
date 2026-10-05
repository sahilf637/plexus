//! Shared state handed to commands through Tauri's `State`.

use crate::settings::SettingsStore;

pub struct AppState {
    pub settings: SettingsStore,
}
