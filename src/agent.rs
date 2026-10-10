//! ACP connection to Claude Code, run on its own thread: one connection hosting
//! any number of sessions, each with at most one running turn. The UI talks to it
//! over two channels: [`Command`]s in, [`AgentEvent`]s out. Message queues live in
//! the UI (so they're editable and agent-agnostic); this side only runs turns.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ClientCapabilities, ClientSessionCapabilities, NoticeCapabilities,
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

/// The agents Endeavor can run. Each has one ACP connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    #[default]
    Claude,
    Codex,
}

impl Agent {
    pub const ALL: [Agent; 2] = [Agent::Claude, Agent::Codex];

    pub fn facts(self) -> &'static AgentFacts {
        match self {
            Agent::Claude => &CLAUDE_CODE,
            Agent::Codex => &CODEX,
        }
    }

    /// The agent's name in the app ("Claude", "Codex").
    pub fn name(self) -> &'static str {
        self.facts().name
    }
}

/// A fixed sentence naming the agent, as a `&'static str`:
/// `agent_text!(agent, "You stopped ", "")` is "You stopped Claude" or "You stopped Codex".
#[macro_export]
macro_rules! agent_text {
    ($agent:expr, $before:literal, $after:literal) => {
        match $agent {
            $crate::agent::Agent::Claude => concat!($before, "Claude", $after),
            $crate::agent::Agent::Codex => concat!($before, "Codex", $after),
        }
    };
}

/// What Endeavor relies on about an agent, so each can differ.
pub struct AgentFacts {
    pub name: &'static str,
    /// Who makes it, as the agent choice says ("by Anthropic").
    pub maker: &'static str,
    /// Its ACP adapter's npm package. The app's resources folder `pins` holds
    /// package.json + package-lock.json pinning it and its dependencies, which
    /// install into `<installed>-<version>` in the app's folder.
    package: &'static str,
    pins: &'static str,
    installed: &'static str,
    /// Set on the adapter's process.
    env: &'static [(&'static str, &'static str)],
    /// It loads Endeavor's skills as a Claude Code plugin (`session_options`),
    /// so the runtime leaves out its own guide to them.
    plugin: bool,
    /// Where the agent keeps "don't ask again" rules inside the session
    /// folder, if it does: Endeavor then offers "Always in this folder" and
    /// lists the rules. None for an agent whose lasting rules live elsewhere
    /// (Cursor's and Codex's change the user's own config).
    pub folder_rules: Option<&'static str>,
    /// Its sessions can run on a server. The agent always runs on this
    /// computer; on a server its own file and shell tools must be off.
    pub on_servers: bool,
    /// It asks before every notebook write, whatever Endeavor's mode, and has
    /// no mode that leaves that to Endeavor. Endeavor answers those prompts
    /// allow-once and leaves the decision to the runtime, which holds what
    /// the session's mode asks about.
    pub asks_every_write: bool,
    /// The config options the composer shows, by id, with their names.
    pub config: &'static [(&'static str, &'static str)],
    /// How its sign-in is checked.
    pub sign_in: SignIn,
    /// Sent ahead of a new session's first message, for an agent that doesn't
    /// load Endeavor's skills: what the notebook is and where its tools are.
    pub session_intro: Option<&'static str>,
    /// It starts when one of its sessions first needs it, not with the app,
    /// so its sessions say when they wait for it to install, connect or be
    /// signed in to. (Claude starts at launch, and its sign-in has its own screens.)
    pub on_demand: bool,
}

/// How an agent's sign-in is checked and made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SignIn {
    /// `claude auth status` / `claude auth login` (signin.rs).
    ClaudeAuth,
    /// `codex login status`, and `codex-acp login` for the browser sign-in (codex.rs).
    CodexLogin,
}

/// Claude Code writes its rules to the folder's `.claude/settings.local.json`
/// (Endeavor's session options load `local` settings).
pub const CLAUDE_CODE: AgentFacts = AgentFacts {
    name: "Claude",
    maker: "by Anthropic",
    package: "@agentclientprotocol/claude-agent-acp",
    pins: "adapter",
    installed: "adapter",
    // A run waits in the runtime for the user's answer, for as long as that takes.
    env: &[("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT", "0")],
    plugin: true,
    folder_rules: Some(".claude/settings.local.json"),
    on_servers: true,
    asks_every_write: false,
    config: &[("model", "Model"), ("effort", "Effort")],
    sign_in: SignIn::ClaudeAuth,
    session_intro: None,
    on_demand: false,
};

