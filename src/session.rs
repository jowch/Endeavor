//! One chat session: its transcript, message queue, cell-code memory, execution
//! "stop asking" state, and the notebook it was last looking at. Agent events are
//! applied here; anything that needs the workspace (sending to the agent, driving
//! the notebook pane, checking run state) comes back as an [`Effect`].

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::Instant;

use agent_client_protocol::Responder;
use agent_client_protocol::schema::MaybeUndefined;
use agent_client_protocol::schema::v1::{
    AvailableCommand, ContentBlock, PermissionOption, PermissionOptionKind, PlanEntry, PlanEntryStatus,
    RequestPermissionOutcome, RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigSelectOption, SessionConfigSelectOptions,
    SessionConfigValueId, SessionId,
    SessionModeId, SessionModeState, SessionUpdate, StopReason, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::text::TextView;
use gpui_component::tooltip::Tooltip;

use crate::Workspace;
use crate::theme;
use crate::agent::{SessionEvent, Started, Turn};
use crate::celldiff::{self, CellCodes};
use crate::gate;
use crate::hosts::Place;
use crate::pluto;
use crate::outbox::{Dispatch, Outbox, Queued};

pub enum Entry {
    User { text: SharedString, expanded: bool },
    Agent(String),
    Tool {
        id: ToolCallId,
        title: String,
        kind: ToolKind,
        /// The first file the call touches, when the agent says.
        path: Option<PathBuf>,
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
        /// Plan mode's end: the plan to approve (markdown).
        plan: Option<String>,
    },
    Note(SharedString),
}

/// Work a session hands back to the workspace.
pub enum Effect {
    Send(Turn),
    /// The agent opened or created this notebook (its id, and its file).
    ShowNotebook { id: String, path: Option<String> },
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
    /// The host it runs on, and its working folder there.
    pub place: Place,
    /// The server's name, for a session on a server.
    pub server: Option<String>,
    /// Its agent session opens once its host's runtime is ready (it needs the bridge).
    pub agent_waiting: bool,
    pub title: String,
    /// The user named it; the agent's titles no longer replace it.
    pub named: bool,
    pub entries: Vec<Entry>,
    pub outbox: Outbox,
    /// Each cell's code as last seen in the agent's reads and edits, for diffs.
    pub cell_codes: CellCodes,
    /// "Always this session": approve this session's runs from now on.
    pub run_without_asking: bool,
    /// The notebook this session was last looking at.
    pub notebook: Option<String>,
    /// The session's one notebook file, once it has one.
    pub notebook_path: Option<String>,
    /// Its notebook was stopped from the notebook's ⋯ menu, or for being idle.
    pub stopped: Option<Stopped>,
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
    /// The pinned plan (above the composer) is folded.
    pub plan_folded: bool,
    /// Reopening a past session: its history is replaying.
    replaying: bool,
    /// Notebook file to open once the agent session is up: the one the replayed
    /// history last opened, or the one picked on the new-session screen.
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stopped {
    /// It was in safe preview, so Start reopens it without running.
    pub safe_preview: bool,
    /// The runtime stopped it after this many hours idle.
    pub idle_hours: Option<u64>,
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
    pub fn new(key: u64, place: Place, server: Option<String>) -> Self {
        Self {
            key,
            id: None,
            place,
            server,
            agent_waiting: true,
            title: "New session".into(),
            named: false,
            entries: Vec::new(),
            outbox: Outbox::waiting(),
            cell_codes: CellCodes::default(),
            run_without_asking: false,
            notebook: None,
            notebook_path: None,
            stopped: None,
            user_edits: Vec::new(),
            list: {
                let list = ListState::new(0, ListAlignment::Top, px(1000.));
                list.set_follow_mode(FollowMode::Tail);
                list
            },
            list_len: Cell::new(0),
            dirty_from: Cell::new(None),
            busy_since: None,
            plan_folded: false,
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
    pub fn loading(key: u64, id: SessionId, place: Place, server: Option<String>, title: String) -> Self {
        let mut session = Self::new(key, place, server);
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
    /// The spec's modes (docs/ui-spec.md, Composer): Plan is the agent's plan mode;
    /// Ask to run and Auto are its auto mode with our run gate asking or not. Any
    /// other agent mode shows under the agent's own name.
    pub fn mode_name(&self) -> Option<String> {
        let modes = self.modes.as_ref()?;
        Some(match modes.current_mode_id.to_string().as_str() {
            "plan" => "Plan".into(),
            "auto" if self.run_without_asking => "Auto".into(),
            "auto" => "Ask to run".into(),
            _ => modes.available_modes.iter().find(|m| m.id == modes.current_mode_id)?.name.clone(),
        })
    }

    /// ⇧⇥: Ask to run → Auto → Plan → Ask to run, showing it at once; the agent
    /// confirms mode switches with a mode update. Agents without plan and auto
    /// modes cycle through their own.
    pub fn cycle_mode(&mut self) -> Vec<Effect> {
        let Some(modes) = self.modes.as_mut() else { return Vec::new() };
        let has = |id: &str| modes.available_modes.iter().any(|m| m.id.to_string() == id);
        let next = if has("plan") && has("auto") {
            let (next, auto) = match (modes.current_mode_id.to_string().as_str(), self.run_without_asking) {
                ("plan", _) => ("auto", false),
                ("auto", false) => ("auto", true),
                _ => ("plan", false),
            };
            self.run_without_asking = auto;
            SessionModeId::from(next.to_string())
        } else {
            let at = modes.available_modes.iter().position(|m| m.id == modes.current_mode_id).unwrap_or(0);
            let Some(next) = modes.available_modes.get((at + 1) % modes.available_modes.len().max(1)) else { return Vec::new() };
            next.id.clone()
        };
        let mut effects = Vec::new();
        if next != modes.current_mode_id {
            modes.current_mode_id = next.clone();
            effects.push(Effect::SetMode(next));
        }
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

    /// Open this notebook file in the notebook pane once the agent session is up
    /// (Julia is running by then).
    pub fn open_on_start(&mut self, path: String) {
        self.replayed_path = Some(path);
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
            self.push(Entry::User { text: label.into(), expanded: false });
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
                    self.push(Entry::User { text: format!("{label}\n↳ sent into the running turn").into(), expanded: false });
                }
            }
            SessionEvent::Unsent => {
                let next = self.outbox.unsent();
                self.dispatch(next, &mut effects);
            }
            SessionEvent::Permission(request, responder) => {
                let fields = &request.tool_call.fields;
                let title = fields.title.clone().unwrap_or_else(|| "Tool call".into());
                // Only runs get the run card ("Always this session"); other pluto
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
                if let Some(tool) = tool.clone().filter(|t| runs_code && t != "run_shell") {
                    effects.push(Effect::PreviewRun { ix: self.entries.len(), tool, input: input.clone() });
                }
                // The adapter's ExitPlanMode prompt: its options carry these ids.
                let plan = request
                    .options
                    .iter()
                    .any(|o| o.option_id.to_string().starts_with("exit-plan-"))
                    .then(|| input["plan"].as_str().unwrap_or("").to_owned());
                self.push(Entry::Permission { title, code, options: request.options, responder: Some(responder), runs_code, tool, input, preview: None, plan });
            }
            SessionEvent::Update(update) => self.apply_update(update, &mut effects),
        }
        effects
    }

    fn turn_ended(&mut self, effects: &mut Vec<Effect>) {
        let next = self.outbox.turn_ended();
        let idle = next.is_none();
        if idle {
            // The pinned plan folds back into the transcript.
            if let Some(ix) = self.pinned_plan() {
                self.mark(ix);
            }
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
                    Some(Entry::User { text: existing, .. }) => *existing = format!("{existing}\n{text}").into(),
                    _ => self.push(Entry::User { text: text.into(), expanded: false }),
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
            SessionUpdate::Plan(plan) => match self.turn_plan() {
                Some(ix) => {
                    self.entries[ix] = Entry::Plan(plan.entries);
                    self.mark(ix);
                }
                None => self.push(Entry::Plan(plan.entries)),
            },
            SessionUpdate::ToolCall(call) => self.push(Entry::Tool {
                id: call.tool_call_id,
                title: call.title,
                kind: call.kind,
                path: call.locations.into_iter().next().map(|l| l.path),
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
                        effects.push(Effect::ShowNotebook { id, path });
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
        let Entry::Tool { title, kind, path, status, input, output, diffs, .. } = &mut self.entries[ix] else { return None };
        let fields = update.fields;
        if let Some(t) = fields.title {
            *title = t;
        }
        if let Some(k) = fields.kind {
            *kind = k;
        }
        if let Some(locations) = fields.locations {
            *path = locations.into_iter().next().map(|l| l.path);
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
        if let Some(Entry::Tool { expanded, .. } | Entry::Thought { expanded, .. } | Entry::User { expanded, .. }) = self.entries.get_mut(ix) {
            *expanded = !*expanded;
            self.mark(ix);
        }
    }

    /// What the agent is doing, for the working line: this turn's latest running tool
    /// call, else thinking or working. The second part (a cell or file name) is code-like.
    fn activity(&self) -> (String, Option<String>) {
        let turn_start = self.entries.iter().rposition(|e| matches!(e, Entry::User { .. })).unwrap_or(0);
        let running = self.entries[turn_start..].iter().rev().find_map(|e| match e {
            Entry::Tool { title, kind, path, status: ToolCallStatus::Pending | ToolCallStatus::InProgress, input, .. } => {
                Some((title, *kind, path, input))
            }
            _ => None,
        });
        let Some((title, kind, path, input)) = running else {
            let verb = if matches!(self.entries.last(), Some(Entry::Thought { .. })) { "Thinking" } else { "Working" };
            return (verb.into(), None);
        };
        let input = input.as_ref().unwrap_or(&serde_json::Value::Null);
        let named = |field: &str| input[field].as_str().and_then(|id| self.cell_codes.get(id)).and_then(defined_name);
        let cells = |n: usize| if n == 1 { "1 cell".to_string() } else { format!("{n} cells") };
        match celldiff::pluto_tool(title) {
            Some(tool) => {
                let (verb, object) = match tool {
                    "read_cell" | "view_cell_output" => ("Reading", named("cell_id")),
                    "edit_cell" => ("Editing", input["code"].as_str().and_then(defined_name).or_else(|| named("cell_id"))),
                    "add_cell" => ("Adding", input["code"].as_str().and_then(defined_name)),
                    "delete_cell" => ("Deleting", named("cell_id")),
                    "move_cell" => ("Moving", named("cell_id")),
                    "execute_cell" => ("Running", named("cell_id")),
                    "edit_cells" => return (format!("Editing {}", cells(input["cells"].as_array().map_or(0, Vec::len))), None),
                    "submit_changes" => match input["cell_ids"].as_array() {
                        Some(ids) => return (format!("Running {}", cells(ids.len())), None),
                        None => return ("Running changed cells".into(), None),
                    },
                    "run_all_cells" => return ("Running all cells".into(), None),
                    "read_notebook_code" => return ("Reading the notebook".into(), None),
                    "search_code" => return ("Searching the notebook".into(), None),
                    "open_notebook" => return ("Opening a notebook".into(), None),
                    "new_notebook" => return ("Creating a notebook".into(), None),
                    _ => return ("Working".into(), None),
                };
                match object {
                    Some(name) => (verb.into(), Some(name)),
                    None => (format!("{verb} a cell"), None),
                }
            }
            None => {
                let file = path.as_deref().and_then(Path::file_name).map(|f| f.to_string_lossy().into_owned());
                let verb = match kind {
                    ToolKind::Read => "Reading",
                    ToolKind::Edit => "Editing",
                    ToolKind::Delete => "Deleting",
                    ToolKind::Move => "Moving",
                    ToolKind::Search => return ("Searching".into(), None),
                    ToolKind::Execute => return ("Running a command".into(), None),
                    ToolKind::Think => return ("Thinking".into(), None),
                    ToolKind::Fetch => return ("Fetching".into(), None),
                    _ => return ("Working".into(), None),
                };
                match file {
                    Some(file) => (verb.into(), Some(file)),
                    None => (format!("{verb} a file"), None),
                }
            }
        }
    }

    /// This turn's plan entry: the agent updates it in place.
    fn turn_plan(&self) -> Option<usize> {
        let turn_start = self.entries.iter().rposition(|e| matches!(e, Entry::User { .. })).unwrap_or(0);
        self.entries[turn_start..].iter().position(|e| matches!(e, Entry::Plan(_))).map(|offset| turn_start + offset)
    }

    /// While a turn runs, its plan is pinned above the composer instead of in the transcript.
    pub fn pinned_plan(&self) -> Option<usize> {
        self.busy_since.and(self.turn_plan())
    }

    /// The permission request waiting for an answer, if any.
    pub fn pending_permission(&self) -> Option<usize> {
        self.entries.iter().position(|e| matches!(e, Entry::Permission { responder: Some(_), .. }))
    }

    /// Answer the pending request by kind (keys: ⏎ allow, ⌘⏎ always, Esc deny).
    /// For a plan: ⏎ starts (asking before runs), ⌘⏎ starts in Auto.
    pub fn answer_pending(&mut self, kind: PermissionOptionKind, stop_asking: bool) -> bool {
        let Some(ix) = self.pending_permission() else { return false };
        let Entry::Permission { options, plan, .. } = &self.entries[ix] else { return false };
        let option = match (plan.is_some(), kind) {
            (true, PermissionOptionKind::AllowOnce) => plan_option(options),
            _ => option_of_kind(options, kind),
        };
        let Some(option) = option.cloned() else { return false };
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

/// The adapter's plan-approval option ids (claude-agent-acp permissions/options),
/// best first. Starting means the agent's auto mode; whether runs ask is ours
/// (Start: Ask to run; Start in Auto: runs without asking).
const PLAN_START: [&str; 3] = ["exit-plan-auto", "exit-plan-accept-edits", "exit-plan-default"];

fn plan_option(options: &[PermissionOption]) -> Option<&PermissionOption> {
    PLAN_START.iter().find_map(|id| options.iter().find(|o| o.option_id.to_string() == *id))
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
    list(session.list.clone(), move |ix, window, cx| {
        workspace
            .update(cx, |this, cx| {
                let Some(session) = this.sessions.iter().find(|s| s.key == key) else { return div().into_any_element() };
                let Some(entry) = session.entries.get(ix).filter(|_| session.pinned_plan() != Some(ix)) else {
                    return div().into_any_element();
                };
                div().px_4().pb_4().child(render_entry(key, ix, entry, window, cx)).into_any_element()
            })
            .unwrap_or_else(|_| div().into_any_element())
    })
    .flex_1()
    .pt_3()
}

/// The working line: an orbit, then what the agent is doing and for how long
/// ("Editing `residuals` · 12s"), or nothing while it waits on the user.
pub fn render_activity(session: &Session) -> Option<impl IntoElement + use<>> {
    let since = session.busy_since?;
    // The approval card above the composer says it all.
    if session.needs_approval() {
        return None;
    }
    let secs = since.elapsed().as_secs();
    let elapsed = if secs < 60 { format!("{secs}s") } else { format!("{}m {:02}s", secs / 60, secs % 60) };
    let (verb, object) = session.activity();
    Some(
        div()
            .px_3()
            .pb_2()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(theme::size_meta())
            .text_color(theme::text_muted())
            .child(
                div()
                    .mr(px(4.))
                    .size(px(ORBIT))
                    .with_animation(
                        ElementId::NamedInteger("orbit".into(), session.key),
                        Animation::new(Duration::from_secs(1)).repeat(),
                        |d, t| d.child(orbit(t)),
                    ),
            )
            .child(verb)
            .children(object.map(|o| div().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_secondary()).child(o)))
            .child(format!("· {elapsed}"))
            .into_any_element(),
    )
}

const ORBIT: f32 = 14.;

/// A dot circling a small sphere on a tilted ring, `t` of the way round a lap.
/// It passes behind the sphere on the ring's far half, so the ring's back half,
/// a dot there, the sphere, the front half and a dot there are drawn in that order.
fn orbit(t: f32) -> impl IntoElement {
    use std::f32::consts::{PI, TAU};
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let (rx, tilt) = (ORBIT * 0.36, -25f32.to_radians());
            let ry = rx * 0.36;
            let on_ring = |a: f32| {
                let (x, y) = (rx * a.cos(), ry * a.sin());
                point(center.x + px(x * tilt.cos() - y * tilt.sin()), center.y + px(x * tilt.sin() + y * tilt.cos()))
            };
            let half = |from: f32, window: &mut Window| {
                const STEPS: usize = 24;
                let mut path = PathBuilder::stroke(px(1.));
                for i in 0..=STEPS {
                    let p = on_ring(from + PI * i as f32 / STEPS as f32);
                    if i == 0 { path.move_to(p) } else { path.line_to(p) }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, theme::text_section());
                }
            };
            let disc = |at: Point<Pixels>, r: f32, color: Rgba, window: &mut Window| {
                window.paint_quad(fill(Bounds::centered_at(at, size(px(2. * r), px(2. * r))), color).corner_radii(px(r)));
            };
            let angle = t * TAU;
            // sin < 0: the far half, above the sphere on screen.
            let behind = angle > PI;
            let dot = |window: &mut Window| disc(on_ring(angle), ORBIT * 1.7 / 14., theme::accent(), window);
            half(PI, window);
            if behind {
                dot(window);
            }
            disc(center, ORBIT * 3.4 / 14., theme::orbit_sphere(), window);
            half(0., window);
            if !behind {
                dot(window);
            }
        },
    )
    .size(px(ORBIT))
}

/// User messages taller than this many wrapped lines fold to their first
/// `FOLD_TO` lines, so the transcript shows mostly the agent's replies.
const FOLD_AFTER: usize = 12;
const FOLD_TO: usize = 10;
const USER_BUBBLE_WIDTH: f32 = 300.;
const USER_BUBBLE_PAD_X: f32 = 12.;

/// How many lines `text` wraps to inside a user bubble. The bubble sets its
/// font itself: list items don't see the ancestors' text style here.
fn bubble_lines(text: &SharedString, window: &Window) -> usize {
    let style = TextStyle { font_family: theme::SANS.into(), ..window.text_style() };
    let wrap = px(USER_BUBBLE_WIDTH - 2. * USER_BUBBLE_PAD_X);
    window
        .text_system()
        .shape_text(text.clone(), theme::size_body(), &[style.to_run(text.len())], Some(wrap), None)
        .map_or(0, |lines| lines.iter().map(|l| l.wrap_boundaries().len() + 1).sum())
}

fn render_entry(key: u64, ix: usize, entry: &Entry, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let muted = theme::text_muted();
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), (key as u64) << 32 | ix as u64);
    match entry {
        Entry::User { text, expanded } => {
            let bubble = div()
                .max_w(px(USER_BUBBLE_WIDTH))
                .px(px(USER_BUBBLE_PAD_X))
                .py_2()
                .rounded(px(8.))
                .bg(theme::bg_raised())
                .font_family(theme::SANS)
                .text_size(theme::size_body())
                .line_height(theme::line_body());
            let line_height = theme::line_body();
            if bubble_lines(text, window) <= FOLD_AFTER {
                return div().flex().justify_end().child(bubble.child(text.clone())).into_any_element();
            }
            let fade = div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(line_height)
                .bg(linear_gradient(180., linear_color_stop(theme::bg_raised().opacity(0.), 0.), linear_color_stop(theme::bg_raised(), 1.)));
            let body = div()
                .relative()
                .child(text.clone())
                .when(!*expanded, |d| d.max_h(line_height * FOLD_TO as f32).overflow_hidden().child(fade));
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .child(bubble.child(body))
                .child(
                    div()
                        .id(id("user-fold"))
                        .cursor_pointer()
                        .text_size(theme::size_meta())
                        .text_color(muted)
                        .hover(|s| s.text_color(theme::text_primary()))
                        .child(if *expanded { "Show less" } else { "Show more" })
                        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.toggle(ix)))),
                )
                .into_any_element()
        }
        Entry::Agent(text) => TextView::markdown(id("agent"), text.clone()).into_any_element(),
        Entry::Note(text) => div().text_size(theme::size_meta()).text_color(muted).child(text.clone()).into_any_element(),
        Entry::Tool { title, kind, path, status, input, output, diffs, expanded, .. } => {
            // One line: grey verb, what it acted on in mono, ± counts, › to expand.
            let args = input.as_ref().unwrap_or(&serde_json::Value::Null);
            let pluto = celldiff::pluto_tool(title).is_some();
            let file_diff = if pluto { None } else { file_diff(*kind, path.as_deref(), args) };
            let all_diffs: Vec<&celldiff::CellDiff> = diffs.iter().chain(&file_diff).collect();
            let (added, removed) = all_diffs.iter().flat_map(|d| &d.lines).fold((0, 0), |(a, r), (change, _)| match change {
                celldiff::Change::Added => (a + 1, r),
                celldiff::Change::Removed => (a, r + 1),
                celldiff::Change::Same => (a, r),
            });
            let line = if pluto {
                let names: Vec<String> = diffs.iter().map(cell_name).collect();
                ToolLine { verb: tool_verb(title), object: (!names.is_empty()).then(|| names.join(", ")), mono: true, full: None }
            } else {
                tool_line(title, *kind, path.as_deref(), args)
            };
            let mono = |text: String, color: Rgba| div().flex_none().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(color).child(text);
            let state = match status {
                ToolCallStatus::Failed => Some(div().flex_none().text_color(theme::danger()).child("failed")),
                ToolCallStatus::Pending | ToolCallStatus::InProgress => Some(div().flex_none().child("…")),
                _ => None,
            };
            let object = line.object.map(|text| {
                let d = div().id(id("tool-object")).min_w_0().truncate().text_color(theme::text_secondary()).child(text);
                let d = if line.mono { d.font_family(theme::MONO).text_size(theme::size_meta_small()) } else { d };
                match line.full {
                    Some(full) => d.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)),
                    None => d,
                }
            });
            let sections = if pluto {
                let json = |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_default();
                input
                    .iter()
                    .map(|v| ("input", json(v)))
                    .chain(output.iter().map(|v| ("result", json(&celldiff::tool_json(v).unwrap_or_else(|| v.clone())))))
                    .collect()
            } else {
                tool_details(*kind, path.as_deref(), input.as_ref(), output.as_ref(), file_diff.is_some())
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
                        .text_size(theme::size_meta())
                        .text_color(theme::text_faint())
                        .hover(|s| s.text_color(theme::text_secondary()))
                        .child(div().flex_none().whitespace_nowrap().child(line.verb))
                        .children(object)
                        .when(added > 0, |d| d.child(mono(format!("+{added}"), theme::diff_add())))
                        .when(removed > 0, |d| d.child(mono(format!("−{removed}"), theme::diff_del())))
                        .children(state)
                        .child(div().flex_none().child(if *expanded { "⌄" } else { "›" }))
                        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.toggle(ix)))),
                )
                .when(*expanded, |d| {
                    d.children(all_diffs.into_iter().map(render_diff))
                        .when(diffs.is_empty() && !sections.is_empty(), |d| d.child(detail(id("tool-detail"), sections)))
                })
                .into_any_element()
        }
        Entry::Thought { text, expanded } => div()
            .flex()
            .flex_col()
            .gap_1()
            .text_size(theme::size_meta())
            .text_color(muted)
            .child(
                div()
                    .id(id("thought"))
                    .cursor_pointer()
                    .child(if *expanded { "▾ Thinking" } else { "▸ Thinking" })
                    .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.toggle(ix)))),
            )
            .when(*expanded, |d| d.child(div().italic().text_size(theme::size_body()).child(text.clone())))
            .into_any_element(),
        Entry::Plan(entries) => div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_size(theme::size_meta()).text_color(muted).child(progress(entries)))
            .children(plan_rows(entries))
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
    let Entry::Permission { title, code, options, runs_code, tool, input, preview, plan, .. } = &session.entries[ix] else { return None };
    let key = session.key;
    if let Some(plan) = plan {
        return Some(render_plan_card(key, ix, plan, options, cx));
    }
    let mono = |text: String| div().font_family(theme::MONO).text_size(theme::size_code()).child(text);
    // An unnamed cell (e.g. markdown) by its first line, cut short.
    let first_line = |code: &str| {
        let line = code.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        match line.char_indices().nth(60) {
            Some((cut, _)) => format!("{}…", &line[..cut]),
            None => line.to_string(),
        }
    };
    let tool = tool.as_deref().unwrap_or("");

    let (heading, body): (String, Vec<AnyElement>) = if !*runs_code {
        (format!("Allow {}?", celldiff::pluto_tool(title).unwrap_or(title)), vec![])
    } else if tool == "run_shell" {
        let host = session.server.clone().unwrap_or_else(|| "the server".into());
        let cwd = input["cwd"].as_str().filter(|c| !c.is_empty()).unwrap_or("~");
        (format!("Run a command on {host}?"), vec![div().text_color(theme::text_muted()).child(format!("In {cwd}")).into_any_element()])
    } else if tool == "add_cell" {
        ("Add a cell and run it?".into(), vec![])
    } else if let Some(p) = preview {
        let names: Vec<String> = p.cells.iter().map(|c| c.name.clone().unwrap_or_else(|| first_line(&c.code))).collect();
        let heading = match (tool, p.all, p.count) {
            ("delete_cell", _, _) => format!("Delete {}?", names.first().map(|n| format!("`{n}`")).unwrap_or("this cell".into())),
            ("allow_execution", false, _) => "Let this notebook run? (Nothing runs yet.)".into(),
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
        .or_else(|| input["command"].as_str().map(str::to_owned))
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
    let code = code.map(|code| {
        const LINES: usize = 8;
        let mut shown: Vec<&str> = code.lines().take(LINES).collect();
        if code.lines().count() > LINES {
            shown.push("…");
        }
        mono(shown.join("\n")).p(px(6.)).rounded(px(4.)).bg(theme::bg_page()).text_color(theme::text_secondary()).into_any_element()
    });
    // Wrap: the agent's own option labels can be long.
    let buttons = buttons
        .into_iter()
        .enumerate()
        .map(|(i, (label, hint, option, stop))| {
            approval_button(ElementId::NamedInteger("perm".into(), (key << 32) | (ix as u64 * 16 + i as u64)), &label, hint, i == primary)
                .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.answer(ix, &option, stop))))
                .into_any_element()
        })
        .collect();
    Some(approval_card(
        inline_code(&heading).text_size(theme::size_subhead()).font_weight(FontWeight::MEDIUM).text_color(theme::text_primary()).into_any_element(),
        code.into_iter().chain(body).collect(),
        buttons,
    ))
}

