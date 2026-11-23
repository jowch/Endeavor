use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use agent_client_protocol::schema::v1::{ToolCallStatus, ToolKind};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::text::{MarkdownNode, MarkdownParseContext, MarkdownPlugin, TextView, TextViewState, TextViewStyle, markdown_ast};
use gpui_component::tooltip::Tooltip;

use crate::Workspace;
use crate::celldiff;
use crate::details;
use crate::diagram;
use crate::failure;
use crate::failure::hand_on;
use crate::motion;
use crate::new_session::Glyph;
use crate::outbox::Delivery;
use crate::pluto;
use crate::runs;
use crate::session::{Approval, Entry, Failed, FailedKind, Session, cell_label, file_name, file_path};
use crate::signin::Look;
use crate::theme;
use crate::theme::FocusRing as _;
use crate::theme::TextButton as _;

pub(crate) fn run_state_line(warning: &pluto::RunWarning) -> String {
    format!("⚠ {warning}")
}

const LIST_TOP: Pixels = px(12.);

pub fn render_transcript(session: &Session, margin: Pixels, window: &Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    session.sync_list();
    if motion::quiet() {
        session.settle_arrivals();
    }
    motion::follow(&session.glide, &session.list, LIST_TOP, window, cx);
    for (ix, entry) in session.entries.iter().enumerate() {
        if let Entry::Agent { text, .. } = entry {
            session.reply_text(ix, text, cx);
        }
    }
    let key = session.key;
    let workspace = cx.entity().downgrade();
    list(session.list.clone(), move |ix, window, cx| {
        workspace
            .update(cx, |this, cx| {
                let Some(session) = this.sessions.iter().find(|s| s.key == key) else { return div().into_any_element() };
                let Some(entry) = session.entries.get(ix).filter(|_| session.pinned_plan() != Some(ix)) else {
                    return div().into_any_element();
                };
                let element = match runs::run_at(&session.entries, ix) {
                    Some(run) if run.start == ix => render_run(session, run, window, cx),
                    Some(_) => None,
                    None => render_entry(this, session, ix, entry, window, cx),
                };
                let message = matches!(entry, Entry::User { .. } | Entry::Agent { .. });
                let fade = session.arrival(ix).map(|at| motion::eased(at, theme::motion_fast(), window));
                match element {
                    Some(element) => div()
                        .px(margin)
                        .when(!message, |d| d.pb_4())
                        .when(message, |d| d.pb(px(2.)))
                        .when_some(fade, |d, p| d.opacity(p))
                        .child(element)
                        .into_any_element(),
                    None => div().into_any_element(),
                }
            })
            .unwrap_or_else(|_| div().into_any_element())
    })
    .flex_1()
    .pt(LIST_TOP)
}

pub fn render_jump(session: &Session, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let at_end = session.list.is_scrolled_to_end();
    if session.below.get().is_some() && crate::transcript_copy::clears_below(session.reached_end.take(), session.list.is_following_tail(), at_end) {
        session.below.set(None);
        session.list.set_follow_mode(FollowMode::Tail);
    }
    let label = crate::transcript_copy::pill(session.following(), at_end, session.below.get())?;
    let key = session.key;
    Some(
        div()
            .absolute()
            .bottom(px(8.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id(ElementId::NamedInteger("jump-to-latest".into(), key))
                    .role(Role::Button)
                    .px(px(12.))
                    .py(px(4.))
                    .rounded_full()
                    .border_1()
                    .border_color(theme::popover_edge())
                    .bg(theme::popover_bg())
                    .shadow(theme::popover_shadow())
                    .cursor_pointer()
                    .text_size(theme::chat_meta())
                    .text_color(theme::text_primary())
                    .hover(|s| s.bg(theme::button_hover()))
                    .aria_label(label.trim_end_matches(" ↓").to_owned())
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(session) = this.sessions.iter().find(|s| s.key == key) {
                            session.below.set(None);
                            session.list.set_follow_mode(FollowMode::Tail);
                        }
                        cx.notify();
                    })),
            )
            .into_any_element(),
    )
}

pub fn render_activity(session: &Session, offline_since: Option<Instant>, margin: Pixels, cx: &App) -> Option<impl IntoElement + use<>> {
    let activity = activity(session, offline_since)?;
    let id = ElementId::NamedInteger("orbit".into(), session.key);
    let mark = if activity.waiting { crate::orbit::orbit_with(id, ORBIT, theme::text_muted(), cx) } else { crate::orbit::orbit(id, ORBIT, cx) };
    Some(
        div()
            .px(margin)
            .pb_2()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(theme::chat_meta())
            .text_color(theme::text_muted())
            .child(div().mr(px(4.)).child(mark))
            .child(activity.verb)
            .children(activity.object.map(|o| div().font_family(theme::MONO).text_size(theme::chat_meta_small()).text_color(theme::text_secondary()).child(o)))
            .children(activity.took)
            .into_any_element(),
    )
}

pub(crate) struct Activity {
    pub waiting: bool,
    pub verb: String,
    pub object: Option<String>,
    pub took: Option<String>,
}

