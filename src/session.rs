//! One chat session: its transcript, message queue, cell-code memory, execution
//! "stop asking" state, and the notebook it was last looking at. Agent events are
//! applied here; anything that needs the workspace (sending to the agent, driving
//! the notebook pane, checking run state) comes back as an [`Effect`].

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use agent_client_protocol::Responder;
use agent_client_protocol::schema::MaybeUndefined;
use agent_client_protocol::schema::v1::{
    AvailableCommand, ContentBlock, PermissionOption, PermissionOptionKind, PlanEntry,
    RequestPermissionOutcome, RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigSelectOption, SessionConfigSelectOptions,
    SessionConfigValueId, SessionId,
    SessionModeId, SessionModeState, SessionUpdate, StopReason, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::text::{TextView, TextViewStyle};
use gpui_component::tooltip::Tooltip;

use crate::Workspace;
use crate::attach::{self, Attachment};
use crate::theme;
use crate::agent::{SessionEvent, Started, Turn};
use crate::celldiff::{self, CellCodes};
use crate::details;
use crate::gate;
use crate::hosts::Place;
use crate::pluto;
use crate::runs;
use crate::outbox::{Copying, Delivery, Dispatch, Outbox, Queued, Shown};

pub enum Entry {
    /// The user's words and, above them, their chips. `sent`: when, not known
    /// for replayed history.
    User { text: SharedString, expanded: bool, attachments: Vec<Attachment>, delivery: Delivery, sent: Option<SystemTime> },
    /// A reply, and when it started (not known for replayed history).
    Agent { text: String, at: Option<SystemTime> },
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
        /// How the user answered its prompt, if it asked.
        approval: Option<Approval>,
    },
    /// A stretch of thinking: when it started (not known for replayed history)
    /// and, once something follows it, how long it took.
    Thought { text: String, expanded: bool, started: Option<Instant>, took: Option<Duration> },
    Plan(Vec<PlanEntry>),
    /// A prompt for the call `call`. Once answered it stays, unseen, so it
    /// doesn't split the run it belongs to; the call's row shows the answer.
    Permission {
        call: ToolCallId,
        title: String,
        /// Code the call would run, when known.
        code: Option<String>,
        options: Vec<PermissionOption>,
        responder: Option<Responder<RequestPermissionResponse>>,
        /// Raised by the execution gate (a pluto call that runs code).
        runs_code: bool,
        /// The notebook tool and its input, for the run card.
        tool: Option<String>,
        input: serde_json::Value,
        /// What the run would run, once the runtime answers.
        preview: Option<pluto::RunPreview>,
        /// Plan mode's end: the plan to approve (markdown).
        plan: Option<String>,
    },
    Note(SharedString),
    /// What a turn left unrun or still running, cut back as the runtime reports
    /// progress: it shows only what is still true, and nothing once all is.
    RunState(Vec<pluto::RunWarning>),
}

/// The answer to a call's prompt, shown on its row.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Approval {
    Allowed,
    /// Allowed, and later runs in this session don't ask.
    AllowedFromNowOn,
    Denied,
    /// "Always this session" was on, so it didn't ask.
    WithoutAsking,
}

impl Approval {
    pub fn label(self) -> &'static str {
        match self {
            Approval::Allowed => "allowed",
            Approval::AllowedFromNowOn => "allowed, won't ask again",
            Approval::Denied => "denied",
            Approval::WithoutAsking => "ran without asking",
        }
    }
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
    /// A turn failed for want of sign-in: Claude is signed out.
    SignedOut,
}

/// A select config option's current value and its choices.
pub fn config_choices(config: &[SessionConfigOption], id: &str) -> Option<(SessionConfigValueId, Vec<SessionConfigSelectOption>)> {
    let option = config.iter().find(|c| c.id.to_string() == id)?;
    let SessionConfigKind::Select(select) = &option.kind else { return None };
    let options = match &select.options {
        SessionConfigSelectOptions::Ungrouped(options) => options.clone(),
        SessionConfigSelectOptions::Grouped(groups) => groups.iter().flat_map(|g| g.options.clone()).collect(),
        _ => Vec::new(),
    };
    Some((select.current_value.clone(), options))
}

/// A mode in the composer's mode menu: the agent mode it switches to, and
/// whether the app's run gate stops asking (Ask to run and Auto are both the
/// agent's auto mode).
#[derive(Clone, Debug, PartialEq)]
pub struct ModeChoice {
    pub mode: String,
    pub run_without_asking: bool,
    pub name: String,
    pub description: String,
}

impl ModeChoice {
    pub fn mode(&self) -> Mode {
        Mode { agent: self.mode.clone(), run_without_asking: self.run_without_asking }
    }
}

/// A session's mode: the agent's mode, and whether the run gate lets runs through
/// without asking. The label, the agent's mode, the runtime policy and the gate
/// all follow from it, and it is saved per session so a reopened one starts in it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mode {
    pub agent: String,
    pub run_without_asking: bool,
}

/// Where Manual sits in `app_modes()`.
pub const MANUAL: usize = 0;

