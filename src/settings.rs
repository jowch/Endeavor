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
    /// The R for R notebooks on this computer: an Rscript path, or a shell line
    /// such as `module load R`. None is the login shell's Rscript.
    pub r: Option<String>,
    /// New sessions run notebook code without asking first.
    pub run_without_asking: bool,
    /// The mode new sessions start in: the last Manual, Ask to run or Auto
    /// picked (`remember_mode`). None is Manual.
    pub mode: Option<crate::session::Mode>,
    /// Light, dark, or follow macOS: the whole window, notebook included.
    pub appearance: Appearance,
    pub notebook_theme: NotebookTheme,
    /// The panes as the user left them.
    pub layout: Layout,
    /// Claude's config values last picked (e.g. "model", "effort"), applied to
    /// each session as it starts: the adapter only sets them per session.
    pub agent_config: std::collections::BTreeMap<String, String>,
    /// Codex's, the same way (its effort values and fast mode differ).
    pub codex_config: std::collections::BTreeMap<String, String>,
    /// Antigravity's, the same way (its model only).
    pub antigravity_config: std::collections::BTreeMap<String, String>,
    /// The agent new sessions start with: the last one picked.
    pub agent: crate::agent::Agent,
    /// The sidebar's Status / Where / Group by / Sort by / Show empty folders
    /// filter menu (src/sidebar_filter.rs). Replaces the old `show_archived`
    /// bool; `Settings::load` migrates a saved `show_archived: true` to
    /// `StatusFilter::All`.
    pub sidebar_filters: SidebarFilters,
    /// Folders whose sidebar heading is collapsed (its name reads "name ›").
    pub collapsed_folders: HashSet<Place>,
    /// The notebook's zoom (⌘= / ⌘− / ⌘0); 0 means unset (1.0).
    pub zoom: f64,
    /// The chat's text size, as a multiple of the default (⌘= / ⌘− / ⌘0
    /// after a click in the chat); 0 means unset (1.0).
    pub chat_zoom: f64,
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
    /// The kind of new notebook last picked on the notebook chip; None is Julia's.
    pub notebook_kind: Option<wire::backend::Backend>,
}

impl Settings {
    /// Keep `mode` for new sessions, if it is one of the app's modes other
    /// than Plan, which is a step in a task rather than a way of working.
    /// Whether it changed.
    pub fn remember_mode(&mut self, mode: &crate::session::Mode) -> bool {
        let keeps = mode.agent != "plan" && crate::session::app_modes().iter().any(|c| c.mode() == *mode);
        if !keeps || self.mode.as_ref() == Some(mode) {
            return false;
        }
        self.mode = Some(mode.clone());
        true
    }

    /// Where the kept mode sits in `app_modes()`: Manual when none is kept.
    pub fn start_mode(&self) -> usize {
        self.mode.as_ref().and_then(|m| crate::session::app_modes().iter().position(|c| c.mode() == *m)).unwrap_or(crate::session::MANUAL)
    }
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
    /// An agent's config values last picked.
    pub fn config_picks(&self, agent: crate::agent::Agent) -> &std::collections::BTreeMap<String, String> {
        match agent {
            crate::agent::Agent::Claude => &self.agent_config,
            crate::agent::Agent::Codex => &self.codex_config,
            crate::agent::Agent::Antigravity => &self.antigravity_config,
        }
    }

    pub fn config_picks_mut(&mut self, agent: crate::agent::Agent) -> &mut std::collections::BTreeMap<String, String> {
        match agent {
            crate::agent::Agent::Claude => &mut self.agent_config,
            crate::agent::Agent::Codex => &mut self.codex_config,
            crate::agent::Agent::Antigravity => &mut self.antigravity_config,
        }
    }

    pub fn load() -> Self {
        let file = crate::install::app_dir().ok().map(|d| d.join(FILE));
        // Bytes, as in `load_json_at`: a file cut mid-character doesn't parse.
        let bytes = file.as_ref().and_then(|f| std::fs::read(f).ok());
        let text = bytes.as_deref().and_then(|b| std::str::from_utf8(b).ok());
        let mut settings: Settings = match bytes.as_deref().map(serde_json::from_slice) {
            Some(Ok(settings)) => settings,
            Some(Err(e)) => {
                // Keep the file that doesn't parse, rather than saving defaults over it.
                if let Some(f) = &file {
                    crate::set_aside(f, &e);
                }
                Settings::default()
            }
            None => Settings::default(),
        };
        // `show_archived: true` ("All, including archived") is now `StatusFilter::All`;
        // `false` needs no migration, since `StatusFilter::Active` is still the default.
        let showed_archived = text.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).and_then(|v| v.get("show_archived").and_then(serde_json::Value::as_bool));
        if showed_archived == Some(true) {
            settings.sidebar_filters.status = crate::sidebar_filter::StatusFilter::All;
        }
        settings
    }

    pub fn save(&self) {
        // ponytail: best effort, like the app's other small files; a failed save
        // keeps the last good file, so it only loses this change.
        if let (Ok(dir), Ok(json)) = (crate::install::app_dir(), serde_json::to_string_pretty(self))
            && let Err(e) = crate::write_atomic(&dir.join(FILE), json.as_bytes())
        {
            eprintln!("Couldn't save the settings: {e}");
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

/// WebView2's `prefers-color-scheme` follows the theme set on it.
#[cfg(windows)]
pub fn set_webview_appearance(webview: &wry::WebView, light: bool) {
    use wry::WebViewExtWindows;
    let _ = webview.set_theme(if light { wry::Theme::Light } else { wry::Theme::Dark });
}

#[cfg(test)]
mod tests {
    use super::Settings;
    use crate::session::{Mode, app_modes};

    #[test]
    fn new_sessions_keep_the_last_mode_but_plan() {
        let mut settings = Settings::default();
        let mode = |name: &str| app_modes().into_iter().find(|c| c.name == name).unwrap().mode();
        assert!(settings.remember_mode(&mode("Auto")));
        assert!(!settings.remember_mode(&mode("Auto")), "the same mode again changes nothing");
        assert!(!settings.remember_mode(&mode("Plan")));
        assert!(!settings.remember_mode(&Mode { agent: "bypassPermissions".into(), run_without_asking: false }), "only the app's modes");
        assert_eq!(settings.mode, Some(mode("Auto")));
        assert_eq!(app_modes()[settings.start_mode()].name, "Auto");
        assert_eq!(Settings::default().start_mode(), crate::session::MANUAL);
        assert!(settings.remember_mode(&mode("Ask to run")));
        // Saved and read back, as Settings::load reads it; a file from before has none.
        let back: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(back.mode, Some(mode("Ask to run")));
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().mode, None);
    }
}
