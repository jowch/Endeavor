//! Endeavor: app-owned Julia running Pluto + PlutoMCP, the live Pluto frontend in a
//! child webview, and an ACP agent panel wired to the same Pluto session over MCP.

use std::time::Duration;

mod agent;
mod annotate;
mod celldiff;
mod gate;
mod outbox;
mod pluto;
mod runtime;

use agent::AgentEvent;
use agent_client_protocol::Responder;
use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOption, PermissionOptionKind, RequestPermissionOutcome,
    PlanEntry, PlanEntryStatus, RequestPermissionResponse, SelectedPermissionOutcome, SessionUpdate,
    StopReason, TextContent, ToolCallId, ToolCallStatus,
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
use runtime::Runtime;

enum Entry {
    User(SharedString),
    Agent(String),
    Tool {
        id: ToolCallId,
        title: String,
        status: ToolCallStatus,
        input: Option<serde_json::Value>,
        output: Option<serde_json::Value>,
        /// Cell edits made by this call, shown inline.
        diffs: Vec<celldiff::CellDiff>,
        expanded: bool,
    },
    Thought { text: String, expanded: bool },
    Plan(Vec<PlanEntry>),
    Permission {
        title: String,
        /// Code the call would run, when known.
        code: Option<String>,
        options: Vec<PermissionOption>,
        responder: Option<Responder<RequestPermissionResponse>>,
        /// Raised by the execution gate (a pluto call that runs code).
        runs_code: bool,
    },
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

actions!(endeavor, [Interrupt, ToggleAnnotation]);

struct Workspace {
    webview: Entity<WebView>,
    input: Entity<TextareaState>,
    scroll: ScrollHandle,
    entries: Vec<Entry>,
    prompts: Option<UnboundedSender<agent::Command>>,
    outbox: Outbox,
    /// Each cell's code as last seen in the agent's reads and edits, for diffs.
    cell_codes: celldiff::CellCodes,
    /// "Allow & stop asking": approve the agent's runs for the rest of this session.
    run_without_asking: bool,
    annotating: bool,
    runtime: Option<Runtime>,
    /// Booting or restarting Julia.
    starting: bool,
    /// Ports of the runtime that died, reused on restart so the agent reconnects.
    last_ports: Option<[u16; 2]>,
    /// Open notebooks (id, path) as last seen, to reopen after a restart.
    last_notebooks: Vec<(String, String)>,
    died_tx: UnboundedSender<String>,
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

