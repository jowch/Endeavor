//! One chat session: its transcript, message queue, cell-code memory, execution
//! "stop asking" state, and the notebook it was last looking at. Agent events are
//! applied here; anything that needs the workspace (sending to the agent, driving
//! the notebook pane, checking run state) comes back as an [`Effect`].

use std::path::{Path, PathBuf};

use agent_client_protocol::Responder;
use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOption, PermissionOptionKind, PlanEntry, PlanEntryStatus,
    RequestPermissionOutcome, RequestPermissionResponse, SelectedPermissionOutcome, SessionId,
    SessionUpdate, StopReason, ToolCallId, ToolCallStatus, ToolCallUpdate,
};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::text::TextView;

use crate::Workspace;
use crate::agent::{SessionEvent, Turn};
use crate::celldiff::{self, CellCodes};
use crate::gate;
use crate::outbox::{Dispatch, Outbox, Queued};

pub enum Entry {
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

/// Work a session hands back to the workspace.
pub enum Effect {
    Send(Turn),
    /// The agent opened or created this notebook.
    ShowNotebook(String),
    /// The session went idle: check for unrun edits.
    CheckRunState,
}

pub struct Session {
    /// App-local identity, stable before and after the agent assigns `id`.
    pub key: u64,
    pub id: Option<SessionId>,
    pub cwd: PathBuf,
    pub title: String,
    pub entries: Vec<Entry>,
    pub outbox: Outbox,
    /// Each cell's code as last seen in the agent's reads and edits, for diffs.
    pub cell_codes: CellCodes,
    /// "Allow & stop asking": approve this session's runs from now on.
    pub run_without_asking: bool,
    /// The notebook this session was last looking at.
    pub notebook: Option<String>,
    pub scroll: ScrollHandle,
}

pub fn folder_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

impl Session {
    pub fn new(key: u64, cwd: PathBuf) -> Self {
        Self {
            key,
            id: None,
            title: "New session".into(),
            cwd,
            entries: Vec::new(),
            outbox: Outbox::waiting(),
            cell_codes: CellCodes::default(),
            run_without_asking: false,
            notebook: None,
            scroll: ScrollHandle::new(),
        }
    }

    /// Waiting on the user to approve something.
    pub fn needs_approval(&self) -> bool {
        self.entries.iter().any(|e| matches!(e, Entry::Permission { responder: Some(_), .. }))
    }

    pub fn note(&mut self, text: impl Into<SharedString>) {
        self.entries.push(Entry::Note(text.into()));
        self.scroll.scroll_to_bottom();
    }

    /// The agent created this session: send whatever was queued meanwhile.
    pub fn started(&mut self, id: SessionId) -> Vec<Effect> {
        self.id = Some(id);
        let mut effects = Vec::new();
        let next = self.outbox.turn_ended();
        self.dispatch(next, &mut effects);
        effects
    }

    /// Send or queue a message. Before the session exists everything queues.
    pub fn submit(&mut self, message: Queued, now: bool) -> Vec<Effect> {
        if self.title == "New session" {
            self.title = message.label.lines().next().unwrap_or_default().chars().take(60).collect();
        }
        let mut effects = Vec::new();
        let dispatch = self.outbox.submit(message, now && self.id.is_some());
        self.dispatch(dispatch, &mut effects);
        effects
    }

    pub fn interrupt(&self) -> Option<Effect> {
        self.outbox.busy.then_some(Effect::Send(Turn::Cancel)).filter(|_| self.id.is_some())
    }

    fn dispatch(&mut self, dispatch: Option<Dispatch>, effects: &mut Vec<Effect>) {
        let Some(Dispatch { turn, shown }) = dispatch else { return };
        effects.push(Effect::Send(turn));
        if let Some(label) = shown {
            self.entries.push(Entry::User(label.into()));
            self.scroll.scroll_to_bottom();
        }
    }

