//! The pending-approval card above the composer: run/edit/plan prompts, the
//! plan card, and the pinned running plan.

use std::path::Path;

use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, PlanEntry, PlanEntryStatus, ToolKind};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::text::{TextView, TextViewStyle};

use crate::Workspace;
use crate::celldiff::{self, Change};
use crate::hosts::HostId;
use crate::new_session::{Glyph, glyph, tilde};
use crate::pluto;
use crate::runs;
use crate::session::{Entry, Scope, Session, cell_label, file_name, folder_name, option_of_kind, plan_option};
use crate::theme;
use crate::theme::FocusRing as _;
use crate::transcript::markdown_style;

/// The pending approval card, as it reads.
pub(crate) struct ApprovalView {
    pub heading: String,
    /// "1 of 3" while several prompts wait.
    pub count: Option<String>,
    /// What it would run or change: code, a command, a diff.
    pub code: Option<CardCode>,
    pub lines: Vec<(String, Tone)>,
    /// Plan mode's end: the plan to approve, in place of code and lines.
    pub plan: Option<PlanCard>,
    pub buttons: Vec<CardButton>,
    /// "In this folder", under the ⌄ on Always this session: the agent's own
    /// lasting rule, offered when the agent keeps those in the folder.
    pub folder: Option<PermissionOption>,
    /// The notebook cells it asks to run, those that re-run after them, and
    /// those it needs that never ran (they run first).
    pub cells: Vec<String>,
    pub rerun: Vec<String>,
    pub needed: Vec<String>,
    /// The prompts waiting behind this one, by their questions.
    pub then: Vec<String>,
    /// The cells' names, as the heading names them.
    pub names: Vec<String>,
}

/// A card's code box.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CardCode {
    Plain(String),
    /// The change, removed lines first, in the transcript's diff colours.
    Diff(Vec<(Change, String)>),
    /// Code going away (a deleted cell), struck through.
    Struck(String),
}

/// The card shows this many lines of code until "Show all N lines".
pub(crate) const CODE_LINES: usize = 8;

impl CardCode {
    fn lines(&self) -> Vec<(Change, String)> {
        match self {
            CardCode::Plain(code) | CardCode::Struck(code) => code.lines().map(|l| (Change::Same, l.to_owned())).collect(),
            CardCode::Diff(lines) => lines.clone(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.lines().len()
    }

    /// As plain text (diff lines marked "- " and "+ "), cut to the first lines unless `all`.
    pub fn text(&self, all: bool) -> String {
        let lines = self.lines();
        let mut shown: Vec<String> = lines
            .iter()
            .take(if all { usize::MAX } else { CODE_LINES })
            .map(|(change, line)| match (self, change) {
                (CardCode::Diff(_), Change::Added) => format!("+ {line}"),
                (CardCode::Diff(_), Change::Removed) => format!("- {line}"),
                (CardCode::Diff(_), Change::Same) => format!("  {line}"),
                _ => line.clone(),
            })
            .collect();
        if !all && lines.len() > CODE_LINES {
            shown.push("…".into());
        }
        shown.join("\n")
    }
}

/// A line's colour; `backticked` names in it are mono.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tone {
    Muted,
    Secondary,
    Faint,
}

pub(crate) struct CardButton {
    pub label: String,
    /// The key that presses it.
    pub hint: &'static str,
    pub weight: Weight,
    pub option: PermissionOption,
    /// How far the answer reaches.
    pub scope: Scope,
}

/// How a card's button looks and where it sits: the answer that declines on
/// the left, plain, then the others outlined, and the one ⏎ presses filled.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Weight {
    Quiet,
    Outlined,
    Primary,
}

impl Weight {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Weight::Quiet => "quiet",
            Weight::Outlined => "outlined",
            Weight::Primary => "primary",
        }
    }
}

/// The card for the first prompt waiting: the only heavy element (accent
/// edge, soft ring, filled primary), with a count and the prompts behind it.
pub(crate) fn approval_view(session: &Session) -> Option<ApprovalView> {
    let waiting = session.waiting_prompts();
    let mut view = view_at(session, *waiting.first()?)?;
    view.count = session.asks.count();
    view.then = waiting[1..].iter().filter_map(|ix| view_at(session, *ix)).map(|v| v.heading.trim_end_matches('?').to_owned()).collect();
    Some(view)
}

/// How far ⌘⏎ in an empty message box answers the card shown: this session
/// only when the card offers Always this session, else once, as ⏎ does.
pub(crate) fn command_enter_scope(session: &Session) -> Scope {
    let always = approval_view(session).is_some_and(|view| view.buttons.iter().any(|b| b.scope == Scope::Session));
    if always { Scope::Session } else { Scope::Once }
}

/// The notebook cells prompt `ix` asks to run (none for any other prompt).
pub(crate) fn run_cells_at(session: &Session, ix: usize) -> Vec<String> {
    view_at(session, ix).map(|v| v.cells).unwrap_or_default()
}

/// The names prompt `ix`'s heading gives its cells ("rates").
pub(crate) fn heading_names_at(session: &Session, ix: usize) -> Vec<String> {
    view_at(session, ix).map(|v| v.names).unwrap_or_default()
}

/// The cells every waiting run card asks to run, each once, with what it
/// defines when known: the page asks before the user's own run reaches one.
pub(crate) fn waiting_run_cells(session: &Session) -> Vec<(String, Option<String>)> {
    let mut cells: Vec<(String, Option<String>)> = Vec::new();
    for ix in session.waiting_prompts() {
        let preview = match session.entries.get(ix) {
            Some(Entry::Permission { preview, .. }) => preview.as_ref(),
            _ => None,
        };
        for id in run_cells_at(session, ix) {
            if cells.iter().any(|(c, _)| *c == id) {
                continue;
            }
            let name = preview
                .and_then(|p| p.cells.iter().find(|c| c.id.as_deref() == Some(id.as_str())))
                .and_then(|c| c.name.clone())
                .or_else(|| session.cell_codes.get(&id).map(cell_label));
            cells.push((id, name));
        }
    }
    cells
}

/// A prompt's question ("Run `rates`?"), for the note when it went unanswered.
pub(crate) fn heading_at(session: &Session, ix: usize) -> Option<String> {
    view_at(session, ix).map(|v| v.heading)
}

