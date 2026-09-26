//! One chat session: its transcript, message queue, cell-code memory, execution
//! "stop asking" state, and the notebook it was last looking at. Agent events are
//! applied here; anything that needs the workspace (sending to the agent, driving
//! the notebook pane, checking run state) comes back as an [`Effect`].

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Instant;

use agent_client_protocol::Responder;
use agent_client_protocol::schema::MaybeUndefined;
use agent_client_protocol::schema::v1::{
    AvailableCommand, ContentBlock, PermissionOption, PermissionOptionKind, PlanEntry, PlanEntryStatus,
    RequestPermissionOutcome, RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigSelectOption, SessionConfigSelectOptions,
    SessionConfigValueId, SessionId,
    SessionModeId, SessionModeState, SessionUpdate, StopReason, ToolCallId, ToolCallStatus, ToolCallUpdate,
};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::text::TextView;

use crate::Workspace;
use crate::theme;
use crate::agent::{SessionEvent, Started, Turn};
use crate::celldiff::{self, CellCodes};
use crate::gate;
use crate::pluto;
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
        /// The pluto tool and its input, for the run card.
        tool: Option<String>,
        input: serde_json::Value,
        /// What the run would run, once the runtime answers.
        preview: Option<pluto::RunPreview>,
    },
    Note(SharedString),
}

/// Work a session hands back to the workspace.
pub enum Effect {
    Send(Turn),
    /// The agent opened or created this notebook.
    ShowNotebook(String),
    /// A reopened session last worked in this notebook file: open it in the
    /// current Pluto (its old id died with the previous Julia) and show it.
    ReopenNotebook(String),
    /// The session went idle: check for unrun edits.
    CheckRunState,
    /// Set one of the agent's config options (model, effort).
    SetConfig(String, SessionConfigValueId),
    /// Ask the agent to switch mode.
    SetMode(SessionModeId),
    /// Ask the runtime what the pending run at this entry would run.
    PreviewRun { ix: usize, tool: String, input: serde_json::Value },
    /// Tell the runtime this session's policy changed ("plan" | "ask").
    SetPolicy(&'static str),
}

pub struct Session {
    /// App-local identity, stable before and after the agent assigns `id`.
    pub key: u64,
    pub id: Option<SessionId>,
    pub cwd: PathBuf,
    pub title: String,
    /// The user named it; the agent's titles no longer replace it.
    pub named: bool,
    pub entries: Vec<Entry>,
    pub outbox: Outbox,
    /// Each cell's code as last seen in the agent's reads and edits, for diffs.
    pub cell_codes: CellCodes,
    /// "Allow & stop asking": approve this session's runs from now on.
    pub run_without_asking: bool,
    /// The notebook this session was last looking at.
    pub notebook: Option<String>,
    /// Cells the user changed since the agent last heard (cell id, name); told
    /// with the next prompt.
    pub user_edits: Vec<(String, Option<String>)>,
    /// The transcript as a virtualized list: only visible entries are laid out,
    /// and it follows new content unless the user has scrolled up to read.
    pub list: ListState,
    /// Entries the list knows about, and the lowest index changed since (see `sync_list`).
    list_len: Cell<usize>,
    dirty_from: Cell<Option<usize>>,
    /// When the current turn started, for the "Working · 12s" indicator.
    pub busy_since: Option<Instant>,
    /// Reopening a past session: its history is replaying.
    replaying: bool,
    /// Notebook file the replayed history last opened, reopened once loading ends.
    replayed_path: Option<String>,
    /// Starting or reopening failed; the session can't take messages.
    pub failed: Option<Failure>,
    /// The agent's modes (e.g. plan / default / auto) and which is current.
    pub modes: Option<SessionModeState>,
    /// The agent's session config options (e.g. model, effort).
    pub config: Vec<SessionConfigOption>,
    /// Context used / size, in tokens, from the agent's usage updates.
    pub usage: Option<(u64, u64)>,
    /// Slash commands the agent offers.
    pub commands: Vec<AvailableCommand>,
    /// The policy last sent to the runtime.
    policy_sent: &'static str,
}

pub struct Failure {
    pub message: String,
    /// A reopen that failed can still be continued as a copy (session/fork).
    pub can_copy: bool,
}

/// A readable reason from an agent error, which may wrap stderr in JSON.
fn failure_message(error: &str) -> (String, bool) {
    if error.contains("running as a background session") || error.contains("claude attach") {
        let message = "This session is still open in the Claude Code CLI. Close it there to continue it \
                       here, or open a copy (it keeps the conversation so far)."
            .to_string();
        return (message, true);
    }
    let detail = error
        .split_once("\"details\": \"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(detail, _)| detail)
        .unwrap_or(error);
    (format!("Couldn't open the session: {}", detail.lines().next().unwrap_or(detail).trim()), false)
}

/// A session title from its first message: cut at a word boundary, with "…".
fn short_title(text: &str) -> String {
    const MAX: usize = 50;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", cut.trim_end_matches([',', '.', ':', ';']))
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
            named: false,
            cwd,
            entries: Vec::new(),
            outbox: Outbox::waiting(),
            cell_codes: CellCodes::default(),
            run_without_asking: false,
            notebook: None,
            user_edits: Vec::new(),
            list: {
                let list = ListState::new(0, ListAlignment::Top, px(1000.));
                list.set_follow_mode(FollowMode::Tail);
                list
            },
            list_len: Cell::new(0),
            dirty_from: Cell::new(None),
            busy_since: None,
            replaying: false,
            replayed_path: None,
            failed: None,
            modes: None,
            config: Vec::new(),
            usage: None,
            commands: Vec::new(),
            policy_sent: "ask",
        }
    }