pub(crate) fn activity(session: &Session, offline_since: Option<Instant>) -> Option<Activity> {
    let since = session.busy_since?;
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

pub(crate) fn elapsed(secs: u64) -> String {
    if secs < 60 { format!("{secs}s") } else { format!("{}m {:02}s", secs / 60, secs % 60) }
}

const ORBIT: f32 = 14.;

const FOLD_AFTER: usize = 12;
const FOLD_TO: usize = 10;
const USER_BUBBLE_PAD_X: f32 = 12.;

fn bubble_width(chat_width: f32) -> f32 {
    ((chat_width - 2. * crate::chat_margin(chat_width)).min(crate::CHAT_CONTENT_MAX) * 0.85).max(200.)
}

fn bubble_lines(text: &SharedString, width: f32, window: &Window) -> usize {
    let style = TextStyle { font_family: theme::SANS.into(), ..window.text_style() };
    let wrap = px(width - 2. * USER_BUBBLE_PAD_X);
    window
        .text_system()
        .shape_text(text.clone(), theme::chat_body(), &[style.to_run(text.len())], Some(wrap), None)
        .map_or(0, |lines| lines.iter().map(|l| l.wrap_boundaries().len() + 1).sum())
}

fn sent_command(session: &Session, text: &str) -> Option<usize> {
    let name = text.strip_prefix('/')?.split(char::is_whitespace).next()?;
    (session.commands.iter().any(|c| c.name == name) || crate::slash::hidden(name)).then(|| name.len() + 1)
}

fn user_text(session: &Session, text: &SharedString) -> AnyElement {
    let Some(len) = sent_command(session, text) else { return text.clone().into_any_element() };
    let color: Hsla = theme::text_primary().into();
    let run = |len, font_family: &'static str, background_color| TextRun { len, font: font(font_family), color, background_color, underline: None, strikethrough: None };
    let runs = vec![run(len, theme::MONO, Some(theme::accent().opacity(0.28).into())), run(text.len() - len, theme::SANS, None)];
    StyledText::new(text.clone()).with_runs(runs.into_iter().filter(|r| r.len > 0).collect()).into_any_element()
}

fn render_entry(this: &Workspace, session: &Session, ix: usize, entry: &Entry, window: &mut Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let key = session.key;
    let muted = theme::text_muted();
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), key << 32 | ix as u64);
    Some(match entry {
        Entry::User { text, expanded, attachments, delivery, sent } => {
            let chips = this.render_sent_chips(key, ix, attachments, cx);
            let column = div().group(MESSAGE).flex().flex_col().items_end().gap(px(4.)).children(chips);
            let delivered = delivery_note(*delivery, session.agent).map(|note| div().text_size(theme::chat_meta()).text_color(muted).child(note));
            let actions = message_actions(session, ix, text.to_string(), *sent, cx);
            let mut quotes = this.render_sent_quotes(key, ix, attachments, cx);
            if text.is_empty() && quotes.is_empty() {
                return Some(column.children(delivered).child(actions).into_any_element());
            }
            if !quotes.is_empty() && !text.is_empty() {
                quotes.push(div().h(px(1.)).my(px(2.)).bg(theme::border()).into_any_element());
            }
            let bubble = div()
                .max_w(px(bubble_width(this.settings.layout.chat_width)))
                .px(px(USER_BUBBLE_PAD_X))
                .py_2()
                .rounded(px(8.))
                .bg(theme::bg_raised())
                .font_family(theme::SANS)
                .text_size(theme::chat_body())
                .line_height(theme::chat_line_body())
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(quotes);
            let unanswered = (session.unanswered == Some(ix)).then(|| this.render_unanswered(session.agent));
            let line_height = theme::chat_line_body();
            if text.is_empty() || bubble_lines(text, bubble_width(this.settings.layout.chat_width), window) <= FOLD_AFTER {
                let bubble = bubble.when(!text.is_empty(), |d| d.child(user_text(session, text)));
                return Some(column.child(bubble).children(delivered).children(unanswered).child(actions).into_any_element());
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
                .child(user_text(session, text))
                .when(!*expanded, |d| d.max_h(line_height * FOLD_TO as f32).overflow_hidden().child(fade));
            column
                .child(bubble.child(body))
                .child(
                    div()
                        .id(id("user-fold"))
                        .role(Role::Button)
                        .cursor_pointer()
                        .text_size(theme::chat_meta())
                        .text_color(muted)
                        .hover(|s| s.text_color(theme::text_primary()))
                        .button_text(if *expanded { "Show less" } else { "Show more" })
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
            .text_size(theme::chat_body())
            .line_height(theme::chat_line_body())
            .child(markdown(&session.reply_text(ix, text, cx), this.settings.layout.chat_width, cx))
            .child(message_actions(session, ix, text.clone(), *at, cx))
            .into_any_element(),
        Entry::Note(text) if session.restart_offered_at(ix) => {
            let host = session.place.host.clone();
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(4.))
                .text_size(theme::chat_meta())
                .text_color(muted)
                .child(text.clone())
                .child(
                    div()
                        .id(id("older-restart"))
                        .role(Role::Button)
                        .cursor_pointer()
                        .text_color(theme::text_secondary())
                        .hover(|s| s.text_color(theme::text_primary()))
                        .button_text("Restart…")
                        .on_click(cx.listener(move |this, _, window, cx| this.confirm_restart(&host, window, cx))),
                )
                .into_any_element()
        }
        Entry::Note(text) => div().text_size(theme::chat_meta()).text_color(muted).child(text.clone()).into_any_element(),
        Entry::Reopened(at) => {
            let rule = || div().flex_1().h(px(1.)).bg(theme::border());
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(theme::chat_meta())
                .text_color(theme::text_faint())
                .child(rule())
                .child(format!("Reopened {}", crate::when::day_at(*at)))
                .child(rule())
                .into_any_element()
        }
        Entry::RunState(warnings) if warnings.is_empty() => return None,
        Entry::RunState(warnings) => div()
            .flex()
            .flex_col()
            .gap_1()
            .text_size(theme::chat_meta())
            .text_color(muted)
            .children(warnings.iter().map(run_state_line))
            .into_any_element(),
        Entry::Tool { .. } | Entry::Thought { .. } => render_row(session, ix, false, window, cx),
        Entry::Changes(cells) => render_changes(session, key, ix, cells, cx),
        Entry::Plan(entries) => div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_size(theme::chat_meta()).text_color(muted).child(crate::approval::progress(entries)))
            .children(crate::approval::plan_rows(entries))
            .into_any_element(),
        Entry::Permission { .. } => return None,
        Entry::Failed(failed) => return render_failed(key, ix, failed, session.agent, cx),
    })
}

fn render_failed(key: u64, ix: usize, failed: &Failed, agent: crate::agent::Agent, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), key << 32 | ix as u64);
    let run = move |this: &mut Workspace, cx: &mut Context<Workspace>| {
        let Some(session) = this.session_mut(key) else { return };
        let effects = match &session.entries.get(ix) {
            Some(Entry::Failed(Failed { kind: FailedKind::NoAnswer { .. }, .. })) => session.try_again(ix),
            _ => session.continue_reply(ix),
        };
        this.apply_effects(key, effects, cx);
    };
    let button = |look| {
        let icon = if matches!(failed.kind, FailedKind::NoAnswer { .. }) { Glyph::Restart } else { Glyph::Play };
        failure::action(id("failed-action"), Some(icon), failed.kind.action(), look).on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
    };
    match &failed.kind {
        FailedKind::NoAnswer { .. } if failed.used => None,
        FailedKind::CutOff => Some(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(8.))
                .child(div().text_size(theme::chat_meta()).text_color(theme::text_faint()).child(failed.kind.title(agent)))
                .when(!failed.used, |d| d.child(button(Look::Secondary)))
                .into_any_element(),
        ),
        kind => {
            let details = failed.raw.clone().map(|raw| {
                let entity = cx.entity().downgrade();
                failure::details(id("failed-details"), raw, failed.details_open, move |_, cx| {
                    let _ = entity.update(cx, |this, cx| this.with_session(key, cx, |s| s.toggle_details(ix)));
                })
            });
            let buttons = (!failed.used).then(|| button(Look::Primary).into_any_element()).into_iter().collect();
            Some(failure::card(kind.title(agent), kind.body(), buttons, details).into_any_element())
        }
    }
}

pub(crate) fn delivery_note(delivery: Delivery, agent: crate::agent::Agent) -> Option<&'static str> {
    match delivery {
        Delivery::Turn => None,
        Delivery::Joined => Some(crate::agent_text!(agent, "", " got this while working")),
        Delivery::AfterStop => Some(crate::agent_text!(agent, "Stopped ", "'s work to send this")),
    }
}

const MESSAGE: &str = "message";

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
            .focus_visible(|s| s.opacity(1.).shadow(theme::ring(theme::bg_page())))
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
            .text_size(theme::chat_meta())
            .text_color(theme::text_faint())
            .tooltip(move |window, cx| Tooltip::new(clock.clone()).build(window, cx))
            .child(crate::when::ago(at))
    });
    div().flex().items_center().gap(px(4.)).h(theme::chat_px(20.)).children(copy).children(time).into_any_element()
}

