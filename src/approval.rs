//! The pending-approval card above the composer: run/edit/plan prompts, the
//! plan card, the pinned running plan, and the queued-message list.

use std::path::Path;

use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, PlanEntry, PlanEntryStatus};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::text::{TextView, TextViewStyle};

use crate::Workspace;
use crate::pluto;
use crate::runs;
use crate::session::{Entry, Session, defined_name, folder_name, markdown_style, option_of_kind, plan_option};
use crate::theme;

/// The pending approval card, as it reads.
pub(crate) struct ApprovalView {
    pub heading: String,
    /// Code it would run, cut to its first lines.
    pub code: Option<String>,
    pub lines: Vec<(String, Tone)>,
    /// Plan mode's end: the plan to approve, in place of code and lines.
    pub plan: Option<PlanCard>,
    pub buttons: Vec<CardButton>,
}

/// A line's colour; `backticked` names in it are mono.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tone {
    Muted,
    Secondary,
}

pub(crate) struct CardButton {
    pub label: String,
    /// The key that presses it.
    pub hint: &'static str,
    pub weight: Weight,
    option: PermissionOption,
    /// Approve this session's runs from now on.
    stop: bool,
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

/// The pending approval, pinned above the composer: the only heavy element
/// (accent edge, soft ring, filled primary). Runs get "Run N cells?", the cells,
/// and how many dependents re-run; other prompts get the agent's own options.
pub(crate) fn approval_view(session: &Session) -> Option<ApprovalView> {
    let ix = session.pending_permission()?;
    let Entry::Permission { title, code, options, runs_code, tool, input, preview, plan, .. } = &session.entries[ix] else { return None };
    if let Some(plan) = plan {
        let card = PlanCard { open: session.plan_open, ..plan_card(plan) };
        return Some(ApprovalView { heading: "Plan".into(), code: None, lines: vec![], plan: Some(card), buttons: plan_buttons(options) });
    }
    let tool = tool.as_deref().unwrap_or("");

    let mut run_word = "Run";
    let (heading, lines): (String, Vec<(String, Tone)>) = if !*runs_code {
        let muted = |text: &str| (text.to_owned(), Tone::Muted);
        if let Some(what) = runs::asked(title, input) {
            (format!("Let Claude {what}?"), vec![])
        } else if input["command"].is_string() {
            // Claude Code's own shell: its title is the whole command, shown below instead.
            ("Run a command on This Mac?".into(), input["description"].as_str().map(muted).into_iter().collect())
        } else {
            let short = cut_line(title, 60);
            let full = (short != title.trim()).then(|| muted(title));
            (format!("Allow {short}?"), full.into_iter().collect())
        }
    } else if tool == "run_shell" {
        let host = session.server.clone().unwrap_or_else(|| "the server".into());
        let folder = session.place.path.display().to_string();
        let cwd = input["cwd"].as_str().filter(|c| !c.is_empty()).unwrap_or(&folder);
        (format!("Run a command on {host}?"), vec![(format!("In {cwd}"), Tone::Muted)])
    } else if tool == "allow_execution" {
        let file = session.notebook_path.as_deref().map(|p| folder_name(Path::new(p)));
        let host = session.server.clone().unwrap_or_else(|| "This Mac".into());
        let mut lines = Vec::new();
        if let Some(line) = notebook_summary(file.as_deref(), preview.as_ref()) {
            lines.push((line, Tone::Secondary));
        }
        lines.push((format!("Nothing runs yet. Running it lets its code read and change files on {host}."), Tone::Muted));
        ("Let this notebook run?".into(), lines)
    } else {
        let question = run_question(tool, preview.as_ref(), input);
        run_word = question.button;
        let mut lines = Vec::new();
        if question.names.len() > 1 && preview.as_ref().is_none_or(|p| !p.all) {
            const SHOWN: usize = 5;
            let mut names = question.names.iter().take(SHOWN).map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
            if question.names.len() > SHOWN {
                names.push_str(&format!(" and {} more", question.names.len() - SHOWN));
            }
            lines.push((names, Tone::Secondary));
        }
        if let Some(p) = preview.as_ref().filter(|p| p.dependents > 0) {
            let them = if p.count == 1 { "it" } else { "them" };
            let n = p.dependents;
            let cells = if n == 1 { "cell" } else { "cells" };
            lines.push((format!("Also re-runs {n} {cells} that depend on {them}."), Tone::Muted));
        }
        (question.heading, lines)
    };
    // A single cell's code (or the new cell's) is short enough to show.
    let code = code
        .clone()
        .or_else(|| input["code"].as_str().map(str::to_owned))
        .or_else(|| input["command"].as_str().map(str::to_owned))
        .or_else(|| preview.as_ref().and_then(|p| p.cells.first()).map(|c| c.code.clone())).filter(|_| preview.as_ref().is_none_or(|p| p.count <= 1 && !p.all));
    let code = code.map(|code| {
        const LINES: usize = 8;
        let mut shown: Vec<&str> = code.lines().take(LINES).collect();
        if code.lines().count() > LINES {
            shown.push("…");
        }
        shown.join("\n")
    });

    // (label, key, option, runs from now on without asking)
    let mut buttons: Vec<(String, &'static str, PermissionOption, bool)> = Vec::new();
    if tool == "allow_execution" {
        // The same question as the notebook's safe-preview callout: once, not "always".
        if let Some(deny) = option_of_kind(options, PermissionOptionKind::RejectOnce) {
            buttons.push(("Not now".into(), "esc", deny.clone(), false));
        }
        if let Some(allow) = option_of_kind(options, PermissionOptionKind::AllowOnce) {
            buttons.push(("Run notebook".into(), "⏎", allow.clone(), false));
        }
    } else if *runs_code {
        if let Some(deny) = option_of_kind(options, PermissionOptionKind::RejectOnce) {
            buttons.push(("Deny".into(), "esc", deny.clone(), false));
        }
        if let Some(allow) = option_of_kind(options, PermissionOptionKind::AllowOnce) {
            buttons.push(("Always this session".into(), "⌘⏎", allow.clone(), true));
            buttons.push((run_word.into(), "⏎", allow.clone(), false));
        }
    }
    if buttons.is_empty() {
        buttons = option_buttons(options);
    }
    let primary = buttons.iter().rposition(|(_, _, o, _)| matches!(o.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways));
    let buttons = buttons
        .into_iter()
        .enumerate()
        .map(|(i, (label, hint, option, stop))| {
            let weight = match option.kind {
                _ if Some(i) == primary => Weight::Primary,
                PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways => Weight::Quiet,
                _ => Weight::Outlined,
            };
            CardButton { label, hint, weight, option, stop }
        })
        .collect();
    Some(ApprovalView { heading, code, lines, plan: None, buttons })
}

pub fn render_approval(session: &Session, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ix = session.pending_permission()?;
    let view = approval_view(session)?;
    let key = session.key;
    let buttons = view
        .buttons
        .into_iter()
        .enumerate()
        .map(|(i, CardButton { label, hint, weight, option, stop })| {
            let focus = session.approval_focus(i, cx);
            let button = approval_button(ElementId::NamedInteger("perm".into(), (key << 32) | (ix as u64 * 16 + i as u64)), &label, hint, weight, &focus)
                .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.answer(ix, &option, stop))))
                .into_any_element();
            (weight, button)
        })
        .collect();
    if let Some(plan) = view.plan {
        let shown = match (&plan.steps, plan.open) {
            (Some(steps), false) => steps.clone(),
            _ => plan.full.clone(),
        };
        let title = plan.title.map(|title| div().text_size(theme::size_body()).text_color(theme::text_secondary()).child(title).into_any_element());
        let text = div()
            .id(ElementId::NamedInteger("plan".into(), key))
            .max_h(px(260.))
            .overflow_y_scroll()
            .text_size(theme::size_body())
            .text_color(theme::text_row_active())
            .child(TextView::markdown(ElementId::NamedInteger("plan-text".into(), key), shown).style(plan_style()));
        let toggle = plan.steps.is_some().then(|| {
            let label = if plan.open { "Show only the steps" } else { "Show the whole plan" };
            div()
                .id(ElementId::NamedInteger("plan-toggle".into(), key))
                .role(Role::Button)
                .aria_label(label)
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .text_color(theme::text_faint())
                .hover(|s| s.text_color(theme::text_secondary()))
                .track_focus(&session.plan_card_focus(cx))
                .tab_stop(true)
                .focus_visible(|s| s.border_2().border_color(theme::focus_ring()))
                .child(label)
                .child(if plan.open { "⌄" } else { "›" })
                .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.plan_open = !s.plan_open)))
                .into_any_element()
        });
        let body = title.into_iter().chain([text.into_any_element()]).chain(toggle).collect();
        return Some(approval_card(card_heading(&view.heading), body, buttons));
    }
    let code = view.code.map(|code| {
        div().font_family(theme::MONO).text_size(theme::size_code()).p(px(6.)).rounded(px(4.)).bg(theme::bg_page()).text_color(theme::text_secondary()).child(code).into_any_element()
    });
    let lines = view.lines.into_iter().map(|(text, tone)| {
        let color = match tone {
            Tone::Muted => theme::text_muted(),
            Tone::Secondary => theme::text_secondary(),
        };
        inline_code(&text, theme::size_meta_small()).text_color(color).into_any_element()
    });
    Some(approval_card(card_heading(&view.heading), code.into_iter().chain(lines).collect(), buttons))
}