/// What one prompt's card says. Runs get "Run N cells?", the cells, and how
/// many dependents re-run; the agent's own prompts get a question in its
/// verb (Edit, Create, Run, Fetch) and never its option labels or tool names.
fn view_at(session: &Session, ix: usize) -> Option<ApprovalView> {
    let Entry::Permission { title, code, options, runs_code, tool, input, preview, plan, kind, path, .. } = session.entries.get(ix)? else { return None };
    if let Some(plan) = plan {
        let card = PlanCard { open: session.plan_open, ..plan_card(plan) };
        return Some(ApprovalView {
            heading: "Plan".into(),
            count: None,
            code: None,
            lines: vec![],
            plan: Some(card),
            buttons: plan_buttons(options),
            folder: None,
            cells: vec![],
            rerun: vec![],
            needed: vec![],
            then: vec![],
            names: vec![],
        });
    }
    let tool = tool.as_deref().unwrap_or("");
    let ask = if !*runs_code {
        agent_prompt(session, title, *kind, input, path.as_deref(), code.as_deref())
    } else if tool == "run_shell" {
        let host = session.server.clone().unwrap_or_else(|| "the server".into());
        let folder = session.place.path.clone();
        let cwd = input["cwd"].as_str().filter(|c| !c.is_empty()).unwrap_or(&folder);
        Prompt {
            heading: format!("Run a command on {host}?"),
            verb: "Run",
            code: input["command"].as_str().map(|c| CardCode::Plain(c.to_owned())),
            lines: vec![(format!("In {cwd}"), Tone::Muted)],
            ..Default::default()
        }
    } else if tool == "allow_execution" {
        let file = session.notebook_path.as_deref().map(|p| folder_name(Path::new(p)));
        let host = session.server.clone().unwrap_or_else(|| crate::platform::this_computer!().into());
        let mut lines = Vec::new();
        if let Some(line) = notebook_summary(file.as_deref(), preview.as_ref()) {
            lines.push((line, Tone::Secondary));
        }
        lines.push((format!("Nothing runs yet. Running it lets its code read and change files on {host}."), Tone::Muted));
        Prompt { heading: "Let this notebook run?".into(), verb: "Run notebook", code: None, lines, ..Default::default() }
    } else {
        run_prompt(session, tool, input, preview.as_ref())
    };

    let cells: Vec<String> = match preview.as_ref().filter(|p| !p.all) {
        Some(p) if p.cells.iter().any(|c| c.id.is_some()) => p.cells.iter().filter_map(|c| c.id.clone()).collect(),
        _ if *runs_code && !matches!(tool, "allow_execution" | "run_shell" | "run_all_cells") => {
            input["cell_id"].as_str().map(str::to_owned).into_iter().chain(input["cell_ids"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_owned))).collect()
        }
        _ => vec![],
    };
    let rerun = preview.as_ref().map(|p| p.dependent_ids.clone()).unwrap_or_default();
    let needed = preview.as_ref().map(|p| p.needed_ids.clone()).unwrap_or_default();

    // (label, key, option, scope)
    let mut buttons: Vec<(String, &'static str, PermissionOption, Scope)> = Vec::new();
    let allow = option_of_kind(options, PermissionOptionKind::AllowOnce);
    let deny = option_of_kind(options, PermissionOptionKind::RejectOnce);
    if tool == "allow_execution" && *runs_code {
        // The same question as the notebook's safe-preview callout: once, not "always".
        buttons.extend(deny.map(|d| ("Not now".into(), "esc", d.clone(), Scope::Once)));
        buttons.extend(allow.map(|a| (ask.verb.into(), "⏎", a.clone(), Scope::Once)));
    } else if let Some(allow) = allow {
        buttons.extend(deny.map(|d| ("Deny".into(), "esc", d.clone(), Scope::Once)));
        buttons.push(("Always this session".into(), crate::platform::shortcut!("⏎"), allow.clone(), Scope::Session));
        buttons.push((ask.verb.into(), "⏎", allow.clone(), Scope::Once));
    } else {
        buttons = option_buttons(options);
    }
    let folder = (!*runs_code && session.agent.facts().folder_rules.is_some() && session.place.host == HostId::ThisMac)
        .then(|| crate::permits::folder_rule_option(options).cloned())
        .flatten();
    let primary = buttons.iter().rposition(|(_, _, o, _)| matches!(o.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways));
    let buttons = buttons
        .into_iter()
        .enumerate()
        .map(|(i, (label, hint, option, scope))| {
            let weight = match option.kind {
                _ if Some(i) == primary => Weight::Primary,
                PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways => Weight::Quiet,
                _ => Weight::Outlined,
            };
            CardButton { label, hint, weight, option, scope }
        })
        .collect();
    Some(ApprovalView { heading: ask.heading, count: None, code: ask.code, lines: ask.lines, plan: None, buttons, folder, cells, rerun, needed, then: vec![], names: ask.names })
}

/// A prompt's question, the word on its ⏎ button, and its body.
#[derive(Default)]
struct Prompt {
    heading: String,
    verb: &'static str,
    code: Option<CardCode>,
    lines: Vec<(String, Tone)>,
    names: Vec<String>,
}

/// A run card (the execution gate): "Run `rates`?" with the cell's code, the
/// edit as a diff, a new cell's code, a deleted cell struck through.
fn run_prompt(session: &Session, tool: &str, input: &serde_json::Value, preview: Option<&pluto::RunPreview>) -> Prompt {
    let question = run_question(tool, preview, input);
    let mut lines = Vec::new();
    if question.names.len() > 1 && preview.is_none_or(|p| !p.all) {
        const SHOWN: usize = 5;
        let mut names = question.names.iter().take(SHOWN).map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
        if question.names.len() > SHOWN {
            names.push_str(&format!(" and {} more", question.names.len() - SHOWN));
        }
        lines.push((names, Tone::Secondary));
    }
    let one = preview.is_none_or(|p| p.count <= 1 && !p.all) && question.names.len() <= 1;
    let old = input["cell_id"].as_str().and_then(|id| session.cell_codes.get(id)).map(str::to_owned).or_else(|| preview.and_then(|p| p.cells.first()).map(|c| c.code.clone()));
    let new = input["code"].as_str();
    let code = match (tool, new) {
        ("add_cell", Some(code)) => Some(CardCode::Plain(code.to_owned())),
        ("edit_cell", Some(code)) => Some(match old.as_deref().filter(|o| !o.trim().is_empty()) {
            Some(old) if old != code => CardCode::Diff(celldiff::line_diff(old, code)),
            _ => CardCode::Plain(code.to_owned()),
        }),
        ("delete_cell", _) => old.map(CardCode::Struck),
        _ if one => old.map(CardCode::Plain),
        _ => None,
    };
    let waiting = code.is_none() && one && preview.is_none() && tool != "submit_changes";
    let code = code.or_else(|| waiting.then(|| CardCode::Plain("…".into())));
    let them = if question.names.len() > 1 || preview.is_some_and(|p| p.count > 1) { "them" } else { "it" };
    if let Some(n) = preview.map(|p| p.needed_ids.len()).filter(|n| *n > 0) {
        let need = if them == "it" { "it needs" } else { "they need" };
        let cells = if n == 1 { "cell" } else { "cells" };
        let has = if n == 1 { "hasn't" } else { "haven't" };
        lines.push((format!("Also runs {n} {cells} {need} that {has} run yet."), Tone::Muted));
    }
    if waiting {
        lines.push(("Finding out what it runs…".into(), Tone::Muted));
    } else if tool == "add_cell" {
        lines.push(("Nothing depends on it yet.".into(), Tone::Faint));
    } else if let Some(p) = preview.filter(|p| p.dependents > 0) {
        let n = p.dependents;
        let (cells, depend) = if n == 1 { ("cell", "depends") } else { ("cells", "depend") };
        lines.push((format!("Also re-runs {n} {cells} that {depend} on {them}."), Tone::Muted));
    }
    if tool == "delete_cell" {
        lines.push((format!("{} in the notebook brings the cell back.", crate::platform::shortcut!("Z")), Tone::Muted));
    }
    Prompt { heading: question.heading, verb: question.button, code, lines, names: question.names }
}