fn markdown(state: &Entity<TextViewState>, chat_width: f32, cx: &mut Context<Workspace>) -> TextView {
    let workspace = cx.entity().downgrade();
    let link = move |url: &SharedString, event: &ClickEvent, _: &mut Window, cx: &mut App| match crate::annotate::linked_cell(url) {
        _ if !activates(event) => {}
        Some(cell) => {
            let cell = cell.to_owned();
            let _ = workspace.update(cx, |this, cx| this.reveal_cells(vec![cell], cx));
        }
        None => crate::platform::open_url(url, cx),
    };
    let cell_links = CellLinks(cx.entity().downgrade());
    let width = (chat_width - 2. * crate::chat_margin(chat_width)).min(crate::CHAT_CONTENT_MAX) - 20.;
    TextView::new(state)
        .style(markdown_style())
        .inline_code_style(inline_code_style())
        .on_link_click(link)
        .plugin(cell_links)
        .code_block_actions(|block, _, _| copy_button("copy".into(), block.code().to_string(), "Copy code"))
        .markdown_block_parser(diagram::parse_block)
        .markdown_block_renderer(diagram::NODE, move |node, _, _| {
            let Some(block) = node.data::<diagram::Block>() else { return div().into_any_element() };
            let Ok(d) = diagram::fitted(&block.source, width) else {
                return div().font_family(theme::MONO).text_size(theme::chat_code()).whitespace_normal().child(block.source.clone()).into_any_element();
            };
            div()
                .relative()
                .w_full()
                .p(px(10.))
                .rounded(px(8.))
                .bg(theme::bg_card())
                .child(
                    img(d.image(theme::is_light()))
                        .w(px(d.width))
                        .max_w_full()
                        .aspect_ratio(d.width / d.height)
                        .aria_label(node.accessibility_name().to_owned()),
                )
                .child(copy_button(("copy-diagram", node.source_range().map_or(0, |r| r.start)).into(), block.source.clone(), "Copy diagram source"))
                .into_any_element()
        })
}

fn copy_button(id: ElementId, text: String, label: &'static str) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Button)
        .px(px(6.))
        .py(px(2.))
        .rounded(px(4.))
        .cursor_pointer()
        .font_family(theme::SANS)
        .text_size(theme::chat_meta_small())
        .text_color(theme::text_faint())
        .absolute()
        .top_0()
        .right_0()
        .whitespace_nowrap()
        .bg(theme::bg_card())
        .opacity(0.)
        .group_hover(MESSAGE, |s| s.opacity(1.))
        .hover(|s| s.text_color(theme::text_primary()).bg(theme::bg_raised()))
        .button_text("Copy")
        .aria_label(label)
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))
}

const CELL_LINK: &str = "cell-link";

struct CellLink {
    cell: String,
    words: SharedString,
}

struct CellLinks(WeakEntity<Workspace>);

impl MarkdownPlugin for CellLinks {
    fn name(&self) -> &str {
        CELL_LINK
    }

    fn parse(&self, node: &markdown_ast::Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let markdown_ast::Node::Link(link) = node else { return None };
        let cell = crate::annotate::linked_cell(&link.url)?.to_owned();
        let words: SharedString = node.to_string().into();
        Some(MarkdownNode::new(CELL_LINK, CellLink { cell, words: words.clone() }).text(words.clone()).accessibility_label(words))
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(link) = node.data::<CellLink>() else { return div().into_any_element() };
        let gone = self.0.upgrade().is_some_and(|w| cell_here(w.read(cx), &link.cell) == Some(false));
        let color = if gone { theme::text_faint() } else { theme::accent_text() };
        let pill = div()
            .id(SharedString::from(format!("cell-link-{}-{}", link.cell, node.source_range().map_or(0, |r| r.start))))
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .rounded(px(5.))
            .bg(if gone { theme::bg_tag() } else { theme::lit_bg() })
            .text_color(color)
            .whitespace_nowrap()
            .child(crate::new_session::glyph(crate::new_session::Glyph::Code, color))
            .child(div().when(gone, |d| d.line_through()).child(link.words.clone()));
        if gone {
            return pill.tooltip(|window, cx| Tooltip::new("This cell isn't in this notebook").build(window, cx)).into_any_element();
        }
        let focus = window.use_keyed_state(SharedString::from(format!("cell-link-focus-{}-{}", link.cell, node.source_range().map_or(0, |r| r.start))), cx, |_, cx| cx.focus_handle().tab_stop(true)).read(cx).clone();
        let workspace = self.0.clone();
        let cell = link.cell.clone();
        pill.role(Role::Link)
            .track_focus(&focus)
            .tab_stop(true)
            .focus_ring()
            .cursor_pointer()
            .hover(|s| s.bg(theme::mark_bg()))
            .on_click(move |_, _, cx| {
                let _ = workspace.update(cx, |this, cx| this.reveal_cells(vec![cell.clone()], cx));
            })
            .into_any_element()
    }
}

fn cell_here(workspace: &Workspace, cell: &str) -> Option<bool> {
    let notebook = workspace.active_session()?.notebook.as_deref()?;
    let cells = workspace.connections.values().find_map(|c| c.cells.get(notebook)?.as_array())?;
    Some(cells.iter().any(|c| c["cell_id"] == cell))
}

fn activates(event: &ClickEvent) -> bool {
    match event {
        ClickEvent::Mouse(click) => matches!(click.up.button, MouseButton::Left | MouseButton::Middle),
        ClickEvent::Keyboard(_) => true,
        ClickEvent::Touch(click) => !click.long_press,
    }
}

pub(crate) fn markdown_style() -> TextViewStyle {
    let code_block = StyleRefinement::default()
        .p(px(10.))
        .rounded(px(8.))
        .bg(theme::bg_card())
        .font_family(theme::MONO)
        .text_size(theme::chat_code());
    let table = StyleRefinement::default().rounded(px(8.));
    let table_head = StyleRefinement::default().text_size(theme::chat_meta()).text_color(theme::text_muted());
    let table_cell = StyleRefinement::default().px(px(8.)).py(px(3.));
    TextViewStyle::default()
        .paragraph_gap(rems(0.75))
        .heading_font_size(|level, _| if level <= 2 { theme::chat_subhead() } else { theme::chat_body() })
        .code_block(code_block)
        .table(table)
        .table_head(table_head)
        .table_cell(table_cell)
        .inline_code(HighlightStyle { background_color: Some(theme::bg_tag().into()), ..Default::default() })
}

pub(crate) fn inline_code_style() -> StyleRefinement {
    StyleRefinement::default().px(px(2.))
}

const DETAIL_MAX_H: f32 = 160.;
const DETAIL_LINE: f32 = 16.;