/// The agent's own options as buttons: declining first, allowing once last
/// (the primary one), with the keys that answer them (⏎ allows once, Esc declines).
fn option_buttons(options: &[PermissionOption]) -> Vec<(String, &'static str, PermissionOption, bool)> {
    let rank = |o: &PermissionOption| match o.kind {
        PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways => 0,
        PermissionOptionKind::AllowOnce => 2,
        _ => 1,
    };
    let mut sorted: Vec<&PermissionOption> = options.iter().collect();
    sorted.sort_by_key(|o| rank(o));
    let keyed = |kind: PermissionOptionKind, hint: &'static str, o: &PermissionOption| {
        option_of_kind(options, kind).is_some_and(|k| k.option_id == o.option_id).then_some(hint)
    };
    sorted
        .into_iter()
        .map(|o| {
            let hint = keyed(PermissionOptionKind::AllowOnce, "⏎", o).or_else(|| keyed(PermissionOptionKind::RejectOnce, "esc", o)).unwrap_or("");
            (o.name.clone(), hint, o.clone(), false)
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
    markdown_style().heading_font_size(|_, _| theme::size_body())
}

/// Plan mode's end: Keep planning · Start in Auto · **Start**.
fn plan_buttons(options: &[PermissionOption]) -> Vec<CardButton> {
    // (label, key, option, runs without asking, weight)
    let buttons: Vec<(&str, &'static str, Option<&PermissionOption>, bool, Weight)> = vec![
        ("Keep planning", "esc", option_of_kind(options, PermissionOptionKind::RejectOnce), false, Weight::Quiet),
        ("Start in Auto", "⌘⏎", plan_option(options), true, Weight::Outlined),
        ("Start", "⏎", plan_option(options), false, Weight::Primary),
    ];
    buttons
        .into_iter()
        .filter_map(|(label, hint, option, stop, weight)| Some(CardButton { label: label.into(), hint, weight, option: option?.clone(), stop }))
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
    let first_line = |code: &str| cut_line(code, 60);
    let new_code = input["code"].as_str().filter(|_| matches!(tool, "edit_cell" | "add_cell"));
    let names: Vec<String> = match (preview, new_code) {
        (_, Some(code)) => vec![defined_name(code).unwrap_or_else(|| first_line(code))],
        (Some(p), None) => p.cells.iter().map(|c| c.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| first_line(&c.code))).collect(),
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
        .shadow(vec![BoxShadow {
            color: Hsla::from(theme::accent()).opacity(0.10),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(3.),
            inset: false,
        }])
        .child(div().px(px(10.)).pt(px(10.)).pb(px(6.)).child(heading))
        .when(!body.is_empty(), |d| d.child(div().flex().flex_col().gap(px(6.)).px(px(10.)).pb(px(10.)).text_size(theme::size_meta()).children(body)))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(6.))
                .px(px(10.))
                .py(px(8.))
                .border_t_1()
                .border_color(theme::accent().opacity(0.25))
                .children(left.into_iter().map(|(_, b)| b.into_any_element()))
                // The agent's own labels can be long: these wrap among themselves.
                .child(div().flex_1().min_w_0().flex().flex_wrap().justify_end().gap(px(6.)).children(right.into_iter().map(|(_, b)| b))),
        )
        .into_any_element()
}

