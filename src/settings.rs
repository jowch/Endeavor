//! The user's choices from the Settings screen, kept in Application Support.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hosts::Place;
use crate::sidebar_filter::SidebarFilters;

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
    /// Light, dark, or follow macOS: the whole window, notebook included.
    pub appearance: Appearance,
    pub notebook_theme: NotebookTheme,
    /// The panes as the user left them.
    pub layout: Layout,
    /// The agent config values last picked (e.g. "model", "effort"), applied to
    /// each session as it starts: the adapter only sets them per session.
    pub agent_config: std::collections::BTreeMap<String, String>,
    /// The sidebar's Status / Where / Group by / Sort by / Show empty folders
    /// filter menu (src/sidebar_filter.rs). Replaces the old `show_archived`
    /// bool; `Settings::load` migrates a saved `show_archived: true` to
    /// `StatusFilter::All`.
    pub sidebar_filters: SidebarFilters,
    /// Folders whose sidebar heading is collapsed (its name reads "name ›").
    pub collapsed_folders: HashSet<Place>,
    /// The notebook's zoom (⌘= / ⌘− / ⌘0); 0 means unset (1.0).
    pub zoom: f64,
    /// Open notebooks with no turns, edits or running cells for this long stop.
    pub idle_stop: IdleStop,
    /// Julia and its open notebooks keep running after Endeavor quits; the next
    /// launch reconnects to them.
    pub keep_running: bool,
    /// The one-time tip on how to add a file ("Two ways to add your file") is done.
    pub file_tip_seen: bool,
    /// The one-time tip under the notebook header's Point button is done.
    pub point_tip_seen: bool,
    /// How Claude was last signed in to, for signing in again the same way.
    pub sign_in_method: Option<crate::signin::Method>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdleStop {
    Hours12,
    Hours24,
    #[default]
    Hours48,
    Week,
    Never,
}

impl IdleStop {
    pub const ALL: [(IdleStop, &str); 5] = [
        (IdleStop::Hours12, "12 hours"),
        (IdleStop::Hours24, "24 hours"),
        (IdleStop::Hours48, "48 hours"),
        (IdleStop::Week, "1 week"),
        (IdleStop::Never, "Never"),
    ];

    /// The runtime's limit; 0 never stops.
    pub fn hours(self) -> u32 {
        match self {
            IdleStop::Hours12 => 12,
            IdleStop::Hours24 => 24,
            IdleStop::Hours48 => 48,
            IdleStop::Week => 168,
            IdleStop::Never => 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub chat_width: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self { sidebar_open: true, sidebar_width: 232., chat_width: 440. }
    }
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
    /// Pluto's colours mapped to Endeavor's.
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
        let mut settings: Settings = text.as_deref().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
        // `show_archived: true` ("All, including archived") is now `StatusFilter::All`;
        // `false` needs no migration, since `StatusFilter::Active` is still the default.
        let showed_archived = text.as_deref().and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).and_then(|v| v.get("show_archived").and_then(serde_json::Value::as_bool));
        if showed_archived == Some(true) {
            settings.sidebar_filters.status = crate::sidebar_filter::StatusFilter::All;
        }
        settings
    }

    pub fn save(&self) {
        // ponytail: best effort, like the app's other small files; a failed save
        // only loses the change on the next launch.
        if let (Ok(dir), Ok(json)) = (crate::install::app_dir(), serde_json::to_string_pretty(self)) {
            let _ = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(FILE), json));
        }
    }
}

/// Make the notebook webview light or dark, as the rest of the window resolved
/// it: Pluto's own themes and Endeavor's page colours switch on the page's
/// `prefers-color-scheme`, which follows the view's appearance.
#[cfg(target_os = "macos")]
pub fn set_webview_appearance(webview: &wry::WebView, light: bool) {
    use objc2_app_kit::{NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
    use wry::WebViewExtMacOS;
    let name = if light { unsafe { NSAppearanceNameAqua } } else { unsafe { NSAppearanceNameDarkAqua } };
    let look = NSAppearance::appearanceNamed(name);
    webview.webview().setAppearance(look.as_deref());
}

/// WebKitGTK's `prefers-color-scheme` follows GTK's dark-theme preference.
#[cfg(target_os = "linux")]
pub fn set_webview_appearance(_: &wry::WebView, light: bool) {
    use gtk::prelude::GtkSettingsExt;
    let Some(gtk) = gtk::Settings::default() else { return };
    gtk.set_gtk_application_prefer_dark_theme(!light);
}
