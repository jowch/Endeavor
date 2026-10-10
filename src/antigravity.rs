//! What Antigravity needs that Claude doesn't (docs/antigravity-agent.md):
//! its sign-in, and the dialect that turns its server's sessions into the
//! shape the app knows from Claude's.
//!
//! Antigravity's modes are permission presets (`default`, `auto_edit`,
//! `yolo`), the same as its `mode` option. Endeavor keeps every session in
//! `default`, where it asks before every tool call, and makes its own modes
//! out of the runtime's gate, as for Codex: Manual and Ask to run differ only
//! in what the runtime holds. It has no plan mode.
//!
//! Its notebook calls are titled `notebook_<tool>` with the arguments under
//! `rawInput.arguments`, and say which MCP tool they are in `_meta.mcp`. Their
//! live output is only a label ("New notebook"), so the app asks the runtime
//! for the result (`Effect::FetchResult`).

use agent_client_protocol::schema::v1::{
    ConfigOptionUpdate, CurrentModeUpdate, Meta, PermissionOptionKind, RequestPermissionRequest, SessionConfigKind, SessionConfigOption, SessionConfigValueId, SessionMode,
    SessionModeId, SessionModeState, SessionUpdate,
};

use crate::agent::{ModeSwitch, Started};

/// The sign-in Endeavor offers: a Google account, in the browser.
pub const SIGN_IN_METHOD: &str = "oauth-personal";

/// Antigravity's preset for every Endeavor session.
const PRESET: &str = "default";
const MODE_OPTION: &str = "mode";

/// Signed in on this computer: its server keeps the sign-in in
/// `~/.gemini/antigravity-acp/acp_token.json`, which a sign-out or a sign-in
/// that times out removes. Changes nothing.
pub fn signed_in() -> bool {
    wire::files::home().join(".gemini").join("antigravity-acp").join("acp_token.json").is_file()
}

/// Translates each Antigravity session at the ACP boundary.
#[derive(Default)]
pub struct Dialect;

impl Dialect {
    /// A session is up: Endeavor's modes in place of Antigravity's, its
    /// options without the mode, and the mode to set if it isn't `default`.
    pub fn started(&mut self, started: Started) -> (Started, Option<(String, SessionConfigValueId)>) {
        let preset = current(&started.config, MODE_OPTION).or_else(|| started.modes.as_ref().map(|m| m.current_mode_id.to_string()));
        let fix = preset.is_some_and(|p| p != PRESET).then(|| (MODE_OPTION.to_owned(), PRESET.into()));
        let modes = SessionModeState::new("default", vec![SessionMode::new("default", "Manual"), SessionMode::new("auto", "Ask to run")]);
        (Started { id: started.id, modes: Some(modes), config: app_options(started.config) }, fix)
    }

    /// An update from Antigravity, as the app takes it. Its own mode updates
    /// name presets, which the app never shows. A notebook call gets the name
    /// and input Claude's adapter gives it.
    pub fn update(&mut self, update: SessionUpdate) -> Vec<SessionUpdate> {
        match update {
            SessionUpdate::CurrentModeUpdate(_) => Vec::new(),
            SessionUpdate::ConfigOptionUpdate(update) => vec![SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(app_options(update.config_options)))],
            SessionUpdate::ToolCall(mut call) => {
                if let Some(tool) = notebook_tool(call.meta.as_ref()) {
                    call.title = format!("{}{tool}", crate::celldiff::TOOL_PREFIX);
                    call.raw_input = call.raw_input.map(arguments);
                } else {
                    not_a_notebook_call(&mut call.title, &call.raw_input);
                }
                vec![SessionUpdate::ToolCall(call)]
            }
            SessionUpdate::ToolCallUpdate(mut update) => {
                if let Some(tool) = notebook_tool(update.meta.as_ref()) {
                    if update.fields.title.is_some() {
                        update.fields.title = Some(format!("{}{tool}", crate::celldiff::TOOL_PREFIX));
                    }
                    update.fields.raw_input = update.fields.raw_input.map(arguments);
                } else if let Some(title) = &mut update.fields.title {
                    not_a_notebook_call(title, &update.fields.raw_input);
                }
                vec![SessionUpdate::ToolCallUpdate(update)]
            }
            update => vec![update],
        }
    }

    /// A permission request as the app takes it: a notebook call named as
    /// its updates are, anything else never named like one, and without
    /// "Allow Always". Antigravity keeps that answer as a rule of its own,
    /// which the app can't show or remove and which would hold in Manual too.
    pub fn permission(&self, request: &mut RequestPermissionRequest) {
        let call = &mut request.tool_call;
        if let Some(tool) = notebook_tool(call.meta.as_ref()) {
            call.fields.title = Some(format!("{}{tool}", crate::celldiff::TOOL_PREFIX));
            call.fields.raw_input = call.fields.raw_input.take().map(arguments);
        } else if let Some(title) = &mut call.fields.title {
            not_a_notebook_call(title, &call.fields.raw_input);
        }
        request.options.retain(|o| o.kind != PermissionOptionKind::AllowAlways);
    }

    /// Antigravity's reply to an option change: its options, as the app shows them.
    pub fn options(&self, options: Vec<SessionConfigOption>) -> Vec<SessionConfigOption> {
        app_options(options)
    }

    /// The app switches a session to one of Endeavor's modes: nothing changes
    /// in Antigravity, which stays in `default`.
    pub fn set_mode(&mut self, mode: &SessionModeId) -> ModeSwitch {
        let mode = if mode.to_string() == "auto" { "auto" } else { "default" };
        ModeSwitch { mode: None, set: None, confirm: Some(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(mode))) }
    }
}

