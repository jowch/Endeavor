//! Endeavor: app-owned Julia running Pluto + PlutoMCP, the live Pluto frontend in a
//! child webview, and an ACP agent panel wired to the same Pluto session over MCP.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};

mod agent;
mod annotate;
mod outbox;

use agent::AgentEvent;
use agent_client_protocol::Responder;
use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOption, PermissionOptionKind, RequestPermissionOutcome,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionUpdate, StopReason,
    TextContent, ToolCallId, ToolCallStatus,
};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;
use outbox::{Dispatch, Outbox, Queued};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{InputEvent, Textarea, TextareaState};
use gpui_component::text::TextView;
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
    annotate::is_uuid(id).then_some(id)
}

/// Notebook id from a PlutoMCP `open_notebook`/`new_notebook` result (`{"notebook_id": "<uuid>", …}`,
/// possibly nested as JSON text inside the MCP content array).
fn opened_notebook_id(raw: &serde_json::Value) -> Option<String> {
    let text = raw.to_string();
    let rest = &text[text.find("notebook_id")?..];
    rest.split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .find(|t| annotate::is_uuid(t))
        .map(str::to_owned)
}

actions!(endeavor, [Interrupt, ToggleAnnotation]);

struct Workspace {
    webview: Entity<WebView>,
    input: Entity<TextareaState>,
    scroll: ScrollHandle,
    entries: Vec<Entry>,
    prompts: Option<UnboundedSender<agent::Command>>,
    outbox: Outbox,
    annotating: bool,
    _runtime: Option<Runtime>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (page_tx, mut page_rx) = futures::channel::mpsc::unbounded::<String>();
        let webview = cx.new(|cx| {
            let handle = window.window_handle().expect("window handle");
            let webview = wry::WebViewBuilder::new()
                .with_devtools(true)
                .with_initialization_script(annotate::SCRIPT)
                .with_ipc_handler(move |request| {
                    let _ = page_tx.unbounded_send(request.into_body());
                })
                .build_as_child(&handle)
                .expect("child webview");
            WebView::new(webview, window, cx)
        });