/// Plan mode's end: the plan, then Keep planning · Start in Auto · **Start**.
fn render_plan_card(key: u64, ix: usize, plan: &str, options: &[PermissionOption], cx: &mut Context<Workspace>) -> AnyElement {
    // (label, key, option, runs without asking, primary)
    let buttons: Vec<(&str, &str, Option<&PermissionOption>, bool, bool)> = vec![
        ("Keep planning", "esc", option_of_kind(options, PermissionOptionKind::RejectOnce), false, false),
        ("Start in Auto", "⌘⏎", plan_option(options), true, false),
        ("Start", "⏎", plan_option(options), false, true),
    ];
    approval_card(
        div().text_size(theme::size_subhead()).font_weight(FontWeight::MEDIUM).text_color(theme::text_primary()).child("Ready to start?").into_any_element(),
        vec![
            div()
                .id(ElementId::NamedInteger("plan".into(), key))
                .max_h(px(260.))
                .overflow_y_scroll()
                .child(TextView::markdown(ElementId::NamedInteger("plan-text".into(), key), plan.to_string()))
                .into_any_element(),
        ],
        buttons
            .into_iter()
            .filter_map(|(label, hint, option, auto, primary)| {
                let option = option?.clone();
                Some(
                    approval_button(ElementId::NamedInteger(label.into(), key), label, hint, primary)
                        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.answer(ix, &option, auto))))
                        .into_any_element(),
                )
            })
            .collect(),
    )
}

