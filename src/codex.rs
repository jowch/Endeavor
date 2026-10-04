//! What Codex needs that Claude doesn't (docs/codex-agent.md): its sign-in,
//! and the dialect that turns its adapter's sessions into the shape the app
//! knows from Claude's, so the rest of the app needs no Codex branches.
//!
//! Codex's modes are sandbox presets (`read-only`, `workspace-write`,
//! `agent`, `agent-full-access`), and planning is a separate option,
//! `collaboration_mode`. Endeavor keeps every session in `workspace-write`
//! (`INITIAL_AGENT_MODE`), where Codex asks before each notebook write, and
//! makes its own modes out of the runtime's gate: Manual and Ask to run differ
//! only in what the runtime holds, and Plan is the plan collaboration mode.
//! The app sees the three as the mode ids it knows: `default` (Manual),
//! `auto` (Ask to run and Auto), `plan`.

use std::collections::HashMap;

use agent_client_protocol::schema::v1::{
    ConfigOptionUpdate, CurrentModeUpdate, SessionConfigKind, SessionConfigOption, SessionConfigSelectOptions, SessionConfigValueId, SessionId, SessionMode,
    SessionModeId, SessionModeState, SessionUpdate,
};

use crate::agent::{ModeSwitch, Started};

/// Codex's sandbox preset for every Endeavor session.
const SANDBOX: &str = "workspace-write";
const SANDBOX_OPTION: &str = "mode";
const PLANNING_OPTION: &str = "collaboration_mode";
/// Codex's effort option, `effort` in the app (Claude's id).
const EFFORT_OPTION: &str = "reasoning_effort";
const FAST_OPTION: &str = "fast-mode";

/// Codex's sign-in on this computer, as `codex login status` tells it (exit 0
/// when signed in). It reads the user's own Codex login (`CODEX_HOME`, else
/// `~/.codex`) and changes nothing.
pub fn signed_in() -> Result<bool, String> {
    let out = crate::agent::codex_adapter(&["cli", "login", "status"])?.output().map_err(|e| format!("Couldn't check Codex's sign-in: {e}"))?;
    Ok(out.status.success())
}

/// Codex's browser sign-in (`codex-acp login`): it opens the ChatGPT sign-in
/// page and ends once the sign-in is done or fails (blocking).
pub fn log_in() -> Result<(), String> {
    let out = crate::agent::codex_adapter(&["login", "--client-name=endeavor", "--client-title=Endeavor"])?
        .output()
        .map_err(|e| format!("Couldn't start Codex's sign-in: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}

/// Codex's sign-in, as the app knows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Account {
    /// Not checked yet: Codex hasn't started.
    Unknown,
    SignedIn,
    SignedOut,
    /// Its browser sign-in is under way.
    SigningIn,
    /// The browser sign-in didn't finish.
    Failed,
}

impl Account {
    pub fn signed_out(self) -> bool {
        matches!(self, Account::SignedOut | Account::SigningIn | Account::Failed)
    }
}

/// One session's mode, as Endeavor's ids name it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    /// Codex's plan collaboration mode is on.
    planning: bool,
    /// The mode outside planning: `default` (Manual) or `auto`.
    working: &'static str,
}

impl View {
    fn mode(self) -> &'static str {
        if self.planning { "plan" } else { self.working }
    }
}

/// Translates each Codex session at the ACP boundary.
#[derive(Default)]
pub struct Dialect {
    sessions: HashMap<SessionId, View>,
}

impl Dialect {
    /// A session is up: Endeavor's modes in place of Codex's, its options as
    /// the app names them, and the option to set if Codex didn't start it in
    /// `workspace-write`. Every session starts in Manual, as Claude's do;
    /// the app then puts it in its start mode.
    pub fn started(&mut self, started: Started) -> (Started, Option<(String, SessionConfigValueId)>) {
        let view = View { planning: current(&started.config, PLANNING_OPTION).as_deref() == Some("plan"), working: "default" };
        self.sessions.insert(started.id.clone(), view);
        let fix = (current(&started.config, SANDBOX_OPTION).as_deref().is_some_and(|mode| mode != SANDBOX)).then(|| (SANDBOX_OPTION.to_owned(), SANDBOX.into()));
        let modes = SessionModeState::new(
            view.mode(),
            vec![
                SessionMode::new("default", "Manual"),
                SessionMode::new("auto", "Ask to run"),
                SessionMode::new("plan", "Plan"),
            ],
        );
        (Started { id: started.id, modes: Some(modes), config: app_options(started.config) }, fix)
    }

