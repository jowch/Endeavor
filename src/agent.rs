use std::collections::HashSet;
use std::path::{Path, PathBuf};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AuthenticateRequest, AuthenticateResponse, ClientCapabilities, ClientSessionCapabilities, NoticeCapabilities,
    CancelNotification, CloseSessionRequest, ContentBlock, DeleteSessionRequest, ForkSessionRequest, HttpHeader, InitializeRequest, ListSessionsRequest, LoadSessionRequest, McpServer,
    McpServerHttp, NewSessionRequest, PromptRequest, PromptResponse, RequestPermissionRequest,
    RequestPermissionResponse, SessionConfigOption, SessionId, SessionInfo,
    SessionModeId, SessionModeState, SessionNotification, SessionUpdate, SetSessionModeRequest,
    SessionConfigValueId, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
    StopReason,
};
use agent_client_protocol::{AcpAgent, ConnectionTo, ErrorCode, Responder, UntypedMessage};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::{Either, LocalBoxFuture, select};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};

use crate::pluto::Bridge;
use crate::splash::{Progress, Step};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    #[default]
    Claude,
    Codex,
    Antigravity,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Antigravity];

    pub fn facts(self) -> &'static AgentFacts {
        match self {
            Agent::Claude => &CLAUDE_CODE,
            Agent::Codex => &CODEX,
            Agent::Antigravity => &ANTIGRAVITY,
        }
    }

    pub fn name(self) -> &'static str {
        self.facts().name
    }

    pub fn available(self) -> bool {
        match &self.facts().install {
            Install::Npm { .. } => true,
            Install::Program { zip, .. } => zip.is_some(),
        }
    }
}

#[macro_export]
macro_rules! agent_text {
    ($agent:expr, $before:literal, $after:literal) => {
        match $agent {
            $crate::agent::Agent::Claude => concat!($before, "Claude", $after),
            $crate::agent::Agent::Codex => concat!($before, "Codex", $after),
            $crate::agent::Agent::Antigravity => concat!($before, "Antigravity", $after),
        }
    };
}

