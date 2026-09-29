//! Settings: a panel over the dimmed window, with Search and the sections on
//! the left and one section's page on the right. Assistants and languages
//! have their own pages (Claude, Julia), reached from a Settings button.
//!
//! Every page is built as data first (`View`: groups of rows, each with its
//! words and controls), then drawn by one renderer, so every row follows the
//! same rule for text: title, status and description share one left edge,
//! and a radio or icon sits in its own column to the left of it. Search reads
//! the same pages, and so does the debug state dump.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::about::{self, Adapter};
use crate::connection::Status;
use crate::hosts::{Cluster, HostId, Server};
use crate::host_list::HostState;
use crate::new_session::{Glyph, glyph_at};
use crate::runtime::{self, ChosenJulia};
use crate::settings::{Appearance, IdleStop, NotebookTheme};
use crate::signin::{self, Account, Look, Method, Stage};
use crate::{Workspace, overlay, theme};

/// A section in the list on the left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Assistants,
    Notebooks,
    Hosts,
    Appearance,
    Troubleshooting,
    About,
}

impl Section {
    pub const ALL: [Section; 6] = [Section::Assistants, Section::Notebooks, Section::Hosts, Section::Appearance, Section::Troubleshooting, Section::About];

    pub fn name(self) -> &'static str {
        match self {
            Section::Assistants => "Assistants",
            Section::Notebooks => "Notebooks",
            Section::Hosts => "Where notebooks run",
            Section::Appearance => "Appearance",
            Section::Troubleshooting => "Troubleshooting",
            Section::About => "About",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Section::Assistants => "The AI that writes and runs code in your notebooks.",
            Section::Notebooks => "What happens to your notebooks, and the languages they run.",
            Section::Hosts => "On this Mac, or on a server or cluster you connect to over SSH.",
            Section::Appearance => "How Endeavor and your notebooks look.",
            Section::Troubleshooting => "For when something doesn't work. Try these in order.",
            Section::About => "Version, updates and credits.",
        }
    }

    fn glyph(self) -> Glyph {
        match self {
            Section::Assistants => Glyph::Bubble,
            Section::Notebooks => Glyph::File,
            Section::Hosts => Glyph::Server,
            Section::Appearance => Glyph::Contrast,
            Section::Troubleshooting => Glyph::Wrench,
            Section::About => Glyph::Info,
        }
    }

    /// In "Your work"; the rest are under "Endeavor".
    fn yours(self) -> bool {
        matches!(self, Section::Assistants | Section::Notebooks | Section::Hosts)
    }

    pub fn key(self) -> &'static str {
        match self {
            Section::Assistants => "assistants",
            Section::Notebooks => "notebooks",
            Section::Hosts => "hosts",
            Section::Appearance => "appearance",
            Section::Troubleshooting => "troubleshooting",
            Section::About => "about",
        }
    }
}

/// A page: a section's own, or one of its sub-pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Section(Section),
    /// Assistants › Claude.
    Claude,
    /// Notebooks › Julia.
    Julia,
}

impl Page {
    pub const ALL: [Page; 8] = [
        Page::Section(Section::Assistants),
        Page::Claude,
        Page::Section(Section::Notebooks),
        Page::Julia,
        Page::Section(Section::Hosts),
        Page::Section(Section::Appearance),
        Page::Section(Section::Troubleshooting),
        Page::Section(Section::About),
    ];

    pub fn section(self) -> Section {
        match self {
            Page::Section(s) => s,
            Page::Claude => Section::Assistants,
            Page::Julia => Section::Notebooks,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Page::Section(s) => s.key(),
            Page::Claude => "claude",
            Page::Julia => "julia",
        }
    }

    /// A sub-page's name, as its title and in search results' paths.
    fn sub_name(self) -> Option<&'static str> {
        match self {
            Page::Section(_) => None,
            Page::Claude => Some("Claude"),
            Page::Julia => Some("Julia"),
        }
    }
}

/// The open panel.
pub struct Panel {
    pub page: Page,
    pub search: Entity<InputState>,
    /// The row a search result opened, lit until the page changes.
    pub highlight: Option<SharedString>,
    /// The "Stop idle notebooks after" list is open.
    pub idle_menu: bool,
    focus: FocusHandle,
    /// What had focus before the panel opened, to give it back.
    restore: Option<FocusHandle>,
    /// Each control's Tab-stop handle, by its element id: pages are rebuilt each render.
    controls: RefCell<HashMap<SharedString, FocusHandle>>,
}

/// What Settings shows that has to be found out: kept on the workspace, so the
/// Notebooks row and the list's dots can use it too.
#[derive(Default)]
pub struct Checks {
    /// The account `claude auth status` reports, as last read.
    pub profile: Option<signin::Profile>,
    /// The Julia chosen in Settings, and what running it found.
    pub julia: Option<(PathBuf, ChosenJulia)>,
    /// The Julia This Mac's helper was started with, as Settings chose it then:
    /// a different choice since needs a restart.
    pub local_julia: Option<Option<PathBuf>>,
    /// Repair Julia is under way.
    pub repairing: bool,
}

// ---------------------------------------------------------------------------
// Pages as data
// ---------------------------------------------------------------------------

/// A page's content, before drawing.
pub(crate) struct View {
    pub back: Option<Page>,
    pub title: SharedString,
    pub aside: Option<SharedString>,
    pub subtitle: Option<SharedString>,
    pub groups: Vec<Group>,
}

pub(crate) struct Group {
    pub heading: Option<&'static str>,
    pub items: Vec<Item>,
    pub foot: Option<SharedString>,
}

