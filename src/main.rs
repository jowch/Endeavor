//! Endeavor: app-owned Julia running Pluto + PlutoMCP, the live Pluto frontend in a
//! child webview, and an ACP agent panel wired to the same Pluto session over MCP.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};

mod agent;

use agent::AgentEvent;
use agent_client_protocol::Responder;
use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOption, PermissionOptionKind, RequestPermissionOutcome,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionUpdate, StopReason,
    TextContent, ToolCallId, ToolCallStatus,
};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Root, Theme, ThemeMode};
use gpui_wry::WebView;
use raw_window_handle::HasWindowHandle;

struct Runtime {
    // Holding the child holds its stdin; boot.jl exits when stdin closes.
    _julia: Child,
    pluto_url: String,
    mcp_url: String,
}

/// Start the app-owned Julia and block until boot.jl reports `READY`.
fn start_runtime() -> Result<Runtime, String> {
    // ponytail: dev-tree paths; resolve from the .app bundle's resources when packaging.
    let root = env!("CARGO_MANIFEST_DIR");
    // ponytail: ENDEAVOR_JULIA is the "use my Julia" opt-in; the managed download (§11) comes next.
    let julia = std::env::var("ENDEAVOR_JULIA").unwrap_or_else(|_| "julia".into());
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    // Trailing ':' stacks the default depots (~/.julia) read-only behind ours.
    let depot = format!("{home}/Library/Application Support/endeavor/depot:");

    // Hold both listeners at once so the OS can't hand out the same port twice.
    let pluto = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let mcp = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let ports = [&pluto, &mcp].map(|l| l.local_addr().unwrap().port().to_string());
    drop((pluto, mcp));

    let mut child = Command::new(&julia)
        .arg(format!("--project={root}/runtime"))
        .arg(format!("{root}/runtime/boot.jl"))
        .args(ports)
        .env("JULIA_DEPOT_PATH", depot)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("couldn't start {julia}: {e}"))?;

    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    for line in lines.by_ref() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some((pluto_url, mcp_url)) = line.strip_prefix("READY ").and_then(|r| r.split_once(' ')) {
            let (pluto_url, mcp_url) = (pluto_url.to_owned(), mcp_url.to_owned());
            // Keep draining stdout so a chatty Julia never blocks on a full pipe.
            std::thread::spawn(move || lines.map_while(Result::ok).for_each(|l| println!("{l}")));
            return Ok(Runtime { _julia: child, pluto_url, mcp_url });
        }
        println!("{line}");
    }
    Err("Julia exited before reporting READY (see stderr)".into())
}

enum Entry {
    User(SharedString),
    Agent(String),
    Tool { id: ToolCallId, title: String, status: ToolCallStatus },
    Permission { title: String, options: Vec<PermissionOption>, responder: Option<Responder<RequestPermissionResponse>> },
    Note(SharedString),
}

/// Notebook id from a Pluto `/edit?id=…` URL. Only the id is used: the URL also
/// carries Pluto's secret, which must never reach the agent.
fn viewed_notebook_id(url: &str) -> Option<&str> {
    let (path, query) = url.split_once('?')?;
    if !path.ends_with("/edit") {
        return None;
    }
    let id = query.split('&').find_map(|kv| kv.strip_prefix("id="))?;
    let is_uuid = id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    is_uuid.then_some(id)
}

