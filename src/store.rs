//! The collector's few files, in `%LOCALAPPDATA%\BigRocketCampCollector`: queue, settings and protected token.

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("BigRocketCampCollector")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// The paired player's name, shown in the window; not a secret.
    pub player: Option<String>,
    /// Once the player has chosen whether the collector starts with Windows, it never decides again.
    pub autostart_chosen: bool,
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let saved: Value =
            fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Self { player: saved["player"].as_str().map(str::to_owned), autostart_chosen: saved["autostartChosen"] == true }
    }

    pub fn save(&self, path: &Path) {
        let body = json!({ "player": self.player, "autostartChosen": self.autostart_chosen }).to_string();
        let _ = fs::write(path, body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_survive_a_missing_file() {
        let path = std::env::temp_dir().join(format!("brc-collector-settings-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);
        assert_eq!(Settings::load(&path), Settings::default());
        let settings = Settings { player: Some("player-one".into()), autostart_chosen: true };
        settings.save(&path);
        assert_eq!(Settings::load(&path), settings);
        let _ = fs::remove_file(&path);
    }
}