/// Codex through `@agentclientprotocol/codex-acp`, with the `codex` it bundles
/// (docs/codex-agent.md). Its sessions start in `workspace-write`, where it
/// asks before each notebook write; Endeavor's modes come from the runtime's
/// gate and Codex's plan collaboration mode (`codex::Dialect`).
pub const CODEX: AgentFacts = AgentFacts {
    name: "Codex",
    maker: "by OpenAI",
    package: "@agentclientprotocol/codex-acp",
    pins: "adapter-codex",
    installed: "codex-adapter",
    env: &[("INITIAL_AGENT_MODE", "workspace-write")],
    plugin: false,
    folder_rules: None,
    // Its shell and file tools can't be turned off per session, and would act on this computer.
    on_servers: false,
    asks_every_write: true,
    config: &[("model", "Model"), ("effort", "Effort"), ("fast-mode", "Speed")],
    sign_in: SignIn::CodexLogin,
    // Without it Codex made a Jupyter notebook with its shell for "make a new notebook".
    session_intro: Some(
        "[Endeavor] This session works in Endeavor, where the user's notebook is a Pluto.jl notebook (Julia), \
         shown beside this chat. Do all notebook work with the notebook MCP server's tools, and call its \
         notebook_guide tool once before the first notebook call. Don't create Jupyter or other notebooks, \
         and don't start Julia or Pluto yourself from the shell.",
    ),
    on_demand: true,
};

/// The Node.js the adapter runs on, installed on first launch like Julia.
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

/// The adapter version this build of the app pins (`<pins>/package.json`).
fn pinned_adapter_version(agent: Agent) -> Result<String, String> {
    let facts = agent.facts();
    let manifest = std::fs::read_to_string(crate::install::resources().join(facts.pins).join("package.json")).map_err(|e| e.to_string())?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).map_err(|e| e.to_string())?;
    manifest["dependencies"][facts.package].as_str().map(str::to_owned).ok_or_else(|| format!("{}/package.json has no adapter version", facts.pins))
}

/// The pinned adapter version, and whether it is installed yet. A new app
/// version can pin a new adapter, which the app installs when the agent starts.
pub fn adapter_status(agent: Agent) -> Result<(String, bool), String> {
    let (_, entry) = adapter_paths(agent)?;
    Ok((pinned_adapter_version(agent)?, entry.exists()))
}

/// Where the app's Node and the pinned adapter's entry point live (installed or not).
fn adapter_paths(agent: Agent) -> Result<(PathBuf, PathBuf), String> {
    let app = crate::install::app_dir()?;
    let facts = agent.facts();
    let version = pinned_adapter_version(agent)?;
    // Windows' Node keeps node.exe at the top of its folder, not in bin/.
    let node = if cfg!(windows) { app.join(format!("node-v{NODE_VERSION}")).join("node.exe") } else { app.join(format!("node-v{NODE_VERSION}/bin/node")) };
    let entry = app.join(format!("{}-{version}/node_modules/{}/dist/index.js", facts.installed, facts.package));
    Ok((node, entry))
}

/// The Claude Code CLI bundled with the adapter (`claude <args>`). Debug builds
/// run `ENDEAVOR_CLAUDE_CLI` instead when it is set, to test sign-in without an account.
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
    // Node is a console program, and the app on Windows has no console to share.
    endeavor_mcp::client::no_window(&mut command);
    Ok(command)
}

/// The Codex adapter run with `args`: `cli <codex args>` runs the `codex` it
/// bundles, `login` its browser sign-in. Debug builds run `ENDEAVOR_CODEX_CLI`
/// instead when it is set, to test sign-in without touching the account.
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

/// Debug builds only: while the file `ENDEAVOR_TEST_NO_STEERING` names exists,
/// Cmd+Enter takes the path for an agent that can't steer (stop, then send).
fn test_no_steering() -> bool {
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("ENDEAVOR_TEST_NO_STEERING") {
        return Path::new(&file).exists();
    }
    false
}