/// The app's modes (docs/ui-spec.md, Composer), in the menu's order, plus the
/// agent's own Manual mode that sessions start in.
pub fn app_modes() -> Vec<ModeChoice> {
    let choice = |mode: &str, run_without_asking, name: &str, description: &str| ModeChoice {
        mode: mode.into(),
        run_without_asking,
        name: name.into(),
        description: description.into(),
    };
    vec![
        choice("default", false, "Manual", "Asks before each change"),
        choice("auto", false, "Ask to run", "Asks before running code"),
        choice("auto", true, "Auto", "Runs code without asking"),
        choice("plan", false, "Plan", "Reads only, then proposes a plan"),
    ]
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
    /// What the app tells Claude about how the session started (its server,
    /// its notebook), sent ahead of the first message whenever that comes.
    pub start_context: Option<ContentBlock>,
    pub title: String,
    /// The user named it; the agent's titles no longer replace it.
    pub named: bool,
    /// The title stands in for one ("New session", the notebook's name): the
    /// user's first message, sent or replayed, replaces it.
    pub untitled: bool,
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
    /// Its notebook file isn't there (moved, renamed or deleted outside Endeavor).
    pub missing: bool,
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
    /// The user message the running turn answers.
    turn_entry: Option<usize>,
    /// When the agent last said anything about this session.
    heard: Option<Instant>,
    /// A user message Claude couldn't answer (signed out, offline); it goes
    /// again by itself once Claude can be reached.
    pub unanswered: Option<usize>,
    /// The pinned plan (above the composer) is folded.
    pub plan_folded: bool,
    /// The message just copied from its Copy button, which shows a tick for a moment.
    pub copied: Option<usize>,
    /// The plan card shows the whole plan, not just its steps.
    pub plan_open: bool,
    /// Runs of tool calls the user opened, by their first call.
    open_runs: HashSet<ToolCallId>,
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
    /// On a cluster: what its job asks for (from the resources chip).
    pub resources: Option<wire::slurm::Resources>,
    /// The mode to put the agent in once it is up: the new-session screen's pick,
    /// or the mode a reopened session was last in. The agent itself starts every
    /// session, new or reopened, in its settings' default mode.
    pub start_mode: Option<Mode>,
    /// The sidebar row's Tab-stop handle. Lazily created (no `App` is available
    /// in `Session::new`'s many test call sites) and cached, so it stays stable.
    pub focus: RefCell<Option<FocusHandle>>,
    /// The open approval card's button handles, resized to match its button count.
    approval_focus: RefCell<Vec<FocusHandle>>,
    /// The pinned plan's fold toggle's Tab-stop handle.
    pinned_plan_focus: RefCell<Option<FocusHandle>>,
    /// The plan card's "Show the whole plan" toggle's Tab-stop handle.
    plan_card_focus: RefCell<Option<FocusHandle>>,
    /// A folded run's header toggle, by the run's first entry index.
    run_focus: RefCell<HashMap<usize, FocusHandle>>,
    /// An entry's own control (a tool or thought row's toggle, a message's
    /// Copy), by its entry index.
    row_focus: RefCell<HashMap<usize, FocusHandle>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stopped {
    /// It was in safe preview, so Start reopens it without running.
    pub safe_preview: bool,
    /// The runtime stopped it after this many hours idle.
    pub idle_hours: Option<u64>,
    /// The file's modification time when it stopped: if it changed by Start, the
    /// notebook opens in safe preview even if it was running.
    pub modified: Option<f64>,
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

/// A title from the agent, unless it is really the app's context note: without a
/// generated title, Claude Code falls back to the prompt's text, which starts
/// with the "[Endeavor] …" note the app sends ahead of the user's words.
pub fn agent_title(title: &str) -> Option<&str> {
    let title = title.trim();
    (!title.is_empty() && !title.starts_with("[Endeavor]")).then_some(title)
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
            start_context: None,
            title: "New session".into(),
            named: false,
            untitled: true,
            entries: Vec::new(),
            outbox: Outbox::waiting(),
            cell_codes: CellCodes::default(),
            run_without_asking: false,
            notebook: None,
            notebook_path: None,
            stopped: None,
            missing: false,
            user_edits: Vec::new(),
            list: {
                let list = ListState::new(0, ListAlignment::Top, px(1000.));
                list.set_follow_mode(FollowMode::Tail);
                list
            },
            list_len: Cell::new(0),
            dirty_from: Cell::new(None),
            busy_since: None,
            turn_entry: None,
            heard: None,
            unanswered: None,
            plan_folded: false,
            copied: None,
            plan_open: false,
            open_runs: HashSet::new(),
            replaying: false,
            replayed_path: None,
            failed: None,
            modes: None,
            config: Vec::new(),
            usage: None,
            commands: Vec::new(),
            policy_sent: "ask",
            resources: None,
            start_mode: None,
            focus: RefCell::new(None),
            approval_focus: RefCell::new(Vec::new()),
            pinned_plan_focus: RefCell::new(None),
            plan_card_focus: RefCell::new(None),
            run_focus: RefCell::new(HashMap::new()),
            row_focus: RefCell::new(HashMap::new()),
        }
    }

    /// The sidebar row's Tab-stop handle, created on first use.
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.focus.borrow_mut().get_or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    /// The approval card's Tab-stop handle for button `i`, growing the pool if needed.
    pub fn approval_focus(&self, i: usize, cx: &App) -> FocusHandle {
        let mut pool = self.approval_focus.borrow_mut();
        while pool.len() <= i {
            pool.push(cx.focus_handle().tab_stop(true));
        }
        pool[i].clone()
    }

    /// The pinned plan's fold toggle's Tab-stop handle, created on first use.
    pub fn pinned_plan_focus(&self, cx: &App) -> FocusHandle {
        self.pinned_plan_focus.borrow_mut().get_or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    /// The plan card's toggle's Tab-stop handle, created on first use.
    pub fn plan_card_focus(&self, cx: &App) -> FocusHandle {
        self.plan_card_focus.borrow_mut().get_or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    /// A folded run header's Tab-stop handle, by the run's first entry index.
    pub fn run_focus(&self, start: usize, cx: &App) -> FocusHandle {
        self.run_focus.borrow_mut().entry(start).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    /// An entry's own control's Tab-stop handle, by its entry index.
    pub fn row_focus(&self, ix: usize, cx: &App) -> FocusHandle {
        self.row_focus.borrow_mut().entry(ix).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
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
        self.open_runs.clear();
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
        config_choices(&self.config, id)
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

    /// The mode menu's choices: the app's modes the agent offers (see
    /// `app_modes`), or, for an agent without plan and auto modes, its own.
    pub fn mode_choices(&self) -> Vec<ModeChoice> {
        let Some(modes) = &self.modes else { return Vec::new() };
        let has = |id: &str| modes.available_modes.iter().any(|m| m.id.to_string() == id);
        if has("plan") && has("auto") {
            return app_modes().into_iter().filter(|c| has(&c.mode)).collect();
        }
        modes
            .available_modes
            .iter()
            .map(|m| ModeChoice { mode: m.id.to_string(), run_without_asking: false, name: m.name.clone(), description: m.description.clone().unwrap_or_default() })
            .collect()
    }

    /// Which of `mode_choices` is current.
    pub fn current_mode(&self) -> Option<usize> {
        let current = self.modes.as_ref()?.current_mode_id.to_string();
        self.mode_choices().iter().position(|c| c.mode == current && (c.mode != "auto" || c.run_without_asking == self.run_without_asking))
    }

    /// The session's mode, once the agent is up.
    pub fn mode(&self) -> Option<Mode> {
        let modes = self.modes.as_ref()?;
        Some(Mode { agent: modes.current_mode_id.to_string(), run_without_asking: self.run_without_asking })
    }

    /// Switch to a mode from the menu, showing it at once.
    pub fn choose_mode(&mut self, choice: &ModeChoice) -> Vec<Effect> {
        self.set_mode(&choice.mode())
    }

    /// Switch to a mode the agent offers, showing it at once; the agent confirms
    /// with a mode update.
    fn set_mode(&mut self, target: &Mode) -> Vec<Effect> {
        let Some(modes) = self.modes.as_mut() else { return Vec::new() };
        if !modes.available_modes.iter().any(|m| m.id.to_string() == target.agent) {
            return Vec::new();
        }
        self.run_without_asking = target.run_without_asking;
        let mut effects = Vec::new();
        let mode = SessionModeId::from(target.agent.clone());
        if mode != modes.current_mode_id {
            modes.current_mode_id = mode.clone();
            effects.push(Effect::SetMode(mode));
        }
        self.sync_policy(&mut effects);
        effects
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

    pub fn note_run_state(&mut self, warnings: Vec<pluto::RunWarning>) {
        if !warnings.is_empty() {
            self.push(Entry::RunState(warnings));
        }
    }

    /// Cut each run-state note back to what `notebooks` (`list_notebooks`
    /// shape) says is still true; whether any changed.
    pub fn refresh_run_state(&mut self, notebooks: &serde_json::Value) -> bool {
        let mut changed = Vec::new();
        for (ix, entry) in self.entries.iter_mut().enumerate() {
            if let Entry::RunState(said) = entry {
                let now = pluto::still_true(said, notebooks);
                if now != *said {
                    *said = now;
                    changed.push(ix);
                }
            }
        }
        changed.iter().for_each(|ix| self.mark(*ix));
        !changed.is_empty()
    }

    fn push(&mut self, entry: Entry) {
        self.end_thought();
        self.mark(self.entries.len());
        self.entries.push(entry);
    }

    /// Thinking ends when anything follows it, or the turn ends.
    fn end_thought(&mut self) {
        if let Some(Entry::Thought { started: Some(started), took: took @ None, .. }) = self.entries.last_mut() {
            *took = Some(started.elapsed());
            self.mark(self.entries.len() - 1);
        }
    }

    /// Entry `ix` (and anything after it) changed; the list re-measures it. A run
    /// of tool calls is drawn at its first entry, so that is re-measured too.
    fn mark(&self, ix: usize) {
        let ix = ix.min(self.entries.len());
        let ix = self.entries[..ix].iter().rposition(|e| !runs::is_member(e)).map_or(0, |i| i + 1).min(ix);
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
        if let Some(mode) = self.start_mode.take() {
            effects.extend(self.set_mode(&mode));
        }
        self.sync_policy(&mut effects);
        let next = self.outbox.turn_ended();
        self.dispatch(next, &mut effects);
        effects
    }

    /// Send or queue a message. Before the session exists everything queues.
    pub fn submit(&mut self, mut message: Queued, now: bool) -> Vec<Effect> {
        if self.failed.is_some() {
            self.note("This session isn't open, so nothing was sent");
            return Vec::new();
        }
        if let Some(context) = self.start_context.take() {
            message.blocks.insert(0, context);
        }
        self.title_from(&message.text);
        let mut effects = Vec::new();
        let dispatch = self.outbox.submit(message, now && self.id.is_some());
        self.dispatch(dispatch, &mut effects);
        effects
    }

    /// A queued message's files are copied (see `Outbox::copied`).
    pub fn copied(&mut self, ticket: Copying, done: Option<(Vec<Attachment>, Vec<ContentBlock>)>) -> Vec<Effect> {
        let mut effects = Vec::new();
        let dispatch = self.outbox.copied(ticket, done);
        self.dispatch(dispatch, &mut effects);
        effects
    }

    fn title_from(&mut self, text: &str) {
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().trim();
        if self.untitled && !self.named && !line.is_empty() {
            self.title = short_title(line);
            self.untitled = false;
        }
    }

    pub fn interrupt(&self) -> Option<Effect> {
        self.outbox.busy.then_some(Effect::Send(Turn::Cancel)).filter(|_| self.id.is_some())
    }

    fn dispatch(&mut self, dispatch: Option<Dispatch>, effects: &mut Vec<Effect>) {
        let Some(Dispatch { turn, shown }) = dispatch else { return };
        let again = shown.is_none() && matches!(turn, Turn::Prompt(_));
        effects.push(Effect::Send(turn));
        if let Some(Shown { text, attachments, delivery }) = shown {
            self.push(Entry::User { text: text.into(), expanded: false, attachments, delivery, sent: Some(SystemTime::now()) });
            self.turn_entry = Some(self.entries.len() - 1);
            self.busy_since.get_or_insert_with(Instant::now);
            // Sending jumps back to the bottom even if the user had scrolled up.
            self.list.set_follow_mode(FollowMode::Tail);
        } else if again {
            // The unanswered message went again: its bubble is already there.
            self.turn_entry = self.unanswered.take();
            if let Some(ix) = self.turn_entry {
                self.mark(ix);
            }
            self.busy_since.get_or_insert_with(Instant::now);
        }
    }

    /// Claude can't be reached: messages queue until `release`.
    pub fn hold(&mut self) {
        self.outbox.hold();
    }

    /// Claude can be reached again: the unanswered message goes first, then the queue.
    pub fn release(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.outbox.held && self.id.is_some() {
            let next = self.outbox.release();
            self.dispatch(next, &mut effects);
        } else {
            self.outbox.held = false;
        }
        effects
    }

    /// The turn got no answer because Claude couldn't be reached: its message
    /// stays, marked, and goes again on `release`.
    fn turn_unanswered(&mut self) {
        self.outbox.turn_unanswered();
        self.unanswered = self.turn_entry.take().or(self.unanswered);
        if let Some(ix) = self.unanswered {
            self.mark(ix);
        }
        self.busy_since = None;
        self.end_thought();
        self.mark(self.entries.len());
    }

    pub fn apply(&mut self, event: SessionEvent) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            SessionEvent::TurnEnded(reason) => {
                // Stopped to send the next message: its bubble says so.
                if let Some(note) = turn_ended_note(reason).filter(|_| !(reason == StopReason::Cancelled && self.outbox.stopping())) {
                    self.note(note);
                }
                self.turn_ended(&mut effects);
            }
            SessionEvent::AuthRequired => {
                self.turn_unanswered();
                effects.push(Effect::SignedOut);
            }
            // Held: Claude went out of reach while it worked.
            SessionEvent::TurnFailed(_) if self.outbox.held => self.turn_unanswered(),
            SessionEvent::ApiFailed(error) => return self.api_failed(&error),
            SessionEvent::TurnFailed(e) => {
                self.note(format!("⚠ Turn failed: {e}"));
                self.turn_ended(&mut effects);
            }
            SessionEvent::Steered => {
                if let Some(Shown { text, attachments, delivery }) = self.outbox.steered() {
                    self.push(Entry::User { text: text.into(), expanded: false, attachments, delivery, sent: Some(SystemTime::now()) });
                }
            }
            SessionEvent::Unsent => {
                let next = self.outbox.unsent();
                self.dispatch(next, &mut effects);
            }
            SessionEvent::Stopping => {
                let next = self.outbox.stopping_for();
                self.dispatch(next, &mut effects);
            }
            SessionEvent::Permission(request, responder) => {
                let fields = &request.tool_call.fields;
                let title = fields.title.clone().unwrap_or_else(|| "Tool call".into());
                // Only runs get the run card ("Always this session"); other notebook
                // prompts (e.g. Manual asking before an edit) get the agent's options.
                let input = fields.raw_input.clone().unwrap_or_default();
                let runs_code = title.strip_prefix(celldiff::TOOL_PREFIX).is_some_and(|tool| gate::runs_code(tool, &input));
                if runs_code && self.run_without_asking {
                    if let Some(allow) = option_of_kind(&request.options, PermissionOptionKind::AllowOnce) {
                        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(allow.option_id.clone()));
                        let _ = responder.respond(RequestPermissionResponse::new(outcome));
                        self.approve(&request.tool_call.tool_call_id, Approval::WithoutAsking, &title, &input);
                        return effects;
                    }
                }
                let code = input["code"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| input["cell_id"].as_str().and_then(|id| self.cell_codes.get(id)).map(str::to_owned));
                let tool = title.strip_prefix(celldiff::TOOL_PREFIX).map(str::to_owned);
                if let Some(tool) = tool.clone().filter(|t| runs_code && t != "run_shell") {
                    effects.push(Effect::PreviewRun { ix: self.entries.len(), tool, input: input.clone() });
                }
                // The adapter's ExitPlanMode prompt: its options carry these ids.
                let plan = request
                    .options
                    .iter()
                    .any(|o| o.option_id.to_string().starts_with("exit-plan-"))
                    .then(|| input["plan"].as_str().unwrap_or("").to_owned());
                let call = request.tool_call.tool_call_id.clone();
                if plan.is_some() {
                    self.plan_open = false;
                }
                self.push(Entry::Permission { call, title, code, options: request.options, responder: Some(responder), runs_code, tool, input, preview: None, plan });
            }
            SessionEvent::Update(update) => {
                self.heard = Some(Instant::now());
                self.apply_update(update, &mut effects);
            }
            SessionEvent::Config(options) => self.config = options,
        }
        effects
    }

    /// Claude's API failed the turn: the reply that only repeats the error goes,
    /// then the turn fails, or, held because Claude is out of reach, its message
    /// waits to go again.
    pub fn api_failed(&mut self, error: &str) -> Vec<Effect> {
        let last = self.entries.len().saturating_sub(1);
        if let Some(Entry::Agent { text, .. }) = self.entries.last_mut()
            && let Some(before) = text.trim_end().strip_suffix(error)
        {
            let before = before.trim_end().to_owned();
            self.mark(last);
            if before.is_empty() {
                self.entries.pop();
            } else if let Some(Entry::Agent { text, .. }) = self.entries.last_mut() {
                *text = before;
            }
        }
        self.apply(SessionEvent::TurnFailed(error.to_owned()))
    }

    fn turn_ended(&mut self, effects: &mut Vec<Effect>) {
        self.turn_entry = None;
        let next = self.outbox.turn_ended();
        let idle = next.is_none();
        if idle {
            // The pinned plan folds back into the transcript.
            if let Some(ix) = self.pinned_plan() {
                self.mark(ix);
            }
            self.busy_since = None;
            self.end_thought();
            // A run still going folds away its live row.
            self.mark(self.entries.len());
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
                // What the app added comes back as chips where it can (images and
                // text files carry their contents), else not at all.
                let (text, attachment) = match chunk.content {
                    ContentBlock::Text(t) => match attach::replayed_text_file(&t.text).or_else(|| attach::replayed_notebook(&t.text)).or_else(|| attach::replayed_saved_file(&t.text)) {
                        Some(attachment) => (None, Some(attachment)),
                        None if attach::is_app_text(&t.text) => return,
                        // Claude Code's own marker for a turn it stopped mid-flight: not
                        // the user's words, so replay shows the same note a live stop does.
                        None if attach::is_stopped_marker(&t.text) => return self.note("You stopped Claude"),
                        None => (Some(t.text), None),
                    },
                    ContentBlock::Image(image) => {
                        // A region's image follows its block.
                        if let Some(Entry::User { attachments, .. }) = self.entries.last_mut()
                            && let Some(Attachment::Region { png, .. }) = attachments.last_mut()
                            && png.is_empty()
                        {
                            if let Some(Attachment::Image { bytes, .. }) = attach::replayed_image(&image.data, &image.mime_type) {
                                *png = bytes;
                            }
                            return self.mark(self.entries.len() - 1);
                        }
                        (None, attach::replayed_image(&image.data, &image.mime_type))
                    }
                    _ => return,
                };
                if let Some(text) = &text {
                    self.title_from(text);
                }
                match self.entries.last_mut() {
                    Some(Entry::User { text: existing, attachments, .. }) => {
                        if let Some(text) = text {
                            *existing = if existing.is_empty() { text.into() } else { format!("{existing}\n{text}").into() };
                        }
                        attachments.extend(attachment);
                    }
                    _ => self.push(Entry::User { text: text.unwrap_or_default().into(), expanded: false, attachments: attachment.into_iter().collect(), delivery: Delivery::Turn, sent: None }),
                }
                self.mark(self.entries.len() - 1);
            }
            SessionUpdate::AgentMessageChunk(chunk) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Agent { text, .. }) => text.push_str(&t.text),
                        _ => {
                            let at = (!self.replaying).then(SystemTime::now);
                            self.push(Entry::Agent { text: t.text, at })
                        }
                    }
                    self.mark(self.entries.len() - 1);
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                if let ContentBlock::Text(t) = chunk.content {
                    match self.entries.last_mut() {
                        Some(Entry::Thought { text, .. }) => text.push_str(&t.text),
                        _ => {
                            let started = (!self.replaying).then(Instant::now);
                            self.push(Entry::Thought { text: t.text, expanded: false, started, took: None })
                        }
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
                approval: None,
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
                    if let Some(title) = agent_title(&title) {
                        self.title = title.to_string();
                        self.untitled = false;
                    }
                }
            }
            SessionUpdate::Notice(notice) => match notice.description.filter(|d| !d.trim().is_empty()) {
                Some(description) => self.note(format!("{}: {description}", notice.title)),
                None => self.note(notice.title),
            },
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
        let tool = celldiff::notebook_tool(title).filter(|_| *status == ToolCallStatus::Completed)?;
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

    /// Open or fold the run of tool calls starting at `ix`.
    pub fn toggle_run(&mut self, ix: usize) {
        let first = runs::run_at(&self.entries, ix)
            .and_then(|run| self.entries[run].iter().find_map(|e| if let Entry::Tool { id, .. } = e { Some(id.clone()) } else { None }));
        if let Some(id) = first {
            if !self.open_runs.remove(&id) {
                self.open_runs.insert(id);
            }
            self.mark(ix);
        }
    }

    pub fn toggle(&mut self, ix: usize) {
        if let Some(Entry::Tool { expanded, .. } | Entry::Thought { expanded, .. } | Entry::User { expanded, .. }) = self.entries.get_mut(ix) {
            *expanded = !*expanded;
            self.mark(ix);
        }
    }

    /// What the agent is doing, for the working line: this turn's latest running tool
    /// call in the folded runs' words, else thinking or working. The second part
    /// (a cell or file name) is code-like.
    pub(crate) fn activity(&self) -> (String, Option<String>) {
        let turn_start = self.turn_start();
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
        let Some((verb, phrase, names)) = runs::doing(title, kind, input) else { return ("Working".into(), None) };
        let name = match names {
            runs::Names::File => file_path(path.as_deref(), input).map(|p| file_name(&p)),
            runs::Names::Cell => input["code"]
                .as_str()
                .and_then(defined_name)
                .or_else(|| input["cell_id"].as_str().and_then(|id| self.cell_codes.get(id)).and_then(defined_name)),
            runs::Names::Nothing => None,
        };
        match name {
            Some(name) => (verb.into(), Some(name)),
            None => (phrase, None),
        }
    }

    /// Where the current turn starts: its user message. A message that joined
    /// the turn is part of it.
    fn turn_start(&self) -> usize {
        self.entries.iter().rposition(|e| matches!(e, Entry::User { delivery, .. } if *delivery != Delivery::Joined)).unwrap_or(0)
    }

    /// This turn's plan entry: the agent updates it in place.
    fn turn_plan(&self) -> Option<usize> {
        let turn_start = self.turn_start();
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

    /// The agent is asking to let the notebook run ("Let this notebook run?").
    pub fn asking_to_run(&self) -> bool {
        self.pending_permission().is_some_and(|ix| matches!(&self.entries[ix], Entry::Permission { tool: Some(tool), .. } if tool == "allow_execution"))
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
        let Some(Entry::Permission { call, title, responder, input, .. }) = self.entries.get_mut(ix) else { return };
        if let Some(responder) = responder.take() {
            let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option.option_id.clone()));
            let _ = responder.respond(RequestPermissionResponse::new(outcome));
            let allowed = matches!(option.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways);
            let approval = match (allowed, stop_asking) {
                (true, true) => Approval::AllowedFromNowOn,
                (true, false) => Approval::Allowed,
                (false, _) => Approval::Denied,
            };
            let (call, title, input) = (call.clone(), title.clone(), input.clone());
            self.mark(ix);
            self.approve(&call, approval, &title, &input);
            self.run_without_asking |= stop_asking;
        }
    }

    /// Show an answer on its call's row; a prompt with no call in the
    /// transcript gets a note instead.
    fn approve(&mut self, call: &ToolCallId, approval: Approval, title: &str, input: &serde_json::Value) {
        let found = self.entries.iter().rposition(|e| matches!(e, Entry::Tool { id, .. } if id == call));
        match found {
            Some(ix) => {
                if let Entry::Tool { approval: slot, .. } = &mut self.entries[ix] {
                    *slot = Some(approval);
                }
                self.mark(ix);
            }
            None => self.note(answer_note(approval, &runs::asked(title, input).unwrap_or_else(|| crate::approval::cut_line(title, 60)))),
        }
    }
}