/// The agent's own prompt (Manual, or a call its rules ask about), in Endeavor's words.
/// `existing`: for a write, the file's text when the prompt came, if it was there.
fn agent_prompt(session: &Session, title: &str, kind: Option<ToolKind>, input: &serde_json::Value, path: Option<&Path>, existing: Option<&str>) -> Prompt {
    let field = |name: &str| input[name].as_str().filter(|s| !s.trim().is_empty());
    let here = crate::platform::this_computer!();
    let folder = tilde(Path::new(&session.place.path));
    // A file inside the session's folder by its path there, else by its name.
    let named = |file: &str| {
        Path::new(file).strip_prefix(&session.place.path).ok().map(|p| p.display().to_string()).filter(|p| !p.is_empty()).unwrap_or_else(|| file_name(file))
    };
    let file = path.map(|p| p.display().to_string()).or_else(|| field("file_path").or(field("notebook_path")).map(str::to_owned));
    let in_folder = |file: &str| {
        let dir = Path::new(file).parent().map(tilde).unwrap_or_else(|| folder.clone());
        (format!("In {dir}"), Tone::Muted)
    };

    if let Some(command) = field("command").filter(|_| kind.is_none_or(|k| k == ToolKind::Execute)) {
        let mut lines: Vec<(String, Tone)> = field("description").map(|d| (d.to_owned(), Tone::Secondary)).into_iter().collect();
        let cwd = field("cwd").map(|c| tilde(Path::new(c))).unwrap_or_else(|| folder.clone());
        lines.push((format!("In {cwd}"), Tone::Muted));
        return Prompt { heading: format!("Run a command on {here}?"), verb: "Run", code: Some(CardCode::Plain(command.to_owned())), lines, ..Default::default() };
    }
    if kind == Some(ToolKind::Fetch) {
        if let Some(url) = field("url") {
            return Prompt {
                heading: "Fetch a web page?".into(),
                verb: "Fetch",
                code: Some(CardCode::Plain(url.to_owned())),
                lines: vec![(format!("{} reads the page. Nothing on {here} changes.", session.agent.name()), Tone::Muted)],
                ..Default::default()
            };
        }
        if let Some(query) = field("query") {
            return Prompt { heading: "Search the web?".into(), verb: "Search", code: Some(CardCode::Plain(query.to_owned())), lines: vec![], ..Default::default() };
        }
    }
    if kind == Some(ToolKind::Edit)
        && let Some(file) = file.as_deref()
    {
        let edits: Vec<(&str, &str)> = match input["edits"].as_array() {
            Some(edits) => edits.iter().filter_map(|e| Some((e["old_string"].as_str()?, e["new_string"].as_str()?))).collect(),
            None => field("old_string").or(Some("")).zip(input["new_string"].as_str()).filter(|_| input["new_string"].is_string()).into_iter().collect(),
        };
        if !edits.is_empty() {
            let lines = edits.iter().flat_map(|(old, new)| celldiff::line_diff(old, new)).collect();
            return Prompt { heading: format!("Edit `{}`?", named(file)), verb: "Edit", code: Some(CardCode::Diff(lines)), lines: vec![in_folder(file)], ..Default::default() };
        }
        if let Some(content) = input["content"].as_str() {
            return match existing {
                Some(old) => Prompt {
                    heading: format!("Replace `{}`?", named(file)),
                    verb: "Replace",
                    code: Some(CardCode::Diff(celldiff::line_diff(old, content))),
                    lines: vec![in_folder(file)],
                    ..Default::default()
                },
                None => {
                    let dir = Path::new(file).parent().map(tilde).unwrap_or_else(|| folder.clone());
                    Prompt {
                        heading: format!("Create `{}`?", named(file)),
                        verb: "Create",
                        code: Some(CardCode::Plain(content.to_owned())),
                        lines: vec![(format!("New file in {dir}. Nothing runs."), Tone::Muted)],
                        ..Default::default()
                    }
                }
            };
        }
    }
    if let Some(tool) = celldiff::notebook_tool(title) {
        let nothing_runs = match session.mode_name().as_deref() {
            Some("Manual") => "Nothing runs. In Manual, every change to the notebook asks.",
            _ => "Nothing runs.",
        };
        let nothing_runs = (nothing_runs.to_owned(), Tone::Muted);
        let question = run_question(tool, None, input);
        let what = question.heading.split(" and run").next().unwrap_or("").trim_end_matches('?').to_owned();
        match tool {
            "edit_cell" if input["code"].is_string() => {
                let new = input["code"].as_str().unwrap_or("");
                let old = input["cell_id"].as_str().and_then(|id| session.cell_codes.get(id)).filter(|o| !o.trim().is_empty());
                let code = match old {
                    Some(old) if old != new => CardCode::Diff(celldiff::line_diff(old, new)),
                    _ => CardCode::Plain(new.to_owned()),
                };
                return Prompt { heading: format!("{what}?"), verb: "Edit", code: Some(code), lines: vec![nothing_runs], ..Default::default() };
            }
            "add_cell" if input["code"].is_string() => {
                let code = input["code"].as_str().unwrap_or("").to_owned();
                return Prompt { heading: format!("{what}?"), verb: "Add", code: Some(CardCode::Plain(code)), lines: vec![nothing_runs], ..Default::default() };
            }
            "edit_cells" => return Prompt { heading: format!("{what}?"), verb: "Edit", code: None, lines: vec![nothing_runs], ..Default::default() },
            _ => {}
        }
        if let Some(what) = runs::asked(title, input) {
            return Prompt { heading: format!("Let {} {what}?", session.agent.name()), verb: "Allow", code: None, lines: vec![], ..Default::default() };
        }
    }
    let short = cut_line(&plain_title(title), 60);
    let full = (short != title.trim() && !title.starts_with("mcp__")).then(|| (title.to_owned(), Tone::Muted));
    Prompt { heading: format!("Allow {short}?"), verb: "Allow", code: None, lines: full.into_iter().collect(), ..Default::default() }
}

/// A tool's title without its raw name: `mcp__github__create_issue` → "`create issue` from `github`".
fn plain_title(title: &str) -> String {
    match title.trim().strip_prefix("mcp__").map(|t| t.split_once("__").unwrap_or((t, ""))) {
        Some((server, "")) => format!("a `{server}` tool"),
        Some((server, name)) => format!("`{}` from `{server}`", name.replace('_', " ")),
        None => title.trim().to_owned(),
    }
}