/// Debug builds only: while the file `ENDEAVOR_FAKE_AUTH_ERROR` names exists,
/// turns fail as an expired sign-in makes them fail, without reaching Claude.
fn fake_auth_error() -> bool {
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("ENDEAVOR_FAKE_AUTH_ERROR") {
        return Path::new(&file).exists();
    }
    false
}

/// Debug builds only: while the file `ENDEAVOR_TEST_TURN_ERROR` names exists,
/// turns end as the file says, without reaching Claude. Its first line is the
/// adapter's error kind (`-` for none), or `max_tokens` / `max_turn_requests`
/// for a turn that stops at a limit; the rest is the error's message. A
/// limit, and `transport_lost`, come after a short made-up reply.
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

/// The command that runs the ACP adapter: the app's own Node and a `npm ci` of
/// the pinned lockfile (integrity-checked), both installed on first launch.
fn adapter_command(agent: Agent, progress: &dyn Fn(Progress)) -> Result<Vec<String>, String> {
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

    let pinned = crate::install::resources().join(facts.pins);
    // entry = <adapter>/node_modules/<scope>/<package>/dist/index.js
    let adapter = entry.ancestors().nth(5).ok_or("bad adapter path")?.to_path_buf();
    if !entry.exists() {
        progress(Progress::new(Step::Agent, format!("Installing the {} agent…", facts.name)));
        // Install beside the target, then rename, so a partial install is never used.
        let staging = app.join(format!("{}.installing", facts.installed));
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
        std::fs::rename(&staging, &adapter).map_err(|e| e.to_string())?;
        crate::install::remove_other_versions(&format!("{}-", facts.installed), &pinned_adapter_version(agent)?, &[]);
    }

    let env = facts.env.iter().map(|(name, value)| format!("{name}={value}"));
    let program = crate::agent_job::command(node.display().to_string(), vec![entry.display().to_string()])?;
    Ok(env.chain(program).collect())
}

/// Claude Code's own tools that read, write or run things on this Mac: off in
/// sessions on a server, whose files they can't reach (the runtime's host tools
/// stand in for them).
const LOCAL_TOOLS: [&str; 8] = ["Bash", "Read", "Write", "Edit", "MultiEdit", "Glob", "Grep", "NotebookEdit"];

/// The runtime's tools that only read: they run without asking in every mode, as
/// Claude Code's own reads do in its default mode. Claude Code prompts for any
/// MCP tool without an allow rule, `readOnlyHint` or not (2.1.280). Every other
/// notebook and host tool (edits, runs, `run_shell`) still asks in Manual.
const READ_ONLY_TOOLS: [&str; 15] = [
    "mcp__notebook__list_notebooks",
    "mcp__notebook__pluto_session_status",
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
    "mcp__notebook__list_folder",
    "mcp__notebook__read_file",
];

/// Claude Code options for a session, in layers: Endeavor's own plugin (Pluto
/// skills and guards) always; the project's settings and CLAUDE.md (from the
/// working directory) always; the user's personal setup (user settings, their MCP
/// servers) only when they opt in (Settings). On a server, no local file tools.
/// The skills point the agent at reference files beside them, outside the
/// working folder, which Claude Code would ask about (it doesn't allow reading
/// a plugin's own files by itself), so they get a Read rule.
fn session_options(personal: bool, plugin_dir: &str, on_server: bool) -> serde_json::Value {
    let sources: &[&str] = if personal { &["user", "project", "local"] } else { &["project", "local"] };
    let disallowed: &[&str] = if on_server { &LOCAL_TOOLS } else { &[] };
    let allowed: Vec<String> = READ_ONLY_TOOLS.iter().map(|t| t.to_string()).chain(plugin_rules(plugin_dir)).collect();
    serde_json::json!({
        "claudeCode": { "options": {
            "settingSources": sources,
            "strictMcpConfig": !personal,
            "plugins": [{ "type": "local", "path": plugin_dir }],
            "disallowedTools": disallowed,
            "allowedTools": allowed,
        } }
    })
}

