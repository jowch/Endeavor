use std::rc::Rc;

use gpui::*;
use gpui_component::FocusTrapElement as _;

use crate::server_dialog::button;
use crate::layer::{self, Hole};
use crate::{Workspace, theme};

pub struct Confirm {
    title: SharedString,
    body: SharedString,
    action_label: &'static str,
    on_confirm: Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>,
    focus_cancel: FocusHandle,
    focus_action: FocusHandle,
    container_focus: FocusHandle,
    restore: Option<FocusHandle>,
}

impl Workspace {
    pub(crate) fn open_confirm(
        &mut self,
        title: impl Into<SharedString>,
        body: impl Into<SharedString>,
        action_label: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_confirm: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        let focus_action = cx.focus_handle().tab_stop(true);
        let restore = window.focused(cx);
        self.confirm = Some(Confirm {
            title: title.into(),
            body: body.into(),
            action_label,
            on_confirm: Rc::new(on_confirm),
            focus_cancel: cx.focus_handle().tab_stop(true),
            focus_action: focus_action.clone(),
            container_focus: cx.focus_handle(),
            restore,
        });
        window.focus(&focus_action, cx);
        cx.notify();
    }

    pub(crate) fn close_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirm) = self.confirm.take() else { return };
        if let Some(restore) = confirm.restore {
            window.focus(&restore, cx);
        }
        cx.notify();
    }

    fn confirm_act(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirm) = self.confirm.take() else { return };
        if let Some(restore) = &confirm.restore {
            window.focus(restore, cx);
        }
        (confirm.on_confirm)(self, window, cx);
        cx.notify();
    }

    pub fn render_confirm(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let confirm = self.confirm.as_ref()?;
        let card = div()
            .id("confirm-dialog")
            .role(Role::Dialog)
            .aria_label(confirm.title.clone())
            .focus_trap("confirm-dialog", &confirm.container_focus)
            .occlude()
            .relative()
            .w(px(360.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .rounded(px(12.))
            .bg(theme::popover_bg())
            .border_1()
            .border_color(theme::popover_edge())
            .shadow(theme::dialog_shadow(20., 50.))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_size(theme::size_body()).text_color(theme::text_primary()).child(confirm.title.clone()))
            .child(div().text_size(theme::size_meta()).text_color(theme::text_secondary()).child(confirm.body.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                button("confirm-cancel", "Cancel", false, &confirm.focus_cancel, theme::popover_bg())
                                    .aria_label("Cancel")
                                    .on_click(cx.listener(|this, _, window, cx| this.close_confirm(window, cx))),
                            )
                            .child(div().text_size(theme::size_meta()).text_color(theme::text_faint()).child("esc")),
                    )
                    .child(
                        button("confirm-action", confirm.action_label, true, &confirm.focus_action, theme::popover_bg())
                            .aria_label(confirm.action_label)
                            .on_click(cx.listener(|this, _, window, cx| this.confirm_act(window, cx))),
                    ),
            );
        Some(
            div()
                .id("confirm-backdrop")
                .absolute()
                .inset_0()
                .bg(theme::scrim())
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_dismissible(window, cx)))
                .child(layer::cut(card, Hole::Confirm, Some(self.webview.read(cx)).filter(|w| w.visible())))
                .into_any_element(),
        )
    }
}

#[cfg(debug_assertions)]
impl Confirm {
    pub(crate) fn debug_state(&self) -> serde_json::Value {
        serde_json::json!({
            "title": self.title,
            "text": self.body,
            "buttons": [
                { "label": "Cancel", "primary": false },
                { "label": self.action_label, "primary": true },
            ],
        })
    }
}