pub fn render_approval(session: &Session, notebook_shown: bool, window: &Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ix = session.pending_permission()?;
    let view = approval_view(session)?;
    let key = session.key;
    let folder = view.folder.clone();
    let always_menu = session.asks.always_menu && folder.is_some();
    let buttons = view
        .buttons
        .into_iter()
        .enumerate()
        .map(|(i, CardButton { label, hint, weight, option, scope })| {
            let focus = session.approval_focus(i, cx);
            let id = ElementId::NamedInteger("perm".into(), (key << 32) | (ix as u64 * 16 + i as u64));
            let button = approval_button(id, &label, hint, weight, &focus).on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.answer(ix, &option, scope))));
            // "Always this session" with a ⌄ for "In this folder", when the agent keeps folder rules.
            let button = match (&folder, scope) {
                (Some(folder), Scope::Session) => always_split(session, ix, button, folder.clone(), always_menu, cx),
                _ => button.into_any_element(),
            };
            (weight, button)
        })
        .collect();
    let heading = card_heading(&view.heading, view.count.as_deref());
    let card = if let Some(plan) = view.plan {
        let shown = match (&plan.steps, plan.open) {
            (Some(steps), false) => steps.clone(),
            _ => plan.full.clone(),
        };
        let title = plan.title.map(|title| div().text_size(theme::chat_body()).text_color(theme::text_secondary()).child(title).into_any_element());
        let text = div()
            .id(ElementId::NamedInteger("plan".into(), key))
            .max_h(px(260.))
            .overflow_y_scroll()
            .text_size(theme::chat_body())
            .text_color(theme::text_row_active())
            .child(TextView::markdown(ElementId::NamedInteger("plan-text".into(), key), shown).style(plan_style()));
        let toggle = plan.steps.is_some().then(|| {
            let label = if plan.open { "Show only the steps" } else { "Show the whole plan" };
            toggle_link(ElementId::NamedInteger("plan-toggle".into(), key), label, plan.open, &session.plan_card_focus(cx))
                .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.plan_open = !s.plan_open)))
                .into_any_element()
        });
        let body = title.into_iter().chain([text.into_any_element()]).chain(toggle).collect();
        approval_card(heading, body, buttons)
    } else {
        let mut body: Vec<AnyElement> = Vec::new();
        // The asked-about cell is off screen in the notebook: a line to show it.
        if notebook_shown && session.asks.cells_visible == Some(false) && !view.cells.is_empty() {
            let cells = view.cells.clone();
            body.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(theme::chat_meta_small())
                    .text_color(theme::text_muted())
                    .child(glyph(Glyph::Code, theme::text_muted()))
                    .child(div().flex_1().min_w_0().truncate().font_family(theme::MONO).child(view.names.join(", ")))
                    .child(
                        div()
                            .id(ElementId::NamedInteger("perm-show".into(), key))
                            .role(Role::Link)
                            .aria_label("Show in notebook")
                            .cursor_pointer()
                            .text_color(theme::accent_text())
                            .hover(|s| s.underline())
                            .child("Show in notebook →")
                            .on_click(cx.listener(move |this, _, _, cx| this.reveal_cells(cells.clone(), cx))),
                    )
                    .into_any_element(),
            );
        }
        if let Some(code) = &view.code {
            let open = session.asks.code_open;
            let max_h = window.viewport_size().height * 0.4;
            body.push(code_box(code, open, max_h, ElementId::NamedInteger("perm-code".into(), key)));
            let n = code.line_count();
            if n > CODE_LINES {
                let label = if open { format!("Show the first {CODE_LINES} lines") } else { format!("Show all {n} lines") };
                body.push(
                    toggle_link(ElementId::NamedInteger("perm-code-toggle".into(), key), &label, open, &session.plan_card_focus(cx))
                        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.asks.code_open = !s.asks.code_open)))
                        .into_any_element(),
                );
            }
        }
        body.extend(view.lines.into_iter().map(|(text, tone)| {
            let color = match tone {
                Tone::Muted => theme::text_muted(),
                Tone::Secondary => theme::text_secondary(),
                Tone::Faint => theme::text_faint(),
            };
            inline_code(&text, theme::chat_meta_small()).text_color(color).into_any_element()
        }));
        approval_card(heading, body, buttons)
    };
    // One height through a run of queued prompts: the chat above doesn't move.
    // The opened code box doesn't count: closing it again shrinks the card.
    let height = session.asks.card_height.clone();
    let measure = !session.asks.code_open;
    let min = height.get().filter(|_| measure);
    let card = div()
        .relative()
        .flex()
        .flex_col()
        .when_some(min, |d, h| d.min_h(h))
        .child(card)
        .child(
            canvas(
                move |bounds, _, _| {
                    if measure && height.get().is_none_or(|h| bounds.size.height > h) {
                        height.set(Some(bounds.size.height));
                    }
                },
                |_, _, _, _| (),
            )
            .absolute()
            .size_full(),
        );
    let then = (!view.then.is_empty()).then(|| {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .pt(px(2.))
            .text_size(theme::chat_meta())
            .text_color(theme::text_muted())
            .child(glyph(Glyph::Clock, theme::text_muted()))
            .child(inline_code(&format!("Then: {}", view.then.join(" · ")), theme::chat_meta_small()).min_w_0())
    });
    Some(div().flex().flex_col().gap(px(6.)).child(card).children(then).into_any_element())
}

/// The outlined "Always this session" with its ⌄, which opens This session / In this folder.
fn always_split(session: &Session, ix: usize, button: Stateful<Div>, folder: PermissionOption, open: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let key = session.key;
    let chevron = div()
        .id(ElementId::NamedInteger("perm-always-more".into(), key))
        .role(Role::Button)
        .aria_label("More ways to always allow")
        .flex()
        .items_center()
        .justify_center()
        .w(px(20.))
        .h(px(30.))
        .rounded_r(px(5.))
        .border_2()
        .border_l_0()
        .border_color(theme::control_edge())
        .cursor_pointer()
        .hover(|s| s.bg(theme::bg_raised()))
        .when(open, |d| d.bg(theme::bg_raised()))
        .child(glyph(Glyph::Chevron, theme::text_muted()))
        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.asks.always_menu = !s.asks.always_menu)));
    let item = |id: &'static str, label: &'static str, hint: &'static str| {
        div()
            .id(ElementId::NamedInteger(id.into(), key))
            .role(Role::MenuItem)
            .aria_label(label)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::menu_hover()))
            .child(label)
            .child(div().text_color(theme::text_faint()).child(hint))
    };
    let allow = match &session.entries[ix] {
        Entry::Permission { options, .. } => option_of_kind(options, PermissionOptionKind::AllowOnce).cloned(),
        _ => None,
    };
    let menu = open.then(|| {
        let mut this_session = item("perm-always-session", "This session", crate::platform::shortcut!("⏎"));
        if let Some(allow) = allow {
            this_session = this_session.on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.answer(ix, &allow, Scope::Session))));
        }
        let in_folder = item("perm-always-folder", "In this folder", "").on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.answer(ix, &folder, Scope::Folder))));
        deferred(
            anchored().anchor(Anchor::TopLeft).child(
                div()
                    .id(ElementId::NamedInteger("perm-always-menu".into(), key))
                    .occlude()
                    .mt(px(4.))
                    .w(px(170.))
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .map(theme::popover)
                    .text_size(theme::chat_meta())
                    .text_color(theme::text_primary())
                    .on_mouse_down_out(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.asks.always_menu = false)))
                    .child(this_session)
                    .child(in_folder),
            ),
        )
        .with_priority(1)
    });
    div()
        .relative()
        .flex()
        .child(button.rounded_r(px(0.)))
        .child(chevron)
        .child(div().absolute().top(px(30.)).left_0().children(menu))
        .into_any_element()
}

/// A card's code box: the first lines (all of them, scrolling past `max_h`, once opened).
fn code_box(code: &CardCode, open: bool, max_h: Pixels, id: ElementId) -> AnyElement {
    let lines = code.lines();
    let total = lines.len();
    let struck = matches!(code, CardCode::Struck(_));
    let rows = lines.into_iter().take(if open { usize::MAX } else { CODE_LINES }).map(|(change, line)| {
        let (sign, bg) = match change {
            Change::Added => (Some("+ "), Some(theme::diff_add_tint())),
            Change::Removed => (Some("- "), Some(theme::diff_del_tint())),
            Change::Same if matches!(code, CardCode::Diff(_)) => (Some("  "), None),
            Change::Same => (None, None),
        };
        div()
            .px(px(6.))
            .whitespace_nowrap()
            .when_some(bg, |d, bg| d.bg(bg))
            .when(struck, |d| d.line_through())
            .child(format!("{}{}", sign.unwrap_or(""), if line.is_empty() { " " } else { &line }))
    });
    let more = (!open && total > CODE_LINES).then(|| div().px(px(6.)).child("…"));
    div()
        .id(id)
        .font_family(theme::MONO)
        .text_size(theme::chat_code())
        .py(px(6.))
        .rounded(px(4.))
        .bg(theme::bg_page())
        .text_color(theme::text_secondary())
        .when(open, |d| d.max_h(max_h).overflow_y_scroll())
        .overflow_x_hidden()
        .children(rows)
        .children(more)
        .into_any_element()
}