/// The Read rules for the plugin folder. When its path goes through a symlink
/// (a symlinked home, XDG folder or AppData), Claude Code only reads without
/// asking with a rule for the path as given and one for the resolved path, so
/// both go in.
fn plugin_rules(plugin_dir: &str) -> Vec<String> {
    let mut rules = vec![format!("Read({}/**)", rule_path(plugin_dir))];
    if let Some(real) = std::fs::canonicalize(plugin_dir).ok().and_then(|p| p.to_str().map(rule_path))
        && real != rule_path(plugin_dir)
    {
        rules.push(format!("Read({real}/**)"));
    }
    rules
}

/// An absolute path as a Claude Code permission rule writes it: `//` and the
/// path with forward slashes. Claude Code compares a Windows path in POSIX
/// form, `C:\Users\me` as `/c/Users/me`, so its rule is `//c/Users/me`.
/// The rule is a gitignore-style pattern, so a folder name with `[`, `*` or
/// `?` in it would still ask; and a UNC path (`\\server\share`) isn't
/// handled, since the app's folder is never on one in practice.
fn rule_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path).replace('\\', "/");
    let path = path.trim_end_matches('/');
    match path.as_bytes() {
        [drive, b':', b'/', ..] if drive.is_ascii_alphabetic() => format!("//{}{}", drive.to_ascii_lowercase() as char, &path[2..]),
        _ => format!("/{path}"),
    }
}

/// Where a session's notebook tools are: its host's runtime bridge, and the
/// server's name when that host is a server (which also gives it the host tools).
#[derive(Clone, Debug)]
pub struct Tools {
    pub bridge: Bridge,
    pub server: Option<String>,
}