    /// Starting or reopening failed: stop looking busy and say why.
    pub fn fail(&mut self, error: &str) {
        let (message, cli_live) = failure_message(error);
        self.failed = Some(Failure { message, can_copy: cli_live || self.replaying });
        self.outbox.busy = false;
        self.busy_since = None;
        self.replaying = false;
    }

    /// Retry a failed reopen as a copy: the transcript refills from the copy's replay.
    pub fn reopen_as_copy(&mut self) -> Option<SessionId> {
        self.failed.take()?;
        self.entries.clear();
        self.mark(0);
        self.outbox = Outbox::waiting();
        self.replaying = true;
        self.title = format!("{} (copy)", self.title);
        self.id.take()
    }

    /// A past session being reopened: its id is known up front so the replayed
    /// history (which arrives before the load completes) lands here.
    pub fn loading(key: u64, id: SessionId, cwd: PathBuf, title: String) -> Self {
        let mut session = Self::new(key, cwd);
        session.id = Some(id);
        session.title = title;
        session.replaying = true;
        session
    }

    /// The current choice of a select config option, by its display name (e.g.
    /// "model" → "Opus 5.5").
    pub fn config_label(&self, id: &str) -> Option<String> {
        let (current, options) = self.config_choices(id)?;
        options.iter().find(|o| o.value == current).map(|o| o.name.clone())
    }

    /// A select config option's current value and its choices.
    pub fn config_choices(&self, id: &str) -> Option<(SessionConfigValueId, Vec<SessionConfigSelectOption>)> {
        let option = self.config.iter().find(|c| c.id.to_string() == id)?;
        let SessionConfigKind::Select(select) = &option.kind else { return None };
        let options = match &select.options {
            SessionConfigSelectOptions::Ungrouped(options) => options.clone(),
            SessionConfigSelectOptions::Grouped(groups) => groups.iter().flat_map(|g| g.options.clone()).collect(),
            _ => Vec::new(),
        };
        Some((select.current_value.clone(), options))
    }

    /// Pick a config value, showing it at once; the agent's reply confirms it.
    pub fn set_config(&mut self, id: &str, value: SessionConfigValueId) -> Vec<Effect> {
        let Some(option) = self.config.iter_mut().find(|c| c.id.to_string() == id) else { return Vec::new() };
        if let SessionConfigKind::Select(select) = &mut option.kind {
            select.current_value = value.clone();
        }
        vec![Effect::SetConfig(id.to_string(), value)]
    }

    /// The agent's name for the current mode.
    pub fn mode_name(&self) -> Option<&str> {
        let modes = self.modes.as_ref()?;
        modes.available_modes.iter().find(|m| m.id == modes.current_mode_id).map(|m| m.name.as_str())
    }

    /// Switch to the next mode (⇧⇥), showing it at once; the agent confirms with
    /// a mode update.
    pub fn cycle_mode(&mut self) -> Vec<Effect> {
        let Some(modes) = self.modes.as_mut() else { return Vec::new() };
        let at = modes.available_modes.iter().position(|m| m.id == modes.current_mode_id).unwrap_or(0);
        let Some(next) = modes.available_modes.get((at + 1) % modes.available_modes.len().max(1)).map(|m| m.id.clone()) else {
            return Vec::new();
        };
        modes.current_mode_id = next.clone();
        let mut effects = vec![Effect::SetMode(next)];
        self.sync_policy(&mut effects);
        effects
    }