/// The note for a turn that ended before Claude finished, in plain words.
/// Notes read as labels: one clause, no full stop.
pub(crate) fn turn_ended_note(reason: StopReason) -> Option<&'static str> {
    Some(match reason {
        StopReason::EndTurn => return None,
        StopReason::Cancelled => "You stopped Claude",
        StopReason::MaxTokens => "Claude stopped: the reply got too long",
        StopReason::MaxTurnRequests => "Claude stopped: it took too many steps in one go",
        StopReason::Refusal => "Claude declined to continue",
        _ => "Claude stopped",
    })
}

/// The note for an answer to a prompt whose call isn't in the transcript:
/// "Allowed: edit a cell", "Denied: run 2 cells".
pub(crate) fn answer_note(approval: Approval, what: &str) -> String {
    let answer = match approval {
        Approval::Allowed => "Allowed",
        Approval::AllowedFromNowOn => "Allowed from now on",
        Approval::WithoutAsking => "Allowed without asking",
        Approval::Denied => "Denied",
    };
    format!("{answer}: {what}")
}

/// A run-state warning as the transcript shows it.
pub(crate) fn run_state_line(warning: &pluto::RunWarning) -> String {
    format!("⚠ {warning}")
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

pub(crate) fn plan_option(options: &[PermissionOption]) -> Option<&PermissionOption> {
    PLAN_START.iter().find_map(|id| options.iter().find(|o| o.option_id.to_string() == *id))
}

pub(crate) fn option_of_kind(options: &[PermissionOption], kind: PermissionOptionKind) -> Option<&PermissionOption> {
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
                // A run of tool calls is drawn whole at its first entry.
                let element = match runs::run_at(&session.entries, ix) {
                    Some(run) if run.start == ix => Some(render_run(session, run, window, cx)),
                    Some(_) => None,
                    None => render_entry(this, session, ix, entry, window, cx),
                };
                // A message's own row of actions (Copy, its time) makes most of the gap below it.
                let message = matches!(entry, Entry::User { .. } | Entry::Agent { .. });
                match element {
                    Some(element) => div().px_4().when(!message, |d| d.pb_4()).when(message, |d| d.pb(px(2.))).child(element).into_any_element(),
                    None => div().into_any_element(),
                }
            })
            .unwrap_or_else(|_| div().into_any_element())
    })
    .flex_1()
    .pt_3()
}

/// The working line: an orbit, then what the agent is doing and for how long
/// ("Adding `residuals` · 12s"), or nothing while it waits on the user.
/// `offline_since`: the network went away then; a turn that has heard nothing
/// since is waiting for it.
pub fn render_activity(session: &Session, offline_since: Option<Instant>, cx: &App) -> Option<impl IntoElement + use<>> {
    let activity = activity(session, offline_since)?;
    let id = ElementId::NamedInteger("orbit".into(), session.key);
    let mark = if activity.waiting { crate::orbit::orbit_with(id, ORBIT, theme::text_muted(), cx) } else { crate::orbit::orbit(id, ORBIT, cx) };
    Some(
        div()
            .px_3()
            .pb_2()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(theme::size_meta())
            .text_color(theme::text_muted())
            .child(div().mr(px(4.)).child(mark))
            .child(activity.verb)
            .children(activity.object.map(|o| div().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_secondary()).child(o)))
            .children(activity.took)
            .into_any_element(),
    )
}

/// The working line's words.
pub(crate) struct Activity {
    /// Waiting for the network, not working.
    pub waiting: bool,
    pub verb: String,
    /// A cell or file name.
    pub object: Option<String>,
    /// "· 12s".
    pub took: Option<String>,
}

pub(crate) fn activity(session: &Session, offline_since: Option<Instant>) -> Option<Activity> {
    let since = session.busy_since?;
    // The approval card above the composer says it all.
    if session.needs_approval() {
        return None;
    }
    let waiting = offline_since.filter(|offline| session.heard.is_none_or(|heard| heard <= *offline));
    if let Some(offline) = waiting {
        let secs = offline.max(since).elapsed().as_secs();
        return Some(Activity { waiting: true, verb: format!("Waiting for the connection · {}:{:02}", secs / 60, secs % 60), object: None, took: None });
    }
    let (verb, object) = session.activity();
    Some(Activity { waiting: false, verb, object, took: Some(format!("· {}", elapsed(since.elapsed().as_secs()))) })
}

/// "12s", "1m 05s".
pub(crate) fn elapsed(secs: u64) -> String {
    if secs < 60 { format!("{secs}s") } else { format!("{}m {:02}s", secs / 60, secs % 60) }
}

const ORBIT: f32 = 14.;

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