impl Tools {
    /// Each session's tool calls carry its key, so the runtime applies its policy.
    /// An agent that loads the skills from Endeavor's plugin (`session_options`)
    /// says so, and the runtime leaves out its own guide to them.
    fn mcp_server(&self, key: u64, agent: Agent) -> McpServer {
        let mut headers = vec![
            HttpHeader::new("Authorization", format!("Bearer {}", self.bridge.token)),
            HttpHeader::new("X-Endeavor-Session", key.to_string()),
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


/// What to do with a session's turn. Produced by the UI's outbox.
pub enum Turn {
    /// Start a turn. The UI sends this only when the session is idle.
    Prompt(Vec<ContentBlock>),
    /// Fold into the running turn (ACP `_session/steering`), or start a turn if idle.
    SendNow(Vec<ContentBlock>),
    /// Stop the running turn (`session/cancel`).
    Cancel,
}

pub enum Command {
    /// Create a session in `cwd`; answered by [`AgentEvent::Started`] with the same `key`.
    NewSession { key: u64, cwd: PathBuf, tools: Tools },
    /// Reopen a past session. Its history replays as session updates for `id`,
    /// then [`AgentEvent::Started`] with the same `key`.
    LoadSession { key: u64, id: SessionId, cwd: PathBuf, tools: Tools },
    /// Open a copy of a past session (e.g. one still live in the Claude Code CLI):
    /// [`AgentEvent::Forked`] with the copy's id, its history replays, then
    /// [`AgentEvent::Started`] with the same `key`.
    ForkSession { key: u64, source: SessionId, cwd: PathBuf, tools: Tools },
    /// Past sessions in `cwd`; answered by [`AgentEvent::Listed`].
    ListSessions { cwd: PathBuf },
    /// Switch a session's mode (e.g. plan); fire and forget.
    SetMode(SessionId, SessionModeId),
    /// Set a session's config option (model, effort); the reply's options come
    /// back as a config update.
    SetConfig(SessionId, String, SessionConfigValueId),
    /// Stop a session (cancelling its turn); it stays in the folder's history.
    CloseSession(SessionId),
    /// Stop a session and delete its history.
    DeleteSession(SessionId),
    Turn(SessionId, Turn),
}

pub enum SessionEvent {
    Update(SessionUpdate),
    /// The agent's reply to a config change: its options as they were then. Its
    /// mode can be older than a mode update already applied (a reply can reach
    /// the app after a notification the agent sent later), so only updates
    /// change the mode.
    Config(Vec<SessionConfigOption>),
    /// Answer by calling `respond` on the responder; the agent waits until then.
    Permission(RequestPermissionRequest, Responder<RequestPermissionResponse>),
    TurnEnded(StopReason),
    /// The turn failed (the session stays usable): the adapter's kind for it
    /// (`errorKind`, such as `server_error`, `overloaded`, `rate_limit`), when
    /// it gave one, and the error. For an API failure the agent also wrote the
    /// error as its reply.
    TurnFailed { kind: Option<String>, error: String },
    /// A model or effort change was refused.
    ConfigFailed(String),
    /// The turn failed because Claude's sign-in ran out.
    AuthRequired,
    /// A `SendNow` joined the running turn.
    Steered,
    /// A `SendNow` couldn't join the running turn; it goes back to the queue.
    Unsent,
    /// The agent can't take a `SendNow` mid-turn, so the running turn is being
    /// stopped; the message goes as soon as it ends.
    Stopping,
}

/// A session is up: its id, and the modes and config options the agent offers.
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
    /// A failed listing is an error, not an empty folder.
    Listed { cwd: PathBuf, sessions: Result<Vec<SessionInfo>, String> },
    /// The copy made by `ForkSession` exists; its history replays next.
    Forked { key: u64, id: SessionId },
    Session(SessionId, SessionEvent),
    /// Setup progress (installing Node and the adapter, then connecting).
    Setup(Progress),
    /// Claude Code's sign-in, checked before connecting: how, or None when signed out.
    SignedIn(Option<crate::signin::Method>),
    /// Codex's sign-in, checked before connecting.
    CodexSignedIn(bool),
    /// The connection is gone; no session works any more.
    Failed(String),
}

/// A mode change as the agent takes it: its own mode (ACP `session/set_mode`),
/// an option to set, and the update that tells the app at once.
#[derive(Debug, PartialEq)]
pub struct ModeSwitch {
    pub mode: Option<SessionModeId>,
    pub set: Option<(String, SessionConfigValueId)>,
    pub confirm: Option<SessionUpdate>,
}

/// How an agent's sessions read at the ACP boundary. Claude's are the shape
/// the app is written for; another agent's are translated here.
enum Dialect {
    Claude,
    Codex(crate::codex::Dialect),
}

impl Dialect {
    fn of(agent: Agent) -> Dialect {
        match agent {
            Agent::Claude => Dialect::Claude,
            Agent::Codex => Dialect::Codex(crate::codex::Dialect::default()),
        }
    }

    /// The session as the app takes it, and an option to set before anything else.
    fn started(&mut self, started: Started) -> (Started, Option<(String, SessionConfigValueId)>) {
        match self {
            Dialect::Claude => (started, None),
            Dialect::Codex(codex) => codex.started(started),
        }
    }

    fn update(&mut self, session: &SessionId, update: SessionUpdate) -> Vec<SessionUpdate> {
        match self {
            Dialect::Claude => vec![update],
            Dialect::Codex(codex) => codex.update(session, update),
        }
    }

    fn options(&self, options: Vec<SessionConfigOption>) -> Vec<SessionConfigOption> {
        match self {
            Dialect::Claude => options,
            Dialect::Codex(codex) => codex.options(options),
        }
    }

    fn set_mode(&mut self, session: &SessionId, mode: SessionModeId) -> ModeSwitch {
        match self {
            Dialect::Claude => ModeSwitch { mode: Some(mode), set: None, confirm: None },
            Dialect::Codex(codex) => codex.set_mode(session, &mode),
        }
    }

    fn option_id(&self, id: String) -> String {
        match self {
            Dialect::Claude => id,
            Dialect::Codex(codex) => codex.option_id(&id),
        }
    }

    fn closed(&mut self, session: &SessionId) {
        if let Dialect::Codex(codex) = self {
            codex.closed(session);
        }
    }
}

/// Start the agent. Each session gets its host's runtime bridge (MCP over
/// Streamable HTTP, or SSE for a runtime from before that switch)
/// from the command that opens it. Commands sent before the connection is up
/// wait in `commands`.
pub fn start(agent: Agent, commands: UnboundedReceiver<Command>) -> UnboundedReceiver<AgentEvent> {
    let (event_tx, event_rx) = unbounded();
    std::thread::spawn(move || {
        let events = event_tx.clone();
        let command = adapter_command(agent, &|p| {
            let _ = events.unbounded_send(AgentEvent::Setup(p));
        });
        // A failed install stays the agent's step, not connecting's.
        if command.is_ok() {
            let _ = events.unbounded_send(AgentEvent::Setup(Progress::new(Step::Claude, "Connecting…")));
        }
        // Checked again when the window comes back to the front, and a turn that fails for want of sign-in says so.
        if command.is_ok() {
            let signed_in = match agent.facts().sign_in {
                SignIn::ClaudeAuth => crate::signin::status().map(AgentEvent::SignedIn),
                SignIn::CodexLogin => crate::codex::signed_in().map(AgentEvent::CodexSignedIn),
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

/// Work in flight on the connection, awaited alongside incoming commands.
enum Done {
    Turn(SessionId, Result<PromptResponse, agent_client_protocol::Error>),
    Started(u64, Result<Started, agent_client_protocol::Error>),
    Listed(PathBuf, Result<Vec<SessionInfo>, agent_client_protocol::Error>),
    Forked(u64, PathBuf, Tools, Result<SessionId, agent_client_protocol::Error>),
    Config(SessionId, Result<SetSessionConfigOptionResponse, agent_client_protocol::Error>),
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
    let translate = dialect.clone();

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
            // Hand the responder to the UI and return at once: awaiting the user's
            // click here would stall the connection's dispatch loop.
            async move |request: RequestPermissionRequest, responder, _cx| {
                let session = request.session_id.clone();
                let _ = permit.unbounded_send(AgentEvent::Session(session, SessionEvent::Permission(request, responder)));
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        // Without this the loop below only notices the adapter is gone at its next command.
        .on_close(async move |_| Err(agent_client_protocol::Error::internal_error().data(format!("{}'s process exited", agent.name()))))
        .connect_with(adapter, async move |connection: ConnectionTo<agent_client_protocol::Agent>| {
            let init = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(ClientCapabilities::new().session(
                    // Without it the agent writes notices ("Auto mode unavailable: …") into its reply.
                    ClientSessionCapabilities::new().notices(NoticeCapabilities::new()),
                )))
                .block_task()
                .await?;
            let steering = init
                .meta
                .as_ref()
                .and_then(|m| m.get("steering")?.get("supported")?.as_bool())
                .unwrap_or(false);
            // Read per session, so a Settings change applies to the next one.
            let plugin = match agent.facts().plugin {
                true => Some(crate::install::plugin().map_err(|e| agent_client_protocol::Error::internal_error().data(e))?.display().to_string()),
                false => None,
            };
            let options = |tools: &Tools| {
                let plugin = plugin.as_deref()?;
                session_options(crate::settings::Settings::load().personal_claude, plugin, tools.server.is_some()).as_object().cloned()
            };
            let _ = events.unbounded_send(AgentEvent::Ready);

            let mut pending: FuturesUnordered<LocalBoxFuture<'_, Done>> = FuturesUnordered::new();
            let mut running: HashSet<SessionId> = HashSet::new();
            let emit = |session: &SessionId, event: SessionEvent| {
                let _ = events.unbounded_send(AgentEvent::Session(session.clone(), event));
            };
            loop {
                // Keep taking commands while turns run, so Cancel/SendNow can act on them.
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
                            // The adapter went: `AgentEvent::Failed` follows, and the reply is cut off.
                            Err(e) if agent_client_protocol::is_incoming_transport_closed(&e) => {}
                            // claude-agent-acp marks the failures it can tell apart (Claude's
                            // API failing after Claude Code's retries, a usage limit) with an
                            // `errorKind`, and gives the error's text as the message.
                            Err(e) => {
                                let kind = e.data.as_ref().and_then(|d| d.get("errorKind")?.as_str()).map(str::to_owned);
                                let error = if kind.is_some() { e.message.clone() } else { e.to_string() };
                                emit(&session, SessionEvent::TurnFailed { kind, error });
                            }
                        }
                        continue;
                    }
                    Either::Left(Some(Done::Started(key, result))) => {
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
                        let sessions = result.map_err(|e| e.to_string());
                        let _ = events.unbounded_send(AgentEvent::Listed { cwd, sessions });
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
                                // Load the copy so its history replays into the new session.
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
                        // ponytail: first page only; a folder with a long history shows its newest sessions.
                        let listed = connection.send_request(ListSessionsRequest::new().cwd(cwd.clone())).block_task();
                        pending.push(async move { Done::Listed(cwd, listed.await.map(|r| r.sessions)) }.boxed_local());
                    }
                    // ponytail: fire and forget; the agent confirms with a mode/config update.
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
                    // ponytail: fire and forget; a failed close or delete only leaves the file behind.
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
                    // The UI never sends Prompt mid-turn; hand it back rather than drop it.
                    Command::Turn(session, Turn::Prompt(_)) => emit(&session, SessionEvent::Unsent),
                    Command::Turn(session, Turn::SendNow(prompt)) if steering && !test_no_steering() => {
                        // promptRequired: if the turn already ended, the adapter hands the
                        // message back instead of starting a turn we don't track.
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
                    // No steering: stop the turn; the message leads the queue after it ends.
                    Command::Turn(session, Turn::SendNow(_)) => {
                        connection.send_notification(CancelNotification::new(session.clone()))?;
                        emit(&session, SessionEvent::Stopping);
                    }
                    Command::Turn(session, Turn::Cancel) => {
                        if running.contains(&session) {
                            connection.send_notification(CancelNotification::new(session))?;
                        }
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

    /// A bridge from the environment, for the live tests that bring their own
    /// runtime: `ENDEAVOR_TEST_MCP_URL` (its `/mcp` endpoint) and `ENDEAVOR_TEST_TOKEN`.
    fn test_tools() -> super::Tools {
        let url = std::env::var("ENDEAVOR_TEST_MCP_URL").expect("ENDEAVOR_TEST_MCP_URL");
        let token = std::env::var("ENDEAVOR_TEST_TOKEN").unwrap_or_default();
        super::Tools { bridge: crate::pluto::Bridge { url, token }, server: None }
    }

    /// This Mac's runtime, started for a live test.
    fn local_tools() -> (crate::runtime::Channel, super::Tools) {
        let listener = crate::runtime::Listener::start("This Mac").unwrap();
        let (channel, _) = crate::runtime::connect(false, &|_| {}).expect("helper");
        let runtime = crate::runtime::start_local(&channel, &listener, &|_| {}, |_| {}).expect("runtime");
        (channel, super::Tools { bridge: runtime.bridge, server: None })
    }

    /// A finished session is listed for its folder and reloads with its history:
    /// `ENDEAVOR_TEST_MCP_URL=… ENDEAVOR_TEST_TOKEN=… cargo test -- --ignored live_list_and_load`.
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

    /// Forking a session gives a new id whose load replays the original's history:
    /// `ENDEAVOR_TEST_MCP_URL=… ENDEAVOR_TEST_TOKEN=… cargo test -- --ignored live_fork`.
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

    /// The agent reaches the runtime's tools through the token-protected bridge (the
    /// real runtime, the real agent): `cargo test -- --ignored live_tools_through_authenticated_bridge`.
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

    /// What the agent offers as modes and config, and that a mode switch is confirmed:
    /// `cargo test -- --ignored live_modes -- --nocapture`.
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
                    // The adapter confirms through the "mode" config option (a mode update
                    // only when it falls back to another mode).
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

    /// Two sessions on one connection with overlapping turns, each reply routed to its
    /// own session: `ENDEAVOR_TEST_MCP_URL=http://127.0.0.1:PORT/mcp ENDEAVOR_TEST_TOKEN=… cargo test -- --ignored live_two_sessions`.
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
                            // Both turns in flight at once.
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

    /// Codex's session through the dialect, with no model turn: Endeavor's
    /// modes and Codex's options, and Plan as its collaboration mode. Run in a
    /// private HOME (the adapter installs into the app's folder there) with
    /// `CODEX_HOME` at a signed-in Codex: `ENDEAVOR_TEST_MCP_URL=http://127.0.0.1:9/mcp
    /// cargo test -- --ignored live_codex_session_and_plan_mode --nocapture`.
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
    fn personal_setup_is_opt_in_and_the_app_plugin_always_loads() {
        let default = &session_options(false, "/p", false)["claudeCode"]["options"];
        assert_eq!(default["settingSources"], serde_json::json!(["project", "local"]));
        assert_eq!(default["strictMcpConfig"], true);
        assert_eq!(default["plugins"][0]["path"], "/p");
        assert_eq!(default["disallowedTools"], serde_json::json!([]));

        let personal = &session_options(true, "/p", false)["claudeCode"]["options"];
        assert_eq!(personal["settingSources"], serde_json::json!(["user", "project", "local"]));
        assert_eq!(personal["strictMcpConfig"], false);
        assert_eq!(personal["plugins"][0]["path"], "/p");
    }

    /// The plugin's reference files are read without asking, wherever the app's
    /// folder is: a rule Claude Code matches, in its own form of the path.
    #[test]
    fn the_plugins_own_files_are_read_without_asking() {
        let allowed = |dir: &str| session_options(false, dir, false)["claudeCode"]["options"]["allowedTools"].clone();
        let rule = |dir: &str| allowed(dir).as_array().unwrap().iter().filter_map(|t| t.as_str()).find(|t| t.starts_with("Read(")).map(str::to_owned);
        assert_eq!(rule("/Users/me/Library/Application Support/endeavor/plugin/0.1.0/plugin").as_deref(), Some("Read(//Users/me/Library/Application Support/endeavor/plugin/0.1.0/plugin/**)"));
        assert_eq!(rule(r"C:\Users\me\AppData\Local\Endeavor\plugin\0.1.0\plugin").as_deref(), Some("Read(//c/Users/me/AppData/Local/Endeavor/plugin/0.1.0/plugin/**)"));
        assert_eq!(rule(r"\\?\D:\Endeavor\plugin\").as_deref(), Some("Read(//d/Endeavor/plugin/**)"));
    }

    /// Through a symlink, the plugin folder gets a rule for each form of its path.
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

    /// Reads run without asking; edits, runs and commands still ask. Every tool
    /// the runtime offers is on one side or the other, so a new tool needs a decision here.
    #[test]
    fn reads_run_without_asking_and_everything_else_asks() {
        let tools: Vec<String> = serde_json::from_str::<Vec<serde_json::Value>>(endeavor_mcp::NOTEBOOK_TOOLS_JSON)
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .chain(["list_folder", "read_file", "run_shell"].map(String::from))
            .collect();
        let asks = [
            "edit_cell", "edit_cells", "add_cell", "delete_cell", "move_cell", "fold_cell", "execute_cell", "submit_changes", "run_all_cells",
            "allow_execution", "new_notebook", "open_notebook", "keep_notebook_alive", "run_shell",
        ];
        for mode in [false, true] {
            let allowed = session_options(false, "/p", mode)["claudeCode"]["options"]["allowedTools"].clone();
            let allowed: Vec<String> = serde_json::from_value::<Vec<String>>(allowed).unwrap().into_iter().filter(|t| t.starts_with("mcp__")).collect();
            for tool in &tools {
                let full = format!("mcp__notebook__{tool}");
                assert_ne!(allowed.contains(&full), asks.contains(&tool.as_str()), "{tool}: decide whether it asks");
            }
            assert_eq!(allowed.len() + asks.len(), tools.len(), "no allowed tool the runtime doesn't have");
        }
    }

    #[test]
    fn server_sessions_lose_local_file_tools_and_gain_the_host_header() {
        let off = &session_options(false, "/p", true)["claudeCode"]["options"]["disallowedTools"];
        for tool in ["Bash", "Read", "Write", "Edit", "MultiEdit", "Glob", "Grep", "NotebookEdit"] {
            assert!(off.as_array().unwrap().iter().any(|t| t == tool), "{tool} still on");
        }
        let bridge = crate::pluto::Bridge { url: "http://127.0.0.1:9/mcp".into(), token: "t".into() };
        let header = |tools: super::Tools| {
            let super::McpServer::Http(http) = tools.mcp_server(7, super::Agent::Claude) else { panic!() };
            http.headers.iter().find(|h| h.name == "X-Endeavor-Host").map(|h| h.value.clone())
        };
        assert_eq!(header(super::Tools { bridge: bridge.clone(), server: Some("lab".into()) }), Some("lab".into()));
        assert_eq!(header(super::Tools { bridge: bridge.clone(), server: None }), None);
        let super::McpServer::Http(http) = (super::Tools { bridge, server: None }).mcp_server(7, super::Agent::Claude) else { panic!() };
        assert!(http.headers.iter().any(|h| h.name == "X-Endeavor-Skills" && h.value == "plugin"), "Claude Code has the skills");
    }
}