/// A faint "Show all …" / "Show the whole plan" toggle under a card's code or plan.
fn toggle_link(id: ElementId, label: &str, open: bool, focus: &FocusHandle) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.to_owned())
        .flex()
        .items_center()
        .gap(px(4.))
        .cursor_pointer()
        .text_color(theme::text_faint())
        .hover(|s| s.text_color(theme::text_secondary()))
        .track_focus(focus)
        .tab_stop(true)
        .focus_ring_on(theme::bg_urgent())
        .child(label.to_owned())
        .child(if open { "⌄" } else { "›" })
}

/// The agent's own options as buttons, for a prompt without an allow-once
/// option: declining first, then the rest, in Endeavor's words.
fn option_buttons(options: &[PermissionOption]) -> Vec<(String, &'static str, PermissionOption, Scope)> {
    let rank = |o: &PermissionOption| match o.kind {
        PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways => 0,
        PermissionOptionKind::AllowOnce => 2,
        _ => 1,
    };
    let mut sorted: Vec<&PermissionOption> = options.iter().collect();
    sorted.sort_by_key(|o| rank(o));
    sorted
        .into_iter()
        .map(|o| {
            let (label, hint, scope) = match o.kind {
                PermissionOptionKind::RejectOnce => ("Deny", "esc", Scope::Once),
                PermissionOptionKind::RejectAlways => ("Never", "", Scope::Once),
                PermissionOptionKind::AllowAlways if crate::permits::folder_rule_option(options).is_some() => ("Always in this folder", "", Scope::Folder),
                PermissionOptionKind::AllowAlways => ("Always", "", Scope::Once),
                _ => ("Allow", "⏎", Scope::Once),
            };
            (label.to_owned(), hint, o.clone(), scope)
        })
        .collect()
}

/// Plan mode's plan, as its card shows it.
pub(crate) struct PlanCard {
    /// The plan's own title, from a heading that opens it ("Plan:" dropped).
    pub title: Option<String>,
    /// Its first numbered list, as written (markdown), when it has one: the
    /// card shows these steps until the user asks for the whole plan.
    pub steps: Option<String>,
    /// The whole plan (markdown), without its title.
    pub full: String,
    /// The whole plan is showing.
    pub open: bool,
}

/// A plan's own title, from the heading that opens it.
pub(crate) fn plan_title(markdown: &str) -> Option<String> {
    plan_card(markdown).title
}

/// Splits a plan (markdown) into its title, its numbered steps and the rest.
/// Steps are the first run of top-level numbered items ("1. …", "2) …"),
/// with the lines indented under them; the list needs two items or more.
fn plan_card(markdown: &str) -> PlanCard {
    let mut lines: Vec<&str> = markdown.lines().collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    let title = lines.first().and_then(|l| l.strip_prefix('#')).map(|l| l.trim_start_matches('#').trim()).map(|t| {
        let t = t.strip_prefix("Plan:").or_else(|| t.strip_prefix("Plan -")).or_else(|| t.strip_prefix("Plan —")).unwrap_or(t).trim();
        t.to_string()
    });
    if title.is_some() {
        lines.remove(0);
    }
    let title = title.filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("plan"));
    let full = lines.join("\n").trim().to_string();

    let numbered = |line: &str| {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && line[digits..].starts_with(['.', ')']) && line[digits + 1..].starts_with(' ')
    };
    let mut steps: Vec<&str> = Vec::new();
    let mut items = 0;
    for (i, line) in lines.iter().enumerate() {
        if numbered(line) {
            items += 1;
        } else if steps.is_empty() {
            continue;
        } else if line.trim().is_empty() {
            // A blank line inside the list only if the list goes on after it.
            let next = lines[i + 1..].iter().find(|l| !l.trim().is_empty());
            if !next.is_some_and(|l| numbered(l) || l.starts_with([' ', '\t'])) {
                break;
            }
        } else if !line.starts_with([' ', '\t']) {
            break;
        }
        steps.push(line);
    }
    let steps = (items >= 2).then(|| steps.join("\n").trim_end().to_string());
    PlanCard { title, steps, full, open: false }
}

/// A plan's markdown inside its card: headings at body size, so the steps and
/// sections read as one plan rather than a document.
fn plan_style() -> TextViewStyle {
    markdown_style().heading_font_size(|_, _| theme::chat_body())
}

/// Plan mode's end: Keep planning · Start in Auto · **Start**.
fn plan_buttons(options: &[PermissionOption]) -> Vec<CardButton> {
    // (label, key, option, runs without asking, weight)
    let buttons: Vec<(&str, &'static str, Option<&PermissionOption>, bool, Weight)> = vec![
        ("Keep planning", "esc", option_of_kind(options, PermissionOptionKind::RejectOnce), false, Weight::Quiet),
        ("Start in Auto", crate::platform::shortcut!("⏎"), plan_option(options), true, Weight::Outlined),
        ("Start", "⏎", plan_option(options), false, Weight::Primary),
    ];
    buttons
        .into_iter()
        .filter_map(|(label, hint, option, stop, weight)| Some(CardButton { label: label.into(), hint, weight, option: option?.clone(), scope: if stop { Scope::Session } else { Scope::Once } }))
        .collect()
}