    /// The runtime policy for the current mode: plan mode is read-only there too
    /// (Claude's plan mode only restricts its own tools, not ours).
    pub fn policy(&self) -> &'static str {
        match &self.modes {
            Some(m) if m.current_mode_id.to_string() == "plan" => "plan",
            _ => "ask",
        }
    }

    fn sync_policy(&mut self, effects: &mut Vec<Effect>) {
        let policy = self.policy();
        if policy != self.policy_sent {
            self.policy_sent = policy;
            effects.push(Effect::SetPolicy(policy));
        }
    }

    /// Waiting on the user to approve something.
    pub fn needs_approval(&self) -> bool {
        self.entries.iter().any(|e| matches!(e, Entry::Permission { responder: Some(_), .. }))
    }

    /// The user changed a cell in this session's notebook.
    pub fn note_user_edit(&mut self, cell: String, name: Option<String>) {
        self.note(match &name {
            Some(name) => format!("You edited `{name}`"),
            None => "You edited a cell".into(),
        });
        if !self.user_edits.iter().any(|(id, _)| *id == cell) {
            self.user_edits.push((cell, name));
        }
    }

    pub fn note(&mut self, text: impl Into<SharedString>) {
        self.push(Entry::Note(text.into()));
    }

    fn push(&mut self, entry: Entry) {
        self.mark(self.entries.len());
        self.entries.push(entry);
    }

    /// Entry `ix` (and anything after it) changed; the list re-measures it.
    fn mark(&self, ix: usize) {
        let from = self.dirty_from.get().map_or(ix, |d| d.min(ix));
        self.dirty_from.set(Some(from));
    }

    /// Bring the virtual list up to date with `entries` (called before rendering).
    pub fn sync_list(&self) {
        let Some(from) = self.dirty_from.take() else { return };
        let old = self.list_len.get();
        let from = from.min(old);
        self.list.splice(from..old, self.entries.len() - from);
        self.list_len.set(self.entries.len());
    }

    /// The agent created this session: send whatever was queued meanwhile.
    pub fn started(&mut self, started: Started) -> Vec<Effect> {
        self.id = Some(started.id);
        self.modes = started.modes;
        self.config = started.config;
        self.replaying = false;
        let mut effects: Vec<Effect> = self.replayed_path.take().map(Effect::ReopenNotebook).into_iter().collect();
        self.sync_policy(&mut effects);
        let next = self.outbox.turn_ended();
        self.dispatch(next, &mut effects);
        effects
    }

    /// Send or queue a message. Before the session exists everything queues.
    pub fn submit(&mut self, message: Queued, now: bool) -> Vec<Effect> {
        if self.failed.is_some() {
            self.note("This session isn't open, so nothing was sent.");
            return Vec::new();
        }
        if self.title == "New session" {
            self.title = short_title(message.label.lines().next().unwrap_or_default());
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
            self.push(Entry::User(label.into()));
            self.busy_since.get_or_insert_with(Instant::now);
            // Sending jumps back to the bottom even if the user had scrolled up.
            self.list.set_follow_mode(FollowMode::Tail);
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
                    self.push(Entry::User(format!("{label}\n↳ sent into the running turn").into()));
                }
            }
            SessionEvent::Unsent => {
                let next = self.outbox.unsent();
                self.dispatch(next, &mut effects);
            }
            SessionEvent::Permission(request, responder) => {
                let fields = &request.tool_call.fields;
                let title = fields.title.clone().unwrap_or_else(|| "Tool call".into());
                // Only runs get the run card ("Allow & stop asking"); other pluto
                // prompts (e.g. plan mode asking before a read) get the agent's options.
                let input = fields.raw_input.clone().unwrap_or_default();
                let runs_code = title.strip_prefix("mcp__pluto__").is_some_and(|tool| gate::runs_code(tool, &input));
                if runs_code && self.run_without_asking {
                    if let Some(allow) = option_of_kind(&request.options, PermissionOptionKind::AllowOnce) {
                        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(allow.option_id.clone()));
                        let _ = responder.respond(RequestPermissionResponse::new(outcome));
                        self.note(format!("▶ Ran without asking: {title}"));
                        return effects;
                    }
                }
                let code = input["code"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| input["cell_id"].as_str().and_then(|id| self.cell_codes.get(id)).map(str::to_owned));
                let tool = title.strip_prefix("mcp__pluto__").map(str::to_owned);
                if let Some(tool) = tool.clone().filter(|_| runs_code) {
                    effects.push(Effect::PreviewRun { ix: self.entries.len(), tool, input: input.clone() });
                }
                self.push(Entry::Permission { title, code, options: request.options, responder: Some(responder), runs_code, tool, input, preview: None });
            }
            SessionEvent::Update(update) => self.apply_update(update, &mut effects),
        }
        effects
    }

    fn turn_ended(&mut self, effects: &mut Vec<Effect>) {
        let next = self.outbox.turn_ended();
        let idle = next.is_none();
        if idle {
            self.busy_since = None;
        }
        self.dispatch(next, effects);
        if idle {
            effects.push(Effect::CheckRunState);
        }
    }

    fn apply_update(&mut self, update: SessionUpdate, effects: &mut Vec<Effect>) {
        match update {
            // Only while replaying history: live messages are already in the transcript.
            SessionUpdate::UserMessageChunk(chunk) if self.replaying => {
                let text = match chunk.content {
                    // The app's own context notes aren't the user's words.
                    ContentBlock::Text(t) if t.text.starts_with("[Endeavor]") => return,
                    ContentBlock::Text(t) => t.text,
                    ContentBlock::ResourceLink(link) => format!("✎ {}", link.name),
                    _ => return,
                };
                match self.entries.last_mut() {
                    Some(Entry::User(existing)) => *existing = format!("{existing}\n{text}").into(),
                    _ => self.push(Entry::User(text.into())),
                }
                self.mark(self.entries.len() - 1);
            }
            SessionUpdate::AgentMessageChunk(chunk) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Agent(text)) => text.push_str(&t.text),
                        _ => self.push(Entry::Agent(t.text)),
                    }
                    self.mark(self.entries.len() - 1);
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Thought { text, .. }) => text.push_str(&t.text),
                        _ => self.push(Entry::Thought { text: t.text, expanded: false }),
                    }
                    self.mark(self.entries.len() - 1);
                }
            }
            // One plan per turn, updated in place.
            SessionUpdate::Plan(plan) => {
                let turn_start = self.entries.iter().rposition(|e| matches!(e, Entry::User(_))).unwrap_or(0);
                match self.entries[turn_start..].iter().position(|e| matches!(e, Entry::Plan(_))) {
                    Some(offset) => {
                        self.entries[turn_start + offset] = Entry::Plan(plan.entries);
                        self.mark(turn_start + offset);
                    }
                    None => self.push(Entry::Plan(plan.entries)),
                }
            }
            SessionUpdate::ToolCall(call) => self.push(Entry::Tool {
                id: call.tool_call_id,
                title: call.title,
                status: call.status,
                input: call.raw_input,
                output: call.raw_output,
                diffs: Vec::new(),
                expanded: false,
            }),
            SessionUpdate::ToolCallUpdate(update) => {
                if let Some((id, path)) = self.on_tool_update(update) {
                    if self.replaying {
                        // History, not a live open: that id belongs to an earlier Julia.
                        self.replayed_path = path.or(self.replayed_path.take());
                    } else {
                        effects.push(Effect::ShowNotebook(id));
                    }
                }
            }
            SessionUpdate::CurrentModeUpdate(update) => {
                if let Some(modes) = &mut self.modes {
                    modes.current_mode_id = update.current_mode_id;
                }
                self.sync_policy(effects);
            }
            SessionUpdate::ConfigOptionUpdate(update) => {
                self.config = update.config_options;
                // The agent confirms a mode switch through its "mode" config option.
                if let (Some(modes), Some(mode)) = (&mut self.modes, config_value(&self.config, "mode")) {
                    modes.current_mode_id = mode.to_string().into();
                }
                self.sync_policy(effects);
            }
            SessionUpdate::UsageUpdate(usage) => self.usage = Some((usage.used, usage.size)),
            SessionUpdate::AvailableCommandsUpdate(update) => self.commands = update.available_commands,
            SessionUpdate::SessionInfoUpdate(info) => {
                if let (MaybeUndefined::Value(title), false) = (info.title, self.named) {
                    self.title = title;
                }
            }
            // ponytail: modes, usage, available commands not shown yet.
            _ => {}
        }
    }

    /// Apply a tool-call update; on a completed pluto call, learn cell code and
    /// diff edits. Returns the id (and file path) of a notebook the agent just opened or created.
    fn on_tool_update(&mut self, update: ToolCallUpdate) -> Option<(String, Option<String>)> {
        let ix = self.entries.iter().rposition(|e| matches!(e, Entry::Tool { id, .. } if *id == update.tool_call_id))?;
        self.mark(ix);
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
        if !matches!(tool, "open_notebook" | "new_notebook") {
            return None;
        }
        let id = result["notebook_id"].as_str()?.to_owned();
        Some((id, result["path"].as_str().map(str::to_owned)))
    }

    pub fn toggle(&mut self, ix: usize) {
        if let Some(Entry::Tool { expanded, .. } | Entry::Thought { expanded, .. }) = self.entries.get_mut(ix) {
            *expanded = !*expanded;
            self.mark(ix);
        }
    }

    /// The permission request waiting for an answer, if any.
    pub fn pending_permission(&self) -> Option<usize> {
        self.entries.iter().position(|e| matches!(e, Entry::Permission { responder: Some(_), .. }))
    }

    /// Answer the pending request by kind (keys: ⏎ allow, ⌘⏎ always, Esc deny).
    pub fn answer_pending(&mut self, kind: PermissionOptionKind, stop_asking: bool) -> bool {
        let Some(ix) = self.pending_permission() else { return false };
        let Entry::Permission { options, .. } = &self.entries[ix] else { return false };
        let Some(option) = option_of_kind(options, kind).cloned() else { return false };
        self.answer(ix, &option, stop_asking);
        true
    }

    pub fn set_preview(&mut self, ix: usize, value: pluto::RunPreview) {
        if let Some(Entry::Permission { preview, .. }) = self.entries.get_mut(ix) {
            *preview = Some(value);
        }
    }

    /// Answer a permission request; `stop_asking` approves this session's later runs.
    pub fn answer(&mut self, ix: usize, option: &PermissionOption, stop_asking: bool) {
        let Some(Entry::Permission { title, responder, .. }) = self.entries.get_mut(ix) else { return };
        if let Some(responder) = responder.take() {
            let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option.option_id.clone()));
            let _ = responder.respond(RequestPermissionResponse::new(outcome));
            let allowed = matches!(option.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways);
            let verb = match (allowed, stop_asking) {
                (true, true) => "Allowed, and won't ask again this session",
                (true, false) => "Allowed",
                (false, _) => "Denied",
            };
            let tool = celldiff::pluto_tool(title).unwrap_or(title);
            self.entries[ix] = Entry::Note(format!("{verb}: {tool}").into());
            self.mark(ix);
            self.run_without_asking |= stop_asking;
        }
    }
}