fn render_entry(this: &Workspace, session: &Session, ix: usize, entry: &Entry, window: &mut Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let key = session.key;
    let muted = theme::text_muted();
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), key << 32 | ix as u64);
    Some(match entry {
        Entry::User { text, expanded, attachments, delivery, sent } => {
            let chips = this.render_sent_chips(key, ix, attachments, cx);
            let column = div().group(MESSAGE).flex().flex_col().items_end().gap(px(4.)).children(chips);
            let delivered = delivery_note(*delivery).map(|note| div().text_size(theme::size_meta()).text_color(muted).child(note));
            let actions = message_actions(session, ix, text.to_string(), *sent, cx);
            if text.is_empty() {
                return Some(column.children(delivered).child(actions).into_any_element());
            }
            let bubble = div()
                .max_w(px(USER_BUBBLE_WIDTH))
                .px(px(USER_BUBBLE_PAD_X))
                .py_2()
                .rounded(px(8.))
                .bg(theme::bg_raised())
                .font_family(theme::SANS)
                .text_size(theme::size_body())
                .line_height(theme::line_body());
            let unanswered = (session.unanswered == Some(ix)).then(|| this.render_unanswered());
            let line_height = theme::line_body();
            if bubble_lines(text, window) <= FOLD_AFTER {
                return Some(column.child(bubble.child(text.clone())).children(delivered).children(unanswered).child(actions).into_any_element());
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
            column
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
                .children(delivered)
                .children(unanswered)
                .child(actions)
                .into_any_element()
        }
        Entry::Agent { text, at } => div()
            .group(MESSAGE)
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(markdown(id("agent"), text.clone()))
            .child(message_actions(session, ix, text.clone(), *at, cx))
            .into_any_element(),
        Entry::Note(text) => div().text_size(theme::size_meta()).text_color(muted).child(text.clone()).into_any_element(),
        Entry::RunState(warnings) if warnings.is_empty() => return None,
        Entry::RunState(warnings) => div()
            .flex()
            .flex_col()
            .gap_1()
            .text_size(theme::size_meta())
            .text_color(muted)
            .children(warnings.iter().map(run_state_line))
            .into_any_element(),
        Entry::Tool { .. } | Entry::Thought { .. } => render_row(session, ix, false, window, cx),
        Entry::Plan(entries) => div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_size(theme::size_meta()).text_color(muted).child(crate::approval::progress(entries)))
            .children(crate::approval::plan_rows(entries))
            .into_any_element(),
        // Pending: shown as the approval card above the composer (render_approval).
        Entry::Permission { .. } => return None,
    })
}

/// The line under a message sent with Cmd+Enter while Claude worked.
pub(crate) fn delivery_note(delivery: Delivery) -> Option<&'static str> {
    match delivery {
        Delivery::Turn => None,
        Delivery::Joined => Some("Claude got this while working"),
        Delivery::AfterStop => Some("Stopped Claude's work to send this"),
    }
}

/// A message, for its actions (and a reply's code blocks' Copy) to show on hover.
const MESSAGE: &str = "message";

/// Under a message, shown while it's hovered: Copy, and how long ago it was
/// sent ("just now", "5 min ago"; the clock time on hover). Replayed history
/// has no times.
fn message_actions(session: &Session, ix: usize, text: String, at: Option<SystemTime>, cx: &mut Context<Workspace>) -> AnyElement {
    let key = session.key;
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), key << 32 | ix as u64);
    let copied = session.copied == Some(ix);
    let label = if copied { "Copied" } else { "Copy message" };
    let copy = (!text.is_empty()).then(|| {
        div()
            .id(id("copy-message"))
            .role(Role::Button)
            .aria_label(label)
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .opacity(if copied { 1. } else { 0. })
            .group_hover(MESSAGE, |s| s.opacity(1.))
            .hover(|s| s.bg(theme::bg_raised()))
            .track_focus(&session.row_focus(ix, cx))
            .tab_stop(true)
            .focus_visible(|s| s.opacity(1.).border_2().border_color(theme::focus_ring()))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .child(crate::new_session::glyph(if copied { crate::new_session::Glyph::Check } else { crate::new_session::Glyph::Copy }, theme::text_faint()))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                this.with_session(key, cx, |s| s.copied = Some(ix));
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(1500)).await;
                    let _ = this.update(cx, |this, cx| this.with_session(key, cx, |s| s.copied = s.copied.filter(|c| *c != ix)));
                })
                .detach();
            }))
    });
    let time = at.map(|at| {
        let clock = crate::when::clock(at.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
        div()
            .id(id("message-time"))
            .opacity(0.)
            .group_hover(MESSAGE, |s| s.opacity(1.))
            .text_size(theme::size_meta())
            .text_color(theme::text_faint())
            .tooltip(move |window, cx| Tooltip::new(clock.clone()).build(window, cx))
            .child(crate::when::ago(at))
    });
    div().flex().items_center().gap(px(4.)).h(px(20.)).children(copy).children(time).into_any_element()
}

/// An agent reply's markdown, with a copy button on each code block.
fn markdown(id: ElementId, text: String) -> TextView {
    TextView::markdown(id, text).style(markdown_style()).code_block_actions(|block, _, _| {
        let code = block.code().to_string();
        div()
            .id("copy")
            .px(px(6.))
            .py(px(2.))
            .rounded(px(4.))
            .cursor_pointer()
            .font_family(theme::SANS)
            .text_size(theme::size_meta_small())
            .text_color(theme::text_faint())
            // Out of the flow so the library's opaque holder for it stays empty and
            // doesn't hide the code's top-right corner while the button is invisible.
            // Changing display on hover instead panics when hover flips mid-frame.
            .absolute()
            .top_0()
            .right_0()
            .whitespace_nowrap()
            .bg(theme::bg_card())
            .opacity(0.)
            .group_hover(MESSAGE, |s| s.opacity(1.))
            .hover(|s| s.text_color(theme::text_primary()).bg(theme::bg_raised()))
            .child("Copy")
            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(code.clone())))
    })
}

/// Markdown on the type scale: headings at 15 px (h1, h2) and 13 px (h3 on),
/// code in JuliaMono 12 on the card colour, tag-coloured inline code, compact
/// tables with a muted 12 px header row. Borders, links and backgrounds come
/// from the component theme (set in main).
pub(crate) fn markdown_style() -> TextViewStyle {
    let code_block = StyleRefinement::default()
        .p(px(10.))
        .rounded(px(8.))
        .bg(theme::bg_card())
        .font_family(theme::MONO)
        .text_size(theme::size_code());
    let table = StyleRefinement::default().rounded(px(8.));
    let table_head = StyleRefinement::default().text_size(theme::size_meta()).text_color(theme::text_muted());
    let table_cell = StyleRefinement::default().px(px(8.)).py(px(3.));
    TextViewStyle::default()
        .paragraph_gap(rems(0.75))
        .heading_font_size(|level, _| if level <= 2 { theme::size_subhead() } else { theme::size_body() })
        .code_block(code_block)
        .table(table)
        .table_head(table_head)
        .table_cell(table_cell)
        .inline_code(HighlightStyle { background_color: Some(theme::bg_tag().into()), ..Default::default() })
}

/// A tool call's input and output panels scroll past this height.
const DETAIL_MAX_H: f32 = 160.;
const DETAIL_LINE: f32 = 16.;

/// A run of tool calls: one line summing it up ("Read 2 files, ran a command ›")
/// that opens into a bordered list with a row per call. While the run is still
/// going, its latest call shows under the line. A lone call is just its row.
fn render_run(session: &Session, run: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let key = session.key;
    let rows: Vec<usize> = run.clone().filter(|&i| matches!(session.entries[i], Entry::Tool { .. } | Entry::Thought { .. })).collect();
    if let [only] = rows[..] {
        return render_row(session, only, true, window, cx);
    }
    let calls: Vec<usize> = rows.iter().copied().filter(|&i| matches!(session.entries[i], Entry::Tool { .. })).collect();
    let open = run_open(session, &run);
    let live = session.busy_since.is_some() && run.end == session.entries.len();
    let start = run.start;
    // One run of text, so a long summary wraps with the failures and the chevron in line.
    let (mut text, counts) = run_summary(session, run.clone());
    let aria_label = format!("{text}, {}", if open { "expanded" } else { "collapsed" });
    let highlights: Vec<_> = counts.into_iter().map(|range| (range, HighlightStyle { color: Some(theme::danger().into()), ..Default::default() })).collect();
    text.push_str(if open { " ⌄" } else { " ›" });
    let header = div()
        .id(ElementId::NamedInteger("run".into(), key << 32 | start as u64))
        .role(Role::Button)
        .aria_label(aria_label)
        .cursor_pointer()
        .text_size(theme::size_meta())
        .text_color(theme::text_faint())
        .hover(|s| s.text_color(theme::text_secondary()))
        .border_2()
        .border_color(gpui::transparent_black())
        .track_focus(&session.run_focus(start, cx))
        .tab_stop(true)
        .focus_visible(|s| s.border_color(theme::focus_ring()))
        .child(StyledText::new(text).with_highlights(highlights))
        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.toggle_run(start))));
    let shown: Vec<usize> = match (open, live) {
        (true, _) => rows,
        (false, true) => calls.last().copied().into_iter().collect(),
        (false, false) => Vec::new(),
    };
    let list = (!shown.is_empty()).then(|| {
        div().flex().flex_col().rounded(px(6.)).border_1().border_color(theme::border()).children(shown.into_iter().enumerate().map(|(n, i)| {
            div()
                .px(px(8.))
                .py(px(4.))
                .when(n > 0, |d| d.border_t_1().border_color(theme::border()))
                .child(render_row(session, i, true, window, cx))
        }))
    });
    div().flex().flex_col().gap(px(6.)).child(header).children(list).into_any_element()
}

/// A tool call's folded row: its line, its edits' ± counts, and its state at the end.
pub(crate) struct ToolRow {
    pub line: ToolLine,
    /// A file edit's diff (a notebook call's edits are its `diffs`).
    pub file_diff: Option<celldiff::CellDiff>,
    pub added: usize,
    pub removed: usize,
    pub failed: bool,
    pub state: Option<RowState>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum RowState {
    Running,
    Denied,
    Failed,
}

impl RowState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            RowState::Running => "…",
            RowState::Denied => "denied",
            RowState::Failed => "failed",
        }
    }
}

pub(crate) fn tool_row(session: &Session, entry: &Entry) -> Option<ToolRow> {
    let Entry::Tool { title, kind, path, status, input, output, diffs, approval, .. } = entry else { return None };
    let args = input.as_ref().unwrap_or(&serde_json::Value::Null);
    let pluto = celldiff::notebook_tool(title).is_some();
    let file_diff = if pluto { None } else { file_diff(*kind, title, path.as_deref(), args) };
    let (added, removed) = diffs.iter().chain(&file_diff).flat_map(|d| &d.lines).fold((0, 0), |(a, r), (change, _)| match change {
        celldiff::Change::Added => (a + 1, r),
        celldiff::Change::Removed => (a, r + 1),
        celldiff::Change::Same => (a, r),
    });
    let name = |id: &str| session.cell_codes.get(id).and_then(defined_name);
    let running = matches!(status, ToolCallStatus::Pending | ToolCallStatus::InProgress);
    let line = if pluto { pluto_line(title, diffs, args, output.as_ref(), running, &name) } else { running_line(tool_line(title, *kind, path.as_deref(), args), title, *kind, args, running) };
    let failed = runs::failed(*status, title, output.as_ref());
    // A live denial is known from the approval card's answer; a replayed one
    // only from the call's raw result, which carries no such answer.
    let state = if *approval == Some(Approval::Denied) || runs::denied(output.as_ref()) {
        Some(RowState::Denied)
    } else if failed {
        Some(RowState::Failed)
    } else if running {
        Some(RowState::Running)
    } else {
        None
    };
    Some(ToolRow { line, file_diff, added, removed, failed, state })
}

