//! Queued messages above the composer: a heading that says how they go, then
//! one row each. ✎ puts a message in the composer while its row keeps its
//! place; ✕ removes it at once, with Undo for a few seconds; ↑ sends it now.
//! Clicking a row's words opens the whole message in place.

use std::time::Instant;

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::Workspace;
use crate::attach::Attachment;
use crate::composer::chip;
use crate::new_session::{Glyph, glyph};
use crate::outbox::{Dispatch, Outbox, UNDO_FOR};
use crate::session::Session;
use crate::theme::{self, FocusRing as _};

/// With more than this many rows, the stack shows `SHOWN` and "N more ›".
const FOLD_OVER: usize = 4;
const SHOWN: usize = 3;
/// Chips a closed row shows before "+N".
const ROW_CHIPS: usize = 2;

/// A queued message being edited in the composer, and what the composer held
/// before, which comes back once the edit is done.
pub struct QueueEdit {
    pub key: u64,
    aside: (String, Vec<Attachment>),
}

impl Workspace {
    /// ✎ on row `ix` of session `key`: the message goes into the composer and
    /// its row stays as a placeholder. Words already in the box are set aside.
    pub fn edit_queued(&mut self, key: u64, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_queue_edit(window, cx);
        let Some((text, attachments)) = self.session_mut(key).and_then(|s| s.begin_edit(ix)) else { return };
        let aside = self.take_composer(window, cx).map(|(text, attachments, _)| (text, attachments)).unwrap_or_default();
        self.queue_edit = Some(QueueEdit { key, aside });
        self.restore_composer(text, attachments, window, cx);
    }

    /// Whether the composer holds session `key`'s queued message being edited.
    pub fn editing_queued(&self, key: u64) -> bool {
        self.queue_edit.as_ref().is_some_and(|e| e.key == key)
    }

    /// The edit went back in its place: what the box held before comes back.
    pub fn queue_edit_done(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(QueueEdit { aside: (text, attachments), .. }) = self.queue_edit.take() {
            self.restore_composer(text, attachments, window, cx);
        }
    }

