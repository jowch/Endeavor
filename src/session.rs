//! One chat session: its transcript, message queue, cell-code memory, execution
//! "stop asking" state, and the notebook it was last looking at. Agent events are
//! applied here; anything that needs the workspace (sending to the agent, driving
//! the notebook pane, checking run state) comes back as an [`Effect`].

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use agent_client_protocol::Responder;
use agent_client_protocol::schema::MaybeUndefined;
use agent_client_protocol::schema::v1::{
    AvailableCommand, ContentBlock, PermissionOption, PermissionOptionKind, PlanEntry,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigSelectOption, SessionConfigSelectOptions,
    SessionConfigValueId, SessionId,
    SessionModeId, SessionModeState, SessionUpdate, StopReason, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use gpui::*;
use gpui_component::text::TextViewState;

use crate::attach::{self, Attachment, Part, Quote, Quoted};
use crate::agent::{SessionEvent, Started, Turn};
use crate::celldiff::{self, CellCodes};
use crate::hosts::Place;
use crate::permits::{Asked, Asks, Rule};
use crate::pluto;
use crate::motion;
use crate::runs;
use crate::transcript_copy::{self, Below, Swap};
use crate::outbox::{Copying, Delivery, Dispatch, Outbox, Paused, Queued, Shown};

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
        /// Code the call would run, when known; for a write over an existing
        /// file on This Mac, the file's text when the prompt came (its card shows the change).
        code: Option<String>,
        options: Vec<PermissionOption>,
        /// Who waits for the answer, until it's given.
        responder: Option<Asker>,
        /// A notebook call that runs code (a run card).
        runs_code: bool,
        /// The notebook tool and its input, for the run card.
        tool: Option<String>,
        input: serde_json::Value,
        /// The call's kind and the first file it touches, as the agent says.
        kind: Option<ToolKind>,
        path: Option<PathBuf>,
        /// What the run would run, once the runtime answers.
        preview: Option<pluto::RunPreview>,
        /// Plan mode's end: the plan to approve (markdown).
        plan: Option<String>,
    },
    Note(SharedString),
    /// What a turn left unrun or still running, cut back as the runtime reports
    /// progress: it shows only what is still true, and nothing once all is.
    RunState(Vec<pluto::RunWarning>),
    /// The end-of-turn card: the cells the turn changed.
    Changes(Vec<celldiff::ChangedCell>),
    /// A turn that didn't finish: a card where the reply would be, or the
    /// quiet note after Claude restarted under it.
    Failed(Failed),
    /// After a reopened session's history: the faint "Reopened today at 9:14" line.
    Reopened(SystemTime),
}

/// Who asked, and so who hears the answer: the agent's own permission
/// request, or the runtime holding a call (its ask's id): a run in Ask to
/// run, a run or edit in Manual.
pub enum Asker {
    Agent(Responder<RequestPermissionResponse>),
    Runtime(u64),
}

/// The option ids of the cards the app makes for the runtime's asks.
pub const RUN_ALLOW: &str = "allow";
pub const RUN_DENY: &str = "deny";

/// A turn that didn't finish, and the one thing to do about it.
pub struct Failed {
    pub kind: FailedKind,
    /// The error as Claude Code gave it, under Details.
    pub raw: Option<String>,
    pub details_open: bool,
    /// Its Try again or Continue was pressed: a Try again card goes (the
    /// message is answered under it), a Continue button goes.
    pub used: bool,
}

pub enum FailedKind {
    /// No reply came. Try again sends the kept message again: its blocks, and
    /// its bubble, to mark while it waits.
    NoAnswer { reason: &'static str, message: Option<(Option<usize>, Vec<ContentBlock>)> },
    /// The reply stopped partway; what came stays, and Continue picks it up.
    Partway { reason: &'static str },
    /// Claude's process stopped under this reply and was restarted.
    CutOff,
}

impl FailedKind {
    pub fn title(&self) -> &'static str {
        match self {
            FailedKind::NoAnswer { .. } => "Claude couldn't answer",
            FailedKind::Partway { .. } => "Claude stopped before finishing",
            FailedKind::CutOff => "Claude restarted. This reply was cut off.",
        }
    }

    pub fn body(&self) -> String {
        match self {
            FailedKind::NoAnswer { reason, .. } => format!("{reason} Your message is kept."),
            FailedKind::Partway { reason } => (*reason).to_owned(),
            FailedKind::CutOff => String::new(),
        }
    }

    /// Its button: Try again, or Continue.
    pub fn action(&self) -> &'static str {
        match self {
            FailedKind::NoAnswer { .. } => "Try again",
            FailedKind::Partway { .. } | FailedKind::CutOff => "Continue",
        }
    }
}

/// What Continue sends, as a message of its own.
pub const CONTINUE: &str = "Continue from where you stopped.";

/// Why a reply stopped partway, for a turn that ended at one of Claude Code's limits.
fn partway_reason(reason: StopReason) -> Option<&'static str> {
    match reason {
        StopReason::MaxTokens => Some("The reply reached its length limit."),
        StopReason::MaxTurnRequests => Some("Claude took too many steps in one go."),
        _ => None,
    }
}

/// The answer to a call's prompt, shown on its row.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Approval {
    Allowed,
    /// "Always this session": allowed, and later matching prompts in this
    /// session are answered without a card (or, for runs, the session went to Auto).
    ForSession,
    /// "Always in this folder": the agent keeps the rule in the folder's settings.
    InFolder,
    Denied,
    /// The session runs without asking (Auto), so it didn't ask.
    WithoutAsking,
    /// A plan approved with Start, Start in Auto, or sent back with Keep planning.
    Started,
    StartedInAuto,
    KeptPlanning,
}

impl Approval {
    pub fn label(self) -> &'static str {
        match self {
            Approval::Allowed => "allowed",
            Approval::ForSession => "allowed for this session",
            Approval::InFolder => "allowed in this folder",
            Approval::Denied => "denied",
            Approval::WithoutAsking => "ran without asking",
            Approval::Started => "started",
            Approval::StartedInAuto => "started in Auto",
            Approval::KeptPlanning => "kept planning",
        }
    }
}

/// How far an answer that allows reaches.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scope {
    Once,
    /// "Always this session": Endeavor remembers it (for runs: the session goes to Auto).
    Session,
    /// "Always in this folder": the agent's own lasting rule.
    Folder,
}