/// A run of tool calls' folded header: what the calls did, then how many
/// failed or were denied ("Added 2 cells, ran 1 cell, 1 failed"), and the
/// ranges of the counts to show in red.
pub(crate) fn run_summary(session: &Session, run: std::ops::Range<usize>) -> (String, Vec<std::ops::Range<usize>>) {
    let mut failed = 0;
    let mut denied = 0;
    let mut summed = Vec::new();
    for entry in &session.entries[run] {
        let Entry::Tool { title, kind, status, input, output, approval, .. } = entry else { continue };
        if *approval == Some(Approval::Denied) || runs::denied(output.as_ref()) {
            denied += 1;
            continue;
        }
        failed += runs::failed(*status, title, output.as_ref()) as usize;
        summed.push((title.as_str(), *kind, input.as_ref().unwrap_or(&serde_json::Value::Null)));
    }
    let mut text = runs::summary(summed);
    let mut counts = Vec::new();
    for (n, what) in [(failed, "failed"), (denied, "denied")] {
        if n == 0 {
            continue;
        }
        if !text.is_empty() {
            text.push_str(", ");
        }
        let counted = format!("{n} {what}");
        counts.push(text.len()..text.len() + counted.len());
        text.push_str(&counted);
    }
    (text, counts)
}

/// Whether a run of tool calls is open, by its first call.
pub(crate) fn run_open(session: &Session, run: &std::ops::Range<usize>) -> bool {
    session.entries[run.clone()].iter().find_map(|e| if let Entry::Tool { id, .. } = e { Some(id) } else { None }).is_some_and(|id| session.open_runs.contains(id))
}

/// One call's line (grey verb, what it acted on, ± counts, `›`), opening in
/// place to its edits or input, then its output; or a stretch of thinking.
fn render_row(session: &Session, ix: usize, in_run: bool, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let (key, entry) = (session.key, &session.entries[ix]);
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), key << 32 | ix as u64);
    let toggle = cx.listener(move |this: &mut Workspace, _: &ClickEvent, _: &mut Window, cx: &mut Context<Workspace>| this.with_session(key, cx, |s| s.toggle(ix)));
    let line = |name: &'static str| {
        div()
            .id(id(name))
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .text_size(theme::size_meta())
            .text_color(theme::text_faint())
            .hover(|s| s.text_color(theme::text_secondary()))
    };
    match entry {
        Entry::Thought { text, expanded, started, took } => div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                line("thought")
                    .role(Role::Button)
                    .aria_label(format!("{}, {}", thought_label(in_run, *started, *took), if *expanded { "expanded" } else { "collapsed" }))
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .track_focus(&session.row_focus(ix, cx))
                    .tab_stop(true)
                    .focus_visible(|s| s.border_color(theme::focus_ring()))
                    .child(thought_label(in_run, *started, *took))
                    .child(if *expanded { "⌄" } else { "›" })
                    .on_click(toggle),
            )
            .when(*expanded, |d| {
                d.child(
                    scroll_y(div().id(id("thought-text")).max_h(px(DETAIL_MAX_H)), window, cx).line_height(px(DETAIL_LINE))
                        .pl(px(10.))
                        .border_l_1()
                        .border_color(theme::border())
                        .text_size(theme::size_meta())
                        .text_color(theme::text_muted())
                        .child(text.clone()),
                )
            })
            .into_any_element(),
        Entry::Tool { title, kind, path, input, output, diffs, expanded, approval, .. } => {
            let args = input.as_ref().unwrap_or(&serde_json::Value::Null);
            let Some(ToolRow { line: summary, file_diff, added, removed, failed, state }) = tool_row(session, entry) else { return div().into_any_element() };
            let aria_label = format!(
                "{}{}, {}",
                summary.verb,
                summary.object.as_deref().map(|o| format!(" {o}")).unwrap_or_default(),
                if *expanded { "expanded" } else { "collapsed" }
            );
            let all_diffs: Vec<&celldiff::CellDiff> = diffs.iter().chain(&file_diff).collect();
            let name = |id: &str| session.cell_codes.get(id).and_then(defined_name);
            let mono = |text: String, color: Rgba| div().flex_none().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(color).child(text);
            let state = state.map(|state| match state {
                RowState::Running => div().flex_none().child(state.label()),
                RowState::Denied | RowState::Failed => div().flex_none().text_color(theme::danger()).child(state.label()),
            });
            let object = summary.object.map(|text| {
                let d = div().id(id("tool-object")).min_w_0().truncate().text_color(theme::text_secondary()).child(text);
                let d = if summary.mono { d.font_family(theme::MONO).text_size(theme::size_meta_small()) } else { d };
                match summary.full {
                    Some(full) => d.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)),
                    None => d,
                }
            });
            let (input_panel, output_panel) = if *expanded {
                let details = details::details(title, *kind, path.as_deref(), args, output.as_ref(), failed, !all_diffs.is_empty(), &name);
                (render_parts(details.input, id("tool-input"), window, cx), render_parts(details.output, id("tool-output"), window, cx))
            } else {
                (None, None)
            };
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    line("tool")
                        .role(Role::Button)
                        .aria_label(aria_label)
                        .border_2()
                        .border_color(gpui::transparent_black())
                        .track_focus(&session.row_focus(ix, cx))
                        .tab_stop(true)
                        .focus_visible(|s| s.border_color(theme::focus_ring()))
                        .child(div().flex_none().whitespace_nowrap().child(summary.verb))
                        .children(object)
                        .when(added > 0, |d| d.child(mono(format!("+{added}"), theme::diff_add())))
                        .when(removed > 0, |d| d.child(mono(format!("−{removed}"), theme::diff_del())))
                        .children(approval.filter(|a| *a != Approval::Denied).map(|a| div().flex_none().whitespace_nowrap().child(format!("· {}", a.label()))))
                        .children(state)
                        .child(div().flex_none().child(if *expanded { "⌄" } else { "›" }))
                        .on_click(toggle),
                )
                .when(*expanded, |d| d.children(all_diffs.into_iter().map(render_diff)).children(input_panel).children(output_panel))
                .into_any_element()
        }
        _ => div().into_any_element(),
    }
}

/// A stretch of thinking's line: "Thought for 8s" once it's over, "Thinking"
/// while it goes on and inside a folded run of calls, "Thought" when replayed
/// history doesn't say how long.
pub(crate) fn thought_label(in_run: bool, started: Option<Instant>, took: Option<Duration>) -> String {
    match (in_run, started, took) {
        (true, _, _) | (false, Some(_), None) => "Thinking".into(),
        (false, _, Some(took)) => format!("Thought for {}", elapsed(took.as_secs().max(1))),
        (false, None, None) => "Thought".into(),
    }
}

/// An opened call's input or output: commands and code in a block, label and
/// value rows, sentences, plain mono text, failures in red, numbered lines.
fn render_parts(parts: Vec<details::Part>, id: ElementId, window: &mut Window, cx: &mut App) -> Option<Stateful<Div>> {
    use details::Part;
    if parts.is_empty() {
        return None;
    }
    let mono = |text: String, color: Rgba| div().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(color).child(text);
    let parts = parts.into_iter().map(|part| match part {
        Part::Code(code) => mono(code, theme::text_secondary()).px(px(8.)).py(px(5.)).rounded(px(4.)).bg(theme::bg_card()),
        Part::Fields(rows) => div().flex().flex_col().children(rows.into_iter().map(|(label, value)| {
            div()
                .flex()
                .gap(px(8.))
                .child(div().flex_none().min_w(px(44.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child(label))
                .child(mono(value.replace('`', ""), theme::text_secondary()).min_w_0())
        })),
        Part::Line(text) => div().flex().flex_wrap().text_size(theme::size_meta()).text_color(theme::text_muted()).children(text.split('`').enumerate().map(|(i, piece)| {
            // Flex drops a piece's edge spaces; non-breaking ones survive.
            let piece = piece.replace(' ', "\u{a0}");
            if i % 2 == 1 { mono(piece, theme::text_secondary()) } else { div().child(piece) }
        })),
        Part::Text(text) => mono(text, theme::text_muted()),
        Part::Error(text) => mono(text, theme::danger()),
        Part::Numbered(lines) => {
            let numbers: Vec<String> = lines.iter().map(|(n, _)| n.to_string()).collect();
            let code: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
            div()
                .flex()
                .child(mono(numbers.join("\n"), theme::text_section()).flex_none().text_right().pr(px(10.)))
                .child(mono(code.join("\n"), theme::text_muted()).min_w_0().overflow_hidden().whitespace_nowrap())
        }
    });
    Some(scroll_y(div().id(id).max_h(px(DETAIL_MAX_H)), window, cx).line_height(px(DETAIL_LINE)).flex().flex_col().gap(px(4.)).children(parts))
}

/// A panel that scrolls up to its max height, and hands the wheel on to the
/// transcript only once it can't scroll further that way.
fn scroll_y(panel: Stateful<Div>, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let mut panel = panel;
    let key = panel.interactivity().element_id.clone().expect("scroll panels have an id");
    let handle = window.use_keyed_state(key, cx, |_, _| ScrollHandle::new()).read(cx).clone();
    let tracked = handle.clone();
    panel.overflow_y_scroll().track_scroll(&handle).on_scroll_wheel(move |event, window, cx| {
        let dy = event.delta.pixel_delta(window.line_height()).y;
        let (at, max) = (tracked.offset().y, tracked.max_offset().y);
        let can_scroll = if dy < px(0.) { at > -max } else { at < px(0.) };
        if max > px(0.) && can_scroll {
            cx.stop_propagation();
        }
    })
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
    let Some(tool) = celldiff::notebook_tool(title) else { return title.to_string() };
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
        "list_folder" => "Listed",
        "read_file" => "Read",
        "run_shell" => "Ran",
        "keep_notebook_alive" => "Kept notebook running",
        other => other,
    }
    .to_string()
}

/// A notebook call's line: its verb, then the cells it changed or acted on, the
/// command it ran, the file it read or the notebook it read. A running call's
/// verb is in the present ("Adding `x`", or "Adding a cell" before its input
/// arrives); a bare verb with nothing to name says what it acted on ("Read a cell").
fn pluto_line(
    title: &str,
    diffs: &[celldiff::CellDiff],
    input: &serde_json::Value,
    output: Option<&serde_json::Value>,
    running: bool,
    name: &dyn Fn(&str) -> Option<String>,
) -> ToolLine {
    let mut line = pluto_object(title, diffs, input, output, name);
    let known = runs::doing(title, ToolKind::Other, input);
    let doing = known.clone().filter(|_| running);
    line.verb = match (doing, &line.object) {
        (Some((verb, _, _)), Some(_)) => verb.to_string(),
        (Some((_, phrase, _)), None) => phrase,
        (None, None) if known.is_some() && !line.verb.contains(' ') => runs::summary([(title, ToolKind::Other, input)]),
        (None, _) => std::mem::take(&mut line.verb),
    };
    line
}

fn pluto_object(
    title: &str,
    diffs: &[celldiff::CellDiff],
    input: &serde_json::Value,
    output: Option<&serde_json::Value>,
    name: &dyn Fn(&str) -> Option<String>,
) -> ToolLine {
    let verb = tool_verb(title);
    let names: Vec<String> = diffs.iter().map(cell_name).collect();
    if !names.is_empty() {
        return ToolLine { verb, object: Some(names.join(", ")), mono: true, full: None };
    }
    let field = |name: &str| input[name].as_str().map(str::trim).filter(|s| !s.is_empty());
    let answer = output.and_then(celldiff::tool_json);
    let cell = || {
        let read = answer.as_ref().filter(|a| a["cell_id"] == input["cell_id"]).and_then(|a| a["code"].as_str()).and_then(defined_name);
        read.or_else(|| field("code").and_then(defined_name)).or_else(|| field("cell_id").and_then(name))
    };
    let names_a_cell = matches!(runs::doing(title, ToolKind::Other, input), Some((_, _, runs::Names::Cell)));
    match (celldiff::notebook_tool(title), field("path"), field("command")) {
        (Some("run_shell"), _, Some(command)) => ToolLine { verb, object: Some(first_line(command)), mono: true, full: None },
        (Some("read_file" | "list_folder"), Some(path), _) => ToolLine { verb, object: Some(file_name(path)), mono: true, full: Some(path.to_string()) },
        (Some("read_notebook_code"), _, _) => {
            let path = answer.as_ref().and_then(|a| a["path"].as_str()).filter(|p| !p.is_empty()).map(str::to_owned);
            ToolLine { verb, object: path.as_deref().map(file_name), mono: true, full: path }
        }
        _ if names_a_cell => ToolLine { verb, object: cell(), mono: true, full: None },
        _ => ToolLine { verb, object: None, mono: true, full: None },
    }
}

/// A cell's name for the transcript: what it defines (`model(S, p) = …` → `model`,
/// `x = …` → `x`), else its label.
fn cell_name(diff: &celldiff::CellDiff) -> String {
    let first = diff.lines.iter().find(|(c, l)| !matches!(c, celldiff::Change::Removed) && !l.trim().is_empty());
    first.and_then(|(_, line)| defined_name(line)).unwrap_or_else(|| diff.label.clone())
}

/// What code defines, from its first line: `model(S, p) = …` → `model`, `x = …` → `x`.
pub(crate) fn defined_name(code: &str) -> Option<String> {
    let line = code.lines().find(|l| !l.trim().is_empty())?;
    let lhs = line.split_once('=').map(|(lhs, _)| lhs).unwrap_or(line);
    let lhs = lhs.trim().trim_start_matches("function ").trim_start_matches("const ");
    let name: String = lhs.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '!').collect();
    (!name.is_empty() && line.contains('=')).then_some(name)
}