fn render_run(session: &Session, run: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let key = session.key;
    let rows: Vec<usize> = run
        .clone()
        .filter(|&i| match &session.entries[i] {
            Entry::Tool { title, status, output, .. } => !runs::hidden(title, *status, output.as_ref()),
            Entry::Thought { .. } => true,
            _ => false,
        })
        .collect();
    match rows[..] {
        [] => return None,
        [only] => return Some(render_row(session, only, true, window, cx)),
        _ => {}
    }
    let calls: Vec<usize> = rows.iter().copied().filter(|&i| matches!(session.entries[i], Entry::Tool { .. })).collect();
    let open = run_open(session, &run);
    let live = session.busy_since.is_some() && run.end == session.entries.len();
    let start = run.start;
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), key << 32 | start as u64);
    let (mut text, counts) = run_summary(session, run.clone());
    let aria_label = format!("{text}, {}", if open { "expanded" } else { "collapsed" });
    text.push_str(if open { " ⌄" } else { " ›" });
    let words = |(text, counts): (String, Vec<std::ops::Range<usize>>)| {
        let highlights: Vec<_> = counts.into_iter().map(|range| (range, HighlightStyle { color: Some(theme::danger().into()), ..Default::default() })).collect();
        StyledText::new(text).with_highlights(highlights)
    };
    let rewritten = motion::changed(id("run-words"), &(text.clone(), counts.clone()), window, cx);
    let header = div()
        .id(id("run"))
        .role(Role::Button)
        .aria_label(aria_label)
        .cursor_pointer()
        .text_size(theme::chat_meta())
        .text_color(theme::text_faint())
        .hover(|s| s.text_color(theme::text_secondary()))
        .border_2()
        .border_color(gpui::transparent_black())
        .track_focus(&session.run_focus(start, cx))
        .tab_stop(true)
        .focus_visible(|s| s.border_color(theme::focus_ring()))
        .relative()
        .child(div().when_some(rewritten.as_ref(), |d, (_, p)| d.opacity(*p)).child(words((text, counts))))
        .children(rewritten.map(|(was, p)| div().absolute().top_0().left_0().right_0().opacity(1. - p).child(words(was))))
        .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.toggle_run(start))));
    let shown: Vec<usize> = match (open, live) {
        (true, _) => rows,
        (false, true) => calls.last().copied().into_iter().collect(),
        (false, false) => Vec::new(),
    };
    let latest = (!open && live).then(|| shown.first().copied()).flatten();
    let replaced = latest.and_then(|ix| motion::changed(id("run-latest"), &ix, window, cx)).filter(|(was, _)| *was < session.entries.len());
    let opened = motion::changed(id("run-open"), &open, window, cx).filter(|(was, _)| !was).map(|(_, p)| p);
    let list = (!shown.is_empty()).then(|| {
        div().flex().flex_col().rounded(px(6.)).border_1().border_color(theme::border()).when_some(opened, |d, p| d.opacity(p)).children(shown.into_iter().enumerate().map(|(n, i)| {
            let row = div().px(px(8.)).py(px(4.)).when(n > 0, |d| d.border_t_1().border_color(theme::border()));
            match replaced {
                Some((was, p)) => row
                    .relative()
                    .child(div().absolute().top(px(4.)).left(px(8.)).right(px(8.)).opacity(1. - p).child(render_row(session, was, true, window, cx)))
                    .child(div().opacity(p).child(render_row(session, i, true, window, cx))),
                None => row.child(render_row(session, i, true, window, cx)),
            }
        }))
    });
    let header = if live { motion::arriving(header, id("run-header"), false).into_any_element() } else { header.into_any_element() };
    Some(div().flex().flex_col().gap(px(6.)).child(header).children(list).into_any_element())
}

fn render_changes(session: &Session, key: u64, ix: usize, cells: &[celldiff::ChangedCell], cx: &mut Context<Workspace>) -> AnyElement {
    use celldiff::CellChange;
    let mono = |text: String, color: Rgba| div().flex_none().font_family(theme::MONO).text_size(theme::chat_meta_small()).text_color(color).child(text);
    let tag = |text: &'static str| div().flex_none().px(px(5.)).rounded(px(3.)).bg(theme::bg_tag()).text_color(theme::text_tag()).text_size(theme::size_meta_small()).child(text);
    let rows = cells.iter().enumerate().map(|(n, cell)| {
        let deleted = cell.change == CellChange::Deleted;
        let tag_text = match cell.change {
            CellChange::New => ", new",
            CellChange::Edited => "",
            CellChange::Deleted => ", deleted",
        };
        let label = if deleted { format!("{}{tag_text}, {} added, {} removed", cell.name, cell.added, cell.removed) } else { format!("Show {} in the notebook", cell.name) };
        let id = cell.cell.clone();
        div()
            .id(ElementId::NamedInteger(format!("changed-cell-{n}").into(), key << 32 | ix as u64))
            .aria_label(label)
            .flex()
            .items_center()
            .gap(px(6.))
            .h(theme::chat_px(28.))
            .px(px(8.))
            .when(n > 0, |d| d.border_t_1().border_color(theme::border()))
            .text_size(theme::chat_meta())
            .text_color(theme::text_faint())
            .child(crate::new_session::glyph(crate::new_session::Glyph::Code, theme::text_faint()))
            .child(mono(cell.name.clone(), if deleted { theme::text_faint() } else { theme::text_secondary() }).min_w_0().truncate())
            .when(cell.change == CellChange::New, |d| d.child(tag("new")))
            .when(deleted, |d| d.child(tag("deleted")))
            .child(div().flex_1())
            .when(cell.added > 0, |d| d.child(mono(format!("+{}", cell.added), theme::diff_add())))
            .when(cell.removed > 0, |d| d.child(mono(format!("−{}", cell.removed), theme::diff_del())))
            .when(!deleted, |d| {
                d.role(Role::Button)
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .track_focus(&session.changed_cell_focus(ix, n, cx))
                    .tab_stop(true)
                    .focus_visible(|s| s.border_color(theme::focus_ring()))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::row_active()).text_color(theme::text_secondary()))
                    .child(div().flex_none().child("›"))
                    .on_click(cx.listener(move |this, _, _, cx| this.reveal_cells(vec![id.clone()], cx)))
            })
    });
    let card = div().flex().flex_col().rounded(px(6.)).border_1().border_color(theme::border()).overflow_hidden().children(rows);
    let Some((line, pressable)) = session.restore_footer(ix) else {
        return card.into_any_element();
    };
    let footer = div()
        .flex()
        .items_center()
        .gap(theme::chat_px(8.))
        .min_h(theme::chat_px(34.))
        .px(theme::chat_px(8.))
        .py(theme::chat_px(4.))
        .border_t_1()
        .border_color(theme::border())
        .text_size(theme::chat_meta())
        .text_color(theme::text_faint())
        .child(div().flex_1().min_w_0().child(line))
        .when(pressable, |d| {
            d.child(
                crate::signin::button(ElementId::NamedInteger("restore-turn".into(), key << 32 | ix as u64), "Restore", crate::signin::Look::Secondary)
                    .h(theme::chat_px(26.))
                    .px(theme::chat_px(10.))
                    .rounded(theme::chat_px(6.))
                    .text_size(theme::chat_meta())
                    .track_focus(&session.changed_cell_focus(ix, cells.len(), cx))
                    .tab_stop(true)
                    .focus_visible(|s| s.border_color(theme::focus_ring()))
                    .on_click(cx.listener(move |this, _, _, cx| this.restore_turn(key, ix, cx))),
            )
        });
    card.child(footer).into_any_element()
}

pub(crate) struct ToolRow {
    pub line: ToolLine,
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
    CellError,
}

impl RowState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            RowState::Running => "…",
            RowState::Denied => "denied",
            RowState::Failed => "failed",
            RowState::CellError => "error",
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
    let name = |id: &str| session.cell_codes.get(id).map(cell_label);
    let running = matches!(status, ToolCallStatus::Pending | ToolCallStatus::InProgress);
    let line = if pluto { pluto_line(title, diffs, args, output.as_ref(), running, &name) } else { running_line(tool_line(title, *kind, path.as_deref(), args), title, *kind, args, running) };
    let failed = runs::failed(*status, title, output.as_ref());
    let state = if *approval == Some(Approval::Denied) || runs::denied(output.as_ref()) {
        Some(RowState::Denied)
    } else if failed {
        Some(RowState::Failed)
    } else if running {
        Some(RowState::Running)
    } else if runs::cell_error(title, output.as_ref()) {
        Some(RowState::CellError)
    } else {
        None
    };
    Some(ToolRow { line, file_diff, added, removed, failed, state })
}