/// Work a session hands back to the workspace.
pub enum Effect {
    /// A prompt waits for the user (its entry): a notification if Endeavor is in the background.
    Asked(usize),
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
    /// Ask the runtime for a notebook call's result, which the agent didn't
    /// pass on (`Session::result_fetched` takes it).
    FetchResult { call: ToolCallId, tool: String, input: serde_json::Value },
    /// Answer a call the runtime holds (`ask`), with the cells the user's own
    /// run reached meanwhile (each with its `last_run` before that run).
    AnswerRun { ask: u64, allow: bool, user_ran: Vec<(String, f64)> },
    /// Tell the runtime this session's policy changed ("plan" | "ask" |
    /// "auto"), and whether its edits ask too (Manual).
    SetPolicy(&'static str, bool),
    /// A turn failed for want of sign-in: Claude is signed out.
    SignedOut,
    /// A turn hit the account's usage limit: messages wait until it resets.
    UsageLimit(Option<crate::trouble::Reset>),
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
    pub(crate) heard: Option<Instant>,
    /// A user message Claude couldn't answer (signed out, offline); it goes
    /// again by itself once Claude can be reached.
    pub unanswered: Option<usize>,
    /// The pinned plan (above the composer) is folded.
    pub plan_folded: bool,
    /// The queued message opened in place (by its id).
    pub queue_open: Option<u64>,
    /// The queue shows all its rows, not the first three.
    pub queue_all: bool,
    /// The last turn stopped with an error (the sidebar's warning mark);
    /// cleared when a new turn starts.
    pub errored: bool,
    /// A turn ended while the user was in another session (the sidebar's
    /// new-reply dot); cleared when the session is opened.
    pub unseen: bool,
    /// The last /compact: when, and how full the context was before it.
    pub summarized: Option<(SystemTime, Option<u64>)>,
    /// The message just copied from its Copy button, which shows a tick for a moment.
    pub copied: Option<usize>,
    /// The plan card shows the whole plan, not just its steps.
    pub plan_open: bool,
    /// Runs of tool calls the user opened, by their first call.
    pub(crate) open_runs: HashSet<ToolCallId>,
    /// Reopening a past session: its history is replaying.
    replaying: bool,
    /// Reopening from Endeavor's copy of the transcript: `entries` shows the
    /// copy, and the agent's replay gathers here until it has all arrived.
    replay: Option<Vec<Entry>>,
    /// What the replay left below a view it kept in place, for the floating
    /// button. While it's set the list doesn't follow its end, until the
    /// button is pressed or the user scrolls to the end (`reached_end`).
    pub below: Cell<Option<transcript_copy::Below>>,
    /// The user has scrolled the list's last entry into view.
    pub reached_end: Rc<Cell<bool>>,
    /// Eases the list to its end as content arrives (`motion::Glide`).
    pub glide: RefCell<motion::Glide>,
    /// Entries that arrived live in the last moments, by index, so they fade
    /// in; replayed history shows at once.
    arrivals: RefCell<Vec<(usize, Instant)>>,
    /// When reopening began, for the wait line's time.
    pub opening_since: Option<Instant>,
    /// Loading the session again after Claude's process restarted: its history
    /// replays, and the transcript already has it.
    reloading: bool,
    /// Claude's process stopped while this session's reply came: once it's
    /// back, the transcript says the reply was cut off.
    cut_off: bool,
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
    /// A /compact is running: the context used before it, for the note after.
    compacting: Option<Option<(u64, u64)>>,
    /// The policy last sent to the runtime, and whether edits ask.
    policy_sent: (&'static str, bool),
    /// Whether its host's runtime is from another Endeavor build
    /// (`older_runtime`); none until the runtime says which.
    pub runtime_older: Option<bool>,
    /// The older runtime's note is still to show (after the history, for a reopened session).
    older_note: bool,
    /// The runtime's asks this session has shown or answered, so none shows twice.
    seen_asks: HashSet<u64>,
    /// Work for the workspace from an answer given outside `apply` (a click,
    /// a key, the user's own run): taken with `take_later`.
    later: Vec<Effect>,
    /// On a cluster: what its job asks for (from the resources chip).
    pub resources: Option<wire::slurm::Resources>,
    /// The mode to put the agent in once it is up: the new-session screen's pick,
    /// or the mode a reopened session was last in. The agent itself starts every
    /// session, new or reopened, in its settings' default mode.
    pub start_mode: Option<Mode>,
    /// The sidebar row's Tab-stop handle. Lazily created (no `App` is available
    /// in `Session::new`'s many test call sites) and cached, so it stays stable.
    pub focus: RefCell<Option<FocusHandle>>,
    /// The agent's prompts as Endeavor answers them: "Always this session"
    /// rules, the queued prompts' count, when each was answered.
    pub asks: Asks,
    /// Of each waiting run card's cells (by its call), those that went from
    /// unrun to run some other way while it waited.
    ran_while_asked: HashMap<String, HashSet<String>>,
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
    /// A changed-cells card's row, by its entry index and row number.
    changed_cell_focus: RefCell<HashMap<(usize, usize), FocusHandle>>,
    /// A queued message's buttons (↑ ✎ ✕ ⌃, and Undo once removed), by the message's id and the button.
    queue_focus: RefCell<HashMap<(u64, &'static str), FocusHandle>>,
    /// Each reply's parsed markdown, by its entry index (see `reply_text`).
    replies: RefCell<HashMap<usize, (Entity<TextViewState>, Rc<Cell<usize>>, Subscription)>>,
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

/// Starting or reopening a session failed: the page in place of its transcript.
pub struct Failure {
    pub kind: OpenFailure,
    /// The agent's error, under Details.
    pub raw: String,
    pub details_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OpenFailure {
    /// It's open in Claude Code in a terminal: open a copy here, or close it there and try again.
    InCli,
    /// Anything else, with a plain reason when Endeavor can tell.
    Other(Option<&'static str>),
}

/// Why a session couldn't open, from the agent's error (which may wrap the
/// CLI's stderr in JSON).
pub fn open_failure(error: &str) -> OpenFailure {
    if error.contains("running as a background session") || error.contains("claude attach") {
        return OpenFailure::InCli;
    }
    let lower = error.to_lowercase();
    let reason = if ["json", "unexpected token", "unexpected end", "parse error"].iter().any(|w| lower.contains(w)) {
        Some("Its history couldn't be read.")
    } else if ["enoent", "no such file or directory", "does not exist"].iter().any(|w| lower.contains(w)) {
        Some("Its folder isn't there any more.")
    } else if ["connection closed", "channel closed", "broken pipe", "agent connection"].iter().any(|w| lower.contains(w)) {
        Some("Claude isn't running.")
    } else if lower.contains("not found") {
        Some("Claude Code no longer has its history.")
    } else {
        None
    };
    OpenFailure::Other(reason)
}

impl Failure {
    pub fn title(&self, new: bool) -> &'static str {
        match self.kind {
            OpenFailure::InCli => "This session is open in Claude Code",
            OpenFailure::Other(_) if new => "Couldn't start this session",
            OpenFailure::Other(_) => "Couldn't open this session",
        }
    }

    /// The page's words, which say where the session's notebook stands.
    pub fn body(&self, notebook: Beside) -> String {
        match self.kind {
            OpenFailure::InCli => "It's running in a terminal. Close it there, then Try again. Or open a copy here: it has the \
                                   conversation so far, and the two go separate ways after that."
                .into(),
            OpenFailure::Other(reason) => {
                let why = reason.unwrap_or("Claude Code couldn't load it.");
                match notebook {
                    Beside::Open => format!("{why} The notebook is open beside it."),
                    Beside::Fine => format!("{why} The notebook and its file are fine."),
                    Beside::Unknown => format!("{}, so Endeavor doesn't know its notebook yet.", why.trim_end_matches('.')),
                    Beside::Nothing => why.to_owned(),
                }
            }
        }
    }
}

/// Where a session that couldn't open stands with its notebook.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Beside {
    /// It's open in the pane.
    Open,
    /// Its file is there, though it isn't open.
    Fine,
    /// A past session with no notebook in Endeavor's record: only its history knows it.
    Unknown,
    /// None, or its file is gone.
    Nothing,
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
        let reached_end = Rc::new(Cell::new(false));
        let list = ListState::new(0, ListAlignment::Top, px(1000.));
        list.set_follow_mode(FollowMode::Tail);
        let reached = reached_end.clone();
        list.set_scroll_handler(move |event, _, _| {
            if event.visible_range.end >= event.count {
                reached.set(true);
            }
        });
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
            list,
            list_len: Cell::new(0),
            dirty_from: Cell::new(None),
            busy_since: None,
            turn_entry: None,
            heard: None,
            unanswered: None,
            plan_folded: false,
            queue_open: None,
            queue_all: false,
            errored: false,
            unseen: false,
            summarized: None,
            copied: None,
            plan_open: false,
            open_runs: HashSet::new(),
            replaying: false,
            replay: None,
            below: Cell::new(None),
            reached_end,
            glide: RefCell::new(motion::Glide::default()),
            arrivals: RefCell::new(Vec::new()),
            opening_since: None,
            reloading: false,
            cut_off: false,
            replayed_path: None,
            failed: None,
            modes: None,
            config: Vec::new(),
            usage: None,
            commands: Vec::new(),
            compacting: None,
            policy_sent: ("ask", false),
            runtime_older: None,
            older_note: false,
            seen_asks: HashSet::new(),
            later: Vec::new(),
            resources: None,
            start_mode: None,
            focus: RefCell::new(None),
            asks: Asks::default(),
            ran_while_asked: HashMap::new(),
            approval_focus: RefCell::new(Vec::new()),
            pinned_plan_focus: RefCell::new(None),
            plan_card_focus: RefCell::new(None),
            run_focus: RefCell::new(HashMap::new()),
            row_focus: RefCell::new(HashMap::new()),
            changed_cell_focus: RefCell::new(HashMap::new()),
            queue_focus: RefCell::new(HashMap::new()),
            replies: RefCell::new(HashMap::new()),
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

    /// A changed-cells card's row's Tab-stop handle, by the card's entry index and row number.
    pub fn changed_cell_focus(&self, ix: usize, row: usize, cx: &App) -> FocusHandle {
        self.changed_cell_focus.borrow_mut().entry((ix, row)).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    /// A queued message's button's Tab-stop handle, by the message's id and the button's name.
    pub fn queue_focus(&self, id: u64, button: &'static str, cx: &App) -> FocusHandle {
        self.queue_focus.borrow_mut().entry((id, button)).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    /// The parsed markdown of the reply at entry `ix`, kept for as long as the
    /// session. A `TextView`'s own state lasts only while it is drawn, and a
    /// reply over 4 KB parses in the background, so a reply drawn from new
    /// state is measured empty and then grows by its whole height, moving the
    /// transcript under the reader. When a parse lands, the reply's list item
    /// is measured again, even while it is off screen.
    pub fn reply_text(&self, ix: usize, text: &str, cx: &mut App) -> Entity<TextViewState> {
        let existing = self.replies.borrow().get(&ix).map(|(state, ..)| state.clone());
        let state = existing.unwrap_or_else(|| {
            let state = cx.new(|cx| TextViewState::markdown(text, cx));
            let list = self.list.clone();
            // Its entry can move when a replay replaces Endeavor's copy (`keep_replies`).
            let at = Rc::new(Cell::new(ix));
            let entry = at.clone();
            let remeasure = cx.observe(&state, move |_, _| list.remeasure_items(entry.get()..entry.get() + 1));
            self.replies.borrow_mut().insert(ix, (state.clone(), at, remeasure));
            state
        });
        state.update(cx, |state, cx| state.set_text(text, cx));
        state
    }

    /// Where the reply at entry `ix` was last drawn, if it has been.
    pub fn reply_bounds(&self, ix: usize, cx: &App) -> Option<Bounds<Pixels>> {
        self.replies.borrow().get(&ix).map(|(state, ..)| state.read(cx).bounds())
    }

    /// The first reply with text selected in it: its entry, and the text.
    pub fn selected_reply(&self, cx: &App) -> Option<(usize, String)> {
        let replies = self.replies.borrow();
        let mut selected: Vec<(usize, String)> = replies.iter().map(|(ix, (state, ..))| (*ix, state.read(cx).selected_text())).filter(|(_, text)| !text.trim().is_empty()).collect();
        selected.sort_by_key(|(ix, _)| *ix);
        selected.into_iter().next().map(|(ix, text)| (ix, text.trim().to_string()))
    }

    /// Starting or reopening failed: stop looking busy and say why.
    pub fn fail(&mut self, error: &str) {
        self.failed = Some(Failure { kind: open_failure(error), raw: error.trim().to_owned(), details_open: false });
        self.outbox.busy = false;
        self.busy_since = None;
        self.replaying = false;
        self.opening_since = None;
        self.reloading = false;
    }

    /// Open a session that couldn't be reopened (e.g. open in the CLI) as a
    /// copy: the transcript refills from the copy's replay, and messages sent
    /// meanwhile go once it's open.
    pub fn reopen_as_copy(&mut self) -> Option<SessionId> {
        self.failed.take()?;
        self.clear_for_replay();
        self.title = format!("{} (copy)", self.title);
        self.id.take()
    }

    /// Try again on a session that couldn't open: it starts, or loads its
    /// history, again. Messages sent meanwhile wait for it.
    pub fn retry_open(&mut self) {
        if self.failed.take().is_none() {
            return;
        }
        if self.id.is_some() {
            self.clear_for_replay();
        }
        self.outbox.busy = true;
        self.agent_waiting = true;
    }

    /// The history starts loading over: the replay so far goes (Endeavor's copy stays on show).
    fn clear_history(&mut self) {
        match &mut self.replay {
            Some(replay) => replay.clear(),
            None => {
                self.entries.clear();
                self.replies.get_mut().clear();
                self.open_runs.clear();
                self.mark(0);
            }
        }
    }

    fn clear_for_replay(&mut self) {
        self.clear_history();
        self.outbox = Outbox::waiting();
        self.replaying = true;
        self.opening_since = Some(Instant::now());
    }

    pub fn toggle_failure_details(&mut self) {
        if let Some(failure) = &mut self.failed {
            failure.details_open = !failure.details_open;
        }
    }

    /// A past session being reopened: its id is known up front so the replayed
    /// history (which arrives before the load completes) lands here.
    pub fn loading(key: u64, id: SessionId, place: Place, server: Option<String>, title: String) -> Self {
        let mut session = Self::new(key, place, server);
        session.id = Some(id);
        session.title = title;
        session.replaying = true;
        session.opening_since = Some(Instant::now());
        session
    }

    /// Show Endeavor's copy of this session's transcript while its history
    /// loads: read-only, at its end. The replay replaces it once it's all in.
    pub fn show_copy(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.replay = Some(Vec::new());
        self.mark(0);
        self.list.set_follow_mode(FollowMode::Tail);
    }

    /// Reopening, with Endeavor's copy of the transcript showing meanwhile.
    pub fn showing_copy(&self) -> bool {
        self.opening() && self.replay.is_some()
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
    /// (Claude's plan mode only restricts its own tools, not ours); otherwise
    /// runs ask first unless the session runs without asking.
    pub fn policy(&self) -> &'static str {
        match &self.modes {
            Some(m) if m.current_mode_id.to_string() == "plan" => "plan",
            _ if self.run_without_asking => "auto",
            _ => "ask",
        }
    }

    /// Whether every change to the notebook asks first (Manual, Claude's own
    /// default mode), whatever the agent's own settings allow: the runtime
    /// holds those calls too.
    pub fn edits_ask(&self) -> bool {
        self.modes.as_ref().is_some_and(|m| m.current_mode_id.to_string() == "default")
    }

    /// Whether the runtime holds a call to notebook tool `tool` with `input`
    /// for the user's answer, by the session's mode.
    fn runtime_holds(&self, tool: &str, input: &serde_json::Value) -> bool {
        endeavor_mcp::asks_first(tool, input, self.policy(), self.edits_ask())
    }

    /// The session's mode as the rules for an older runtime name it
    /// (`older_runtime::refusal`): "manual" when the agent asks before each
    /// change itself (Claude's own default mode), else the runtime policy.
    pub fn guard_mode(&self) -> &'static str {
        match (&self.modes, self.policy()) {
            (Some(m), "ask") if m.current_mode_id.to_string() == "default" => "manual",
            (_, policy) => policy,
        }
    }

    /// Its host's runtime said which build it is: from another build than
    /// the app's (`older`) or not. An older one gets a note, once.
    pub fn runtime_build(&mut self, older: bool) {
        if self.runtime_older == Some(older) {
            return;
        }
        self.runtime_older = Some(older);
        self.older_note = older;
        self.note_older_runtime();
    }

    fn note_older_runtime(&mut self) {
        if self.older_note && !self.replaying {
            self.older_note = false;
            let host = self.server.clone().unwrap_or_else(|| crate::platform::this_computer!().into());
            self.note(crate::older_runtime::note(&host));
        }
    }

    fn sync_policy(&mut self, effects: &mut Vec<Effect>) {
        let policy = (self.policy(), self.edits_ask());
        if policy != self.policy_sent {
            self.policy_sent = policy;
            effects.push(Effect::SetPolicy(policy.0, policy.1));
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
        if !self.replaying {
            self.arrivals.borrow_mut().push((self.entries.len(), Instant::now()));
        }
        self.entries.push(entry);
    }

    /// When entry `ix` arrived, while it is still fading in.
    pub fn arrival(&self, ix: usize) -> Option<Instant> {
        let mut arrivals = self.arrivals.borrow_mut();
        let fade = crate::theme::motion_fast();
        arrivals.retain(|(_, at)| at.elapsed() < fade);
        arrivals.iter().find(|(i, _)| *i == ix).map(|(_, at)| *at)
    }

    /// Show what has arrived in place, as when the session is shown again.
    pub fn settle_arrivals(&self) {
        self.arrivals.borrow_mut().clear();
    }

    /// The list follows its end: by itself, or held by its glide.
    pub fn following(&self) -> bool {
        self.list.is_following_tail() || self.glide.borrow().holding()
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
    /// Entries that changed in place are measured again, which keeps the
    /// view's place in them; a splice over the view's first entry would put
    /// the view back to that entry's top.
    pub fn sync_list(&self) {
        let Some(from) = self.dirty_from.take() else { return };
        let (old, new) = (self.list_len.get(), self.entries.len());
        let from = from.min(old);
        if new >= old {
            if from < old {
                self.list.remeasure_items(from..old);
            }
            self.list.splice(old..old, new - old);
        } else {
            self.list.splice(from..old, new - from);
        }
        self.list_len.set(new);
    }

    /// Where the session stands with its notebook, for the page when it couldn't open.
    pub fn notebook_beside(&self) -> Beside {
        match (&self.notebook_path, self.missing) {
            (Some(_), false) if self.notebook.is_some() => Beside::Open,
            (Some(_), false) => Beside::Fine,
            (None, _) if self.id.is_some() => Beside::Unknown,
            _ => Beside::Nothing,
        }
    }

    /// A past session whose history hasn't loaded yet: the chat shows its
    /// summary, and messages wait for it.
    pub fn opening(&self) -> bool {
        self.replaying && self.failed.is_none()
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
        if let Some(replay) = self.replay.take().filter(|_| self.replaying) {
            self.take_replay(replay);
        } else if self.replaying {
            self.push_changes();
            // The history shows whole, at its end, with a line between it and what comes now.
            if !self.entries.is_empty() {
                self.push(Entry::Reopened(SystemTime::now()));
            }
            self.list.set_follow_mode(FollowMode::Tail);
            self.list.scroll_to_end();
        }
        self.replaying = false;
        self.opening_since = None;
        self.reloading = false;
        if std::mem::take(&mut self.cut_off) {
            self.push(Entry::Failed(Failed { kind: FailedKind::CutOff, raw: None, details_open: false, used: false }));
        }
        self.note_older_runtime();
        let mut effects: Vec<Effect> = self.replayed_path.take().map(Effect::ReopenNotebook).into_iter().collect();
        if let Some(mode) = self.start_mode.take() {
            effects.extend(self.set_mode(&mode));
        }
        self.sync_policy(&mut effects);
        let next = self.outbox.turn_ended();
        self.dispatch(next, &mut effects);
        effects
    }

    /// The replay has all arrived: it replaces the copy on show, and the view
    /// keeps its place (see `transcript_copy::merge`).
    fn take_replay(&mut self, replay: Vec<Entry>) {
        self.replay = Some(replay);
        // The last turn's card, as a reopen without a copy gets.
        self.in_replay(Session::push_changes);
        let replay = self.replay.take().unwrap_or_default();
        let top = self.list.logical_scroll_top();
        let len = self.entries.len();
        let replies: Vec<(usize, String)> = self.entries.iter().enumerate().filter_map(|(ix, e)| if let Entry::Agent { text, .. } = e { Some((ix, text.clone())) } else { None }).collect();
        let merged = transcript_copy::merge(&mut self.entries, replay, top.item_ix);
        self.reached_end.set(false);
        match merged.swap {
            Swap::Same => {}
            Swap::Added { messages } => {
                // Not even at the end: a reply measures short until its text is laid out.
                self.list.set_follow_mode(FollowMode::Normal);
                self.list.scroll_to(top);
                self.mark(len);
                self.below.set(Some(Below::New(messages)));
            }
            Swap::Changed => {
                self.keep_replies(&replies);
                self.open_runs.clear();
                self.mark(0);
                self.sync_list();
                let offset_in_item = if merged.found { top.offset_in_item } else { px(0.) };
                self.list.set_follow_mode(FollowMode::Normal);
                self.list.scroll_to(ListOffset { item_ix: merged.anchor, offset_in_item });
                self.below.set(Some(Below::Changed));
            }
        }
        // One line between the history and what comes now: a reopen with no turn since moves it.
        match self.entries.last_mut() {
            Some(Entry::Reopened(at)) => {
                *at = SystemTime::now();
                self.mark(self.entries.len() - 1);
            }
            _ => self.push(Entry::Reopened(SystemTime::now())),
        }
    }

    /// The replies laid out from the copy (`old`: their entries and texts)
    /// keep their laid-out text where the same reply is now, so they keep
    /// their height and the view its place.
    fn keep_replies(&mut self, old: &[(usize, String)]) {
        let mut laid_out = std::mem::take(self.replies.get_mut());
        let mut taken = HashSet::new();
        for (ix, entry) in self.entries.iter().enumerate() {
            let Entry::Agent { text, .. } = entry else { continue };
            let Some((was, _)) = old.iter().find(|(was, t)| t == text && !taken.contains(was)) else { continue };
            taken.insert(*was);
            if let Some(reply) = laid_out.remove(was) {
                reply.1.set(ix);
                self.replies.get_mut().insert(ix, reply);
            }
        }
    }

    /// Run `f` on the replay's entries while the copy shows, else on the transcript.
    fn in_replay<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let Some(mut replay) = self.replay.take() else { return f(self) };
        // The list shows the copy: what the replay changes isn't on screen.
        let dirty = self.dirty_from.get();
        std::mem::swap(&mut self.entries, &mut replay);
        let result = f(self);
        std::mem::swap(&mut self.entries, &mut replay);
        self.dirty_from.set(dirty);
        self.replay = Some(replay);
        result
    }

    /// Send or queue a message. Before the session exists everything queues.
    pub fn submit(&mut self, mut message: Queued, now: bool) -> Vec<Effect> {
        if self.failed.is_some() {
            self.note("This session isn't open, so nothing was sent");
            return Vec::new();
        }
        // A command goes alone; the context waits for the next message.
        if !crate::slash::is_command(&message.text)
            && let Some(context) = self.start_context.take()
        {
            message.blocks.insert(0, context);
        }
        self.title_from(&message.text);
        let mut effects = Vec::new();
        // A reopened session has its id while its history loads, but nothing goes until it's open.
        let dispatch = self.outbox.submit(message, now && self.id.is_some() && !self.replaying);
        self.dispatch(dispatch, &mut effects);
        effects
    }

    /// ✎ on queued message `ix`: its words and chips, for the composer.
    pub fn begin_edit(&mut self, ix: usize) -> Option<(String, Vec<Attachment>)> {
        let edit = self.outbox.begin_edit(ix);
        self.queue_open = None;
        edit
    }

    /// Run one of the outbox's queue actions and send what it starts.
    pub fn queue_action(&mut self, act: impl FnOnce(&mut Outbox) -> Option<Dispatch>) -> Vec<Effect> {
        let mut effects = Vec::new();
        let dispatch = act(&mut self.outbox);
        self.dispatch(dispatch, &mut effects);
        effects
    }

    /// Whether queued message `ix` can be sent now (its ↑): only once the session is open.
    pub fn can_send_now(&self, ix: usize) -> bool {
        self.id.is_some() && !self.replaying && !self.agent_waiting && self.outbox.can_send_now(ix)
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
        self.outbox.busy.then_some(Effect::Send(Turn::Cancel)).filter(|_| self.id.is_some() && !self.replaying)
    }

    fn dispatch(&mut self, dispatch: Option<Dispatch>, effects: &mut Vec<Effect>) {
        let Some(Dispatch { turn, shown }) = dispatch else { return };
        let again = shown.is_none() && matches!(turn, Turn::Prompt(_));
        if matches!(turn, Turn::Prompt(_)) {
            self.errored = false;
        }
        effects.push(Effect::Send(turn));
        if let Some(Shown { text, attachments, delivery }) = shown {
            if text.split_whitespace().next() == Some("/compact") {
                self.compacting = Some(self.usage);
            }
            self.push(Entry::User { text: text.into(), expanded: false, attachments, delivery, sent: Some(SystemTime::now()) });
            self.turn_entry = Some(self.entries.len() - 1);
            self.busy_since.get_or_insert_with(Instant::now);
            // Sending jumps back to the bottom even if the user had scrolled up;
            // while following, the view eases there.
            if !self.following() {
                self.list.set_follow_mode(FollowMode::Tail);
            }
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
        if self.replaying {
            return self.in_replay(|s| s.apply_now(event));
        }
        self.apply_now(event)
    }

    fn apply_now(&mut self, event: SessionEvent) -> Vec<Effect> {
        let mut effects = Vec::new();
        // The transcript already has what a reload replays.
        if self.reloading && matches!(event, SessionEvent::Update(_)) {
            return effects;
        }
        match event {
            SessionEvent::TurnEnded(reason) => {
                self.push_changes();
                let stopped_for_next = reason == StopReason::Cancelled && self.outbox.stopping();
                if let Some(reason) = partway_reason(reason) {
                    self.push(Entry::Failed(Failed { kind: FailedKind::Partway { reason }, raw: None, details_open: false, used: false }));
                    self.outbox.pause(Paused::Error);
                    self.errored = true;
                } else if let Some(note) = turn_ended_note(reason).filter(|_| !stopped_for_next) {
                    // Stopped to send the next message: its bubble says so.
                    self.note(note);
                }
                if reason == StopReason::Cancelled && !stopped_for_next {
                    self.outbox.pause(Paused::Stopped);
                }
                if reason != StopReason::Cancelled && !self.replaying {
                    self.unseen = true;
                }
                if let Some(before) = self.compacting.take().filter(|_| reason == StopReason::EndTurn) {
                    self.note(compacted_note(before, self.usage));
                    let percent = before.filter(|(_, size)| *size > 0).map(|(used, size)| used * 100 / size);
                    self.summarized = Some((SystemTime::now(), percent));
                }
                self.drop_prompts();
                self.turn_ended(&mut effects);
            }
            SessionEvent::AuthRequired => {
                self.turn_unanswered();
                effects.push(Effect::SignedOut);
            }
            SessionEvent::TurnFailed { kind, error } => return self.turn_failed(crate::trouble::classify(kind.as_deref(), &error), &error),
            SessionEvent::ConfigFailed(e) => self.note(e),
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
                let (title, input) = self.permission_input(&request);
                // Only runs get the run card ("Always this session"); other notebook
                // prompts (e.g. Manual asking before an edit) get the agent's options.
                let input = input.unwrap_or_default();
                let kind = fields.kind;
                let path = fields.locations.as_ref().and_then(|l| l.first()).map(|l| l.path.clone());
                let runs_code = celldiff::notebook_tool(&title).is_some_and(|tool| endeavor_mcp::runs_code(tool, &input));
                // The adapter's ExitPlanMode prompt: its options carry these ids.
                let plan = request
                    .options
                    .iter()
                    .any(|o| o.option_id.to_string().starts_with("exit-plan-"))
                    .then(|| input["plan"].as_str().unwrap_or("").to_owned());
                let held = plan.is_none() && celldiff::notebook_tool(&title).is_some_and(|tool| self.runtime_holds(tool, &input));
                if let Some(allow) = self.runtime_asks_instead(held, &request.options) {
                    let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(allow.option_id.clone()));
                    let _ = responder.respond(RequestPermissionResponse::new(outcome));
                    return effects;
                }
                let call = request.tool_call.tool_call_id.clone();
                let asked = Asked { title: &title, kind, input: &input, path: path.as_deref() };
                let answered = option_of_kind(&request.options, PermissionOptionKind::AllowOnce).zip(self.answer_on_arrival(&asked, runs_code, plan.is_some()));
                if let Some((allow, approval)) = answered {
                    let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(allow.option_id.clone()));
                    let _ = responder.respond(RequestPermissionResponse::new(outcome));
                    self.approve(&call, approval, &title, &input);
                    return effects;
                }
                let written = (kind == Some(ToolKind::Edit) && input["content"].is_string() && self.place.host == crate::hosts::HostId::ThisMac)
                    .then(|| input["file_path"].as_str().and_then(|f| std::fs::read_to_string(f).ok()))
                    .flatten();
                let code = written.or_else(|| {
                    input["code"].as_str().map(str::to_owned).or_else(|| input["cell_id"].as_str().and_then(|id| self.cell_codes.get(id)).map(str::to_owned))
                });
                let tool = celldiff::notebook_tool(&title).map(str::to_owned);
                if let Some(tool) = tool.clone().filter(|t| runs_code && t != "run_shell") {
                    effects.push(Effect::PreviewRun { ix: self.entries.len(), tool, input: input.clone() });
                }
                if plan.is_some() {
                    self.plan_open = false;
                }
                self.asks.asked(self.waiting_prompts().len());
                self.push(Entry::Permission { call, title, code, options: request.options, responder: Some(Asker::Agent(responder)), runs_code, tool, input, kind, path, preview: None, plan });
                effects.push(Effect::Asked(self.entries.len() - 1));
            }
            SessionEvent::Update(update) => {
                self.heard = Some(Instant::now());
                self.apply_update(update, &mut effects);
            }
            SessionEvent::Config(options) => self.config = options,
        }
        effects
    }

    /// The turn failed. The reply that only repeats the error goes. Held
    /// because Claude is out of reach, signed out or at its usage limit, the
    /// message waits to go again; otherwise a card says why, where the reply
    /// would have been, with Try again, or Continue for a reply that had begun.
    pub fn turn_failed(&mut self, trouble: crate::trouble::Trouble, error: &str) -> Vec<Effect> {
        use crate::trouble::Trouble;
        let said = error.trim().strip_prefix("Internal error: ").unwrap_or(error.trim());
        let last = self.entries.len().saturating_sub(1);
        if let Some(Entry::Agent { text, .. }) = self.entries.last_mut()
            && let Some(before) = text.trim_end().strip_suffix(said)
        {
            let before = before.trim_end().to_owned();
            self.mark(last);
            if before.is_empty() {
                self.entries.pop();
            } else if let Some(Entry::Agent { text, .. }) = self.entries.last_mut() {
                *text = before;
            }
        }
        let mut effects = Vec::new();
        match trouble {
            _ if self.outbox.held => self.turn_unanswered(),
            Trouble::SignIn => {
                self.turn_unanswered();
                effects.push(Effect::SignedOut);
            }
            Trouble::UsageLimit(reset) => {
                self.turn_unanswered();
                effects.push(Effect::UsageLimit(reset));
            }
            trouble => {
                self.push_changes();
                let kind = if trouble == Trouble::Connection && self.replied() {
                    FailedKind::Partway { reason: "The connection to Claude dropped partway through this reply." }
                } else {
                    FailedKind::NoAnswer { reason: trouble.reason(), message: self.outbox.take_current().map(|blocks| (self.turn_entry, blocks)) }
                };
                self.push(Entry::Failed(Failed { kind, raw: Some(said.to_owned()), details_open: false, used: false }));
                self.outbox.pause(Paused::Error);
                self.errored = true;
                self.unseen = true;
                self.turn_ended(&mut effects);
            }
        }
        effects
    }

    /// Claude said or did something in this turn.
    fn replied(&self) -> bool {
        self.entries.iter().skip(self.turn_start() + 1).any(|e| matches!(e, Entry::Agent { .. } | Entry::Tool { .. }))
    }

    /// Try again on the card at `ix`: its message goes again, first, without a
    /// second bubble, and the card goes.
    pub fn try_again(&mut self, ix: usize) -> Vec<Effect> {
        let mut effects = Vec::new();
        let Some(Entry::Failed(failed)) = self.entries.get_mut(ix) else { return effects };
        let FailedKind::NoAnswer { message, .. } = &mut failed.kind else { return effects };
        let Some((bubble, blocks)) = message.take() else { return effects };
        failed.used = true;
        self.mark(ix);
        self.unanswered = bubble;
        if let Some(ix) = bubble {
            self.mark(ix);
        }
        let next = self.outbox.send_again(blocks);
        self.dispatch(next, &mut effects);
        effects
    }

    /// Continue on the card or note at `ix`: a new message asks Claude to pick up where it stopped.
    pub fn continue_reply(&mut self, ix: usize) -> Vec<Effect> {
        let Some(Entry::Failed(failed)) = self.entries.get_mut(ix) else { return Vec::new() };
        if failed.used || matches!(failed.kind, FailedKind::NoAnswer { .. }) {
            return Vec::new();
        }
        failed.used = true;
        self.mark(ix);
        let blocks = vec![ContentBlock::Text(agent_client_protocol::schema::v1::TextContent::new(CONTINUE))];
        self.submit(Queued::new(CONTINUE.into(), Vec::new(), blocks), false)
    }

    pub fn toggle_details(&mut self, ix: usize) {
        if let Some(Entry::Failed(failed)) = self.entries.get_mut(ix) {
            failed.details_open = !failed.details_open;
            self.mark(ix);
        }
    }

    /// Claude's process stopped. A reply under way is cut off (said so once
    /// it's back), the agent's open prompts can't be answered any more, and
    /// the session opens again, in its mode, once Claude is back; until then
    /// messages queue.
    pub fn agent_stopped(&mut self) {
        if self.failed.is_some() {
            return;
        }
        if self.busy_since.is_some() && self.id.is_some() {
            self.push_changes();
            self.cut_off = true;
            self.errored = true;
        }
        self.drop_prompts();
        self.mark(0);
        self.turn_entry = None;
        self.busy_since = None;
        self.end_thought();
        self.outbox.restart();
        self.agent_waiting = true;
        // A load cut short starts over; a loaded transcript stays and its replay is skipped.
        if self.replaying {
            self.clear_history();
        } else {
            self.reloading = self.id.is_some();
        }
        if self.start_mode.is_none() {
            self.start_mode = self.mode();
        }
    }

    /// The end-of-turn card for the turn's edits since it began (or since its
    /// last card), when they changed any cell.
    fn push_changes(&mut self) {
        let last_card = self.entries.iter().rposition(|e| matches!(e, Entry::Changes(_))).map_or(0, |ix| ix + 1);
        let from = last_card.max(self.turn_start());
        let edits = self.entries[from..]
            .iter()
            .flat_map(|e| match e {
                Entry::Tool { diffs, .. } => diffs.as_slice(),
                _ => &[],
            })
            .filter_map(|d| d.edit.as_ref());
        let cells = celldiff::net_changes(edits);
        if !cells.is_empty() {
            self.push(Entry::Changes(cells));
        }
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
                    ContentBlock::Text(t) => match attach::replayed_text_file(&t.text)
                        .or_else(|| attach::replayed_notebook(&t.text))
                        .or_else(|| attach::replayed_quote(&t.text))
                        .or_else(|| attach::replayed_saved_file(&t.text))
                    {
                        Some(attachment) => (None, Some(attachment)),
                        None => match attach::without_agent_blocks(&t.text) {
                            text if text.is_empty() || attach::is_app_text(&text) => return,
                            // Claude Code's own marker for a turn it stopped mid-flight: not
                            // the user's words, so replay shows the same note a live stop does.
                            text if attach::is_stopped_marker(&text) => {
                                self.push_changes();
                                return self.note("You stopped Claude");
                            }
                            text => (Some(text), None),
                        },
                    },
                    ContentBlock::Image(image) => {
                        // A figure's or box's picture follows its quote.
                        if let Some(Entry::User { attachments, .. }) = self.entries.last_mut()
                            && let Some(Attachment::Quote(Quote { from: Quoted::Cell { part: Part::Figure(png), .. } | Quoted::Box { png, .. }, .. })) = attachments.last_mut()
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
                    _ => {
                        // Replayed history has no turn ends: the next message marks one.
                        self.push_changes();
                        self.push(Entry::User { text: text.unwrap_or_default().into(), expanded: false, attachments: attachment.into_iter().collect(), delivery: Delivery::Turn, sent: None })
                    }
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
            SessionUpdate::ToolCall(mut call) => {
                celldiff::name_notebook_call(&mut call.title, &mut call.raw_input);
                self.push(Entry::Tool {
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
                })
            }
            SessionUpdate::ToolCallUpdate(update) => {
                if let Some((id, path)) = self.on_tool_update(update, effects) {
                    if self.replaying {
                        // History, not a live open: that id belongs to an earlier Julia.
                        // The notebook Endeavor recorded wins over the history's: the
                        // user may have located the file since Claude last opened it.
                        if self.notebook_path.is_none() {
                            self.replayed_path = path.or(self.replayed_path.take());
                        }
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
            _ => {}
        }
    }

    /// The title and input a permission request names. Cursor's request
    /// carries no `rawInput`, only the arguments as a fenced json block in
    /// `content`; the tool_call_update for the same id arrived first, with
    /// `rawInput` already unwrapped by [`celldiff::name_notebook_call`] onto
    /// its recorded [`Entry::Tool`], so a missing or empty input falls back
    /// to that.
    fn permission_input(&self, request: &RequestPermissionRequest) -> (String, Option<serde_json::Value>) {
        let fields = &request.tool_call.fields;
        let (mut title, mut input) = (fields.title.clone().unwrap_or_else(|| "Tool call".into()), fields.raw_input.clone());
        celldiff::name_notebook_call(&mut title, &mut input);
        if input.as_ref().is_none_or(|i| i.as_object().is_some_and(serde_json::Map::is_empty)) {
            input = self.entries.iter().rev().find_map(|e| match e {
                Entry::Tool { id, input: recorded, .. } if *id == request.tool_call.tool_call_id => recorded.clone(),
                _ => None,
            });
        }
        (title, input)
    }

    /// Apply a tool-call update; on a completed notebook call, learn cell code
    /// and diff edits. Returns the id (and file path) of a notebook the agent
    /// just opened or created. A finished notebook call whose output isn't the
    /// tool's result (Cursor sends only `{"success": true}`) asks the runtime
    /// for it instead.
    fn on_tool_update(&mut self, update: ToolCallUpdate, effects: &mut Vec<Effect>) -> Option<(String, Option<String>)> {
        let ix = self.entries.iter().rposition(|e| matches!(e, Entry::Tool { id, .. } if *id == update.tool_call_id))?;
        self.mark(ix);
        let Entry::Tool { id, title, kind, path, status, input, output, .. } = &mut self.entries[ix] else { return None };
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
        celldiff::name_notebook_call(title, input);
        let finished = matches!(*status, ToolCallStatus::Completed | ToolCallStatus::Failed);
        if let Some(tool) = celldiff::notebook_tool(title).filter(|_| finished && !self.replaying)
            && output.as_ref().and_then(celldiff::tool_json).is_none()
            && !runs::denied(output.as_ref())
        {
            effects.push(Effect::FetchResult { call: id.clone(), tool: tool.to_owned(), input: input.clone().unwrap_or_default() });
            return None;
        }
        self.result_arrived(ix)
    }

    /// The result of the notebook call at `ix` is in its output: learn cell
    /// code, diff its edits, and say which notebook it opened or created.
    fn result_arrived(&mut self, ix: usize) -> Option<(String, Option<String>)> {
        let Some(Entry::Tool { title, status, input, output, diffs, .. }) = self.entries.get_mut(ix) else { return None };
        let tool = celldiff::notebook_tool(title).filter(|_| *status == ToolCallStatus::Completed)?;
        let result = output.as_ref().and_then(celldiff::tool_json)?;
        if result.get("error").is_some() {
            return None;
        }
        self.cell_codes.observe(tool, &result);
        if let Some(input) = input {
            *diffs = self.cell_codes.diff(tool, input, &result);
        }
        if !matches!(tool, "open_notebook" | "new_notebook") {
            return None;
        }
        let id = result["notebook_id"].as_str()?.to_owned();
        Some((id, result["path"].as_str().map(str::to_owned)))
    }

    /// The runtime's copy of call `call`'s result (`{content, isError}`), which
    /// the agent didn't pass on: it becomes the call's output, as if it had.
    pub fn result_fetched(&mut self, call: &ToolCallId, result: serde_json::Value) -> Vec<Effect> {
        let Some(ix) = self.entries.iter().rposition(|e| matches!(e, Entry::Tool { id, .. } if id == call)) else { return Vec::new() };
        if let Entry::Tool { status, output, .. } = &mut self.entries[ix] {
            *output = Some(result["content"].clone());
            if result["isError"] == true {
                *status = ToolCallStatus::Failed;
            }
        }
        self.mark(ix);
        match self.result_arrived(ix) {
            Some((id, path)) => vec![Effect::ShowNotebook { id, path }],
            None => Vec::new(),
        }
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
            runs::Names::Cell => input["code"].as_str().or_else(|| input["cell_id"].as_str().and_then(|id| self.cell_codes.get(id))).map(cell_label),
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

    /// The permission request waiting for an answer, if any: the first of
    /// those waiting, which the card shows.
    pub fn pending_permission(&self) -> Option<usize> {
        self.waiting_prompts().first().copied()
    }

    /// Every prompt waiting for an answer, in the order they came.
    pub fn waiting_prompts(&self) -> Vec<usize> {
        self.entries.iter().enumerate().filter(|(_, e)| matches!(e, Entry::Permission { responder: Some(_), .. })).map(|(ix, _)| ix).collect()
    }

    /// Prompts the agent no longer waits on (its turn ended, its process
    /// stopped): their cards go, and a note says which went unanswered.
    fn drop_prompts(&mut self) {
        let waiting = self.waiting_prompts();
        let headings: Vec<String> = waiting.iter().filter_map(|ix| crate::approval::heading_at(self, *ix)).collect();
        for ix in waiting {
            if let Some(Entry::Permission { responder, .. }) = self.entries.get_mut(ix) {
                match responder.take() {
                    Some(Asker::Agent(responder)) => {
                        let _ = responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled));
                    }
                    Some(Asker::Runtime(ask)) => self.later.push(Effect::AnswerRun { ask, allow: false, user_ran: Vec::new() }),
                    None => {}
                }
            }
            self.mark(ix);
        }
        for heading in headings {
            self.note(format!("Claude stopped before you answered “{}”", heading.replace('`', "")));
        }
        self.asks.forget_batch();
    }

    /// Fix with Claude / Explain asks from the notebook's error boxes that are
    /// under way: (cell, "fix" | "explain", queued). The running turn's, if it
    /// is one, then those waiting in the queue.
    pub fn error_asks(&self) -> Vec<(String, &'static str, bool)> {
        let ask = |text: &str, attachments: &[Attachment], queued: bool| {
            let kind = if text == crate::annotate::EXPLAIN_ERROR { "explain" } else { "fix" };
            attachments.iter().find_map(|a| match a {
                Attachment::Error { cell, .. } => Some((cell.id.clone(), kind, queued)),
                _ => None,
            })
        };
        let mut asks = Vec::new();
        if self.outbox.busy {
            if let Some(Entry::User { text, attachments, .. }) = self.entries.get(self.turn_start()) {
                asks.extend(ask(text, attachments, false));
            }
        }
        asks.extend(self.outbox.items.iter().filter_map(|q| ask(&q.text, &q.attachments, !q.in_flight())));
        asks
    }

    /// Scroll the transcript to the running turn's message, when it is the
    /// error ask about `cell`.
    pub fn show_error_ask(&mut self, cell: &str) {
        if self.error_asks().iter().any(|(id, _, queued)| id == cell && !queued) {
            self.list.set_follow_mode(FollowMode::Normal);
            self.list.scroll_to_reveal_item(self.turn_start());
        }
    }

    /// Take the queued error ask about `cell` out of the queue.
    pub fn cancel_error_ask(&mut self, cell: &str) {
        let about = |q: &crate::outbox::Queued| q.attachments.iter().any(|a| matches!(a, Attachment::Error { cell: c, .. } if c.id == cell));
        if let Some(ix) = self.outbox.items.iter().position(|q| !q.in_flight() && about(q)) {
            self.outbox.take(ix);
        }
    }

    /// The agent is asking to let the notebook run ("Let this notebook run?").
    pub fn asking_to_run(&self) -> bool {
        self.pending_permission().is_some_and(|ix| matches!(&self.entries[ix], Entry::Permission { tool: Some(tool), .. } if tool == "allow_execution"))
    }

    /// Answer the pending request by kind (keys: ⏎ allow, ⌘⏎ always this
    /// session, Esc deny). For a plan: ⏎ starts (asking before runs), ⌘⏎ starts in Auto.
    pub fn answer_pending(&mut self, kind: PermissionOptionKind, scope: Scope) -> bool {
        let Some(ix) = self.pending_permission() else { return false };
        let Entry::Permission { options, plan, .. } = &self.entries[ix] else { return false };
        let option = match (plan.is_some(), kind) {
            (true, PermissionOptionKind::AllowOnce) => plan_option(options),
            _ => option_of_kind(options, kind),
        };
        let Some(option) = option.cloned() else { return false };
        self.answer(ix, &option, scope);
        true
    }

    /// Run anyway, in the page, when the user's own run reaches `cells` (each
    /// with its `last_run` before that run) that cards ask to run: allow each
    /// waiting card asking about any of them, as its Run button does. The
    /// runtime's cards carry the cells in their answer, so the runtime doesn't
    /// run them again; the agent's own (on an older runtime) only with
    /// `agent`, once the app told the runtime itself. How many it answered.
    pub fn allow_runs_of(&mut self, cells: &[(String, f64)], agent: bool) -> usize {
        let ids: Vec<String> = cells.iter().map(|(id, _)| id.clone()).collect();
        let asking = self.prompts_running(&self.waiting_prompts(), &ids);
        let mut answered = 0;
        for &ix in &asking {
            let Some(Entry::Permission { options, responder, .. }) = self.entries.get(ix) else { continue };
            if matches!(responder, Some(Asker::Agent(_))) && !agent {
                continue;
            }
            if let Some(allow) = option_of_kind(options, PermissionOptionKind::AllowOnce).cloned() {
                self.answer_with(ix, &allow, Scope::Once, cells.to_vec());
                answered += 1;
            }
        }
        answered
    }

    /// Whether an agent's own card (on an older runtime) asks to run any of `cells`.
    pub fn agent_asks_to_run(&self, cells: &[String]) -> bool {
        let agents: Vec<usize> = self.waiting_prompts().into_iter().filter(|ix| matches!(&self.entries[*ix], Entry::Permission { responder: Some(Asker::Agent(_)), .. })).collect();
        !self.prompts_running(&agents, cells).is_empty()
    }

    /// Of `prompts`, those asking to run any of `cells`.
    fn prompts_running(&self, prompts: &[usize], cells: &[String]) -> Vec<usize> {
        prompts.iter().copied().filter(|ix| crate::approval::run_cells_at(self, *ix).iter().any(|c| cells.contains(c))).collect()
    }

    /// `ran`: cells that were unrun and have run since, some way other than
    /// the agent's call (the user's run reached them without asking first).
    /// A card asking to run cells that have all run while it waited is
    /// answered as allowed, with a note; the runtime doesn't run them again.
    pub fn cells_ran(&mut self, ran: &[String]) {
        for ix in self.ran_cards(&self.waiting_prompts(), ran) {
            let names = crate::approval::heading_names_at(self, ix);
            let Some(Entry::Permission { options, .. }) = self.entries.get(ix) else { continue };
            let Some(allow) = option_of_kind(options, PermissionOptionKind::AllowOnce).cloned() else { continue };
            self.answer(ix, &allow, Scope::Once);
            let what = if names.is_empty() { "The cells".to_owned() } else { names.join(", ") };
            self.note(format!("{what} ran after your change"));
        }
    }

    /// Of `prompts`, the run cards (an `execute_cell` or `submit_changes` of
    /// cells the agent had already changed) whose cells have all run while
    /// they waited, counting `ran` now.
    fn ran_cards(&mut self, prompts: &[usize], ran: &[String]) -> Vec<usize> {
        let mut done = Vec::new();
        let mut waiting = HashSet::new();
        for &ix in prompts {
            let Some(Entry::Permission { call, tool: Some(tool), runs_code: true, .. }) = self.entries.get(ix) else { continue };
            if !matches!(tool.as_str(), "execute_cell" | "submit_changes") {
                continue;
            }
            let call = call.to_string();
            let cells = crate::approval::run_cells_at(self, ix);
            if cells.is_empty() {
                continue;
            }
            let seen = self.ran_while_asked.entry(call.clone()).or_default();
            seen.extend(ran.iter().filter(|c| cells.contains(c)).cloned());
            if cells.iter().all(|c| seen.contains(c)) {
                done.push(ix);
            } else {
                waiting.insert(call);
            }
        }
        self.ran_while_asked.retain(|call, _| waiting.contains(call));
        done
    }

    pub fn set_preview(&mut self, ix: usize, value: pluto::RunPreview) {
        if let Some(Entry::Permission { preview, .. }) = self.entries.get_mut(ix) {
            *preview = Some(value);
        }
    }

    /// Answer a permission request with `option`. `Scope::Session` remembers a
    /// rule for this session (for runs and plans: runs stop asking). It answers
    /// only this prompt: those already waiting stay questions of their own, and
    /// only prompts that arrive later are answered by the rule.
    pub fn answer(&mut self, ix: usize, option: &PermissionOption, scope: Scope) {
        self.answer_with(ix, option, scope, Vec::new());
    }

    /// `answer`, telling the runtime which cells the user's own run reached
    /// meanwhile (`user_ran`), for a run it holds.
    fn answer_with(&mut self, ix: usize, option: &PermissionOption, scope: Scope, user_ran: Vec<(String, f64)>) {
        let Some(Entry::Permission { responder, .. }) = self.entries.get_mut(ix) else { return };
        let Some(asker) = responder.take() else { return };
        match asker {
            Asker::Agent(responder) => {
                let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option.option_id.clone()));
                let _ = responder.respond(RequestPermissionResponse::new(outcome));
            }
            Asker::Runtime(ask) => {
                let allow = matches!(option.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways);
                self.later.push(Effect::AnswerRun { ask, allow, user_ran });
            }
        }
        self.record_answer(ix, option, scope);
    }

    /// What answers given outside `apply` left for the workspace to do.
    pub fn take_later(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.later)
    }

    /// The option that lets the agent's own prompt through at once, with no
    /// card and no mark on its row, for a call the runtime holds (`held`,
    /// `runtime_holds`): the runtime asks about it itself (it's from the
    /// app's build), and its card is the one the user sees.
    fn runtime_asks_instead<'a>(&self, held: bool, options: &'a [PermissionOption]) -> Option<&'a PermissionOption> {
        let asks = held && self.runtime_older == Some(false);
        option_of_kind(options, PermissionOptionKind::AllowOnce).filter(|_| asks)
    }

    /// The runtime's calls waiting for an answer (its event's `asks`): each
    /// new one of this session's gets a run card, or an edit card for an edit
    /// held in Manual. A card whose ask has gone (the agent gave up on the
    /// call) goes too, with a note.
    pub fn runtime_asks_now(&mut self, asks: &[serde_json::Value]) -> Vec<Effect> {
        let key = self.key.to_string();
        let mine: Vec<&serde_json::Value> = asks.iter().filter(|a| a["owner"] == key.as_str()).collect();
        let waiting: HashSet<u64> = mine.iter().filter_map(|a| a["id"].as_u64()).collect();
        let gone: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(ix, e)| matches!(e, Entry::Permission { responder: Some(Asker::Runtime(ask)), .. } if !waiting.contains(ask)).then_some(ix))
            .collect();
        for ix in gone {
            let heading = crate::approval::heading_at(self, ix);
            if let Some(Entry::Permission { responder, .. }) = self.entries.get_mut(ix) {
                *responder = None;
            }
            self.mark(ix);
            if let Some(heading) = heading {
                self.note(format!("Claude stopped waiting for your answer to “{}”", heading.replace('`', "")));
            }
        }
        let mut effects = Vec::new();
        for ask in mine {
            if let Some(id) = ask["id"].as_u64().filter(|id| self.seen_asks.insert(*id)) {
                effects.extend(self.runtime_ask(id, ask));
            }
        }
        effects
    }