pub(crate) enum Item {
    Row(Row),
    /// Label and value lines (the account).
    Table(Vec<(&'static str, String)>),
    /// Choices drawn as pictures (Appearance).
    Pictures(Vec<Picture>),
    /// The logo, version and credits (About).
    Brand,
    /// A line of links (About).
    Links(Vec<Control>),
    /// "Add server…": a row that is its own button, at a card's foot.
    Add { key: SharedString, label: &'static str, act: Act },
}

#[derive(Default)]
pub(crate) struct Row {
    /// Names the row for ids, search results and the highlight.
    pub key: SharedString,
    pub lead: Lead,
    pub title: SharedString,
    /// Faint words after the title ("by Anthropic", "Slurm").
    pub aside: Option<SharedString>,
    /// The title itself is a state ("Not signed in").
    pub title_tone: Option<Tone>,
    /// An alert symbol after the title.
    pub problem: bool,
    /// A search result's path ("Notebooks › Languages").
    pub crumb: Option<SharedString>,
    pub status: Option<Status2>,
    pub desc: Option<SharedString>,
    pub extra: Option<Extra>,
    pub controls: Vec<Control>,
    /// Greyed: listed, but not available yet.
    pub unavailable: bool,
    /// The whole row acts (a search result).
    pub act: Option<Act>,
    /// What search finds this row by, besides its title and description; None
    /// leaves it out of search.
    pub search: Option<SharedString>,
    /// What a search result says under the title, when not the description's first sentence.
    pub summary: Option<SharedString>,
}

#[derive(Default)]
pub(crate) enum Lead {
    #[default]
    None,
    /// `act` None: can't be picked.
    Radio { checked: bool, act: Option<Act> },
    Icon { glyph: Glyph, dot: bool, faint: bool, tone: Option<Tone> },
}

/// A row's state line: words, and a leading path in code type.
pub(crate) struct Status2 {
    pub mono: Option<String>,
    pub text: String,
    pub tone: Tone,
}

impl Status2 {
    fn new(text: impl Into<String>, tone: Tone) -> Self {
        Status2 { mono: None, text: text.into(), tone }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    /// A plain fact ("Signed in as …").
    Plain,
    /// Something running.
    Running,
    Quiet,
    /// Needs you (orange): signed out, an update.
    Attention,
    /// Broken (red).
    Danger,
}

impl Tone {
    fn color(self) -> Rgba {
        match self {
            Tone::Plain => theme::text_new(),
            Tone::Running => theme::text_secondary(),
            Tone::Quiet => theme::text_muted(),
            Tone::Attention => theme::accent_text(),
            Tone::Danger => theme::danger(),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Tone::Plain => "plain",
            Tone::Running => "running",
            Tone::Quiet => "quiet",
            Tone::Attention => "attention",
            Tone::Danger => "danger",
        }
    }
}

/// More under a row's words, on the same left edge.
pub(crate) enum Extra {
    /// A second line of advice.
    Hint(String),
    /// The chosen Julia: its path in a box, and its version.
    Path { path: String, version: String },
    /// Work under way: what it's doing, how far, and a thin bar.
    Progress { text: String, step: String, fraction: f32 },
}

pub(crate) enum Control {
    Button { label: &'static str, look: Look, icon: Option<Glyph>, act: Option<Act>, aria: SharedString },
    Toggle { on: bool, act: Act, aria: SharedString },
    Select { value: &'static str, act: Act, aria: SharedString },
    Gear { act: Act, aria: SharedString },
    Link { label: &'static str, external: bool, act: Act },
    /// Small faint words ("Not available yet").
    Meta(SharedString),
    /// A fact at the right, as large as a description ("1.12.6 · recommended").
    Note(SharedString),
    /// A result's "opens" chevron; the row itself acts.
    Chevron,
}

pub(crate) struct Picture {
    pub key: SharedString,
    pub thumb: Thumb,
    pub label: &'static str,
    pub desc: Option<&'static str>,
    pub selected: bool,
    pub act: Act,
}

#[derive(Clone, Copy)]
pub enum Thumb {
    Dark,
    Light,
    Match,
    Endeavor,
    Classic,
}

/// What a control does.
#[derive(Clone, PartialEq)]
pub enum Act {
    Go(Page),
    /// A search result: its page, with its row lit.
    Found(Page, SharedString),
    /// The Assistants row's Sign in: the way it was done last, else Claude's page.
    SignInAgain,
    SignIn(Method),
    CancelSignIn,
    ReopenSignIn,
    SignOut,
    OpenClaudeAi,
    PersonalClaude,
    KeepRunning,
    RunWithoutAsking,
    IdleMenu,
    Idle(IdleStop),
    OwnJulia,
    ChooseJulia,
    RestartJulia,
    RepairJulia,
    Stop(HostId),
    TryNow(HostId),
    HostSettings(String),
    AddServer,
    AddCluster,
    Appearance(Appearance),
    Look(NotebookTheme),
    ShowLogs,
    Report,
    Update(about::Action),
    Website,
    Help,
    Licences,
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// A setting search can find.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub page: Page,
    pub key: SharedString,
    /// Where it lives: "Notebooks › Languages".
    pub crumb: String,
    pub title: String,
    /// Its description's first sentence.
    pub summary: Option<String>,
    /// What else it's found by.
    pub words: String,
}

/// The settings matching `query`: each of its words is in the result's path,
/// title, description or other words, ignoring case.
pub fn matches<'a>(entries: &'a [Found], query: &str) -> Vec<&'a Found> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    entries
        .iter()
        .filter(|f| {
            let hay = format!("{} {} {} {}", f.crumb, f.title, f.summary.as_deref().unwrap_or(""), f.words).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// Where each of `query`'s words is in `text`, ignoring case, for marking it.
pub fn marks(text: &str, query: &str) -> Vec<Range<usize>> {
    let lower = text.to_lowercase();
    // Lower-casing can change a character's length; then nothing is marked rather than the wrong letters.
    if lower.len() != text.len() {
        return Vec::new();
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for word in query.split_whitespace().map(str::to_lowercase) {
        let mut from = 0;
        while let Some(at) = lower[from..].find(&word) {
            let start = from + at;
            ranges.push(start..start + word.len());
            from = start + word.len().max(1);
        }
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// A description's first sentence, for a result.
fn first_sentence(text: &str) -> String {
    match text.find(". ") {
        Some(at) => text[..=at].to_owned(),
        None => text.to_owned(),
    }
}

/// "residuals.jl closes. Its file is saved." for Stop's question.
pub fn closes(files: &[String]) -> String {
    match files {
        [] => "No notebooks are open there.".into(),
        [one] => format!("{one} closes. Its file is saved."),
        [rest @ .., last] => format!("{} and {last} close. Their files are saved.", rest.join(", ")),
    }
}

// ---------------------------------------------------------------------------
// Building the pages
// ---------------------------------------------------------------------------

fn row(key: impl Into<SharedString>, title: impl Into<SharedString>) -> Row {
    let title = title.into();
    Row { key: key.into(), search: Some(title.clone()), title, ..Row::default() }
}

fn button(label: &'static str, look: Look, act: Act, aria: impl Into<SharedString>) -> Control {
    Control::Button { label, look, icon: None, act: Some(act), aria: aria.into() }
}

fn group(heading: Option<&'static str>, items: Vec<Item>) -> Group {
    Group { heading, items, foot: None }
}

impl Group {
    fn foot(mut self, text: impl Into<SharedString>) -> Self {
        self.foot = Some(text.into());
        self
    }
}

impl Workspace {
    /// The page's content as data.
    pub(crate) fn settings_view(&self, page: Page) -> View {
        let section = |s: Section, groups| View { back: None, title: s.name().into(), aside: None, subtitle: Some(s.subtitle().into()), groups };
        match page {
            Page::Section(s @ Section::Assistants) => section(s, self.assistants_groups()),
            Page::Section(s @ Section::Notebooks) => section(s, self.notebooks_groups()),
            Page::Section(s @ Section::Hosts) => section(s, self.hosts_groups()),
            Page::Section(s @ Section::Appearance) => section(s, self.appearance_groups()),
            Page::Section(s @ Section::Troubleshooting) => section(s, self.troubleshooting_groups()),
            Page::Section(s @ Section::About) => section(s, self.about_groups()),
            Page::Claude => View { back: Some(Page::Section(Section::Assistants)), title: "Claude".into(), aside: Some("by Anthropic".into()), subtitle: None, groups: self.claude_groups() },
            Page::Julia => View {
                back: Some(Page::Section(Section::Notebooks)),
                title: "Julia".into(),
                aside: Some("Pluto notebooks".into()),
                subtitle: Some("Runs your Pluto notebooks on this Mac.".into()),
                groups: self.julia_groups(),
            },
        }
    }

    fn assistants_groups(&self) -> Vec<Group> {
        let signed_out = self.account.signed_out();
        let mut claude = row("claude", "Claude");
        claude.aside = Some("by Anthropic".into());
        claude.lead = Lead::Radio { checked: true, act: Some(Act::Go(Page::Claude)) };
        claude.search = Some("Claude assistant sign in account".into());
        claude.status = Some(match (&self.account, &self.settings_checks.profile) {
            (Account::SignedOut(Stage::Waiting(_)), _) => Status2::new("Finish signing in in your browser.", Tone::Plain),
            (Account::SignedOut(_), _) => Status2::new("Not signed in. Claude can't answer until you sign in.", Tone::Attention),
            (_, Some(p)) => Status2::new(format!("Signed in as {}", p.email.clone().unwrap_or_else(|| p.plan.clone())), Tone::Plain),
            (Account::SignedIn, None) => Status2::new("Signed in", Tone::Plain),
            (Account::Unknown, None) => Status2::new("Checking the sign-in…", Tone::Plain),
        });
        if signed_out && !matches!(self.account, Account::SignedOut(Stage::Waiting(_))) {
            claude.controls.push(button("Sign in", Look::Primary, Act::SignInAgain, "Sign in to Claude"));
        }
        claude.controls.push(Control::Button { label: "Settings", look: Look::Secondary, icon: Some(Glyph::Gear), act: Some(Act::Go(Page::Claude)), aria: "Claude settings".into() });
        let later = |key: &'static str, name: &'static str, by: &'static str| {
            let mut r = row(key, name);
            r.aside = Some(by.into());
            r.lead = Lead::Radio { checked: false, act: None };
            r.unavailable = true;
            r.search = None;
            r.controls.push(Control::Meta("Not available yet".into()));
            Item::Row(r)
        };
        vec![
            group(
                Some("New sessions use"),
                vec![Item::Row(claude), later("cursor", "Cursor", "by Anysphere"), later("codex", "Codex", "by OpenAI"), later("gemini", "Gemini", "by Google")],
            )
            .foot("Pick one for new sessions. Settings opens that assistant's sign-in and options."),
        ]
    }

    fn claude_groups(&self) -> Vec<Group> {
        let setup = {
            let mut r = row("personal-claude", "Use my Claude Code setup");
            r.desc = Some("New sessions also load your own Claude Code settings and the tools you connected to it. Leave this off if you only use Claude here.".into());
            r.controls.push(Control::Toggle { on: self.settings.personal_claude, act: Act::PersonalClaude, aria: "Use my Claude Code setup".into() });
            group(Some("If you also use Claude Code"), vec![Item::Row(r)])
        };
        let account = match &self.account {
            Account::SignedOut(stage) => {
                let mut state = row("not-signed-in", "Not signed in");
                state.title_tone = Some(Tone::Attention);
                state.search = None;
                state.desc = Some("Claude can't answer until you sign in. Your notebooks still work.".into());
                let mut items = vec![Item::Row(state)];
                match stage {
                    Stage::Waiting(login) => {
                        let mut waiting = row("sign-in-waiting", "Finish signing in in your browser");
                        waiting.search = None;
                        waiting.desc = Some(format!("We opened {}. Sign in there, then come back. This page moves on by itself.", site(login.method)).into());
                        waiting.controls.push(Control::Link { label: "Open it again", external: false, act: Act::ReopenSignIn });
                        waiting.controls.push(button("Cancel", Look::Secondary, Act::CancelSignIn, "Cancel signing in"));
                        items.push(Item::Row(waiting));
                    }
                    _ => {
                        if let Stage::Failed { reason, .. } = stage {
                            let mut failed = row("sign-in-failed", "Sign-in didn't finish");
                            failed.search = None;
                            failed.title_tone = Some(Tone::Danger);
                            failed.problem = true;
                            failed.desc = Some(reason.says().0.into());
                            items.push(Item::Row(failed));
                        }
                        let mut plan = row("sign-in-claude", "With a Claude plan");
                        plan.desc = Some("Pro, Max, Team or Enterprise. You pay monthly and chat with Claude at claude.ai.".into());
                        plan.controls.push(button("Sign in", Look::Primary, Act::SignIn(Method::ClaudeAi), "Sign in with a Claude plan"));
                        let mut console = row("sign-in-console", "With an Anthropic Console account");
                        console.desc = Some("You or your lab pay for what you use, at console.anthropic.com.".into());
                        console.controls.push(button("Sign in", Look::Secondary, Act::SignIn(Method::Console), "Sign in with an Anthropic Console account"));
                        items.push(Item::Row(plan));
                        items.push(Item::Row(console));
                    }
                }
                group(Some("Account"), items).foot("Sign-in opens in your browser. Your password stays there.")
            }
            _ => {
                let mut items = Vec::new();
                if let Some(p) = &self.settings_checks.profile {
                    let mut lines = Vec::new();
                    if let Some(email) = &p.email {
                        lines.push(("Signed in as", email.clone()));
                    }
                    lines.push(("Plan", p.plan.clone()));
                    if let Some(org) = &p.org {
                        lines.push(("Organisation", org.clone()));
                    }
                    items.push(Item::Table(lines));
                }
                let mut usage = row("plan-and-usage", "Plan and usage");
                usage.desc = Some("See your plan, and how much of it you've used.".into());
                usage.controls.push(Control::Link { label: "Open claude.ai", external: true, act: Act::OpenClaudeAi });
                let mut out = row("sign-out", "Sign out");
                out.desc = Some("Claude stops answering in every session until you sign in again.".into());
                out.controls.push(button("Sign out", Look::Secondary, Act::SignOut, "Sign out of Claude"));
                items.push(Item::Row(usage));
                items.push(Item::Row(out));
                group(Some("Account"), items)
            }
        };
        vec![account, setup]
    }

    /// What the Notebooks page's Julia row says: Endeavor's, or the chosen one and its version, or what's wrong with it.
    fn julia_line(&self) -> Status2 {
        match (&self.settings.julia, &self.settings_checks.julia) {
            (None, _) => Status2::new(format!("Endeavor's Julia {}", runtime::JULIA_VERSION), Tone::Plain),
            (Some(path), Some((checked, ChosenJulia::Usable(version)))) if checked == path => {
                Status2::new(format!("{} · {version}", crate::new_session::tilde(path)), Tone::Plain)
            }
            (Some(path), Some((checked, problem))) if checked == path => Status2 { mono: Some(crate::new_session::tilde(path)), text: problem_text(problem), tone: Tone::Danger },
            (Some(path), _) => Status2::new(crate::new_session::tilde(path), Tone::Plain),
        }
    }

    /// The chosen Julia can't be used, so Endeavor's runs instead.
    pub fn julia_broken(&self) -> bool {
        matches!((&self.settings.julia, &self.settings_checks.julia), (Some(path), Some((checked, check))) if checked == path && !matches!(check, ChosenJulia::Usable(_)))
    }

    fn notebooks_groups(&self) -> Vec<Group> {
        let s = &self.settings;
        let idle_label = IdleStop::ALL.iter().find(|(v, _)| *v == s.idle_stop).map_or("48 hours", |(_, l)| l);
        let mut idle = row("idle-stop", "Stop idle notebooks after");
        idle.desc = Some("A notebook nobody has used for this long stops, even with Endeavor open, to free memory. Start brings it back.".into());
        idle.controls.push(Control::Select { value: idle_label, act: Act::IdleMenu, aria: format!("Stop idle notebooks after: {idle_label}").into() });
        let mut keep = row("keep-running", "Keep notebooks running after Endeavor quits");
        keep.desc = Some("They carry on while Endeavor is closed, and it reconnects when you open it. Idle ones still stop.".into());
        keep.controls.push(Control::Toggle { on: s.keep_running, act: Act::KeepRunning, aria: "Keep notebooks running after Endeavor quits".into() });
        let mut ask = row("run-without-asking", "Run notebook code without asking");
        ask.desc = Some("When on, the assistant runs the cells it writes without stopping to ask you first. It applies to new sessions. To change one session, use Ask to run under the message box.".into());
        ask.controls.push(Control::Toggle { on: s.run_without_asking, act: Act::RunWithoutAsking, aria: "Run notebook code without asking".into() });
        let mut julia = row("julia", "Julia");
        julia.aside = Some("Pluto notebooks".into());
        julia.lead = Lead::Icon { glyph: Glyph::File, dot: false, faint: false, tone: None };
        julia.problem = self.julia_broken();
        julia.status = Some(self.julia_line());
        julia.desc = None;
        julia.search = Some("Julia Pluto notebooks".into());
        julia.summary = Some(format!("{}. Choose which program runs your Pluto notebooks.", self.julia_line().text).into());
        julia.controls.push(Control::Button { label: "Settings", look: Look::Secondary, icon: Some(Glyph::Gear), act: Some(Act::Go(Page::Julia)), aria: "Julia settings".into() });
        let later = |key: &'static str, name: &'static str, kind: &'static str| {
            let mut r = row(key, name);
            r.aside = Some(kind.into());
            r.lead = Lead::Icon { glyph: Glyph::File, dot: false, faint: true, tone: None };
            r.unavailable = true;
            r.search = None;
            r.controls.push(Control::Meta("Not available yet".into()));
            Item::Row(r)
        };
        vec![
            group(Some("When notebooks stop"), vec![Item::Row(idle), Item::Row(keep)]),
            group(Some("Running code"), vec![Item::Row(ask)]),
            group(Some("Languages"), vec![Item::Row(julia), later("r", "R", "Ember notebooks"), later("python", "Python", "marimo notebooks")])
                .foot("Each language has its own program. Servers and clusters set theirs in Where notebooks run."),
        ]
    }

    /// This Mac's open notebooks' file names.
    pub fn open_notebook_files(&self, host: &HostId) -> Vec<String> {
        let Some(c) = self.connections.get(host).filter(|c| c.status == Status::Ready) else { return Vec::new() };
        c.notebooks
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|nb| nb["path"].as_str())
            .map(|p| Path::new(p).file_name().map_or(p.to_owned(), |n| n.to_string_lossy().into_owned()))
            .collect()
    }

    /// The chosen Julia differs from the one This Mac's Julia was started with.
    fn julia_needs_restart(&self) -> bool {
        self.status(&HostId::ThisMac) == Some(&Status::Ready) && self.julia_changed()
    }

    /// The Julia This Mac's helper runs isn't the one chosen now. A chosen
    /// file known to be broken counts as Endeavor's, which runs in its place.
    pub fn julia_changed(&self) -> bool {
        let runs = |choice: &Option<PathBuf>| {
            choice.clone().filter(|p| !matches!(&self.settings_checks.julia, Some((checked, check)) if checked == p && !matches!(check, ChosenJulia::Usable(_))))
        };
        self.settings_checks.local_julia.as_ref().is_some_and(|started| runs(started) != runs(&self.settings.julia))
    }

    fn julia_groups(&self) -> Vec<Group> {
        let own = self.settings.julia.is_none();
        let mut ours = row("julia-own", "Endeavor's Julia");
        ours.lead = Lead::Radio { checked: own, act: Some(Act::OwnJulia) };
        ours.desc = Some("Installed and kept up to date by Endeavor.".into());
        // Search finds the choice by its other half, "Another Julia on this Mac".
        ours.search = None;
        ours.controls.push(Control::Note(format!("{} · recommended", runtime::JULIA_VERSION).into()));
        let mut theirs = row("julia-another", "Another Julia on this Mac");
        theirs.lead = Lead::Radio { checked: !own, act: Some(Act::ChooseJulia) };
        theirs.summary = Some("Use a Julia you installed yourself.".into());
        let broken = self.julia_broken();
        let label = if !own && !broken { "Change…" } else { "Choose…" };
        theirs.controls.push(button(label, Look::Secondary, Act::ChooseJulia, "Choose another Julia"));
        match (&self.settings.julia, &self.settings_checks.julia) {
            (None, _) => theirs.desc = Some("If you already have Julia installed and want to use it.".into()),
            (Some(path), Some((checked, ChosenJulia::Usable(version)))) if checked == path => {
                theirs.extra = Some(Extra::Path { path: crate::new_session::tilde(path), version: format!("Julia {version}") });
            }
            (Some(path), Some((checked, problem))) if checked == path => {
                theirs.problem = true;
                theirs.status = Some(Status2 { mono: Some(crate::new_session::tilde(path)), text: problem_text(problem), tone: Tone::Danger });
            }
            (Some(path), _) => theirs.extra = Some(Extra::Path { path: crate::new_session::tilde(path), version: "Checking…".into() }),
        }
        let mut items = vec![Item::Row(ours), Item::Row(theirs)];
        let restart = self.julia_needs_restart();
        if restart {
            let n = self.open_notebook_files(&HostId::ThisMac).len();
            let mut r = row("julia-restart", "Restart Julia to use it");
            r.search = None;
            r.lead = Lead::Icon { glyph: Glyph::Info, dot: false, faint: false, tone: Some(Tone::Attention) };
            r.desc = Some(
                match n {
                    0 => "Julia stops and starts again with the one you chose.".to_owned(),
                    1 => "Your open notebook stops and starts again with the new Julia. Its file is already saved.".to_owned(),
                    n => format!("Your {n} open notebooks stop and start again with the new Julia. Their files are already saved."),
                }
                .into(),
            );
            r.controls.push(Control::Button { label: "Restart Julia", look: Look::Primary, icon: Some(Glyph::Restart), act: Some(Act::RestartJulia), aria: "Restart Julia".into() });
            items.push(Item::Row(r));
        }
        let g = group(Some("Which Julia to use"), items);
        vec![if restart { g } else { g.foot("Changing it takes effect when Julia restarts.") }]
    }

    fn hosts_groups(&self) -> Vec<Group> {
        let this_mac = Item::Row(self.host_row(HostId::ThisMac, "This Mac".into(), None));
        let rows = |clusters: bool| -> Vec<Item> {
            self.hosts
                .servers
                .iter()
                .filter(|s| s.cluster.is_some() == clusters)
                .map(|s| Item::Row(self.host_row(HostId::Server(s.id.clone()), s.name.clone().into(), clusters.then_some("Slurm"))))
                .collect()
        };
        let mut servers = rows(false);
        servers.push(Item::Add { key: "add-server".into(), label: "Add server…", act: Act::AddServer });
        let mut clusters = rows(true);
        clusters.push(Item::Add { key: "add-cluster".into(), label: "Add cluster…", act: Act::AddCluster });
        vec![
            group(Some("This Mac"), vec![this_mac]),
            group(Some("Servers"), servers),
            group(Some("Clusters"), clusters).foot("The gear sets a host's connection, which programs to use, and for a cluster its account and default resources."),
        ]
    }

    fn host_row(&self, host: HostId, name: SharedString, kind: Option<&'static str>) -> Row {
        let state = self.host_state(&host);
        let glyph = match (&host, kind) {
            (HostId::ThisMac, _) => Glyph::Laptop,
            (_, Some(_)) => Glyph::Cluster,
            _ => Glyph::Server,
        };
        let mut r = row(host_key(&host), name.clone());
        r.aside = kind.map(Into::into);
        r.lead = Lead::Icon { glyph, dot: state.running(), faint: false, tone: None };
        let (text, hint) = state.words();
        r.status = Some(Status2::new(text, if state.running() { Tone::Running } else { Tone::Quiet }));
        r.extra = hint.map(Extra::Hint);
        r.search = Some(format!("{name} {}", kind.unwrap_or(if host == HostId::ThisMac { "" } else { "server" })).into());
        match state.action() {
            Some(crate::host_list::HostAct::Stop) => r.controls.push(button("Stop", Look::Secondary, Act::Stop(host.clone()), format!("Stop Julia on {name}"))),
            Some(crate::host_list::HostAct::CancelJob) => r.controls.push(button("Cancel job", Look::Secondary, Act::Stop(host.clone()), format!("Cancel the job on {name}"))),
            Some(crate::host_list::HostAct::TryNow) => r.controls.push(button("Try now", Look::Secondary, Act::TryNow(host.clone()), format!("Try {name} now"))),
            None => {}
        }
        if let HostId::Server(id) = &host {
            r.controls.push(Control::Gear { act: Act::HostSettings(id.clone()), aria: format!("{name} settings").into() });
        }
        r
    }

    fn appearance_groups(&self) -> Vec<Group> {
        let s = &self.settings;
        let pic = |key: &'static str, thumb, label, desc, selected, act| Picture { key: key.into(), thumb, label, desc, selected, act };
        vec![
            group(
                Some("Light or dark"),
                vec![Item::Pictures(vec![
                    pic("look-dark", Thumb::Dark, "Dark", None, s.appearance == Appearance::Dark, Act::Appearance(Appearance::Dark)),
                    pic("look-light", Thumb::Light, "Light", None, s.appearance == Appearance::Light, Act::Appearance(Appearance::Light)),
                    pic("look-system", Thumb::Match, "Match macOS", None, s.appearance == Appearance::System, Act::Appearance(Appearance::System)),
                ])],
            )
            .foot("For now only the notebook changes. The rest of Endeavor stays dark."),
            group(
                Some("Pluto notebook look"),
                vec![Item::Pictures(vec![
                    pic("look-endeavor", Thumb::Endeavor, "Endeavor", Some("Pluto's controls sit in the pane's header, in Endeavor's colours."), s.notebook_theme == NotebookTheme::Endeavor, Act::Look(NotebookTheme::Endeavor)),
                    pic("look-classic", Thumb::Classic, "Pluto classic", Some("Pluto's own page, as it looks outside Endeavor."), s.notebook_theme == NotebookTheme::Pluto, Act::Look(NotebookTheme::Pluto)),
                ])],
            )
            .foot("You can also switch from the notebook's ⋮ menu. Shared and exported notebooks always use Pluto's standard look."),
        ]
    }

    /// Repair's progress: the step it's on, its number of all, and how far along.
    fn repair_progress(&self) -> Option<(String, usize, usize)> {
        if !self.settings_checks.repairing {
            return None;
        }
        let c = self.connections.get(&HostId::ThisMac)?;
        if !matches!(c.status, Status::Starting | Status::Connecting) && !matches!(&c.status, Status::Failed(e) if e.is_empty()) {
            return None;
        }
        let done = c.steps.done.len();
        let total = done + 1 + c.steps.pending().len();
        Some((c.steps.current.clone(), done + 1, total))
    }

    fn troubleshooting_groups(&self) -> Vec<Group> {
        let repair = self.repair_progress();
        let ready = self.status(&HostId::ThisMac) == Some(&Status::Ready);
        let mut restart = row("restart-julia", "Restart Julia");
        restart.desc = Some(
            if repair.is_some() {
                "Not available while Julia is being repaired."
            } else if ready {
                "Stops your Julia notebooks on this Mac and starts them again. Their files are already saved."
            } else {
                "Julia isn't running on this Mac now. It starts when you open a session."
            }
            .into(),
        );
        restart.summary = Some("Stops your Julia notebooks on this Mac and starts them again.".into());
        restart.controls.push(Control::Button { label: "Restart", look: Look::Secondary, icon: Some(Glyph::Restart), act: (ready && repair.is_none()).then_some(Act::RestartJulia), aria: "Restart Julia".into() });
        let mut fix = row("repair-julia", "Repair Julia");
        fix.summary = Some("If Julia notebooks won't start or keep failing.".into());
        match repair {
            Some((step, n, total)) => {
                fix.desc = Some("Your notebooks open again when it's done.".into());
                let fraction = (n - 1) as f32 / total.max(1) as f32;
                fix.extra = Some(Extra::Progress { text: format!("Repairing · {}", lower_first(&step)), step: format!("step {n} of {total}"), fraction: fraction.max(0.08) });
                fix.controls.push(Control::Button { label: "Repair…", look: Look::Secondary, icon: None, act: None, aria: "Repair Julia".into() });
            }
            None => {
                fix.desc = Some("If Julia notebooks won't start or keep failing. Endeavor clears what Julia saved and compiled, then starts it again. It takes a few minutes. Your notebooks, packages and settings stay.".into());
                fix.controls.push(button("Repair…", Look::Secondary, Act::RepairJulia, "Repair Julia"));
            }
        }
        let mut logs = row("log-files", "Log files");
        logs.desc = Some("What Endeavor did this time and the time before. Attach them when you report a problem.".into());
        logs.controls.push(Control::Button { label: "Show in Finder", look: Look::Secondary, icon: Some(Glyph::Finder), act: Some(Act::ShowLogs), aria: "Show log files in Finder".into() });
        let mut report = row("report", "Report a problem");
        report.desc = Some("Opens a new issue on GitHub, where you can describe what happened.".into());
        report.controls.push(Control::Link { label: "Report", external: true, act: Act::Report });
        vec![
            group(Some("Julia on this Mac"), vec![Item::Row(restart), Item::Row(fix)]).foot("R and Python get their own group here when they arrive."),
            group(Some("Report a problem"), vec![Item::Row(logs), Item::Row(report)]),
        ]
    }

    fn about_groups(&self) -> Vec<Group> {
        let rows = about::parts(&self.updates())
            .into_iter()
            .map(|part| {
                let mut r = row(part.key, part.name);
                r.status = Some(Status2::new(part.state, if part.attention { Tone::Attention } else { Tone::Quiet }));
                if let Some((label, action, primary)) = part.button {
                    r.controls.push(button(label, if primary { Look::Primary } else { Look::Secondary }, Act::Update(action), format!("{label}: {}", part.name)));
                }
                Item::Row(r)
            })
            .collect();
        vec![
            group(None, vec![Item::Brand]),
            group(Some("Updates"), rows).foot("The adapter is how Endeavor talks to Claude. The About window shows the same updates."),
            group(
                None,
                vec![Item::Links(vec![
                    Control::Link { label: "Website", external: true, act: Act::Website },
                    Control::Link { label: "Endeavor Help", external: true, act: Act::Help },
                    Control::Link { label: "Licences", external: false, act: Act::Licences },
                ])],
            ),
        ]
    }

    /// Everything search can find, from the pages as they are now.
    pub fn search_index(&self) -> Vec<Found> {
        let mut found = Vec::new();
        for page in Page::ALL {
            let view = self.settings_view(page);
            for g in &view.groups {
                let crumb = match (page.sub_name(), g.heading) {
                    (Some(sub), _) => format!("{} › {sub}", page.section().name()),
                    (None, Some(heading)) => format!("{} › {heading}", page.section().name()),
                    (None, None) => page.section().name().to_owned(),
                };
                for item in &g.items {
                    match item {
                        Item::Row(r) => {
                            let Some(words) = &r.search else { continue };
                            found.push(Found {
                                page,
                                key: r.key.clone(),
                                crumb: crumb.clone(),
                                title: r.title.to_string(),
                                summary: r.summary.as_ref().map(ToString::to_string).or_else(|| r.desc.as_ref().map(|d| first_sentence(d))),
                                words: words.to_string(),
                            });
                        }
                        Item::Pictures(pictures) => found.extend(pictures.iter().map(|p| Found {
                            page,
                            key: p.key.clone(),
                            crumb: crumb.clone(),
                            title: p.label.to_owned(),
                            summary: p.desc.map(str::to_owned),
                            words: String::new(),
                        })),
                        Item::Links(links) => found.extend(links.iter().filter_map(|l| match l {
                            Control::Link { label, .. } => Some(Found { page, key: format!("link-{label}").into(), crumb: crumb.clone(), title: (*label).to_owned(), summary: None, words: String::new() }),
                            _ => None,
                        })),
                        _ => {}
                    }
                }
            }
        }
        found
    }

    /// The results page.
    fn results_view(&self, query: &str) -> (View, Vec<(Section, usize)>) {
        let index = self.search_index();
        let hits = matches(&index, query);
        let mut counts: Vec<(Section, usize)> = Vec::new();
        for hit in &hits {
            match counts.iter_mut().find(|(s, _)| *s == hit.page.section()) {
                Some((_, n)) => *n += 1,
                None => counts.push((hit.page.section(), 1)),
            }
        }
        let title = match hits.len() {
            0 => format!("No results for “{}”", query.trim()),
            1 => format!("1 result for “{}”", query.trim()),
            n => format!("{n} results for “{}”", query.trim()),
        };
        let rows: Vec<Item> = hits
            .iter()
            .map(|hit| {
                let mut r = row(format!("result-{}-{}", hit.page.key(), hit.key), hit.title.clone());
                r.crumb = Some(hit.crumb.clone().into());
                r.desc = hit.summary.clone().map(Into::into);
                r.act = Some(Act::Found(hit.page, hit.key.clone()));
                r.controls.push(Control::Chevron);
                Item::Row(r)
            })
            .collect();
        let groups = if rows.is_empty() {
            vec![group(None, Vec::new()).foot("Try another word. Esc clears the search; esc again closes Settings.")]
        } else {
            vec![group(None, rows).foot("A result opens its page with the setting highlighted. Esc clears the search; esc again closes Settings.")]
        };
        (View { back: None, title: title.into(), aside: None, subtitle: None, groups }, counts)
    }

    /// A section in the list gets a dot: something there needs you.
    fn needs_you(&self, section: Section) -> bool {
        match section {
            Section::Assistants => self.account.signed_out() && self.setup.is_none(),
            Section::Notebooks => self.julia_broken(),
            Section::About => update_available(&self.updates()),
            _ => false,
        }
    }
}

pub fn update_available(updates: &about::Updates) -> bool {
    updates.app.is_some() || matches!(updates.adapter, Adapter::Available(_))
}

fn problem_text(problem: &ChosenJulia) -> String {
    match problem {
        ChosenJulia::Gone => " isn't there any more. Notebooks use Endeavor's Julia until you choose another.".into(),
        ChosenJulia::NotJulia => " isn't Julia. Choose the file named julia, in a bin folder. Notebooks use Endeavor's Julia until then.".into(),
        ChosenJulia::TooOld(v) => format!(" is Julia {v}; Endeavor needs {} or newer. Notebooks use Endeavor's Julia until you choose another.", runtime::min_julia()),
        ChosenJulia::Usable(v) => format!(" · {v}"),
    }
}

/// A host's row in Where notebooks run.
fn host_key(host: &HostId) -> SharedString {
    match host {
        HostId::ThisMac => "host-this-mac".into(),
        HostId::Server(id) => format!("host-{id}").into(),
    }
}

fn site(method: Method) -> &'static str {
    match method {
        Method::ClaudeAi => "claude.ai",
        Method::Console => "console.anthropic.com",
    }
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Opening, closing, acting
// ---------------------------------------------------------------------------

impl Workspace {
    /// Open Settings at `page` (or bring it there if open).
    pub fn open_settings_at(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = &mut self.settings_panel {
            panel.page = page;
            panel.highlight = None;
            panel.idle_menu = false;
            return cx.notify();
        }
        self.close_dismissible(window, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(panel) = &mut this.settings_panel
            {
                panel.idle_menu = false;
                cx.notify();
            }
        })
        .detach();
        let focus = cx.focus_handle();
        let restore = window.focused(cx);
        window.focus(&focus, cx);
        self.settings_panel = Some(Panel { page, search, highlight: None, idle_menu: false, focus, restore, controls: RefCell::new(HashMap::new()) });
        // Settings shows every host as it is now, the account, and whether the chosen Julia still works.
        let hosts: Vec<HostId> = std::iter::once(HostId::ThisMac).chain(self.hosts.servers.iter().map(|s| HostId::Server(s.id.clone()))).collect();
        for host in hosts {
            self.check_host(&host, cx);
        }
        self.refresh_profile(cx);
        self.check_chosen_julia(cx);
        cx.notify();
    }

    /// Open Settings at a host's row in Where notebooks run, lit ("Can't reach lab-server").
    pub fn open_settings_at_host(&mut self, host: &HostId, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_at(Page::Section(Section::Hosts), window, cx);
        if let Some(panel) = &mut self.settings_panel {
            panel.highlight = Some(host_key(host));
        }
    }

    /// Open Settings where it was last left (⌘, and the sidebar's gear).
    pub fn open_settings_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.settings_page;
        self.open_settings_at(page, window, cx);
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.settings_panel.take() else { return };
        self.settings_page = panel.page;
        if let Some(restore) = panel.restore {
            window.focus(&restore, cx);
        }
        cx.notify();
    }

    /// Esc: closes the idle list, then clears the search, then closes Settings.
    /// False when Settings isn't open.
    pub fn settings_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // A server's dialog opened from here is on top: Esc isn't Settings' then.
        if self.server_dialog.is_some() || !self.asks.is_empty() {
            return self.settings_panel.is_some();
        }
        let Some(panel) = &mut self.settings_panel else { return false };
        if panel.idle_menu {
            panel.idle_menu = false;
        } else if !panel.search.read(cx).value().is_empty() {
            let search = panel.search.clone();
            search.update(cx, |s, cx| s.set_value("", window, cx));
            let focus = panel.focus.clone();
            window.focus(&focus, cx);
        } else {
            self.close_settings(window, cx);
        }
        cx.notify();
        true
    }

    /// ⌘F while Settings is open.
    pub fn focus_settings_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = &self.settings_panel {
            let search = panel.search.clone();
            search.update(cx, |s, cx| s.focus(window, cx));
        }
    }

    /// Read the account again (Settings opened, signed in or out).
    pub fn refresh_profile(&mut self, cx: &mut Context<Self>) {
        let read = cx.background_executor().spawn(async { signin::profile() });
        cx.spawn(async move |this, cx| {
            let profile = read.await;
            let _ = this.update(cx, |this, cx| {
                let Ok(profile) = profile else { return };
                // As the window's own check: only a sign-out is taken from it.
                if profile.is_none() && matches!(this.account, Account::SignedIn) {
                    this.signed_out(cx);
                }
                this.settings_checks.profile = profile;
                cx.notify();
            });
        })
        .detach();
    }

    /// Run the chosen Julia once to read its version.
    pub fn check_chosen_julia(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.settings.julia.clone() else {
            self.settings_checks.julia = None;
            return;
        };
        let check = cx.background_executor().spawn({
            let path = path.clone();
            async move { runtime::check_chosen(&path) }
        });
        cx.spawn(async move |this, cx| {
            let found = check.await;
            let _ = this.update(cx, |this, cx| {
                this.settings_checks.julia = Some((path, found));
                cx.notify();
            });
        })
        .detach();
    }

    /// Choose another Julia: the file picker, or in debug builds the path in
    /// the file `ENDEAVOR_TEST_PICK_JULIA` names (so tests never open the picker).
    fn choose_julia(&mut self, cx: &mut Context<Self>) {
        #[cfg(debug_assertions)]
        if let Some(file) = std::env::var_os("ENDEAVOR_TEST_PICK_JULIA")
            && let Ok(text) = std::fs::read_to_string(&file)
        {
            return self.set_julia(Some(PathBuf::from(text.trim())), cx);
        }
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Choose".into()) });
        crate::platform::set_open_panel_message("Choose the julia program. It's in the bin folder of a Julia install.", cx);
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = picked.await {
                let _ = this.update(cx, |this, cx| this.set_julia(paths.pop(), cx));
            }
        })
        .detach();
    }

    fn set_julia(&mut self, julia: Option<PathBuf>, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| s.julia = julia);
        self.check_chosen_julia(cx);
    }

    /// Ask before stopping Julia on `host`, or cancelling its queued job.
    fn confirm_host_stop(&mut self, host: HostId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.hosts.name(&host);
        let (title, detail, yes) = match self.host_state(&host) {
            HostState::Queued { starting: false, .. } => (format!("Cancel the job on {name}?"), "It's still waiting in the queue, so nothing is lost.".to_owned(), "Cancel job"),
            _ => (format!("Stop Julia on {name}?"), closes(&self.open_notebook_files(&host)), "Stop"),
        };
        let answer = window.prompt(PromptLevel::Warning, &title, Some(&detail), &[PromptButton::cancel("Cancel"), PromptButton::new(yes)], cx);
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| this.stop_host(&host, cx));
            }
        })
        .detach();
    }

    fn confirm_repair(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let detail = match self.open_notebook_files(&HostId::ThisMac).len() {
            0 => "Julia is cleared and starts again. This takes a few minutes.".to_owned(),
            1 => "Your open notebook closes while Julia is cleared and starts again. This takes a few minutes.".to_owned(),
            n => format!("Your {n} open notebooks close while Julia is cleared and starts again. This takes a few minutes."),
        };
        let detail = format!("{detail}\n\nYour notebooks, packages and settings stay.");
        let answer = window.prompt(PromptLevel::Warning, "Repair Julia on this Mac?", Some(&detail), &[PromptButton::cancel("Cancel"), PromptButton::new("Repair")], cx);
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| this.repair_local(cx));
            }
        })
        .detach();
    }

    pub fn settings_act(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = &mut self.settings_panel {
            panel.idle_menu = matches!(act, Act::IdleMenu) && !panel.idle_menu;
        }
        match act {
            Act::Go(page) => self.settings_go(page, None, window, cx),
            Act::Found(page, key) => self.settings_go(page, Some(key), window, cx),
            Act::SignInAgain => match self.settings.sign_in_method {
                Some(method) => self.begin_sign_in(method, cx),
                None => self.settings_go(Page::Claude, None, window, cx),
            },
            Act::SignIn(method) => self.begin_sign_in(method, cx),
            Act::CancelSignIn => drop(self.cancel_sign_in(cx)),
            Act::ReopenSignIn => self.open_sign_in_again(cx),
            Act::SignOut => self.sign_out_of_claude(window, cx),
            Act::OpenClaudeAi => cx.open_url("https://claude.ai/settings/usage"),
            Act::PersonalClaude => self.update_settings(cx, |s| s.personal_claude = !s.personal_claude),
            Act::KeepRunning => self.update_settings(cx, |s| s.keep_running = !s.keep_running),
            Act::RunWithoutAsking => self.update_settings(cx, |s| s.run_without_asking = !s.run_without_asking),
            Act::IdleMenu => {}
            Act::Idle(value) => {
                self.update_settings(cx, |s| s.idle_stop = value);
                let hosts: Vec<HostId> = self.connections.keys().cloned().collect();
                for host in hosts {
                    self.send_idle_limit(&host, cx);
                }
            }
            Act::OwnJulia => self.set_julia(None, cx),
            Act::ChooseJulia => self.choose_julia(cx),
            Act::RestartJulia => self.restart_local(cx),
            Act::RepairJulia => self.confirm_repair(window, cx),
            Act::Stop(host) => self.confirm_host_stop(host, window, cx),
            Act::TryNow(host) => match self.host_state(&host) {
                HostState::Lost => self.reconnect_lost(&host, cx),
                _ => self.check_host(&host, cx),
            },
            Act::HostSettings(id) => self.open_server_dialog(Some(id), window, cx),
            Act::AddServer => self.open_new_host(Server::default(), window, cx),
            Act::AddCluster => self.open_new_host(Server { cluster: Some(Cluster::default()), ..Default::default() }, window, cx),
            Act::Appearance(value) => {
                self.update_settings(cx, |s| s.appearance = value);
                self.apply_look(cx);
            }
            Act::Look(value) => {
                self.update_settings(cx, |s| s.notebook_theme = value);
                self.apply_look(cx);
            }
            Act::ShowLogs => crate::logs::reveal(),
            Act::Report => cx.open_url(about::REPORT_ISSUE),
            Act::Update(action) => match action {
                about::Action::CheckNow => {}
                about::Action::Restart => cx.restart(),
                about::Action::UpdateAdapter => self.update_adapter(cx),
            },
            Act::Website => cx.open_url(about::WEBSITE),
            Act::Help => cx.open_url(about::HELP),
            Act::Licences => about::open_licences(cx),
        }
        cx.notify();
    }

    /// Show `page`, with `highlight`'s row lit (a search result's); leaves the search.
    fn settings_go(&mut self, page: Page, highlight: Option<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = &mut self.settings_panel else { return };
        panel.page = page;
        panel.highlight = highlight;
        let search = panel.search.clone();
        if !search.read(cx).value().is_empty() {
            search.update(cx, |s, cx| s.set_value("", window, cx));
        }
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn scrim() -> Rgba {
    rgba(0x08080A9E)
}

fn nav_bg() -> Rgba {
    rgb(0x1A1A1D)
}

fn icon_grey() -> Rgba {
    rgb(0x999999)
}

/// A searched word in a result, as the boards mark it.
fn mark_bg() -> Rgba {
    rgba(0xCC3F0047)
}

/// The row a result opened: a quiet tint, so the row still reads.
fn lit_bg() -> Rgba {
    rgba(0xCC3F001A)
}

fn ring(d: Stateful<Div>) -> Stateful<Div> {
    d.tab_stop(true).focus_visible(|s| s.border_2().border_color(theme::focus_ring()))
}

impl Workspace {
    fn control_focus(&self, id: &SharedString, cx: &App) -> Option<FocusHandle> {
        let panel = self.settings_panel.as_ref()?;
        Some(panel.controls.borrow_mut().entry(id.clone()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone())
    }

    /// A focusable, labelled control frame.
    fn focusable(&self, id: SharedString, role: Role, aria: SharedString, cx: &App) -> Stateful<Div> {
        let d = div().id(ElementId::Name(id.clone())).role(role).aria_label(aria);
        match self.control_focus(&id, cx) {
            Some(focus) => ring(d.track_focus(&focus)),
            None => d,
        }
    }

    pub fn render_settings_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let panel = self.settings_panel.as_ref()?;
        let query = panel.search.read(cx).value().to_string();
        let searching = !query.trim().is_empty();
        let (view, counts) = if searching { self.results_view(&query) } else { (self.settings_view(panel.page), Vec::new()) };
        let viewport = window.viewport_size();
        let (w, h) = ((f32::from(viewport.width) - 48.).clamp(560., 960.), (f32::from(viewport.height) - 48.).clamp(400., 690.));
        let hole = self.webview.read(cx).visible().then(|| {
            let webview = self.webview.read(cx);
            let (handle, under) = (webview.handle(), webview.bounds());
            canvas(move |bounds, _, _| overlay::set_hole(handle.raw(), overlay::Hole::Settings, Some(Bounds { origin: bounds.origin - under.origin, size: bounds.size })), |_, _, _, _| ())
                .absolute()
                .size_full()
        });
        let close = self
            .focusable("settings-close".into(), Role::Button, "Close settings".into(), cx)
            .absolute()
            .right(px(12.))
            .top(px(12.))
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::bg_raised()))
            .child(glyph_at(Glyph::Close, theme::text_new(), 1.25))
            .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)));
        let card = div()
            .id("settings-panel")
            .role(Role::Dialog)
            .aria_label("Settings")
            .key_context("Settings")
            .track_focus(&panel.focus)
            .occlude()
            .relative()
            .w(px(w))
            .h(px(h))
            .flex()
            .overflow_hidden()
            .rounded(px(14.))
            .bg(theme::bg_card())
            .border_1()
            .border_color(theme::composer_edge())
            .shadow(vec![BoxShadow { color: hsla(0., 0., 0., 0.6), offset: point(px(0.), px(30.)), blur_radius: px(80.), spread_radius: px(0.), inset: false }])
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(panel) = &mut this.settings_panel
                        && panel.idle_menu
                    {
                        panel.idle_menu = false;
                        cx.notify();
                    }
                }),
            )
            .children(hole)
            .child(self.render_settings_nav(panel, searching, &counts, window, cx))
            .child(
                div()
                    .id("settings-page")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(self.render_view(&view, panel, &query, cx)),
            )
            .child(close);
        Some(
            div()
                .id("settings-backdrop")
                .absolute()
                .inset_0()
                .bg(scrim())
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .text_color(theme::text_primary())
                .text_size(theme::size_body())
                .line_height(theme::line_body())
                .font_family(theme::SANS)
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
                .child(card)
                .into_any_element(),
        )
    }

    fn render_settings_nav(&self, panel: &Panel, searching: bool, counts: &[(Section, usize)], window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let focused = panel.search.read(cx).focus_handle(cx).is_focused(window);
        let search = panel.search.clone();
        let search_box = div()
            .id("settings-search")
            .role(Role::SearchInput)
            .aria_label("Search settings")
            .h(px(30.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(9.))
            .rounded(px(7.))
            .bg(theme::bg_page())
            .border_1()
            .border_color(if focused { theme::accent() } else { theme::composer_edge() })
            .child(glyph_at(Glyph::Search, theme::text_muted(), 13. / 12.))
            .child(div().flex_1().min_w_0().child(Input::new(&panel.search).appearance(false).text_size(theme::size_body())))
            .child(if searching {
                self.focusable("settings-search-clear".into(), Role::Button, "Clear search".into(), cx)
                    .flex()
                    .cursor_pointer()
                    .child(glyph_at(Glyph::Close, theme::text_muted(), 1.))
                    .on_click(cx.listener(move |_, _, window, cx| search.update(cx, |s, cx| s.set_value("", window, cx))))
                    .into_any_element()
            } else {
                div().text_size(theme::size_meta_small()).text_color(theme::text_faint()).child("⌘F").into_any_element()
            });
        let list = |yours: bool, cx: &mut Context<Self>| {
            let rows: Vec<_> = Section::ALL
                .into_iter()
                .filter(|s| s.yours() == yours)
                .filter_map(|s| {
                    let count = counts.iter().find(|(c, _)| *c == s).map(|(_, n)| *n);
                    if searching && count.is_none() {
                        return None;
                    }
                    let current = !searching && panel.page.section() == s;
                    let dot = !searching && self.needs_you(s);
                    Some(
                        self.focusable(format!("settings-nav-{}", s.key()).into(), Role::Tab, s.name().into(), cx)
                            .aria_selected(current)
                            .h(px(30.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(9.))
                            .px(px(8.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_color(if current { theme::text_primary() } else { theme::text_secondary() })
                            .when(current, |d| d.bg(theme::border()))
                            .when(!current, |d| d.hover(|h| h.bg(theme::row_active())))
                            .child(glyph_at(s.glyph(), if current { theme::accent_text() } else { theme::text_muted() }, 1.25))
                            .child(div().flex_1().child(s.name()))
                            .children(count.map(|n| div().text_size(theme::size_meta_small()).text_color(theme::text_muted()).child(n.to_string())))
                            .when(dot, |d| d.child(div().size(px(6.)).rounded_full().bg(theme::accent()).flex_shrink_0()))
                            .on_click(cx.listener(move |this, _, window, cx| this.settings_act(Act::Go(Page::Section(s)), window, cx))),
                    )
                })
                .collect();
            (!rows.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(div().px(px(8.)).pb(px(4.)).text_size(theme::size_meta_small()).line_height(px(16.)).text_color(theme::text_faint()).child(if yours { "Your work" } else { "Endeavor" }))
                    .children(rows)
            })
        };
        div()
            .id("settings-nav")
            .role(Role::TabList)
            .aria_label("Settings sections")
            .w(px(212.))
            .flex_shrink_0()
            .h_full()
            .px(px(10.))
            .py(px(14.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .border_r_1()
            .border_color(theme::border())
            .bg(nav_bg())
            .child(search_box)
            .children(list(true, cx))
            .children(list(false, cx))
    }

    fn render_view(&self, view: &View, panel: &Panel, query: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let back = view.back.map(|page| {
            self.focusable(format!("settings-back-{}", page.key()).into(), Role::Link, format!("Back to {}", page.section().name()).into(), cx)
                .self_start()
                .h(px(24.))
                .flex()
                .items_center()
                .gap(px(2.))
                .pl(px(2.))
                .pr(px(6.))
                .ml(px(-4.))
                .mb(px(-8.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(theme::size_meta())
                .text_color(theme::text_new())
                .hover(|s| s.bg(theme::row_active()))
                .child(glyph_at(Glyph::Back, theme::text_new(), 1.))
                .child(page.section().name())
                .on_click(cx.listener(move |this, _, window, cx| this.settings_act(Act::Go(page), window, cx)))
        });
        let header = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .text_size(px(18.))
                    .line_height(px(26.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(view.title.clone())
                    .children(view.aside.clone().map(|a| div().text_size(theme::size_body()).font_weight(FontWeight::NORMAL).text_color(theme::text_muted()).child(a))),
            )
            .children(view.subtitle.clone().map(|s| div().text_color(theme::text_new()).child(s)));
        div()
            .pt(px(22.))
            .pb(px(32.))
            .pl(px(32.))
            .pr(px(36.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .children(back)
            .child(header)
            .children(view.groups.iter().map(|g| self.render_group(g, panel, query, cx)))
    }

    fn render_group(&self, g: &Group, panel: &Panel, query: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut items: Vec<AnyElement> = Vec::new();
        for (i, item) in g.items.iter().enumerate() {
            if i > 0 {
                items.push(div().h(px(1.)).mx(px(14.)).bg(theme::border()).into_any_element());
            }
            items.push(match item {
                Item::Row(r) => self.render_row(r, panel, query, cx).into_any_element(),
                Item::Table(lines) => div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .px(px(14.))
                    .py(px(12.))
                    .children(lines.iter().map(|(label, value)| {
                        div().flex().gap(px(12.)).child(div().w(px(110.)).flex_shrink_0().text_color(theme::text_muted()).child(*label)).child(div().min_w_0().child(value.clone()))
                    }))
                    .into_any_element(),
                Item::Pictures(pictures) => self.render_pictures(pictures, cx).into_any_element(),
                Item::Brand => brand().into_any_element(),
                Item::Links(links) => div()
                    .flex()
                    .gap(px(18.))
                    .px(px(14.))
                    .py(px(12.))
                    .children(links.iter().enumerate().map(|(i, l)| self.render_control(format!("about-link-{i}").into(), l, cx)))
                    .into_any_element(),
                Item::Add { key, label, act } => {
                    let act = act.clone();
                    self.focusable(key.clone(), Role::Button, (*label).into(), cx)
                        .h(px(36.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .px(px(14.))
                        .rounded_b(px(10.))
                        .cursor_pointer()
                        .text_color(theme::text_new())
                        .hover(|s| s.bg(theme::row_active()).text_color(theme::text_primary()))
                        .child(div().w(px(16.)).flex().justify_center().child(glyph_at(Glyph::Plus, theme::text_muted(), 1.)))
                        .child(*label)
                        .on_click(cx.listener(move |this, _, window, cx| this.settings_act(act.clone(), window, cx)))
                        .into_any_element()
                }
            });
        }
        div()
            .flex()
            .flex_col()
            .children(g.heading.map(|h| div().mb(px(6.)).ml(px(2.)).text_size(theme::size_meta()).line_height(px(16.)).font_weight(FontWeight::MEDIUM).text_color(theme::text_secondary()).child(h)))
            .when(!items.is_empty(), |d| d.child(div().rounded(px(10.)).border_1().border_color(theme::border()).bg(theme::bg_card()).flex().flex_col().children(items)))
            .children(g.foot.clone().map(|f| div().mt(px(6.)).ml(px(2.)).text_size(theme::size_meta()).line_height(px(17.)).text_color(theme::text_faint()).child(f)))
    }

    fn render_row(&self, r: &Row, panel: &Panel, query: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let lit = panel.highlight.as_ref() == Some(&r.key);
        let lead = match &r.lead {
            Lead::None => None,
            Lead::Radio { checked, act } => {
                let dot = div()
                    .size(px(16.))
                    .flex_shrink_0()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .map(|d| match (checked, act.is_some()) {
                        (true, _) => d.bg(theme::accent()).child(div().size(px(6.)).rounded_full().bg(gpui::white())),
                        (false, true) => d.border(px(1.5)).border_color(theme::text_section()),
                        (false, false) => d.border(px(1.5)).border_color(theme::composer_edge()),
                    });
                let radio = self
                    .focusable(format!("{}-radio", r.key).into(), Role::RadioButton, r.title.clone(), cx)
                    .aria_toggled(if *checked { accesskit::Toggled::True } else { accesskit::Toggled::False })
                    .pt(px(1.))
                    .flex()
                    .rounded_full()
                    .child(dot);
                Some(match act.clone() {
                    Some(act) => radio.cursor_pointer().on_click(cx.listener(move |this, _, window, cx| this.settings_act(act.clone(), window, cx))).into_any_element(),
                    None => radio.into_any_element(),
                })
            }
            Lead::Icon { glyph, dot, faint, tone } => {
                let color = match (tone, faint) {
                    (Some(t), _) => t.color(),
                    (None, true) => theme::composer_edge(),
                    (None, false) => icon_grey(),
                };
                Some(
                    div()
                        .pt(px(1.))
                        .flex()
                        .child(
                            div()
                                .relative()
                                .flex()
                                .child(glyph_at(*glyph, color, 4. / 3.))
                                .when(*dot, |d| d.child(div().absolute().right(px(-3.)).bottom(px(-2.)).size(px(7.)).rounded_full().bg(theme::accent()).border_2().border_color(theme::bg_card()))),
                        )
                        .into_any_element(),
                )
            }
        };
        let title_color = match (r.title_tone, r.unavailable) {
            (Some(t), _) => t.color(),
            (None, true) => theme::text_faint(),
            (None, false) => theme::text_primary(),
        };
        let marked = |text: &SharedString, color: Rgba| -> AnyElement {
            let ranges = if r.act.is_some() { marks(text, query) } else { Vec::new() };
            if ranges.is_empty() {
                return div().text_color(color).child(text.clone()).into_any_element();
            }
            let style = HighlightStyle { background_color: Some(mark_bg().into()), color: Some(theme::text_primary().into()), ..Default::default() };
            div().text_color(color).child(StyledText::new(text.clone()).with_highlights(ranges.into_iter().map(|r| (r, style)))).into_any_element()
        };
        let title = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(marked(&r.title, title_color))
            .children(r.aside.clone().map(|a| div().text_size(theme::size_meta()).text_color(if r.unavailable { theme::text_section() } else { theme::text_muted() }).child(a)))
            .when(r.problem, |d| d.child(glyph_at(Glyph::Warning, theme::danger(), 13. / 12.)));
        let status = r.status.as_ref().map(|s| {
            div()
                .text_size(theme::size_meta())
                .line_height(px(17.))
                .text_color(s.tone.color())
                .children(s.mono.clone().map(|m| div().font_family(theme::MONO).text_size(px(11.5)).child(m)))
                .child(s.text.trim_start().to_owned())
        });
        let desc = r.desc.clone().map(|d| div().text_size(theme::size_meta()).line_height(px(17.)).text_color(theme::text_muted()).child(d));
        let extra = r.extra.as_ref().map(|e| match e {
            Extra::Hint(text) => div().text_size(theme::size_meta()).line_height(px(17.)).text_color(theme::text_muted()).child(text.clone()).into_any_element(),
            Extra::Path { path, version } => div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(8.))
                .rounded(px(6.))
                .bg(theme::bg_page())
                .border_1()
                .border_color(theme::border())
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().font_family(theme::MONO).text_size(theme::size_code()).child(path.clone()))
                .child(div().flex_shrink_0().text_size(theme::size_meta()).text_color(theme::text_muted()).child(version.clone()))
                .into_any_element(),
            Extra::Progress { text, step, fraction } => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .text_size(theme::size_meta())
                        .child(div().flex_1().text_color(theme::text_secondary()).child(text.clone()))
                        .child(div().text_color(theme::text_muted()).child(step.clone()))
                        .child(div().ml(px(8.)).child(crate::session::orbit("repair-orbit".into(), 12., cx))),
                )
                .child(div().h(px(2.)).rounded_full().bg(theme::border()).child(div().h_full().rounded_full().bg(theme::accent()).w(relative(*fraction))))
                .into_any_element(),
        });
        let crumb = r.crumb.as_ref().map(|c| div().mb(px(2.)).text_size(theme::size_meta_small()).line_height(px(15.)).child(marked(c, theme::text_muted())));
        let text = div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).children(crumb).child(title).children(status).children(desc);
        let controls = div()
            .flex_shrink_0()
            .min_h(px(20.))
            .flex()
            .items_center()
            .gap(px(8.))
            .children(r.controls.iter().enumerate().map(|(i, c)| self.render_control(format!("{}-{i}", r.key).into(), c, cx)));
        let body = div()
            .id(ElementId::Name(format!("row-{}", r.key).into()))
            .px(px(14.))
            .py(px(if r.unavailable { 9. } else { 11. }))
            .flex()
            .flex_col()
            .gap(px(8.))
            .when(lit, |d| d.bg(lit_bg()).rounded(px(9.)))
            .child(div().flex().items_start().gap(px(12.)).children(lead).child(text).child(controls))
            // Under the words, on their left edge: past the lead's column when there is one.
            .children(extra.map(|e| div().when(!matches!(r.lead, Lead::None), |d| d.ml(px(28.))).when(matches!(r.extra, Some(Extra::Hint(_))), |d| d.mt(px(-6.))).child(e)));
        match r.act.clone() {
            Some(act) => self
                .focusable(format!("row-{}", r.key).into(), Role::Link, format!("{} {}", r.crumb.clone().unwrap_or_default(), r.title).into(), cx)
                .cursor_pointer()
                .hover(|s| s.bg(theme::row_active()))
                .on_click(cx.listener(move |this, _, window, cx| this.settings_act(act.clone(), window, cx)))
                .child(body)
                .into_any_element(),
            None => body.into_any_element(),
        }
    }

    fn render_control(&self, id: SharedString, control: &Control, cx: &mut Context<Self>) -> AnyElement {
        let on = |act: Act| cx.listener(move |this: &mut Workspace, _: &ClickEvent, window: &mut Window, cx: &mut Context<Workspace>| this.settings_act(act.clone(), window, cx));
        match control {
            Control::Button { label, look, icon, act, aria } => {
                let enabled = act.is_some();
                let fg = match look {
                    Look::Primary => Rgba::from(gpui::white()),
                    _ => theme::text_primary(),
                };
                let d = self
                    .focusable(id, Role::Button, aria.clone(), cx)
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(6.))
                    .h(px(26.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .whitespace_nowrap()
                    .text_size(theme::size_meta())
                    .font_weight(FontWeight::MEDIUM)
                    .border_1()
                    .map(|d| match look {
                        Look::Primary => d.bg(theme::accent()).border_color(theme::accent()).text_color(fg),
                        _ => d.bg(theme::bg_tag()).border_color(theme::composer_edge()).text_color(fg),
                    })
                    .children(icon.map(|g| glyph_at(g, if *look == Look::Primary { Rgba::from(gpui::white()) } else { icon_grey() }, 13. / 12.)))
                    .child(*label);
                match act.clone() {
                    Some(act) if enabled => d.cursor_pointer().hover(|s| s.bg(theme::bg_raised())).on_click(on(act)).into_any_element(),
                    _ => d.opacity(0.45).into_any_element(),
                }
            }
            Control::Toggle { on: is_on, act, aria } => self
                .focusable(id, Role::Switch, aria.clone(), cx)
                .aria_toggled(if *is_on { accesskit::Toggled::True } else { accesskit::Toggled::False })
                .relative()
                .w(px(30.))
                .h(px(18.))
                .flex_shrink_0()
                .rounded(px(9.))
                .cursor_pointer()
                .bg(if *is_on { theme::accent() } else { theme::composer_edge() })
                .child(div().absolute().top(px(2.)).left(px(if *is_on { 14. } else { 2. })).size(px(14.)).rounded_full().bg(gpui::white()))
                .on_click(on(act.clone()))
                .into_any_element(),
            Control::Select { value, act, aria } => {
                let open = self.settings_panel.as_ref().is_some_and(|p| p.idle_menu);
                // Deferred draws paint in priority order across the window: the
                // panel's own priority, and later in the list, puts it on top of the panel.
                let menu = open.then(|| {
                    let current = self.settings.idle_stop;
                    div().absolute().top(px(32.)).right_0().child(deferred(
                            div()
                                .id("idle-stop-menu")
                                .role(Role::ListBox)
                                .aria_label("Stop idle notebooks after")
                                .occlude()
                                .w(px(132.))
                                .p(px(4.))
                                .rounded(px(8.))
                                .border_1()
                                .border_color(theme::composer_edge())
                                .bg(theme::bg_raised())
                                .flex()
                                .flex_col()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .children(IdleStop::ALL.iter().map(|(value, label)| {
                                    let value = *value;
                                    self.focusable(format!("idle-stop-{label}").into(), Role::ListBoxOption, (*label).into(), cx)
                                        .aria_selected(value == current)
                                        .h(px(26.))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .px(px(8.))
                                        .rounded(px(5.))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(theme::row_active()))
                                        .child(*label)
                                        .when(value == current, |d| d.child(glyph_at(Glyph::Check, theme::text_secondary(), 1.)))
                                        .on_click(cx.listener(move |this, _, window, cx| this.settings_act(Act::Idle(value), window, cx)))
                                })),
                        )
                        .with_priority(2),
                    )
                });
                div()
                    .relative()
                    .child(
                        self.focusable(id, Role::ComboBox, aria.clone(), cx)
                            .aria_expanded(open)
                            .w(px(132.))
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .pl(px(10.))
                            .pr(px(8.))
                            .rounded(px(6.))
                            .bg(theme::bg_tag())
                            .border_1()
                            .border_color(theme::composer_edge())
                            .cursor_pointer()
                            .child(*value)
                            .child(glyph_at(Glyph::Chevron, icon_grey(), 1.))
                            .on_click(on(act.clone())),
                    )
                    .children(menu)
                    .into_any_element()
            }
            Control::Gear { act, aria } => self
                .focusable(id, Role::Button, aria.clone(), cx)
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::bg_raised()))
                .child(glyph_at(Glyph::Gear, theme::text_muted(), 1.))
                .on_click(on(act.clone()))
                .into_any_element(),
            Control::Link { label, external, act } => self
                .focusable(id, Role::Link, (*label).into(), cx)
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .whitespace_nowrap()
                .text_size(theme::size_meta())
                .line_height(px(17.))
                .text_color(theme::accent_text())
                .hover(|s| s.underline())
                .child(*label)
                .when(*external, |d| d.child(glyph_at(Glyph::External, theme::accent_text(), 11. / 12.)))
                .on_click(on(act.clone()))
                .into_any_element(),
            Control::Meta(text) => div().text_size(theme::size_meta_small()).text_color(theme::text_faint()).whitespace_nowrap().child(text.clone()).into_any_element(),
            Control::Note(text) => div().text_size(theme::size_meta()).line_height(px(17.)).text_color(theme::text_muted()).whitespace_nowrap().child(text.clone()).into_any_element(),
            Control::Chevron => div().pt(px(2.)).child(glyph_at(Glyph::Forward, theme::text_faint(), 1.)).into_any_element(),
        }
    }

    fn render_pictures(&self, pictures: &[Picture], cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let big = pictures.iter().any(|p| matches!(p.thumb, Thumb::Endeavor | Thumb::Classic));
        div()
            .id(if big { "notebook-look" } else { "light-or-dark" })
            .role(Role::RadioGroup)
            .aria_label(if big { "Pluto notebook look" } else { "Light or dark" })
            .flex()
            .gap(px(if big { 24. } else { 22. }))
            .px(px(16.))
            .pt(px(16.))
            .pb(px(14.))
            .children(pictures.iter().map(|p| {
                let act = p.act.clone();
                let panel_lit = self.settings_panel.as_ref().and_then(|panel| panel.highlight.as_ref()) == Some(&p.key);
                self.focusable(p.key.clone(), Role::RadioButton, p.label.into(), cx)
                    .aria_toggled(if p.selected { accesskit::Toggled::True } else { accesskit::Toggled::False })
                    .w(px(if big { 236. } else { 132. }))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .cursor_pointer()
                    .rounded(px(7.))
                    .when(panel_lit, |d| d.bg(lit_bg()))
                    .child(thumb(p.thumb, p.selected))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(div().text_color(if p.selected { theme::text_primary() } else { theme::text_secondary() }).when(p.selected, |d| d.font_weight(FontWeight::MEDIUM)).child(p.label))
                            .children(p.desc.map(|d| div().text_size(theme::size_meta()).line_height(px(17.)).text_color(theme::text_muted()).child(d))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.settings_act(act.clone(), window, cx)))
            }))
    }
}

/// A small picture of a look.
fn thumb(kind: Thumb, selected: bool) -> AnyElement {
    let frame = |d: Div| {
        d.rounded(px(7.)).overflow_hidden().map(|d| if selected { d.border_2().border_color(theme::accent()) } else { d.border_1().border_color(theme::composer_edge()) })
    };
    let window = |light: bool| {
        let (bg, line, block) = if light { (rgb(0xFAFAF7), rgb(0xD0CFC8), rgb(0xECEBE6)) } else { (theme::bg_page(), theme::composer_edge(), theme::bg_raised()) };
        div()
            .size_full()
            .flex()
            .bg(bg)
            .child(div().w(px(30.)).h_full().bg(theme::bg_sidebar()))
            .child(
                div()
                    .flex_1()
                    .px(px(8.))
                    .py(px(10.))
                    .flex()
                    .flex_col()
                    .child(div().h(px(4.)).w(relative(0.5)).rounded(px(2.)).bg(line).mb(px(8.)))
                    .children([0.9, 0.7, 0.84].map(|w| div().h(px(9.)).w(relative(w)).rounded(px(2.)).bg(block).mb(px(5.)))),
            )
    };
    match kind {
        Thumb::Dark => frame(div().w(px(132.)).h(px(82.))).child(window(false)).into_any_element(),
        Thumb::Light => frame(div().w(px(132.)).h(px(82.))).child(window(true)).into_any_element(),
        Thumb::Match => frame(div().w(px(132.)).h(px(82.)).relative())
            .child(window(false))
            .child(
                // The light half, below the diagonal from top-right to bottom-left.
                canvas(
                    |_, _, _| (),
                    |b, _, window, _| {
                        let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
                        // The light window's page and blocks, each cut along the diagonal.
                        let rects = [(30., 0., w, h, 0xFAFAF7), (38., 10., 38. + 43., 14., 0xD0CFC8), (38., 22., 38. + 77.4, 31., 0xECEBE6), (38., 36., 38. + 60.2, 45., 0xECEBE6), (38., 50., 38. + 72.2, 59., 0xECEBE6)];
                        for (x0, y0, x1, y1, color) in rects {
                            let part = below_diagonal(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], w, h);
                            let mut path = PathBuilder::fill();
                            for (i, (x, y)) in part.iter().enumerate() {
                                let p = point(b.left() + px(*x), b.top() + px(*y));
                                if i == 0 { path.move_to(p) } else { path.line_to(p) }
                            }
                            path.close();
                            if part.len() >= 3
                                && let Ok(path) = path.build()
                            {
                                window.paint_path(path, rgb(color));
                            }
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element(),
        Thumb::Endeavor => frame(div().w(px(236.)).h(px(110.)).bg(theme::bg_page()))
            .child(
                div()
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(8.))
                    .border_b_1()
                    .border_color(theme::divider())
                    .child(div().w(px(3.)).h(px(8.)).rounded(px(1.)).bg(theme::accent()))
                    .child(div().w(px(40.)).h(px(4.)).rounded(px(2.)).bg(theme::text_muted()))
                    .child(div().flex_1())
                    .child(div().w(px(22.)).h(px(4.)).rounded(px(2.)).bg(theme::composer_edge())),
            )
            .children([12., 12., 22.].map(|h| div().h(px(h)).mx(px(10.)).mt(px(6.)).rounded(px(3.)).bg(theme::bg_card())))
            .into_any_element(),
        Thumb::Classic => frame(div().w(px(236.)).h(px(110.)).bg(rgb(0x1F1F1F)))
            .child(
                div()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .px(px(8.))
                    .border_b_1()
                    .border_color(rgb(0x333333))
                    .child(div().w(px(6.)).h(px(9.)).rounded(px(1.)).bg(theme::text_muted()))
                    .child(div().w(px(30.)).h(px(4.)).rounded(px(2.)).bg(rgb(0x6B6B6B)))
                    .child(div().flex_1())
                    .child(div().w(px(14.)).h(px(8.)).rounded(px(2.)).border_1().border_color(rgb(0x555555)))
                    .child(div().w(px(14.)).h(px(8.)).rounded(px(2.)).border_1().border_color(rgb(0x555555))),
            )
            .children([12., 12., 18.].map(|h| div().h(px(h)).mx(px(10.)).mt(px(6.)).rounded(px(2.)).bg(rgb(0x2A2A2A)).border_l(px(3.)).border_color(rgb(0x3C3C3C))))
            .into_any_element(),
    }
}

/// The part of a polygon on or below the diagonal from a `w` by `h` box's
/// top-right corner to its bottom-left one.
fn below_diagonal(polygon: &[(f32, f32)], w: f32, h: f32) -> Vec<(f32, f32)> {
    let side = |(x, y): (f32, f32)| x / w + y / h - 1.;
    let mut out = Vec::new();
    for (i, &p) in polygon.iter().enumerate() {
        let q = polygon[(i + 1) % polygon.len()];
        let (sp, sq) = (side(p), side(q));
        if sp >= 0. {
            out.push(p);
        }
        if (sp >= 0.) != (sq >= 0.) {
            let t = sp / (sp - sq);
            out.push((p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)));
        }
    }
    out
}

/// About's card: the turtle, the name, the version and build, the credits.
fn brand() -> impl IntoElement {
    const R: f32 = 22.;
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .p(px(16.))
        .child(
            canvas(|_, _, _| (), |b, _, window, _| crate::turtle::paint(window, point(b.left() + px(R + 2.), b.bottom() - px(4.)), R, &crate::turtle::Pose::default()))
                .w(px(64.))
                .h(px(40.))
                .flex_shrink_0(),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(theme::size_subhead()).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child("Endeavor"))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(4.))
                        .text_size(theme::size_meta())
                        .text_color(theme::text_new())
                        .child(format!("Version {}", about::VERSION))
                        .child(div().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_muted()).child(format!("(build {})", about::BUILD))),
                )
                .child(div().text_size(theme::size_meta()).text_color(theme::text_muted()).child("Notebooks by Pluto.jl. Runs on Julia. Works with Claude, by Anthropic.")),
        )
}

/// The debug state dump's part (debug_state.rs): the page as the panel draws it.
#[cfg(debug_assertions)]
impl Workspace {
    pub fn settings_debug(&self, cx: &App) -> serde_json::Value {
        use serde_json::{Value, json};
        let Some(panel) = &self.settings_panel else { return Value::Null };
        let query = panel.search.read(cx).value().to_string();
        let searching = !query.trim().is_empty();
        let (view, counts) = if searching { self.results_view(&query) } else { (self.settings_view(panel.page), Vec::new()) };
        let control = |c: &Control| match c {
            Control::Button { label, act, look, .. } => json!({ "button": label, "enabled": act.is_some(), "primary": *look == Look::Primary }),
            Control::Toggle { on, aria, .. } => json!({ "toggle": aria.to_string(), "on": on }),
            Control::Select { value, .. } => json!({ "select": value, "open": panel.idle_menu }),
            Control::Gear { aria, .. } => json!({ "gear": aria.to_string() }),
            Control::Link { label, .. } => json!({ "link": label }),
            Control::Meta(text) | Control::Note(text) => json!({ "meta": text.to_string() }),
            Control::Chevron => json!("chevron"),
        };
        let item = |item: &Item| match item {
            Item::Row(r) => json!({
                "key": r.key.to_string(),
                "title": r.title.to_string(),
                "title_tone": r.title_tone.map(Tone::key),
                "crumb": r.crumb.as_ref().map(ToString::to_string),
                "lead": match &r.lead {
                    Lead::None => Value::Null,
                    Lead::Radio { checked, act } => json!({ "radio": checked, "enabled": act.is_some() }),
                    Lead::Icon { dot, .. } => json!({ "icon": true, "running_dot": dot }),
                },
                "problem": r.problem,
                "state": r.status.as_ref().map(|s| format!("{}{}", s.mono.clone().unwrap_or_default(), s.text)),
                "tone": r.status.as_ref().map(|s| s.tone.key()),
                "desc": r.desc.as_ref().map(ToString::to_string),
                "extra": match &r.extra {
                    None => Value::Null,
                    Some(Extra::Hint(text)) => json!({ "hint": text }),
                    Some(Extra::Path { path, version }) => json!({ "path": path, "version": version }),
                    Some(Extra::Progress { text, step, fraction }) => json!({ "progress": text, "step": step, "fraction": fraction }),
                },
                "controls": r.controls.iter().map(control).collect::<Vec<_>>(),
                "unavailable": r.unavailable,
                "highlighted": panel.highlight.as_ref() == Some(&r.key),
            }),
            Item::Table(lines) => json!({ "table": lines.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>() }),
            Item::Pictures(pictures) => json!({ "pictures": pictures.iter().map(|p| json!({ "label": p.label, "selected": p.selected })).collect::<Vec<_>>() }),
            Item::Brand => json!({ "brand": format!("Endeavor {} (build {})", about::VERSION, about::BUILD) }),
            Item::Links(links) => json!({ "links": links.iter().map(control).collect::<Vec<_>>() }),
            Item::Add { label, .. } => json!({ "add": label }),
        };
        json!({
            "section": panel.page.section().key(),
            "page": panel.page.key(),
            "search": query,
            "list": Section::ALL.iter().filter_map(|s| {
                let count = counts.iter().find(|(c, _)| c == s).map(|(_, n)| *n);
                (!searching || count.is_some()).then(|| json!({ "name": s.name(), "current": !searching && panel.page.section() == *s, "dot": !searching && self.needs_you(*s), "count": count }))
            }).collect::<Vec<_>>(),
            "back": view.back.map(|p| p.section().name()),
            "title": view.title.to_string(),
            "aside": view.aside.as_ref().map(ToString::to_string),
            "subtitle": view.subtitle.as_ref().map(ToString::to_string),
            "groups": view.groups.iter().map(|g| json!({
                "heading": g.heading,
                "items": g.items.iter().map(item).collect::<Vec<_>>(),
                "foot": g.foot.as_ref().map(ToString::to_string),
            })).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Found, Page, Section, closes, marks, matches};

    fn found(page: Page, crumb: &str, title: &str, summary: &str) -> Found {
        Found { page, key: title.to_lowercase().into(), crumb: crumb.into(), title: title.into(), summary: Some(summary.into()), words: String::new() }
    }

    #[test]
    fn search_finds_settings_by_their_words_and_where_they_live() {
        let index = vec![
            found(Page::Section(Section::Notebooks), "Notebooks › Languages", "Julia", "Endeavor's Julia 1.12.6."),
            found(Page::Julia, "Notebooks › Julia", "Another Julia on this Mac", "Use a Julia you installed yourself."),
            found(Page::Section(Section::Troubleshooting), "Troubleshooting › Julia on this Mac", "Restart Julia", "Stops your Julia notebooks."),
            found(Page::Section(Section::Notebooks), "Notebooks › When notebooks stop", "Stop idle notebooks after", "A notebook nobody has used for this long stops, to free memory."),
            found(Page::Section(Section::Appearance), "Appearance › Light or dark", "Match macOS", ""),
        ];
        let titles = |q: &str| matches(&index, q).into_iter().map(|f| f.title.as_str()).collect::<Vec<_>>();
        assert_eq!(titles("julia"), ["Julia", "Another Julia on this Mac", "Restart Julia"]);
        assert_eq!(titles("JULIA restart"), ["Restart Julia"]);
        assert_eq!(titles("memory"), ["Stop idle notebooks after"]);
        assert_eq!(titles("dark"), ["Match macOS"]);
        assert_eq!(titles("  "), Vec::<&str>::new());
        assert_eq!(titles("cobol"), Vec::<&str>::new());
    }

    #[test]
    fn a_results_words_are_marked_where_the_query_is() {
        assert_eq!(marks("Restart Julia", "julia"), [8..13]);
        assert_eq!(marks("Troubleshooting › Julia on this Mac", "julia mac"), [20..25, 34..37]);
        assert_eq!(marks("Julia", "jul julia"), [0..5]);
        assert_eq!(marks("Dark", "light"), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn stop_says_which_notebooks_close() {
        assert_eq!(closes(&["residuals.jl".into()]), "residuals.jl closes. Its file is saved.");
        assert_eq!(closes(&["a.jl".into(), "b.jl".into(), "c.jl".into()]), "a.jl, b.jl and c.jl close. Their files are saved.");
        assert_eq!(closes(&[]), "No notebooks are open there.");
    }
}