/// "Let this notebook run?"'s line about it: "bootstrap.jl · 7 cells · uses CSV, Plots".
fn notebook_summary(file: Option<&str>, preview: Option<&pluto::RunPreview>) -> Option<String> {
    let mut parts: Vec<String> = file.map(str::to_owned).into_iter().collect();
    if let Some(p) = preview {
        parts.push(if p.count == 1 { "1 cell".into() } else { format!("{} cells", p.count) });
        let packages = if p.packages.is_empty() { imported_packages(p.cells.iter().map(|c| c.code.as_str())) } else { p.packages.clone() };
        if !packages.is_empty() {
            parts.push(format!("uses {}", packages.join(", ")));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Packages a notebook's code imports (`using A, B`, `import C: f`), in order.
fn imported_packages<'a>(codes: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in codes.flat_map(str::lines) {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("using ").or_else(|| line.strip_prefix("import ")) else { continue };
        let list = rest.split('#').next().unwrap_or("").split(':').next().unwrap_or("");
        for part in list.split(',') {
            let name = part.trim().split(['.', ' ']).next().unwrap_or("");
            let valid = name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            if valid && !["Base", "Core", "Main"].contains(&name) && !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
    }
    names
}

/// `text`'s first non-blank line, cut to `max` characters with "…".
pub(crate) fn cut_line(text: &str, max: usize) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

/// A run card's question ("Run `fit`?", "Delete a cell?", "Add `x` and run
/// it?"), the word on its ⏎ button, and the cells it would run.
pub(crate) struct RunQuestion {
    pub heading: String,
    pub button: &'static str,
    pub names: Vec<String>,
}

/// Every run card asks "<Verb> <what>?": `name` for one cell with a name, "a
/// cell" for one without, "N cells", "all N cells". Before the runtime says
/// what would run, the call's input says what it can. A cell is named by what
/// it defines, else by its first line; a cell being edited or added by its new
/// code, since the preview only knows the code before the edit.
fn run_question(tool: &str, preview: Option<&pluto::RunPreview>, input: &serde_json::Value) -> RunQuestion {
    // The runtime can't say what a cell that doesn't exist yet would run.
    let preview = preview.filter(|_| tool != "add_cell");
    let new_code = input["code"].as_str().filter(|_| matches!(tool, "edit_cell" | "add_cell"));
    let label = |code: &str| if code.trim().is_empty() { String::new() } else { cell_label(code) };
    let names: Vec<String> = match (preview, new_code) {
        (_, Some(code)) => vec![label(code)],
        (Some(p), None) => p.cells.iter().map(|c| c.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| label(&c.code))).collect(),
        (None, None) => Vec::new(),
    };
    let listed = |field: &str| input[field].as_array().map(Vec::len);
    let (all, count) = match preview {
        Some(p) => (p.all, Some(p.count)),
        None => (tool == "run_all_cells", listed("cell_ids").or(listed("cells")).or((tool != "submit_changes" && tool != "run_all_cells").then_some(1))),
    };
    let what = match (all, count) {
        (true, Some(n)) if n > 1 => format!("all {n} cells"),
        (true, _) => "all cells".into(),
        (false, Some(1)) => match names.first().filter(|n| !n.is_empty()) {
            Some(name) => format!("`{name}`"),
            None => "a cell".into(),
        },
        (false, Some(n)) => format!("{n} cells"),
        (false, None) => "the changed cells".into(),
    };
    let them = if count == Some(1) { "it" } else { "them" };
    let (heading, button) = match tool {
        "delete_cell" => (format!("Delete {what}?"), "Delete"),
        "add_cell" => (format!("Add {what} and run it?"), "Add and run"),
        "edit_cell" | "edit_cells" => (format!("Edit {what} and run {them}?"), "Edit and run"),
        _ => (format!("Run {what}?"), "Run"),
    };
    RunQuestion { heading, button, names }
}

/// The approval card's frame: accent edge, soft ring, a heading, its body, and
/// a footer of buttons (the one that declines on the left).
fn approval_card(heading: AnyElement, body: Vec<AnyElement>, buttons: Vec<(Weight, AnyElement)>) -> AnyElement {
    let (left, right): (Vec<_>, Vec<_>) = buttons.into_iter().partition(|(weight, _)| *weight == Weight::Quiet);
    div()
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(theme::accent().opacity(0.7))
        .bg(theme::bg_urgent())
        .shadow(vec![BoxShadow { color: theme::approval_ring().into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(3.), inset: false }])
        .child(div().px(px(10.)).pt(px(10.)).pb(px(6.)).child(heading))
        .when(!body.is_empty(), |d| d.child(div().flex().flex_col().gap(px(6.)).px(px(10.)).pb(px(10.)).text_size(theme::chat_meta()).children(body)))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap(px(4.))
                .px(px(8.))
                .py(px(8.))
                .border_t_1()
                .border_color(theme::accent().opacity(0.25))
                .children(left.into_iter().map(|(_, b)| b.into_any_element()))
                .child(div().flex_1().flex().justify_end().gap(px(4.)).children(right.into_iter().map(|(_, b)| b))),
        )
        .into_any_element()
}

/// A card's heading: chat subhead size, medium, with `backticked` names in
/// chat code size mono.
/// `count` ("1 of 3") sits at its right, muted.
fn card_heading(text: &str, count: Option<&str>) -> AnyElement {
    let heading = inline_code(text, theme::chat_code()).text_size(theme::chat_subhead()).font_weight(FontWeight::MEDIUM).text_color(theme::text_primary());
    match count {
        None => heading.into_any_element(),
        Some(count) => div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(8.))
            .child(heading.min_w_0())
            .child(div().flex_none().pt(px(2.)).text_size(theme::chat_meta_small()).text_color(theme::text_muted()).child(count.to_owned()))
            .into_any_element(),
    }
}

fn approval_button(id: ElementId, label: &str, hint: &str, weight: Weight, focus: &FocusHandle) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .items_center()
        .gap(px(4.))
        .h(px(30.))
        .px(px(match weight {
            Weight::Primary => 10.,
            Weight::Outlined => 8.,
            Weight::Quiet => 6.,
        }))
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(theme::chat_meta())
        .border_2()
        .border_color(gpui::transparent_black())
        .track_focus(focus)
        .tab_stop(true)
        .map(|d| match weight {
            Weight::Primary => d.bg(theme::accent()).text_color(gpui::white()).font_weight(FontWeight::SEMIBOLD).focus_ring_on(theme::bg_urgent()),
            // Its outline is the reserved 2 px border, so focus only recolours it.
            Weight::Outlined => d
                .border_color(theme::control_edge())
                .text_color(theme::text_row_active())
                .hover(|s| s.bg(theme::bg_raised()))
                .focus_visible(|s| s.border_color(theme::focus_ring())),
            Weight::Quiet => d.text_color(theme::text_secondary()).hover(|s| s.bg(theme::bg_raised())).focus_ring_on(theme::bg_urgent()),
        })
        .child(label.to_string())
        .when(!hint.is_empty(), |d| d.child(div().text_size(theme::chat_meta_small()).font_weight(FontWeight::NORMAL).opacity(0.6).child(hint.to_string())))
}

/// Text with `backticked` spans in mono at `mono` size.
fn inline_code(text: &str, mono: Pixels) -> Div {
    // Plain text wraps as text does, even inside a long word.
    if !text.contains('`') {
        return div().child(text.to_owned());
    }
    div().flex().flex_wrap().children(text.split('`').enumerate().map(move |(i, part)| {
        // Flex drops a part's edge spaces; non-breaking ones survive.
        let d = div().child(part.replace(' ', "\u{a0}"));
        if i % 2 == 1 { d.font_family(theme::MONO).font_weight(FontWeight::NORMAL).text_size(mono) } else { d }
    }))
}

/// "Progress · 1 of 3": steps done of all.
pub(crate) fn progress(entries: &[PlanEntry]) -> String {
    let done = entries.iter().filter(|e| e.status == PlanEntryStatus::Completed).count();
    format!("Progress · {done} of {}", entries.len())
}