    /// A card for the runtime's ask `id`: a run card for a call that runs
    /// code, else the edit card the agent's own prompt would get. Answered at
    /// once as the agent's prompt would be: a run once runs stop asking, an
    /// edit by an "Always this session" rule.
    fn runtime_ask(&mut self, id: u64, ask: &serde_json::Value) -> Vec<Effect> {
        let tool = ask["tool"].as_str().unwrap_or_default().to_owned();
        let input = ask["arguments"].clone();
        let title = format!("{}{tool}", celldiff::TOOL_PREFIX);
        let call = self.asked_call(id, ask, &tool, &input);
        let runs_code = endeavor_mcp::runs_code(&tool, &input);
        let arrival = self.answer_on_arrival(&Asked { title: &title, kind: None, input: &input, path: None }, runs_code, false);
        if let Some(approval) = arrival {
            self.approve(&call, approval, &title, &input);
            return vec![Effect::AnswerRun { ask: id, allow: true, user_ran: Vec::new() }];
        }
        let options = vec![
            PermissionOption::new(RUN_ALLOW, "Allow", PermissionOptionKind::AllowOnce),
            PermissionOption::new(RUN_DENY, "Deny", PermissionOptionKind::RejectOnce),
        ];
        let code = input["code"].as_str().map(str::to_owned).or_else(|| input["cell_id"].as_str().and_then(|c| self.cell_codes.get(c)).map(str::to_owned));
        let mut effects = Vec::new();
        if runs_code && tool != "run_shell" {
            effects.push(Effect::PreviewRun { ix: self.entries.len(), tool: tool.clone(), input: input.clone() });
        }
        self.asks.asked(self.waiting_prompts().len());
        self.push(Entry::Permission {
            call,
            title,
            code,
            options,
            responder: Some(Asker::Runtime(id)),
            runs_code,
            tool: Some(tool),
            input,
            kind: None,
            path: None,
            preview: None,
            plan: None,
        });
        effects.push(Effect::Asked(self.entries.len() - 1));
        effects
    }

