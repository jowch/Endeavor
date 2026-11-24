use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hosts::Place;
use crate::sidebar_filter::SidebarFilters;

const FILE: &str = "settings.json";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub claude_setup: Option<bool>,
    pub julia: Option<PathBuf>,
    pub endeavors_julia: bool,
    pub r: Option<String>,
    pub run_without_asking: bool,
    pub mode: Option<crate::session::Mode>,
    pub appearance: Appearance,
    pub notebook_theme: NotebookTheme,
    pub layout: Layout,
    pub agent_config: std::collections::BTreeMap<String, String>,
    pub codex_config: std::collections::BTreeMap<String, String>,
    pub antigravity_config: std::collections::BTreeMap<String, String>,
    pub agent: crate::agent::Agent,
    pub sidebar_filters: SidebarFilters,
    pub collapsed_folders: HashSet<Place>,
    pub zoom: f64,
    pub chat_zoom: f64,
    pub idle_stop: IdleStop,
    pub file_tip_seen: bool,
    pub point_tip_seen: bool,
    pub sign_in_method: Option<crate::signin::Method>,
    pub notebook_kind: Option<wire::backend::Backend>,
}

impl Settings {
    pub fn remember_mode(&mut self, mode: &crate::session::Mode) -> bool {
        let keeps = mode.agent != "plan" && crate::session::app_modes().iter().any(|c| c.mode() == *mode);
        if !keeps || self.mode.as_ref() == Some(mode) {
            return false;
        }
        self.mode = Some(mode.clone());
        true
    }

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
    System,
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotebookTheme {
    #[default]
    Endeavor,
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
    pub fn julia_preference(&self) -> endeavor_mcp::julia::Preference {
        use endeavor_mcp::julia::Preference;
        match (&self.julia, self.endeavors_julia) {
            (Some(path), _) => Preference::Path(path.clone()),
            (None, true) => Preference::Endeavors,
            (None, false) => Preference::Auto,
        }
    }

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

    pub fn own_claude_setup(&self) -> bool {
        self.claude_setup.unwrap_or(true)
    }

    pub fn load() -> Self {
        let file = crate::install::app_dir().ok().map(|d| d.join(FILE));
        let bytes = file.as_ref().and_then(|f| std::fs::read(f).ok());
        let text = bytes.as_deref().and_then(|b| std::str::from_utf8(b).ok());
        let mut settings: Settings = match bytes.as_deref().map(serde_json::from_slice) {
            Some(Ok(settings)) => settings,
            Some(Err(e)) => {
                if let Some(f) = &file {
                    crate::set_aside(f, &e);
                }
                Settings::default()
            }
            None => Settings::default(),
        };
        let showed_archived = text.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).and_then(|v| v.get("show_archived").and_then(serde_json::Value::as_bool));
        if showed_archived == Some(true) {
            settings.sidebar_filters.status = crate::sidebar_filter::StatusFilter::All;
        }
        settings
    }

    pub fn save(&self) {
        if let (Ok(dir), Ok(json)) = (crate::install::app_dir(), serde_json::to_string_pretty(self))
            && let Err(e) = crate::write_atomic(&dir.join(FILE), json.as_bytes())
        {
            eprintln!("Couldn't save the settings: {e}");
        }
    }
}

#[cfg(target_os = "macos")]
pub fn set_webview_appearance(webview: &wry::WebView, light: bool) {
    use objc2_app_kit::{NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
    use wry::WebViewExtMacOS;
    let name = if light { unsafe { NSAppearanceNameAqua } } else { unsafe { NSAppearanceNameDarkAqua } };
    let look = NSAppearance::appearanceNamed(name);
    webview.webview().setAppearance(look.as_deref());
}

#[cfg(target_os = "linux")]
pub fn set_webview_appearance(_: &wry::WebView, light: bool) {
    use gtk::prelude::GtkSettingsExt;
    let Some(gtk) = gtk::Settings::default() else { return };
    gtk.set_gtk_application_prefer_dark_theme(!light);
}

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
        let back: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(back.mode, Some(mode("Ask to run")));
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().mode, None);
    }

    #[test]
    fn the_users_claude_setup_is_on_unless_they_turned_it_off() {
        assert!(Settings::default().own_claude_setup());
        assert!(serde_json::from_str::<Settings>(r#"{"personal_claude": false}"#).unwrap().own_claude_setup());
        assert!(serde_json::from_str::<Settings>(r#"{"personal_claude": true}"#).unwrap().own_claude_setup());
        let off = Settings { claude_setup: Some(false), ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
        assert!(!back.own_claude_setup());
    }
}
