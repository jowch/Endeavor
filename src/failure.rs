//! How failures look (boards XTurn, XStopped, XOpen). Three tiers: waiting
//! (muted, fixes itself: `wait_line`), failed (needs the user: `card`, or a
//! `page` in place of what couldn't show, or a `notice` under the control that
//! was used), and recovered (a quiet note in the transcript).

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use std::sync::Arc;

use crate::new_session::{Glyph, glyph_at};
use crate::signin::{Look, button_frame};
use crate::theme;
use crate::theme::TextButton as _;

/// A button with an icon before its label.
pub fn action(id: impl Into<ElementId>, icon: Option<Glyph>, label: impl Into<SharedString>, look: Look) -> Stateful<Div> {
    let tint = match look {
        Look::Primary => gpui::white().into(),
        Look::Secondary | Look::Plain => theme::text_muted(),
    };
    button_frame(id, look).role(Role::Button).children(icon.map(|g| glyph_at(g, tint, 1.))).button_text(label)
}

/// Text whose `backticked` names (a cell's) stand out as code. It wraps as
/// one run of text, which a highlight can't give a font of its own.
pub fn code_text(text: &str) -> StyledText {
    let mut plain = String::new();
    let mut ranges = Vec::new();
    for (i, part) in text.split('`').enumerate() {
        if i % 2 == 1 {
            ranges.push(plain.len()..plain.len() + part.len());
        }
        plain.push_str(part);
    }
    let style = HighlightStyle { color: Some(theme::text_primary().into()), background_color: Some(theme::bg_tag().into()), ..Default::default() };
    StyledText::new(plain).with_highlights(ranges.into_iter().map(move |r| (r, style)))
}

/// The tallest Details gets before it scrolls: about fourteen lines, so a
/// long stack trace stays inside its notice or card instead of running off
/// the window.
pub const DETAILS_MAX_H: f32 = 232.;

/// "Details ›": the raw error, closed until clicked. Open, it scrolls past
/// `DETAILS_MAX_H`, and Copy puts the whole error on the clipboard.
pub fn details(id: impl Into<ElementId>, text: impl Into<SharedString>, open: bool, toggle: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    Details { id: id.into(), text: text.into(), open, toggle: Box::new(toggle) }.into_any_element()
}

#[derive(IntoElement)]
struct Details {
    id: ElementId,
    text: SharedString,
    open: bool,
    toggle: Box<Toggle>,
}

type Toggle = dyn Fn(&mut Window, &mut App);

impl RenderOnce for Details {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Details { id, text, open, toggle } = self;
        let shown = open && !text.is_empty();
        // The box's scroll position, and whether Copy just copied, live as
        // long as the Details do.
        let (scroll, copied) = window.with_element_namespace(id.clone(), |window| {
            let scroll = window.use_keyed_state("scroll", cx, |_, _| ScrollHandle::new()).read(cx).clone();
            (scroll, window.use_keyed_state("copied", cx, |_, _| false))
        });
        let copy = shown.then(|| {
            let done = *copied.read(cx);
            let label = if done { "Copied" } else { "Copy" };
            let text = text.clone();
            div()
                .id(ElementId::NamedChild(Arc::new(id.clone()), "copy".into()))
                .role(Role::Button)
                .aria_label(if done { "Copied" } else { "Copy error" })
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(4.))
                .mr(px(-4.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(theme::size_meta())
                .text_color(theme::text_faint())
                .hover(|s| s.text_color(theme::text_secondary()))
                .child(glyph_at(if done { Glyph::Check } else { Glyph::Copy }, theme::text_faint(), 11. / 12.))
                .child(label)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                    copied.update(cx, |c, cx| {
                        *c = true;
                        cx.notify();
                    });
                    let copied = copied.downgrade();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                        let _ = copied.update(cx, |c, cx| {
                            *c = false;
                            cx.notify();
                        });
                    })
                    .detach();
                })
        });
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(id.clone())
                            .role(Role::Button)
                            .aria_label(if open { "Hide details" } else { "Details" })
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .cursor_pointer()
                            .text_size(theme::size_meta())
                            .text_color(theme::text_faint())
                            .hover(|s| s.text_color(theme::text_secondary()))
                            .child("Details")
                            .child(glyph_at(if open { Glyph::Chevron } else { Glyph::Forward }, theme::text_faint(), 11. / 12.))
                            .on_click(move |_, window, cx| toggle(window, cx)),
                    )
                    .children(copy),
            )
            .when(shown, |d| {
                d.child(
                    div()
                        .id(ElementId::NamedChild(Arc::new(id.clone()), "text".into()))
                        .max_h(px(DETAILS_MAX_H))
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .px(px(10.))
                        .py(px(8.))
                        .rounded(px(6.))
                        .bg(theme::bg_sunken())
                        .font_family(theme::MONO)
                        .text_size(px(11.5))
                        .line_height(px(16.))
                        .text_color(theme::text_muted())
                        // A wheel turn here scrolls the error, not what's behind it, until the error is at its end.
                        .on_scroll_wheel(move |event, window, cx| {
                            let delta = event.delta.pixel_delta(window.line_height());
                            let (at, max) = (scroll.offset(), scroll.max_offset());
                            if delta.y != px(0.) && max.y > px(0.) && if delta.y < px(0.) { at.y > -max.y } else { at.y < px(0.) } {
                                cx.stop_propagation();
                            }
                        })
                        .child(text),
                )
            })
    }
}