        let (died_tx, mut died_rx) = futures::channel::mpsc::unbounded::<String>();
        cx.spawn(async move |this, cx| {
            while let Some(reason) = died_rx.next().await {
                if this.update(cx, |this, cx| this.on_runtime_died(reason, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        // Keep the open-notebook list fresh so a restart can reopen them.
        // ponytail: 10 s poll of a loopback call; a Pluto open/close event would be exact.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(10)).await;
            let Ok(url) = this.update(cx, |this, _| this.runtime.as_ref().map(|r| r.mcp_url.clone())) else { break };
            let Some(url) = url else { continue };
            let list = cx
                .background_executor()
                .spawn(async move { pluto::call_tool(&url, "list_notebooks", serde_json::json!({})) })
                .await;
            if let Ok(list) = list {
                let _ = this.update(cx, |this, _| this.remember_notebooks(&list));
            }
        })
        .detach();

        let mut this = Self {
            webview,
            input,
            scroll: ScrollHandle::new(),
            entries: Vec::new(),
            prompts: None,
            outbox: Outbox::default(),
            cell_codes: celldiff::CellCodes::default(),
            run_without_asking: false,
            annotating: false,
            runtime: None,
            starting: false,
            last_ports: None,
            last_notebooks: Vec::new(),
            died_tx,
        };
        this.boot(None, cx);
        this
    }

    /// Start Julia, or restart it on the previous ports.
    fn boot(&mut self, ports: Option<[u16; 2]>, cx: &mut Context<Self>) {
        self.starting = true;
        self.note(if ports.is_some() { "Restarting Julia…" } else { "Starting Julia…" }, cx);
        let died = self.died_tx.clone();
        // First run instantiates + precompiles (~1 min); later launches are seconds.
        let boot = cx.background_executor().spawn(async move { runtime::start(ports, died) });
        cx.spawn(async move |this, cx| {
            let result = boot.await;
            let _ = this.update(cx, |this, cx| this.on_booted(result, cx));
        })
        .detach();
    }

    fn on_booted(&mut self, result: Result<Runtime, String>, cx: &mut Context<Self>) {
        self.starting = false;
        let runtime = match result {
            Ok(runtime) => runtime,
            Err(e) => return self.note(format!("⚠ {e}"), cx),
        };
        self.webview.update(cx, |w, _| w.load_url(&runtime.pluto_url));
        let mcp_url = runtime.mcp_url.clone();
        self.runtime = Some(runtime);
        if self.prompts.is_none() {
            self.note(format!("Pluto ready · MCP {mcp_url}\nConnecting to Claude…"), cx);
            self.start_agent(mcp_url, cx);
        } else {
            self.note("Julia restarted.", cx);
            self.reopen_notebooks(cx);
        }
    }

    fn start_agent(&mut self, mcp_url: String, cx: &mut Context<Self>) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| "/".into());
        let (prompts, mut events) = agent::start(mcp_url, cwd);
        self.prompts = Some(prompts);
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_runtime_died(&mut self, reason: String, cx: &mut Context<Self>) {
        self.last_ports = self.runtime.take().map(|r| r.ports);
        self.note(format!("⚠ {reason}\nNotebook tools are unavailable until Julia restarts."), cx);
    }

    fn remember_notebooks(&mut self, list: &serde_json::Value) {
        self.last_notebooks = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|nb| Some((nb["notebook_id"].as_str()?.to_owned(), nb["path"].as_str()?.to_owned())))
            .collect();
    }