    /// An update from Codex, as the app takes it. Its own mode updates name
    /// sandbox presets, which the app never shows; a change of collaboration
    /// mode is a mode update. Leaving plan by approving the plan goes on in
    /// Ask to run, as Claude's Start does. A tool's result is the MCP content,
    /// as Claude's adapter gives it.
    pub fn update(&mut self, session: &SessionId, update: SessionUpdate) -> Vec<SessionUpdate> {
        match update {
            SessionUpdate::CurrentModeUpdate(_) => Vec::new(),
            SessionUpdate::ToolCall(mut call) => {
                call.raw_output = call.raw_output.map(tool_output);
                vec![SessionUpdate::ToolCall(call)]
            }
            SessionUpdate::ToolCallUpdate(mut update) => {
                update.fields.raw_output = update.fields.raw_output.map(tool_output);
                vec![SessionUpdate::ToolCallUpdate(update)]
            }
            SessionUpdate::ConfigOptionUpdate(update) => {
                let planning = current(&update.config_options, PLANNING_OPTION).map(|mode| mode == "plan");
                let mut updates = vec![SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(app_options(update.config_options)))];
                if let (Some(view), Some(planning)) = (self.sessions.get_mut(session), planning)
                    && view.planning != planning
                {
                    if view.planning {
                        view.working = "auto";
                    }
                    view.planning = planning;
                    updates.push(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(view.mode())));
                }
                updates
            }
            update => vec![update],
        }
    }

    /// Codex's reply to an option change: its options, as the app names them.
    pub fn options(&self, options: Vec<SessionConfigOption>) -> Vec<SessionConfigOption> {
        app_options(options)
    }

    /// The app switches a session to one of Endeavor's modes.
    pub fn set_mode(&mut self, session: &SessionId, mode: &SessionModeId) -> ModeSwitch {
        let view = self.sessions.entry(session.clone()).or_insert(View { planning: false, working: "default" });
        let (planning, working) = match mode.to_string().as_str() {
            "plan" => (true, view.working),
            "auto" => (false, "auto"),
            _ => (false, "default"),
        };
        let set = (planning != view.planning).then(|| (PLANNING_OPTION.to_owned(), SessionConfigValueId::from(if planning { "plan" } else { "default" })));
        *view = View { planning, working };
        ModeSwitch { mode: None, set, confirm: Some(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(view.mode()))) }
    }

    /// Codex's id for an option the app names (`effort` → `reasoning_effort`).
    pub fn option_id(&self, id: &str) -> String {
        if id == "effort" { EFFORT_OPTION.to_owned() } else { id.to_owned() }
    }

    pub fn closed(&mut self, session: &SessionId) {
        self.sessions.remove(session);
    }
}