pub struct AgentFacts {
    pub name: &'static str,
    pub maker: &'static str,
    pub install: Install,
    env: &'static [(&'static str, &'static str)],
    private_temp: bool,
    plugin: bool,
    pub folder_rules: Option<&'static str>,
    pub on_servers: bool,
    pub asks_every_write: bool,
    pub config: &'static [(&'static str, &'static str)],
    pub sign_in: SignIn,
    pub session_intro: Option<&'static str>,
    pub on_demand: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Account {
    Unknown,
    SignedIn,
    SignedOut,
    SigningIn,
    Failed,
}

impl Account {
    pub fn signed_out(self) -> bool {
        matches!(self, Account::SignedOut | Account::SigningIn | Account::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SignIn {
    ClaudeAuth,
    CodexLogin,
    Authenticate(&'static str),
}

pub enum Install {
    Npm { package: &'static str, pins: &'static str, installed: &'static str },
    Program { what: &'static str, zip: Option<(&'static str, &'static str, u64)>, folder: &'static str, program: &'static str },
}

pub const CLAUDE_CODE: AgentFacts = AgentFacts {
    name: "Claude",
    maker: "by Anthropic",
    install: Install::Npm { package: "@agentclientprotocol/claude-agent-acp", pins: "adapter", installed: "adapter" },
    env: &[("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT", "0")],
    private_temp: false,
    plugin: true,
    folder_rules: Some(".claude/settings.local.json"),
    on_servers: true,
    asks_every_write: false,
    config: &[("model", "Model"), ("effort", "Effort")],
    sign_in: SignIn::ClaudeAuth,
    session_intro: None,
    on_demand: false,
};

pub const CODEX: AgentFacts = AgentFacts {
    name: "Codex",
    maker: "by OpenAI",
    install: Install::Npm { package: "@agentclientprotocol/codex-acp", pins: "adapter-codex", installed: "codex-adapter" },
    env: &[("INITIAL_AGENT_MODE", "workspace-write"), ("CODEX_CONFIG", r#"{"plugins":{"endeavor@endeavor":{"enabled":false}}}"#)],
    private_temp: false,
    plugin: false,
    folder_rules: None,
    on_servers: false,
    asks_every_write: true,
    config: &[("model", "Model"), ("effort", "Effort"), ("fast-mode", "Speed")],
    sign_in: SignIn::CodexLogin,
    session_intro: Some(
        "[Endeavor] This session works in Endeavor, where the user's notebook is a Pluto.jl notebook (Julia), \
         shown beside this chat. Do all notebook work with the notebook MCP server's tools, and call its \
         notebook_guide tool once before the first notebook call. Don't create Jupyter or other notebooks, \
         and don't start Julia or Pluto yourself from the shell.",
    ),
    on_demand: true,
};

pub const ANTIGRAVITY: AgentFacts = AgentFacts {
    name: "Antigravity",
    maker: "by Google",
    install: Install::Program { what: "Antigravity's ACP server 1.3.0", zip: ANTIGRAVITY_ZIP, folder: "antigravity-acp-1.3.0", program: ANTIGRAVITY_PROGRAM },
    env: &[],
    private_temp: true,
    plugin: false,
    folder_rules: None,
    on_servers: false,
    asks_every_write: true,
    config: &[("model", "Model")],
    sign_in: SignIn::Authenticate(crate::antigravity::SIGN_IN_METHOD),
    session_intro: CODEX.session_intro,
    on_demand: true,
};

#[cfg(all(windows, target_arch = "x86_64"))]
const ANTIGRAVITY_ZIP: Option<(&str, &str, u64)> = Some((
    "https://dl.google.com/agy-extensions/releases/windows/agy-acp-server-1.3.0-windows-x86_64.zip",
    "65215e0688681fa3116e048a9eab27ef53af1bbd6f3da3f1c52bd4911d8b17f9",
    124_509_787,
));
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const ANTIGRAVITY_ZIP: Option<(&str, &str, u64)> = Some((
    "https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-1.3.0-darwin-arm64.zip",
    "7cd97045f7b4fe81175a107cdf16f9c51484e3c78a5162cae415338bb6aa5b88",
    111_456_962,
));
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const ANTIGRAVITY_ZIP: Option<(&str, &str, u64)> = Some((
    "https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-1.3.0-darwin-x86_64.zip",
    "bb23956b89984bf5d354af2c3725e6c57f0cc1b7228e77a0e91c9c2bc1d47646",
    117_245_544,
));
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const ANTIGRAVITY_ZIP: Option<(&str, &str, u64)> = Some((
    "https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-1.3.0-linux-arm64.zip",
    "500b0bc0fb858e88f4df404d4cedf80bf9298c178291e39e383d6c50b111cbdf",
    321_690_363,
));
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const ANTIGRAVITY_ZIP: Option<(&str, &str, u64)> = Some((
    "https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-1.3.0-linux-x86_64.zip",
    "9fb60956af0a9d76220a4db91ca9ac88e2a2372ad68f985ab5fceace6b825b96",
    333_727_150,
));
#[cfg(not(any(
    all(windows, target_arch = "x86_64"),
    all(any(target_os = "macos", target_os = "linux"), any(target_arch = "aarch64", target_arch = "x86_64"))
)))]
const ANTIGRAVITY_ZIP: Option<(&str, &str, u64)> = None;
const ANTIGRAVITY_PROGRAM: &str = if cfg!(windows) { "agy_acp_server.exe" } else { "agy_acp_server.par" };

const NODE_VERSION: &str = "24.21.0";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const NODE_TARBALL: (&str, &str, u64, &str) = (
    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-darwin-arm64.tar.gz",
    "bed7eea5325e1108f32ce5228ddd6a5f0f08a499ee42aa7442aea583702f6057",
    52_909_993,
    "node-v24.21.0-darwin-arm64",
);
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const NODE_TARBALL: (&str, &str, u64, &str) = (
    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-darwin-x64.tar.gz",
    "1462cb3b3046b815cf8ea436d3da450ec1a9f11dac7e5a46b0ada5305d7e8097",
    54_203_979,
    "node-v24.21.0-darwin-x64",
);
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const NODE_TARBALL: (&str, &str, u64, &str) = (
    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-linux-arm64.tar.gz",
    "724282c3b43aec998aa9527380465b45d229e021b58035f5f4f63095eabfe5d5",
    57_824_078,
    "node-v24.21.0-linux-arm64",
);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const NODE_TARBALL: (&str, &str, u64, &str) = (
    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-linux-x64.tar.gz",
    "6e1db87ef58b8819e5d5402eff1536491b18edd8eb7bee5ef7897876e88dc5ff",
    58_088_022,
    "node-v24.21.0-linux-x64",
);
#[cfg(all(windows, target_arch = "x86_64"))]
const NODE_TARBALL: (&str, &str, u64, &str) = (
    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-win-x64.zip",
    "158f7685b44de51f6c0df1d153526cbcd3e1bc739a8dfc607721cef75de9e541",
    37_618_919,
    "node-v24.21.0-win-x64",
);
#[cfg(all(windows, target_arch = "aarch64"))]
const NODE_TARBALL: (&str, &str, u64, &str) = (
    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-win-arm64.zip",
    "8779b1bde1d39f8d420e3b57aa657b39891af434d3de44a919044cec06785921",
    33_679_608,
    "node-v24.21.0-win-arm64",
);

fn pinned_adapter_version(agent: Agent) -> Result<String, String> {
    let Install::Npm { package, pins, .. } = agent.facts().install else { return Err(format!("{} has no npm adapter", agent.name())) };
    let manifest = std::fs::read_to_string(crate::install::resources().join(pins).join("package.json")).map_err(|e| e.to_string())?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).map_err(|e| e.to_string())?;
    manifest["dependencies"][package].as_str().map(str::to_owned).ok_or_else(|| format!("{pins}/package.json has no adapter version"))
}

pub fn adapter_status(agent: Agent) -> Result<(String, bool), String> {
    let (_, entry) = adapter_paths(agent)?;
    Ok((pinned_adapter_version(agent)?, entry.exists()))
}

fn adapter_paths(agent: Agent) -> Result<(PathBuf, PathBuf), String> {
    let app = crate::install::app_dir()?;
    let Install::Npm { package, installed, .. } = agent.facts().install else { return Err(format!("{} has no npm adapter", agent.name())) };
    let version = pinned_adapter_version(agent)?;
    let node = if cfg!(windows) { app.join(format!("node-v{NODE_VERSION}")).join("node.exe") } else { app.join(format!("node-v{NODE_VERSION}/bin/node")) };
    let entry = app.join(format!("{installed}-{version}/node_modules/{package}/dist/index.js"));
    Ok((node, entry))
}

pub(crate) fn claude_cli(args: &[&str]) -> Result<std::process::Command, String> {
    #[cfg(debug_assertions)]
    if let Some(fake) = std::env::var_os("ENDEAVOR_CLAUDE_CLI") {
        let mut command = std::process::Command::new(fake);
        command.args(args).stdin(std::process::Stdio::null());
        endeavor_mcp::client::no_window(&mut command);
        return Ok(command);
    }
    let (node, entry) = adapter_paths(Agent::Claude)?;
    let mut command = std::process::Command::new(node);
    command.arg(entry).arg("--cli").args(args).stdin(std::process::Stdio::null());
    endeavor_mcp::client::no_window(&mut command);
    Ok(command)
}

pub(crate) fn codex_adapter(args: &[&str]) -> Result<std::process::Command, String> {
    #[cfg(debug_assertions)]
    if let Some(fake) = std::env::var_os("ENDEAVOR_CODEX_CLI") {
        let mut command = std::process::Command::new(fake);
        command.args(args).stdin(std::process::Stdio::null());
        endeavor_mcp::client::no_window(&mut command);
        return Ok(command);
    }
    let (node, entry) = adapter_paths(Agent::Codex)?;
    let mut command = std::process::Command::new(node);
    command.arg(entry).args(args).stdin(std::process::Stdio::null());
    endeavor_mcp::client::no_window(&mut command);
    Ok(command)
}

pub(crate) fn bundled_codex() -> Option<PathBuf> {
    let (_, entry) = adapter_paths(Agent::Codex).ok()?;
    let modules = entry.ancestors().nth(4)?;
    let (os, exe) = match std::env::consts::OS {
        "macos" => ("darwin", "codex"),
        "windows" => ("win32", "codex.exe"),
        _ => ("linux", "codex"),
    };
    let mut arches = ["x64", "arm64"];
    if std::env::consts::ARCH == "aarch64" { arches.reverse(); }
    arches.into_iter().find_map(|arch| {
        let vendor = modules.join(format!("@openai/codex-{os}-{arch}/vendor"));
        std::fs::read_dir(&vendor).ok()?.flatten().map(|triple| triple.path().join("bin").join(exe)).find(|path| path.is_file())
    })
}

fn test_no_steering() -> bool {
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("ENDEAVOR_TEST_NO_STEERING") {
        return Path::new(&file).exists();
    }
    false
}

fn fake_auth_error() -> bool {
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("ENDEAVOR_FAKE_AUTH_ERROR") {
        return Path::new(&file).exists();
    }
    false
}

fn fake_turn_error() -> Option<Vec<SessionEvent>> {
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("ENDEAVOR_TEST_TURN_ERROR")
        && let Ok(text) = std::fs::read_to_string(file)
    {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, SessionUpdate, TextContent};
        let (kind, message) = text.split_once('\n').unwrap_or((&text, ""));
        let kind = kind.trim();
        let partial = SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            "(test) I'll resample the rows 1,000 times and refit the model on each copy. First the resampling cell:",
        )))));
        let end = match kind {
            "max_tokens" => SessionEvent::TurnEnded(StopReason::MaxTokens),
            "max_turn_requests" => SessionEvent::TurnEnded(StopReason::MaxTurnRequests),
            _ => SessionEvent::TurnFailed { kind: (kind != "-").then(|| kind.to_owned()), error: message.trim().to_owned() },
        };
        let midway = matches!(kind, "max_tokens" | "max_turn_requests" | "transport_lost");
        return Some(if midway { vec![partial, end] } else { vec![end] });
    }
    None
}

fn adapter_command(agent: Agent, progress: &dyn Fn(Progress)) -> Result<Vec<String>, String> {
    #[cfg(debug_assertions)]
    if let Some(stand_in) = std::env::var_os("ENDEAVOR_TEST_AGENT").filter(|v| !v.is_empty()) {
        let _ = progress;
        return Ok(vec![stand_in.to_string_lossy().into_owned()]);
    }
    let facts = agent.facts();
    let mut env: Vec<String> = facts.env.iter().map(|(name, value)| format!("{name}={}", with_inherited(name, value, std::env::var(name).ok()))).collect();
    let program = match facts.install {
        Install::Npm { pins, installed, .. } => npm_adapter(agent, pins, installed, progress)?,
        Install::Program { what, zip, folder, program } => {
            let app = crate::install::app_dir()?;
            let zip = zip.ok_or_else(|| format!("{} isn't available on this computer yet.", facts.name))?;
            let dir = app.join(folder);
            let exe = dir.join(program);
            if !exe.exists() {
                progress(Progress::new(Step::Agent, format!("Installing the {} agent…", facts.name)));
                crate::install::tarball(&dir, what, "", zip, &|detail, fraction| progress(Progress { fraction, ..Progress::new(Step::Agent, detail) }))?;
                if let Some((name, version)) = folder.rsplit_once('-') {
                    crate::install::remove_other_versions(&format!("{name}-"), version, &[]);
                }
            }
            vec![exe.display().to_string()]
        }
    };
    if facts.private_temp {
        let temp = crate::install::app_dir()?.join(format!("{}-temp", agent_slug(agent)));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).map_err(|e| format!("Couldn't make {}'s temp folder: {e}", facts.name))?;
        env.extend(["TEMP", "TMP", "TMPDIR"].map(|name| format!("{name}={}", temp.display())));
    }
    let [program, args @ ..] = program.as_slice() else { return Err("no adapter program".into()) };
    let program = crate::agent_job::command(program.clone(), args.to_vec())?;
    Ok(env.into_iter().chain(program).collect())
}