/// The notebook tool a call's `_meta` names, when its server is Endeavor's.
fn notebook_tool(meta: Option<&Meta>) -> Option<String> {
    let mcp = meta?.get("mcp")?;
    let tool = mcp.get("tool")?.as_str()?;
    (mcp.get("server")?.as_str()? == crate::celldiff::MCP_SERVER && endeavor_mcp::is_tool(tool)).then(|| tool.to_owned())
}

/// A call that isn't a notebook call by its `_meta` keeps its title unless
/// the app would take that title for a notebook call's. A shell command's
/// title is its command line, which the model writes, so
/// `mcp__notebook__read_cell; Remove-Item …` would otherwise pass for a read.
fn not_a_notebook_call(title: &mut String, input: &Option<serde_json::Value>) {
    let (mut named, mut input) = (title.clone(), input.clone());
    crate::celldiff::name_notebook_call(&mut named, &mut input);
    if crate::celldiff::notebook_tool(&named).is_some() {
        // A word of its own, so neither the prefix nor a title naming only
        // the server and a tool matches any more.
        *title = format!("Antigravity: {title}");
    }
}

/// A notebook call's arguments: Antigravity's `rawInput` wraps them as
/// `arguments`, beside copies of some of them.
fn arguments(input: serde_json::Value) -> serde_json::Value {
    match input.get("arguments") {
        Some(args) if args.is_object() => args.clone(),
        _ => input,
    }
}