/// Codex's options as the app shows them: no sandbox or collaboration mode
/// (Endeavor's modes stand in for them), effort under Claude's id, and fast
/// mode named as a speed ("Standard" or "Fast").
fn app_options(options: Vec<SessionConfigOption>) -> Vec<SessionConfigOption> {
    options
        .into_iter()
        .filter(|o| !matches!(o.id.to_string().as_str(), SANDBOX_OPTION | PLANNING_OPTION))
        .map(|mut o| {
            match o.id.to_string().as_str() {
                EFFORT_OPTION => o.id = "effort".into(),
                FAST_OPTION => {
                    if let SessionConfigKind::Select(select) = &mut o.kind
                        && let SessionConfigSelectOptions::Ungrouped(choices) = &mut select.options
                    {
                        for choice in choices {
                            match choice.value.to_string().as_str() {
                                "off" => choice.name = "Standard".into(),
                                "on" => choice.name = "Fast".into(),
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
            o
        })
        .collect()
}

/// Codex puts an MCP tool's reply in `rawOutput` as `{result: {content,
/// …}, error}`. The app reads the content (`celldiff::tool_json`), or, for a
/// call that failed before the tool replied, the error's text. Any other
/// output (a shell command's) stays as it is.
fn tool_output(raw: serde_json::Value) -> serde_json::Value {
    match (raw.pointer("/result/content"), raw.get("error")) {
        (Some(content), _) => content.clone(),
        (None, Some(serde_json::Value::String(error))) => serde_json::Value::String(error.clone()),
        (None, Some(error)) if !error.is_null() => error.get("message").cloned().unwrap_or_else(|| error.clone()),
        _ => raw,
    }
}

fn current(options: &[SessionConfigOption], id: &str) -> Option<String> {
    options.iter().find(|o| o.id.to_string() == id).and_then(|o| match &o.kind {
        SessionConfigKind::Select(select) => Some(select.current_value.to_string()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codex's `session/new` reply (adapter 2.1.1), trimmed of its model list.
    fn new_session() -> Started {
        let reply: serde_json::Value = serde_json::from_str(include_str!("fixtures/codex/session-new.json")).unwrap();
        let config: Vec<SessionConfigOption> = serde_json::from_value(reply["configOptions"].clone()).unwrap();
        Started::new(SessionId::new(reply["sessionId"].as_str().unwrap()), serde_json::from_value(reply["modes"].clone()).unwrap(), Some(config))
    }

    fn with(mut config: Vec<SessionConfigOption>, id: &str, value: &str) -> Vec<SessionConfigOption> {
        for o in &mut config {
            if o.id.to_string() == id
                && let SessionConfigKind::Select(select) = &mut o.kind
            {
                select.current_value = value.to_owned().into();
            }
        }
        config
    }

    fn ids(config: &[SessionConfigOption]) -> Vec<String> {
        config.iter().map(|o| o.id.to_string()).collect()
    }

    fn mode_of(updates: &[SessionUpdate]) -> Option<String> {
        updates.iter().find_map(|u| match u {
            SessionUpdate::CurrentModeUpdate(m) => Some(m.current_mode_id.to_string()),
            _ => None,
        })
    }

    #[test]
    fn a_codex_session_shows_endeavors_modes_and_its_own_model_effort_and_speed() {
        let mut dialect = Dialect::default();
        let (started, fix) = dialect.started(new_session());
        let modes = started.modes.unwrap();
        assert_eq!(modes.current_mode_id.to_string(), "default");
        assert_eq!(modes.available_modes.iter().map(|m| m.id.to_string()).collect::<Vec<_>>(), ["default", "auto", "plan"]);
        assert_eq!(ids(&started.config), ["model", "effort", "fast-mode"]);
        let effort = crate::session::config_choices(&started.config, "effort").unwrap();
        assert_eq!(effort.1.iter().map(|o| o.value.to_string()).collect::<Vec<_>>(), ["low", "medium", "high", "xhigh", "max", "ultra"]);
        let speed = crate::session::config_choices(&started.config, "fast-mode").unwrap();
        assert_eq!(speed.1.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["Standard", "Fast"]);
        // The capture's session was in Codex's default `agent` mode: Endeavor moves it.
        assert_eq!(fix, Some(("mode".to_owned(), "workspace-write".into())));
    }

    #[test]
    fn a_session_already_in_workspace_write_needs_no_fix() {
        let mut dialect = Dialect::default();
        let mut started = new_session();
        started.config = with(started.config, "mode", "workspace-write");
        assert_eq!(dialect.started(started).1, None);
    }

    #[test]
    fn plan_is_the_collaboration_mode_and_manual_and_ask_to_run_change_nothing_in_codex() {
        let mut dialect = Dialect::default();
        let (started, _) = dialect.started(new_session());
        let id = started.id.clone();

        let to_auto = dialect.set_mode(&id, &"auto".into());
        assert_eq!((to_auto.mode, to_auto.set), (None, None), "Ask to run is the runtime's gate, not Codex's");
        assert_eq!(mode_of(&to_auto.confirm.into_iter().collect::<Vec<_>>()).as_deref(), Some("auto"));

        let to_plan = dialect.set_mode(&id, &"plan".into());
        assert_eq!(to_plan.set, Some(("collaboration_mode".to_owned(), "plan".into())));
        assert_eq!(mode_of(&to_plan.confirm.into_iter().collect::<Vec<_>>()).as_deref(), Some("plan"));

        let to_manual = dialect.set_mode(&id, &"default".into());
        assert_eq!(to_manual.set, Some(("collaboration_mode".to_owned(), "default".into())));
        assert_eq!(dialect.option_id("effort"), "reasoning_effort");
        assert_eq!(dialect.option_id("model"), "model");
    }

    #[test]
    fn approving_a_plan_goes_on_in_ask_to_run() {
        let mut dialect = Dialect::default();
        let (started, _) = dialect.started(new_session());
        let id = started.id.clone();
        let config = new_session().config;
        dialect.set_mode(&id, &"plan".into());

        // Codex's own confirmation of plan changes nothing more.
        let planning = dialect.update(&id, SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(with(config.clone(), "collaboration_mode", "plan"))));
        assert_eq!(mode_of(&planning), None);
        let SessionUpdate::ConfigOptionUpdate(shown) = &planning[0] else { panic!() };
        assert_eq!(ids(&shown.config_options), ["model", "effort", "fast-mode"]);

        // implement_plan: the adapter leaves plan and says so in its options.
        let approved = dialect.update(&id, SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(with(config, "collaboration_mode", "default"))));
        assert_eq!(mode_of(&approved).as_deref(), Some("auto"));
    }

    #[test]
    fn codex_mode_updates_and_thread_status_never_change_the_apps_mode() {
        let mut dialect = Dialect::default();
        let (started, _) = dialect.started(new_session());
        assert!(dialect.update(&started.id, SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("agent"))).is_empty());
        let status: SessionUpdate = serde_json::from_value(serde_json::json!({"sessionUpdate": "session_info_update", "_meta": {"codex": {"threadStatus": {"type": "idle"}}}})).unwrap();
        assert_eq!(dialect.update(&started.id, status.clone()).len(), 1, "passed on; the session ignores an update without a title");
    }
}