    pub fn apply(&mut self, event: SessionEvent) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            SessionEvent::TurnEnded(reason) => {
                if reason != StopReason::EndTurn {
                    self.note(format!("Turn ended: {reason:?}"));
                }
                self.turn_ended(&mut effects);
            }
            SessionEvent::TurnFailed(e) => {
                self.note(format!("⚠ Turn failed: {e}"));
                self.turn_ended(&mut effects);
            }
            SessionEvent::Steered => {
                if let Some(label) = self.outbox.steered() {
                    self.entries.push(Entry::User(format!("{label}\n↳ sent into the running turn").into()));
                }
            }
            SessionEvent::Unsent => {
                let next = self.outbox.unsent();
                self.dispatch(next, &mut effects);
            }
            SessionEvent::Permission(request, responder) => {
                let fields = &request.tool_call.fields;
                let title = fields.title.clone().unwrap_or_else(|| "Tool call".into());
                let runs_code = gate::is_pluto(&title);
                if runs_code && self.run_without_asking {
                    if let Some(allow) = option_of_kind(&request.options, PermissionOptionKind::AllowOnce) {
                        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(allow.option_id.clone()));
                        let _ = responder.respond(RequestPermissionResponse::new(outcome));
                        self.note(format!("▶ Ran without asking: {title}"));
                        return effects;
                    }
                }
                let input = fields.raw_input.clone().unwrap_or_default();
                let code = input["code"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| input["cell_id"].as_str().and_then(|id| self.cell_codes.get(id)).map(str::to_owned));
                self.entries.push(Entry::Permission { title, code, options: request.options, responder: Some(responder), runs_code });
            }
            SessionEvent::Update(update) => self.apply_update(update, &mut effects),
        }
        self.scroll.scroll_to_bottom();
        effects
    }

    fn turn_ended(&mut self, effects: &mut Vec<Effect>) {
        let next = self.outbox.turn_ended();
        let idle = next.is_none();
        self.dispatch(next, effects);
        if idle {
            effects.push(Effect::CheckRunState);
        }
    }

    fn apply_update(&mut self, update: SessionUpdate, effects: &mut Vec<Effect>) {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Agent(text)) => text.push_str(&t.text),
                        _ => self.entries.push(Entry::Agent(t.text)),
                    }
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Thought { text, .. }) => text.push_str(&t.text),
                        _ => self.entries.push(Entry::Thought { text: t.text, expanded: false }),
                    }
                }
            }
            // One plan per turn, updated in place.
            SessionUpdate::Plan(plan) => {
                let turn_start = self.entries.iter().rposition(|e| matches!(e, Entry::User(_))).unwrap_or(0);
                match self.entries[turn_start..].iter_mut().find(|e| matches!(e, Entry::Plan(_))) {
                    Some(existing) => *existing = Entry::Plan(plan.entries),
                    None => self.entries.push(Entry::Plan(plan.entries)),
                }
            }
            SessionUpdate::ToolCall(call) => self.entries.push(Entry::Tool {
                id: call.tool_call_id,
                title: call.title,
                status: call.status,
                input: call.raw_input,
                output: call.raw_output,
                diffs: Vec::new(),
                expanded: false,
            }),
            SessionUpdate::ToolCallUpdate(update) => {
                if let Some(opened) = self.on_tool_update(update) {
                    effects.push(Effect::ShowNotebook(opened));
                }
            }
            // ponytail: modes, usage, available commands not shown yet.
            _ => {}
        }
    }

    /// Apply a tool-call update; on a completed pluto call, learn cell code and
    /// diff edits. Returns a notebook id the agent just opened or created.
    fn on_tool_update(&mut self, update: ToolCallUpdate) -> Option<String> {
        let ix = self.entries.iter().rposition(|e| matches!(e, Entry::Tool { id, .. } if *id == update.tool_call_id))?;
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
        matches!(tool, "open_notebook" | "new_notebook").then(|| result["notebook_id"].as_str().map(str::to_owned)).flatten()
    }

    pub fn toggle(&mut self, ix: usize) {
        if let Some(Entry::Tool { expanded, .. } | Entry::Thought { expanded, .. }) = self.entries.get_mut(ix) {
            *expanded = !*expanded;
        }
    }

    /// Answer a permission request; `stop_asking` approves this session's later runs.
    pub fn answer(&mut self, ix: usize, option: &PermissionOption, stop_asking: bool) {
        let Some(Entry::Permission { title, responder, .. }) = self.entries.get_mut(ix) else { return };
        if let Some(responder) = responder.take() {
            let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option.option_id.clone()));
            let _ = responder.respond(RequestPermissionResponse::new(outcome));
            let verb = if stop_asking { "Allowed (won't ask again this session)" } else { option.name.as_str() };
            self.entries[ix] = Entry::Note(format!("{verb}: {title}").into());
            self.run_without_asking |= stop_asking;
        }
    }
}

fn option_of_kind(options: &[PermissionOption], kind: PermissionOptionKind) -> Option<&PermissionOption> {
    options.iter().find(|o| o.kind == kind)
}