/// The approval card's frame: accent edge, soft ring, buttons right-aligned.
fn approval_card(heading: AnyElement, body: Vec<AnyElement>, buttons: Vec<AnyElement>) -> AnyElement {
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
        .child(heading)
        .children(body)
        .child(div().flex().flex_wrap().justify_end().gap_2().children(buttons))
        .into_any_element()
}

fn approval_button(id: ElementId, label: &str, hint: &str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .px(px(10.))
        .rounded(px(5.))
        .cursor_pointer()
        .map(|d| if primary { d.bg(theme::accent()).text_color(gpui::white()) } else { d.bg(theme::bg_raised()).hover(|s| s.bg(theme::row_active())) })
        .child(label.to_string())
        .when(!hint.is_empty(), |d| d.child(div().text_size(theme::size_meta_small()).opacity(0.6).child(hint.to_string())))
}

/// A card title with `backticked` spans in mono, a size smaller (as in body text).
fn inline_code(text: &str) -> Div {
    div().flex().flex_wrap().children(text.split('`').enumerate().map(|(i, part)| {
        // Flex drops a part's edge spaces; non-breaking ones survive.
        let d = div().child(part.replace(' ', "\u{a0}"));
        if i % 2 == 1 { d.font_family(theme::MONO).font_weight(FontWeight::NORMAL).text_size(theme::size_body()) } else { d }
    }))
}