/// A card's heading: body size, medium, with `backticked` names in mono.
fn card_heading(text: &str) -> AnyElement {
    inline_code(text, theme::size_code()).text_size(theme::size_body()).font_weight(FontWeight::MEDIUM).text_color(theme::text_primary()).into_any_element()
}

fn approval_button(id: ElementId, label: &str, hint: &str, weight: Weight, focus: &FocusHandle) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(30.))
        .px(px(if weight == Weight::Primary { 14. } else { 10. }))
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(theme::size_meta())
        .border_2()
        .border_color(gpui::transparent_black())
        .track_focus(focus)
        .tab_stop(true)
        .focus_visible(|s| s.border_color(theme::focus_ring()))
        .map(|d| match weight {
            Weight::Primary => d.bg(theme::accent()).text_color(gpui::white()).font_weight(FontWeight::SEMIBOLD),
            // Its outline is the reserved focus border, so focus only recolours it.
            Weight::Outlined => d.border_color(theme::composer_edge()).text_color(theme::text_row_active()).hover(|s| s.bg(theme::bg_raised())),
            Weight::Quiet => d.text_color(theme::text_secondary()).hover(|s| s.bg(theme::bg_raised())),
        })
        .child(label.to_string())
        .when(!hint.is_empty(), |d| d.child(div().text_size(theme::size_meta_small()).font_weight(FontWeight::NORMAL).opacity(0.6).child(hint.to_string())))
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
                    .text_size(theme::size_meta())
                    .text_color(theme::text_muted())
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .track_focus(&session.pinned_plan_focus(cx))
                    .tab_stop(true)
                    .focus_visible(|s| s.border_color(theme::focus_ring()))
                    .child(progress(entries))
                    .child(if folded { "›" } else { "⌄" })
                    .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.plan_folded = !s.plan_folded))),
            )
            .when(!folded, |d| d.children(plan_rows(entries)))
            .into_any_element(),
    )
}