/// Something that needs the user, where the thing would have been: a title
/// saying what didn't happen, a plain reason, one primary button, and the raw
/// error under Details.
pub fn card(title: impl Into<SharedString>, body: impl Into<SharedString>, buttons: Vec<AnyElement>, details: Option<AnyElement>) -> Div {
    div()
        .flex()
        .gap(px(10.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg_card())
        .child(div().pt(px(2.)).flex().child(glyph_at(Glyph::Warning, theme::danger(), 15. / 12.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .text_size(theme::size_body())
                .line_height(px(19.))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_primary()).child(title.into()))
                .child(div().text_color(theme::text_secondary()).child(body.into()))
                .when(!buttons.is_empty(), |d| d.child(div().mt(px(4.)).flex().flex_wrap().items_center().gap(px(8.)).children(buttons)))
                .children(details),
        )
}

/// What leads a wait line: a clock, the offline mark, or a spinner.
pub enum Lead {
    Clock,
    Spinner(ElementId),
}

/// Waiting, as offline does it: muted, above the composer, and it fixes
/// itself; at most a quiet Try now.
pub fn wait_line(lead: Lead, text: impl IntoElement, button: Option<AnyElement>, cx: &App) -> Div {
    let lead = match lead {
        Lead::Clock => glyph_at(Glyph::Clock, theme::text_muted(), 14. / 12.).into_any_element(),
        Lead::Spinner(id) => crate::orbit::orbit_with(id, 14., theme::orbit_sphere(), cx),
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(2.))
        .text_size(theme::chat_meta())
        .line_height(px(18.))
        .text_color(theme::text_muted())
        .child(div().flex_shrink_0().flex().child(lead))
        .child(div().flex_1().min_w_0().child(text))
        .children(button)
}

/// A page in place of what couldn't show (a transcript, a notebook): an icon,
/// what happened, why, and at most two things to do.
pub fn page(icon: Glyph, title: impl Into<SharedString>, body: Vec<AnyElement>, buttons: Vec<AnyElement>, details: Option<AnyElement>) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .px(px(40.))
        .text_center()
        .child(div().flex().child(glyph_at(icon, theme::text_muted(), 22. / 12.)))
        .child(div().text_size(theme::size_subhead()).line_height(px(21.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_primary()).child(title.into()))
        .child(div().max_w(px(420.)).flex().flex_col().gap(px(4.)).text_size(theme::size_body()).line_height(px(19.)).text_color(theme::text_secondary()).children(body))
        .child(div().mt(px(6.)).flex().gap(px(8.)).children(buttons))
        .children(details.map(|d| div().w(px(380.)).max_w_full().mt(px(6.)).text_left().child(d)))
}

/// A one-off failure, under the control that was used: what didn't happen,
/// why, a way on, and ×. It stays until closed or tried again.
pub fn notice(title: impl Into<SharedString>, body: impl Into<SharedString>, buttons: Vec<AnyElement>, details: Option<AnyElement>, close: impl Fn(&mut Window, &mut App) + 'static) -> Stateful<Div> {
    div()
        .id("failure-notice")
        .role(Role::Alert)
        .occlude()
        .w(px(340.))
        .flex()
        .gap(px(10.))
        .px(px(14.))
        .py(px(12.))
        .map(theme::popover)
        .rounded(px(10.))
        .font_family(theme::SANS)
        .text_size(theme::size_body())
        .line_height(px(19.))
        .child(div().pt(px(2.)).flex().child(glyph_at(Glyph::Warning, theme::danger(), 15. / 12.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .child(div().flex_1().min_w_0().font_weight(FontWeight::SEMIBOLD).text_color(theme::text_primary()).child(title.into()))
                        .child(
                            div()
                                .id("failure-notice-close")
                                .role(Role::Button)
                                .aria_label("Close")
                                .size(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::row_active()))
                                .child(glyph_at(Glyph::Close, theme::text_faint(), 13. / 12.))
                                .on_click(move |_, window, cx| close(window, cx)),
                        ),
                )
                .child(div().text_color(theme::text_secondary()).child(body.into()))
                .when(!buttons.is_empty(), |d| d.child(div().mt(px(4.)).flex().gap(px(8.)).children(buttons)))
                .children(details),
        )
}