pub(crate) fn run_summary(session: &Session, run: std::ops::Range<usize>) -> (String, Vec<std::ops::Range<usize>>) {
    let mut failed = 0;
    let mut denied = 0;
    let mut summed = Vec::new();
    for entry in &session.entries[run] {
        let Entry::Tool { title, kind, status, input, output, approval, .. } = entry else { continue };
        if runs::hidden(title, *status, output.as_ref()) {
            continue;
        }
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

pub(crate) fn run_open(session: &Session, run: &std::ops::Range<usize>) -> bool {
    session.entries[run.clone()].iter().find_map(|e| if let Entry::Tool { id, .. } = e { Some(id) } else { None }).is_some_and(|id| session.open_runs.contains(id))
}

fn render_row(session: &Session, ix: usize, in_run: bool, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let (key, entry) = (session.key, &session.entries[ix]);
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), key << 32 | ix as u64);
    let toggle = cx.listener(move |this: &mut Workspace, _: &ClickEvent, _: &mut Window, cx: &mut Context<Workspace>| this.with_session(key, cx, |s| s.toggle(ix)));
    let expanded_now = matches!(entry, Entry::Thought { expanded: true, .. } | Entry::Tool { expanded: true, .. });
    let opening = motion::changed(id("open"), &expanded_now, window, cx).filter(|(was, _)| !was).map(|(_, p)| p);
    let line = |name: &'static str| {
        div()
            .id(id(name))
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .text_size(theme::chat_meta())
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
                    scroll_y(div().id(id("thought-text")).max_h(theme::chat_px(DETAIL_MAX_H)), window, cx)
                        .when_some(opening, |d, p| d.opacity(p))
                        .line_height(theme::chat_px(DETAIL_LINE))
                        .pl(px(10.))
                        .border_l_1()
                        .border_color(theme::border())
                        .text_size(theme::chat_meta())
                        .text_color(theme::text_muted())
                        .child(text.clone()),
                )
            })
            .into_any_element(),
        Entry::Tool { title, status, output, .. } if runs::hidden(title, *status, output.as_ref()) => div().into_any_element(),
        Entry::Tool { id: call, title, kind, path, input, output, diffs, expanded, approval, .. } => {
            let args = input.as_ref().unwrap_or(&serde_json::Value::Null);
            let answered = (*approval).zip(session.asks.answered_at.get(call)).filter(|_| *expanded).map(|(approval, at)| {
                let secs = at.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                div().text_size(theme::chat_meta()).text_color(theme::text_muted()).child(answered_line(approval, &crate::when::clock(secs)))
            });
            let Some(ToolRow { line: summary, file_diff, added, removed, failed, state }) = tool_row(session, entry) else { return div().into_any_element() };
            let aria_label = format!(
                "{}{}, {}",
                summary.verb,
                summary.object.as_deref().map(|o| format!(" {o}")).unwrap_or_default(),
                if *expanded { "expanded" } else { "collapsed" }
            );
            let all_diffs: Vec<&celldiff::CellDiff> = diffs.iter().chain(&file_diff).collect();
            let name = |id: &str| session.cell_codes.get(id).map(cell_label);
            let mono = |text: String, color: Rgba| div().flex_none().font_family(theme::MONO).text_size(theme::chat_meta_small()).text_color(color).child(text);
            let state = state.map(|state| match state {
                RowState::Running => div().flex_none().child(state.label()),
                RowState::Denied | RowState::Failed | RowState::CellError => div().flex_none().text_color(theme::danger()).child(state.label()),
            });
            let object = summary.object.map(|text| {
                let d = div().id(id("tool-object")).min_w_0().truncate().text_color(theme::text_secondary()).child(text);
                let d = if summary.mono { d.font_family(theme::MONO).text_size(theme::chat_meta_small()) } else { d };
                match summary.full {
                    Some(full) => d.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)),
                    None => d,
                }
            });
            let call_box = (*expanded).then(|| {
                let details = details::details(title, *kind, path.as_deref(), args, output.as_ref(), failed, !all_diffs.is_empty(), &name);
                let item = runs::run_at(&session.entries, ix).map_or(ix, |run| run.start);
                render_call_box(details, &all_diffs, call_key(key, &call.0), (session.list.clone(), item), window, cx)
            }).flatten();
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
                .when(*expanded, |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .when_some(opening, |d, p| d.opacity(p))
                            .children(call_box)
                            .children(answered),
                    )
                })
                .into_any_element()
        }
        _ => div().into_any_element(),
    }
}

pub(crate) fn thought_label(in_run: bool, started: Option<Instant>, took: Option<Duration>) -> String {
    match (in_run, started, took) {
        (true, _, _) | (false, Some(_), None) => "Thinking".into(),
        (false, _, Some(took)) => format!("Thought for {}", elapsed(took.as_secs().max(1))),
        (false, None, None) => "Thought".into(),
    }
}

const DETAIL_LINES: f32 = 12.;
const DETAIL_MAX_SHARE: f32 = 0.4;
const FADE: f32 = 32.;

#[derive(Default)]
struct KeptStates(std::collections::HashMap<ElementId, AnyEntity>);

impl Global for KeptStates {}

fn kept_state<T: 'static>(id: ElementId, cx: &mut App, init: impl FnOnce(&mut App) -> T) -> Entity<T> {
    if let Some(kept) = cx.default_global::<KeptStates>().0.get(&id).and_then(|e| e.clone().downcast::<T>().ok()) {
        return kept;
    }
    let value = init(cx);
    let kept = cx.new(|_| value);
    cx.global_mut::<KeptStates>().0.insert(id, kept.clone().into_any());
    kept
}

fn call_key(key: u64, call: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (key, call).hash(&mut hasher);
    hasher.finish()
}