    /// Reopen the notebooks that were open when Julia died, and show the one the
    /// user was viewing. Pluto saves on every change, so the files are current.
    fn reopen_notebooks(&mut self, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        let url = self.webview.read(cx).raw().url().unwrap_or_default();
        let viewed = viewed_notebook_id(&url).map(str::to_owned);
        let viewed_path = self.last_notebooks.iter().find(|(id, _)| Some(id) == viewed.as_ref()).map(|(_, p)| p.clone());
        let paths: Vec<String> = self.last_notebooks.iter().map(|(_, p)| p.clone()).collect();
        let reopen = cx.background_executor().spawn(async move {
            paths
                .into_iter()
                .filter_map(|path| {
                    let result = pluto::call_tool(&mcp_url, "open_notebook", serde_json::json!({ "path": path })).ok()?;
                    Some((result["notebook_id"].as_str()?.to_owned(), path))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let reopened = reopen.await;
            let _ = this.update(cx, |this, cx| {
                if let Some((id, _)) = reopened.iter().find(|(_, p)| Some(p) == viewed_path.as_ref()) {
                    this.show_notebook(&id.clone(), cx);
                }
                if !reopened.is_empty() {
                    this.note(format!("Reopened {} notebook(s) in safe preview.", reopened.len()), cx);
                }
                this.last_notebooks = reopened;
            });
        })
        .detach();
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

    /// After the agent goes idle, say if it left edited cells unrun or still running.
    fn check_run_state(&mut self, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        let list = cx
            .background_executor()
            .spawn(async move { pluto::call_tool(&mcp_url, "list_notebooks", serde_json::json!({})) });
        cx.spawn(async move |this, cx| {
            // ponytail: a failed check stays silent; it's advisory, and a dead runtime reports itself.
            let Ok(list) = list.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.remember_notebooks(&list);
                for warning in pluto::run_warnings(&list) {
                    this.note(format!("⚠ {warning}"), cx);
                }
            });
        })
        .detach();
    }

    fn show_notebook(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(runtime) = &self.runtime else { return };
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
                let idle = next.is_none();
                self.dispatch(next, cx);
                if idle {
                    self.check_run_state(cx);
                }
                return;
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
            AgentEvent::Permission(request, responder) => {
                let fields = &request.tool_call.fields;
                let title = fields.title.clone().unwrap_or_else(|| "Tool call".into());
                let runs_code = gate::is_pluto(&title);
                if runs_code && self.run_without_asking {
                    if let Some(allow) = option_of_kind(&request.options, PermissionOptionKind::AllowOnce) {
                        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(allow.option_id.clone()));
                        let _ = responder.respond(RequestPermissionResponse::new(outcome));
                        return self.note(format!("▶ Ran without asking: {title}"), cx);
                    }
                }
                let input = fields.raw_input.clone().unwrap_or_default();
                let code = input["code"].as_str().map(str::to_owned).or_else(|| {
                    input["cell_id"].as_str().and_then(|id| self.cell_codes.get(id)).map(str::to_owned)
                });
                self.entries.push(Entry::Permission {
                    title,
                    code,
                    options: request.options,
                    responder: Some(responder),
                    runs_code,
                });
            }
            AgentEvent::Update(SessionUpdate::AgentMessageChunk(chunk)) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Agent(text)) => text.push_str(&t.text),
                        _ => self.entries.push(Entry::Agent(t.text)),
                    }
                }
            }
            AgentEvent::Update(SessionUpdate::AgentThoughtChunk(chunk)) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Thought { text, .. }) => text.push_str(&t.text),
                        _ => self.entries.push(Entry::Thought { text: t.text, expanded: false }),
                    }
                }
            }
            // One plan per turn, updated in place.
            AgentEvent::Update(SessionUpdate::Plan(plan)) => {
                let turn_start = self.entries.iter().rposition(|e| matches!(e, Entry::User(_))).unwrap_or(0);
                match self.entries[turn_start..].iter_mut().find(|e| matches!(e, Entry::Plan(_))) {
                    Some(existing) => *existing = Entry::Plan(plan.entries),
                    None => self.entries.push(Entry::Plan(plan.entries)),
                }
            }
            AgentEvent::Update(SessionUpdate::ToolCall(call)) => self.entries.push(Entry::Tool {
                id: call.tool_call_id,
                title: call.title,
                status: call.status,
                input: call.raw_input,
                output: call.raw_output,
                diffs: Vec::new(),
                expanded: false,
            }),
            AgentEvent::Update(SessionUpdate::ToolCallUpdate(update)) => {
                if let Some(opened) = self.on_tool_update(update) {
                    // Follow the agent: show notebooks it opens in the pane.
                    self.show_notebook(&opened, cx);
                }
            }
            // ponytail: modes, usage, available commands not shown yet.
            AgentEvent::Update(_) => return,
        }
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Apply a tool-call update; on a completed pluto call, learn cell code and
    /// diff edits. Returns a notebook id the agent just opened or created.
    fn on_tool_update(&mut self, update: agent_client_protocol::schema::v1::ToolCallUpdate) -> Option<String> {
        let ix = self
            .entries
            .iter()
            .rposition(|e| matches!(e, Entry::Tool { id, .. } if *id == update.tool_call_id))?;
        let Entry::Tool { title, status, input, output, diffs, .. } = &mut self.entries[ix] else { return None };
        let fields = update.fields;
        if let Some(t) = fields.title {
            *title = t;
        }
        if let Some(s) = fields.status {
            *status = s;
        }
        if fields.raw_input.is_some() {
            *input = fields.raw_input;
        }
        if fields.raw_output.is_some() {
            *output = fields.raw_output;
        }
        let tool = celldiff::pluto_tool(title).filter(|_| *status == ToolCallStatus::Completed)?;
        let result = output.as_ref().and_then(celldiff::tool_json)?;
        if result.get("error").is_some() {
            return None;
        }
        self.cell_codes.observe(tool, &result);
        if let Some(input) = input {
            *diffs = self.cell_codes.diff(tool, input);
        }
        matches!(tool, "open_notebook" | "new_notebook")
            .then(|| result["notebook_id"].as_str().map(str::to_owned))
            .flatten()
    }

    fn toggle(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(Entry::Tool { expanded, .. } | Entry::Thought { expanded, .. }) = self.entries.get_mut(ix) {
            *expanded = !*expanded;
            cx.notify();
        }
    }

    /// Answer a permission request; `stop_asking` approves later runs this session.
    fn answer(&mut self, ix: usize, option: &PermissionOption, stop_asking: bool, cx: &mut Context<Self>) {
        let Some(Entry::Permission { title, responder, .. }) = self.entries.get_mut(ix) else {
            return;
        };
        if let Some(responder) = responder.take() {
            let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option.option_id.clone()));
            let _ = responder.respond(RequestPermissionResponse::new(outcome));
            let verb = if stop_asking { "Allowed (won't ask again this session)" } else { option.name.as_str() };
            self.entries[ix] = Entry::Note(format!("{verb}: {title}").into());
            self.run_without_asking |= stop_asking;
            cx.notify();
        }
    }

    fn render_entry(&self, ix: usize, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let muted = rgb(0x8a8a8a);
        match entry {
            Entry::User(text) => div().p_2().rounded_md().bg(rgb(0x2d2d30)).child(text.clone()).into_any_element(),
            Entry::Agent(text) => TextView::markdown(("agent", ix), text.clone()).into_any_element(),
            Entry::Note(text) => div().text_sm().text_color(muted).child(text.clone()).into_any_element(),
            Entry::Tool { title, status, input, output, diffs, expanded, .. } => {
                let arrow = if *expanded { "▾" } else { "▸" };
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .child(
                        div()
                            .id(("tool", ix))
                            .cursor_pointer()
                            .text_color(muted)
                            .child(format!("{arrow} ⚙ {title} · {status:?}"))
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle(ix, cx))),
                    )
                    .children(diffs.iter().map(render_diff))
                    .when(*expanded, |d| {
                        d.children(input.as_ref().map(|v| detail("input", v)))
                            .children(output.as_ref().map(|v| detail("result", &celldiff::tool_json(v).unwrap_or_else(|| v.clone()))))
                    })
                    .into_any_element()
            }
            Entry::Thought { text, expanded } => div()
                .flex()
                .flex_col()
                .gap_1()
                .text_sm()
                .text_color(muted)
                .child(
                    div()
                        .id(("thought", ix))
                        .cursor_pointer()
                        .child(if *expanded { "▾ Thinking" } else { "▸ Thinking" })
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle(ix, cx))),
                )
                .when(*expanded, |d| d.child(div().italic().child(text.clone())))
                .into_any_element(),
            Entry::Plan(entries) => div()
                .flex()
                .flex_col()
                .gap_1()
                .text_sm()
                .child(div().text_color(muted).child("Plan"))
                .children(entries.iter().map(|e| {
                    let (mark, color) = match e.status {
                        PlanEntryStatus::Completed => ("☑", rgb(0x6a9955)),
                        PlanEntryStatus::InProgress => ("◐", rgb(0xc8a040)),
                        _ => ("☐", rgb(0xaaaaaa)),
                    };
                    div().flex().gap_2().child(div().text_color(color).child(mark)).child(e.content.clone())
                }))
                .into_any_element(),
            Entry::Permission { title, code, options, runs_code, .. } => {
                // Run approvals: Allow / Allow & stop asking / Deny. Anything else: the agent's own options.
                let mut buttons: Vec<(String, PermissionOption, bool)> = Vec::new();
                if *runs_code {
                    if let Some(allow) = option_of_kind(options, PermissionOptionKind::AllowOnce) {
                        buttons.push(("Allow".into(), allow.clone(), false));
                        buttons.push(("Allow & stop asking".into(), allow.clone(), true));
                    }
                    if let Some(deny) = option_of_kind(options, PermissionOptionKind::RejectOnce) {
                        buttons.push(("Deny".into(), deny.clone(), false));
                    }
                }
                if buttons.is_empty() {
                    buttons = options.iter().map(|o| (o.name.clone(), o.clone(), false)).collect();
                }
                let heading = match celldiff::pluto_tool(title) {
                    Some(tool) if *runs_code => format!("Run code? · {tool}"),
                    _ => format!("Allow {title}?"),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0xc8a040))
                    .child(heading)
                    .children(code.as_ref().map(|code| {
                        div().p_1().rounded_sm().bg(rgb(0x252526)).font_family("Menlo").text_xs().child(code.clone())
                    }))
                    .child(div().flex().gap_2().children(buttons.into_iter().enumerate().map(|(i, (label, option, stop))| {
                        let allow = matches!(option.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways);
                        div()
                            .id(("perm", ix * 16 + i))
                            .px_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(if allow { rgb(0x2f5d3a) } else { rgb(0x5d2f2f) })
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| this.answer(ix, &option, stop, cx)))
                    })))
                    .into_any_element()
            }
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

