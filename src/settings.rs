//! The user's choices from the Settings screen, kept in Application Support.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Load the user's own Claude Code setup (user settings, their MCP servers)
    /// into new sessions, besides Endeavor's plugin and the project's settings.
    pub personal_claude: bool,
    /// A julia binary to use instead of Endeavor's own.
    pub julia: Option<PathBuf>,
    /// New sessions run notebook code without asking first.
    pub run_without_asking: bool,
}

impl Settings {
    pub fn load() -> Self {
        let text = crate::install::app_dir().ok().and_then(|d| std::fs::read_to_string(d.join(FILE)).ok());
        text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        // ponytail: best effort, like the app's other small files; a failed save
        // only loses the change on the next launch.
        if let (Ok(dir), Ok(json)) = (crate::install::app_dir(), serde_json::to_string_pretty(self)) {
            let _ = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(FILE), json));
        }
    }
}