/// The current value of a select config option, by id.
fn config_value<'a>(config: &'a [SessionConfigOption], id: &str) -> Option<&'a SessionConfigValueId> {
    config.iter().find(|c| c.id.to_string() == id).and_then(|c| match &c.kind {
        SessionConfigKind::Select(select) => Some(&select.current_value),
        _ => None,
    })
}

fn option_of_kind(options: &[PermissionOption], kind: PermissionOptionKind) -> Option<&PermissionOption> {
    options.iter().find(|o| o.kind == kind)
}

// ---------------------------------------------------------------------------
// Rendering. Clicks find their session by `key`.
// ---------------------------------------------------------------------------

pub fn render_transcript(session: &Session, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    session.sync_list();
    let key = session.key;
    let workspace = cx.entity().downgrade();
    list(session.list.clone(), move |ix, _window, cx| {
        workspace
            .update(cx, |this, cx| {
                let Some(entry) = this.sessions.iter().find(|s| s.key == key).and_then(|s| s.entries.get(ix)) else {
                    return div().into_any_element();
                };
                div().px_4().pb_4().child(render_entry(key, ix, entry, cx)).into_any_element()
            })
            .unwrap_or_else(|_| div().into_any_element())
    })
    .flex_1()
    .pt_3()
}

/// "Working · 12s" with a rocket crossing a dotted track, or a note that the
/// session is waiting on the user.
pub fn render_activity(session: &Session) -> Option<impl IntoElement + use<>> {
    let since = session.busy_since?;
    let muted = theme::text_muted();
    // The approval card above the composer says it all.
    if session.needs_approval() {
        return None;
    }
    let secs = since.elapsed().as_secs();
    let elapsed = if secs < 60 { format!("{secs}s") } else { format!("{}m {:02}s", secs / 60, secs % 60) };
    // Matches the width of the dotted track below.
    const TRACK: f32 = 78.;
    Some(
        div()
            .px_3()
            .pb_2()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(muted)
            .child(
                div()
                    .relative()
                    .w(px(TRACK + 16.))
                    .h(px(18.))
                    .child(div().absolute().top(px(4.)).text_xs().text_color(theme::text_section()).child("· · · · · · · · · · · · ·"))
                    .child(
                        div()
                            .absolute()
                            .child("🚀")
                            .with_animation(
                                ElementId::NamedInteger("rocket".into(), session.key),
                                Animation::new(std::time::Duration::from_millis(2400)).repeat().with_easing(ease_in_out),
                                |rocket, t| rocket.left(px(t * TRACK)),
                            ),
                    ),
            )
            .child(format!("Working · {elapsed}"))
            .into_any_element(),
    )
}

