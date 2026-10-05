//! Clipboard watching (opt-in): while it is on, text you copy anywhere is saved on its own.
//!
//! It polls the clipboard, so it can't tell which app a copy came from. To stay out of the way
//! it ignores what was already on the clipboard when it was switched on, very short copies, huge
//! ones, and anything the secret detector flags (a password manager's copy is dropped that way,
//! but only if it looks like a secret). On GNOME/Wayland the desktop doesn't let background apps
//! read the clipboard, so it may see nothing there.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use super::{announce, clipboard, save};
use crate::state::AppState;

const INTERVAL: Duration = Duration::from_millis(800);
/// Copies shorter than this (a word, a number) aren't worth a node.
const MIN_CHARS: usize = 12;
const MAX_BYTES: usize = 200_000;

/// Whether a copy is the kind of thing worth saving on its own.
fn worth_saving(text: &str) -> bool {
    text.len() <= MAX_BYTES && text.trim().chars().count() >= MIN_CHARS
}

fn fingerprint(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Starts the watcher thread. It idles (cheaply) while the setting is off.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    let watcher = move || {
        let state = app.state::<AppState>();
        let (mut last, mut was_on, mut warned) = (None, false, false);
        loop {
            std::thread::sleep(INTERVAL);
            if !state.settings.get().watch_clipboard {
                was_on = false;
                continue;
            }
            let current = match clipboard::text() {
                Ok(text) => text,
                Err(e) => {
                    if !warned {
                        tracing::warn!("can't watch the clipboard: {e}");
                        warned = true;
                    }
                    continue;
                }
            };
            let seen = current.as_deref().map(fingerprint);
            // What was on the clipboard when watching began isn't saved.
            if !was_on || seen == last {
                (was_on, last) = (true, seen);
                continue;
            }
            last = seen;
            let Some(text) = current.filter(|t| worth_saving(t)) else { continue };
            match save::text(&state.library, &state.db, &text, false) {
                Ok(saved) => {
                    announce(&app, saved);
                }
                Err(e) => tracing::warn!("couldn't save a copy: {e}"),
            }
        }
    };
    if let Err(e) = std::thread::Builder::new().name("clipboard-watch".into()).spawn(watcher) {
        tracing::error!("couldn't start clipboard watching: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_short_and_huge_copies() {
        assert!(!worth_saving("  42  ") && !worth_saving("hello"));
        assert!(worth_saving("a sentence worth keeping"));
        assert!(!worth_saving(&"x".repeat(MAX_BYTES + 1)));
    }

    #[test]
    fn same_text_same_fingerprint() {
        assert_eq!(fingerprint("a"), fingerprint("a"));
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }
}