    /// The tool call an ask is about: the id the agent's client gave it, else
    /// the latest call under way to the same tool with the same arguments,
    /// else one made up (its answer then shows as a note).
    fn asked_call(&self, id: u64, ask: &serde_json::Value, tool: &str, input: &serde_json::Value) -> ToolCallId {
        if let Some(call) = ask["call_id"].as_str() {
            return ToolCallId::new(call);
        }
        let under_way = |status: &ToolCallStatus| matches!(status, ToolCallStatus::Pending | ToolCallStatus::InProgress);
        self.entries
            .iter()
            .rev()
            .find_map(|e| match e {
                Entry::Tool { id, title, status, input: Some(given), .. } if under_way(status) && celldiff::notebook_tool(title) == Some(tool) && given == input => Some(id.clone()),
                _ => None,
            })
            .unwrap_or_else(|| ToolCallId::new(format!("ask-{id}")))
    }

    /// How a prompt arriving now is answered without a card: in Auto for
    /// runs, by an "Always this session" rule for the agent's own prompts.
    fn answer_on_arrival(&self, asked: &Asked, runs_code: bool, plan: bool) -> Option<Approval> {
        match () {
            _ if plan => None,
            _ if runs_code => self.run_without_asking.then_some(Approval::WithoutAsking),
            _ => self.asks.allowed(asked).then_some(Approval::ForSession),
        }
    }

