use agent_client_protocol::schema::v1::{
    ConfigOptionUpdate, CurrentModeUpdate, Meta, PermissionOptionKind, RequestPermissionRequest, SessionConfigKind, SessionConfigOption, SessionConfigValueId, SessionMode,
    SessionModeId, SessionModeState, SessionUpdate,
};

use crate::agent::{ModeSwitch, Started};

pub const SIGN_IN_METHOD: &str = "oauth-personal";

const PRESET: &str = "default";
const MODE_OPTION: &str = "mode";

pub fn signed_in() -> bool {
    wire::files::home().join(".gemini").join("antigravity-acp").join("acp_token.json").is_file()
}

#[derive(Default)]
pub struct Dialect;

impl Dialect {
    pub fn started(&mut self, started: Started) -> (Started, Option<(String, SessionConfigValueId)>) {
        let preset = current(&started.config, MODE_OPTION).or_else(|| started.modes.as_ref().map(|m| m.current_mode_id.to_string()));
        let fix = preset.is_some_and(|p| p != PRESET).then(|| (MODE_OPTION.to_owned(), PRESET.into()));
        let modes = SessionModeState::new("default", vec![SessionMode::new("default", "Manual"), SessionMode::new("auto", "Ask to run")]);
        (Started { id: started.id, modes: Some(modes), config: app_options(started.config) }, fix)
    }

    pub fn update(&mut self, update: SessionUpdate) -> Vec<SessionUpdate> {
        match update {
            SessionUpdate::CurrentModeUpdate(_) => Vec::new(),
            SessionUpdate::ConfigOptionUpdate(update) => vec![SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(app_options(update.config_options)))],
            SessionUpdate::ToolCall(mut call) => {
                if let Some(tool) = notebook_tool(call.meta.as_ref()) {
                    call.title = format!("{}{tool}", crate::celldiff::TOOL_PREFIX);
                    call.raw_input = call.raw_input.map(arguments);
                } else {
                    shell_command(&mut call.raw_input);
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
                } else {
                    shell_command(&mut update.fields.raw_input);
                    if let Some(title) = &mut update.fields.title {
                        not_a_notebook_call(title, &update.fields.raw_input);
                    }
                }
                vec![SessionUpdate::ToolCallUpdate(update)]
            }
            update => vec![update],
        }
    }

    pub fn permission(&self, request: &mut RequestPermissionRequest) {
        let call = &mut request.tool_call;
        if let Some(tool) = notebook_tool(call.meta.as_ref()) {
            call.fields.title = Some(format!("{}{tool}", crate::celldiff::TOOL_PREFIX));
            call.fields.raw_input = call.fields.raw_input.take().map(arguments);
        } else {
            shell_command(&mut call.fields.raw_input);
            if let Some(title) = &mut call.fields.title {
                not_a_notebook_call(title, &call.fields.raw_input);
            }
        }
        request.options.retain(|o| o.kind != PermissionOptionKind::AllowAlways);
    }

    pub fn options(&self, options: Vec<SessionConfigOption>) -> Vec<SessionConfigOption> {
        app_options(options)
    }

    pub fn set_mode(&mut self, mode: &SessionModeId) -> ModeSwitch {
        let mode = if mode.to_string() == "auto" { "auto" } else { "default" };
        ModeSwitch { mode: None, set: None, confirm: Some(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(mode))) }
    }
}

fn notebook_tool(meta: Option<&Meta>) -> Option<String> {
    let mcp = meta?.get("mcp")?;
    let tool = mcp.get("tool")?.as_str()?;
    (mcp.get("server")?.as_str()? == crate::celldiff::MCP_SERVER && endeavor_mcp::is_tool(tool)).then(|| tool.to_owned())
}

fn not_a_notebook_call(title: &mut String, input: &Option<serde_json::Value>) {
    let (mut named, mut input) = (title.clone(), input.clone());
    crate::celldiff::name_notebook_call(&mut named, &mut input);
    if crate::celldiff::notebook_tool(&named).is_some() {
        *title = format!("Antigravity: {title}");
    }
}

fn shell_command(input: &mut Option<serde_json::Value>) {
    if let Some(serde_json::Value::Object(fields)) = input
        && !fields.contains_key("command")
        && let Some(command) = fields.get("CommandLine").filter(|c| c.is_string()).cloned()
    {
        fields.insert("command".into(), command);
    }
}

fn arguments(input: serde_json::Value) -> serde_json::Value {
    match input.get("arguments") {
        Some(args) if args.is_object() => args.clone(),
        _ => input,
    }
}

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
        let mut before = serde_json::to_value(&shell.tool_call).unwrap();
        Dialect.permission(&mut shell);
        before["rawInput"]["command"] = before["rawInput"]["CommandLine"].clone();
        assert_eq!(serde_json::to_value(&shell.tool_call).unwrap(), before);
    }

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
        let mut ask = disguised_shell("Test-Path C:\\Users\\me");
        Dialect.permission(&mut ask);
        assert_eq!(ask.tool_call.fields.title.as_deref(), Some("Test-Path C:\\Users\\me"));
    }

    #[test]
    fn a_shell_call_shows_its_command() {
        let mut ask: RequestPermissionRequest = serde_json::from_value(serde_json::from_str::<serde_json::Value>(include_str!("fixtures/antigravity/permission-shell.json")).unwrap()["params"].clone()).unwrap();
        Dialect.permission(&mut ask);
        let input = ask.tool_call.fields.raw_input.clone().unwrap();
        assert_eq!(input["command"], input["CommandLine"]);
        let call = SessionUpdate::ToolCall(serde_json::from_value(serde_json::json!({"toolCallId": "1", "title": "Get-Location", "kind": "execute", "rawInput": {"CommandLine": "Get-Location"}})).unwrap());
        let updates: [SessionUpdate; 1] = Dialect.update(call).try_into().ok().unwrap();
        let [SessionUpdate::ToolCall(call)] = updates else { panic!() };
        assert_eq!(call.raw_input.unwrap()["command"], "Get-Location");
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