fn render_call_box(details: details::Details, diffs: &[&celldiff::CellDiff], key: u64, (list, item): (ListState, usize), window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    use details::Part;
    let id = |name: &'static str| ElementId::NamedInteger(name.into(), key);
    let details::Details { input, output, status, cut } = details;
    let (strip, code_in): (Vec<Part>, Vec<Part>) = input.into_iter().partition(|p| !matches!(p, Part::Code(_)));
    let (errors, output): (Vec<Part>, Vec<Part>) = output.into_iter().partition(|p| matches!(p, Part::Error(_)));
    let body: Vec<Part> = code_in.into_iter().chain(output).collect();
    if strip.is_empty() && body.is_empty() && errors.is_empty() && diffs.is_empty() && status.is_none() {
        return None;
    }
    let copy_text = body.iter().chain(&errors).map(details::Part::text).collect::<Vec<_>>().join("\n");
    let shown_text = body.iter().map(details::Part::text).collect::<Vec<_>>().join("\n");
    let show_all = kept_state(id("call-all"), cx, |_| false);
    let all = *show_all.read(cx);
    let line = theme::chat_px(DETAIL_LINE);
    let strip_rows: usize = strip.iter().map(|p| if let Part::Fields(rows) = p { rows.len() } else { 1 }).sum();
    let strip_height = if strip_rows == 0 { px(0.) } else { (line + px(2.)) * strip_rows as f32 + px(13.) };
    let footer_height = if !copy_text.is_empty() || status.is_some() { line + px(9.) } else { px(0.) };
    let room = window.viewport_size().height * DETAIL_MAX_SHARE - strip_height - footer_height;
    let cap = (line * DETAIL_LINES + px(12.)).min(room).max(line * 3. + px(12.));

    let strip = (!strip.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(10.))
            .py(px(6.))
            .bg(theme::bg_card())
            .when(!body.is_empty() || !errors.is_empty() || !diffs.is_empty() || status.is_some(), |d| d.border_b_1().border_color(theme::border()))
            .children(strip.into_iter().enumerate().map(|(n, part)| strip_part(part, ElementId::NamedInteger(format!("call-strip-{n}").into(), key), cx)))
    });

    let diffs_need = (!diffs.is_empty()).then(|| {
        let rows: usize = diffs.iter().map(|d| d.lines.len().min(DIFF_LINES) + 1 + (d.lines.len() > DIFF_LINES) as usize).sum();
        line * rows as f32 + px(6.) * diffs.len() as f32 + px(6.)
    });
    let output_need = (!body.is_empty()).then(|| line * (shown_text.lines().count() + 1) as f32 + px(14.) * body.len() as f32);
    let caps = share_cap(cap.into(), &[diffs_need.map(f32::from), output_need.map(f32::from), (!errors.is_empty()).then_some(f32::INFINITY)]);
    let [diffs_cap, output_cap, errors_cap] = caps.map(px);

    let diffs_handle = kept_state(id("call-diffs-scroll"), cx, |_| ScrollHandle::new()).read(cx).clone();
    let (diffs_offset, diffs_max) = (diffs_handle.offset(), diffs_handle.max_offset());
    let diffs_cut = !all && (diffs_max.y > px(0.5) || diffs.iter().any(|d| d.lines.len() > DIFF_LINES));
    let diffs = (!diffs.is_empty()).then(|| {
        let panel = div()
            .id(id("call-diffs"))
            .when(!all, |d| d.max_h(diffs_cap))
            .flex()
            .flex_col()
            .gap(px(6.))
            .py(px(6.))
            .children(diffs.iter().enumerate().map(|(n, diff)| render_diff(diff, all, ElementId::NamedInteger(format!("call-diff-{n}").into(), key), cx)));
        div()
            .relative()
            .when(!body.is_empty() || !errors.is_empty(), |d| d.border_b_1().border_color(theme::border()))
            .child(hand_on(panel.overflow_y_scroll().track_scroll(&diffs_handle), diffs_handle.clone()))
            .child(remeasure(&diffs_handle, diffs_max))
            .when(!all && diffs_max.y > px(0.5) && diffs_offset.y > -diffs_max.y + px(1.), |d| d.child(fade(FadeEdge::Bottom, theme::bg_sunken())))
    });

    let handle = kept_state(id("call-out-scroll"), cx, |_| ScrollHandle::new()).read(cx).clone();
    let (offset, max) = (handle.offset(), handle.max_offset());
    let cut_below = !all && max.y > px(0.5);
    let more_below = cut_below && offset.y > -max.y + px(1.);
    let more_right = max.x > px(0.5) && offset.x > -max.x + px(1.);
    let lines = shown_text.lines().count() - (cut && shown_text.ends_with("\n…")) as usize;
    let output_area = (!body.is_empty()).then(|| {
        let parts = div().flex().flex_col().items_start().gap(px(4.)).px(px(10.)).py(px(6.)).children(body.into_iter().map(output_part));
        let area = scroll_xy(div().id(id("call-out")).flex().flex_col().items_start().when(!all, |d| d.max_h(output_cap)), handle.clone()).line_height(line).child(parts.flex_none().min_w_full());
        div()
            .relative()
            .child(area)
            .child(remeasure(&handle, max))
            .when(more_below, |d| d.child(fade(FadeEdge::Bottom, theme::bg_sunken())))
            .when(more_right, |d| d.child(fade(FadeEdge::Right, theme::bg_sunken())))
    });

    let errors_handle = kept_state(id("call-errors-scroll"), cx, |_| ScrollHandle::new()).read(cx).clone();
    let (err_offset, err_max) = (errors_handle.offset(), errors_handle.max_offset());
    let errors_cut = !all && !errors.is_empty() && err_max.y > px(0.5);
    let errors = (!errors.is_empty()).then(|| {
        let panel = div()
            .id(id("call-errors"))
            .when(!all, |d| d.max_h(errors_cap))
            .flex()
            .flex_col()
            .gap(px(4.))
            .px(px(10.))
            .py(px(6.))
            .line_height(line)
            .text_color(theme::danger())
            .children(errors.into_iter().map(|e| div().child(e.text())));
        div()
            .relative()
            .when(output_area.is_some(), |d| d.border_t_1().border_color(theme::border()))
            .child(hand_on(panel.overflow_y_scroll().track_scroll(&errors_handle), errors_handle.clone()))
            .child(remeasure(&errors_handle, err_max))
            .when(errors_cut && err_offset.y > -err_max.y + px(1.), |d| d.child(fade(FadeEdge::Bottom, theme::bg_sunken())))
    });

    let shown_all = all || cut_below || errors_cut || diffs_cut;
    let output_long = output_need.is_some_and(|need| need > output_cap);
    let footer = (shown_all || status.is_some() || !copy_text.is_empty()).then(|| {
        let toggle = show_all.clone();
        let (out_handle, err_handle, diff_handle) = (handle.clone(), errors_handle.clone(), diffs_handle.clone());
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(10.))
            .py(px(4.))
            .border_t_1()
            .border_color(theme::border())
            .font_family(theme::SANS)
            .text_size(theme::chat_meta_small())
            .text_color(theme::text_faint())
            .children(status.map(|s| div().flex_none().text_color(theme::danger()).child(s)))
            .when(cut || (lines > 0 && (cut_below || errors_cut || (all && output_long))), |d| {
                let count = match (cut, lines) {
                    (true, n) => format!("first {n} {}", if n == 1 { "line" } else { "lines" }),
                    (false, 1) => "1 line".to_string(),
                    (false, n) => format!("{n} lines"),
                };
                d.child(div().flex_none().child(count))
            })
            .when(shown_all, |d| {
                d.child(
                    div()
                        .id(id("call-show-all"))
                        .role(Role::Button)
                        .flex_none()
                        .cursor_pointer()
                        .text_color(theme::text_secondary())
                        .hover(|s| s.text_color(theme::text_primary()))
                        .child(if all { "Show less" } else { "Show all" })
                        .on_click(move |_, _, cx| toggle.update(cx, |all, cx| {
                            let top = list.logical_scroll_top();
                            if *all && !list.is_following_tail() && top.item_ix == item {
                                let over = |handle: &ScrollHandle, cap: Pixels| (handle.bounds().size.height - cap).max(px(0.));
                                let lost = over(&out_handle, output_cap) + over(&err_handle, errors_cap) + over(&diff_handle, diffs_cap);
                                list.scroll_to(ListOffset { item_ix: item, offset_in_item: (top.offset_in_item - lost).max(px(0.)) });
                            }
                            *all = !*all;
                            cx.notify();
                        })),
                )
            })
            .child(div().flex_1())
            .when(!copy_text.is_empty(), |d| {
                d.child(
                    div()
                        .id(id("call-copy"))
                        .role(Role::Button)
                        .aria_label(if cut { "Copy what's shown" } else { "Copy the output" })
                        .flex_none()
                        .cursor_pointer()
                        .text_color(theme::text_secondary())
                        .hover(|s| s.text_color(theme::text_primary()))
                        .child("Copy")
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))),
                )
            })
    });

    Some(
        div()
            .flex()
            .flex_col()
            .rounded(px(6.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_sunken())
            .overflow_hidden()
            .font_family(theme::MONO)
            .text_size(theme::chat_meta_small())
            .children(strip)
            .children(diffs)
            .children(output_area)
            .children(errors)
            .children(footer)
            .into_any_element(),
    )
}

fn share_cap(cap: f32, needs: &[Option<f32>; 3]) -> [f32; 3] {
    let mut caps = [0.0; 3];
    let mut open: Vec<usize> = (0..3).filter(|&i| needs[i].is_some()).collect();
    let mut left = cap;
    while !open.is_empty() {
        let share = left / open.len() as f32;
        let small: Vec<usize> = open.iter().copied().filter(|&i| needs[i].is_some_and(|n| n <= share)).collect();
        if small.is_empty() {
            for &i in &open {
                caps[i] = share;
            }
            break;
        }
        for &i in &small {
            caps[i] = needs[i].unwrap_or(0.0);
            left -= caps[i];
        }
        open.retain(|i| !small.contains(i));
    }
    caps
}

