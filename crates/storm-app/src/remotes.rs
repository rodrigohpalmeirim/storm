//! Remembers the last remote server the user connected to.
//!
//! Stored as JSON in the app's config dir. The token is stored in plain text there
//! for now (it's a user-only directory); moving it to the OS keychain is a TODO.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePrefs {
    pub url: String,
    pub token: String,
}

fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "storm", "storm").map(|d| d.config_dir().join("remote.json"))
}

pub fn load() -> Option<RemotePrefs> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(prefs: &RemotePrefs) {
    let Some(p) = path() else { return };
    let res = p
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| std::fs::write(&p, serde_json::to_string_pretty(prefs).unwrap_or_default()));
    if let Err(e) = res {
        tracing::warn!("saving {}: {e}", p.display());
    }
}

pub fn forget() {
    if let Some(p) = path() {
        let _ = std::fs::remove_file(p);
    }
}