fn render_entry(key: u64, ix: usize, entry: &Entry, cx: &mut Context<Workspace>) -> AnyElement {
    let muted = theme::text_muted();
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), (key as u64) << 32 | ix as u64);
    match entry {
        Entry::User(text) => div()
            .flex()
            .justify_end()
            .child(div().max_w(px(300.)).px_3().py_2().rounded(px(8.)).bg(theme::bg_raised()).child(text.clone()))
            .into_any_element(),
        Entry::Agent(text) => TextView::markdown(id("agent"), text.clone()).into_any_element(),
        Entry::Note(text) => div().text_sm().text_color(muted).child(text.clone()).into_any_element(),
        Entry::Tool { title, status, input, output, diffs, expanded, .. } => {
            // One line: grey verb, the cells in mono, ± counts, › to expand (the diff).
            let names: Vec<String> = diffs.iter().map(cell_name).collect();
            let (added, removed) = diffs.iter().flat_map(|d| &d.lines).fold((0, 0), |(a, r), (change, _)| match change {
                celldiff::Change::Added => (a + 1, r),
                celldiff::Change::Removed => (a, r + 1),
                celldiff::Change::Same => (a, r),
            });
            let mono = |text: String, color: Rgba| div().font_family("Menlo").text_size(px(12.)).text_color(color).child(text);
            let state = match status {
                ToolCallStatus::Failed => Some(div().text_color(theme::danger()).child("failed")),
                ToolCallStatus::Pending | ToolCallStatus::InProgress => Some(div().child("…")),
                _ => None,
            };
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .id(id("tool"))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(theme::text_faint())
                        .hover(|s| s.text_color(theme::text_secondary()))
                        .child(div().overflow_hidden().whitespace_nowrap().child(tool_verb(title)))
                        .when(!names.is_empty(), |d| d.child(mono(names.join(", "), theme::text_secondary())))
                        .when(added > 0, |d| d.child(mono(format!("+{added}"), theme::diff_add())))
                        .when(removed > 0, |d| d.child(mono(format!("−{removed}"), theme::diff_del())))
                        .children(state)
                        .child(if *expanded { "⌄" } else { "›" })
                        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.toggle(ix)))),
                )
                .when(*expanded, |d| {
                    d.children(diffs.iter().map(render_diff))
                        .when(diffs.is_empty(), |d| {
                            d.children(input.as_ref().map(|v| detail("input", v)))
                                .children(output.as_ref().map(|v| detail("result", &celldiff::tool_json(v).unwrap_or_else(|| v.clone()))))
                        })
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
                    PlanEntryStatus::Completed => ("☑", theme::diff_add()),
                    PlanEntryStatus::InProgress => ("◐", theme::accent_text()),
                    _ => ("☐", theme::text_secondary()),
                };
                div().flex().gap_2().child(div().text_color(color).child(mark)).child(e.content.clone())
            }))
            .into_any_element(),
        // Pending: shown as the approval card above the composer (render_approval).
        Entry::Permission { .. } => div().into_any_element(),
    }
}

