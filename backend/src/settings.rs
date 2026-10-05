//! User settings, kept as JSON in the app's config folder (not in the library, since the
//! library's location is itself a setting).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Folder that holds the note files.
    pub library_path: PathBuf,
    /// Global shortcut, e.g. "Ctrl+Alt+V".
    pub hotkey: String,
    pub hotkey_action: HotkeyAction,
    /// Launch at login so the hotkey is always ready.
    pub start_at_login: bool,
    /// Download the web page behind a saved link, so it can be searched and linked by what it
    /// says. Off keeps links as they were pasted, with no requests made.
    pub fetch_links: bool,
}

/// What the global shortcut does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyAction {
    /// Open the capture window to paste, drop or type something.
    #[default]
    OpenCapture,
    /// Save whatever is on the clipboard straight away.
    SaveClipboard,
}

impl Default for Settings {
    fn default() -> Self {
        let home = std::env::home_dir().unwrap_or_default();
        Self {
            library_path: home.join("Plexus"),
            hotkey: "Ctrl+Alt+V".into(),
            hotkey_action: HotkeyAction::default(),
            start_at_login: true,
            fetch_links: true,
        }
    }
}

/// The current settings plus where they're saved.
pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Reads the settings file, falling back to defaults when it doesn't exist yet.
    /// Missing fields (from an older version) take their default values.
    pub fn load(path: PathBuf) -> Result<Self> {
        let current = match fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, current: Mutex::new(current) })
    }

    pub fn get(&self) -> Settings {
        self.current.lock().expect("settings lock poisoned").clone()
    }

    pub fn set(&self, settings: Settings) -> Result<()> {
        write_atomic(&self.path, serde_json::to_string_pretty(&settings)?.as_bytes())?;
        *self.current.lock().expect("settings lock poisoned") = settings;
        Ok(())
    }
}

/// Writes to a temp file next to `path` and renames it into place, so a crash never leaves a
/// half-written file behind.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_set_round_trips() {
        let dir = std::env::temp_dir().join(format!("plexus-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let store = SettingsStore::load(path.clone()).unwrap();
        assert_eq!(store.get(), Settings::default());

        let changed = Settings { hotkey_action: HotkeyAction::SaveClipboard, ..Settings::default() };
        store.set(changed.clone()).unwrap();
        assert_eq!(SettingsStore::load(path).unwrap().get(), changed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_fields_take_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+Space"}"#).unwrap();
        assert_eq!(settings.hotkey, "Ctrl+Shift+Space");
        assert_eq!(settings.hotkey_action, HotkeyAction::OpenCapture);
        assert!(settings.start_at_login);
    }
}