/// Messages waiting for Claude: click ✎ to pull one back into the input, ✕ to drop it.
pub fn render_queue(this: &Workspace, session: &Session, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let muted = theme::text_muted();
    let key = session.key;
    div().flex().flex_col().gap_1().children(this.render_queue_heading(session)).children(session.outbox.items.iter().enumerate().map(|(i, q)| {
        let id = |name: &'static str| ElementId::NamedInteger(name.into(), (key << 32) | i as u64);
        div()
            .flex()
            .gap_2()
            .text_color(muted)
            .items_center()
            .h(px(30.))
            .px(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::border())
            .child(this.render_queued(i, &q.attachments, &q.text))
            .when(q.in_flight(), |d| d.child("sending now…"))
            .when(q.is_copying(), |d| d.child(div().flex_shrink_0().child("copying files…")))
            .when(!q.in_flight(), |d| {
                d.child(div().id(id("edit")).role(Role::Button).aria_label("Edit").cursor_pointer().hover(|s| s.text_color(theme::text_primary())).child("✎").on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(q) = this.session_mut(key).and_then(|s| s.outbox.take(i)) {
                        this.restore_composer(q.text, q.attachments, window, cx);
                    }
                })))
            })
            .when(!q.in_flight(), |d| {
                d.child(div().id(id("drop")).role(Role::Button).aria_label("Remove").cursor_pointer().hover(|s| s.text_color(theme::text_primary())).child("✕").on_click(cx.listener(move |this, _, _, cx| {
                    this.with_session(key, cx, |s| {
                        s.outbox.take(i);
                    })
                })))
            })
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_agents_own_options_put_allowing_last_and_declining_first() {
        use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind};
        // The adapter's order for a tool it has no special wording for.
        let options = [
            PermissionOption::new("allow-once", "Yes", PermissionOptionKind::AllowOnce),
            PermissionOption::new("allow-with-updates", "Yes, and don't ask again", PermissionOptionKind::AllowAlways),
            PermissionOption::new("reject", "No", PermissionOptionKind::RejectOnce),
        ];
        let buttons: Vec<(String, &str)> = super::option_buttons(&options).into_iter().map(|(label, hint, _, _)| (label, hint)).collect();
        assert_eq!(buttons, vec![("No".into(), "esc"), ("Yes, and don't ask again".into(), ""), ("Yes".into(), "⏎")]);
    }

    #[test]
    fn run_cards_name_cells_that_have_no_name_yet() {
        use crate::pluto::{PreviewCell, RunPreview};
        use serde_json::json;
        let one = |name: Option<&str>, code: &str| RunPreview {
            count: 1,
            cells: vec![PreviewCell { name: name.map(str::to_owned), code: code.into() }],
            ..Default::default()
        };
        let heading = |tool, p: &RunPreview, input| super::run_question(tool, Some(p), &input).heading;
        // A new notebook's empty first cell, edited and run in one call.
        assert_eq!(heading("edit_cell", &one(None, ""), json!({ "code": "x = 1\ny = 2", "run_after": true })), "Edit `x` and run it?");
        assert_eq!(heading("execute_cell", &one(None, ""), json!({})), "Run a cell?");
        assert_eq!(heading("execute_cell", &one(Some(""), "  \n"), json!({})), "Run a cell?");
        assert_eq!(heading("delete_cell", &one(None, ""), json!({})), "Delete a cell?");
        assert_eq!(heading("execute_cell", &one(Some("fit, model"), "fit = 1"), json!({})), "Run `fit, model`?");
        assert_eq!(heading("execute_cell", &one(None, "md\"# Intro\""), json!({})), "Run `md\"# Intro\"`?");
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
        let cell = |code: &str| PreviewCell { name: None, code: code.into() };
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