/// The plan checklist: ✓ done (struck through), ◐ current (bright), ○ upcoming (grey).
pub(crate) fn plan_rows(entries: &[PlanEntry]) -> impl Iterator<Item = Div> + '_ {
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
                    .role(Role::Button)
                    .aria_label(format!("Plan, {}", if folded { "collapsed" } else { "expanded" }))
                    .flex()
                    .justify_between()
                    .cursor_pointer()
                    .text_size(theme::chat_meta())
                    .text_color(theme::text_muted())
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .track_focus(&session.pinned_plan_focus(cx))
                    .tab_stop(true)
                    .focus_ring()
                    .child(progress(entries))
                    .child(if folded { "›" } else { "⌄" })
                    .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.plan_folded = !s.plan_folded))),
            )
            .when(!folded, |d| d.children(plan_rows(entries)))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use crate::celldiff::Change;
    use crate::hosts::Place;
    use crate::session::{Entry, Scope, Session};
    use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, SessionMode, SessionModeState, ToolCallId, ToolKind};
    use serde_json::{Value, json};

    /// Claude Code's options for a Bash call with a suggested rule (the adapter's own labels).
    fn claude_options() -> Vec<PermissionOption> {
        vec![
            PermissionOption::new("allow-once", "Yes", PermissionOptionKind::AllowOnce),
            PermissionOption::new("allow-with-updates", "Yes, and don't ask again for npm test commands", PermissionOptionKind::AllowAlways),
            PermissionOption::new("reject", "No", PermissionOptionKind::RejectOnce),
        ]
    }

    fn prompt(s: &mut Session, title: &str, kind: ToolKind, input: Value, runs_code: bool, options: Vec<PermissionOption>) -> super::ApprovalView {
        let tool = crate::celldiff::notebook_tool(title).map(str::to_owned);
        s.entries.push(Entry::Permission {
            call: ToolCallId::new("c"),
            title: title.into(),
            code: None,
            options,
            responder: None,
            runs_code,
            tool,
            input,
            kind: Some(kind),
            path: None,
            preview: None,
            plan: None,
        });
        super::view_at(s, s.entries.len() - 1).expect("a card")
    }

    #[test]
    fn command_enter_reaches_this_session_only_on_a_card_that_offers_it() {
        let waiting = |title: &str, input: Value| {
            let mut s = Session::new(1, Place::local("/Users/jc/projects/decay-fits"), None);
            prompt(&mut s, title, ToolKind::Execute, input, true, claude_options());
            if let Some(Entry::Permission { responder, .. }) = s.entries.last_mut() {
                *responder = Some(crate::session::Asker::Runtime(1));
            }
            s
        };
        let safe = waiting("mcp__notebook__allow_execution", json!({ "notebook_id": "n" }));
        assert_eq!(super::command_enter_scope(&safe), Scope::Once, "Let this notebook run? has no Always");
        let run = waiting("mcp__notebook__execute_cell", json!({ "notebook_id": "n", "cell_id": "c1" }));
        assert_eq!(super::command_enter_scope(&run), Scope::Session);
    }

    fn buttons(view: &super::ApprovalView) -> Vec<(String, &'static str, Scope, String)> {
        view.buttons.iter().map(|b| (b.label.clone(), b.hint, b.scope, b.option.option_id.to_string())).collect()
    }

    #[test]
    fn the_agents_prompts_read_in_endeavors_words_never_its_labels() {
        let mut s = Session::new(1, Place::local("/Users/jc/projects/decay-fits"), None);
        let shortcut = crate::platform::shortcut!("⏎");

        let bash = prompt(&mut s, "npm test", ToolKind::Execute, json!({ "command": "npm test", "description": "Runs the tests" }), false, claude_options());
        assert_eq!(bash.heading, format!("Run a command on {}?", crate::platform::this_computer!()));
        assert_eq!(bash.code, Some(super::CardCode::Plain("npm test".into())));
        assert_eq!(bash.lines[0].0, "Runs the tests");
        assert!(bash.lines[1].0.starts_with("In ") && bash.lines[1].0.ends_with("projects/decay-fits"));
        // ⏎ allows once, ⌘⏎ is Always this session (allow-once, remembered by Endeavor),
        // and "In this folder" is the agent's own allow-always, under the ⌄.
        assert_eq!(
            buttons(&bash),
            vec![
                ("Deny".into(), "esc", Scope::Once, "reject".into()),
                ("Always this session".into(), shortcut, Scope::Session, "allow-once".into()),
                ("Run".into(), "⏎", Scope::Once, "allow-once".into())
            ]
        );
        assert_eq!(bash.folder.map(|o| o.option_id.to_string()).as_deref(), Some("allow-with-updates"));

        let edit = prompt(&mut s, "Edit notes.md", ToolKind::Edit, json!({ "file_path": "/Users/jc/projects/decay-fits/notes.md", "old_string": "a\nold", "new_string": "a\nnew" }), false, claude_options()[..1].iter().chain(&claude_options()[2..]).cloned().collect());
        assert_eq!(edit.heading, "Edit `notes.md`?");
        assert_eq!(edit.code, Some(super::CardCode::Diff(vec![(Change::Same, "a".into()), (Change::Removed, "old".into()), (Change::Added, "new".into())])));
        assert_eq!(buttons(&edit).iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), vec!["Deny", "Always this session", "Edit"]);
        assert!(edit.folder.is_none(), "no allow-always option: no In this folder");

        let create = prompt(&mut s, "Write", ToolKind::Edit, json!({ "file_path": "/Users/jc/projects/decay-fits/scripts/clean.jl", "content": "using CSV" }), false, claude_options());
        assert_eq!(create.heading, "Create `scripts/clean.jl`?");
        assert!(create.lines[0].0.starts_with("New file in ") && create.lines[0].0.ends_with("decay-fits/scripts. Nothing runs."));
        assert_eq!(buttons(&create)[2].0, "Create");

        let fetch = prompt(&mut s, "Fetch", ToolKind::Fetch, json!({ "url": "https://juliastats.org/Bootstrap.jl/stable/" }), false, claude_options());
        assert_eq!((fetch.heading.as_str(), buttons(&fetch)[2].0.as_str()), ("Fetch a web page?", "Fetch"));
        assert_eq!(fetch.lines[0].0, format!("Claude reads the page. Nothing on {} changes.", crate::platform::this_computer!()));

        let mcp = prompt(&mut s, "mcp__github__create_issue", ToolKind::Other, json!({}), false, claude_options());
        assert_eq!(mcp.heading, "Allow `create issue` from `github`?");
        assert!(mcp.lines.is_empty(), "never the raw tool name");
        assert_eq!(buttons(&mcp)[2].0, "Allow");

        let modes = |current: &str| Some(SessionModeState::new(current.to_owned(), vec![SessionMode::new("default", "Manual"), SessionMode::new("plan", "Plan"), SessionMode::new("auto", "Auto")]));
        s.modes = modes("default");
        let manual = prompt(&mut s, "mcp__notebook__add_cell", ToolKind::Other, json!({ "code": "residuals = 1" }), false, claude_options());
        assert_eq!((manual.heading.as_str(), buttons(&manual)[2].0.as_str()), ("Add `residuals`?", "Add"));
        assert_eq!(manual.lines[0].0, "Nothing runs. In Manual, every change to the notebook asks.");
        s.modes = modes("plan");
        let plan = prompt(&mut s, "mcp__notebook__edit_cell", ToolKind::Other, json!({ "cell_id": "x", "code": "residuals = 2" }), false, claude_options());
        assert_eq!(plan.lines[0].0, "Nothing runs.", "only Manual asks before every change");
    }

    #[test]
    fn a_codex_sessions_prompts_name_codex() {
        let mut s = Session::new(1, Place::local("/Users/jc/projects/decay-fits"), None);
        s.agent = crate::agent::Agent::Codex;
        let fetch = prompt(&mut s, "Fetch", ToolKind::Fetch, json!({ "url": "https://juliastats.org/Bootstrap.jl/stable/" }), false, claude_options());
        assert_eq!(fetch.lines[0].0, format!("Codex reads the page. Nothing on {} changes.", crate::platform::this_computer!()));
        let read = prompt(&mut s, "mcp__notebook__read_cell", ToolKind::Other, json!({}), false, claude_options());
        assert_eq!(read.heading, "Let Codex read a cell?");
    }

    #[test]
    fn run_cards_show_what_runs_and_offer_no_folder_rule() {
        let mut s = Session::new(1, Place::local("/tmp/p"), None);
        let gate = || vec![PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce), PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce)];
        let add = prompt(&mut s, "mcp__notebook__add_cell", ToolKind::Other, json!({ "code": "residuals = y .- fit", "run_after": true }), true, gate());
        assert_eq!(add.heading, "Add `residuals` and run it?");
        assert_eq!(add.code, Some(super::CardCode::Plain("residuals = y .- fit".into())));
        assert_eq!(add.lines.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), vec!["Nothing depends on it yet."]);
        assert_eq!(buttons(&add).iter().map(|b| (b.0.as_str(), b.2)).collect::<Vec<_>>(), vec![("Deny", Scope::Once), ("Always this session", Scope::Session), ("Add and run", Scope::Once)]);
        assert!(add.folder.is_none());

        let run = prompt(&mut s, "mcp__notebook__execute_cell", ToolKind::Other, json!({ "cell_id": "a" }), true, gate());
        assert_eq!(run.heading, "Run a cell?");
        assert_eq!(run.code, Some(super::CardCode::Plain("…".into())), "before the runtime says what it runs");
        assert_eq!(run.lines[0].0, "Finding out what it runs…");
        assert_eq!(run.cells, vec!["a".to_string()]);
        // Once the runtime says: `b` needs `a`, which never ran, and `c` re-runs after it.
        if let Some(Entry::Permission { preview, .. }) = s.entries.last_mut() {
            *preview = Some(crate::pluto::RunPreview {
                count: 1,
                cells: vec![crate::pluto::PreviewCell { id: Some("b".into()), name: Some("b".into()), code: "b = a + 1".into() }],
                needed_ids: vec!["a".into()],
                dependents: 1,
                ..Default::default()
            });
        }
        let run = super::view_at(&s, s.entries.len() - 1).unwrap();
        assert_eq!(run.lines.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), vec!["Also runs 1 cell it needs that hasn't run yet.", "Also re-runs 1 cell that depends on it."]);
        assert_eq!(run.needed, vec!["a".to_string()]);

        let long = super::CardCode::Plain((1..=15).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n"));
        assert_eq!(long.line_count(), 15);
        assert_eq!(long.text(false).lines().count(), 9, "eight lines and …");
        assert_eq!(long.text(true).lines().count(), 15);
    }

    #[test]
    fn run_cards_name_cells_that_have_no_name_yet() {
        use crate::pluto::{PreviewCell, RunPreview};
        use serde_json::json;
        let one = |name: Option<&str>, code: &str| RunPreview {
            count: 1,
            cells: vec![PreviewCell { id: None, name: name.map(str::to_owned), code: code.into() }],
            ..Default::default()
        };
        let heading = |tool, p: &RunPreview, input| super::run_question(tool, Some(p), &input).heading;
        // A new notebook's empty first cell, edited and run in one call.
        assert_eq!(heading("edit_cell", &one(None, ""), json!({ "code": "x = 1\ny = 2", "run_after": true })), "Edit `x` and run it?");
        assert_eq!(heading("execute_cell", &one(None, ""), json!({})), "Run a cell?");
        assert_eq!(heading("execute_cell", &one(Some(""), "  \n"), json!({})), "Run a cell?");
        assert_eq!(heading("delete_cell", &one(None, ""), json!({})), "Delete a cell?");
        assert_eq!(heading("execute_cell", &one(Some("fit, model"), "fit = 1"), json!({})), "Run `fit, model`?");
        assert_eq!(heading("execute_cell", &one(None, "md\"# Intro\""), json!({})), "Run `Intro`?");
    }

    #[test]
    fn a_plan_card_shows_the_plans_numbered_steps() {
        let plan = "# Plan: Add small analysis to cards.jl\n\n## Context\nThe notebook defines `a`.\n\n## Approach\nAdd 3 cells:\n\n1. **Values** — `values = [a, b]`\n2. **Mean** — `m = sum(values) / 2`\n   (no Statistics needed)\n\n3. **Plot** — `bar(values)`\n\n## Verification\n- read each cell\n";
        let card = super::plan_card(plan);
        assert_eq!(card.title.as_deref(), Some("Add small analysis to cards.jl"));
        assert_eq!(
            card.steps.as_deref(),
            Some("1. **Values** — `values = [a, b]`\n2. **Mean** — `m = sum(values) / 2`\n   (no Statistics needed)\n\n3. **Plot** — `bar(values)`")
        );
        assert!(card.full.starts_with("## Context") && card.full.ends_with("- read each cell"));

        let bullets = super::plan_card("I'll do this:\n- add a cell\n- run it");
        assert_eq!((bullets.title, bullets.steps), (None, None), "no numbered steps: the whole plan shows");
        assert_eq!(bullets.full, "I'll do this:\n- add a cell\n- run it");
        assert_eq!(super::plan_card("# Plan\n1. one\n2. two").title, None, "a bare \"Plan\" heading is the card's own");
    }

    #[test]
    fn run_cards_ask_one_way_with_or_without_a_preview() {
        use crate::pluto::RunPreview;
        use serde_json::json;
        let ask = |tool, p: Option<RunPreview>, input| {
            let q = super::run_question(tool, p.as_ref(), &input);
            (q.heading, q.button)
        };
        let cells = |count, all| Some(RunPreview { count, all, ..Default::default() });
        assert_eq!(ask("submit_changes", cells(3, false), json!({})), ("Run 3 cells?".into(), "Run"));
        assert_eq!(ask("run_all_cells", cells(7, true), json!({})), ("Run all 7 cells?".into(), "Run"));
        assert_eq!(ask("run_all_cells", None, json!({})), ("Run all cells?".into(), "Run"));
        assert_eq!(ask("submit_changes", None, json!({})), ("Run the changed cells?".into(), "Run"));
        assert_eq!(ask("submit_changes", None, json!({ "cell_ids": ["a", "b"] })), ("Run 2 cells?".into(), "Run"));
        assert_eq!(ask("execute_cell", None, json!({ "cell_id": "a" })), ("Run a cell?".into(), "Run"));
        assert_eq!(ask("add_cell", None, json!({ "code": "residuals = y .- fit" })), ("Add `residuals` and run it?".into(), "Add and run"));
        assert_eq!(ask("add_cell", cells(0, false), json!({ "code": "d = c + 1" })), ("Add `d` and run it?".into(), "Add and run"), "the runtime doesn't know a new cell");
        assert_eq!(ask("add_cell", None, json!({ "code": "plot(x, y)" })), ("Add `plot(x, y)` and run it?".into(), "Add and run"));
        assert_eq!(ask("edit_cells", None, json!({ "cells": [{}, {}] })), ("Edit 2 cells and run them?".into(), "Edit and run"));
        assert_eq!(ask("delete_cell", None, json!({ "cell_id": "a" })), ("Delete a cell?".into(), "Delete"));
    }

    #[test]
    fn the_run_notebook_card_says_what_the_notebook_uses() {
        use crate::pluto::{PreviewCell, RunPreview};
        let cell = |code: &str| PreviewCell { id: None, name: None, code: code.into() };
        let preview = RunPreview {
            all: true,
            count: 7,
            cells: vec![cell("using CSV, DataFrames # data"), cell("import LsqFit: curve_fit\nusing Base.Threads"), cell("using Plots, CSV")],
            ..Default::default()
        };
        assert_eq!(super::notebook_summary(Some("bootstrap.jl"), Some(&preview)).as_deref(), Some("bootstrap.jl · 7 cells · uses CSV, DataFrames, LsqFit, Plots"));
        let named = RunPreview { all: true, count: 2, packages: vec!["Colors".into(), "Statistics".into()], ..Default::default() };
        assert_eq!(super::notebook_summary(Some("colors.jl"), Some(&named)).as_deref(), Some("colors.jl · 2 cells · uses Colors, Statistics"));
        assert_eq!(super::notebook_summary(Some("bootstrap.jl"), None).as_deref(), Some("bootstrap.jl"));
        assert_eq!(super::notebook_summary(None, None), None);
    }
}
