//! The global shortcut. It works system-wide for as long as Plexus runs, even with every window
//! closed. Wayland doesn't let apps grab keys themselves, so there the desktop's GlobalShortcuts
//! portal binds it (GNOME asks the user to approve it once); everywhere else the
//! global-shortcut plugin does.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::settings::HotkeyAction;
use crate::state::AppState;
use crate::windows;

/// What the settings screen shows about the shortcut.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    /// "portal" (Wayland), "native", or "none" when nothing is registered yet.
    pub backend: &'static str,
    /// The keys actually bound, as the system describes them.
    pub trigger: Option<String>,
    pub error: Option<String>,
    /// Whether the system offers its own dialog for changing the keys (the portal does).
    pub system_configurable: bool,
}

#[derive(Default)]
pub struct Hotkey {
    status: Mutex<HotkeyStatus>,
}

impl Hotkey {
    pub fn status(&self) -> HotkeyStatus {
        self.status.lock().expect("hotkey lock poisoned").clone()
    }

    fn set(&self, status: HotkeyStatus) {
        if let Some(error) = &status.error {
            tracing::warn!(backend = status.backend, "global shortcut unavailable: {error}");
        } else {
            tracing::info!(backend = status.backend, trigger = ?status.trigger, "global shortcut ready");
        }
        *self.status.lock().expect("hotkey lock poisoned") = status;
    }
}

/// Registers the shortcut from the settings. Call again after the shortcut setting changes.
pub fn register(app: &AppHandle) {
    if uses_portal() {
        #[cfg(target_os = "linux")]
        portal::start(app);
    } else {
        register_native(app);
    }
}

/// Opens the system's own dialog for changing the shortcut, where there is one.
pub fn configure(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    if uses_portal() {
        portal::configure();
        return;
    }
    let _ = app;
}

/// The shortcut fired. `activation_token` lets the desktop hand focus to our window (Wayland).
fn fired(app: &AppHandle, activation_token: Option<String>) {
    tracing::debug!(has_activation_token = activation_token.is_some(), "global shortcut pressed");
    match app.state::<AppState>().settings.get().hotkey_action {
        HotkeyAction::OpenCapture => windows::show_capture_activated(app, activation_token),
        HotkeyAction::SaveClipboard => {
            let app = app.clone();
            // Off the shortcut's thread: reading the clipboard and writing files take a moment.
            std::thread::spawn(move || super::save_clipboard(&app));
        }
    }
}

fn uses_portal() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn set_status(app: &AppHandle, status: HotkeyStatus) {
    app.state::<AppState>().hotkey.set(status);
}

fn register_native(app: &AppHandle) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    let keys = app.state::<AppState>().settings.get().hotkey;
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    let result = shortcuts.on_shortcut(keys.as_str(), |app, _, event| {
        if event.state == ShortcutState::Pressed {
            fired(app, None);
        }
    });
    let status = match result {
        Ok(()) => HotkeyStatus { backend: "native", trigger: Some(keys), ..Default::default() },
        Err(e) => HotkeyStatus {
            backend: "none",
            error: Some(format!("{keys} couldn't be registered ({e}). Another app may be using it.")),
            ..Default::default()
        },
    };
    set_status(app, status);
}

/// "Ctrl+Alt+V" → "CTRL+ALT+v", the XDG shortcuts-spec form the portal expects.
fn portal_trigger(keys: &str) -> String {
    let mut parts: Vec<String> = keys.split('+').map(|p| p.trim().to_uppercase()).collect();
    if let Some(key) = parts.last_mut() {
        *key = key.to_lowercase();
    }
    parts.join("+")
}

#[cfg(target_os = "linux")]
mod portal {
    use std::sync::OnceLock;

    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    use tauri::{AppHandle, Manager};
    use tokio::sync::mpsc;

    use super::{HotkeyStatus, fired, portal_trigger, set_status};
    use crate::state::AppState;

    const SHORTCUT_ID: &str = "capture";

    /// Asks the running portal session to open the system's shortcut dialog.
    static CONFIGURE: OnceLock<mpsc::UnboundedSender<()>> = OnceLock::new();

