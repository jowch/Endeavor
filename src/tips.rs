//! One-time tips: "Two ways to add your file" over the new-session composer,
//! and Point's tip under the notebook header's Point button. Each goes for good
//! after Got it or the first use of what it explains (the settings remember).

use gpui::*;

use crate::new_session::{Glyph, glyph_at};
use crate::{Workspace, overlay, theme};

const ARROW: f32 = 7.;

const POINT_TIP: &str = "Point lets you click a cell, or drag over part of a plot, and ask Claude about just that.";

/// The tip's card: raised, outlined, with a soft shadow.
fn card() -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::composer_edge())
        .bg(theme::bg_raised())
        .shadow(vec![BoxShadow {
            color: hsla(0., 0., 0., 0.45),
            offset: point(px(0.), px(12.)),
            blur_radius: px(32.),
            spread_radius: px(0.),
            inset: false,
        }])
        .text_size(theme::size_meta())
        .line_height(px(17.))
        .text_color(theme::text_row_active())
}

/// The card's pointer, a small triangle on its edge, outlined like the card.
/// `down`: it hangs under the card, else it sits on top.
fn arrow(down: bool) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let (l, r, mid) = (b.left(), b.right(), b.left() + b.size.width / 2.);
            let (base, tip) = if down { (b.top(), b.bottom()) } else { (b.bottom(), b.top()) };
            let mut fill = PathBuilder::fill();
            fill.move_to(point(l, base));
            fill.line_to(point(mid, tip));
            fill.line_to(point(r, base));
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, theme::bg_raised());
            }
            let mut edge = PathBuilder::stroke(px(1.));
            edge.move_to(point(l, base));
            edge.line_to(point(mid, tip));
            edge.line_to(point(r, base));
            if let Ok(path) = edge.build() {
                window.paint_path(path, theme::composer_edge());
            }
        },
    )
    .w(px(ARROW * 2.))
    .h(px(ARROW))
}

/// A key or button named in a tip's text: "@", "+".
fn key(text: &'static str) -> Div {
    div()
        .min_w(px(16.))
        .px(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme::composer_edge())
        .bg(theme::bg_card())
        .font_family(theme::MONO)
        .text_size(theme::size_meta_small())
        .line_height(px(16.))
        .text_center()
        .text_color(theme::text_primary())
        .child(text)
}

fn got_it(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .h(px(24.))
        .flex()
        .items_center()
        .px(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme::composer_edge())
        .bg(theme::bg_tag())
        .cursor_pointer()
        .hover(|s| s.bg(theme::composer_edge()))
        .text_color(theme::text_primary())
        .font_weight(FontWeight::MEDIUM)
        .child("Got it")
}

impl Workspace {
    /// The file tip is done: Got it, or the user added a file.
    pub fn file_tip_done(&mut self) {
        self.file_tip = false;
        if !self.settings.file_tip_seen {
            self.settings.file_tip_seen = true;
            self.settings.save();
        }
    }

    /// Point's tip is done: Got it, or Point was used.
    pub fn point_tip_done(&mut self) {
        if !self.settings.point_tip_seen {
            self.settings.point_tip_seen = true;
            self.settings.save();
        }
    }

    pub fn file_tip_shows(&self) -> bool {
        self.file_tip && !self.settings.file_tip_seen
    }

    /// "Two ways to add your file", laid over the chips above the composer
    /// with its pointer at the box. The caller's element is `relative`, with
    /// the chips at its top and 16px side padding.
    pub fn render_file_tip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.file_tip_shows() {
            return None;
        }
        let line = |before: &'static str, k: &'static str, after: &'static str| {
            div().flex().items_center().gap(px(4.)).child(before).child(key(k)).child(after)
        };
        Some(
            div()
                .absolute()
                .left(px(16.))
                .right(px(16.))
                .bottom(relative(1.))
                .mb(px(-28.))
                .child(
                    card()
                        .occlude()
                        .pl(px(12.))
                        .pr(px(8.))
                        .py(px(10.))
                        .child(
                            div()
                                .flex_1().min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .child(div().font_weight(FontWeight::MEDIUM).text_color(theme::text_primary()).child("Two ways to add your file"))
                                .child(line("Type", "@", "to name a file in this session's folder."))
                                .child(line("Use", "+", "to attach one from anywhere else.")),
                        )
                        .child(got_it("file-tip-ok").on_click(cx.listener(|this, _, _, cx| {
                            this.file_tip_done();
                            cx.notify();
                        })))
                        .child(div().absolute().left(px(40.)).bottom(px(1. - ARROW)).child(arrow(true))),
                )
                .into_any_element(),
        )
    }

    /// Whether Point's tip shows, given that the notebook is on screen. Not
    /// while a menu is open: the menu would cover it.
    pub fn point_tip_shows(&self, notebook_shown: bool) -> bool {
        notebook_shown && !self.settings.point_tip_seen && self.menu.is_none()
    }

    /// Point's tip, hanging from the notebook header's Point button over the
    /// notebook. The web view is a native view on top, so it gets a hole where
    /// the tip is; the workspace closes it once the tip stops showing.
    /// `trailing`: how far the header's buttons after Point reach past it, so
    /// the tip lines up with the pane's edge as its pointer stays on Point.
    pub fn render_point_tip(&self, trailing: f32, cx: &mut Context<Self>) -> AnyElement {
        const WIDTH: f32 = 400.;
        let webview = self.webview.read(cx);
        let (handle, under) = (webview.handle(), webview.bounds());
        let cut = canvas(
            move |bounds, _, _| {
                let rect = Bounds { origin: bounds.origin - under.origin, size: bounds.size };
                overlay::set_hole(handle.raw(), overlay::Hole::Tip, Some(rect));
            },
            |_, _, _, _| (),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .absolute()
            .top(relative(1.))
            .right(px(-trailing))
            .pt(px(6.))
            .w(px(WIDTH))
            .child(
                card()
                    .occlude()
                    .cursor_default()
                    .pl(px(12.))
                    .pr(px(8.))
                    .py(px(8.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex_1().min_w_0()
                            .flex()
                            .items_start()
                            .gap(px(8.))
                            .child(div().mt(px(2.)).child(glyph_at(Glyph::Pointer, theme::accent_text(), 13. / 12.)))
                            .child(div().flex_1().min_w_0().child(StyledText::new(POINT_TIP).with_highlights([(
                                0.."Point".len(),
                                HighlightStyle { color: Some(theme::text_primary().into()), font_weight: Some(FontWeight::MEDIUM), ..Default::default() },
                            )]))),
                    )
                    .child(got_it("point-tip-ok").on_click(cx.listener(|this, _, _, cx| {
                        this.point_tip_done();
                        cx.notify();
                    })))
                    .child(div().absolute().right(px(trailing + 26. - ARROW)).top(px(1. - ARROW)).child(arrow(false)))
                    .child(cut),
            )
            .into_any_element()
    }
}