/// "Progress · 1 of 3": steps done of all.
fn progress(entries: &[PlanEntry]) -> String {
    let done = entries.iter().filter(|e| e.status == PlanEntryStatus::Completed).count();
    format!("Progress · {done} of {}", entries.len())
}

/// The plan checklist: ✓ done (struck through), ◐ current (bright), ○ upcoming (grey).
fn plan_rows(entries: &[PlanEntry]) -> impl Iterator<Item = Div> + '_ {
    entries.iter().map(|e| {
        let (mark, mark_color, text_color) = match e.status {
            PlanEntryStatus::Completed => ("✓", theme::diff_add(), theme::text_muted()),
            PlanEntryStatus::InProgress => ("◐", theme::accent_text(), theme::text_primary()),
            _ => ("○", theme::text_faint(), theme::text_secondary()),
        };
        div()
            .flex()
            .gap_2()
            .child(div().w(px(12.)).flex_shrink_0().text_color(mark_color).child(mark))
            .child(div().text_color(text_color).when(e.status == PlanEntryStatus::Completed, |d| d.line_through()).child(e.content.clone()))
    })
}

/// The running turn's plan, pinned above the composer; click the header to fold it.
pub fn render_pinned_plan(session: &Session, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let Entry::Plan(entries) = &session.entries[session.pinned_plan()?] else { return None };
    let (key, folded) = (session.key, session.plan_folded);
    Some(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px(px(10.))
            .py(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::composer_edge())
            .child(
                div()
                    .id("pinned-plan")
                    .flex()
                    .justify_between()
                    .cursor_pointer()
                    .text_size(theme::size_meta())
                    .text_color(theme::text_muted())
                    .child(progress(entries))
                    .child(if folded { "›" } else { "⌄" })
                    .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.plan_folded = !s.plan_folded))),
            )
            .when(!folded, |d| d.children(plan_rows(entries)))
            .into_any_element(),
    )
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
        .font_family(theme::MONO)
        .text_size(theme::size_code())
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
    first.and_then(|(_, line)| defined_name(line)).unwrap_or_else(|| diff.label.clone())
}