/// The pending approval, pinned above the composer: the only heavy element
/// (accent edge, soft ring, filled primary). Runs get "Run N cells?", the cells,
/// and how many dependents re-run; other prompts get the agent's own options.
pub fn render_approval(session: &Session, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ix = session.pending_permission()?;
    let Entry::Permission { title, code, options, runs_code, tool, input, preview, .. } = &session.entries[ix] else { return None };
    let key = session.key;
    let mono = |text: String| div().font_family("Menlo").text_size(px(12.)).child(text);
    let first_line = |code: &str| code.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    let tool = tool.as_deref().unwrap_or("");

    let (heading, body): (String, Vec<AnyElement>) = if !*runs_code {
        (format!("Allow {}?", celldiff::pluto_tool(title).unwrap_or(title)), vec![])
    } else if tool == "add_cell" {
        ("Add a cell and run it?".into(), vec![])
    } else if let Some(p) = preview {
        let names: Vec<String> = p.cells.iter().map(|c| c.name.clone().unwrap_or_else(|| first_line(&c.code))).collect();
        let heading = match (tool, p.all, p.count) {
            ("delete_cell", _, _) => format!("Delete {}?", names.first().map(|n| format!("`{n}`")).unwrap_or("this cell".into())),
            (_, true, n) => format!("Run all {n} cells?"),
            (_, _, 1) => format!("Run {}?", names.first().map(|n| format!("`{n}`")).unwrap_or("1 cell".into())),
            (_, _, n) => format!("Run {n} cells?"),
        };
        let mut body: Vec<AnyElement> = Vec::new();
        if p.count > 1 && !p.all {
            const SHOWN: usize = 5;
            body.extend(names.iter().take(SHOWN).map(|n| mono(n.clone()).text_color(theme::text_secondary()).into_any_element()));
            if names.len() > SHOWN {
                body.push(div().text_color(theme::text_muted()).child(format!("and {} more", names.len() - SHOWN)).into_any_element());
            }
        }
        if p.dependents > 0 {
            let them = if p.count == 1 { "it" } else { "them" };
            let n = p.dependents;
            let cells = if n == 1 { "cell" } else { "cells" };
            body.push(div().text_color(theme::text_muted()).child(format!("Also re-runs {n} {cells} that depend on {them}.")).into_any_element());
        }
        (heading, body)
    } else {
        ("Run code?".into(), vec![])
    };
    // A single cell's code (or the new cell's) is short enough to show.
    let code = code
        .clone()
        .or_else(|| input["code"].as_str().map(str::to_owned))
        .or_else(|| preview.as_ref().and_then(|p| p.cells.first()).map(|c| c.code.clone())).filter(|_| preview.as_ref().is_none_or(|p| p.count <= 1 && !p.all));

    let mut buttons: Vec<(String, &'static str, PermissionOption, bool)> = Vec::new();
    if *runs_code {
        if let Some(deny) = option_of_kind(options, PermissionOptionKind::RejectOnce) {
            buttons.push(("Deny".into(), "esc", deny.clone(), false));
        }
        if let Some(allow) = option_of_kind(options, PermissionOptionKind::AllowOnce) {
            buttons.push(("Always this session".into(), "⌘⏎", allow.clone(), true));
            let run = if tool == "delete_cell" { "Delete" } else { "Run" };
            buttons.push((run.into(), "⏎", allow.clone(), false));
        }
    }
    if buttons.is_empty() {
        buttons = options.iter().map(|o| (o.name.clone(), "", o.clone(), false)).collect();
    }
    let primary = buttons.len() - 1;
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::accent())
            .bg(theme::bg_card())
            .shadow(vec![BoxShadow {
                color: Hsla::from(theme::accent()).opacity(0.25),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(3.),
                inset: false,
            }])
            .text_sm()
            .child(inline_code(&heading).text_color(theme::text_primary()))
            .children(code.map(|code| {
                const LINES: usize = 8;
                let mut shown: Vec<&str> = code.lines().take(LINES).collect();
                if code.lines().count() > LINES {
                    shown.push("…");
                }
                mono(shown.join("\n")).p(px(6.)).rounded(px(4.)).bg(theme::bg_page()).text_color(theme::text_secondary())
            }))
            .children(body)
            // Wrap: the agent's own option labels can be long.
            .child(div().flex().flex_wrap().justify_end().gap_2().children(buttons.into_iter().enumerate().map(|(i, (label, hint, option, stop))| {
                let is_primary = i == primary;
                div()
                    .id(ElementId::NamedInteger("perm".into(), (key << 32) | (ix as u64 * 16 + i as u64)))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(26.))
                    .px(px(10.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .map(|d| if is_primary { d.bg(theme::accent()).text_color(gpui::white()) } else { d.bg(theme::bg_raised()).hover(|s| s.bg(theme::row_active())) })
                    .child(label)
                    .when(!hint.is_empty(), |d| d.child(div().text_size(px(11.)).opacity(0.6).child(hint)))
                    .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.answer(ix, &option, stop))))
            })))
            .into_any_element(),
    )
}


/// Text with `backticked` spans in mono.
fn inline_code(text: &str) -> Div {
    div().flex().flex_wrap().children(text.split('`').enumerate().map(|(i, part)| {
        // Flex drops a part's edge spaces; non-breaking ones survive.
        let d = div().child(part.replace(' ', "\u{a0}"));
        if i % 2 == 1 { d.font_family("Menlo").text_size(px(12.5)) } else { d }
    }))
}