// ---------------------------------------------------------------------------
// Rendering. Clicks find their session by `key`.
// ---------------------------------------------------------------------------

pub fn render_transcript(session: &Session, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let entries: Vec<_> = session.entries.iter().enumerate().map(|(i, e)| render_entry(session.key, i, e, cx)).collect();
    div()
        .id(("transcript", session.key as usize))
        .flex_1()
        .overflow_y_scroll()
        .track_scroll(&session.scroll)
        .p_3()
        .flex()
        .flex_col()
        .gap_3()
        .children(entries)
}

fn render_entry(key: u64, ix: usize, entry: &Entry, cx: &mut Context<Workspace>) -> AnyElement {
    let muted = rgb(0x8a8a8a);
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), (key as u64) << 32 | ix as u64);
    match entry {
        Entry::User(text) => div().p_2().rounded_md().bg(rgb(0x2d2d30)).child(text.clone()).into_any_element(),
        Entry::Agent(text) => TextView::markdown(id("agent"), text.clone()).into_any_element(),
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
                        .id(id("tool"))
                        .cursor_pointer()
                        .text_color(muted)
                        .child(format!("{arrow} ⚙ {title} · {status:?}"))
                        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.toggle(ix)))),
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
                    .id(id("thought"))
                    .cursor_pointer()
                    .child(if *expanded { "▾ Thinking" } else { "▸ Thinking" })
                    .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.toggle(ix)))),
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
                        .id(ElementId::NamedInteger("perm".into(), (key << 32) | (ix as u64 * 16 + i as u64)))
                        .px_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(if allow { rgb(0x2f5d3a) } else { rgb(0x5d2f2f) })
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.with_session(key, cx, |s| s.answer(ix, &option, stop))
                        }))
                })))
                .into_any_element()
        }
    }
}

/// Messages waiting for Claude: click ✎ to pull one back into the input, ✕ to drop it.
pub fn render_queue(session: &Session, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let muted = rgb(0x8a8a8a);
    let key = session.key;
    div().flex().flex_col().gap_1().children(session.outbox.items.iter().enumerate().map(|(i, q)| {
        let id = |name: &'static str| ElementId::NamedInteger(name.into(), (key << 32) | i as u64);
        div()
            .flex()
            .gap_2()
            .text_sm()
            .text_color(muted)
            .child(div().flex_1().overflow_hidden().child(q.label.clone()))
            .when(q.in_flight(), |d| d.child("sending now…"))
            .when(!q.in_flight() && q.editable.is_some(), |d| {
                d.child(div().id(id("edit")).cursor_pointer().child("✎").on_click(cx.listener(move |this, _, window, cx| {
                    let text = this.session_mut(key).and_then(|s| s.outbox.take(i)).and_then(|q| q.editable);
                    if let Some(text) = text {
                        this.input.update(cx, |s, cx| s.set_value(text, window, cx));
                        cx.notify();
                    }
                })))
            })
            .when(!q.in_flight(), |d| {
                d.child(div().id(id("drop")).cursor_pointer().child("✕").on_click(cx.listener(move |this, _, _, cx| {
                    this.with_session(key, cx, |s| {
                        s.outbox.take(i);
                    })
                })))
            })
    }))
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
            div().px_2().when_some(bg, |d, bg| d.bg(bg)).child(format!("{sign} {line}"))
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

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]` macro.
    use super::{Effect, Entry, Session, SessionEvent, Turn};
    use crate::outbox::Queued;
    use agent_client_protocol::schema::v1::{SessionId, StopReason};

    fn text(s: &str) -> Queued {
        Queued::new(s.into(), Some(s.into()), vec![])
    }

    #[test]
    fn a_new_session_queues_until_started_then_sends_the_first_message() {
        let mut s = Session::new(1, "/tmp/project".into());
        assert!(s.submit(text("plot sin"), true).is_empty(), "nothing sent before the session exists");
        assert_eq!(s.title, "plot sin");
        let effects = s.started(SessionId::new("abc"));
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]));
        assert!(matches!(s.entries.as_slice(), [Entry::User(_)]));
    }

    #[test]
    fn going_idle_asks_for_a_run_state_check() {
        let mut s = Session::new(1, "/tmp".into());
        s.started(SessionId::new("abc"));
        s.submit(text("hi"), false);
        let effects = s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(matches!(effects.as_slice(), [Effect::CheckRunState]));
    }
}
