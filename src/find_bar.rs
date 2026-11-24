use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::new_session::{Glyph, glyph};
use crate::{Workspace, theme, webcontent};

pub struct FindBar {
    pub input: Entity<InputState>,
    pub found: Option<bool>,
    search: u64,
}

pub fn result_text(found: Option<bool>) -> Option<&'static str> {
    (found == Some(false)).then_some("No matches")
}

pub const KEYS: &str = "↩ next · ⇧↩ previous · esc close";

impl Workspace {
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.webview.read(cx).raw().focus_parent();
        if let Some(find) = &self.find {
            find.input.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            });
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in notebook"));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::Change => this.find_in_page(false, cx),
            InputEvent::PressEnter { shift, .. } => this.find_in_page(*shift, cx),
            _ => {}
        })
        .detach();
        input.update(cx, |s, cx| s.focus(window, cx));
        self.find = Some(FindBar { input, found: None, search: 0 });
        cx.notify();
    }

    pub fn close_find(&mut self, cx: &mut Context<Self>) {
        if self.find.take().is_none() {
            return;
        }
        let webview = self.webview.read(cx).raw();
        webcontent::clear_find(webview);
        webcontent::give_keyboard(webview);
        cx.notify();
    }

    pub fn find_in_page(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else { return };
        let text = find.input.read(cx).value().to_string();
        let webview = self.webview.read(cx).raw();
        find.search += 1;
        if text.is_empty() {
            webcontent::clear_find(webview);
            find.found = None;
            return cx.notify();
        }
        let (search, answer) = (find.search, webcontent::find(webview, &text, backwards));
        cx.spawn(async move |this, cx| {
            let found = answer.await.unwrap_or(false);
            let _ = this.update(cx, |this, cx| {
                if let Some(find) = this.find.as_mut().filter(|f| f.search == search) {
                    find.found = Some(found);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn render_find_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let focused = find.input.read(cx).focus_handle(cx).is_focused(window);
        let empty = find.input.read(cx).value().is_empty();
        let missed = find.found == Some(false);
        let can_step = !empty && !missed;
        let button = |id: &'static str, icon: Glyph, label: &'static str, enabled: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .flex_shrink_0()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::row_active())))
                .child(glyph(icon, if enabled { theme::text_muted() } else { theme::text_section() }))
                .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(label).build(window, cx))
        };
        let edge = if missed {
            theme::danger()
        } else if focused {
            theme::focus_ring()
        } else {
            theme::control_edge()
        };
        Some(
            div()
                .id("find-bar")
                .role(Role::Search)
                .aria_label("Find in notebook")
                .h(px(36.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(4.))
                .pl(px(16.))
                .pr(px(10.))
                .bg(theme::bg_page())
                .border_b_1()
                .border_color(theme::divider())
                .child(
                    div()
                        .w(px(240.))
                        .flex_shrink_0()
                        .h(px(26.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .bg(theme::composer_bg())
                        .border_1()
                        .border_color(edge)
                        .child(glyph(Glyph::Search, theme::text_muted()))
                        .child(div().flex_1().min_w_0().child(Input::new(&find.input).appearance(false).aria_label("Find in notebook").text_size(theme::size_body()))),
                )
                .child(button("find-previous", Glyph::ChevronUp, "Previous match (⇧↩)", can_step).when(can_step, |d| d.on_click(cx.listener(|this, _, _, cx| this.find_in_page(true, cx)))))
                .child(button("find-next", Glyph::Chevron, "Next match (↩)", can_step).when(can_step, |d| d.on_click(cx.listener(|this, _, _, cx| this.find_in_page(false, cx)))))
                .children(result_text(find.found).map(|text| div().flex_shrink_0().ml(px(6.)).text_size(theme::size_meta()).text_color(theme::text_muted()).child(text)))
                .child(div().flex_1().min_w_0())
                .when(can_step, |d| d.child(div().flex_shrink_1().min_w_0().truncate().text_size(px(11.5)).text_color(theme::text_faint()).child(KEYS)))
                .child(button("find-close", Glyph::Close, "Close (esc)", true).on_click(cx.listener(|this, _, _, cx| this.close_find(cx))))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::result_text;

    #[test]
    fn the_bar_says_no_matches_only_after_a_search_without_a_match() {
        assert_eq!(result_text(None), None);
        assert_eq!(result_text(Some(true)), None);
        assert_eq!(result_text(Some(false)), Some("No matches"));
    }
}