/// Messages waiting for Claude: click ✎ to pull one back into the input, ✕ to drop it.
pub fn render_queue(session: &Session, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let muted = theme::text_muted();
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
        .border_color(theme::border())
        .font_family("Menlo")
        .text_xs()
        .child(div().px_2().text_color(theme::text_muted()).child(diff.label.clone()))
        .children(diff.lines.iter().take(MAX_LINES).map(|(change, line)| {
            let (sign, bg) = match change {
                Change::Added => ("+", Some(theme::diff_add_tint())),
                Change::Removed => ("-", Some(theme::diff_del_tint())),
                Change::Same => (" ", None),
            };
            div().px_2().when_some(bg, |d, bg| d.bg(bg)).child(format!("{sign} {line}"))
        }))
        .when(diff.lines.len() > MAX_LINES, |d| {
            d.child(div().px_2().text_color(theme::text_muted()).child(format!("… {} more lines", diff.lines.len() - MAX_LINES)))
        })
}

/// A tool call as a verb: "Edited", "Ran", …; other tools keep their own title.
fn tool_verb(title: &str) -> String {
    let Some(tool) = celldiff::pluto_tool(title) else { return title.to_string() };
    match tool {
        "read_cell" | "read_notebook_code" => "Read",
        "edit_cell" | "edit_cells" => "Edited",
        "add_cell" => "Added",
        "delete_cell" => "Deleted",
        "move_cell" => "Moved",
        "execute_cell" | "submit_changes" | "run_all_cells" => "Ran",
        "allow_execution" => "Allowed running",
        "open_notebook" => "Opened notebook",
        "new_notebook" => "Created notebook",
        "list_notebooks" => "Listed notebooks",
        "view_cell_output" => "Viewed output",
        "search_code" => "Searched",
        other => other,
    }
    .to_string()
}