struct Workspace {
    webview: Entity<WebView>,
    input: Entity<InputState>,
    scroll: ScrollHandle,
    entries: Vec<Entry>,
    prompts: Option<UnboundedSender<Vec<ContentBlock>>>,
    busy: bool,
    _runtime: Option<Runtime>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let webview = cx.new(|cx| {
            let handle = window.window_handle().expect("window handle");
            let webview = wry::WebViewBuilder::new()
                .with_devtools(true)
                .build_as_child(&handle)
                .expect("child webview");
            WebView::new(webview, window, cx)
        });

        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask Claude about the notebook…"));
        cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary: false, .. } = event {
                let text = input.read(cx).value().trim().to_string();
                if !text.is_empty() && this.send(text, cx) {
                    input.update(cx, |s, cx| s.set_value("", window, cx));
                    cx.notify();
                }
            }
        })
        .detach();

        // First run instantiates + precompiles (~1 min); later launches are seconds.
        let boot = cx.background_executor().spawn(async { start_runtime() });
        cx.spawn(async move |this, cx| {
            let runtime = match boot.await {
                Ok(runtime) => runtime,
                Err(e) => {
                    let _ = this.update(cx, |this, cx| this.note(format!("Runtime failed: {e}"), cx));
                    return;
                }
            };
            let cwd = std::env::current_dir().unwrap_or_else(|_| "/".into());
            let (prompts, mut events) = agent::start(runtime.mcp_url.clone(), cwd);
            let ok = this.update(cx, |this, cx| {
                this.webview.update(cx, |w, _| w.load_url(&runtime.pluto_url));
                this.note(format!("Pluto ready · MCP {}\nConnecting to Claude…", runtime.mcp_url), cx);
                this.prompts = Some(prompts);
                this._runtime = Some(runtime);
            });
            if ok.is_err() {
                return;
            }
            while let Some(event) = events.next().await {
                if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            webview,
            input,
            scroll: ScrollHandle::new(),
            entries: vec![Entry::Note("Starting Julia…".into())],
            prompts: None,
            busy: false,
            _runtime: None,
        }
    }

    /// Queue a prompt; false if the agent isn't ready or is mid-turn.
    fn send(&mut self, text: String, cx: &mut Context<Self>) -> bool {
        let Some(prompts) = self.prompts.as_ref().filter(|_| !self.busy) else {
            return false;
        };
        let mut prompt = Vec::new();
        let url = self.webview.read(cx).raw().url().unwrap_or_default();
        if let Some(id) = viewed_notebook_id(&url) {
            prompt.push(ContentBlock::Text(TextContent::new(format!(
                "[Endeavor] The user is viewing Pluto notebook {id} in the notebook pane. \
                 Unless they say otherwise, \"the notebook\" means this one."
            ))));
        }
        prompt.push(ContentBlock::Text(TextContent::new(text.clone())));
        self.busy = prompts.unbounded_send(prompt).is_ok();
        if self.busy {
            self.entries.push(Entry::User(text.into()));
            self.scroll.scroll_to_bottom();
        }
        self.busy
    }

    fn note(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.entries.push(Entry::Note(text.into()));
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn on_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        match event {
            AgentEvent::Ready => return self.note("Claude connected with the pluto MCP server.", cx),
            AgentEvent::Failed(e) => {
                self.busy = true; // no more prompts
                return self.note(format!("Agent stopped: {e}"), cx);
            }
            AgentEvent::TurnEnded(reason) => {
                self.busy = false;
                if reason != StopReason::EndTurn {
                    return self.note(format!("Turn ended: {reason:?}"), cx);
                }
            }
            AgentEvent::Permission(request, responder) => self.entries.push(Entry::Permission {
                title: request.tool_call.fields.title.unwrap_or_else(|| "Tool call".into()),
                options: request.options,
                responder: Some(responder),
            }),
            AgentEvent::Update(SessionUpdate::AgentMessageChunk(chunk)) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Agent(text)) => text.push_str(&t.text),
                        _ => self.entries.push(Entry::Agent(t.text)),
                    }
                }
            }
            AgentEvent::Update(SessionUpdate::ToolCall(call)) => self.entries.push(Entry::Tool {
                id: call.tool_call_id,
                title: call.title,
                status: call.status,
            }),
            AgentEvent::Update(SessionUpdate::ToolCallUpdate(update)) => {
                let tool = self.entries.iter_mut().rev().find_map(|e| match e {
                    Entry::Tool { id, title, status } if *id == update.tool_call_id => Some((title, status)),
                    _ => None,
                });
                if let Some((title, status)) = tool {
                    if let Some(t) = update.fields.title {
                        *title = t;
                    }
                    if let Some(s) = update.fields.status {
                        *status = s;
                    }
                }
            }
            // ponytail: thoughts, plans, modes, usage not shown yet.
            AgentEvent::Update(_) => return,
        }
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn answer(&mut self, ix: usize, option: &PermissionOption, cx: &mut Context<Self>) {
        let Some(Entry::Permission { title, responder, .. }) = self.entries.get_mut(ix) else {
            return;
        };
        if let Some(responder) = responder.take() {
            let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option.option_id.clone()));
            let _ = responder.respond(RequestPermissionResponse::new(outcome));
            self.entries[ix] = Entry::Note(format!("{}: {}", option.name, title).into());
            cx.notify();
        }
    }

    fn render_entry(&self, ix: usize, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let muted = rgb(0x8a8a8a);
        match entry {
            Entry::User(text) => div().p_2().rounded_md().bg(rgb(0x2d2d30)).child(text.clone()).into_any_element(),
            Entry::Agent(text) => div().child(text.clone()).into_any_element(),
            Entry::Note(text) => div().text_sm().text_color(muted).child(text.clone()).into_any_element(),
            Entry::Tool { title, status, .. } => div()
                .text_sm()
                .text_color(muted)
                .child(format!("⚙ {title} · {status:?}"))
                .into_any_element(),
            Entry::Permission { title, options, .. } => div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(0xc8a040))
                .child(format!("Allow {title}?"))
                .child(div().flex().gap_2().children(options.iter().enumerate().map(|(i, option)| {
                    let option = option.clone();
                    let allow = matches!(option.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways);
                    div()
                        .id(("perm", ix * 16 + i))
                        .px_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(if allow { rgb(0x2f5d3a) } else { rgb(0x5d2f2f) })
                        .child(option.name.clone())
                        .on_click(cx.listener(move |this, _, _, cx| this.answer(ix, &option, cx)))
                })))
                .into_any_element(),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries: Vec<_> = self.entries.iter().enumerate().map(|(i, e)| self.render_entry(i, e, cx)).collect();
        div()
            .flex()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xdddddd))
            .child(
                div()
                    .w(px(420.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(rgb(0x333333))
                    .child(
                        div()
                            .id("transcript")
                            .flex_1()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .children(entries),
                    )
                    .child(div().p_3().border_t_1().border_color(rgb(0x333333)).child(Input::new(&self.input))),
            )
            .child(div().flex_1().h_full().child(self.webview.clone()))
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        gpui_component::init(cx);
        let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                Theme::change(ThemeMode::Dark, Some(window), cx);
                let workspace = cx.new(|cx| Workspace::new(window, cx));
                cx.new(|cx| Root::new(workspace, window, cx))
            },
        )
        .unwrap();
        // Quitting closes Julia's stdin, which shuts the runtime down.
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::viewed_notebook_id;

    #[test]
    fn notebook_id_from_pluto_url() {
        let id = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let url = format!("http://127.0.0.1:1234/edit?secret=s3cr3t&id={id}");
        assert_eq!(viewed_notebook_id(&url), Some(id));
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/?secret=s3cr3t"), None);
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/edit?id=../../secret"), None);
    }
}