        cx.spawn(async move |this, cx| {
            while let Some(body) = page_rx.next().await {
                if this.update(cx, |this, cx| this.on_page_message(&body, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        // Enter sends (queued while Claude works), Cmd+Enter sends now, Shift+Enter is a newline.
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask Claude about the notebook…")
                .submit_on_enter(true)
                .auto_grow(1, 8)
        });
        cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary, shift: false } = event {
                this.submit(input, *secondary, window, cx);
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
            outbox: Outbox::default(),
            annotating: false,
            _runtime: None,
        }
    }

    /// Which notebook the user is looking at, so "the notebook" is unambiguous.
    fn viewing_context(&self, cx: &mut Context<Self>) -> Option<ContentBlock> {
        let url = self.webview.read(cx).raw().url().unwrap_or_default();
        let id = viewed_notebook_id(&url)?;
        Some(ContentBlock::Text(TextContent::new(format!(
            "[Endeavor] The user is viewing Pluto notebook {id} in the notebook pane. \
             Unless they say otherwise, \"the notebook\" means this one."
        ))))
    }

    fn submit(&mut self, input: &Entity<TextareaState>, now: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut blocks: Vec<_> = self.viewing_context(cx).into_iter().collect();
        blocks.push(ContentBlock::Text(TextContent::new(text.clone())));
        if self.enqueue(Queued::new(text.clone(), Some(text), blocks), now, cx) {
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }

    /// Send or queue a message; false if Claude isn't connected.
    fn enqueue(&mut self, message: Queued, now: bool, cx: &mut Context<Self>) -> bool {
        if self.prompts.is_none() {
            self.note("Claude isn't connected yet.", cx);
            return false;
        }
        let dispatch = self.outbox.submit(message, now);
        self.dispatch(dispatch, cx);
        cx.notify();
        true
    }

    fn dispatch(&mut self, dispatch: Option<Dispatch>, cx: &mut Context<Self>) {
        let Some(Dispatch { command, shown }) = dispatch else { return };
        if let Some(prompts) = &self.prompts {
            let _ = prompts.unbounded_send(command);
        }
        if let Some(label) = shown {
            self.entries.push(Entry::User(label.into()));
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    fn interrupt(&mut self, _: &Interrupt, _: &mut Window, _: &mut Context<Self>) {
        if let Some(prompts) = self.prompts.as_ref().filter(|_| self.outbox.busy) {
            let _ = prompts.unbounded_send(agent::Command::Cancel);
        }
    }

    fn on_page_message(&mut self, body: &str, cx: &mut Context<Self>) {
        match annotate::parse(body) {
            Some(annotate::Message::Mode(on)) => self.annotating = on,
            Some(annotate::Message::Annotation(a)) => {
                let comment = if a.comment.is_empty() { "(no comment)" } else { a.comment.as_str() };
                let label = format!("✎ {} cell{}: {comment}", a.cells.len(), if a.cells.len() > 1 { "s" } else { "" });
                let mut blocks: Vec<_> = self.viewing_context(cx).into_iter().collect();
                blocks.extend(annotate::prompt_blocks(std::slice::from_ref(&a)));
                self.enqueue(Queued::new(label, None, blocks), a.now, cx);
            }
            None => return,
        }
        cx.notify();
    }

    /// Cmd+Shift+E from the panel (the page handles it when the notebook has focus).
    fn toggle_annotation(&mut self, _: &ToggleAnnotation, _: &mut Window, cx: &mut Context<Self>) {
        let enable = !self.annotating;
        if enable {
            // Move keyboard focus into the notebook so the comment box takes typing.
            let _ = self.webview.read(cx).raw().focus();
        }
        self.page_script(if enable { "__annotate.set(true)" } else { "__annotate.set(false)" }, cx);
    }

    fn page_script(&self, js: &str, cx: &mut Context<Self>) {
        let _ = self.webview.read(cx).raw().evaluate_script(&format!("window.__annotate && ({{ {js} }})"));
    }

    fn show_notebook(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(runtime) = &self._runtime else { return };
        // pluto_url is `http://host:port/?secret=…`; keep the secret app-side.
        let url = runtime.pluto_url.replacen("/?", &format!("/edit?id={id}&"), 1);
        self.webview.update(cx, |w, _| w.load_url(&url));
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
                self.prompts = None; // no more prompts
                return self.note(format!("Agent stopped: {e}"), cx);
            }
            AgentEvent::TurnEnded(reason) => {
                if reason != StopReason::EndTurn {
                    self.note(format!("Turn ended: {reason:?}"), cx);
                }
                let next = self.outbox.turn_ended();
                return self.dispatch(next, cx);
            }
            AgentEvent::Steered => {
                if let Some(label) = self.outbox.steered() {
                    self.entries.push(Entry::User(format!("{label}\n↳ sent into the running turn").into()));
                }
            }
            AgentEvent::Unsent => {
                let next = self.outbox.unsent();
                return self.dispatch(next, cx);
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
                    // Follow the agent: show notebooks it opens in the pane.
                    let opened = title.contains("pluto")
                        && (title.contains("open_notebook") || title.contains("new_notebook"));
                    if opened && *status == ToolCallStatus::Completed {
                        if let Some(id) = update.fields.raw_output.as_ref().and_then(opened_notebook_id) {
                            self.show_notebook(&id, cx);
                        }
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
            Entry::Agent(text) => TextView::markdown(("agent", ix), text.clone()).into_any_element(),
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

    /// Messages waiting for Claude: click ✎ to pull one back into the input, ✕ to drop it.
    fn render_queue(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = rgb(0x8a8a8a);
        div().flex().flex_col().gap_1().children(self.outbox.items.iter().enumerate().map(|(i, q)| {
            div()
                .flex()
                .gap_2()
                .text_sm()
                .text_color(muted)
                .child(div().flex_1().overflow_hidden().child(q.label.clone()))
                .when(q.in_flight(), |d| d.child("sending now…"))
                .when(!q.in_flight() && q.editable.is_some(), |d| {
                    d.child(div().id(("edit", i)).cursor_pointer().child("✎").on_click(cx.listener(
                        move |this, _, window, cx| {
                            if let Some(text) = this.outbox.take(i).and_then(|q| q.editable) {
                                this.input.update(cx, |s, cx| s.set_value(text, window, cx));
                                cx.notify();
                            }
                        },
                    )))
                })
                .when(!q.in_flight(), |d| {
                    d.child(div().id(("drop", i)).cursor_pointer().child("✕").on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.outbox.take(i);
                            cx.notify();
                        },
                    )))
                })
        }))
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries: Vec<_> = self.entries.iter().enumerate().map(|(i, e)| self.render_entry(i, e, cx)).collect();
        let button = |id: &'static str| div().id(id).px_2().rounded_sm().cursor_pointer().text_sm();
        div()
            .key_context("Workspace")
            .on_action(cx.listener(Self::interrupt))
            .on_action(cx.listener(Self::toggle_annotation))
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
                    .child(
                        div()
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .border_t_1()
                            .border_color(rgb(0x333333))
                            .child(self.render_queue(cx))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        button("annotate-toggle")
                                            .when(self.annotating, |d| d.bg(rgb(0x8a6d1f)))
                                            .child(if self.annotating { "◉ Annotating (⌘⇧E exits)" } else { "◎ Annotate (⌘⇧E)" })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_annotation(&ToggleAnnotation, window, cx)
                                            })),
                                    )
                                    .child(div().flex_1())
                                    .when(self.outbox.busy, |d| {
                                        d.child(
                                            button("stop")
                                                .bg(rgb(0x5d2f2f))
                                                .child("■ Stop (Esc)")
                                                .on_click(cx.listener(|this, _, window, cx| this.interrupt(&Interrupt, window, cx))),
                                        )
                                    }),
                            )
                            .child(Textarea::new(&self.input)),
                    ),
            )
            .child(div().flex_1().h_full().child(self.webview.clone()))
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        gpui_component::init(cx);
        // Input consumes Escape only when it has something to dismiss; otherwise it reaches us.
        cx.bind_keys([
            KeyBinding::new("escape", Interrupt, None),
            KeyBinding::new("cmd-shift-e", ToggleAnnotation, None),
        ]);
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
    use super::{opened_notebook_id, viewed_notebook_id};

    #[test]
    fn notebook_id_from_pluto_url() {
        let id = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let url = format!("http://127.0.0.1:1234/edit?secret=s3cr3t&id={id}");
        assert_eq!(viewed_notebook_id(&url), Some(id));
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/?secret=s3cr3t"), None);
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/edit?id=../../secret"), None);
    }

    #[test]
    fn notebook_id_from_open_notebook_result() {
        let id = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let inner = format!(r#"{{"path":"/tmp/a.jl","notebook_id":"{id}","ran":false}}"#);
        let raw = serde_json::json!([{ "type": "text", "text": inner }]);
        assert_eq!(opened_notebook_id(&raw).as_deref(), Some(id));
        assert_eq!(opened_notebook_id(&serde_json::json!({"error": "file_not_found"})), None);
    }
}
