//! ACP connection to Claude Code, run on its own thread. The UI talks to it over
//! two channels: prompts (content blocks) in, [`AgentEvent`]s out.

use std::path::PathBuf;
use std::str::FromStr;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, InitializeRequest, McpServer, McpServerSse, NewSessionRequest, PromptRequest,
    RequestPermissionRequest, RequestPermissionResponse, SessionNotification, SessionUpdate,
    StopReason,
};
use agent_client_protocol::{AcpAgent, Agent, ConnectionTo, Responder};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

// ponytail: pinned adapter fetched by npx; ship it with the app when packaging.
const AGENT_CMD: &str = "npx -y @agentclientprotocol/claude-agent-acp@0.81.2";

pub enum AgentEvent {
    Ready,
    Update(SessionUpdate),
    /// Answer by calling `respond` on the responder; the agent waits until then.
    Permission(RequestPermissionRequest, Responder<RequestPermissionResponse>),
    TurnEnded(StopReason),
    Failed(String),
}

/// Start the agent with the app's PlutoMCP bridge (`mcp_url`, legacy SSE) attached.
pub fn start(mcp_url: String, cwd: PathBuf) -> (UnboundedSender<Vec<ContentBlock>>, UnboundedReceiver<AgentEvent>) {
    let (prompt_tx, prompt_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();
    std::thread::spawn(move || {
        let events = event_tx.clone();
        let result = futures::executor::block_on(run(mcp_url, cwd, prompt_rx, event_tx));
        let reason = match result {
            Ok(()) => "agent connection closed".to_string(),
            Err(e) => e.to_string(),
        };
        let _ = events.unbounded_send(AgentEvent::Failed(reason));
    });
    (prompt_tx, event_rx)
}

async fn run(
    mcp_url: String,
    cwd: PathBuf,
    mut prompts: UnboundedReceiver<Vec<ContentBlock>>,
    events: UnboundedSender<AgentEvent>,
) -> Result<(), agent_client_protocol::Error> {
    let agent = AcpAgent::from_str(AGENT_CMD)?;
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
            connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let pluto = McpServer::Sse(McpServerSse::new("pluto", mcp_url));
            let session = connection
                .send_request(NewSessionRequest::new(cwd).mcp_servers(vec![pluto]))
                .block_task()
                .await?;
            let _ = events.unbounded_send(AgentEvent::Ready);

            // ponytail: one turn at a time and no session/cancel yet.
            while let Some(prompt) = prompts.next().await {
                let response = connection
                    .send_request(PromptRequest::new(session.session_id.clone(), prompt))
                    .block_task()
                    .await?;
                let _ = events.unbounded_send(AgentEvent::TurnEnded(response.stop_reason));
            }
            Ok(())
        })
        .await
}