    /// Esc while editing: the queued message stays as it was, and the box
    /// gets back what it held. False when nothing was being edited.
    pub fn cancel_queue_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.queue_edit.take() else { return false };
        self.queue_do(edit.key, cx, Outbox::cancel_edit);
        let (text, attachments) = edit.aside;
        self.composer.attachments.clear();
        self.restore_composer(text, attachments, window, cx);
        true
    }

    /// Run a queue action on session `key` and send what it starts.
    fn queue_do(&mut self, key: u64, cx: &mut Context<Self>, act: impl FnOnce(&mut Outbox) -> Option<Dispatch>) {
        let Some(session) = self.session_mut(key) else { return };
        let effects = session.queue_action(act);
        self.apply_effects(key, effects, cx);
        cx.notify();
    }

    fn remove_queued(&mut self, key: u64, ix: usize, cx: &mut Context<Self>) {
        self.queue_do(key, cx, |o| o.remove(ix, Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(UNDO_FOR).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(s) = this.session_mut(key) {
                    s.outbox.forget_removed(Instant::now());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The heading over the queue: why it waits (Claude or the server out of
    /// reach, paused), or how its messages go.
    fn render_queue_heading_line(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(held) = self.render_queue_heading(session) {
            return Some(held);
        }
        let text = session.outbox.heading()?;
        let key = session.key;
        let paused = session.outbox.paused().is_some();
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(theme::size_meta())
                .line_height(px(17.))
                .text_color(theme::text_muted())
                .child(glyph(Glyph::Clock, theme::text_muted()))
                .child(text)
                .when(paused, |d| {
                    d.child("·").child(
                        div()
                            .id("queue-send-next")
                            .role(Role::Button)
                            .aria_label("Send next")
                            .cursor_pointer()
                            .text_color(theme::accent_text())
                            .hover(|s| s.underline())
                            .child("Send next")
                            .on_click(cx.listener(move |this, _, _, cx| this.queue_do(key, cx, Outbox::send_next))),
                    )
                })
                .into_any_element(),
        )
    }

    /// Messages waiting for Claude, under their heading.
    pub fn render_queue(&self, session: &Session, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let key = session.key;
        let outbox = &session.outbox;
        let numbered = outbox.items.len() >= 2;
        let folded = outbox.items.len() > FOLD_OVER && !session.queue_all;
        let shown = if folded { SHOWN } else { outbox.items.len() };
        let removed = outbox.removed_at(Instant::now()).map(|r| r.message.text.clone()).zip(outbox.removed_slot());
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, q) in outbox.items.iter().enumerate().take(shown) {
            if let Some((text, _)) = removed.as_ref().filter(|(_, at)| *at == i) {
                rows.push(arriving("queue-removed", self.render_removed_row(session, text, cx)));
            }
            let n = numbered.then_some(i + 1);
            let row = if q.editing() {
                editing_row(n)
            } else if session.queue_open == Some(q.id) {
                self.render_open_row(session, i, n, cx)
            } else {
                self.render_queued_row(session, i, n, cx)
            };
            rows.push(crate::motion::arriving(div().child(row), ElementId::NamedInteger("queue-row".into(), q.id), false).into_any_element());
        }
        if let Some((text, _)) = removed.as_ref().filter(|(_, at)| *at >= shown) {
            rows.push(arriving("queue-removed", self.render_removed_row(session, text, cx)));
        }
        if folded {
            rows.push(
                div()
                    .id("queue-more")
                    .role(Role::Button)
                    .self_start()
                    .pl(px(10.))
                    .cursor_pointer()
                    .text_size(theme::size_meta())
                    .text_color(theme::text_muted())
                    .hover(|s| s.text_color(theme::text_primary()))
                    .child(format!("{} more ›", outbox.items.len() - SHOWN))
                    .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.queue_all = true)))
                    .into_any_element(),
            );
        }
        div().flex().flex_col().gap(px(6.)).children(self.render_queue_heading_line(session, cx).map(|line| arriving("queue-heading", line))).children(rows)
    }

    /// One closed row: its number, its first chips, its words cut to fit, and
    /// ↑ (on hover), ✎ and ✕.
    fn render_queued_row(&self, session: &Session, ix: usize, n: Option<usize>, cx: &mut Context<Self>) -> AnyElement {
        let key = session.key;
        let q = &session.outbox.items[ix];
        let id = q.id;
        let group: SharedString = format!("queued-{id}").into();
        let chips = q.attachments.iter().take(ROW_CHIPS).enumerate().map(|(i, a)| chip(ElementId::NamedInteger("queued-chip".into(), (id << 8) | i as u64), a).h(px(20.)));
        let more = q.attachments.len().saturating_sub(ROW_CHIPS);
        let words = q.text.lines().next().unwrap_or("").to_string();
        row_frame(&group, id)
            .h(px(32.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(4.))
            .hover(|s| s.bg(theme::bg_raised()))
            .children(n.map(number))
            .child(
                div()
                    .id(ElementId::NamedInteger("queued-words".into(), id))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .children(chips)
                    .when(more > 0, |d| d.child(div().flex_shrink_0().text_size(theme::size_meta_small()).text_color(theme::text_muted()).child(format!("+{more}"))))
                    .child(div().min_w_0().truncate().text_color(theme::text_secondary()).child(words))
                    .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.queue_open = Some(id)))),
            )
            .when(q.in_flight(), |d| d.child(div().flex_shrink_0().text_size(theme::size_meta()).text_color(theme::text_muted()).child("sending now…")))
            .when(q.is_copying(), |d| d.child(div().flex_shrink_0().text_size(theme::size_meta()).text_color(theme::text_muted()).child("copying files…")))
            .when(!q.in_flight(), |d| d.child(self.row_buttons(session, ix, &group, false, cx)))
            .into_any_element()
    }

    /// A row opened in place: everything the sent bubble will show.
    fn render_open_row(&self, session: &Session, ix: usize, n: Option<usize>, cx: &mut Context<Self>) -> AnyElement {
        let key = session.key;
        let q = &session.outbox.items[ix];
        let id = q.id;
        let group: SharedString = format!("queued-{id}").into();
        let chips: Vec<_> = q
            .attachments
            .iter()
            .enumerate()
            .filter(|(_, a)| !matches!(a, Attachment::Quote(_)))
            .map(|(i, a)| chip(ElementId::NamedInteger("queued-chip".into(), (id << 8) | i as u64), a).h(px(20.)))
            .collect();
        // The bubble's quote parts, under ids of their own.
        let quotes = self.render_sent_quotes(key, 0xF0_0000 | (id as usize & 0xFFFF), &q.attachments, cx);
        // With nothing above the words, they take the first line.
        let words_first = chips.is_empty() && quotes.is_empty();
        let words = (!q.text.is_empty()).then(|| {
            div()
                .id(ElementId::NamedInteger("queued-open-words".into(), id))
                .cursor_pointer()
                .text_size(theme::chat_body())
                .line_height(theme::chat_line_body())
                .text_color(theme::text_secondary())
                .child(q.text.clone())
                .on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.queue_open = None)))
        });
        let (head_words, body_words) = if words_first { (words.map(|w| w.flex_1().min_w_0()), None) } else { (None, words) };
        row_frame(&group, id)
            .flex()
            .flex_col()
            .gap(px(6.))
            .pt(px(6.))
            .pb(px(8.))
            .pl(px(10.))
            .pr(px(4.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .min_h(px(20.))
                    .children(n.map(|n| number(n).mt(px(3.))))
                    .child(div().flex_1().min_w_0().flex().flex_wrap().gap(px(4.)).children(chips).children(head_words))
                    .when(!q.in_flight(), |d| d.child(self.row_buttons(session, ix, &group, true, cx))),
            )
            .when(!words_first, |d| {
                d.child(
                    div()
                        .pl(px(if n.is_some() { 18. } else { 0. }))
                        .pr(px(6.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .text_size(theme::chat_body())
                        .line_height(theme::chat_line_body())
                        .text_color(theme::text_secondary())
                        .children(quotes)
                        .children(body_words),
                )
            })
            .into_any_element()
    }

    /// ↑ (shown on hover), ✎ and ✕, and ⌃ on an opened row.
    fn row_buttons(&self, session: &Session, ix: usize, group: &SharedString, open: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let key = session.key;
        let id = session.outbox.items[ix].id;
        let can_send = session.can_send_now(ix);
        let shape = |name: &'static str, icon: Glyph, label: &'static str, color: Rgba| {
            div()
                .id(ElementId::NamedInteger(name.into(), id))
                .role(Role::Button)
                .aria_label(label)
                .flex_shrink_0()
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::button_hover()))
                .child(glyph(icon, color))
                .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(label).build(window, cx))
        };
        let button = |name: &'static str, icon: Glyph, label: &'static str, color: Rgba| {
            shape(name, icon, label, color).track_focus(&session.queue_focus(id, name, cx)).focus_ring_on(row_surface())
        };
        let send = if can_send {
            // Drawn see-through rather than hidden, so Tab still reaches it, and
            // shown with its ring once it has focus (this replaces the ring's own style).
            button("queued-send", Glyph::ArrowUp, "Send now (⌘↩)", theme::text_primary())
                .opacity(0.)
                .group_hover(group.clone(), |s| s.opacity(1.))
                .focus_visible(|s| s.opacity(1.).shadow(theme::ring(row_surface())))
                .on_click(cx.listener(move |this, _, _, cx| this.queue_do(key, cx, move |o| o.send_now(ix))))
        } else {
            shape("queued-send", Glyph::ArrowUp, "Send now (⌘↩)", theme::text_section()).cursor_default().invisible().group_hover(group.clone(), |s| s.visible())
        };
        div()
            .flex_shrink_0()
            .flex()
            .gap(px(1.))
            .child(send)
            .child(button("queued-edit", Glyph::Pencil, "Edit", theme::text_muted()).on_click(cx.listener(move |this, _, window, cx| this.edit_queued(key, ix, window, cx))))
            .child(button("queued-remove", Glyph::Close, "Remove", theme::text_muted()).on_click(cx.listener(move |this, _, _, cx| this.remove_queued(key, ix, cx))))
            .when(open, |d| {
                d.child(button("queued-fold", Glyph::ChevronUp, "Fold", theme::text_muted()).on_click(cx.listener(move |this, _, _, cx| this.with_session(key, cx, |s| s.queue_open = None))))
            })
    }

    /// Where a removed message was, for `UNDO_FOR`: "Removed "…"" and Undo.
    fn render_removed_row(&self, session: &Session, text: &str, cx: &mut Context<Self>) -> AnyElement {
        let key = session.key;
        let focus = session.outbox.removed_at(Instant::now()).map(|r| session.queue_focus(r.message.id, "queued-undo", cx));
        div()
            .h(px(32.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .text_size(px(12.5))
            .text_color(theme::text_muted())
            .child(div().flex_1().min_w_0().truncate().child(format!("Removed “{}”", text.lines().next().unwrap_or(""))))
            .child(
                div()
                    .id("queued-undo")
                    .role(Role::Button)
                    .aria_label("Undo")
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .text_color(theme::accent_text())
                    .hover(|s| s.underline())
                    .rounded(px(4.))
                    .when_some(focus, |d, focus| d.track_focus(&focus).focus_ring_on(row_surface()))
                    .child(glyph(Glyph::Undo, theme::accent_text()))
                    .child("Undo")
                    .on_click(cx.listener(move |this, _, _, cx| this.queue_do(key, cx, Outbox::undo_remove))),
            )
            .into_any_element()
    }
}

/// The rows' surface: sunken, or the composer's white in light.
fn row_surface() -> Rgba {
    if theme::is_light() { theme::composer_bg() } else { theme::bg_sunken() }
}

/// A row's frame: rounded, with a hairline, on the sunken surface (on the
/// composer's white, with its shadow, in light).
fn row_frame(group: &SharedString, id: u64) -> Stateful<Div> {
    div()
        .id(ElementId::NamedInteger("queued".into(), id))
        .group(group.clone())
        .rounded(px(8.))
        .border_1()
        .border_color(theme::border())
        .bg(row_surface())
        .when(theme::is_light(), |d| d.shadow(theme::composer_shadow()))
        .text_size(theme::size_body())
}

fn number(n: usize) -> Div {
    div().w(px(12.)).flex_shrink_0().text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(n.to_string())
}

/// The row of the message being edited: a dashed placeholder that keeps its place.
fn editing_row(n: Option<usize>) -> AnyElement {
    div()
        .h(px(32.))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_dashed()
        .border_color(theme::composer_edge())
        .text_size(px(12.5))
        .text_color(theme::text_muted())
        .children(n.map(number))
        .child(glyph(Glyph::Pencil, theme::text_muted()))
        .child(div().flex_1().min_w_0().truncate().child("Editing in the box below · keeps its place"))
        .child(div().flex_shrink_0().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child("esc"))
        .child(div().flex_shrink_0().text_size(px(11.5)).text_color(theme::text_faint()).child("puts it back"))
        .into_any_element()
}

/// Rising in from the composer as it appears.
fn arriving(name: &'static str, element: AnyElement) -> AnyElement {
    crate::motion::arriving(div().child(element), name, false).into_any_element()
}