/// What code defines, from its first line: `model(S, p) = …` → `model`, `x = …` → `x`.
fn defined_name(code: &str) -> Option<String> {
    let line = code.lines().find(|l| !l.trim().is_empty())?;
    let lhs = line.split_once('=').map(|(lhs, _)| lhs).unwrap_or(line);
    let lhs = lhs.trim().trim_start_matches("function ").trim_start_matches("const ");
    let name: String = lhs.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '!').collect();
    (!name.is_empty() && line.contains('=')).then_some(name)
}

/// A tool call's collapsed line.
struct ToolLine {
    verb: String,
    /// What it acted on: a file name, pattern or command line (mono), or the
    /// agent's own description of a command (not mono). One line.
    object: Option<String>,
    mono: bool,
    /// The full path, shown on hover.
    full: Option<String>,
}

/// A non-notebook call's line: "Ran" + the agent's description or the command's
/// first line; "Read" + a file name; "Searched" + the pattern; "Fetched" + the
/// host; else the agent's title.
fn tool_line(title: &str, kind: ToolKind, path: Option<&Path>, input: &serde_json::Value) -> ToolLine {
    let field = |name: &str| input[name].as_str().map(str::trim).filter(|s| !s.is_empty());
    let line = |verb: &str, object: Option<&str>, mono: bool| ToolLine { verb: verb.into(), object: object.map(first_line), mono, full: None };
    match kind {
        ToolKind::Execute => match field("description") {
            Some(description) => {
                // "List files" → "Ran list files"; acronyms ("JSON …") keep their case.
                let mut chars = description.chars();
                let lower = match (chars.next(), chars.next()) {
                    (Some(first), Some(second)) if !second.is_uppercase() => format!("{}{}", first.to_lowercase(), &description[first.len_utf8()..]),
                    _ => description.to_string(),
                };
                line("Ran", Some(&lower), false)
            }
            None => line("Ran", field("command"), true),
        },
        ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move => {
            let verb = match kind {
                ToolKind::Read => "Read",
                ToolKind::Delete => "Deleted",
                ToolKind::Move => "Moved",
                _ if title.starts_with("Write") => "Wrote",
                _ => "Edited",
            };
            match file_path(path, input) {
                Some(full) => ToolLine { verb: verb.into(), object: Some(file_name(&full)), mono: true, full: Some(full) },
                None => line(&first_line(title), None, false),
            }
        }
        ToolKind::Search => line("Searched", field("pattern").or(field("query")).or(Some(title)), true),
        ToolKind::Fetch => match field("url") {
            Some(url) => line("Fetched", Some(url_host(url)), true),
            None => line("Searched", field("query").or(Some(title)), true),
        },
        _ => line(&first_line(title), None, false),
    }
}