/// A tool call's collapsed line.
pub(crate) struct ToolLine {
    pub verb: String,
    /// What it acted on: a file name, pattern or command line (mono), or the
    /// agent's own description of a command (not mono). One line.
    pub object: Option<String>,
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

/// A running call that names nothing yet (its input still on the way) says
/// what it's doing ("Running a command"), not a bare past tense.
fn running_line(mut line: ToolLine, title: &str, kind: ToolKind, input: &serde_json::Value, running: bool) -> ToolLine {
    if running
        && line.object.is_none()
        && let Some((_, phrase, _)) = runs::doing(title, kind, input)
    {
        line.verb = phrase;
    }
    line
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

/// A file edit's old and new text as a diff; a written file is all new.
fn file_diff(kind: ToolKind, title: &str, path: Option<&Path>, input: &serde_json::Value) -> Option<celldiff::CellDiff> {
    if kind != ToolKind::Edit {
        return None;
    }
    let (old, new) = match (input["old_string"].as_str(), input["new_string"].as_str(), input["content"].as_str()) {
        (None, None, Some(content)) if title.starts_with("Write") || input["file_path"].is_string() => ("", content),
        (None, None, _) => return None,
        (old, new, _) => (old.unwrap_or(""), new.unwrap_or("")),
    };
    let label = file_path(path, input).map_or_else(|| "edit".into(), |p| file_name(&p));
    Some(celldiff::CellDiff { label, lines: celldiff::line_diff(old, new) })
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]` macro.
    use super::{Effect, Entry, Mode, RowState, Session, SessionEvent, Started, Turn, app_modes, run_summary, tool_row};
    use crate::attach::Attachment;
    use crate::hosts::Place;
    use crate::outbox::Queued;
    use agent_client_protocol::schema::v1::{
        AvailableCommand, AvailableCommandsUpdate, CurrentModeUpdate, SessionId, SessionMode, SessionModeState, SessionUpdate,
        StopReason, UsageUpdate,
    };

    #[test]
    fn answers_show_on_their_calls_and_dont_split_runs() {
        use super::Approval;
        use agent_client_protocol::schema::v1::{ToolCall, ToolCallId};
        let call = |id: &str, title: &str| SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new(id.to_string(), title.to_string())));
        let asked = |id: &str, title: &str| Entry::Permission {
            call: ToolCallId::new(id.to_string()),
            title: title.into(),
            code: None,
            options: Vec::new(),
            responder: None,
            runs_code: true,
            tool: None,
            input: serde_json::Value::Null,
            preview: None,
            plan: None,
        };
        let approval = |s: &Session, ix: usize| match &s.entries[ix] {
            Entry::Tool { approval, .. } => *approval,
            _ => panic!("not a call"),
        };
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.apply(call("t1", "mcp__notebook__edit_cell"));
        s.push(asked("t1", "mcp__notebook__edit_cell"));
        s.approve(&"t1".to_string().into(), Approval::Allowed, "mcp__notebook__edit_cell", &serde_json::Value::Null);
        s.apply(call("t2", "mcp__notebook__execute_cell"));
        s.approve(&"t2".to_string().into(), Approval::WithoutAsking, "mcp__notebook__execute_cell", &serde_json::Value::Null);
        s.apply(call("t3", "mcp__notebook__execute_cell"));
        s.push(asked("t3", "mcp__notebook__execute_cell"));
        s.approve(&"t3".to_string().into(), Approval::Denied, "mcp__notebook__execute_cell", &serde_json::Value::Null);
        assert!(!s.entries.iter().any(|e| matches!(e, Entry::Note(_))), "no notes in the transcript");
        assert_eq!(crate::runs::run_at(&s.entries, 0), Some(0..s.entries.len()), "one run");
        assert_eq!(approval(&s, 0), Some(Approval::Allowed));
        assert_eq!(approval(&s, 2), Some(Approval::WithoutAsking));
        assert_eq!(approval(&s, 3), Some(Approval::Denied));

        // A prompt whose call isn't in the transcript is still recorded, as a note.
        s.approve(&"gone".to_string().into(), Approval::Denied, "mcp__notebook__edit_cell", &serde_json::Value::Null);
        assert!(matches!(s.entries.last(), Some(Entry::Note(text)) if text.as_ref() == "Denied: edit a cell"));
    }

    #[test]
    fn a_replayed_denial_is_told_apart_from_a_failure() {
        use agent_client_protocol::schema::v1::{ToolCallId, ToolCallStatus, ToolKind};
        // The exact shape a reopened session replays: `status` failed and the
        // call's raw output is Claude Code's denial text, with no approval
        // answer to check (that's only ever known live).
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.entries.push(Entry::Tool {
            id: ToolCallId::new("t1"),
            title: "Bash".into(),
            kind: ToolKind::Execute,
            path: None,
            status: ToolCallStatus::Failed,
            input: None,
            output: Some(serde_json::json!("User refused permission to run tool")),
            diffs: Vec::new(),
            expanded: false,
            approval: None,
        });
        let row = tool_row(&s, &s.entries[0]).expect("a tool row");
        assert_eq!(row.state, Some(RowState::Denied));
        assert!(!row.failed, "a denial isn't a failure");
        assert_eq!(run_summary(&s, 0..1).0, "1 denied");
    }

    #[test]
    fn transcript_notes_say_what_happened_in_plain_words() {
        use super::{Approval, answer_note, turn_ended_note};
        assert_eq!(turn_ended_note(StopReason::EndTurn), None);
        assert_eq!(turn_ended_note(StopReason::Cancelled), Some("You stopped Claude"));
        assert_eq!(turn_ended_note(StopReason::MaxTokens), Some("Claude stopped: the reply got too long"));
        assert_eq!(turn_ended_note(StopReason::MaxTurnRequests), Some("Claude stopped: it took too many steps in one go"));
        assert_eq!(turn_ended_note(StopReason::Refusal), Some("Claude declined to continue"));
        assert_eq!(answer_note(Approval::Allowed, "edit a cell"), "Allowed: edit a cell");
        assert_eq!(answer_note(Approval::AllowedFromNowOn, "run 2 cells"), "Allowed from now on: run 2 cells");
        assert_eq!(answer_note(Approval::WithoutAsking, "run a cell"), "Allowed without asking: run a cell");
        assert_eq!(answer_note(Approval::Denied, "delete a cell"), "Denied: delete a cell");

        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("go"), false);
        s.apply(SessionEvent::TurnEnded(StopReason::Cancelled));
        assert!(matches!(s.entries.last(), Some(Entry::Note(note)) if note.as_ref() == "You stopped Claude"));
        s.approve(&"gone".to_string().into(), Approval::WithoutAsking, "mcp__notebook__submit_changes", &serde_json::json!({ "cell_ids": ["a", "b"] }));
        assert!(matches!(s.entries.last(), Some(Entry::Note(note)) if note.as_ref() == "Allowed without asking: run 2 cells"));
    }

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
    fn file_edits_and_writes_show_as_diffs() {
        use crate::celldiff::Change;
        use agent_client_protocol::schema::v1::ToolKind;
        use serde_json::json;
        let edit = json!({"file_path": "/repo/a.rs", "old_string": "a\nb", "new_string": "a\nc"});
        let diff = super::file_diff(ToolKind::Edit, "Edit /repo/a.rs", None, &edit).unwrap();
        assert_eq!(diff.label, "a.rs");
        assert_eq!(diff.lines, vec![(Change::Same, "a".into()), (Change::Removed, "b".into()), (Change::Added, "c".into())]);
        let write = json!({"file_path": "/repo/n.txt", "content": "one\ntwo"});
        let diff = super::file_diff(ToolKind::Edit, "Write /repo/n.txt", None, &write).unwrap();
        assert_eq!((diff.label.as_str(), diff.lines), ("n.txt", vec![(Change::Added, "one".into()), (Change::Added, "two".into())]));
        assert!(super::file_diff(ToolKind::Read, "Read /repo/a.rs", None, &edit).is_none());
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
    fn notebook_rows_name_what_they_act_on() {
        use agent_client_protocol::schema::v1::ToolKind;
        use serde_json::json;
        let seen = |id: &str| (id == "c-fit").then(|| "fit".to_string());
        let line = |title: &str, input: serde_json::Value, output: Option<serde_json::Value>, running: bool| {
            let l = super::pluto_line(&format!("mcp__notebook__{title}"), &[], &input, output.as_ref(), running, &seen);
            (l.verb, l.object)
        };
        let out = |v: serde_json::Value| Some(json!([{ "type": "text", "text": v.to_string() }]));
        let null = serde_json::Value::Null;
        assert_eq!(line("add_cell", null.clone(), None, true), ("Adding a cell".into(), None), "before its input arrives");
        assert_eq!(line("add_cell", json!({"code": "y = 2x"}), None, true), ("Adding".into(), Some("y".into())));
        assert_eq!(line("add_cell", null.clone(), None, false), ("Added a cell".into(), None));
        assert_eq!(line("read_cell", json!({"cell_id": "c-9"}), out(json!({"cell_id": "c-9", "code": "model(S, p) = p[1] * S"})), false), ("Read".into(), Some("model".into())));
        assert_eq!(line("read_cell", json!({"cell_id": "c-fit"}), None, true), ("Reading".into(), Some("fit".into())));
        assert_eq!(line("read_cell", json!({"cell_id": "c-9"}), out(json!({"cell_id": "c-9", "code": "scatter(x)"})), false), ("Read a cell".into(), None));
        assert_eq!(line("read_notebook_code", json!({"notebook_id": "n"}), out(json!({"path": "/w/fit.jl", "code": ""})), false), ("Read".into(), Some("fit.jl".into())));
        assert_eq!(line("read_notebook_code", json!({"notebook_id": "n"}), None, true), ("Reading the notebook".into(), None));
        assert_eq!(line("list_notebooks", null.clone(), None, false), ("Listed notebooks".into(), None));
        assert_eq!(line("fold_cell", null.clone(), None, false), ("fold_cell".into(), None));
        let bash = super::running_line(super::tool_line("Terminal", ToolKind::Execute, None, &null), "Terminal", ToolKind::Execute, &null, true);
        assert_eq!((bash.verb, bash.object), ("Running a command".into(), None), "a command before its input arrives");
    }

    #[test]
    fn a_notice_is_its_own_note_not_part_of_the_reply() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, Notice, NoticeSeverity, TextContent};
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        let notice = Notice::new(NoticeSeverity::Warning, "Auto mode unavailable").description("The selected model does not support Auto mode; using Accept edits instead.".to_string());
        s.apply(SessionEvent::Update(SessionUpdate::Notice(notice)));
        s.apply(SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new("Yes — it runs."))))));
        assert!(matches!(&s.entries[..], [
            Entry::Note(note),
            Entry::Agent { text: reply, .. },
        ] if note.as_ref() == "Auto mode unavailable: The selected model does not support Auto mode; using Accept edits instead." && reply == "Yes — it runs."));
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
        assert!(plain.mode_choices().is_empty());
    }

    #[test]
    fn a_session_starts_in_its_start_mode_and_keeps_it() {
        use agent_client_protocol::schema::v1::{ConfigOptionUpdate, SessionConfigOption, SessionConfigSelectOption};
        let agent_modes = |current: &str| {
            SessionModeState::new(current.to_string(), vec![SessionMode::new("default", "Manual"), SessionMode::new("plan", "Plan"), SessionMode::new("auto", "Auto")])
        };
        let mode_option = |current: &str| {
            let options: Vec<SessionConfigSelectOption> = ["default", "plan", "auto"].into_iter().map(|m| SessionConfigSelectOption::new(m, m)).collect();
            SessionConfigOption::select("mode", "Mode", current.to_string(), options)
        };
        let start = |current: &str, pick: usize| {
            let mut s = Session::new(1, Place::local("/tmp/project"), None);
            s.start_mode = Some(app_modes()[pick].mode());
            let effects = s.started(Started::new(SessionId::new("s1"), Some(agent_modes(current)), None));
            (s, effects)
        };

        // Auto picked on the new-session screen: the agent is asked, and runs stop asking.
        let (mut s, effects) = start("default", 2);
        assert!(matches!(effects.as_slice(), [Effect::SetMode(m)] if m.to_string() == "auto"));
        assert_eq!((s.mode_name().as_deref(), s.policy()), (Some("Auto"), "ask"));
        assert_eq!(s.mode(), Some(Mode { agent: "auto".into(), run_without_asking: true }));

        // A config change's reply, sent before the switch, comes back after the
        // agent confirmed it: the mode stays.
        s.apply(SessionEvent::Update(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(vec![mode_option("auto")]))));
        s.apply(SessionEvent::Config(vec![mode_option("default")]));
        assert_eq!(s.mode_name().as_deref(), Some("Auto"));
        // The agent's own switch (a plan approved, a fallback) still counts.
        s.apply(SessionEvent::Update(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(vec![mode_option("plan")]))));
        assert_eq!((s.mode_name().as_deref(), s.policy()), (Some("Plan"), "plan"));

        // Manual is applied too, when the agent starts in another mode.
        let (s, effects) = start("auto", 0);
        assert!(matches!(effects.as_slice(), [Effect::SetMode(m)] if m.to_string() == "default"));
        assert_eq!(s.mode_name().as_deref(), Some("Manual"));

        // A reopened session in Plan: the agent is asked and the runtime told.
        let (s, effects) = start("default", 3);
        assert!(matches!(effects.as_slice(), [Effect::SetMode(m), Effect::SetPolicy("plan")] if m.to_string() == "plan"));
        assert_eq!(s.mode_name().as_deref(), Some("Plan"));

        // Already in the mode: nothing to ask.
        let (s, effects) = start("auto", 1);
        assert!(effects.is_empty());
        assert_eq!(s.mode_name().as_deref(), Some("Ask to run"));
    }

    #[test]
    fn the_mode_menu_picks_a_mode_directly() {
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        let modes = SessionModeState::new(
            "default",
            vec![SessionMode::new("default", "Manual"), SessionMode::new("acceptEdits", "Accept edits"), SessionMode::new("plan", "Plan"), SessionMode::new("auto", "Auto")],
        );
        s.started(Started::new(SessionId::new("s1"), Some(modes), None));
        let names: Vec<String> = s.mode_choices().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Manual", "Ask to run", "Auto", "Plan"]);
        assert_eq!(s.current_mode(), Some(0));
        let auto = s.mode_choices()[2].clone();
        assert!(matches!(s.choose_mode(&auto).as_slice(), [Effect::SetMode(m)] if m.to_string() == "auto"));
        assert_eq!((s.current_mode(), s.mode_name().as_deref()), (Some(2), Some("Auto")));
        let plan = s.mode_choices()[3].clone();
        assert!(matches!(s.choose_mode(&plan).as_slice(), [Effect::SetMode(_), Effect::SetPolicy("plan")]));
        assert!(!s.run_without_asking);

        // An agent without plan and auto: its own modes and descriptions.
        let mut other = Session::new(2, Place::local("/tmp/project"), None);
        let modes = SessionModeState::new("a", vec![SessionMode::new("a", "Careful").description("Asks a lot"), SessionMode::new("b", "Bold")]);
        other.started(Started::new(SessionId::new("s2"), Some(modes), None));
        let choices: Vec<(String, String)> = other.mode_choices().into_iter().map(|c| (c.name, c.description)).collect();
        assert_eq!(choices, [("Careful".to_string(), "Asks a lot".to_string()), ("Bold".into(), "".into())]);
    }

    fn text(s: &str) -> Queued {
        Queued::new(s.into(), vec![], vec![])
    }

    #[test]
    fn a_stopped_turn_notes_that_the_user_stopped_it() {
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("go"), false);
        s.apply(SessionEvent::TurnEnded(StopReason::Cancelled));
        assert!(matches!(s.entries.last(), Some(Entry::Note(note)) if note.as_ref() == "You stopped Claude"));
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
    fn a_message_claude_could_not_answer_waits_marked_and_goes_again_after_sign_in() {
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("plot the residuals"), false);
        let effects = s.apply(SessionEvent::AuthRequired);
        assert!(matches!(effects.as_slice(), [Effect::SignedOut]));
        assert_eq!(s.unanswered, Some(0));
        assert!(s.busy_since.is_none());
        assert!(s.submit(text("and label the axes"), false).is_empty(), "waits while signed out");
        let effects = s.release();
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]));
        assert_eq!(s.unanswered, None);
        assert_eq!(s.entries.len(), 1, "the kept message goes again without a second bubble");
        let effects = s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]));
        assert!(matches!(&s.entries[1], Entry::User { text, .. } if text.as_ref() == "and label the axes"));
    }

    #[test]
    fn an_api_error_is_not_left_as_the_reply() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};
        const ERROR: &str = "API Error: Unable to connect to API. Check your internet connection";
        let reply = |text: &str| SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(text)))));
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));

        s.submit(text("plot it"), false);
        s.apply(reply(ERROR));
        s.hold();
        s.apply(SessionEvent::ApiFailed(ERROR.into()));
        assert!(matches!(s.entries.as_slice(), [Entry::User { .. }]), "offline: only the message, kept");
        assert_eq!(s.unanswered, Some(0));

        s.release();
        s.apply(reply("Here it is.\n\n"));
        s.apply(reply(ERROR));
        s.apply(SessionEvent::ApiFailed(ERROR.into()));
        assert!(matches!(&s.entries[1..], [Entry::Agent { text: said, .. }, Entry::Note(note)]
            if said == "Here it is." && note.as_ref() == format!("⚠ Turn failed: {ERROR}")));
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
        s.apply(chunk("notebook://pluto/n/cell/c"));
        s.apply(chunk("pluto://notebook/n/cell/c"));
        s.apply(chunk("attachment:notes.txt"));
        s.apply(chunk("\n<context ref=\"attachment:notes.txt\">\nt,y\n</context>"));
        s.apply(chunk("plot sin"));
        let [Entry::User { text, attachments, .. }] = s.entries.as_slice() else { panic!("one user entry") };
        assert_eq!(text.as_ref(), "plot sin");
        assert_eq!(attachments.as_slice(), [Attachment::Text { name: "notes.txt".into(), text: "t,y".into() }]);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.apply(chunk("live echo"));
        assert_eq!(s.entries.len(), 1, "after loading, user chunks are ignored");
    }

    #[test]
    fn live_messages_know_when_they_were_sent_and_replayed_ones_do_not() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};
        let reply = |t: &str| SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(t)))));
        let times = |s: &Session| -> Vec<bool> {
            s.entries
                .iter()
                .map(|e| match e {
                    Entry::User { sent, .. } => sent.is_some(),
                    Entry::Agent { at, .. } => at.is_some(),
                    _ => panic!("only messages"),
                })
                .collect()
        };
        let mut old = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old chat".into());
        old.apply(SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new("hi"))))));
        old.apply(reply("Hello."));
        assert_eq!(times(&old), [false, false]);
        old.started(Started::new(SessionId::new("abc"), None, None));
        old.submit(text("plot it"), false);
        old.apply(reply("Plotted."));
        assert_eq!(times(&old), [false, false, true, true]);
    }

    #[test]
    fn a_reopened_session_shows_a_stopped_turn_as_a_note_not_the_users_words() {
        use agent_client_protocol::schema::v1::{ContentChunk, SessionUpdate, TextContent};
        use agent_client_protocol::schema::v1::ContentBlock;
        let chunk = |s: &str| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(s)))));
        // The exact shape Claude Code replays for a stop mid-tool-call: the
        // real prompt, then its own synthetic marker as a separate user turn.
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old chat".into());
        s.apply(chunk("run the slow thing"));
        s.apply(chunk("[Request interrupted by user for tool use]"));
        let [Entry::User { text, .. }, Entry::Note(note)] = s.entries.as_slice() else { panic!("a user entry, then a note") };
        assert_eq!(text.as_ref(), "run the slow thing");
        assert_eq!(note.as_ref(), "You stopped Claude");

        // A stop outside a tool call carries the shorter marker.
        let mut s2 = Session::loading(2, SessionId::new("def"), Place::local("/tmp"), None, "Old chat".into());
        s2.apply(chunk("[Request interrupted by user]"));
        assert!(matches!(s2.entries.as_slice(), [Entry::Note(note)] if note.as_ref() == "You stopped Claude"));
    }

    /// The blocks of a sent prompt as a reopened session replays them: the
    /// adapter stores links and text files as text, and a text file's contents
    /// after the rest (claude-agent-acp's `promptToClaude`).
    fn as_replayed(blocks: Vec<agent_client_protocol::schema::v1::ContentBlock>) -> Vec<SessionEvent> {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, EmbeddedResourceResource, SessionUpdate, TextContent};
        let text = |s: String| ContentBlock::Text(TextContent::new(s));
        let (mut content, mut context) = (Vec::new(), Vec::new());
        for block in blocks {
            match block {
                ContentBlock::ResourceLink(link) => content.push(text(link.uri)),
                ContentBlock::Resource(r) => {
                    if let EmbeddedResourceResource::TextResourceContents(t) = r.resource {
                        content.push(text(t.uri.clone()));
                        context.push(text(format!("\n<context ref=\"{}\">\n{}\n</context>", t.uri, t.text)));
                    }
                }
                other => content.push(other),
            }
        }
        content.into_iter().chain(context).map(|b| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(b)))).collect()
    }

    #[test]
    fn a_reopened_session_gets_its_chips_back_as_sent() {
        use crate::attach::{Cell, CellAsk};
        use std::sync::Arc;
        const NB: &str = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let cell = |id: &str, code: &str| Cell { id: id.into(), code: code.into() };
        let sent = vec![
            Attachment::Cells { notebook: NB.into(), cells: vec![cell("c1", "rates = map(fit, runs)")], ask: CellAsk::About },
            Attachment::Error { notebook: NB.into(), cell: cell("c2", "fit = curve_fit(model, t, y, p0)"), text: "BoundsError: attempt to access 3-element Vector".into() },
            Attachment::Selection { notebook: NB.into(), cell: cell("c1", "rates = map(fit, runs)"), text: "map(fit, runs)".into() },
            Attachment::Region { notebook: NB.into(), cells: vec![cell("c3", "scatter(t, y)"), cell("c4", "")], png: Arc::new(vec![0x89, b'P', b'N', b'G', 1, 2, 3]) },
            Attachment::Image { name: "image.png".into(), mime: "image/png", bytes: Arc::new(b"gel".to_vec()) },
            Attachment::Saved { path: "data/decay (2).csv".into() },
            // The adapter echoes a text file's contents after everything else.
            Attachment::Text { name: "notes.txt".into(), text: "t,y\n1,2".into() },
        ];
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old chat".into());
        for event in as_replayed(crate::attach::prompt_blocks("why do these bunch up?", &sent, &[])) {
            s.apply(event);
        }
        let [Entry::User { text, attachments, .. }] = s.entries.as_slice() else { panic!("one user entry") };
        assert_eq!(text.as_ref(), "why do these bunch up?");
        assert_eq!(attachments, &sent);
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
        // Sessions from before the bridge was named `notebook` replay `mcp__pluto__` names.
        for title in ["mcp__notebook__open_notebook", "mcp__pluto__open_notebook"] {
            let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old".into());
            s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", title))));
            let output = serde_json::json!([{ "type": "text", "text": "{\"notebook_id\":\"old-id\",\"path\":\"/tmp/a.jl\"}" }]);
            let done = ToolCallUpdate::new("t1", ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(output));
            let effects = s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(done)));
            assert!(effects.iter().all(|e| !matches!(e, Effect::ShowNotebook { .. })), "no stale navigation");
            let effects = s.started(Started::new(SessionId::new("abc"), None, None));
            assert!(matches!(effects.first(), Some(Effect::ReopenNotebook(p)) if p == "/tmp/a.jl"), "{title}");
        }
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
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", "mcp__notebook__new_notebook"))));
        let effects = s.apply(tool_update("{\"notebook_id\":\"n1\",\"path\":\"/tmp/fit.jl\",\"created\":true}"));
        assert!(matches!(effects.as_slice(), [Effect::ShowNotebook { id, path: Some(p) }] if id == "n1" && p == "/tmp/fit.jl"));

        // A refused open (one notebook per session) shows nothing.
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", "mcp__notebook__open_notebook"))));
        let effects = s.apply(tool_update("{\"error\":\"one_notebook\",\"message\":\"This session works on one notebook\"}"));
        assert!(effects.is_empty());
    }

    #[test]
    fn titles_come_from_the_users_words_not_the_apps_context() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, SessionInfoUpdate, TextContent};
        let context = "[Endeavor] The user started this session on the Pluto notebook /tmp/a.jl, which is open";
        let info = |t: &str| SessionEvent::Update(SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(t.to_string())));
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.submit(Queued::new("plot the growth curves".into(), Vec::new(), Vec::new()), false);
        assert_eq!(s.title, "plot the growth curves");
        s.apply(info(context));
        assert_eq!(s.title, "plot the growth curves", "the context note is not a title");
        s.apply(info("Growth curve plots"));
        assert_eq!(s.title, "Growth curve plots");

        // Reopened with only a stand-in title: the first replayed message names it.
        let chunk = |t: &str| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(t)))));
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "a.jl".into());
        s.untitled = true;
        s.apply(chunk(context));
        assert_eq!(s.title, "a.jl");
        s.apply(chunk("fit the model"));
        s.apply(chunk("and plot it"));
        assert_eq!(s.title, "fit the model");
        assert_eq!(super::agent_title(context), None);
        assert_eq!(super::agent_title("  "), None);
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
    fn the_working_line_says_what_the_running_call_does() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind};
        use serde_json::json;
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("hi"), false);
        assert_eq!(s.activity(), ("Working".into(), None));
        s.apply(SessionEvent::Update(SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::from("hmm")))));
        assert_eq!(s.activity(), ("Thinking".into(), None));
        let call = |id: &str, title: &str, kind, input| {
            SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new(id.to_string(), title.to_string()).kind(kind).raw_input(input)))
        };
        s.apply(call("t1", "Read /data/data.csv", ToolKind::Read, json!({"file_path": "/data/data.csv"})));
        assert_eq!(s.activity(), ("Reading".into(), Some("data.csv".into())));
        s.apply(call("t2", "mcp__notebook__add_cell", ToolKind::Other, json!({"code": "residuals = y .- ŷ"})));
        assert_eq!(s.activity(), ("Adding".into(), Some("residuals".into())));
        s.apply(call("t3", "mcp__notebook__add_cell", ToolKind::Other, json!({"code": "md\"# Fit\""})));
        assert_eq!(s.activity(), ("Adding a cell".into(), None));
        s.apply(call("t4", "ls", ToolKind::Execute, json!({"command": "ls"})));
        assert_eq!(s.activity(), ("Running a command".into(), None));
        s.apply(call("t5", "ToolSearch", ToolKind::Other, json!({"query": "fetch"})));
        assert_eq!(s.activity(), ("Working".into(), None));
        let done = |id: &str| SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id.to_string(), ToolCallUpdateFields::new().status(ToolCallStatus::Completed))));
        s.apply(done("t5"));
        s.apply(done("t4"));
        assert_eq!(s.activity(), ("Adding a cell".into(), None), "the latest call still running");
        assert_eq!(super::elapsed(12), "12s");
        assert_eq!(super::elapsed(65), "1m 05s");
    }

    #[test]
    fn thinking_says_how_long_it_took_once_something_follows() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk};
        use std::time::Duration;
        let thought = |s: &Session, ix: usize| match &s.entries[ix] {
            Entry::Thought { started, took, .. } => (started.is_some(), *took),
            _ => panic!("not thinking"),
        };
        let think = SessionEvent::Update(SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::from("hmm"))));
        let reply = || SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::from("Done."))));
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("hi"), false);
        s.apply(think);
        let ix = s.entries.len() - 1;
        assert_eq!(thought(&s, ix), (true, None), "still thinking");
        s.apply(reply());
        assert!(thought(&s, ix).1.is_some(), "the reply ends it");

        assert_eq!(super::thought_label(false, None, Some(Duration::from_millis(8_400))), "Thought for 8s");
        assert_eq!(super::thought_label(false, None, Some(Duration::from_millis(200))), "Thought for 1s");
        assert_eq!(super::thought_label(false, Some(std::time::Instant::now()), None), "Thinking");
        assert_eq!(super::thought_label(true, None, Some(Duration::from_secs(8))), "Thinking", "inside a folded run");
        assert_eq!(super::thought_label(false, None, None), "Thought", "replayed");
    }

    #[test]
    fn a_message_sent_now_says_it_joined_or_stopped_the_work() {
        use agent_client_protocol::schema::v1::{Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus};
        let delivery = |s: &Session| match s.entries.last() {
            Some(Entry::User { delivery, .. }) => super::delivery_note(*delivery),
            _ => panic!("a user message last"),
        };
        let notes = |s: &Session| s.entries.iter().filter(|e| matches!(e, Entry::Note(_))).count();
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("fit the data"), false);
        let plan = || SessionUpdate::Plan(Plan::new(vec![PlanEntry::new("fit", PlanEntryPriority::Medium, PlanEntryStatus::InProgress)]));
        s.apply(SessionEvent::Update(plan()));

        assert!(matches!(s.submit(text("use log scale"), true).as_slice(), [Effect::Send(Turn::SendNow(_))]));
        s.apply(SessionEvent::Steered);
        assert_eq!(delivery(&s), Some("Claude got this while working"));
        s.apply(SessionEvent::Update(plan()));
        assert_eq!(s.entries.iter().filter(|e| matches!(e, Entry::Plan(_))).count(), 1, "the joined message is part of the same turn");

        s.submit(text("stop, plot it instead"), true);
        assert!(s.apply(SessionEvent::Stopping).is_empty(), "waits for the stopped turn to end");
        let effects = s.apply(SessionEvent::TurnEnded(StopReason::Cancelled));
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]));
        assert_eq!(delivery(&s), Some("Stopped Claude's work to send this"));
        assert_eq!(notes(&s), 0, "no separate note for the stop");
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
    fn a_run_of_tool_calls_opens_and_folds_by_its_first_call() {
        use agent_client_protocol::schema::v1::{ContentChunk, TextContent, ToolCall};
        use agent_client_protocol::schema::v1::ContentBlock;
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("hi"), false);
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", "Read a.rs"))));
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t2", "Read b.rs"))));
        let reply = ContentChunk::new(ContentBlock::Text(TextContent::new("Done.")));
        s.apply(SessionEvent::Update(SessionUpdate::AgentMessageChunk(reply)));
        assert_eq!(crate::runs::run_at(&s.entries, 2), Some(1..3));
        s.toggle_run(1);
        assert!(s.open_runs.contains(&"t1".to_string().into()));
        s.toggle_run(1);
        assert!(s.open_runs.is_empty());
        s.toggle_run(3);
        assert!(s.open_runs.is_empty(), "a message is not a run");
    }

    #[test]
    fn going_idle_asks_for_a_run_state_check() {
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("hi"), false);
        let effects = s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(matches!(effects.as_slice(), [Effect::CheckRunState]));
    }

    #[test]
    fn the_run_state_note_follows_the_runtime() {
        use crate::pluto;
        use serde_json::json;
        let list = |running: serde_json::Value| json!([{ "path": "/n/slow.jl", "pending_run": running.clone(), "running": running, "execution_allowed": true }]);
        let shown = |s: &Session| -> Vec<String> {
            s.entries.iter().flat_map(|e| match e {
                Entry::RunState(warnings) => warnings.iter().map(ToString::to_string).collect(),
                _ => Vec::new(),
            }).collect()
        };
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.note_run_state(pluto::run_warnings(&list(json!(["a", "b"]))));
        assert_eq!(shown(&s), ["2 cells in slow.jl are still running"]);

        assert!(!s.refresh_run_state(&list(json!(["a", "b"]))), "nothing new");
        assert!(s.refresh_run_state(&list(json!(["b"]))));
        assert_eq!(shown(&s), ["1 cell in slow.jl is still running"]);
        assert!(s.refresh_run_state(&list(json!([]))));
        assert!(shown(&s).is_empty());
        assert!(!s.refresh_run_state(&list(json!(["a"]))), "a later run doesn't bring it back");
        assert!(shown(&s).is_empty());
    }
}