/// A cell's name for the transcript: what it defines (`model(S, p) = …` → `model`,
/// `x = …` → `x`), else its label.
fn cell_name(diff: &celldiff::CellDiff) -> String {
    let first = diff.lines.iter().find(|(c, l)| !matches!(c, celldiff::Change::Removed) && !l.trim().is_empty());
    let defined = first.and_then(|(_, line)| {
        let lhs = line.split_once('=').map(|(lhs, _)| lhs).unwrap_or(line.as_str());
        let lhs = lhs.trim().trim_start_matches("function ").trim_start_matches("const ");
        let name: String = lhs.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '!').collect();
        (!name.is_empty() && line.contains('=')).then_some(name)
    });
    defined.unwrap_or_else(|| diff.label.clone())
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
        .bg(theme::bg_card())
        .p_2()
        .font_family("Menlo")
        .text_xs()
        .child(div().text_color(theme::text_muted()).child(label.to_string()))
        .child(text)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]` macro.
    use super::{Effect, Entry, Session, SessionEvent, Started, Turn};
    use crate::outbox::Queued;
    use agent_client_protocol::schema::v1::{
        AvailableCommand, AvailableCommandsUpdate, CurrentModeUpdate, SessionId, SessionMode, SessionModeState, SessionUpdate,
        StopReason, UsageUpdate,
    };

    #[test]
    fn cells_are_named_by_what_they_define() {
        use crate::celldiff::{CellDiff, Change};
        let diff = |lines: &[(Change, &str)]| CellDiff {
            label: "cell 47ce3f7e".into(),
            lines: lines.iter().map(|(c, l)| (match c { Change::Added => Change::Added, Change::Removed => Change::Removed, Change::Same => Change::Same }, l.to_string())).collect(),
        };
        assert_eq!(super::cell_name(&diff(&[(Change::Added, "x = 5 + 5")])), "x");
        assert_eq!(super::cell_name(&diff(&[(Change::Removed, "old = 1"), (Change::Added, "model(S, p) = p[1] * S")])), "model");
        assert_eq!(super::cell_name(&diff(&[(Change::Added, "function fit!(p) = 1")])), "fit!");
        assert_eq!(super::cell_name(&diff(&[(Change::Added, "scatter(data.S, r)")])), "cell 47ce3f7e");
    }

    #[test]
    fn modes_usage_and_commands_follow_the_agent() {
        let mut s = Session::new(1, "/tmp/project".into());
        let modes = SessionModeState::new("default", vec![SessionMode::new("default", "Ask to run"), SessionMode::new("plan", "Plan"), SessionMode::new("auto", "Auto")]);
        s.started(Started::new(SessionId::new("s1"), Some(modes), None));
        assert_eq!(s.mode_name(), Some("Ask to run"));

        // ⇧⇥ moves on at once and asks the agent; it wraps around.
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(m), Effect::SetPolicy("plan")] if m.to_string() == "plan"));
        assert_eq!(s.mode_name(), Some("Plan"));
        // Leaving plan tells the runtime too.
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(_), Effect::SetPolicy("ask")]));
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(m)] if m.to_string() == "default"));

        // The agent's own updates win.
        s.apply(SessionEvent::Update(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("auto"))));
        assert_eq!(s.mode_name(), Some("Auto"));
        s.apply(SessionEvent::Update(SessionUpdate::UsageUpdate(UsageUpdate::new(12_000, 200_000))));
        assert_eq!(s.usage, Some((12_000, 200_000)));
        let commands = vec![AvailableCommand::new("review", "Review the notebook")];
        s.apply(SessionEvent::Update(SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(commands))));
        assert_eq!(s.commands.len(), 1);

        // No modes offered: nothing to cycle.
        let mut plain = Session::new(2, "/tmp/project".into());
        assert!(plain.cycle_mode().is_empty() && plain.mode_name().is_none());
    }

    fn text(s: &str) -> Queued {
        Queued::new(s.into(), Some(s.into()), vec![])
    }

    #[test]
    fn a_new_session_queues_until_started_then_sends_the_first_message() {
        let mut s = Session::new(1, "/tmp/project".into());
        assert!(s.submit(text("plot sin"), true).is_empty(), "nothing sent before the session exists");
        assert_eq!(s.title, "plot sin");
        let effects = s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]));
        assert!(matches!(s.entries.as_slice(), [Entry::User(_)]));
    }

    #[test]
    fn a_reopened_session_shows_replayed_user_messages_but_not_app_context() {
        use agent_client_protocol::schema::v1::{ContentChunk, SessionUpdate, TextContent};
        use agent_client_protocol::schema::v1::ContentBlock;
        let chunk = |s: &str| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(s)))));
        let mut s = Session::loading(1, SessionId::new("abc"), "/tmp".into(), "Old chat".into());
        s.apply(chunk("[Endeavor] The user is viewing Pluto notebook …"));
        s.apply(chunk("plot sin"));
        assert!(matches!(s.entries.as_slice(), [Entry::User(t)] if t.as_ref() == "plot sin"));
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.apply(chunk("live echo"));
        assert_eq!(s.entries.len(), 1, "after loading, user chunks are ignored");
    }

    #[test]
    fn a_failed_reopen_stops_looking_busy_and_explains() {
        let mut s = Session::loading(1, SessionId::new("abc"), "/tmp".into(), "Old chat".into());
        s.fail(r#"Internal error: { "details": "Claude Code process exited with code 1. stderr: Error: Session abc is running as a background session (abc). Run `claude attach abc` to open it" }"#);
        assert!(!s.outbox.busy);
        let failure = s.failed.as_ref().unwrap();
        assert!(failure.can_copy && failure.message.contains("Claude Code CLI"));
        assert_eq!(s.reopen_as_copy(), Some(SessionId::new("abc")));
        assert!(s.failed.is_none() && s.id.is_none() && s.title.ends_with("(copy)"));

        let mut other = Session::new(2, "/tmp".into());
        other.fail(r#"Internal error: { "details": "boom happened
more" }"#);
        assert_eq!(other.failed.unwrap().message, "Couldn't open the session: boom happened");
    }

    #[test]
    fn replayed_notebook_opens_are_reopened_by_path_not_shown_by_stale_id() {
        use agent_client_protocol::schema::v1::{SessionUpdate, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields};
        let mut s = Session::loading(1, SessionId::new("abc"), "/tmp".into(), "Old".into());
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", "mcp__pluto__open_notebook"))));
        let output = serde_json::json!([{ "type": "text", "text": "{\"notebook_id\":\"old-id\",\"path\":\"/tmp/a.jl\"}" }]);
        let done = ToolCallUpdate::new("t1", ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(output));
        let effects = s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(done)));
        assert!(effects.iter().all(|e| !matches!(e, Effect::ShowNotebook(_))), "no stale navigation");
        let effects = s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(matches!(effects.first(), Some(Effect::ReopenNotebook(p)) if p == "/tmp/a.jl"));
    }

    #[test]
    fn titles_are_cut_at_a_word_boundary() {
        assert_eq!(super::short_title("plot sin"), "plot sin");
        let long = "Reply with one word: what is the capital of France? Use no tools.";
        assert_eq!(super::short_title(long), "Reply with one word: what is the capital of…");
    }

    #[test]
    fn the_virtual_list_tracks_entries_through_pushes_edits_and_clears() {
        let mut s = Session::loading(1, SessionId::new("abc"), "/tmp".into(), "Old".into());
        s.note("a");
        s.note("b");
        s.sync_list();
        assert_eq!(s.list.item_count(), 2);
        s.toggle(0); // in-place edit: count unchanged
        s.note("c");
        s.sync_list();
        assert_eq!(s.list.item_count(), 3);
        s.fail("Error: is running as a background session");
        s.reopen_as_copy(); // clears the transcript
        s.sync_list();
        assert_eq!(s.list.item_count(), 0);
    }

    #[test]
    fn busy_time_runs_from_sending_until_idle() {
        let mut s = Session::new(1, "/tmp".into());
        s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(s.busy_since.is_none());
        s.submit(text("hi"), false);
        assert!(s.busy_since.is_some());
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(s.busy_since.is_none());
    }

    #[test]
    fn going_idle_asks_for_a_run_state_check() {
        let mut s = Session::new(1, "/tmp".into());
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("hi"), false);
        let effects = s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(matches!(effects.as_slice(), [Effect::CheckRunState]));
    }
}
