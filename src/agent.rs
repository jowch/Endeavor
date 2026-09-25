//! ACP connection to Claude Code, run on its own thread: one connection hosting
//! any number of sessions, each with at most one running turn. The UI talks to it
//! over two channels: [`Command`]s in, [`AgentEvent`]s out. Message queues live in
//! the UI (so they're editable and agent-agnostic); this side only runs turns.

use std::collections::HashSet;
use std::path::PathBuf;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, CloseSessionRequest, ContentBlock, DeleteSessionRequest, ForkSessionRequest, InitializeRequest, ListSessionsRequest, LoadSessionRequest, McpServer,
    McpServerSse, NewSessionRequest, PromptRequest, PromptResponse, RequestPermissionRequest,
    RequestPermissionResponse, SessionId, SessionInfo, SessionNotification, SessionUpdate, StopReason,
};
use agent_client_protocol::{AcpAgent, Agent, ConnectionTo, Responder, UntypedMessage};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::{Either, LocalBoxFuture, select};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};

/// In the app's resources, `adapter/` holds package.json + package-lock.json
/// pinning the ACP adapter and its dependencies.
const ADAPTER_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp";

/// The Node.js the adapter runs on, installed on first launch like Julia.
const NODE_VERSION: &str = "24.21.0";
#[cfg(target_arch = "aarch64")]
const NODE_TARBALL: (&str, &str, u64, &str) = (
    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-darwin-arm64.tar.gz",
    "bed7eea5325e1108f32ce5228ddd6a5f0f08a499ee42aa7442aea583702f6057",
    52_909_993,
    "node-v24.21.0-darwin-arm64",
);
#[cfg(target_arch = "x86_64")]
const NODE_TARBALL: (&str, &str, u64, &str) = (
    "https://nodejs.org/dist/v24.21.0/node-v24.21.0-darwin-x64.tar.gz",
    "1462cb3b3046b815cf8ea436d3da450ec1a9f11dac7e5a46b0ada5305d7e8097",
    54_203_979,
    "node-v24.21.0-darwin-x64",
);