/// The file a call touches: its first location, else a path in its input.
fn file_path(path: Option<&Path>, input: &serde_json::Value) -> Option<String> {
    path.map(|p| p.display().to_string())
        .or_else(|| ["file_path", "notebook_path", "path"].iter().find_map(|f| input[*f].as_str()).map(str::to_owned))
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |f| f.to_string_lossy().into_owned())
}

fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// The first non-blank line, with "…" when more follow.
fn first_line(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or("").to_string();
    if lines.next().is_some() { format!("{first} …") } else { first }
}

/// A file edit's old and new text as a diff.
fn file_diff(kind: ToolKind, path: Option<&Path>, input: &serde_json::Value) -> Option<celldiff::CellDiff> {
    let (old, new) = (input["old_string"].as_str(), input["new_string"].as_str());
    if kind != ToolKind::Edit || (old.is_none() && new.is_none()) {
        return None;
    }
    let label = file_path(path, input).map_or_else(|| "edit".into(), |p| file_name(&p));
    Some(celldiff::CellDiff { label, lines: celldiff::line_diff(old.unwrap_or(""), new.unwrap_or("")) })
}

/// What an expanded non-notebook call shows: its input (the command, the path, or
/// its fields) and its output as text. JSON only when the input isn't flat.
fn tool_details(
    kind: ToolKind,
    path: Option<&Path>,
    input: Option<&serde_json::Value>,
    output: Option<&serde_json::Value>,
    has_diff: bool,
) -> Vec<(&'static str, String)> {
    let args = input.unwrap_or(&serde_json::Value::Null);
    let input = match (kind, args["command"].as_str()) {
        (ToolKind::Execute, Some(command)) => Some(("command", command.to_string())),
        _ if has_diff => file_path(path, args).map(|p| ("path", p)),
        _ => match args.as_object() {
            Some(fields) if fields.values().all(|v| !v.is_object() && !v.is_array()) => {
                let lines: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| {
                        let v = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                        if v.contains('\n') { format!("{k}:\n{v}") } else { format!("{k}: {v}") }
                    })
                    .collect();
                (!lines.is_empty()).then(|| ("input", lines.join("\n")))
            }
            Some(_) => Some(("input", serde_json::to_string_pretty(args).unwrap_or_default())),
            None => file_path(path, args).map(|p| ("path", p)),
        },
    };
    let output = output.map(plain_text).filter(|t| !t.trim().is_empty()).map(|t| ("output", t));
    input.into_iter().chain(output).collect()
}