fn option_of_kind(options: &[PermissionOption], kind: PermissionOptionKind) -> Option<&PermissionOption> {
    options.iter().find(|o| o.kind == kind)
}

/// A cell edit as colored lines.
fn render_diff(diff: &celldiff::CellDiff) -> impl IntoElement + use<> {
    use celldiff::Change;
    // ponytail: long diffs are cut, not scrollable; expand the tool call for its input.
    const MAX_LINES: usize = 60;
    div()
        .flex()
        .flex_col()
        .rounded_sm()
        .border_1()
        .border_color(rgb(0x333333))
        .font_family("Menlo")
        .text_xs()
        .child(div().px_2().text_color(rgb(0x8a8a8a)).child(diff.label.clone()))
        .children(diff.lines.iter().take(MAX_LINES).map(|(change, line)| {
            let (sign, bg) = match change {
                Change::Added => ("+", Some(rgb(0x1f3a26))),
                Change::Removed => ("-", Some(rgb(0x4a2226))),
                Change::Same => (" ", None),
            };
            div()
                .px_2()
                .when_some(bg, |d, bg| d.bg(bg))
                .child(format!("{sign} {line}"))
        }))
        .when(diff.lines.len() > MAX_LINES, |d| {
            d.child(div().px_2().text_color(rgb(0x8a8a8a)).child(format!("… {} more lines", diff.lines.len() - MAX_LINES)))
        })
}

