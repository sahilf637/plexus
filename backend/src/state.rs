//! Shared state handed to commands through Tauri's `State`.

use crate::capture::hotkey::Hotkey;
use crate::db::Db;
use crate::db::items::Item;
use crate::fetch::{self, Fetcher};
use crate::library::Library;
use crate::pipeline::Pipeline;
use crate::settings::SettingsStore;

pub struct AppState {
    pub settings: SettingsStore,
    pub hotkey: Hotkey,
    pub library: Library,
    pub db: Db,
    pub pipeline: Pipeline,
    pub fetcher: Fetcher,
}

impl AppState {
    /// Hands a new item to the background work: links have their page read first, everything
    /// else goes straight to linking.
    pub fn process_later(&self, item: &Item) {
        if fetch::is_unread_link(item) {
            self.fetcher.enqueue(&item.id);
        } else {
            self.pipeline.enqueue(&item.id);
        }
    }
}