    /// An answer given to prompt `ix`, once its responder has it: its row, the
    /// batch count, and the rule or mode "Always this session" sets.
    fn record_answer(&mut self, ix: usize, option: &PermissionOption, scope: Scope) {
        let Some(Entry::Permission { call, title, input, runs_code, plan, options, kind, path, .. }) = self.entries.get(ix) else { return };
        let allowed = matches!(option.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways);
        let approval = answer_approval(allowed, plan.is_some(), scope);
        let runs = *runs_code || plan.is_some();
        let rule = (allowed && !runs && scope == Scope::Session).then(|| Rule::for_prompt(&Asked { title, kind: *kind, input, path: path.as_deref() }, options));
        let (call, title, input) = (call.clone(), title.clone(), input.clone());
        self.mark(ix);
        self.asks.answered(call.clone());
        self.approve(&call, approval, &title, &input);
        if allowed && runs && scope == Scope::Session {
            self.run_without_asking = true;
            let mut effects = Vec::new();
            self.sync_policy(&mut effects);
            self.later.extend(effects);
        }
        if let Some(rule) = rule {
            self.asks.rules.push(rule);
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
/// The quiet line after a /compact: "Conversation summarized. Context 62% → 11%."
fn compacted_note(before: Option<(u64, u64)>, after: Option<(u64, u64)>) -> String {
    let percent = |(used, size): (u64, u64)| (size > 0).then(|| used * 100 / size);
    match (before.and_then(percent), after.and_then(percent)) {
        (Some(a), Some(b)) if a != b => format!("Conversation summarized. Context {a}% → {b}%."),
        _ => "Conversation summarized.".into(),
    }
}

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
        Approval::Allowed | Approval::Started | Approval::StartedInAuto => "Allowed",
        Approval::ForSession => "Allowed for this session",
        Approval::InFolder => "Allowed in this folder",
        Approval::WithoutAsking => "Allowed without asking",
        Approval::Denied | Approval::KeptPlanning => "Denied",
    };
    format!("{answer}: {what}")
}

/// The answer a prompt's row shows, from what the user picked.
pub(crate) fn answer_approval(allowed: bool, plan: bool, scope: Scope) -> Approval {
    match (allowed, plan, scope) {
        (false, true, _) => Approval::KeptPlanning,
        (false, false, _) => Approval::Denied,
        (true, true, Scope::Session) => Approval::StartedInAuto,
        (true, true, _) => Approval::Started,
        (true, false, Scope::Once) => Approval::Allowed,
        (true, false, Scope::Session) => Approval::ForSession,
        (true, false, Scope::Folder) => Approval::InFolder,
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

pub(crate) fn plan_option(options: &[PermissionOption]) -> Option<&PermissionOption> {
    PLAN_START.iter().find_map(|id| options.iter().find(|o| o.option_id.to_string() == *id))
}

pub(crate) fn option_of_kind(options: &[PermissionOption], kind: PermissionOptionKind) -> Option<&PermissionOption> {
    options.iter().find(|o| o.kind == kind)
}

/// A cell's name wherever the user sees one: what it defines (`model(S, p) = …`
/// → `model`), a markdown cell's first heading or words, "plot" for a cell that
/// draws one, "text" for one that builds Markdown, a `let` or `begin` block's last value, else its first line of
/// code; cut to `LABEL_MAX` characters.
pub(crate) fn cell_label(code: &str) -> String {
    let label = match markdown_text(code) {
        Some(text) => {
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
            let heading = lines.clone().find(|l| l.starts_with('#'));
            heading.or_else(|| lines.next()).map_or("markdown", |l| l.trim_start_matches('#').trim()).to_owned()
        }
        None => match code_lines(code).next() {
            Some(first) => defined_name(code).or_else(|| inferred_name(code)).unwrap_or_else(|| first.to_owned()),
            None => "cell".to_owned(),
        },
    };
    if label.chars().count() > LABEL_MAX {
        format!("{}…", label.chars().take(LABEL_MAX - 1).collect::<String>())
    } else {
        label
    }
}

const LABEL_MAX: usize = 28;

/// What code defines, from its first line of code: `x = …` → `x`. A Pluto
/// `begin` block is named by its first line inside.
fn defined_name(code: &str) -> Option<String> {
    let mut lines = code_lines(code);
    let line = lines.next().filter(|l| *l != "begin").or_else(|| lines.next())?;
    if let Some(rest) = ["function ", "macro ", "struct ", "mutable struct "].iter().find_map(|k| line.strip_prefix(k)) {
        let name: String = rest.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '!').collect();
        return (!name.is_empty()).then_some(name);
    }
    let (lhs, rhs) = line.split_once('=')?;
    // `f(x, k=1)` and `a == b` assign nothing.
    if rhs.starts_with('=') || lhs.matches('(').count() != lhs.matches(')').count() {
        return None;
    }
    let lhs = lhs.trim().trim_start_matches("const ");
    let name: String = lhs.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '!').collect();
    (!name.is_empty()).then_some(name)
}

/// A name for code of more than one line that defines nothing (a single line
/// names itself): "plot" when it calls a plotting
/// function, "text" when it builds Markdown, else a `let` or `begin` block's
/// last value when that's a name or a tuple of names (`t, y`).
fn inferred_name(code: &str) -> Option<String> {
    if code_lines(code).nth(1).is_none() {
        return None;
    }
    const PLOTS: [&str; 8] = ["plot", "scatter", "heatmap", "histogram", "lines", "surface", "contour", "Figure"];
    let calls_plot = code_lines(code).any(|line| {
        PLOTS.iter().any(|f| {
            line.match_indices(f).any(|(i, _)| {
                let before = line[..i].chars().next_back();
                let after = line[i + f.len()..].trim_start_matches('!');
                !before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') && after.starts_with('(')
            })
        })
    });
    if calls_plot {
        return Some("plot".to_owned());
    }
    if code_lines(code).any(|line| line.contains("Markdown.parse(") || line.contains("md\"")) {
        return Some("text".to_owned());
    }
    let lines: Vec<&str> = code_lines(code).collect();
    if !matches!(lines.first(), Some(&("let" | "begin"))) || lines.last() != Some(&"end") || lines.len() < 3 {
        return None;
    }
    let last = lines[lines.len() - 2];
    let names = last.split(',').map(str::trim).all(|n| !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_'));
    names.then(|| last.to_owned())
}

/// Code's lines, trimmed, without blank lines, `#` comments and `#= … =#` blocks.
fn code_lines(code: &str) -> impl Iterator<Item = &str> {
    let mut in_block = false;
    code.lines().map(str::trim).filter(move |line| {
        if in_block || line.starts_with("#=") {
            in_block = !line.ends_with("=#");
            return false;
        }
        !line.is_empty() && !line.starts_with('#')
    })
}

/// A markdown cell's text: `md"""…"""` or `md"…"` without its quotes.
fn markdown_text(code: &str) -> Option<&str> {
    let first = code_lines(code).next()?;
    let rest = &code[first.as_ptr() as usize - code.as_ptr() as usize..];
    let text = rest.strip_prefix("md\"\"\"").or_else(|| rest.strip_prefix("md\""))?;
    Some(text.trim_end().trim_end_matches('"'))
}

/// The file a call touches: its first location, else a path in its input.
pub(crate) fn file_path(path: Option<&Path>, input: &serde_json::Value) -> Option<String> {
    path.map(|p| p.display().to_string())
        .or_else(|| ["file_path", "notebook_path", "path"].iter().find_map(|f| input[*f].as_str()).map(str::to_owned))
}

pub(crate) fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |f| f.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]` macro.
    use super::{Approval, Effect, Entry, Mode, Scope, Session, SessionEvent, Started, Turn, app_modes};
    use crate::attach::Attachment;
    use crate::hosts::Place;
    use crate::outbox::Queued;
    use agent_client_protocol::schema::v1::{
        AvailableCommand, AvailableCommandsUpdate, CurrentModeUpdate, PermissionOptionKind, SessionId, SessionMode, SessionModeState, SessionUpdate,
        StopReason, ToolCallStatus, UsageUpdate,
    };

    #[test]
    fn live_entries_fade_in_and_replayed_history_shows_at_once() {
        let mut live = Session::new(1, Place::local("/p"), None);
        live.note("You stopped Claude");
        assert!(live.arrival(0).is_some(), "a live entry fades in");
        std::thread::sleep(crate::theme::motion_fast() + std::time::Duration::from_millis(10));
        assert_eq!(live.arrival(0), None, "and is dropped once its fade is over");
        let mut replayed = Session::loading(2, SessionId::new("s"), Place::local("/p"), None, "Old".into());
        replayed.note("You stopped Claude");
        assert_eq!(replayed.arrival(0), None);
    }

    #[test]
    fn always_this_session_answers_only_the_prompt_shown() {
        use super::{Approval, Scope};
        use crate::permits::Asked;
        use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, ToolCall, ToolCallId, ToolKind};
        let allow = PermissionOption::new("allow-once", "Yes", PermissionOptionKind::AllowOnce);
        let options = vec![allow.clone(), PermissionOption::new("reject", "No", PermissionOptionKind::RejectOnce)];
        let edit = |file: &str| serde_json::json!({ "file_path": file, "old_string": "a", "new_string": "b" });
        let mut s = Session::new(1, Place::local("/p"), None);
        for (id, file, runs_code, title) in [("t1", "/p/notes.md", false, "Edit"), ("t2", "/p/notes.md", false, "Edit"), ("r1", "", true, "mcp__notebook__execute_cell"), ("r2", "", true, "mcp__notebook__execute_cell")] {
            s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new(id.to_string(), title.to_string()))));
            s.push(Entry::Permission {
                call: ToolCallId::new(id.to_string()),
                title: title.into(),
                code: None,
                options: options.clone(),
                responder: None,
                runs_code,
                tool: None,
                input: if runs_code { serde_json::json!({ "cell_id": "a" }) } else { edit(file) },
                kind: Some(if runs_code { ToolKind::Other } else { ToolKind::Edit }),
                path: None,
                preview: None,
                plan: None,
            });
        }
        let row = |s: &Session, id: &str| {
            s.entries.iter().find_map(|e| match e {
                Entry::Tool { id: call, approval, .. } if call.to_string() == id => Some(*approval),
                _ => None,
            })
        };
        let at = |s: &Session, id: &str| s.entries.iter().position(|e| matches!(e, Entry::Permission { call, .. } if call.to_string() == id)).unwrap();

        // Always this session on the first edit: the second, already waiting, stays unanswered.
        s.record_answer(at(&s, "t1"), &allow, Scope::Session);
        assert_eq!(row(&s, "t1"), Some(Some(Approval::ForSession)));
        assert_eq!(row(&s, "t2"), Some(None), "a prompt already waiting is its own question");
        assert_eq!(s.asks.answered, 1);
        // A prompt arriving now for the same file is answered by the rule; another file isn't.
        let input = edit("/p/notes.md");
        let same = Asked { title: "Edit", kind: Some(ToolKind::Edit), input: &input, path: None };
        assert_eq!(s.answer_on_arrival(&same, false, false), Some(Approval::ForSession));
        let other_input = edit("/p/other.md");
        let other = Asked { title: "Edit", kind: Some(ToolKind::Edit), input: &other_input, path: None };
        assert_eq!(s.answer_on_arrival(&other, false, false), None);
        assert_eq!(s.answer_on_arrival(&same, false, true), None, "a plan always asks");

        // Always on a run card: the session goes to Auto, and the run already waiting still asks.
        s.record_answer(at(&s, "r1"), &allow, Scope::Session);
        assert!(s.run_without_asking);
        assert_eq!(row(&s, "r1"), Some(Some(Approval::ForSession)));
        assert_eq!(row(&s, "r2"), Some(None));
        let run_input = serde_json::json!({ "cell_id": "b" });
        let run = Asked { title: "mcp__notebook__execute_cell", kind: Some(ToolKind::Other), input: &run_input, path: None };
        assert_eq!(s.answer_on_arrival(&run, true, false), Some(Approval::WithoutAsking));
    }

    #[test]
    fn run_cards_reached_by_the_users_run() {
        use agent_client_protocol::schema::v1::ToolCallId;
        let asked = |id: &str, tool: &str, input: serde_json::Value| Entry::Permission {
            call: ToolCallId::new(id.to_string()),
            title: format!("mcp__notebook__{tool}"),
            code: None,
            options: Vec::new(),
            responder: None,
            runs_code: true,
            tool: Some(tool.into()),
            input,
            kind: None,
            path: None,
            preview: None,
            plan: None,
        };
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.push(asked("r1", "execute_cell", serde_json::json!({ "cell_id": "b" })));
        s.push(asked("r2", "submit_changes", serde_json::json!({ "cell_ids": ["c", "d"] })));
        s.push(asked("r3", "edit_cell", serde_json::json!({ "cell_id": "c", "code": "c = 2", "run_after": true })));
        let prompts = [0, 1, 2];

        // Run anyway answers the cards asking about any cell the user's run reaches.
        assert_eq!(s.prompts_running(&prompts, &["a".into(), "b".into()]), vec![0]);
        assert_eq!(s.prompts_running(&prompts, &["c".into()]), vec![1, 2]);
        assert_eq!(s.prompts_running(&prompts, &["z".into()]), Vec::<usize>::new());

        // Cells that ran without asking close a card once all of its cells have,
        // and never a card that would change the code first.
        assert_eq!(s.ran_cards(&prompts, &["b".into(), "c".into()]), vec![0]);
        assert_eq!(s.ran_cards(&[1, 2], &[]), Vec::<usize>::new(), "d hasn't run");
        assert_eq!(s.ran_cards(&[1, 2], &["d".into()]), vec![1], "c ran earlier, while it waited");
        assert!(s.ran_while_asked.is_empty(), "nothing kept for cards no longer waiting");
    }

    /// A session in Ask to run on a runtime from the app's build, with a
    /// notebook call `t1` to execute cell `a` under way.
    fn asking_session() -> Session {
        use agent_client_protocol::schema::v1::{SessionMode, ToolCall};
        let modes = SessionModeState::new("auto", vec![SessionMode::new("default", "Manual"), SessionMode::new("plan", "Plan"), SessionMode::new("auto", "Auto")]);
        let mut s = Session::new(7, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("s1"), Some(modes), None));
        s.runtime_build(false);
        let call = ToolCall::new("t1", "mcp__notebook__execute_cell").raw_input(serde_json::json!({ "cell_id": "a" })).status(ToolCallStatus::InProgress);
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(call)));
        s
    }

    fn ask(id: u64, call_id: Option<&str>) -> serde_json::Value {
        serde_json::json!({ "id": id, "owner": "7", "call_id": call_id, "tool": "execute_cell", "arguments": { "cell_id": "a" }, "since": 1.0 })
    }

    /// A run answered: its ask, whether allowed, and the cells the user ran.
    type Answered<'a> = (u64, bool, &'a [(String, f64)]);

    fn answers(effects: &[Effect]) -> Vec<Answered<'_>> {
        effects.iter().filter_map(|e| if let Effect::AnswerRun { ask, allow, user_ran } = e { Some((*ask, *allow, user_ran.as_slice())) } else { None }).collect()
    }

    #[test]
    fn a_run_the_runtime_holds_is_one_card_and_its_answer_goes_back() {
        let mut s = asking_session();
        let effects = s.runtime_asks_now(&[ask(41, None), serde_json::json!({ "id": 9, "owner": "8", "tool": "execute_cell", "arguments": {} })]);
        assert!(matches!(effects.as_slice(), [Effect::PreviewRun { tool, .. }, Effect::Asked(_)] if tool == "execute_cell"), "only this session's");
        let card = s.pending_permission().expect("a run card");
        let Entry::Permission { call, runs_code, options, .. } = &s.entries[card] else { panic!() };
        assert_eq!((call.to_string(), *runs_code, options.len()), ("t1".into(), true, 2), "on the call under way with the same arguments");
        assert!(s.runtime_asks_now(&[ask(41, None)]).is_empty(), "shown once");

        assert!(s.answer_pending(PermissionOptionKind::AllowOnce, Scope::Once));
        assert_eq!(answers(&s.take_later()), [(41, true, &[][..])]);
        assert!(matches!(&s.entries[0], Entry::Tool { approval: Some(Approval::Allowed), .. }));
        assert!(s.runtime_asks_now(&[ask(41, None)]).is_empty() && s.pending_permission().is_none(), "answered: no card again while the runtime catches up");

        // Denied, then Always this session: runs stop asking, and the runtime hears it.
        s.runtime_asks_now(&[ask(42, Some("t1"))]);
        assert!(s.answer_pending(PermissionOptionKind::RejectOnce, Scope::Once));
        assert_eq!(answers(&s.take_later()), [(42, false, &[][..])]);
        s.runtime_asks_now(&[ask(43, Some("t1"))]);
        assert!(s.answer_pending(PermissionOptionKind::AllowOnce, Scope::Session));
        let later = s.take_later();
        assert_eq!(answers(&later), [(43, true, &[][..])]);
        assert!(later.iter().any(|e| matches!(e, Effect::SetPolicy("auto", _))) && s.mode_name().as_deref() == Some("Auto"));
        // An ask already on its way when runs stopped asking: allowed with no card.
        assert_eq!(answers(&s.runtime_asks_now(&[ask(44, Some("t1"))])), [(44, true, &[][..])]);
        assert!(s.pending_permission().is_none());
    }

    #[test]
    fn a_run_card_goes_when_the_agent_gives_up_or_the_turn_ends() {
        let mut s = asking_session();
        s.runtime_asks_now(&[ask(41, Some("t1"))]);
        s.runtime_asks_now(&[]);
        assert!(s.pending_permission().is_none());
        assert!(matches!(s.entries.last(), Some(Entry::Note(n)) if n.starts_with("Claude stopped waiting for your answer to")));
        assert!(s.take_later().is_empty(), "nothing to answer");

        s.runtime_asks_now(&[ask(42, Some("t1"))]);
        s.apply(SessionEvent::TurnEnded(StopReason::Cancelled));
        assert_eq!(answers(&s.take_later()), [(42, false, &[][..])], "stopping the turn denies it");
    }

    #[test]
    fn the_users_own_run_answers_the_card_and_the_runtime_hears_which_cells_ran() {
        let mut s = asking_session();
        s.runtime_asks_now(&[ask(41, Some("t1"))]);
        assert_eq!(s.allow_runs_of(&[("a".into(), 5.5)], false), 1);
        assert_eq!(answers(&s.take_later()), [(41, true, &[("a".to_owned(), 5.5)][..])]);
    }

    #[test]
    fn the_agents_own_run_prompt_isnt_asked_twice() {
        use agent_client_protocol::schema::v1::PermissionOption;
        let options = [PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce), PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce)];
        let allowed = |s: &Session, run: bool| s.runtime_asks_instead(run, &options).map(|o| o.option_id.to_string());
        // The runtime holds the run and asks: the agent's own prompt is let through.
        let mut s = asking_session();
        assert_eq!(allowed(&s, true).as_deref(), Some("allow"));
        assert_eq!(allowed(&s, false), None, "an edit that doesn't run is still the agent's card");
        // A runtime from an older build, or one that hasn't said, doesn't ask: the agent's prompt is the card.
        s.runtime_build(true);
        assert_eq!(allowed(&s, true), None);
        let mut s = asking_session();
        s.runtime_older = None;
        assert_eq!(allowed(&s, true), None);
    }

    /// A session in Manual on a runtime from the app's build, with a notebook
    /// call `e1` to edit cell `a` under way.
    fn manual_session() -> Session {
        use agent_client_protocol::schema::v1::{SessionMode, ToolCall};
        let modes = SessionModeState::new("default", vec![SessionMode::new("default", "Manual"), SessionMode::new("plan", "Plan"), SessionMode::new("auto", "Auto")]);
        let mut s = Session::new(7, Place::local("/tmp"), None);
        let effects = s.started(Started::new(SessionId::new("s1"), Some(modes), None));
        assert!(matches!(effects.as_slice(), [Effect::SetPolicy("ask", true)]), "the runtime hears that edits ask");
        s.runtime_build(false);
        s.cell_codes.observe("read_cell", &serde_json::json!({ "cell_id": "a", "code": "a = 1" }));
        let call = ToolCall::new("e1", "mcp__notebook__edit_cell").raw_input(edit_input("a = 2")).status(ToolCallStatus::InProgress);
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(call)));
        s
    }

    fn edit_input(code: &str) -> serde_json::Value {
        serde_json::json!({ "notebook_id": "n", "cell_id": "a", "code": code })
    }

    fn held(id: u64, tool: &str, arguments: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "id": id, "owner": "7", "call_id": null, "tool": tool, "arguments": arguments, "since": 1.0 })
    }

    #[test]
    fn in_manual_an_edit_the_runtime_holds_is_one_edit_card() {
        use agent_client_protocol::schema::v1::PermissionOption;
        let mut s = manual_session();
        // The agent's own prompt before the edit is let through: the runtime asks instead.
        let options = [
            PermissionOption::new("allow", "Yes", PermissionOptionKind::AllowOnce),
            PermissionOption::new("always", "Yes, and don't ask again for mcp__notebook__edit_cell commands", PermissionOptionKind::AllowAlways),
            PermissionOption::new("reject", "No", PermissionOptionKind::RejectOnce),
        ];
        assert!(s.runtime_holds("edit_cell", &edit_input("a = 2")) && s.runtime_holds("move_cell", &serde_json::json!({})));
        assert!(!s.runtime_holds("read_cell", &serde_json::json!({ "cell_id": "a" })), "reads never wait");
        assert_eq!(s.runtime_asks_instead(true, &options).map(|o| o.option_id.to_string()).as_deref(), Some("allow"));

        // The runtime's ask is the one card: the edit card, with the change as a diff.
        let effects = s.runtime_asks_now(&[held(41, "edit_cell", edit_input("a = 2"))]);
        assert!(matches!(effects.as_slice(), [Effect::Asked(_)]), "nothing to preview: it runs nothing");
        assert_eq!(s.waiting_prompts().len(), 1);
        let Entry::Permission { call, runs_code, .. } = &s.entries[s.pending_permission().unwrap()] else { panic!() };
        assert_eq!((call.to_string(), *runs_code), ("e1".into(), false));
        let card = crate::approval::approval_view(&s).unwrap();
        assert_eq!(card.heading, "Edit `a`?");
        assert!(matches!(&card.code, Some(crate::approval::CardCode::Diff(_))));
        assert_eq!(card.lines[0].0, "Nothing runs. In Manual, every change to the notebook asks.");
        let buttons: Vec<&str> = card.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(buttons, ["Deny", "Always this session", "Edit"]);
        assert!(card.folder.is_none(), "no In this folder: the agent's folder rule wouldn't reach the runtime");

        // Denied: the runtime hears it, and the row says so.
        assert!(s.answer_pending(PermissionOptionKind::RejectOnce, Scope::Once));
        assert_eq!(answers(&s.take_later()), [(41, false, &[][..])]);
        assert!(matches!(&s.entries[0], Entry::Tool { approval: Some(Approval::Denied), .. }));

        // Always this session: later edits with the same tool are allowed with no card; other changes still ask.
        s.runtime_asks_now(&[held(42, "edit_cell", edit_input("a = 3"))]);
        assert!(s.answer_pending(PermissionOptionKind::AllowOnce, Scope::Session));
        let later = s.take_later();
        assert_eq!(answers(&later), [(42, true, &[][..])]);
        assert!(!later.iter().any(|e| matches!(e, Effect::SetPolicy(..))), "runs still ask");
        assert_eq!(answers(&s.runtime_asks_now(&[held(43, "edit_cell", edit_input("a = 4"))])), [(43, true, &[][..])]);
        assert!(s.pending_permission().is_none());
        assert!(matches!(s.entries.last(), Some(Entry::Note(n)) if n.as_ref() == "Allowed for this session: edit a cell"));
        s.runtime_asks_now(&[held(44, "move_cell", serde_json::json!({ "cell_id": "a", "after_cell_id": "" }))]);
        assert!(s.pending_permission().is_some(), "moving a cell is another question");
    }

    #[test]
    fn in_manual_a_run_is_still_a_run_card_and_edits_ask_after_runs_stop_asking() {
        let mut s = manual_session();
        let run = held(41, "edit_cell", serde_json::json!({ "notebook_id": "n", "cell_id": "a", "code": "a = 2", "run_after": true }));
        let effects = s.runtime_asks_now(&[run]);
        assert!(matches!(effects.as_slice(), [Effect::PreviewRun { .. }, Effect::Asked(_)]));
        assert_eq!(crate::approval::approval_view(&s).unwrap().heading, "Edit `a` and run it?");
        assert!(s.answer_pending(PermissionOptionKind::AllowOnce, Scope::Session));
        let later = s.take_later();
        assert!(later.iter().any(|e| matches!(e, Effect::SetPolicy("auto", true))), "runs stop asking; edits still do");
        assert_eq!(s.mode_name().as_deref(), Some("Manual"));
        assert!(!s.runtime_holds("execute_cell", &serde_json::json!({ "cell_id": "a" })));
        assert!(s.runtime_holds("add_cell", &serde_json::json!({ "code": "b = 1" })));
        s.runtime_asks_now(&[held(42, "add_cell", serde_json::json!({ "code": "b = 1" }))]);
        assert!(s.pending_permission().is_some(), "an edit still asks");
    }

    #[test]
    fn ask_to_run_and_an_older_runtime_leave_edits_to_the_agent() {
        // Ask to run: the runtime doesn't hold an edit, so the agent's prompt (if any) is the card.
        let s = asking_session();
        assert!(!s.edits_ask() && !s.runtime_holds("edit_cell", &edit_input("a = 2")));
        // An older runtime in Manual can't hold edits: the agent's own prompt asks.
        let mut s = manual_session();
        s.runtime_build(true);
        let options = [agent_client_protocol::schema::v1::PermissionOption::new("allow", "Yes", PermissionOptionKind::AllowOnce)];
        assert_eq!(s.runtime_asks_instead(s.runtime_holds("edit_cell", &edit_input("a = 2")), &options), None);
    }

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
            kind: None,
            path: None,
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
    fn transcript_notes_say_what_happened_in_plain_words() {
        use super::{Approval, answer_note, turn_ended_note};
        assert_eq!(turn_ended_note(StopReason::EndTurn), None);
        assert_eq!(turn_ended_note(StopReason::Cancelled), Some("You stopped Claude"));
        assert_eq!(turn_ended_note(StopReason::MaxTokens), Some("Claude stopped: the reply got too long"));
        assert_eq!(turn_ended_note(StopReason::MaxTurnRequests), Some("Claude stopped: it took too many steps in one go"));
        assert_eq!(turn_ended_note(StopReason::Refusal), Some("Claude declined to continue"));
        assert_eq!(answer_note(Approval::Allowed, "edit a cell"), "Allowed: edit a cell");
        assert_eq!(answer_note(Approval::ForSession, "run 2 cells"), "Allowed for this session: run 2 cells");
        assert_eq!(answer_note(Approval::WithoutAsking, "run a cell"), "Allowed without asking: run a cell");
        assert_eq!(answer_note(Approval::Denied, "delete a cell"), "Denied: delete a cell");
        use super::{Scope, answer_approval};
        // What each answer reads as on its row.
        assert_eq!(answer_approval(true, false, Scope::Once).label(), "allowed");
        assert_eq!(answer_approval(true, false, Scope::Session).label(), "allowed for this session");
        assert_eq!(answer_approval(true, false, Scope::Folder).label(), "allowed in this folder");
        assert_eq!(answer_approval(false, false, Scope::Once).label(), "denied");
        assert_eq!(answer_approval(true, true, Scope::Once).label(), "started");
        assert_eq!(answer_approval(true, true, Scope::Session).label(), "started in Auto");
        assert_eq!(answer_approval(false, true, Scope::Once).label(), "kept planning");
        assert_eq!(crate::transcript::answered_line(Approval::ForSession, "14:03"), "Allowed for this session at 14:03");
        assert_eq!(crate::transcript::answered_line(Approval::Denied, "14:06"), "Denied at 14:06");

        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("go"), false);
        s.apply(SessionEvent::TurnEnded(StopReason::Cancelled));
        assert!(matches!(s.entries.last(), Some(Entry::Note(note)) if note.as_ref() == "You stopped Claude"));
        s.approve(&"gone".to_string().into(), Approval::WithoutAsking, "mcp__notebook__submit_changes", &serde_json::json!({ "cell_ids": ["a", "b"] }));
        assert!(matches!(s.entries.last(), Some(Entry::Note(note)) if note.as_ref() == "Allowed without asking: run 2 cells"));
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
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(m), Effect::SetPolicy("plan", _)] if m.to_string() == "plan"));
        assert_eq!(s.mode_name().as_deref(), Some("Plan"));
        // Leaving plan tells the runtime too.
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(m), Effect::SetPolicy("ask", _)] if m.to_string() == "auto"));
        assert_eq!(s.mode_name().as_deref(), Some("Ask to run"));
        // Auto is the same agent mode, with runs no longer asking: only the runtime hears.
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetPolicy("auto", _)]) && s.run_without_asking);
        assert_eq!(s.mode_name().as_deref(), Some("Auto"));
        assert!(matches!(s.cycle_mode().as_slice(), [Effect::SetMode(m), Effect::SetPolicy("plan", _)] if m.to_string() == "plan"));
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

        // Auto picked on the new-session screen: the agent is asked, and the runtime told runs stop asking.
        let (mut s, effects) = start("default", 2);
        assert!(matches!(effects.as_slice(), [Effect::SetMode(m), Effect::SetPolicy("auto", _)] if m.to_string() == "auto"));
        assert_eq!((s.mode_name().as_deref(), s.policy()), (Some("Auto"), "auto"));
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
        assert!(matches!(effects.as_slice(), [Effect::SetMode(m), Effect::SetPolicy("ask", true)] if m.to_string() == "default"), "edits ask too");
        assert_eq!(s.mode_name().as_deref(), Some("Manual"));
        assert_eq!((s.policy(), s.guard_mode()), ("ask", "manual"), "the runtime asks before runs; so does the agent");

        // A reopened session in Plan: the agent is asked and the runtime told.
        let (s, effects) = start("default", 3);
        assert!(matches!(effects.as_slice(), [Effect::SetMode(m), Effect::SetPolicy("plan", _)] if m.to_string() == "plan"));
        assert_eq!(s.mode_name().as_deref(), Some("Plan"));

        // Already in the mode: nothing to ask.
        let (s, effects) = start("auto", 1);
        assert!(effects.is_empty());
        assert_eq!(s.mode_name().as_deref(), Some("Ask to run"));
        assert_eq!((s.policy(), s.guard_mode()), ("ask", "ask"));
    }

    #[test]
    fn a_runtime_from_an_older_build_gets_a_note_once() {
        let mut s = Session::new(1, Place::local("/tmp"), Some("gpu-box".into()));
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.runtime_build(false);
        assert!(s.entries.is_empty(), "the app's own build: nothing to say");
        s.runtime_build(true);
        s.runtime_build(true);
        let notes: Vec<String> = s.entries.iter().filter_map(|e| if let Entry::Note(n) = e { Some(n.to_string()) } else { None }).collect();
        assert_eq!(
            notes,
            ["Julia on gpu-box is from an older Endeavor. Restart Julia to get the latest changes. Until then, Ask to run doesn't let Claude run code."]
        );

        // A reopened session says so after its history.
        let mut s = Session::loading(2, SessionId::new("old"), Place::local("/tmp"), None, "Fit".into());
        s.runtime_build(true);
        assert!(s.entries.is_empty(), "not while the history loads");
        s.started(Started::new(SessionId::new("old"), None, None));
        assert!(matches!(s.entries.last(), Some(Entry::Note(n)) if n.starts_with(&format!("Julia on {} is from an older Endeavor.", crate::platform::this_computer!()))));
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
        assert!(matches!(s.choose_mode(&auto).as_slice(), [Effect::SetMode(m), Effect::SetPolicy("auto", _)] if m.to_string() == "auto"));
        assert_eq!((s.current_mode(), s.mode_name().as_deref()), (Some(2), Some("Auto")));
        let plan = s.mode_choices()[3].clone();
        assert!(matches!(s.choose_mode(&plan).as_slice(), [Effect::SetMode(_), Effect::SetPolicy("plan", _)]));
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
    fn error_asks_say_which_cell_claude_is_on_and_which_wait() {
        use crate::attach::Cell;
        let error = |cell: &str, text: &str| {
            let attachment = Attachment::Error { notebook: "nb".into(), cell: Cell { id: cell.into(), code: "x".into() }, text: "boom".into() };
            Queued::new(text.into(), vec![attachment], vec![])
        };
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(error("c1", crate::annotate::FIX_ERROR), false);
        s.submit(error("c2", crate::annotate::EXPLAIN_ERROR), false);
        s.submit(text("and plot it"), false);
        assert_eq!(s.error_asks(), vec![("c1".to_string(), "fix", false), ("c2".to_string(), "explain", true)]);
        s.cancel_error_ask("c2");
        s.cancel_error_ask("c1");
        assert_eq!(s.error_asks(), vec![("c1".to_string(), "fix", false)], "the running one can't be cancelled");
        assert_eq!(s.outbox.items.len(), 1);
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(s.error_asks().is_empty());
    }

    #[test]
    fn a_slash_command_goes_first_and_alone() {
        use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
        let words = |effects: &[Effect]| -> Vec<String> {
            effects
                .iter()
                .find_map(|e| match e {
                    Effect::Send(Turn::Prompt(blocks)) => Some(blocks.iter().map(|b| match b {
                        ContentBlock::Text(t) => t.text.clone(),
                        _ => "other".into(),
                    }).collect()),
                    _ => None,
                })
                .expect("a prompt")
        };
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.start_context = Some(ContentBlock::Text(TextContent::new("[Endeavor] The notebook moved.")));
        let blocks = crate::attach::prompt_blocks("/compact keep the fits", &[], &["data.csv".into()]);
        let sent = words(&s.submit(Queued::new("/compact keep the fits".into(), vec![], blocks), false));
        assert_eq!(sent[0], "/compact keep the fits");
        assert_eq!(sent.len(), 2, "the command, then the note on @ mentions");
        // The moved-notebook note waits for a message it can go with.
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        let sent = words(&s.submit(Queued::new("plot it".into(), vec![], crate::attach::prompt_blocks("plot it", &[], &[])), false));
        assert_eq!(sent, ["[Endeavor] The notebook moved.", "plot it"]);
    }

    #[test]
    fn compact_ends_with_a_quiet_note() {
        use agent_client_protocol::schema::v1::{SessionUpdate, UsageUpdate};
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.apply(SessionEvent::Update(SessionUpdate::UsageUpdate(UsageUpdate::new(124_000, 200_000))));
        s.submit(text("/compact"), false);
        s.apply(SessionEvent::Update(SessionUpdate::UsageUpdate(UsageUpdate::new(22_000, 200_000))));
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        let Some(Entry::Note(note)) = s.entries.last() else { panic!("a note last") };
        assert_eq!(note.as_ref(), "Conversation summarized. Context 62% → 11%.");
        // Other messages get no such note.
        s.submit(text("/context"), false);
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(matches!(s.entries.last(), Some(Entry::User { .. })));
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

        let failed = || SessionEvent::TurnFailed { kind: Some("server_error".into()), error: ERROR.into() };
        s.submit(text("plot it"), false);
        s.apply(reply(ERROR));
        s.hold();
        s.apply(failed());
        assert!(matches!(s.entries.as_slice(), [Entry::User { .. }]), "offline: only the message, kept");
        assert_eq!(s.unanswered, Some(0));

        s.release();
        s.apply(reply("Here it is.\n\n"));
        s.apply(reply(ERROR));
        s.apply(failed());
        assert!(matches!(&s.entries[1..], [Entry::Agent { text: said, .. }, Entry::Failed(super::Failed { kind: super::FailedKind::Partway { reason }, raw: Some(raw), .. })]
            if said == "Here it is." && *reason == "The connection to Claude dropped partway through this reply." && raw == ERROR));
    }

    fn card(s: &Session, ix: usize) -> (&'static str, String, &'static str, bool) {
        match &s.entries[ix] {
            Entry::Failed(f) => (f.kind.title(), f.kind.body(), f.kind.action(), f.used),
            _ => panic!("not a failure"),
        }
    }

    #[test]
    fn a_turn_claude_couldnt_answer_keeps_its_message_for_try_again() {
        const OVERLOADED: &str = r#"API Error: 529 {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("add bootstrap intervals"), false);
        let effects = s.apply(SessionEvent::TurnFailed { kind: Some("server_error".into()), error: OVERLOADED.into() });
        assert!(matches!(effects.as_slice(), [Effect::CheckRunState]), "no retry by itself");
        assert_eq!(card(&s, 1), ("Claude couldn't answer", "Anthropic's servers are busy right now. Your message is kept.".into(), "Try again", false));

        let effects = s.try_again(1);
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]), "the same message goes again");
        assert_eq!(s.entries.len(), 2, "no second bubble");
        assert!(card(&s, 1).3, "the card goes");
        assert!(s.try_again(1).is_empty(), "only once");
        assert!(s.busy_since.is_some());
    }

    #[test]
    fn a_reply_that_stops_partway_stays_and_continue_picks_it_up() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("explain"), false);
        s.apply(SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new("First the resampling cell:"))))));
        s.apply(SessionEvent::TurnEnded(StopReason::MaxTokens));
        assert_eq!(card(&s, 2), ("Claude stopped before finishing", "The reply reached its length limit.".into(), "Continue", false));
        let effects = s.continue_reply(2);
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]));
        assert!(matches!(&s.entries[3], Entry::User { text, .. } if text.as_ref() == "Continue from where you stopped."));
        assert!(card(&s, 2).3, "its button goes");

        s.apply(SessionEvent::TurnEnded(StopReason::MaxTurnRequests));
        assert_eq!(card(&s, 4).1, "Claude took too many steps in one go.");
        // Stopped by the user: still a quiet note.
        s.submit(text("again"), false);
        s.apply(SessionEvent::TurnEnded(StopReason::Cancelled));
        assert!(matches!(s.entries.last(), Some(Entry::Note(note)) if note.as_ref() == "You stopped Claude"));
    }

    #[test]
    fn a_usage_limit_holds_the_message_until_it_resets() {
        use crate::trouble::Reset;
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("plot it"), false);
        let effects = s.apply(SessionEvent::TurnFailed { kind: Some("rate_limit".into()), error: "Internal error: You've hit your session limit · resets 3pm (America/Los_Angeles)".into() });
        assert!(matches!(effects.as_slice(), [Effect::UsageLimit(Some(Reset::At { date: None, hour: 15, minute: 0 }))]));
        assert_eq!(s.unanswered, Some(0), "Not answered yet");
        assert!(s.submit(text("and the axes"), false).is_empty(), "new messages queue");
        assert!(matches!(s.release().as_slice(), [Effect::Send(Turn::Prompt(_))]), "at the reset, the held message goes");
    }

    #[test]
    fn a_stop_pauses_the_queue_and_marks_no_new_reply() {
        let mut s = Session::new(1, Place::local("/tmp/project"), None);
        s.started(Started::new(SessionId::new("a"), None, None));
        s.submit(text("fit it"), false);
        s.submit(text("then plot it"), false);
        assert!(s.apply(SessionEvent::TurnEnded(StopReason::Cancelled)).iter().all(|e| !matches!(e, Effect::Send(_))));
        assert_eq!(s.outbox.heading().as_deref(), Some("Paused after you stopped Claude"));
        assert!(!s.unseen);
        assert!(matches!(s.queue_action(crate::outbox::Outbox::send_next).as_slice(), [Effect::Send(Turn::Prompt(_))]));
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(s.unseen && !s.errored);
    }

    #[test]
    fn only_a_session_whose_reply_was_cut_off_says_so_after_a_restart() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};
        let mut working = Session::new(1, Place::local("/tmp/project"), None);
        working.started(Started::new(SessionId::new("a"), None, None));
        working.submit(text("fit it"), false);
        working.apply(SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new("First the model:"))))));
        let mut idle = Session::new(2, Place::local("/tmp/project"), None);
        idle.started(Started::new(SessionId::new("b"), None, None));

        for s in [&mut working, &mut idle] {
            s.agent_stopped();
            assert!(s.agent_waiting && s.busy_since.is_none());
            assert!(s.submit(text("meanwhile"), false).is_empty(), "messages queue while Claude is down");
        }
        // The reload replays the history the transcript already has.
        working.apply(SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new("replayed"))))));
        let effects = working.started(Started::new(SessionId::new("a"), None, None));
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]), "the queued message goes once it's back");
        assert!(matches!(&working.entries[1], Entry::Agent { text, .. } if text == "First the model:"));
        assert_eq!(card(&working, 2), ("Claude restarted. This reply was cut off.", String::new(), "Continue", false));
        idle.started(Started::new(SessionId::new("b"), None, None));
        assert!(!idle.entries.iter().any(|e| matches!(e, Entry::Failed(_))), "an idle session gets no note");
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
        assert_eq!(s.entries.len(), 2, "after loading, user chunks are ignored (the second entry is the Reopened line)");
    }

    #[test]
    fn a_session_opened_from_endeavors_copy_shows_it_until_the_replay_replaces_it() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, SessionUpdate, TextContent};
        let said = |s: &str| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(s)))));
        let reply = |s: &str| SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(s)))));
        let words = |s: &Session| {
            s.entries
                .iter()
                .map(|e| match e {
                    Entry::User { text, .. } => format!("you: {text}"),
                    Entry::Agent { text, .. } => format!("claude: {text}"),
                    Entry::Reopened(_) => "reopened".into(),
                    _ => "other".into(),
                })
                .collect::<Vec<_>>()
        };
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Fit".into());
        s.show_copy(vec![
            Entry::User { text: "fit it".into(), expanded: false, attachments: Vec::new(), delivery: crate::outbox::Delivery::Turn, sent: None },
            Entry::Agent { text: "Done.".into(), at: None },
        ]);
        assert!(s.showing_copy());
        s.apply(said("fit it"));
        s.apply(reply("Done."));
        s.apply(said("and plot it"));
        s.apply(reply("Plotted."));
        assert_eq!(words(&s), ["you: fit it", "claude: Done."], "the copy shows while the replay arrives");
        s.submit(text("thanks"), true);
        assert_eq!(words(&s), ["you: fit it", "claude: Done."], "a message waits for the session to open");
        let effects = s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(!s.showing_copy());
        assert_eq!(words(&s), ["you: fit it", "claude: Done.", "you: and plot it", "claude: Plotted.", "reopened", "you: thanks"]);
        assert_eq!(s.below.get(), Some(crate::transcript_copy::Below::New(2)));
        assert!(effects.iter().any(|e| matches!(e, Effect::Send(_))), "the waiting message goes once open");
    }

    #[test]
    fn live_messages_know_when_they_were_sent_and_replayed_ones_do_not() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};
        let reply = |t: &str| SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(t)))));
        let times = |s: &Session| -> Vec<bool> {
            s.entries
                .iter()
                .filter(|e| !matches!(e, Entry::Reopened(_)))
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

    #[test]
    fn a_reopened_session_leaves_out_claude_codes_task_notifications() {
        use agent_client_protocol::schema::v1::{ContentChunk, SessionUpdate, TextContent};
        use agent_client_protocol::schema::v1::ContentBlock;
        let chunk = |s: &str| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(s)))));
        let notice = "<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n<summary>Background command \"sleep 5\" completed</summary>\n</task-notification>";
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old chat".into());
        s.apply(chunk(notice));
        assert!(s.entries.is_empty());
        s.apply(chunk(&format!("{notice}\nnow plot it")));
        let [Entry::User { text, .. }] = s.entries.as_slice() else { panic!("one user entry") };
        assert_eq!(text.as_ref(), "now plot it");
    }

    /// The blocks of a sent prompt as a reopened session replays them: the
    /// adapter stores links and text files as text, and a text file's contents
    /// after the rest (claude-agent-acp's `promptToClaude`).
    fn as_replayed(blocks: Vec<agent_client_protocol::schema::v1::ContentBlock>) -> Vec<SessionEvent> {
        use agent_client_protocol::schema::v1::{ContentChunk, SessionUpdate};
        crate::transcript_copy::as_replayed(blocks).into_iter().map(|b| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(b)))).collect()
    }

    #[test]
    fn a_reopened_session_gets_its_chips_back_as_sent() {
        use crate::attach::{Cell, CellAsk, Part, Quote, Quoted};
        use std::sync::Arc;
        const NB: &str = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let cell = |id: &str, code: &str| Cell { id: id.into(), code: code.into() };
        let sent = vec![
            Attachment::Cells { notebook: NB.into(), cells: vec![cell("c1", "rates = map(fit, runs)")], ask: CellAsk::About },
            Attachment::Error { notebook: NB.into(), cell: cell("c2", "fit = curve_fit(model, t, y, p0)"), text: "BoundsError: attempt to access 3-element Vector".into() },
            Attachment::Quote(Quote { from: Quoted::Reply { text: "bunching\nhere".into(), at: Some("14:02".into()) }, comment: "why?".into() }),
            Attachment::Quote(Quote {
                from: Quoted::Cell { notebook: NB.into(), cell: "c3".into(), name: "plot".into(), part: Part::Figure(Arc::new(vec![0x89, b'P', b'N', b'G', 9])) },
                comment: "these points".into(),
            }),
            Attachment::Quote(Quote { from: Quoted::Box { notebook: NB.into(), cells: vec!["c3".into(), "c4".into()], png: Arc::new(vec![0x89, b'P', b'N', b'G', 1, 2, 3]) }, comment: String::new() }),
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
        assert_eq!(failure.kind, super::OpenFailure::InCli);
        assert_eq!(failure.title(false), "This session is open in Claude Code");
        assert_eq!(s.reopen_as_copy(), Some(SessionId::new("abc")));
        assert!(s.failed.is_none() && s.id.is_none() && s.title.ends_with("(copy)"));
        assert!(s.submit(text("go on"), false).is_empty(), "a message waits for the copy");

        let mut other = Session::loading(2, SessionId::new("def"), Place::local("/tmp"), None, "Fit".into());
        other.fail(r#"Internal error: { "details": "boom happened
more" }"#);
        let failure = other.failed.as_ref().unwrap();
        assert_eq!((failure.title(false), failure.body(super::Beside::Nothing).as_str()), ("Couldn't open this session", "Claude Code couldn't load it."));
        assert!(failure.raw.contains("boom happened"), "the raw error is under Details");
        other.retry_open();
        assert!(other.failed.is_none() && other.agent_waiting && other.id.is_some(), "Try again loads it again");
    }

    #[test]
    fn a_session_that_wont_open_says_why_in_plain_words() {
        use super::{OpenFailure, open_failure};
        assert_eq!(open_failure("Internal error: Unexpected end of JSON input at line 2214"), OpenFailure::Other(Some("Its history couldn't be read.")));
        assert_eq!(open_failure("ENOENT: no such file or directory, chdir '/Users/sam/gone'"), OpenFailure::Other(Some("Its folder isn't there any more.")));
        assert_eq!(open_failure("Internal error: Session abc not found"), OpenFailure::Other(Some("Claude Code no longer has its history.")));
        assert_eq!(open_failure("Error: Session abc is running as a background session (abc). Run `claude attach abc` to open it"), OpenFailure::InCli);
        let failure = super::Failure { kind: open_failure("Unexpected token } in JSON"), raw: String::new(), details_open: false };
        assert_eq!(failure.body(super::Beside::Fine), "Its history couldn't be read. The notebook and its file are fine.");
        assert_eq!(failure.body(super::Beside::Open), "Its history couldn't be read. The notebook is open beside it.");
        assert_eq!(failure.body(super::Beside::Unknown), "Its history couldn't be read, so Endeavor doesn't know its notebook yet.");
        assert_eq!(failure.title(true), "Couldn't start this session");
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
    fn a_notebook_call_named_another_agents_way_gets_its_diff_and_opens_its_notebook() {
        use agent_client_protocol::schema::v1::{ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields};
        use serde_json::json;
        const A: &str = "11111111-2222-4333-8444-555555555555";
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        // Cursor's shape: a placeholder title, then the real one with a wrapped input.
        let mut call = |id: &str, tool: &str, args: serde_json::Value, result: serde_json::Value| {
            s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new(id.to_string(), "MCP: tool"))));
            let input = json!({ "providerIdentifier": "notebook", "toolName": tool, "args": args });
            let named = ToolCallUpdateFields::new().title(format!("notebook: {tool}")).raw_input(input);
            s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id.to_string(), named))));
            let output = json!([{ "type": "text", "text": result.to_string() }]);
            let done = ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(output);
            s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id.to_string(), done))))
        };
        let opened = call("t0", "open_notebook", json!({ "path": "/tmp/a.jl" }), json!({ "notebook_id": "n1", "path": "/tmp/a.jl" }));
        assert!(matches!(opened.as_slice(), [Effect::ShowNotebook { id, .. }] if id == "n1"), "the pane follows");
        call("t1", "read_cell", json!({ "cell_id": A }), json!({ "cell_id": A, "code": "rate = 0.1" }));
        call("t2", "edit_cell", json!({ "cell_id": A, "code": "rate = 0.2" }), json!({ "ok": true }));
        let Some(Entry::Tool { title, input, diffs, .. }) = s.entries.last() else { panic!("no call") };
        assert_eq!(title, "mcp__notebook__edit_cell");
        assert_eq!(input.as_ref(), Some(&json!({ "cell_id": A, "code": "rate = 0.2" })));
        assert_eq!(diffs.len(), 1, "diffed against the read");
    }

    #[test]
    fn a_call_whose_agent_sends_no_result_gets_it_from_the_runtime() {
        use agent_client_protocol::schema::v1::{ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields};
        use serde_json::json;
        const A: &str = "11111111-2222-4333-8444-555555555555";
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        // Cursor's shape: the call completes with only `{"success": true}`.
        let call = |s: &mut Session, id: &str, tool: &str, args: serde_json::Value| {
            s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new(id.to_string(), "MCP: tool"))));
            let named = ToolCallUpdateFields::new().title(format!("notebook: {tool}")).raw_input(json!({ "providerIdentifier": "notebook", "toolName": tool, "args": args }));
            s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id.to_string(), named))));
            let done = ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(json!({ "success": true }));
            s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id.to_string(), done))))
        };
        let text = |result: serde_json::Value| json!({ "content": [{ "type": "text", "text": result.to_string() }], "isError": false });

        let effects = call(&mut s, "t0", "open_notebook", json!({ "path": "/tmp/a.jl" }));
        let [Effect::FetchResult { call: id, tool, input }] = effects.as_slice() else { panic!("asks the runtime") };
        assert_eq!((id.to_string(), tool.as_str(), input), ("t0".into(), "open_notebook", &json!({ "path": "/tmp/a.jl" })));
        let shown = s.result_fetched(&ToolCallId::new("t0"), text(json!({ "notebook_id": "n1", "path": "/tmp/a.jl" })));
        assert!(matches!(shown.as_slice(), [Effect::ShowNotebook { id, .. }] if id == "n1"), "the pane follows");

        call(&mut s, "t1", "read_cell", json!({ "cell_id": A }));
        s.result_fetched(&ToolCallId::new("t1"), text(json!({ "cell_id": A, "code": "rate = 0.1" })));
        call(&mut s, "t2", "edit_cell", json!({ "cell_id": A, "code": "rate = 0.2" }));
        s.result_fetched(&ToolCallId::new("t2"), text(json!({ "applied": true })));
        let Some(Entry::Tool { diffs, status, .. }) = s.entries.last() else { panic!("no call") };
        assert_eq!((diffs.len(), *status), (1, ToolCallStatus::Completed), "diffed against the read");

        call(&mut s, "t3", "execute_cell", json!({ "cell_id": A }));
        let error = json!({ "content": [{ "type": "text", "text": "{\"error\":\"run_conflict\",\"message\":\"No.\"}" }], "isError": true });
        s.result_fetched(&ToolCallId::new("t3"), error);
        let Some(Entry::Tool { status, .. }) = s.entries.last() else { panic!("no call") };
        assert_eq!(*status, ToolCallStatus::Failed, "the runtime says it failed, though the agent said it completed");
    }

    #[test]
    fn a_permission_request_with_no_rawinput_takes_it_from_the_tool_call_update() {
        use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, RequestPermissionRequest, ToolCall, ToolCallUpdate, ToolCallUpdateFields};
        use serde_json::json;
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        const ID: &str = "tool_7c68647e-18a7-4dfb-aa68-b8e51f67e08";
        let args = json!({ "notebook_id": "a63586b4", "cell_id": "b010b8f2", "code": "using Statistics\ns = mean(x)" });
        // The real Cursor sequence: a placeholder tool_call, then the update that
        // carries the wrapped rawInput, then a permission request for the same id
        // with no rawInput at all (only a json text block in its content).
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new(ID, "MCP: tool"))));
        let wrapped = json!({ "providerIdentifier": "notebook", "toolName": "edit_cell", "args": args });
        let update = ToolCallUpdateFields::new().title("notebook: edit_cell").raw_input(wrapped);
        s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(ID, update))));

        let options = vec![
            PermissionOption::new("allow-once", "Allow once", PermissionOptionKind::AllowOnce),
            PermissionOption::new("allow-always", "Allow always", PermissionOptionKind::AllowAlways),
            PermissionOption::new("reject-once", "Reject", PermissionOptionKind::RejectOnce),
        ];
        let asking = ToolCallUpdateFields::new().title("notebook-edit_cell: edit_cell");
        let request = RequestPermissionRequest::new(SessionId::new("abc"), ToolCallUpdate::new(ID, asking), options);
        let (title, input) = s.permission_input(&request);
        assert_eq!(title, "mcp__notebook__edit_cell", "the title names the tool, not just the server");
        assert_eq!(input, Some(args), "the arguments come from the earlier tool_call_update, already unwrapped");
    }

    #[test]
    fn a_located_notebook_wins_over_the_one_the_history_last_opened() {
        use agent_client_protocol::schema::v1::{SessionUpdate, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields};
        // The user located the file at its new path, which Endeavor recorded; Claude's history still opens the old one.
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old".into());
        s.notebook_path = Some("/tmp/archive/a.jl".into());
        s.apply(SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new("t1", "mcp__notebook__open_notebook"))));
        let output = serde_json::json!([{ "type": "text", "text": "{\"notebook_id\":\"old-id\",\"path\":\"/tmp/a.jl\"}" }]);
        let done = ToolCallUpdate::new("t1", ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(output));
        s.apply(SessionEvent::Update(SessionUpdate::ToolCallUpdate(done)));
        let effects = s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(!effects.iter().any(|e| matches!(e, Effect::ReopenNotebook(_))), "the recorded notebook opened already; the old path doesn't come back");
        assert_eq!(s.notebook_path.as_deref(), Some("/tmp/archive/a.jl"));
    }

    #[test]
    fn a_message_written_while_the_history_loads_waits_and_goes_once_open() {
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old".into());
        assert!(s.opening());
        assert!(s.submit(text("now plot it"), true).is_empty(), "nothing goes while the history loads, not even to steer");
        assert!(s.entries.is_empty());
        let effects = s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]), "it goes once the session is open");
        assert!(!s.opening());
    }

    #[test]
    fn a_reopened_history_shows_whole_once_loaded_then_a_reopened_line() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, SessionUpdate, TextContent};
        let reply = |t: &str| SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(t)))));
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old".into());
        assert!(s.opening_since.is_some());
        s.apply(reply("The interval is [0.081, 0.097] per minute."));
        assert!(s.opening(), "still the summary while the history arrives");
        s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(!s.opening());
        assert!(s.opening_since.is_none());
        assert!(matches!(s.entries.as_slice(), [Entry::Agent { .. }, Entry::Reopened(_)]));
        assert!(s.list.is_following_tail(), "it shows scrolled to the end");

        let mut empty = Session::loading(2, SessionId::new("def"), Place::local("/tmp"), None, "Old".into());
        empty.started(Started::new(SessionId::new("def"), None, None));
        assert!(empty.entries.is_empty(), "no line under nothing");

        let mut new = Session::new(3, Place::local("/tmp"), None);
        assert!(!new.opening(), "a new session has no history to wait for");
        new.started(Started::new(SessionId::new("ghi"), None, None));
        assert!(new.entries.is_empty());
    }

    #[test]
    fn claude_restarting_mid_load_starts_the_history_over_and_keeps_waiting_messages() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, SessionUpdate, TextContent};
        let reply = |t: &str| SessionEvent::Update(SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(t)))));
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old".into());
        s.apply(reply("first half"));
        s.submit(text("then this"), false);
        s.agent_stopped();
        assert!(s.entries.is_empty(), "the part loaded so far goes; the next load brings it all");
        s.apply(reply("whole history"));
        let effects = s.started(Started::new(SessionId::new("abc"), None, None));
        assert!(matches!(&s.entries[0], Entry::Agent { text, .. } if text == "whole history"));
        assert!(matches!(effects.as_slice(), [Effect::Send(Turn::Prompt(_))]), "the waiting message goes");
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

    /// A completed notebook tool call, as Claude Code reports it live and replays it.
    fn notebook_call(id: &str, tool: &str, input: serde_json::Value, result: serde_json::Value) -> [SessionEvent; 2] {
        use agent_client_protocol::schema::v1::{ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields};
        let output = serde_json::json!([{ "type": "text", "text": result.to_string() }]);
        [
            SessionEvent::Update(SessionUpdate::ToolCall(ToolCall::new(id.to_string(), format!("mcp__notebook__{tool}")).raw_input(input))),
            SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id.to_string(), ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(output)))),
        ]
    }

    fn changes(s: &Session) -> Vec<Vec<(String, crate::celldiff::CellChange, usize, usize)>> {
        let cards = s.entries.iter().filter_map(|e| if let Entry::Changes(cells) = e { Some(cells) } else { None });
        cards.map(|cells| cells.iter().map(|c| (c.name.clone(), c.change, c.added, c.removed)).collect()).collect()
    }

    #[test]
    fn a_turn_that_changed_cells_ends_with_its_card() {
        use crate::celldiff::CellChange;
        use serde_json::json;
        const A: &str = "11111111-2222-4333-8444-555555555555";
        const B: &str = "66666666-7777-4888-9999-aaaaaaaaaaaa";
        let mut s = Session::new(1, Place::local("/tmp"), None);
        s.started(Started::new(SessionId::new("abc"), None, None));
        s.submit(text("fit it"), false);
        for event in [
            notebook_call("t1", "read_cell", json!({ "cell_id": A }), json!({ "cell_id": A, "code": "rate = 0.1" })),
            notebook_call("t2", "edit_cell", json!({ "cell_id": A, "code": "rate = 0.2" }), json!({ "ok": true })),
            notebook_call("t3", "add_cell", json!({ "code": "fit = 1" }), json!({ "cell_id": B, "code": "fit = 1" })),
            notebook_call("t4", "edit_cell", json!({ "cell_id": A, "code": "rate = 0.3\nk = 2" }), json!({ "ok": true })),
        ]
        .into_iter()
        .flatten()
        {
            s.apply(event);
        }
        assert!(changes(&s).is_empty(), "no card while the turn runs");
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert!(matches!(s.entries.last(), Some(Entry::Changes(_))), "the card ends the turn");
        assert_eq!(changes(&s), [vec![("rate".to_string(), CellChange::Edited, 2, 1), ("fit".to_string(), CellChange::New, 1, 0)]]);
        s.submit(text("thanks"), false);
        s.apply(SessionEvent::TurnEnded(StopReason::EndTurn));
        assert_eq!(changes(&s).len(), 1, "a turn that changed no cell has no card");
    }

    #[test]
    fn a_reopened_session_shows_each_turns_card() {
        use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, TextContent};
        use crate::celldiff::CellChange;
        use serde_json::json;
        const A: &str = "11111111-2222-4333-8444-555555555555";
        let user = |t: &str| SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(t)))));
        let mut s = Session::loading(1, SessionId::new("abc"), Place::local("/tmp"), None, "Old chat".into());
        s.apply(user("add a cell"));
        for event in notebook_call("t1", "add_cell", json!({ "code": "x = 1" }), json!({ "cell_id": A, "code": "x = 1" })) {
            s.apply(event);
        }
        s.apply(user("now delete it"));
        assert_eq!(changes(&s), [vec![("x".to_string(), CellChange::New, 1, 0)]], "the next message ends the replayed turn");
        assert!(matches!(&s.entries[2], Entry::Changes(_)), "the card sits before the next message");
        for event in notebook_call("t2", "delete_cell", json!({ "cell_id": A }), json!({ "ok": true })) {
            s.apply(event);
        }
        s.started(Started::new(SessionId::new("abc"), None, None));
        assert_eq!(changes(&s)[1], [("x".to_string(), CellChange::Deleted, 0, 1)], "the replay's end ends its last turn");
    }

    #[test]
    fn a_cell_is_named_by_what_it_defines_its_heading_or_its_first_line() {
        use super::cell_label;
        let cases = [
            ("flips = rand(Bool, 100)", "flips"),
            ("# true = heads, false = tails\n\nflips = rand(Bool, 100)", "flips"),
            ("#= Draws\n  x = 1 =#\nn_heads = count(flips)", "n_heads"),
            ("model(S, p) = p[1] * S", "model"),
            ("function fit!(p)\n  p\nend", "fit!"),
            ("const K = 3", "K"),
            ("struct Fit\n  k::Float64\nend", "Fit"),
            ("begin\n  x = 1\n  y = 2\nend", "x"),
            ("md\"\"\"\n# Coin flips\nWe flip a coin.\n\"\"\"", "Coin flips"),
            ("md\"\"\"\nWe flip a fair coin a hundred times.\n\"\"\"", "We flip a fair coin a hundr…"),
            ("md\"## Results\"", "Results"),
            ("scatter(t, counts, label = \"data\")", "scatter(t, counts, label = …"),
            ("let\n  n = 1:length(flips)\n  p = plot(n, share)\n  hline!(p, [0.5])\n  p\nend", "plot"),
            ("let\n  A, k = coef(fit)\n  Markdown.parse(\"k = $k\")\nend", "text"),
            ("let\n  a = sum(xs)\n  b = length(xs)\n  a, b\nend", "a, b"),
            ("let\n  a = sum(xs)\n  a / 2\nend", "let"),
            ("splot(x)", "splot(x)"),
            ("x == 1", "x == 1"),
            ("using Plots", "using Plots"),
            ("# just a note", "cell"),
            ("a_rather_long_variable_name_for_tests = 1", "a_rather_long_variable_name…"),
        ];
        for (code, label) in cases {
            assert_eq!(cell_label(code), label, "{code:?}");
        }
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
        assert_eq!(s.activity(), ("Adding".into(), Some("Fit".into())));
        s.apply(call("t4", "ls", ToolKind::Execute, json!({"command": "ls"})));
        assert_eq!(s.activity(), ("Running a command".into(), None));
        s.apply(call("t5", "ToolSearch", ToolKind::Other, json!({"query": "fetch"})));
        assert_eq!(s.activity(), ("Working".into(), None));
        let done = |id: &str| SessionEvent::Update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id.to_string(), ToolCallUpdateFields::new().status(ToolCallStatus::Completed))));
        s.apply(done("t5"));
        s.apply(done("t4"));
        assert_eq!(s.activity(), ("Adding".into(), Some("Fit".into())), "the latest call still running");
        assert_eq!(crate::transcript::elapsed(12), "12s");
        assert_eq!(crate::transcript::elapsed(65), "1m 05s");
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

        assert_eq!(crate::transcript::thought_label(false, None, Some(Duration::from_millis(8_400))), "Thought for 8s");
        assert_eq!(crate::transcript::thought_label(false, None, Some(Duration::from_millis(200))), "Thought for 1s");
        assert_eq!(crate::transcript::thought_label(false, Some(std::time::Instant::now()), None), "Thinking");
        assert_eq!(crate::transcript::thought_label(true, None, Some(Duration::from_secs(8))), "Thinking", "inside a folded run");
        assert_eq!(crate::transcript::thought_label(false, None, None), "Thought", "replayed");
    }

    #[test]
    fn a_message_sent_now_says_it_joined_or_stopped_the_work() {
        use agent_client_protocol::schema::v1::{Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus};
        let delivery = |s: &Session| match s.entries.last() {
            Some(Entry::User { delivery, .. }) => crate::transcript::delivery_note(*delivery),
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