#[derive(Clone, Copy)]
enum FadeEdge {
    Bottom,
    Right,
}

fn fade(edge: FadeEdge, bg: Rgba) -> Div {
    let d = div().absolute();
    match edge {
        FadeEdge::Bottom => d.left_0().right_0().bottom_0().h(px(FADE)).bg(linear_gradient(180., linear_color_stop(bg.opacity(0.), 0.), linear_color_stop(bg, 1.))),
        FadeEdge::Right => d.top_0().bottom_0().right_0().w(px(FADE)).bg(linear_gradient(90., linear_color_stop(bg.opacity(0.), 0.), linear_color_stop(bg, 1.))),
    }
}

fn strip_part(part: details::Part, id: ElementId, cx: &mut App) -> AnyElement {
    use details::Part;
    let one_line = |content: AnyElement, id: ElementId, cx: &mut App| {
        let handle = kept_state(id.clone(), cx, |_| ScrollHandle::new()).read(cx).clone();
        let max = handle.max_offset();
        let more_right = max.x > px(0.5) && handle.offset().x > -max.x + px(1.);
        div()
            .relative()
            .min_w_0()
            .flex_1()
            .child(scroll_x(div().id(id).flex(), handle.clone()).child(div().flex_none().whitespace_nowrap().child(content)))
            .child(remeasure(&handle, max))
            .when(more_right, |d| d.child(fade(FadeEdge::Right, theme::bg_card())))
    };
    match part {
        Part::Command(command) => div()
            .flex()
            .gap(px(8.))
            .line_height(theme::chat_px(DETAIL_LINE))
            .child(div().flex_none().text_color(theme::text_faint()).child("$"))
            .child(one_line(div().text_color(theme::text_secondary()).child(command.replace('\n', " ⏎ ")).into_any_element(), id, cx))
            .into_any_element(),
        Part::Fields(rows) => div()
            .flex()
            .flex_col()
            .line_height(theme::chat_px(DETAIL_LINE))
            .children(rows.into_iter().enumerate().map(|(n, (label, value))| {
                let value = value.replace('`', "");
                let short = short_path(&value);
                let shown = div().text_color(theme::text_secondary()).child(short.clone().unwrap_or_else(|| value.clone()));
                let shown = match short {
                    Some(_) => shown.id(ElementId::Name(format!("{id}-path-{n}").into())).tooltip(move |window, cx| Tooltip::new(value.clone()).build(window, cx)).into_any_element(),
                    None => shown.into_any_element(),
                };
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_none().min_w(px(44.)).font_family(theme::SANS).text_color(theme::text_faint()).child(label))
                    .child(one_line(shown, ElementId::Name(format!("{id}-{n}").into()), cx))
            }))
            .into_any_element(),
        other => div().font_family(theme::SANS).text_color(theme::text_muted()).child(other.text()).into_any_element(),
    }
}

