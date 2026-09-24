//! ACP connection to Claude Code, run on its own thread. The UI talks to it over
//! two channels: [`Command`]s in, [`AgentEvent`]s out. The message queue lives in
//! the UI (so it's editable and agent-agnostic); this side only runs turns.

use std::path::PathBuf;
use std::str::FromStr;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, InitializeRequest, McpServer, McpServerSse, NewSessionRequest,
    PromptRequest, RequestPermissionRequest, RequestPermissionResponse, SessionNotification,
    SessionUpdate, StopReason,
};
use agent_client_protocol::{AcpAgent, Agent, ConnectionTo, Responder, UntypedMessage};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::{Either, select};
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

pub enum Command {
    /// Start a turn. The UI sends this only when idle.
    Prompt(Vec<ContentBlock>),
    /// Fold into the running turn (ACP `_session/steering`), or start a turn if idle.
    SendNow(Vec<ContentBlock>),
    /// Stop the running turn (`session/cancel`).
    Cancel,
}

pub enum AgentEvent {
    Ready,
    Update(SessionUpdate),
    /// Answer by calling `respond` on the responder; the agent waits until then.
    Permission(RequestPermissionRequest, Responder<RequestPermissionResponse>),
    TurnEnded(StopReason),
    /// A `SendNow` joined the running turn.
    Steered,
    /// A `SendNow` couldn't join the running turn; it goes back to the queue.
    Unsent,
    Failed(String),
}

/// Start the agent with the app's PlutoMCP bridge (`mcp_url`, legacy SSE) attached.
pub fn start(mcp_url: String, cwd: PathBuf) -> (UnboundedSender<Command>, UnboundedReceiver<AgentEvent>) {
    let (command_tx, command_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();
    std::thread::spawn(move || {
        let events = event_tx.clone();
        let result = futures::executor::block_on(run(mcp_url, cwd, command_rx, event_tx));
        let reason = match result {
            Ok(()) => "agent connection closed".to_string(),
            Err(e) => e.to_string(),
        };
        let _ = events.unbounded_send(AgentEvent::Failed(reason));
    });
    (command_tx, event_rx)
}

async fn run(
    mcp_url: String,
    cwd: PathBuf,
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
                let _ = notify.unbounded_send(AgentEvent::Update(n.update));
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            // Hand the responder to the UI and return at once: awaiting the user's
            // click here would stall the connection's dispatch loop.
            async move |request: RequestPermissionRequest, responder, _cx| {
                let _ = permit.unbounded_send(AgentEvent::Permission(request, responder));
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
            let request = NewSessionRequest::new(cwd)
                .mcp_servers(vec![pluto])
                .meta(session_options(personal, PLUGIN_DIR).as_object().cloned());
            let session = connection
                .send_request(request)
                .block_task()
                .await?
                .session_id;
            let _ = events.unbounded_send(AgentEvent::Ready);

            let mut turn = None;
            loop {
                // While a turn runs, keep taking commands so Cancel/SendNow can act on it.
                let command = match turn.as_mut() {
                    None => commands.next().await,
                    Some(running) => match select(running, commands.next()).await {
                        Either::Left((response, _)) => {
                            turn = None;
                            let response: agent_client_protocol::schema::v1::PromptResponse = response?;
                            let _ = events.unbounded_send(AgentEvent::TurnEnded(response.stop_reason));
                            continue;
                        }
                        Either::Right((command, _)) => command,
                    },
                };
                let Some(command) = command else { break };
                match command {
                    Command::Prompt(prompt) | Command::SendNow(prompt) if turn.is_none() => {
                        let request = PromptRequest::new(session.clone(), prompt);
                        turn = Some(connection.send_request(request).block_task().boxed_local());
                    }
                    // The UI never sends Prompt mid-turn; hand it back rather than drop it.
                    Command::Prompt(_) => {
                        let _ = events.unbounded_send(AgentEvent::Unsent);
                    }
                    Command::SendNow(prompt) if steering => {
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
                                let event = if injected { AgentEvent::Steered } else { AgentEvent::Unsent };
                                let _ = events.unbounded_send(event);
                                Ok(())
                            })?;
                    }
                    // No steering: stop the turn; the message leads the queue after it ends.
                    Command::SendNow(_) => {
                        connection.send_notification(CancelNotification::new(session.clone()))?;
                        let _ = events.unbounded_send(AgentEvent::Unsent);
                    }
                    Command::Cancel => {
                        if turn.is_some() {
                            connection.send_notification(CancelNotification::new(session.clone()))?;
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