fn with_inherited(name: &str, ours: &str, theirs: Option<String>) -> String {
    fn merge(base: &mut serde_json::Value, over: serde_json::Value) {
        match (base, over) {
            (serde_json::Value::Object(base), serde_json::Value::Object(over)) => {
                for (key, value) in over {
                    merge(base.entry(key).or_insert(serde_json::Value::Null), value);
                }
            }
            (base, over) => *base = over,
        }
    }
    let theirs = theirs.filter(|_| name == "CODEX_CONFIG").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).filter(serde_json::Value::is_object);
    match (theirs, serde_json::from_str(ours)) {
        (Some(mut theirs), Ok(ours)) => {
            merge(&mut theirs, ours);
            theirs.to_string()
        }
        _ => ours.to_owned(),
    }
}

fn agent_slug(agent: Agent) -> String {
    agent.name().to_lowercase()
}

fn npm_adapter(agent: Agent, pins: &str, installed: &str, progress: &dyn Fn(Progress)) -> Result<Vec<String>, String> {
    let app = crate::install::app_dir()?;
    let facts = agent.facts();
    let (node, entry) = adapter_paths(agent)?;
    let bin = node.parent().ok_or("bad Node path")?.to_path_buf();
    let node_dir = if cfg!(windows) { bin.clone() } else { bin.parent().ok_or("bad Node path")?.to_path_buf() };
    if !node.exists() {
        let (url, sha, size, top) = NODE_TARBALL;
        crate::install::tarball(&node_dir, &format!("Node.js {NODE_VERSION}"), top, (url, sha, size), &|detail, fraction| {
            progress(Progress { fraction, ..Progress::new(Step::Agent, detail) })
        })?;
        crate::install::remove_other_versions("node-v", NODE_VERSION, &[]);
    }

    let pinned = crate::install::resources().join(pins);
    let adapter = entry.ancestors().nth(5).ok_or("bad adapter path")?.to_path_buf();
    if !entry.exists() {
        progress(Progress::new(Step::Agent, format!("Installing the {} agent…", facts.name)));
        let staging = app.join(format!("{installed}.installing"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        for file in ["package.json", "package-lock.json"] {
            std::fs::copy(pinned.join(file), staging.join(file)).map_err(|e| e.to_string())?;
        }
        let npm = if cfg!(windows) { node_dir.join("node_modules/npm/bin/npm-cli.js") } else { node_dir.join("lib/node_modules/npm/bin/npm-cli.js") };
        let path = format!("{}{}{}", bin.display(), if cfg!(windows) { ';' } else { ':' }, std::env::var("PATH").unwrap_or_default());
        let mut npm_ci = std::process::Command::new(&node);
        endeavor_mcp::client::no_window(&mut npm_ci);
        let out = npm_ci
            .arg(npm)
            .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
            .current_dir(&staging)
            .env("PATH", path)
            .output()
            .map_err(|e| format!("Couldn't run npm: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<_> = err.lines().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect();
            return Err(format!("Couldn't install the {} agent. Check the internet connection and restart.\n{}", facts.name, tail.join("\n")));
        }
        let _ = std::fs::remove_dir_all(&adapter);
        crate::rename_folder(&staging, &adapter).map_err(|e| e.to_string())?;
        crate::install::remove_other_versions(&format!("{installed}-"), &pinned_adapter_version(agent)?, &[]);
    }

    Ok(vec![node.display().to_string(), entry.display().to_string()])
}

const LOCAL_TOOLS: [&str; 8] = ["Bash", "Read", "Write", "Edit", "MultiEdit", "Glob", "Grep", "NotebookEdit"];

const READ_ONLY_TOOLS: [&str; 16] = [
    "mcp__notebook__list_notebooks",
    "mcp__notebook__session_status",
    "mcp__notebook__read_cell",
    "mcp__notebook__read_notebook_code",
    "mcp__notebook__view_cell_output",
    "mcp__notebook__get_cell_order",
    "mcp__notebook__get_execution_order",
    "mcp__notebook__get_cell_dependencies",
    "mcp__notebook__get_cell_dependents",
    "mcp__notebook__find_symbol_definitions",
    "mcp__notebook__find_symbol_references",
    "mcp__notebook__search_code",
    "mcp__notebook__validate_cell",
    "mcp__notebook__preview_package_update",
    "mcp__notebook__list_folder",
    "mcp__notebook__read_file",
];

const ASKS_ITSELF: [&str; 1] = ["mcp__notebook__request_restart"];

fn session_options(personal: bool, plugin_dir: &str, on_server: bool) -> serde_json::Value {
    let sources: &[&str] = if personal { &["user", "project", "local"] } else { &["project", "local"] };
    let disallowed: Vec<&str> = std::iter::once(USERS_ENDEAVOR_SERVER).chain(if on_server { LOCAL_TOOLS.as_slice() } else { &[] }.iter().copied()).collect();
    let allowed: Vec<String> = READ_ONLY_TOOLS.iter().chain(&ASKS_ITSELF).map(|t| t.to_string()).chain(plugin_rules(plugin_dir)).collect();
    serde_json::json!({
        "claudeCode": { "options": {
            "settingSources": sources,
            "strictMcpConfig": !personal,
            "plugins": [{ "type": "local", "path": plugin_dir }],
            "settings": { "enabledPlugins": { USERS_ENDEAVOR_PLUGIN: false } },
            "disallowedTools": disallowed,
            "allowedTools": allowed,
        } }
    })
}

const USERS_ENDEAVOR_PLUGIN: &str = "endeavor@endeavor";

const USERS_ENDEAVOR_SERVER: &str = "mcp__endeavor";

fn plugin_rules(plugin_dir: &str) -> Vec<String> {
    let mut rules = vec![format!("Read({}/**)", rule_path(plugin_dir))];
    if let Some(real) = std::fs::canonicalize(plugin_dir).ok().and_then(|p| p.to_str().map(rule_path))
        && real != rule_path(plugin_dir)
    {
        rules.push(format!("Read({real}/**)"));
    }
    rules
}

fn rule_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path).replace('\\', "/");
    let path = path.trim_end_matches('/');
    match path.as_bytes() {
        [drive, b':', b'/', ..] if drive.is_ascii_alphabetic() => format!("//{}{}", drive.to_ascii_lowercase() as char, &path[2..]),
        _ => format!("/{path}"),
    }
}

#[derive(Clone, Debug)]
pub struct Tools {
    pub bridge: Bridge,
    pub server: Option<String>,
}

impl Tools {
    fn mcp_server(&self, key: u64, agent: Agent) -> McpServer {
        let mut headers = vec![
            HttpHeader::new("Authorization", format!("Bearer {}", self.bridge.token)),
            HttpHeader::new("X-Endeavor-Session", key.to_string()),
            HttpHeader::new("X-Endeavor-Client", "app"),
        ];
        if agent.facts().plugin {
            headers.push(HttpHeader::new("X-Endeavor-Skills", "plugin"));
        }
        if let Some(server) = &self.server {
            headers.push(HttpHeader::new("X-Endeavor-Host", server.clone()));
        }
        McpServer::Http(McpServerHttp::new(crate::celldiff::MCP_SERVER, self.bridge.url.clone()).headers(headers))
    }
}

pub enum Turn {
    Prompt(Vec<ContentBlock>),
    SendNow(Vec<ContentBlock>),
    Cancel,
}

pub enum Command {
    NewSession { key: u64, cwd: PathBuf, tools: Tools },
    LoadSession { key: u64, id: SessionId, cwd: PathBuf, tools: Tools },
    ForkSession { key: u64, source: SessionId, cwd: PathBuf, tools: Tools },
    ListSessions { cwd: PathBuf },
    SetMode(SessionId, SessionModeId),
    SetConfig(SessionId, String, SessionConfigValueId),
    CloseSession(SessionId),
    DeleteSession(SessionId),
    Turn(SessionId, Turn),
    Authenticate(&'static str),
}

pub enum SessionEvent {
    Update(SessionUpdate),
    Config(Vec<SessionConfigOption>),
    Permission(RequestPermissionRequest, Responder<RequestPermissionResponse>),
    TurnEnded(StopReason),
    TurnFailed { kind: Option<String>, error: String },
    ConfigFailed(String),
    AuthRequired,
    Steered,
    Unsent,
    Stopping,
}

#[derive(Debug)]
pub struct Started {
    pub id: SessionId,
    pub modes: Option<SessionModeState>,
    pub config: Vec<SessionConfigOption>,
}

impl Started {
    pub fn new(id: SessionId, modes: Option<SessionModeState>, config: Option<Vec<SessionConfigOption>>) -> Self {
        Self { id, modes, config: config.unwrap_or_default() }
    }
}

pub enum AgentEvent {
    Ready,
    Started { key: u64, result: Result<Started, String> },
    Listed { cwd: PathBuf, sessions: Result<Vec<SessionInfo>, String>, complete: bool },
    Forked { key: u64, id: SessionId },
    Session(SessionId, SessionEvent),
    Setup(Progress),
    SignedIn(Option<crate::signin::Method>),
    CodexSignedIn(bool),
    AntigravitySignedIn(bool),
    SignInEnded(Result<(), String>),
    Failed(String),
}

#[derive(Debug, PartialEq)]
pub struct ModeSwitch {
    pub mode: Option<SessionModeId>,
    pub set: Option<(String, SessionConfigValueId)>,
    pub confirm: Option<SessionUpdate>,
}

enum Dialect {
    Claude,
    Codex(crate::codex::Dialect),
    Antigravity(crate::antigravity::Dialect),
}

impl Dialect {
    fn of(agent: Agent) -> Dialect {
        match agent {
            Agent::Claude => Dialect::Claude,
            Agent::Codex => Dialect::Codex(crate::codex::Dialect::default()),
            Agent::Antigravity => Dialect::Antigravity(crate::antigravity::Dialect),
        }
    }

    fn started(&mut self, started: Started) -> (Started, Option<(String, SessionConfigValueId)>) {
        match self {
            Dialect::Claude => (started, None),
            Dialect::Codex(codex) => codex.started(started),
            Dialect::Antigravity(antigravity) => antigravity.started(started),
        }
    }

    fn update(&mut self, session: &SessionId, update: SessionUpdate) -> Vec<SessionUpdate> {
        match self {
            Dialect::Claude => vec![update],
            Dialect::Codex(codex) => codex.update(session, update),
            Dialect::Antigravity(antigravity) => antigravity.update(update),
        }
    }

    fn permission(&self, request: &mut RequestPermissionRequest) {
        if let Dialect::Antigravity(antigravity) = self {
            antigravity.permission(request);
        }
    }

    fn options(&self, options: Vec<SessionConfigOption>) -> Vec<SessionConfigOption> {
        match self {
            Dialect::Claude => options,
            Dialect::Codex(codex) => codex.options(options),
            Dialect::Antigravity(antigravity) => antigravity.options(options),
        }
    }

    fn set_mode(&mut self, session: &SessionId, mode: SessionModeId) -> ModeSwitch {
        match self {
            Dialect::Claude => ModeSwitch { mode: Some(mode), set: None, confirm: None },
            Dialect::Codex(codex) => codex.set_mode(session, &mode),
            Dialect::Antigravity(antigravity) => antigravity.set_mode(&mode),
        }
    }

    fn option_id(&self, id: String) -> String {
        match self {
            Dialect::Claude => id,
            Dialect::Codex(codex) => codex.option_id(&id),
            Dialect::Antigravity(_) => id,
        }
    }

    fn closed(&mut self, session: &SessionId) {
        if let Dialect::Codex(codex) = self {
            codex.closed(session);
        }
    }
}

pub fn start(agent: Agent, commands: UnboundedReceiver<Command>) -> UnboundedReceiver<AgentEvent> {
    let (event_tx, event_rx) = unbounded();
    std::thread::spawn(move || {
        let events = event_tx.clone();
        let command = adapter_command(agent, &|p| {
            let _ = events.unbounded_send(AgentEvent::Setup(p));
        });
        if command.is_ok() {
            let _ = events.unbounded_send(AgentEvent::Setup(Progress::new(Step::Claude, "Connecting…")));
        }
        if command.is_ok() {
            let signed_in = match agent.facts().sign_in {
                SignIn::ClaudeAuth => crate::signin::status().map(AgentEvent::SignedIn),
                SignIn::CodexLogin => crate::codex::signed_in().map(AgentEvent::CodexSignedIn),
                SignIn::Authenticate(_) => Ok(AgentEvent::AntigravitySignedIn(crate::antigravity::signed_in())),
            };
            if let Ok(event) = signed_in {
                let _ = events.unbounded_send(event);
            }
        }
        let reason = match command {
            Err(e) => e,
            Ok(command) => match futures::executor::block_on(run(agent, command, commands, event_tx)) {
                Ok(()) => "agent connection closed".to_string(),
                Err(e) => e.to_string(),
            },
        };
        let _ = events.unbounded_send(AgentEvent::Failed(reason));
    });
    event_rx
}

enum Done {
    Turn(SessionId, Result<PromptResponse, agent_client_protocol::Error>),
    Started(u64, Result<Started, agent_client_protocol::Error>),
    Listed(PathBuf, Result<(Vec<SessionInfo>, bool), agent_client_protocol::Error>),
    Forked(u64, PathBuf, Tools, Result<SessionId, agent_client_protocol::Error>),
    Config(SessionId, Result<SetSessionConfigOptionResponse, agent_client_protocol::Error>),
    SignedIn(Result<AuthenticateResponse, agent_client_protocol::Error>),
}

async fn run(
    agent: Agent,
    command: Vec<String>,
    mut commands: UnboundedReceiver<Command>,
    events: UnboundedSender<AgentEvent>,
) -> Result<(), agent_client_protocol::Error> {
    let adapter = AcpAgent::from_args(command)?;
    let (notify, permit) = (events.clone(), events.clone());
    let dialect = std::sync::Arc::new(std::sync::Mutex::new(Dialect::of(agent)));
    let (translate, name_asks) = (dialect.clone(), dialect.clone());

    agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                let updates = translate.lock().map(|mut d| d.update(&n.session_id, n.update)).unwrap_or_default();
                for update in updates {
                    let _ = notify.unbounded_send(AgentEvent::Session(n.session_id.clone(), SessionEvent::Update(update)));
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |mut request: RequestPermissionRequest, responder, _cx| {
                if let Ok(dialect) = name_asks.lock() {
                    dialect.permission(&mut request);
                }
                let session = request.session_id.clone();
                let _ = permit.unbounded_send(AgentEvent::Session(session, SessionEvent::Permission(request, responder)));
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_close(async move |_| Err(agent_client_protocol::Error::internal_error().data(format!("{}'s process exited", agent.name()))))
        .connect_with(adapter, async move |connection: ConnectionTo<agent_client_protocol::Agent>| {
            let init = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(ClientCapabilities::new().session(
                    ClientSessionCapabilities::new().notices(NoticeCapabilities::new()),
                )))
                .block_task()
                .await?;
            let steering = init
                .meta
                .as_ref()
                .and_then(|m| m.get("steering")?.get("supported")?.as_bool())
                .unwrap_or(false);
            let plugin = match agent.facts().plugin {
                true => Some(crate::install::plugin().map_err(|e| agent_client_protocol::Error::internal_error().data(e))?.display().to_string()),
                false => None,
            };
            let options = |tools: &Tools| {
                let plugin = plugin.as_deref()?;
                session_options(crate::settings::Settings::load().own_claude_setup(), plugin, tools.server.is_some()).as_object().cloned()
            };
            let _ = events.unbounded_send(AgentEvent::Ready);

            let mut pending: FuturesUnordered<LocalBoxFuture<'_, Done>> = FuturesUnordered::new();
            let mut running: HashSet<SessionId> = HashSet::new();
            let emit = |session: &SessionId, event: SessionEvent| {
                let _ = events.unbounded_send(AgentEvent::Session(session.clone(), event));
            };
            loop {
                let next = if pending.is_empty() {
                    Either::Right(commands.next().await)
                } else {
                    match select(pending.next(), commands.next()).await {
                        Either::Left((done, _)) => Either::Left(done),
                        Either::Right((command, _)) => Either::Right(command),
                    }
                };
                let command = match next {
                    Either::Left(Some(Done::Turn(session, result))) => {
                        running.remove(&session);
                        match result {
                            Ok(response) => emit(&session, SessionEvent::TurnEnded(response.stop_reason)),
                            Err(e) if e.code == ErrorCode::AuthRequired => emit(&session, SessionEvent::AuthRequired),
                            Err(e) if agent_client_protocol::is_incoming_transport_closed(&e) => {}
                            Err(e) => {
                                let kind = e.data.as_ref().and_then(|d| d.get("errorKind")?.as_str()).map(str::to_owned);
                                let error = if kind.is_some() { e.message.clone() } else { e.to_string() };
                                emit(&session, SessionEvent::TurnFailed { kind, error });
                            }
                        }
                        continue;
                    }
                    Either::Left(Some(Done::Started(key, result))) => {
                        if let Err(e) = &result
                            && e.code == ErrorCode::AuthRequired
                            && matches!(agent.facts().sign_in, SignIn::Authenticate(_))
                        {
                            let _ = events.unbounded_send(AgentEvent::AntigravitySignedIn(false));
                        }
                        let result = result.map_err(|e| e.to_string()).map(|started| {
                            let (started, fix) = dialect.lock().expect("dialect").started(started);
                            if let Some((id, value)) = fix {
                                let request = SetSessionConfigOptionRequest::new(started.id.clone(), id, value);
                                let _ = connection.send_request(request).on_receiving_result(async |_| Ok(()));
                            }
                            started
                        });
                        let _ = events.unbounded_send(AgentEvent::Started { key, result });
                        continue;
                    }
                    Either::Left(Some(Done::Listed(cwd, result))) => {
                        let complete = result.as_ref().is_ok_and(|(_, complete)| *complete);
                        let sessions = result.map(|(sessions, _)| sessions).map_err(|e| e.to_string());
                        let _ = events.unbounded_send(AgentEvent::Listed { cwd, sessions, complete });
                        continue;
                    }
                    Either::Left(Some(Done::Config(session, result))) => {
                        match result {
                            Ok(reply) => emit(&session, SessionEvent::Config(dialect.lock().expect("dialect").options(reply.config_options))),
                            Err(e) => emit(&session, SessionEvent::ConfigFailed(format!("Couldn't change the setting: {e}"))),
                        }
                        continue;
                    }
                    Either::Left(Some(Done::Forked(key, cwd, tools, result))) => {
                        match result {
                            Ok(id) => {
                                let _ = events.unbounded_send(AgentEvent::Forked { key, id: id.clone() });
                                let request = LoadSessionRequest::new(id.clone(), cwd).mcp_servers(vec![tools.mcp_server(key, agent)]).meta(options(&tools));
                                let loaded = connection.send_request(request).block_task();
                                pending.push(async move { Done::Started(key, loaded.await.map(|r| Started::new(id, r.modes, r.config_options))) }.boxed_local());
                            }
                            Err(e) => {
                                let _ = events.unbounded_send(AgentEvent::Started { key, result: Err(e.to_string()) });
                            }
                        }
                        continue;
                    }
                    Either::Left(Some(Done::SignedIn(result))) => {
                        let _ = events.unbounded_send(AgentEvent::SignInEnded(result.map(|_| ()).map_err(|e| e.to_string())));
                        continue;
                    }
                    Either::Left(None) => continue,
                    Either::Right(None) => break,
                    Either::Right(Some(command)) => command,
                };
                match command {
                    Command::NewSession { key, cwd, tools } => {
                        let request = NewSessionRequest::new(cwd).mcp_servers(vec![tools.mcp_server(key, agent)]).meta(options(&tools));
                        let started = connection.send_request(request).block_task();
                        pending.push(async move { Done::Started(key, started.await.map(|r| Started::new(r.session_id, r.modes, r.config_options))) }.boxed_local());
                    }
                    Command::LoadSession { key, id, cwd, tools } => {
                        let request = LoadSessionRequest::new(id.clone(), cwd).mcp_servers(vec![tools.mcp_server(key, agent)]).meta(options(&tools));
                        let loaded = connection.send_request(request).block_task();
                        pending.push(async move { Done::Started(key, loaded.await.map(|r| Started::new(id, r.modes, r.config_options))) }.boxed_local());
                    }
                    Command::ForkSession { key, source, cwd, tools } => {
                        let request = ForkSessionRequest::new(source, cwd.clone()).mcp_servers(vec![tools.mcp_server(key, agent)]).meta(options(&tools));
                        let forked = connection.send_request(request).block_task();
                        pending.push(async move { Done::Forked(key, cwd, tools, forked.await.map(|r| r.session_id)) }.boxed_local());
                    }
                    Command::ListSessions { cwd } => {
                        let listed = connection.send_request(ListSessionsRequest::new().cwd(cwd.clone())).block_task();
                        pending.push(async move { Done::Listed(cwd, listed.await.map(|r| (r.sessions, r.next_cursor.is_none()))) }.boxed_local());
                    }
                    Command::SetMode(session, mode) => {
                        let switch = dialect.lock().expect("dialect").set_mode(&session, mode);
                        if let Some(mode) = switch.mode {
                            connection.send_request(SetSessionModeRequest::new(session.clone(), mode)).on_receiving_result(async |_| Ok(()))?;
                        }
                        if let Some((id, value)) = switch.set {
                            let reply = connection.send_request(SetSessionConfigOptionRequest::new(session.clone(), id, value)).block_task();
                            let session = session.clone();
                            pending.push(async move { Done::Config(session, reply.await) }.boxed_local());
                        }
                        if let Some(update) = switch.confirm {
                            emit(&session, SessionEvent::Update(update));
                        }
                    }
                    Command::SetConfig(session, id, value) => {
                        let id = dialect.lock().expect("dialect").option_id(id);
                        let reply = connection.send_request(SetSessionConfigOptionRequest::new(session.clone(), id, value)).block_task();
                        pending.push(async move { Done::Config(session, reply.await) }.boxed_local());
                    }
                    Command::CloseSession(session) => {
                        running.remove(&session);
                        dialect.lock().expect("dialect").closed(&session);
                        connection.send_request(CloseSessionRequest::new(session)).on_receiving_result(async |_| Ok(()))?;
                    }
                    Command::DeleteSession(session) => {
                        running.remove(&session);
                        dialect.lock().expect("dialect").closed(&session);
                        connection.send_request(DeleteSessionRequest::new(session)).on_receiving_result(async |_| Ok(()))?;
                    }
                    Command::Turn(session, Turn::Prompt(_) | Turn::SendNow(_)) if !running.contains(&session) && fake_auth_error() => {
                        emit(&session, SessionEvent::AuthRequired);
                    }
                    Command::Turn(session, Turn::Prompt(_) | Turn::SendNow(_)) if !running.contains(&session) && let Some(fake) = fake_turn_error() => {
                        for event in fake {
                            emit(&session, event);
                        }
                    }
                    Command::Turn(session, Turn::Prompt(prompt) | Turn::SendNow(prompt)) if !running.contains(&session) => {
                        running.insert(session.clone());
                        let turn = connection.send_request(PromptRequest::new(session.clone(), prompt)).block_task();
                        pending.push(async move { Done::Turn(session, turn.await) }.boxed_local());
                    }
                    Command::Turn(session, Turn::Prompt(_)) => emit(&session, SessionEvent::Unsent),
                    Command::Turn(session, Turn::SendNow(prompt)) if steering && !test_no_steering() => {
                        let params = serde_json::json!({
                            "sessionId": session,
                            "prompt": prompt,
                            "_meta": { "steering": { "idleBehavior": "promptRequired" } },
                        });
                        let events = events.clone();
                        connection
                            .send_request(UntypedMessage::new("_session/steering", params)?)
                            .on_receiving_result(move |result| async move {
                                let injected = matches!(&result, Ok(v) if v["outcome"] == "injected");
                                let event = if injected { SessionEvent::Steered } else { SessionEvent::Unsent };
                                let _ = events.unbounded_send(AgentEvent::Session(session, event));
                                Ok(())
                            })?;
                    }
                    Command::Turn(session, Turn::SendNow(_)) => {
                        connection.send_notification(CancelNotification::new(session.clone()))?;
                        emit(&session, SessionEvent::Stopping);
                    }
                    Command::Turn(session, Turn::Cancel) => {
                        if running.contains(&session) {
                            connection.send_notification(CancelNotification::new(session))?;
                        }
                    }
                    Command::Authenticate(method) => {
                        let signed_in = connection.send_request(AuthenticateRequest::new(method)).block_task();
                        pending.push(async move { Done::SignedIn(signed_in.await) }.boxed_local());
                    }
                }
            }
            Ok(())
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::session_options;

    fn test_tools() -> super::Tools {
        let url = std::env::var("ENDEAVOR_TEST_MCP_URL").expect("ENDEAVOR_TEST_MCP_URL");
        let token = std::env::var("ENDEAVOR_TEST_TOKEN").unwrap_or_default();
        super::Tools { bridge: crate::pluto::Bridge { url, page_token: token.clone(), token }, server: None }
    }

    fn local_tools() -> (crate::runtime::Channel, super::Tools) {
        let listener = crate::runtime::Listener::start("This Mac").unwrap();
        let (channel, _) = crate::runtime::connect().expect("helper");
        let runtime = crate::runtime::start_local(&channel, &listener, &|_| {}, |_| {}).expect("runtime");
        (channel, super::Tools { bridge: runtime.bridge, server: None })
    }

    #[test]
    #[ignore]
    fn live_list_and_load() {
        use super::*;
        use agent_client_protocol::schema::v1::TextContent;

        let tools = test_tools();
        let cwd = std::env::temp_dir().join(format!("endeavor-history-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();
        let (tx, rx) = unbounded();
        let mut events = start(Agent::Claude, rx);
        tx.unbounded_send(Command::NewSession { key: 1, cwd: cwd.clone(), tools: tools.clone() }).unwrap();
        futures::executor::block_on(async {
            let mut id = None;
            let mut replayed_user = String::new();
            let mut replayed_agent = String::new();
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::Started { key: 1, result } => {
                        let sid = result.expect("started").id;
                        id = Some(sid.clone());
                        let prompt = vec![ContentBlock::Text(TextContent::new("Reply with exactly KIWI. Use no tools."))];
                        tx.unbounded_send(Command::Turn(sid, Turn::Prompt(prompt))).unwrap();
                    }
                    AgentEvent::Session(_, SessionEvent::TurnEnded(_)) if replayed_user.is_empty() => {
                        tx.unbounded_send(Command::ListSessions { cwd: cwd.clone() }).unwrap();
                    }
                    AgentEvent::Listed { sessions, .. } => {
                        let sessions = sessions.expect("listed");
                        let sid = id.clone().unwrap();
                        assert!(sessions.iter().any(|s| s.session_id == sid), "listed: {sessions:?}");
                        tx.unbounded_send(Command::LoadSession { key: 2, id: sid, cwd: cwd.clone(), tools: tools.clone() }).unwrap();
                    }
                    AgentEvent::Session(_, SessionEvent::Update(SessionUpdate::UserMessageChunk(c))) => {
                        if let ContentBlock::Text(t) = c.content {
                            replayed_user.push_str(&t.text);
                        }
                    }
                    AgentEvent::Session(_, SessionEvent::Update(SessionUpdate::AgentMessageChunk(c))) if !replayed_user.is_empty() => {
                        if let ContentBlock::Text(t) = c.content {
                            replayed_agent.push_str(&t.text);
                        }
                    }
                    AgentEvent::Started { key: 2, result } => {
                        result.expect("loaded");
                        break;
                    }
                    AgentEvent::Session(_, SessionEvent::TurnFailed { error: e, .. }) | AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
            println!("replayed user: {replayed_user:?}\nreplayed agent: {replayed_agent:?}");
            assert!(replayed_user.contains("KIWI") && replayed_agent.contains("KIWI"));
        });
    }

    #[test]
    #[ignore]
    fn live_fork() {
        use super::*;
        use agent_client_protocol::schema::v1::TextContent;

        let tools = test_tools();
        let cwd = std::env::temp_dir().join(format!("endeavor-fork-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();
        let (tx, rx) = unbounded();
        let mut events = start(Agent::Claude, rx);
        tx.unbounded_send(Command::NewSession { key: 1, cwd: cwd.clone(), tools: tools.clone() }).unwrap();
        futures::executor::block_on(async {
            let (mut original, mut copy, mut replayed) = (None, None, String::new());
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::Started { key: 1, result } => {
                        let id = result.expect("started").id;
                        original = Some(id.clone());
                        let prompt = vec![ContentBlock::Text(TextContent::new("Reply with exactly MANGO. Use no tools."))];
                        tx.unbounded_send(Command::Turn(id, Turn::Prompt(prompt))).unwrap();
                    }
                    AgentEvent::Session(id, SessionEvent::TurnEnded(_)) if Some(&id) == original.as_ref() => {
                        tx.unbounded_send(Command::ForkSession { key: 2, source: id, cwd: cwd.clone(), tools: tools.clone() }).unwrap();
                    }
                    AgentEvent::Forked { key: 2, id } => copy = Some(id),
                    AgentEvent::Session(id, SessionEvent::Update(SessionUpdate::AgentMessageChunk(c))) if Some(&id) == copy.as_ref() => {
                        if let ContentBlock::Text(t) = c.content {
                            replayed.push_str(&t.text);
                        }
                    }
                    AgentEvent::Started { key: 2, result } => {
                        result.expect("copy loaded");
                        break;
                    }
                    AgentEvent::Session(_, SessionEvent::TurnFailed { error: e, .. }) | AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
            println!("original {original:?} copy {copy:?} replayed {replayed:?}");
            assert_ne!(original, copy);
            assert!(replayed.contains("MANGO"));
        });
    }

    #[test]
    #[ignore]
    fn live_tools_through_authenticated_bridge() {
        use super::*;
        use agent_client_protocol::schema::v1::{
            PermissionOptionKind, RequestPermissionOutcome, SelectedPermissionOutcome, TextContent, ToolCallStatus,
        };

        let (_runtime, tools) = local_tools();
        let (tx, rx) = unbounded();
        let mut events = start(Agent::Claude, rx);
        tx.unbounded_send(Command::NewSession { key: 1, cwd: std::env::temp_dir(), tools }).unwrap();
        futures::executor::block_on(async {
            let (mut call, mut completed) = (None, false);
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::Started { key: 1, result } => {
                        let prompt = "Call the notebook list_notebooks tool once, then reply with exactly DONE.";
                        let prompt = vec![ContentBlock::Text(TextContent::new(prompt))];
                        tx.unbounded_send(Command::Turn(result.expect("started").id, Turn::Prompt(prompt))).unwrap();
                    }
                    AgentEvent::Session(_, SessionEvent::Permission(request, responder)) => {
                        let allow = request.options.iter().find(|o| o.kind == PermissionOptionKind::AllowOnce).expect("allow option");
                        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(allow.option_id.clone()));
                        responder.respond(RequestPermissionResponse::new(outcome)).unwrap();
                    }
                    AgentEvent::Session(_, SessionEvent::Update(SessionUpdate::ToolCall(c))) if crate::celldiff::notebook_tool(&c.title) == Some("list_notebooks") => {
                        call = Some(c.tool_call_id);
                    }
                    AgentEvent::Session(_, SessionEvent::Update(SessionUpdate::ToolCallUpdate(u))) if Some(&u.tool_call_id) == call.as_ref() => {
                        completed |= u.fields.status == Some(ToolCallStatus::Completed);
                    }
                    AgentEvent::Session(_, SessionEvent::TurnEnded(_)) => break,
                    AgentEvent::Session(_, SessionEvent::TurnFailed { error: e, .. }) | AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
            assert!(call.is_some(), "no list_notebooks call: the agent didn't get the notebook tools");
            assert!(completed, "list_notebooks didn't complete");
        });
    }

    #[test]
    #[ignore]
    fn live_modes() {
        use super::*;
        let (_runtime, tools) = local_tools();
        let (tx, rx) = unbounded();
        let mut events = start(Agent::Claude, rx);
        tx.unbounded_send(Command::NewSession { key: 1, cwd: std::env::temp_dir(), tools }).unwrap();
        futures::executor::block_on(async {
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::Started { key: 1, result } => {
                        let started = result.expect("started");
                        let modes = started.modes.expect("modes");
                        for m in &modes.available_modes {
                            println!("mode {} = {:?}", m.id, m.name);
                        }
                        println!("current {}", modes.current_mode_id);
                        for c in &started.config {
                            println!("config {} = {:?}", c.id, c.name);
                        }
                        let plan = modes.available_modes.iter().find(|m| m.id.to_string() == "plan").expect("plan mode").id.clone();
                        tx.unbounded_send(Command::SetMode(started.id, plan)).unwrap();
                    }
                    AgentEvent::Session(_, SessionEvent::Update(SessionUpdate::ConfigOptionUpdate(u))) => {
                        let mode = u.config_options.iter().find(|c| c.id.to_string() == "mode").expect("mode option");
                        println!("confirmed {:?}", mode.kind);
                        assert!(format!("{:?}", mode.kind).contains("\"plan\""));
                        break;
                    }
                    AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
        });
    }

    #[test]
    #[ignore]
    fn live_two_sessions() {
        use super::*;
        use agent_client_protocol::schema::v1::TextContent;
        use std::collections::HashMap;

        let tools = test_tools();
        let (tx, rx) = unbounded();
        let mut events = start(Agent::Claude, rx);
        for key in [1, 2] {
            tx.unbounded_send(Command::NewSession { key, cwd: "/tmp".into(), tools: tools.clone() }).unwrap();
        }
        futures::executor::block_on(async {
            let mut ids: HashMap<u64, SessionId> = HashMap::new();
            let mut text: HashMap<SessionId, String> = HashMap::new();
            let mut ended = 0;
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::Started { key, result } => {
                        let id = result.expect("session started").id;
                        ids.insert(key, id.clone());
                        if ids.len() == 2 {
                            for (key, word) in [(1, "ALPHA"), (2, "BRAVO")] {
                                let prompt = vec![ContentBlock::Text(TextContent::new(format!("Reply with exactly the word {word} and nothing else. Use no tools.")))];
                                tx.unbounded_send(Command::Turn(ids[&key].clone(), Turn::Prompt(prompt))).unwrap();
                            }
                        }
                    }
                    AgentEvent::Session(id, SessionEvent::Update(SessionUpdate::AgentMessageChunk(chunk))) => {
                        if let ContentBlock::Text(t) = chunk.content {
                            text.entry(id).or_default().push_str(&t.text);
                        }
                    }
                    AgentEvent::Session(_, SessionEvent::TurnEnded(_)) => {
                        ended += 1;
                        if ended == 2 {
                            break;
                        }
                    }
                    AgentEvent::Session(_, SessionEvent::TurnFailed { error: e, .. }) | AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
            println!("session 1: {:?}\nsession 2: {:?}", text.get(&ids[&1]), text.get(&ids[&2]));
            assert!(text[&ids[&1]].contains("ALPHA") && !text[&ids[&1]].contains("BRAVO"));
            assert!(text[&ids[&2]].contains("BRAVO") && !text[&ids[&2]].contains("ALPHA"));
        });
    }

    #[test]
    #[ignore]
    fn live_codex_session_and_plan_mode() {
        use super::*;
        let tools = test_tools();
        let (tx, rx) = unbounded();
        let mut events = start(Agent::Codex, rx);
        let cwd = std::env::temp_dir().join(format!("endeavor-codex-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        tx.unbounded_send(Command::NewSession { key: 1, cwd: cwd.canonicalize().unwrap(), tools }).unwrap();
        futures::executor::block_on(async {
            let mut signed_in = None;
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::CodexSignedIn(yes) => signed_in = Some(yes),
                    AgentEvent::Started { key: 1, result } => {
                        let started = result.expect("started");
                        let modes = started.modes.expect("modes");
                        println!("modes {:?} current {}", modes.available_modes.iter().map(|m| m.id.to_string()).collect::<Vec<_>>(), modes.current_mode_id);
                        println!("config {:?}", started.config.iter().map(|c| c.id.to_string()).collect::<Vec<_>>());
                        assert_eq!(modes.current_mode_id.to_string(), "default");
                        assert!(started.config.iter().any(|c| c.id.to_string() == "effort"));
                        assert!(!started.config.iter().any(|c| c.id.to_string() == "mode" || c.id.to_string() == "collaboration_mode"));
                        tx.unbounded_send(Command::SetMode(started.id, "plan".into())).unwrap();
                    }
                    AgentEvent::Session(_, SessionEvent::Update(SessionUpdate::CurrentModeUpdate(m))) => {
                        println!("mode {}", m.current_mode_id);
                        assert_eq!(m.current_mode_id.to_string(), "plan");
                    }
                    AgentEvent::Session(_, SessionEvent::Config(options)) => {
                        println!("confirmed {:?}", options.iter().map(|c| c.id.to_string()).collect::<Vec<_>>());
                        break;
                    }
                    AgentEvent::Session(_, SessionEvent::ConfigFailed(e)) | AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
            assert_eq!(signed_in, Some(true), "checked before connecting");
        });
    }

    #[test]
    fn the_users_own_setup_can_be_left_out_and_the_app_plugin_always_loads() {
        let off = &session_options(false, "/p", false)["claudeCode"]["options"];
        assert_eq!(off["settingSources"], serde_json::json!(["project", "local"]));
        assert_eq!(off["strictMcpConfig"], true);
        assert_eq!(off["plugins"][0]["path"], "/p");
        assert_eq!(off["disallowedTools"], serde_json::json!(["mcp__endeavor"]));

        let own = &session_options(true, "/p", false)["claudeCode"]["options"];
        assert_eq!(own["settingSources"], serde_json::json!(["user", "project", "local"]));
        assert_eq!(own["strictMcpConfig"], false);
        assert_eq!(own["plugins"][0]["path"], "/p");
    }

    #[test]
    fn the_users_own_endeavor_plugin_is_off_in_every_session() {
        for own in [false, true] {
            for on_server in [false, true] {
                let settings = &session_options(own, "/p", on_server)["claudeCode"]["options"]["settings"];
                assert_eq!(settings, &serde_json::json!({ "enabledPlugins": { "endeavor@endeavor": false } }));
            }
        }
        let codex = super::CODEX.env.iter().find(|(name, _)| *name == "CODEX_CONFIG").expect("Codex's adapter gets CODEX_CONFIG").1;
        let codex: serde_json::Value = serde_json::from_str(codex).unwrap();
        assert_eq!(codex, serde_json::json!({ "plugins": { super::USERS_ENDEAVOR_PLUGIN: { "enabled": false } } }));
        for on_server in [false, true] {
            let off = &session_options(true, "/p", on_server)["claudeCode"]["options"]["disallowedTools"];
            assert_eq!(off[0], "mcp__endeavor");
        }
    }

    #[test]
    fn a_users_codex_config_is_merged_not_replaced() {
        let ours = r#"{"plugins":{"endeavor@endeavor":{"enabled":false}}}"#;
        let theirs = r#"{"model_provider":"gateway","plugins":{"other@x":{"enabled":true},"endeavor@endeavor":{"enabled":true}}}"#;
        let merged: serde_json::Value = serde_json::from_str(&super::with_inherited("CODEX_CONFIG", ours, Some(theirs.into()))).unwrap();
        assert_eq!(merged, serde_json::json!({ "model_provider": "gateway", "plugins": { "other@x": { "enabled": true }, "endeavor@endeavor": { "enabled": false } } }));
        assert_eq!(super::with_inherited("CODEX_CONFIG", ours, None), ours);
        assert_eq!(super::with_inherited("CODEX_CONFIG", ours, Some("not json".into())), ours, "one that doesn't parse is Codex's to report, not ours to keep");
        assert_eq!(super::with_inherited("INITIAL_AGENT_MODE", "workspace-write", Some("agent".into())), "workspace-write");
    }

    #[test]
    fn the_plugins_own_files_are_read_without_asking() {
        let allowed = |dir: &str| session_options(false, dir, false)["claudeCode"]["options"]["allowedTools"].clone();
        let rule = |dir: &str| allowed(dir).as_array().unwrap().iter().filter_map(|t| t.as_str()).find(|t| t.starts_with("Read(")).map(str::to_owned);
        assert_eq!(rule("/Users/me/Library/Application Support/endeavor/plugin/0.1.0/plugin").as_deref(), Some("Read(//Users/me/Library/Application Support/endeavor/plugin/0.1.0/plugin/**)"));
        assert_eq!(rule(r"C:\Users\me\AppData\Local\Endeavor\plugin\0.1.0\plugin").as_deref(), Some("Read(//c/Users/me/AppData/Local/Endeavor/plugin/0.1.0/plugin/**)"));
        assert_eq!(rule(r"\\?\D:\Endeavor\plugin\").as_deref(), Some("Read(//d/Endeavor/plugin/**)"));
    }

    #[cfg(unix)]
    #[test]
    fn a_plugin_folder_through_a_symlink_gets_both_rules() {
        let dir = std::env::temp_dir().join(format!("endeavor-test-plugin-link-{}", std::process::id()));
        let real = dir.join("real/plugin");
        std::fs::create_dir_all(&real).unwrap();
        let _ = std::fs::remove_file(dir.join("alias"));
        std::os::unix::fs::symlink(dir.join("real"), dir.join("alias")).unwrap();
        let given = dir.join("alias/plugin");
        let rules = super::plugin_rules(given.to_str().unwrap());
        let resolved = std::fs::canonicalize(&real).unwrap();
        assert_eq!(rules, [format!("Read(/{}/**)", given.display()), format!("Read(/{}/**)", resolved.display())]);
        assert_eq!(super::plugin_rules(resolved.to_str().unwrap()).len(), 1, "no second rule without a symlink");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_run_without_asking_and_everything_else_asks() {
        let tools: Vec<String> = serde_json::from_str::<Vec<serde_json::Value>>(endeavor_mcp::NOTEBOOK_TOOLS_JSON)
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .chain(["list_folder", "read_file", "run_shell", "request_restart"].map(String::from))
            .collect();
        let asks = [
            "edit_cell", "edit_cells", "add_cell", "delete_cell", "move_cell", "fold_cell", "execute_cell", "submit_changes", "run_all_cells",
            "allow_execution", "new_notebook", "open_notebook", "keep_notebook_alive", "interrupt_notebook", "restart_notebook",
            "close_notebook", "apply_package_update", "approve_install", "answer_prompt", "set_input", "run_shell",
        ];
        for mode in [false, true] {
            let allowed = session_options(false, "/p", mode)["claudeCode"]["options"]["allowedTools"].clone();
            let allowed: Vec<String> = serde_json::from_value::<Vec<String>>(allowed).unwrap().into_iter().filter(|t| t.starts_with("mcp__")).collect();
            for tool in &tools {
                let full = format!("mcp__notebook__{tool}");
                assert_ne!(allowed.contains(&full), asks.contains(&tool.as_str()), "{tool}: decide whether it asks");
            }
            let other: Vec<&String> = allowed.iter().filter(|t| !tools.iter().any(|tool| **t == format!("mcp__notebook__{tool}"))).collect();
            assert!(other.is_empty(), "no allowed tool the runtime doesn't have: {other:?}");
            assert_eq!(allowed.len() + asks.len(), tools.len());
        }
    }

    #[test]
    fn server_sessions_lose_local_file_tools_and_gain_the_host_header() {
        let off = &session_options(false, "/p", true)["claudeCode"]["options"]["disallowedTools"];
        for tool in ["Bash", "Read", "Write", "Edit", "MultiEdit", "Glob", "Grep", "NotebookEdit"] {
            assert!(off.as_array().unwrap().iter().any(|t| t == tool), "{tool} still on");
        }
        let bridge = crate::pluto::Bridge { url: "http://127.0.0.1:9/mcp".into(), token: "t".into(), page_token: "t".into() };
        let header = |tools: super::Tools| {
            let super::McpServer::Http(http) = tools.mcp_server(7, super::Agent::Claude) else { panic!() };
            http.headers.iter().find(|h| h.name == "X-Endeavor-Host").map(|h| h.value.clone())
        };
        assert_eq!(header(super::Tools { bridge: bridge.clone(), server: Some("lab".into()) }), Some("lab".into()));
        assert_eq!(header(super::Tools { bridge: bridge.clone(), server: None }), None);
        let super::McpServer::Http(http) = (super::Tools { bridge, server: None }).mcp_server(7, super::Agent::Claude) else { panic!() };
        assert!(http.headers.iter().any(|h| h.name == "X-Endeavor-Skills" && h.value == "plugin"), "Claude Code has the skills");
        assert!(http.headers.iter().any(|h| h.name == "X-Endeavor-Client" && h.value == "app"), "the runtime answers it as the app's");
    }
}
