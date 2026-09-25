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
    /// Light or dark (only the notebook follows it so far; the app is dark).
    pub appearance: Appearance,
    pub notebook_theme: NotebookTheme,
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    /// Follow macOS.
    System,
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotebookTheme {
    /// Pluto's colours mapped to Endeavor's (dark only for now).
    #[default]
    Endeavor,
    /// Pluto's own look.
    Pluto,
}

impl NotebookTheme {
    pub fn name(self) -> &'static str {
        match self {
            NotebookTheme::Endeavor => "endeavor",
            NotebookTheme::Pluto => "pluto",
        }
    }
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

/// Make the notebook webview light, dark, or follow macOS: Pluto's own themes
/// switch on the page's `prefers-color-scheme`, which follows the view's appearance.
#[cfg(target_os = "macos")]
pub fn set_webview_appearance(webview: &wry::WebView, appearance: Appearance) {
    use objc2_app_kit::{NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
    use wry::WebViewExtMacOS;
    let name = match appearance {
        Appearance::Dark => Some(unsafe { NSAppearanceNameDarkAqua }),
        Appearance::Light => Some(unsafe { NSAppearanceNameAqua }),
        Appearance::System => None,
    };
    let look = name.and_then(NSAppearance::appearanceNamed);
    webview.webview().setAppearance(look.as_deref());
}
