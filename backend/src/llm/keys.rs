//! API keys live in the system keychain (Secret Service on Linux, Keychain on macOS, Credential
//! Manager on Windows), one entry per provider preset. Never in settings, notes or the index.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::error::Result;

const SERVICE: &str = "plexus";

/// What was read from the keychain, so a call doesn't have to ask the system each time.
static CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> = LazyLock::new(Default::default);

fn entry(slot: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, slot)?)
}

pub fn get(slot: &str) -> Result<Option<String>> {
    if let Some(cached) = CACHE.lock().expect("key cache lock poisoned").get(slot) {
        return Ok(cached.clone());
    }
    let key = match entry(slot)?.get_password() {
        Ok(key) => Some(key),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => return Err(e.into()),
    };
    CACHE.lock().expect("key cache lock poisoned").insert(slot.to_string(), key.clone());
    Ok(key)
}

pub fn set(slot: &str, key: &str) -> Result<()> {
    entry(slot)?.set_password(key.trim())?;
    CACHE.lock().expect("key cache lock poisoned").insert(slot.to_string(), Some(key.trim().to_string()));
    Ok(())
}

pub fn clear(slot: &str) -> Result<()> {
    match entry(slot)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e.into()),
    }
    CACHE.lock().expect("key cache lock poisoned").insert(slot.to_string(), None);
    Ok(())
}