/// A tool result as text: the text itself, the MCP content array's texts, else JSON.
fn plain_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) if items.iter().all(|i| i["text"].is_string()) => {
            items.iter().filter_map(|i| i["text"].as_str()).collect::<Vec<_>>().join("\n")
        }
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// An expanded tool call's input and output, labelled, in a panel that scrolls.
fn detail(id: ElementId, sections: Vec<(&'static str, String)>) -> impl IntoElement + use<> {
    const MAX_CHARS: usize = 20_000;
    div()
        .id(id)
        .max_h(px(240.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_2()
        .rounded_sm()
        .bg(theme::bg_card())
        .p_2()
        .font_family(theme::MONO)
        .text_size(theme::size_meta())
        .children(sections.into_iter().map(|(label, text)| {
            let text = match text.char_indices().nth(MAX_CHARS) {
                Some((cut, _)) => format!("{}\n…", &text[..cut]),
                None => text,
            };
            div()
                .flex()
                .flex_col()
                .child(div().text_color(theme::text_muted()).child(label))
                .child(div().text_color(theme::text_secondary()).child(text))
        }))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]` macro.
    use super::{Effect, Entry, Session, SessionEvent, Started, Turn};
    use crate::hosts::Place;
    use crate::outbox::Queued;
    use agent_client_protocol::schema::v1::{
        AvailableCommand, AvailableCommandsUpdate, CurrentModeUpdate, SessionId, SessionMode, SessionModeState, SessionUpdate,
        StopReason, UsageUpdate,
    };

    #[test]
    fn tool_calls_collapse_to_one_line() {
        use agent_client_protocol::schema::v1::ToolKind;
        use serde_json::json;
        let line = |title: &str, kind, path: Option<&str>, input| {
            let l = super::tool_line(title, kind, path.map(std::path::Path::new), &input);
            (l.verb, l.object, l.mono, l.full)
        };
        let script = "cd /tmp\nfor f in *.jl; do\n  julia $f\ndone";
        assert_eq!(line(script, ToolKind::Execute, None, json!({"command": script})), ("Ran".into(), Some("cd /tmp …".into()), true, None));
        assert_eq!(
            line("ls", ToolKind::Execute, None, json!({"command": "ls", "description": "List files\nin the folder"})),
            ("Ran".into(), Some("list files …".into()), false, None)
        );
        assert_eq!(
            line("Read src/main.rs (1 - 50)", ToolKind::Read, Some("/repo/src/main.rs"), json!({"file_path": "/repo/src/main.rs"})),
            ("Read".into(), Some("main.rs".into()), true, Some("/repo/src/main.rs".into()))
        );
        assert_eq!(line("Write a.txt", ToolKind::Edit, None, json!({"file_path": "/x/a.txt"})).0, "Wrote");
        assert_eq!(line("grep \"fn main\"", ToolKind::Search, None, json!({"pattern": "fn main"})).1, Some("fn main".into()));
        assert_eq!(line("Fetch", ToolKind::Fetch, None, json!({"url": "https://docs.rs/gpui/latest"})).1, Some("docs.rs".into()));
        assert_eq!(line("Update TODOs: a, b", ToolKind::Think, None, json!({})).0, "Update TODOs: a, b");
    }

    #[test]
    fn expanded_tool_calls_show_text_not_json() {
        use agent_client_protocol::schema::v1::ToolKind;
        use serde_json::json;
        let input = json!({"command": "echo hi\nls", "description": "Say hi"});
        let output = json!([{"type": "text", "text": "hi\nREADME.md"}]);
        assert_eq!(
            super::tool_details(ToolKind::Execute, None, Some(&input), Some(&output), false),
            vec![("command", "echo hi\nls".to_string()), ("output", "hi\nREADME.md".to_string())]
        );
        let read = json!({"file_path": "/repo/README.md", "limit": 20});
        assert_eq!(super::tool_details(ToolKind::Read, None, Some(&read), None, false), vec![("input", "file_path: /repo/README.md\nlimit: 20".to_string())]);
        let edit = json!({"file_path": "/repo/a.rs", "old_string": "a\nb", "new_string": "a\nc"});
        let diff = super::file_diff(ToolKind::Edit, None, &edit).unwrap();
        assert_eq!(diff.label, "a.rs");
        assert_eq!(super::tool_details(ToolKind::Edit, None, Some(&edit), None, true), vec![("path", "/repo/a.rs".to_string())]);
    }

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
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        let modes = SessionModeState::new("default", vec![SessionMode::new("default", "Manual"), SessionMode::new("plan", "Plan"), SessionMode::new("auto", "Auto")]);
        s.started(Started::new(SessionId::new("s1"), Some(modes), None));
        assert_eq!(s.mode_name().as_deref(), Some("Manual"), "another agent mode: its own name");

        // ⇧⇥ moves on at once and asks the agent: Plan → Ask to run → Auto → Plan.
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(m), Effect::SetPolicy("plan")] if m.to_string() == "plan"));
        assert_eq!(s.mode_name().as_deref(), Some("Plan"));
        // Leaving plan tells the runtime too.
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(m), Effect::SetPolicy("ask")] if m.to_string() == "auto"));
        assert_eq!(s.mode_name().as_deref(), Some("Ask to run"));
        // Auto is the same agent mode, with runs no longer asking.
        assert!(s.cycle_mode().is_empty() && s.run_without_asking);
        assert_eq!(s.mode_name().as_deref(), Some("Auto"));
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(m), Effect::SetPolicy("plan")] if m.to_string() == "plan"));
        assert!(!s.run_without_asking);

        // The agent's own updates win.
        s.apply(SessionEvent::Update(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("auto"))));
        assert_eq!(s.mode_name().as_deref(), Some("Ask to run"));
        s.apply(SessionEvent::Update(SessionUpdate::UsageUpdate(UsageUpdate::new(12_000, 200_000))));
        assert_eq!(s.usage, Some((12_000, 200_000)));
        let commands = vec![AvailableCommand::new("review", "Review the notebook")];
        s.apply(SessionEvent::Update(SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(commands))));
        assert_eq!(s.commands.len(), 1);

        // No modes offered: nothing to cycle.
        let mut plain = Session::new(2, Place::local("/tmp/project"), None);
        assert!(plain.cycle_mode().is_empty() && plain.mode_name().is_none());
    }

    fn text(s: &str) -> Queued {
        Queued::new(s.into(), Some(s.into()), vec![])
    }

    #[test]
    fn a_new_session_queues_until_started_then_sends_the_first_message() {
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        assert!(s.submit(text("plot sin"), true).is_empty(), "nothing sent before the session exists");
        assert_eq!(s.title, "plot sin");
        let effects = s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]));
        assert!(matches!(s.entries.as_slice(), [Entry::User { .. }]));
    }

    #[test]
    fn a_user_message_unfolds_and_folds_again() {
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        s.submit(text("plot sin"), true);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.toggle(0);
        assert!(matches!(s.entries.as_slice(), [Entry::User { expanded: true, .. }]));
        s.toggle(0);
        assert!(matches!(s.entries.as_slice(), [Entry::User { expanded: false, .. }]));
    }

    #[test]
    fn a_reopened_session_shows_replayed_user_messages_but_not_app_context() {
        use agent_client_protocol::schema::v1::{ContentChunk, SessionUpdate, TextContent};
        use agent_client_protocol::schema::v1::ContentBlock;
        let chunk = |s: &str| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(s)))));
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old chat".into());
        s.apply(chunk("[Endeavor] The user is viewing Pluto notebook …"));
        s.apply(chunk("plot sin"));
        assert!(matches!(s.entries.as_slice(), [Entry::User { text, .. }] if text.as_ref() == "plot sin"));
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.apply(chunk("live echo"));
        assert_eq!(s.entries.len(), 1, "after loading, user chunks are ignored");
    }

    #[test]
    fn a_failed_reopen_stops_looking_busy_and_explains() {
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old chat".into());
        s.fail(r#"Internal error: { "details": "Claude Code process exited with code 1. stderr: Error: Session abc is running as a background session (abc). Run `claude attach abc` to open it" }"#);
        assert!(!s.outbox.busy);
        let failure = s.failed.as_ref().unwrap();
        assert!(failure.can_copy && failure.message.contains("Claude Code CLI"));
        assert_eq!(s.reopen_as_copy(), Some(SessionId::new("abc")));
        assert!(s.failed.is_none() && s.id.is_none() && s.title.ends_with("(copy)"));

        let mut other = Session::new(2, Place::local("/tmp"), None);
        other.fail(r#"Internal error: { "details": "boom happened
more" }"#);
        assert_eq!(other.failed.unwrap().message, "Couldn't open the session: boom happened");
    }

    #[test]
    fn replayed_notebook_opens_are_reopened_by_path_not_shown_by_stale_id() {
        use agent_client_protocol::schema::v1::{SessionUpdate, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields};
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old".into());
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", "mcp__pluto__open_notebook"))));
        let output = serde_json::json!([{ "type": "text", "text": "{\"notebook_id\":\"old-id\",\"path\":\"/tmp/a.jl\"}" }]);
        let done = ToolCallUpdate::new("t1", ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(output));
        let effects = s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(done)));
        assert!(effects.iter().all(|e| !matches!(e, Effect::ShowNotebook { .. })), "no stale navigation");
        let effects = s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(matches!(effects.first(), Some(Effect::ReopenNotebook(p)) if p == "/tmp/a.jl"));
    }

    #[test]
    fn a_live_notebook_creation_is_shown_with_its_file() {
        use agent_client_protocol::schema::v1::{SessionUpdate, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields};
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        let tool_update = |text: &str| {
            let output = serde_json::json!([{ "type": "text", "text": text }]);
            SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("t1", ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(output))))
        };
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", "mcp__pluto__new_notebook"))));
        let effects = s.apply(tool_update("{\"notebook_id\":\"n1\",\"path\":\"/tmp/fit.jl\",\"created\":true}"));
        assert!(matches!(effects.as_slice(), [Effect::ShowNotebook { id, path: Some(p) }] if id == "n1" && p == "/tmp/fit.jl"));

        // A refused open (one notebook per session) shows nothing.
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", "mcp__pluto__open_notebook"))));
        let effects = s.apply(tool_update("{\"error\":\"one_notebook\",\"message\":\"This session works on one notebook\"}"));
        assert!(effects.is_empty());
    }

    #[test]
    fn titles_are_cut_at_a_word_boundary() {
        assert_eq!(super::short_title("plot sin"), "plot sin");
        let long = "Reply with one word: what is the capital of France? Use no tools.";
        assert_eq!(super::short_title(long), "Reply with one word: what is the capital of…");
    }

    #[test]
    fn the_virtual_list_tracks_entries_through_pushes_edits_and_clears() {
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old".into());
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
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(s.busy_since.is_none());
        s.submit(text("hi"), false);
        assert!(s.busy_since.is_some());
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(s.busy_since.is_none());
    }

    #[test]
    fn the_plan_is_pinned_while_the_turn_runs() {
        use agent_client_protocol::schema::v1::{Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus};
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("hi"), false);
        let plan = |status| SessionUpdate::Plan(Plan::new(vec![PlanEntry::new("step", PlanEntryPriority::Medium, status)]));
        s.apply(SessionEvent::Update(plan(PlanEntryStatus::InProgress)));
        let pinned = s.pinned_plan().expect("pinned while running");
        s.apply(SessionEvent::Update(plan(PlanEntryStatus::Completed)));
        assert_eq!(s.pinned_plan(), Some(pinned), "updated in place");
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert_eq!(s.pinned_plan(), None, "back in the transcript");
        assert!(matches!(s.entries[pinned], Entry::Plan(_)));
    }

    #[test]
    fn going_idle_asks_for_a_run_state_check() {
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("hi"), false);
        let effects = s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(matches!(effects.as_slice(), [Effect::CheckRunState]));
    }
}
