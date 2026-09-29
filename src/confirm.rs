//! The shared in-app confirm dialog: Stop a host, Cancel a queued job, Repair
//! Julia, Sign out and Delete session all ask through this now, instead of a
//! macOS alert (`window.prompt`). One at a time; opening a second replaces
//! the first as if it had been cancelled, though nothing today does that.

use std::rc::Rc;

use gpui::*;
use gpui_component::FocusTrapElement as _;

use crate::server_dialog::button;
use crate::{Workspace, overlay, theme};

pub struct Confirm {
    title: SharedString,
    body: SharedString,
    action_label: &'static str,
    on_confirm: Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>,
    focus_cancel: FocusHandle,
    focus_action: FocusHandle,
    /// Anchors the Tab trap; never itself focused or a tab stop.
    container_focus: FocusHandle,
    /// Focus to return to once this closes, either way.
    restore: Option<FocusHandle>,
}

impl Workspace {
    /// Ask before an action a macOS alert used to gate. `title` names what's
    /// affected ("Stop Julia on lab-server?"), `body` is one or two lines
    /// saying what closes and what stays, `action_label` is the primary
    /// button; Cancel is always the other. Confirming runs `on_confirm` and
    /// closes the dialog; Esc, Cancel or a click outside just closes it.
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

    /// Close without acting, as Esc, Cancel or a click outside does.
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
        // Above the web view like Settings: a hole cut where the card is, so
        // its own hit-testing wins there instead of the native view's.
        let hole = self.webview.read(cx).visible().then(|| {
            let webview = self.webview.read(cx);
            let (handle, under) = (webview.handle(), webview.bounds());
            canvas(move |bounds, _, _| overlay::set_hole(handle.raw(), overlay::Hole::Confirm, Some(Bounds { origin: bounds.origin - under.origin, size: bounds.size })), |_, _, _, _| ())
                .absolute()
                .size_full()
        });
        let card = div()
            .id("confirm-dialog")
            .role(Role::Dialog)
            .aria_label(confirm.title.clone())
            // Tab and ⇧⇥ still come from gpui_component::Root's own bindings;
            // this just tells Root's handler to cycle back at this
            // container's edge instead of leaving it (gpui_base's FocusTrap).
            .focus_trap("confirm-dialog", &confirm.container_focus)
            .occlude()
            .relative()
            .w(px(360.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .rounded(px(12.))
            .bg(theme::bg_raised())
            .border_1()
            .border_color(theme::composer_edge())
            .shadow(vec![BoxShadow { color: hsla(0., 0., 0., 0.6), offset: point(px(0.), px(20.)), blur_radius: px(50.), spread_radius: px(0.), inset: false }])
            .children(hole)
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
                .child(card)
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