    pub fn start(app: &AppHandle) {
        // The session lives for the whole run; binding again is done through `configure`.
        if CONFIGURE.get().is_some() {
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = CONFIGURE.set(tx);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = listen(&app, rx).await {
                set_status(
                    &app,
                    HotkeyStatus {
                        backend: "none",
                        error: Some(format!("The desktop didn't provide the shortcut: {e}")),
                        ..Default::default()
                    },
                );
            }
        });
    }

    pub fn configure() {
        if let Some(tx) = CONFIGURE.get() {
            let _ = tx.send(());
        }
    }

    async fn listen(app: &AppHandle, mut configure: mpsc::UnboundedReceiver<()>) -> ashpd::Result<()> {
        // Apps outside Flatpak/Snap have to tell the portal who they are, and the portal looks
        // them up by their desktop entry.
        let identifier = app.config().identifier.as_str();
        if let Err(e) = crate::capture::desktop_entry::ensure(identifier) {
            tracing::warn!("couldn't write the desktop entry: {e}");
        }
        let app_id = identifier.try_into()?;
        ashpd::register_host_app(app_id).await?;

        let keys = app.state::<AppState>().settings.get().hotkey;
        let portal = GlobalShortcuts::new().await?;
        let session = portal.create_session(Default::default()).await?;
        let shortcut = NewShortcut::new(SHORTCUT_ID, "Add something to Plexus")
            .preferred_trigger(portal_trigger(&keys).as_str());
        // The first time, the desktop asks the user to approve (and possibly change) the keys.
        let bound = portal.bind_shortcuts(&session, &[shortcut], None, Default::default()).await?.response()?;
        set_status(app, status_from(bound.shortcuts(), portal.version()));

        let mut activated = portal.receive_activated().await?;
        let mut changed = portal.receive_shortcuts_changed().await?;
        loop {
            tokio::select! {
                Some(event) = activated.next() => {
                    if event.shortcut_id() == SHORTCUT_ID {
                        let token = event
                            .options()
                            .get("activation_token")
                            .and_then(|v| String::try_from(v.clone()).ok());
                        fired(app, token);
                    }
                }
                Some(event) = changed.next() => {
                    set_status(app, status_from(event.shortcuts(), portal.version()));
                }
                Some(()) = configure.recv() => {
                    if let Err(e) = portal.configure_shortcuts(&session, None, Default::default()).await {
                        tracing::warn!("couldn't open the system shortcut dialog: {e}");
                    }
                }
                else => break,
            }
        }
        Ok(())
    }

    /// GNOME describes keys as "Press <Control><Alt>v"; show them as "Ctrl+Alt+V".
    pub(super) fn readable_trigger(description: &str) -> String {
        let keys = description.strip_prefix("Press ").unwrap_or(description);
        let mut parts = Vec::new();
        let mut rest = keys;
        while let Some(tail) = rest.strip_prefix('<') {
            let Some((modifier, after)) = tail.split_once('>') else { break };
            parts.push(match modifier {
                "Control" | "Primary" => "Ctrl",
                "Alt" => "Alt",
                "Shift" => "Shift",
                "Super" | "Meta" => "Super",
                other => other,
            }
            .to_string());
            rest = after;
        }
        if !rest.is_empty() {
            let mut key = rest.to_string();
            if key.chars().count() == 1 {
                key = key.to_uppercase();
            }
            parts.push(key);
        }
        if parts.is_empty() { description.to_string() } else { parts.join("+") }
    }

    fn status_from(shortcuts: &[ashpd::desktop::global_shortcuts::Shortcut], version: u32) -> HotkeyStatus {
        let trigger = shortcuts
            .iter()
            .find(|s| s.id() == SHORTCUT_ID)
            .map(|s| readable_trigger(s.trigger_description()))
            .filter(|t| !t.is_empty());
        HotkeyStatus {
            backend: "portal",
            error: trigger.is_none().then(|| "No keys are assigned yet.".to_string()),
            trigger,
            // ConfigureShortcuts arrived in version 2 of the portal.
            system_configurable: version >= 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::portal_trigger;

    #[cfg(target_os = "linux")]
    #[test]
    fn makes_gnome_trigger_descriptions_readable() {
        use super::portal::readable_trigger;
        assert_eq!(readable_trigger("Press <Control><Alt>v"), "Ctrl+Alt+V");
        assert_eq!(readable_trigger("Press <Shift><Super>space"), "Shift+Super+space");
        assert_eq!(readable_trigger("F9"), "F9");
    }

    #[test]
    fn converts_to_the_portal_trigger_format() {
        assert_eq!(portal_trigger("Ctrl+Alt+V"), "CTRL+ALT+v");
        assert_eq!(portal_trigger("Ctrl + Shift + Space"), "CTRL+SHIFT+space");
    }
}