/// A tool call's input or result, pretty-printed and truncated.
fn detail(label: &str, value: &serde_json::Value) -> impl IntoElement + use<> {
    const MAX_CHARS: usize = 2000;
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let text = match text.char_indices().nth(MAX_CHARS) {
        Some((cut, _)) => format!("{}\n…", &text[..cut]),
        None => text,
    };
    div()
        .flex()
        .flex_col()
        .rounded_sm()
        .bg(rgb(0x252526))
        .p_2()
        .font_family("Menlo")
        .text_xs()
        .child(div().text_color(rgb(0x8a8a8a)).child(label.to_string()))
        .child(text)
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
                                    .when(self.run_without_asking, |d| {
                                        d.child(
                                            button("ask-again")
                                                .bg(rgb(0x3a3a3c))
                                                .child("▶ Runs without asking ✕")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.run_without_asking = false;
                                                    this.note("Will ask before running code again.", cx);
                                                })),
                                        )
                                    })
                                    .child(div().flex_1())
                                    .when(self.runtime.is_none() && !self.starting, |d| {
                                        d.child(
                                            button("restart")
                                                .bg(rgb(0x3a3a3c))
                                                .child("↻ Restart Julia")
                                                .on_click(cx.listener(|this, _, _, cx| this.boot(this.last_ports, cx))),
                                        )
                                    })
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
    // Claude Code runs the plugin's execution-gate hook as `endeavor hook-pretool`.
    if std::env::args().nth(1).as_deref() == Some("hook-pretool") {
        gate::run_pretool_hook();
    }
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
