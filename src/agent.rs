//! ACP connection to Claude Code, run on its own thread: one connection hosting
//! any number of sessions, each with at most one running turn. The UI talks to it
//! over two channels: [`Command`]s in, [`AgentEvent`]s out. Message queues live in
//! the UI (so they're editable and agent-agnostic); this side only runs turns.

use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, InitializeRequest, McpServer, McpServerSse, NewSessionRequest,
    PromptRequest, PromptResponse, RequestPermissionRequest, RequestPermissionResponse, SessionId,
    SessionNotification, SessionUpdate, StopReason,
};
use agent_client_protocol::{AcpAgent, Agent, ConnectionTo, Responder, UntypedMessage};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::{Either, LocalBoxFuture, select};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};

// ponytail: pinned adapter fetched by npx; ship it with the app when packaging.
const AGENT_CMD: &str = "npx -y @agentclientprotocol/claude-agent-acp@0.81.2";
// ponytail: dev-tree path; resolve from the .app bundle's resources when packaging.
const PLUGIN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/plugin");

/// Claude Code options for a session, in layers: Endeavor's own plugin (Pluto
/// skills and guards) always; the project's settings and CLAUDE.md (from the
/// working directory) always; the user's personal setup (user settings, their MCP
/// servers) only when they opt in with ENDEAVOR_PERSONAL_CLAUDE=1.
// ponytail: env-var opt-in; becomes an app setting once there's a settings UI.
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
    Session(SessionId, SessionEvent),
    /// The connection is gone; no session works any more.
    Failed(String),
}

/// Start the agent with the app's PlutoMCP bridge (`mcp_url`, legacy SSE) attached
/// to every session. Commands sent before the connection is up wait in `commands`.
pub fn start(mcp_url: String, commands: UnboundedReceiver<Command>) -> UnboundedReceiver<AgentEvent> {
    let (event_tx, event_rx) = unbounded();
    std::thread::spawn(move || {
        let events = event_tx.clone();
        let result = futures::executor::block_on(run(mcp_url, commands, event_tx));
        let reason = match result {
            Ok(()) => "agent connection closed".to_string(),
            Err(e) => e.to_string(),
        };
        let _ = events.unbounded_send(AgentEvent::Failed(reason));
    });
    event_rx
}

/// Work in flight on the connection, awaited alongside incoming commands.
enum Done {
    Turn(SessionId, Result<PromptResponse, agent_client_protocol::Error>),
    Started(u64, Result<SessionId, agent_client_protocol::Error>),
}

async fn run(
    mcp_url: String,
    mut commands: UnboundedReceiver<Command>,
    events: UnboundedSender<AgentEvent>,
) -> Result<(), agent_client_protocol::Error> {
    // The plugin's execution-gate hook calls back into this binary (see gate.rs).
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let command = format!("env ENDEAVOR_BIN='{}' {AGENT_CMD}", exe.replace('\'', r"'\''"));
    let agent = AcpAgent::from_str(&command)?;
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
            let personal = std::env::var_os("ENDEAVOR_PERSONAL_CLAUDE").is_some_and(|v| v == "1");
            let options = session_options(personal, PLUGIN_DIR).as_object().cloned();
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
                    Either::Left(None) => continue,
                    Either::Right(None) => break,
                    Either::Right(Some(command)) => command,
                };
                match command {
                    Command::NewSession { key, cwd } => {
                        let request = NewSessionRequest::new(cwd).mcp_servers(vec![pluto.clone()]).meta(options.clone());
                        let started = connection.send_request(request).block_task();
                        pending.push(async move { Done::Started(key, started.await.map(|r| r.session_id)) }.boxed_local());
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