/// Antigravity's options as the app shows them: no mode (Endeavor's modes
/// stand in for it).
fn app_options(options: Vec<SessionConfigOption>) -> Vec<SessionConfigOption> {
    options.into_iter().filter(|o| o.id.to_string() != MODE_OPTION).collect()
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
    use agent_client_protocol::schema::v1::SessionId;

    /// Antigravity's `session/new` reply (agy_acp_server 1.3.0), trimmed of most of its models.
    fn new_session(mode: &str) -> Started {
        let reply: serde_json::Value = serde_json::from_str(&include_str!("fixtures/antigravity/session-new.json").replace("\"currentValue\": \"default\"", &format!("\"currentValue\": \"{mode}\""))).unwrap();
        let config: Vec<SessionConfigOption> = serde_json::from_value(reply["configOptions"].clone()).unwrap();
        Started::new(SessionId::new(reply["sessionId"].as_str().unwrap()), serde_json::from_value(reply["modes"].clone()).unwrap(), Some(config))
    }

    fn update(json: &str) -> SessionUpdate {
        let message: serde_json::Value = serde_json::from_str(json).unwrap();
        serde_json::from_value(message["params"]["update"].clone()).unwrap()
    }

    #[test]
    fn a_session_shows_endeavors_modes_and_only_the_model() {
        let (started, fix) = Dialect.started(new_session("default"));
        let modes = started.modes.unwrap();
        assert_eq!(modes.current_mode_id.to_string(), "default");
        assert_eq!(modes.available_modes.iter().map(|m| m.id.to_string()).collect::<Vec<_>>(), ["default", "auto"]);
        assert_eq!(started.config.iter().map(|o| o.id.to_string()).collect::<Vec<_>>(), ["model"]);
        assert_eq!(fix, None);
        let (_, fix) = Dialect.started(new_session("yolo"));
        assert_eq!(fix, Some(("mode".to_owned(), "default".into())), "a session left in yolo goes back to asking");
    }

    #[test]
    fn modes_change_nothing_in_antigravity() {
        for mode in ["auto", "default"] {
            let switch = Dialect.set_mode(&mode.into());
            assert_eq!((switch.mode, switch.set), (None, None));
            assert!(matches!(switch.confirm, Some(SessionUpdate::CurrentModeUpdate(m)) if m.current_mode_id.to_string() == mode));
        }
        assert!(Dialect.update(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("yolo"))).is_empty());
    }

    #[test]
    fn a_notebook_call_gets_claudes_name_and_its_own_arguments() {
        let updates: [SessionUpdate; 1] = Dialect.update(update(include_str!("fixtures/antigravity/tool-call.json"))).try_into().ok().unwrap();
        let [SessionUpdate::ToolCall(call)] = updates else { panic!() };
        assert_eq!(call.title, "mcp__notebook__new_notebook");
        assert_eq!(call.raw_input, Some(serde_json::json!({"path": "C:\\Users\\me\\notebooks\\sum.jl"})));
        assert_eq!(crate::celldiff::notebook_tool(&call.title), Some("new_notebook"));

        // The completed update has no _meta: passed on as it is, its label
        // isn't a result, so the session asks the runtime.
        let done = update(include_str!("fixtures/antigravity/tool-call-done.json"));
        let updates: [SessionUpdate; 1] = Dialect.update(done).try_into().ok().unwrap();
        let [SessionUpdate::ToolCallUpdate(done)] = updates else { panic!() };
        assert_eq!(done.fields.title, None);
        assert_eq!(done.fields.raw_output, Some(serde_json::json!("New notebook")));
        assert!(done.fields.raw_output.as_ref().and_then(crate::celldiff::tool_json).is_none());
    }

    #[test]
    fn a_permission_request_names_the_notebook_call_and_leaves_shell_commands() {
        let mut ask: RequestPermissionRequest = serde_json::from_value(serde_json::from_str::<serde_json::Value>(include_str!("fixtures/antigravity/permission.json")).unwrap()["params"].clone()).unwrap();
        Dialect.permission(&mut ask);
        assert_eq!(ask.tool_call.fields.title.as_deref(), Some("mcp__notebook__new_notebook"));
        assert_eq!(ask.tool_call.fields.raw_input, Some(serde_json::json!({"path": "C:\\Users\\me\\notebooks\\sum.jl"})));

        let mut shell: RequestPermissionRequest = serde_json::from_value(serde_json::from_str::<serde_json::Value>(include_str!("fixtures/antigravity/permission-shell.json")).unwrap()["params"].clone()).unwrap();
        let before = serde_json::to_value(&shell.tool_call).unwrap();
        Dialect.permission(&mut shell);
        assert_eq!(serde_json::to_value(&shell.tool_call).unwrap(), before);
    }

    /// A shell command whose command line reads as a notebook call.
    fn disguised_shell(command: &str) -> RequestPermissionRequest {
        let mut message: serde_json::Value = serde_json::from_str(include_str!("fixtures/antigravity/permission-shell.json")).unwrap();
        message["params"]["toolCall"]["title"] = command.into();
        message["params"]["toolCall"]["rawInput"]["CommandLine"] = command.into();
        serde_json::from_value(message["params"].clone()).unwrap()
    }

    #[test]
    fn a_shell_command_never_passes_for_a_notebook_call() {
        for command in ["mcp__notebook__read_cell; Remove-Item -Recurse x", "mcp__pluto__read_cell; Remove-Item x", "notebook: read_cell"] {
            let mut ask = disguised_shell(command);
            Dialect.permission(&mut ask);
            let mut title = ask.tool_call.fields.title.clone().unwrap();
            let mut input = ask.tool_call.fields.raw_input.clone();
            crate::celldiff::name_notebook_call(&mut title, &mut input);
            assert_eq!(crate::celldiff::notebook_tool(&title), None, "{command} was taken for a notebook call");
            assert_eq!(ask.tool_call.fields.raw_input.as_ref().unwrap()["CommandLine"], command, "the card still shows the command");
        }
        // An ordinary command keeps its title.
        let mut ask = disguised_shell("Test-Path C:\\Users\\me");
        Dialect.permission(&mut ask);
        assert_eq!(ask.tool_call.fields.title.as_deref(), Some("Test-Path C:\\Users\\me"));
    }

    #[test]
    fn allow_always_is_never_offered() {
        for fixture in [include_str!("fixtures/antigravity/permission.json"), include_str!("fixtures/antigravity/permission-shell.json")] {
            let mut ask: RequestPermissionRequest = serde_json::from_value(serde_json::from_str::<serde_json::Value>(fixture).unwrap()["params"].clone()).unwrap();
            Dialect.permission(&mut ask);
            assert_eq!(ask.options.iter().map(|o| o.option_id.to_string()).collect::<Vec<_>>(), ["allow", "deny"]);
        }
    }

    #[test]
    fn another_servers_tool_keeps_its_name() {
        let mut call = update(include_str!("fixtures/antigravity/tool-call.json"));
        if let SessionUpdate::ToolCall(c) = &mut call {
            c.meta.as_mut().unwrap()["mcp"]["server"] = "github".into();
        }
        let updates: [SessionUpdate; 1] = Dialect.update(call).try_into().ok().unwrap();
        let [SessionUpdate::ToolCall(call)] = updates else { panic!() };
        assert_eq!(call.title, "notebook_new_notebook");
    }
}