/// The command that runs the ACP adapter: the app's own Node and a `npm ci` of
/// the pinned lockfile (integrity-checked), both installed on first launch.
fn adapter_command(progress: &dyn Fn(String)) -> Result<Vec<String>, String> {
    let app = crate::install::app_dir()?;
    let node_dir = app.join(format!("node-v{NODE_VERSION}"));
    let node = node_dir.join("bin/node");
    if !node.exists() {
        let (url, sha, size, top) = NODE_TARBALL;
        crate::install::tarball(&node_dir, &format!("Node.js {NODE_VERSION}"), top, (url, sha, size), progress)?;
    }

    let pinned = crate::install::resources().join("adapter");
    let manifest = std::fs::read_to_string(pinned.join("package.json")).map_err(|e| e.to_string())?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).map_err(|e| e.to_string())?;
    let version = manifest["dependencies"][ADAPTER_PACKAGE].as_str().ok_or("adapter/package.json has no adapter version")?;
    let adapter = app.join(format!("adapter-{version}"));
    let entry = adapter.join(format!("node_modules/{ADAPTER_PACKAGE}/dist/index.js"));
    if !entry.exists() {
        progress("Installing the Claude agent (first launch)…".into());
        // Install beside the target, then rename, so a partial install is never used.
        let staging = app.join("adapter.installing");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        for file in ["package.json", "package-lock.json"] {
            std::fs::copy(pinned.join(file), staging.join(file)).map_err(|e| e.to_string())?;
        }
        let npm = node_dir.join("lib/node_modules/npm/bin/npm-cli.js");
        let path = format!("{}:{}", node_dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        let out = std::process::Command::new(&node)
            .arg(npm)
            .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
            .current_dir(&staging)
            .env("PATH", path)
            .output()
            .map_err(|e| format!("Couldn't run npm: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<_> = err.lines().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect();
            return Err(format!("Couldn't install the Claude agent. Check the internet connection and restart.\n{}", tail.join("\n")));
        }
        let _ = std::fs::remove_dir_all(&adapter);
        std::fs::rename(&staging, &adapter).map_err(|e| e.to_string())?;
    }

    // The plugin's execution-gate hook calls back into this binary (see gate.rs).
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    Ok(vec![format!("ENDEAVOR_BIN={exe}"), node.display().to_string(), entry.display().to_string()])
}

/// Claude Code options for a session, in layers: Endeavor's own plugin (Pluto
/// skills and guards) always; the project's settings and CLAUDE.md (from the
/// working directory) always; the user's personal setup (user settings, their MCP
/// servers) only when they opt in (Settings).
fn session_options(personal: bool, plugin_dir: &str) -> serde_json::Value {
    let sources: &[&str] = if personal { &["user", "project", "local"] } else { &["project", "local"] };
    serde_json::json!({
        "claudeCode": { "options": {
            "settingSources": sources,
            "strictMcpConfig": !personal,
            "plugins": [{ "type": "local", "path": plugin_dir }],
        } }
    })
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
    NewSession { key: u64, cwd: PathBuf },
    /// Reopen a past session. Its history replays as session updates for `id`,
    /// then [`AgentEvent::Started`] with the same `key`.
    LoadSession { key: u64, id: SessionId, cwd: PathBuf },
    /// Open a copy of a past session (e.g. one still live in the Claude Code CLI):
    /// [`AgentEvent::Forked`] with the copy's id, its history replays, then
    /// [`AgentEvent::Started`] with the same `key`.
    ForkSession { key: u64, source: SessionId, cwd: PathBuf },
    /// Past sessions in `cwd`; answered by [`AgentEvent::Listed`].
    ListSessions { cwd: PathBuf },
    /// Stop a session (cancelling its turn); it stays in the folder's history.
    CloseSession(SessionId),
    /// Stop a session and delete its history.
    DeleteSession(SessionId),
    Turn(SessionId, Turn),
}

pub enum SessionEvent {
    Update(SessionUpdate),
    /// Answer by calling `respond` on the responder; the agent waits until then.
    Permission(RequestPermissionRequest, Responder<RequestPermissionResponse>),
    TurnEnded(StopReason),
    /// The turn failed outright (the session stays usable).
    TurnFailed(String),
    /// A `SendNow` joined the running turn.
    Steered,
    /// A `SendNow` couldn't join the running turn; it goes back to the queue.
    Unsent,
}

pub enum AgentEvent {
    Ready,
    Started { key: u64, result: Result<SessionId, String> },
    Listed { cwd: PathBuf, sessions: Vec<SessionInfo> },
    /// The copy made by `ForkSession` exists; its history replays next.
    Forked { key: u64, id: SessionId },
    Session(SessionId, SessionEvent),
    /// First-launch install progress, for the status line.
    Status(String),
    /// The connection is gone; no session works any more.
    Failed(String),
}

/// Start the agent with the app's PlutoMCP bridge (`mcp_url`, legacy SSE) attached
/// to every session. Commands sent before the connection is up wait in `commands`.
pub fn start(mcp_url: String, commands: UnboundedReceiver<Command>) -> UnboundedReceiver<AgentEvent> {
    let (event_tx, event_rx) = unbounded();
    std::thread::spawn(move || {
        let events = event_tx.clone();
        let command = adapter_command(&|line| {
            let _ = events.unbounded_send(AgentEvent::Status(line));
        });
        let reason = match command {
            Err(e) => e,
            Ok(command) => match futures::executor::block_on(run(command, mcp_url, commands, event_tx)) {
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
    Started(u64, Result<SessionId, agent_client_protocol::Error>),
    Listed(PathBuf, Result<Vec<SessionInfo>, agent_client_protocol::Error>),
    Forked(u64, PathBuf, Result<SessionId, agent_client_protocol::Error>),
}

async fn run(
    command: Vec<String>,
    mcp_url: String,
    mut commands: UnboundedReceiver<Command>,
    events: UnboundedSender<AgentEvent>,
) -> Result<(), agent_client_protocol::Error> {
    let agent = AcpAgent::from_args(command)?;
    let (notify, permit) = (events.clone(), events.clone());

    agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                let _ = notify.unbounded_send(AgentEvent::Session(n.session_id, SessionEvent::Update(n.update)));
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
        .connect_with(agent, async move |connection: ConnectionTo<Agent>| {
            let init = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let steering = init
                .meta
                .as_ref()
                .and_then(|m| m.get("steering")?.get("supported")?.as_bool())
                .unwrap_or(false);
            let pluto = McpServer::Sse(McpServerSse::new("pluto", mcp_url));
            // Read per session, so a Settings change applies to the next one.
            let plugin = crate::install::resources().join("plugin").display().to_string();
            let options = || session_options(crate::settings::Settings::load().personal_claude, &plugin).as_object().cloned();
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
                            Err(e) => emit(&session, SessionEvent::TurnFailed(e.to_string())),
                        }
                        continue;
                    }
                    Either::Left(Some(Done::Started(key, result))) => {
                        let result = result.map_err(|e| e.to_string());
                        let _ = events.unbounded_send(AgentEvent::Started { key, result });
                        continue;
                    }
                    Either::Left(Some(Done::Listed(cwd, result))) => {
                        // ponytail: a failed listing just shows no history for that folder.
                        let sessions = result.unwrap_or_default();
                        let _ = events.unbounded_send(AgentEvent::Listed { cwd, sessions });
                        continue;
                    }
                    Either::Left(Some(Done::Forked(key, cwd, result))) => {
                        match result {
                            Ok(id) => {
                                let _ = events.unbounded_send(AgentEvent::Forked { key, id: id.clone() });
                                // Load the copy so its history replays into the new session.
                                let request = LoadSessionRequest::new(id.clone(), cwd).mcp_servers(vec![pluto.clone()]).meta(options());
                                let loaded = connection.send_request(request).block_task();
                                pending.push(async move { Done::Started(key, loaded.await.map(|_| id)) }.boxed_local());
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
                    Command::NewSession { key, cwd } => {
                        let request = NewSessionRequest::new(cwd).mcp_servers(vec![pluto.clone()]).meta(options());
                        let started = connection.send_request(request).block_task();
                        pending.push(async move { Done::Started(key, started.await.map(|r| r.session_id)) }.boxed_local());
                    }
                    Command::LoadSession { key, id, cwd } => {
                        let request = LoadSessionRequest::new(id.clone(), cwd).mcp_servers(vec![pluto.clone()]).meta(options());
                        let loaded = connection.send_request(request).block_task();
                        pending.push(async move { Done::Started(key, loaded.await.map(|_| id)) }.boxed_local());
                    }
                    Command::ForkSession { key, source, cwd } => {
                        let request = ForkSessionRequest::new(source, cwd.clone()).mcp_servers(vec![pluto.clone()]).meta(options());
                        let forked = connection.send_request(request).block_task();
                        pending.push(async move { Done::Forked(key, cwd, forked.await.map(|r| r.session_id)) }.boxed_local());
                    }
                    Command::ListSessions { cwd } => {
                        // ponytail: first page only; a folder with a long history shows its newest sessions.
                        let listed = connection.send_request(ListSessionsRequest::new().cwd(cwd.clone())).block_task();
                        pending.push(async move { Done::Listed(cwd, listed.await.map(|r| r.sessions)) }.boxed_local());
                    }
                    // ponytail: fire and forget; a failed close or delete only leaves the file behind.
                    Command::CloseSession(session) => {
                        running.remove(&session);
                        connection.send_request(CloseSessionRequest::new(session)).on_receiving_result(async |_| Ok(()))?;
                    }
                    Command::DeleteSession(session) => {
                        running.remove(&session);
                        connection.send_request(DeleteSessionRequest::new(session)).on_receiving_result(async |_| Ok(()))?;
                    }
                    Command::Turn(session, Turn::Prompt(prompt) | Turn::SendNow(prompt)) if !running.contains(&session) => {
                        running.insert(session.clone());
                        let turn = connection.send_request(PromptRequest::new(session.clone(), prompt)).block_task();
                        pending.push(async move { Done::Turn(session, turn.await) }.boxed_local());
                    }
                    // The UI never sends Prompt mid-turn; hand it back rather than drop it.
                    Command::Turn(session, Turn::Prompt(_)) => emit(&session, SessionEvent::Unsent),
                    Command::Turn(session, Turn::SendNow(prompt)) if steering => {
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
                        emit(&session, SessionEvent::Unsent);
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

    /// A finished session is listed for its folder and reloads with its history:
    /// `ENDEAVOR_TEST_MCP_URL=… cargo test -- --ignored live_list_and_load`.
    #[test]
    #[ignore]
    fn live_list_and_load() {
        use super::*;
        use agent_client_protocol::schema::v1::TextContent;

        let url = std::env::var("ENDEAVOR_TEST_MCP_URL").expect("ENDEAVOR_TEST_MCP_URL");
        let cwd = std::env::temp_dir().join(format!("endeavor-history-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();
        let (tx, rx) = unbounded();
        let mut events = start(url, rx);
        tx.unbounded_send(Command::NewSession { key: 1, cwd: cwd.clone() }).unwrap();
        futures::executor::block_on(async {
            let mut id = None;
            let mut replayed_user = String::new();
            let mut replayed_agent = String::new();
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::Started { key: 1, result } => {
                        let sid = result.expect("started");
                        id = Some(sid.clone());
                        let prompt = vec![ContentBlock::Text(TextContent::new("Reply with exactly KIWI. Use no tools."))];
                        tx.unbounded_send(Command::Turn(sid, Turn::Prompt(prompt))).unwrap();
                    }
                    AgentEvent::Session(_, SessionEvent::TurnEnded(_)) if replayed_user.is_empty() => {
                        tx.unbounded_send(Command::ListSessions { cwd: cwd.clone() }).unwrap();
                    }
                    AgentEvent::Listed { sessions, .. } => {
                        let sid = id.clone().unwrap();
                        assert!(sessions.iter().any(|s| s.session_id == sid), "listed: {sessions:?}");
                        tx.unbounded_send(Command::LoadSession { key: 2, id: sid, cwd: cwd.clone() }).unwrap();
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
                    AgentEvent::Session(_, SessionEvent::TurnFailed(e)) | AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
            println!("replayed user: {replayed_user:?}\nreplayed agent: {replayed_agent:?}");
            assert!(replayed_user.contains("KIWI") && replayed_agent.contains("KIWI"));
        });
    }

    /// Forking a session gives a new id whose load replays the original's history:
    /// `ENDEAVOR_TEST_MCP_URL=… cargo test -- --ignored live_fork`.
    #[test]
    #[ignore]
    fn live_fork() {
        use super::*;
        use agent_client_protocol::schema::v1::TextContent;

        let url = std::env::var("ENDEAVOR_TEST_MCP_URL").expect("ENDEAVOR_TEST_MCP_URL");
        let cwd = std::env::temp_dir().join(format!("endeavor-fork-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();
        let (tx, rx) = unbounded();
        let mut events = start(url, rx);
        tx.unbounded_send(Command::NewSession { key: 1, cwd: cwd.clone() }).unwrap();
        futures::executor::block_on(async {
            let (mut original, mut copy, mut replayed) = (None, None, String::new());
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::Started { key: 1, result } => {
                        let id = result.expect("started");
                        original = Some(id.clone());
                        let prompt = vec![ContentBlock::Text(TextContent::new("Reply with exactly MANGO. Use no tools."))];
                        tx.unbounded_send(Command::Turn(id, Turn::Prompt(prompt))).unwrap();
                    }
                    AgentEvent::Session(id, SessionEvent::TurnEnded(_)) if Some(&id) == original.as_ref() => {
                        tx.unbounded_send(Command::ForkSession { key: 2, source: id, cwd: cwd.clone() }).unwrap();
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
                    AgentEvent::Session(_, SessionEvent::TurnFailed(e)) | AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
            println!("original {original:?} copy {copy:?} replayed {replayed:?}");
            assert_ne!(original, copy);
            assert!(replayed.contains("MANGO"));
        });
    }

    /// Two sessions on one connection with overlapping turns, each reply routed to its
    /// own session: `ENDEAVOR_TEST_MCP_URL=http://127.0.0.1:PORT/sse cargo test -- --ignored live_two_sessions`.
    #[test]
    #[ignore]
    fn live_two_sessions() {
        use super::*;
        use agent_client_protocol::schema::v1::TextContent;
        use std::collections::HashMap;

        let url = std::env::var("ENDEAVOR_TEST_MCP_URL").expect("ENDEAVOR_TEST_MCP_URL");
        let (tx, rx) = unbounded();
        let mut events = start(url, rx);
        for key in [1, 2] {
            tx.unbounded_send(Command::NewSession { key, cwd: "/tmp".into() }).unwrap();
        }
        futures::executor::block_on(async {
            let mut ids: HashMap<u64, SessionId> = HashMap::new();
            let mut text: HashMap<SessionId, String> = HashMap::new();
            let mut ended = 0;
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::Started { key, result } => {
                        let id = result.expect("session started");
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
                    AgentEvent::Session(_, SessionEvent::TurnFailed(e)) | AgentEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
            println!("session 1: {:?}\nsession 2: {:?}", text.get(&ids[&1]), text.get(&ids[&2]));
            assert!(text[&ids[&1]].contains("ALPHA") && !text[&ids[&1]].contains("BRAVO"));
            assert!(text[&ids[&2]].contains("BRAVO") && !text[&ids[&2]].contains("ALPHA"));
        });
    }

    #[test]
    fn personal_setup_is_opt_in_and_the_app_plugin_always_loads() {
        let default = &session_options(false, "/p")["claudeCode"]["options"];
        assert_eq!(default["settingSources"], serde_json::json!(["project", "local"]));
        assert_eq!(default["strictMcpConfig"], true);
        assert_eq!(default["plugins"][0]["path"], "/p");

        let personal = &session_options(true, "/p")["claudeCode"]["options"];
        assert_eq!(personal["settingSources"], serde_json::json!(["user", "project", "local"]));
        assert_eq!(personal["strictMcpConfig"], false);
        assert_eq!(personal["plugins"][0]["path"], "/p");
    }
}
