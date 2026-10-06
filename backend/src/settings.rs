//! User settings, kept as JSON in the app's config folder (not in the library, since the
//! library's location is itself a setting).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::llm::ProviderKind;

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
    /// Save text copied anywhere, without pasting it into Plexus. Off by default.
    pub watch_clipboard: bool,
    pub ai: AiSettings,
    /// The first-run setup was finished or skipped. False for a new install; a settings file from
    /// before setup existed belongs to someone already using Plexus, so a missing field means true.
    #[serde(default = "yes")]
    pub setup_done: bool,
}

fn yes() -> bool {
    true
}

/// The AI side of the settings. API keys are not here: each connection's key lives in the
/// system keychain, under the connection's id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, from = "RawAi")]
pub struct AiSettings {
    /// Enrich new items (titles, summaries, concepts, typed links).
    pub enabled: bool,
    /// Places the user can send requests to, each with its own key.
    pub connections: Vec<AiConnection>,
    /// The id of the connection in use.
    pub active: String,
    /// Most calls per day; 0 means no limit.
    pub daily_limit: u32,
    /// Most questions asked of the library per day (Ask); 0 means no limit.
    pub ask_per_day: u32,
    /// How much each question may spend.
    pub ask_profile: AskProfile,
    /// Slowly enrich the notes saved before AI was switched on, a few a day.
    pub improve_old: bool,
    /// Most older notes improved per day; 0 means only the daily call limit applies.
    pub improve_per_day: u32,
}

/// How much an Ask question may spend: steps, results, how many notes it starts from and how much
/// of the conversation it carries. Economy suits free plans with small per-minute limits,
/// Thorough suits pay-per-use plans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskProfile {
    Economy,
    #[default]
    Standard,
    Thorough,
}

/// One provider, model and key (the key itself is in the keychain).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiConnection {
    /// Names the keychain entry; never changes.
    pub id: String,
    /// What the user calls it ("Work OpenAI").
    pub name: String,
    /// Which preset it was made from (for the form; e.g. "groq").
    pub preset: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    /// The model can read images, so screenshots and photos get described.
    pub vision: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            connections: Vec::new(),
            active: String::new(),
            daily_limit: 500,
            ask_per_day: 20,
            ask_profile: AskProfile::default(),
            improve_old: false,
            improve_per_day: 10,
        }
    }
}

impl Default for AiConnection {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "OpenAI".into(),
            preset: "openai".into(),
            kind: ProviderKind::OpenAi,
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
            vision: true,
        }
    }
}

impl AiSettings {
    /// The connection in use, if there is one.
    pub fn active_connection(&self) -> Option<&AiConnection> {
        self.connections.iter().find(|c| c.id == self.active)
    }
}

/// What the settings file may hold: the current shape, and the earlier one with a single provider
/// (`preset`, `kind`, `baseUrl`, `model`, `vision` beside `enabled`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawAi {
    enabled: bool,
    connections: Vec<AiConnection>,
    active: String,
    daily_limit: Option<u32>,
    ask_per_day: Option<u32>,
    ask_profile: Option<AskProfile>,
    improve_old: bool,
    improve_per_day: Option<u32>,
    preset: Option<String>,
    kind: Option<ProviderKind>,
    base_url: Option<String>,
    model: Option<String>,
    vision: Option<bool>,
}

impl From<RawAi> for AiSettings {
    fn from(raw: RawAi) -> Self {
        let mut ai = AiSettings {
            enabled: raw.enabled,
            connections: raw.connections,
            active: raw.active,
            daily_limit: raw.daily_limit.unwrap_or(AiSettings::default().daily_limit),
            ask_per_day: raw.ask_per_day.unwrap_or(AiSettings::default().ask_per_day),
            ask_profile: raw.ask_profile.unwrap_or_default(),
            improve_old: raw.improve_old,
            improve_per_day: raw.improve_per_day.unwrap_or(AiSettings::default().improve_per_day),
        };
        // An earlier version had one provider. It becomes a connection named after its preset, so
        // the key already in the keychain (stored under the preset's name) keeps working.
        if let (true, Some(preset)) = (ai.connections.is_empty(), raw.preset) {
            let base = AiConnection::default();
            ai.connections.push(AiConnection {
                id: preset.clone(),
                name: preset.clone(),
                preset: preset.clone(),
                kind: raw.kind.unwrap_or(base.kind),
                base_url: raw.base_url.unwrap_or(base.base_url),
                model: raw.model.unwrap_or(base.model),
                vision: raw.vision.unwrap_or(base.vision),
            });
            ai.active = preset;
        }
        if ai.active_connection().is_none() {
            ai.active = ai.connections.first().map(|c| c.id.clone()).unwrap_or_default();
        }
        ai
    }
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
            watch_clipboard: false,
            ai: AiSettings::default(),
            setup_done: false,
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
    fn the_single_provider_of_an_earlier_version_becomes_a_connection() {
        let old = r#"{"ai":{"enabled":true,"preset":"groq","kind":"openai","baseUrl":"https://api.groq.com/openai/v1","model":"llama-3.3-70b-versatile","vision":false,"dailyLimit":100}}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        let ai = settings.ai;
        assert!(ai.enabled && ai.daily_limit == 100);
        assert_eq!(ai.connections.len(), 1);
        // Same id as the old keychain entry, so the saved key is still found.
        assert_eq!((ai.connections[0].id.as_str(), ai.active.as_str()), ("groq", "groq"));
        assert_eq!(ai.active_connection().map(|c| c.model.as_str()), Some("llama-3.3-70b-versatile"));
        assert!(!ai.connections[0].vision);

        // The current shape round-trips, and a dangling `active` falls back to the first.
        let json = serde_json::to_string(&Settings { ai: ai.clone(), ..Settings::default() }).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap().ai, ai);
        let dangling: AiSettings = serde_json::from_str(r#"{"connections":[{"id":"a","name":"A"}],"active":"gone"}"#).unwrap();
        assert_eq!(dangling.active, "a");
        assert_eq!(AiSettings::default().active_connection(), None);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Shift+Space"}"#).unwrap();
        assert_eq!(settings.hotkey, "Ctrl+Shift+Space");
        assert_eq!(settings.hotkey_action, HotkeyAction::OpenCapture);
        assert!(settings.start_at_login);
    }

    #[test]
    fn only_a_new_install_runs_setup() {
        assert!(!Settings::default().setup_done);
        let existing: Settings = serde_json::from_str(r#"{"hotkey":"Ctrl+Alt+V"}"#).unwrap();
        assert!(existing.setup_done);
    }
}