fn short_path(value: &str) -> Option<String> {
    if !(value.starts_with('/') || value.starts_with('~') || value.contains(":\\")) {
        return None;
    }
    let parts: Vec<&str> = value.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    (parts.len() > 2).then(|| format!("…/{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]))
}

fn output_part(part: details::Part) -> AnyElement {
    use details::Part;
    let mono = |text: String, color: Rgba| div().whitespace_nowrap().text_color(color).child(text);
    match part {
        Part::Code(code) | Part::Command(code) => mono(code, theme::text_secondary()).px(px(8.)).py(px(5.)).rounded(px(4.)).bg(theme::bg_card()).into_any_element(),
        Part::Fields(rows) => div()
            .flex()
            .flex_col()
            .children(rows.into_iter().map(|(label, value)| {
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().flex_none().min_w(px(44.)).font_family(theme::SANS).text_color(theme::text_faint()).child(label))
                    .child(mono(value.replace('`', ""), theme::text_secondary()))
            }))
            .into_any_element(),
        Part::Line(text) => div().flex().whitespace_nowrap().font_family(theme::SANS).text_color(theme::text_muted()).children(text.split('`').enumerate().map(|(i, piece)| {
            let piece = piece.replace(' ', "\u{a0}");
            if i % 2 == 1 { div().font_family(theme::MONO).text_color(theme::text_secondary()).child(piece) } else { div().child(piece) }
        }))
        .into_any_element(),
        Part::Text(text) => mono(text, theme::text_muted()).into_any_element(),
        Part::Error(text) => mono(text, theme::danger()).into_any_element(),
        Part::Numbered(lines) => {
            let numbers: Vec<String> = lines.iter().map(|(n, _)| if *n == 0 { String::new() } else { n.to_string() }).collect();
            let code: Vec<&str> = lines.iter().map(|(_, l)| l.as_str()).collect();
            div()
                .flex()
                .child(mono(numbers.join("\n"), theme::text_section()).flex_none().text_right().pr(px(12.)))
                .child(mono(code.join("\n"), theme::text_muted()))
                .into_any_element()
        }
    }
}

fn remeasure(handle: &ScrollHandle, used: Point<Pixels>) -> impl IntoElement {
    let handle = handle.clone();
    canvas(|_, _, _| {}, move |_, _, window, _| {
        let now = handle.max_offset();
        if (now.x - used.x).abs() > px(0.5) || (now.y - used.y).abs() > px(0.5) {
            window.on_next_frame(|window, _| window.refresh());
        }
    })
    .absolute()
    .size_0()
}

fn scroll_y(mut panel: Stateful<Div>, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let key = panel.interactivity().element_id.clone().expect("scroll panels have an id");
    let handle = window.use_keyed_state(key, cx, |_, _| ScrollHandle::new()).read(cx).clone();
    hand_on(panel.overflow_y_scroll().track_scroll(&handle), handle)
}

fn scroll_xy(panel: Stateful<Div>, handle: ScrollHandle) -> Stateful<Div> {
    hand_on(panel.overflow_scroll().restrict_scroll_to_axis().track_scroll(&handle), handle)
}

fn scroll_x(panel: Stateful<Div>, handle: ScrollHandle) -> Stateful<Div> {
    hand_on(panel.overflow_x_scroll().restrict_scroll_to_axis().track_scroll(&handle), handle)
}

const DIFF_LINES: usize = 60;

fn render_diff(diff: &celldiff::CellDiff, all: bool, id: ElementId, cx: &mut App) -> AnyElement {
    use celldiff::Change;
    let max_lines = if all { usize::MAX } else { DIFF_LINES };
    let line = theme::chat_px(DETAIL_LINE);
    let shown: Vec<(&'static str, Option<Rgba>, &str)> = diff
        .lines
        .iter()
        .take(max_lines)
        .map(|(change, text)| match change {
            Change::Added => ("+", Some(theme::diff_add_tint()), text.as_str()),
            Change::Removed => ("−", Some(theme::diff_del_tint()), text.as_str()),
            Change::Same => ("", None, text.as_str()),
        })
        .collect();
    let handle = kept_state(id.clone(), cx, |_| ScrollHandle::new()).read(cx).clone();
    let max = handle.max_offset();
    let more_right = max.x > px(0.5) && handle.offset().x > -max.x + px(1.);
    let sign_color = |sign: &str| match sign {
        "+" => theme::diff_add(),
        "−" => theme::diff_del(),
        _ => theme::text_faint(),
    };
    let signs = div().flex_none().flex().flex_col().w(theme::chat_px(22.)).children(shown.iter().map(|(sign, tint, _)| {
        div().h(line).pl(theme::chat_px(10.)).when_some(*tint, |d, t| d.bg(t)).text_color(sign_color(sign)).child(sign.to_string())
    }));
    let code = div().flex().flex_col().min_w_full().children(shown.iter().map(|(_, tint, text)| {
        div().h(line).pr(px(10.)).whitespace_nowrap().when_some(*tint, |d, t| d.bg(t)).child(text.to_string())
    }));
    div()
        .flex()
        .flex_col()
        .line_height(line)
        .text_color(theme::text_secondary())
        .child(div().px(px(10.)).pb(px(2.)).text_color(theme::text_faint()).child(diff.label.clone()))
        .child(
            div()
                .flex()
                .child(signs)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .child(scroll_x(div().id(id).flex().flex_col().items_start(), handle.clone()).child(code))
                        .child(remeasure(&handle, max))
                        .when(more_right, |d| d.child(fade(FadeEdge::Right, theme::bg_sunken()))),
                ),
        )
        .when(diff.lines.len() > max_lines, |d| {
            d.child(div().px(px(10.)).font_family(theme::SANS).text_color(theme::text_faint()).child(format!("… {} more lines", diff.lines.len() - max_lines)))
        })
        .into_any_element()
}

pub(crate) fn answered_line(approval: Approval, clock: &str) -> String {
    let label = approval.label();
    let mut chars = label.chars();
    let capital = chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
    format!("{capital} at {clock}")
}

fn tool_verb(title: &str) -> String {
    let Some(tool) = celldiff::notebook_tool(title) else { return title.to_string() };
    match tool {
        "read_cell" | "read_notebook_code" | "notebook_guide" => "Read",
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
        "interrupt_notebook" => "Stopped the notebook's run",
        "close_notebook" => "Closed the notebook",
        other => other,
    }
    .to_string()
}

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
        let read = answer.as_ref().filter(|a| a["cell_id"] == input["cell_id"]).and_then(|a| a["code"].as_str()).map(cell_label);
        read.or_else(|| field("code").map(cell_label)).or_else(|| field("cell_id").and_then(name))
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

fn cell_name(diff: &celldiff::CellDiff) -> String {
    match &diff.edit {
        Some(edit) => cell_label(edit.after.as_deref().or(edit.before.as_deref()).unwrap_or("")),
        None => cell_label(&diff.lines.iter().filter(|(c, _)| *c != celldiff::Change::Removed).map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n")),
    }
}

pub(crate) struct ToolLine {
    pub verb: String,
    pub object: Option<String>,
    mono: bool,
    full: Option<String>,
}

fn tool_line(title: &str, kind: ToolKind, path: Option<&Path>, input: &serde_json::Value) -> ToolLine {
    let field = |name: &str| input[name].as_str().map(str::trim).filter(|s| !s.is_empty());
    let line = |verb: &str, object: Option<&str>, mono: bool| ToolLine { verb: verb.into(), object: object.map(first_line), mono, full: None };
    match kind {
        ToolKind::Execute => match field("description") {
            Some(description) => {
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
        ToolKind::SwitchMode if input["plan"].is_string() => {
            let plan = crate::approval::plan_title(input["plan"].as_str().unwrap_or(""));
            ToolLine { verb: "Plan".into(), object: plan, mono: false, full: None }
        }
        ToolKind::Fetch => match field("url") {
            Some(url) => line("Fetched", Some(url_host(url)), true),
            None => line("Searched", field("query").or(Some(title)), true),
        },
        _ => match title.strip_prefix("Load skill: ") {
            Some(skill) => line("Used skill", Some(skill), true),
            None => line(&first_line(title), None, false),
        },
    }
}

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

fn first_line(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or("").to_string();
    if lines.next().is_some() { format!("{first} …") } else { first }
}

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
    Some(celldiff::CellDiff { label, lines: celldiff::line_diff(old, new), edit: None })
}

#[cfg(test)]
mod tests {
    #[test]
    fn share_cap_gives_what_a_part_does_not_need_to_the_others() {
        assert_eq!(super::share_cap(300., &[Some(1000.), Some(20.), None]), [280., 20., 0.], "a one-line output leaves the rest to the diff");
        assert_eq!(super::share_cap(300., &[Some(1000.), Some(1000.), Some(f32::INFINITY)]), [100., 100., 100.], "all long: even shares");
        assert_eq!(super::share_cap(300., &[Some(50.), Some(1000.), Some(f32::INFINITY)]), [50., 125., 125.]);
        assert_eq!(super::share_cap(300., &[None, Some(1000.), None]), [0., 300., 0.], "one part takes the whole cap");
    }

    use super::{RowState, delivery_note, run_summary, tool_row};
    use crate::agent::Agent;
    use crate::hosts::Place;
    use crate::outbox::Delivery;
    use crate::session::{Entry, Session};

    #[test]
    fn a_long_path_shows_its_folder_and_name() {
        assert_eq!(super::short_path("/tmp/a/very/long/folder/growth.csv").as_deref(), Some("…/folder/growth.csv"));
        assert_eq!(super::short_path("/data/x.csv"), None, "short enough");
        assert_eq!(super::short_path("growth.csv"), None);
        assert_eq!(super::short_path("C:\\Users\\me\\data\\x.csv").as_deref(), Some("…/data/x.csv"));
        assert_eq!(super::short_path("yes"), None);
    }

    #[test]
    fn a_codex_delivery_note_names_codex() {
        assert_eq!(delivery_note(Delivery::Turn, Agent::Codex), None);
        assert_eq!(delivery_note(Delivery::Joined, Agent::Codex), Some("Codex got this while working"));
        assert_eq!(delivery_note(Delivery::AfterStop, Agent::Codex), Some("Stopped Codex's work to send this"));
    }

    #[test]
    fn a_replayed_denial_is_told_apart_from_a_failure() {
        use agent_client_protocol::schema::v1::{ToolCallId, ToolCallStatus, ToolKind};
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
            edit: None,
        };
        assert_eq!(super::cell_name(&diff(&[(Change::Added, "x = 5 + 5")])), "x");
        assert_eq!(super::cell_name(&diff(&[(Change::Removed, "old = 1"), (Change::Added, "model(S, p) = p[1] * S")])), "model");
        assert_eq!(super::cell_name(&diff(&[(Change::Added, "function fit!(p) = 1")])), "fit!");
        assert_eq!(super::cell_name(&diff(&[(Change::Added, "scatter(data.S, r)")])), "scatter(data.S, r)");
        assert_eq!(super::cell_name(&diff(&[(Change::Added, "# true = heads"), (Change::Added, "flips = rand(Bool, 100)")])), "flips");
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
        assert_eq!(line("read_cell", json!({"cell_id": "c-9"}), out(json!({"cell_id": "c-9", "code": "scatter(x)"})), false), ("Read".into(), Some("scatter(x)".into())));
        assert_eq!(line("read_notebook_code", json!({"notebook_id": "n"}), out(json!({"path": "/w/fit.jl", "code": ""})), false), ("Read".into(), Some("fit.jl".into())));
        assert_eq!(line("read_notebook_code", json!({"notebook_id": "n"}), None, true), ("Reading the notebook".into(), None));
        assert_eq!(line("list_notebooks", null.clone(), None, false), ("Listed notebooks".into(), None));
        assert_eq!(line("fold_cell", null.clone(), None, false), ("fold_cell".into(), None));
        let bash = super::running_line(super::tool_line("Terminal", ToolKind::Execute, None, &null), "Terminal", ToolKind::Execute, &null, true);
        assert_eq!((bash.verb, bash.object), ("Running a command".into(), None), "a command before its input arrives");
    }
}
