//! The transcript: messages, tool-call rows and runs, and the working line.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use agent_client_protocol::schema::v1::{ToolCallStatus, ToolKind};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::text::{TextView, TextViewStyle};
use gpui_component::tooltip::Tooltip;

use crate::Workspace;
use crate::celldiff;
use crate::details;
use crate::outbox::Delivery;
use crate::pluto;
use crate::runs;
use crate::session::{Approval, Entry, Session, defined_name, file_name, file_path};
use crate::theme;

/// A run-state warning as the transcript shows it.
pub(crate) fn run_state_line(warning: &pluto::RunWarning) -> String {
    format!("⚠ {warning}")
}

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
    use super::{RowState, run_summary, tool_row};
    use crate::hosts::Place;
    use crate::session::{Entry, Session};

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
}
