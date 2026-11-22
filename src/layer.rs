//! Everything that floats over the window's own layout goes through here: the
//! order things stack in, from one list, and the hole a floating thing needs
//! in the notebook's web view to show over it.
//!
//! GPUI paints deferred elements by priority, and equal priorities in the
//! order they were rendered. Hand-picked numbers at each site drifted (the ⋮
//! menu and the failure notice were both 1, so the notice covered the menu),
//! so the order now lives in `Layer` and nowhere else.

use gpui::*;

use crate::overlay;

/// What a hole in the notebook's web view is for; each has at most one. The
/// web view is a native view on top of everything GPUI draws, so a GPUI
/// element shows over it only where it has a hole (src/overlay*.rs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hole {
    Menu,
    Tip,
    Tooltip,
    /// The Settings panel.
    Settings,
    /// The confirm dialog.
    Confirm,
    /// A one-off failure's notice.
    Notice,
}

/// Where a floating thing stacks, from the bottom up. Something the user just
/// opened goes above something that only tells them: a menu opens over a
/// notice, not under it. A menu inside a panel has its own layer just above
/// that panel.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Layer {
    /// A failure's notice over the notebook pane.
    Notice,
    /// Menus and popovers: the ⋮ and session menus, the sidebar filter, the
    /// composer's menus and chips, the new-session pickers, the reply pill.
    Popover,
    Settings,
    /// A menu inside Settings.
    SettingsMenu,
    ServerDialog,
    /// A menu inside the server dialog.
    ServerDialogMenu,
    LoginNodeWarning,
    /// A server's password prompt.
    Askpass,
    /// The confirm dialog. It opens from inside Settings (Stop, Repair, Sign out).
    Confirm,
    /// A failure's notice over Settings, from something Settings (or its
    /// confirm dialog) started.
    NoticeOverSettings,
}

impl Layer {
    /// GPUI's deferred priority for this layer (0 is anything not deferred).
    fn priority(self) -> usize {
        self as usize + 1
    }
}

/// `element` drawn above the window's layout at `layer`.
pub fn place(layer: Layer, element: impl IntoElement) -> Deferred {
    deferred(element).with_priority(layer.priority())
}

/// Spike: ENDEAVOR_TEST_NO_HOLES (debug builds) cuts no holes, to see whether GPUI
/// Fast's window composition draws floating things above the web view by itself.
pub fn no_holes() -> bool {
    cfg!(debug_assertions) && std::env::var_os("ENDEAVOR_TEST_NO_HOLES").is_some()
}

/// `body` with a hole cut for it in the notebook's web view, while `webview`
/// is given (pass it only while the web view shows). The hole is pinned to
/// the corner of a plain box around `body`: drawn inside `body`, it would
/// start inside its border and padding, and the web view would cover its edge.
pub fn cut(body: impl IntoElement, hole: Hole, webview: Option<&gpui_webview::WebView>) -> Div {
    let canvas = webview.map(|webview| {
        let (handle, under) = (webview.handle(), webview.bounds());
        canvas(move |bounds, _, _| overlay::set_hole(handle.raw(), hole, Some(Bounds { origin: bounds.origin - under.origin, size: bounds.size })), |_, _, _, _| ())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
    });
    div().relative().child(body).children(canvas)
}

#[cfg(test)]
mod tests {
    use super::Layer;

    #[test]
    fn what_the_user_opens_stacks_above_what_tells_them() {
        // #234: the ⋮ menu opened under the failure notice.
        assert!(Layer::Popover > Layer::Notice);
        // A menu inside a panel shows over the panel.
        assert!(Layer::SettingsMenu > Layer::Settings);
        assert!(Layer::ServerDialogMenu > Layer::ServerDialog);
        // The confirm dialog opens from Settings; a notice from either goes over both.
        assert!(Layer::Confirm > Layer::SettingsMenu);
        assert!(Layer::NoticeOverSettings > Layer::Confirm);
        assert!(Layer::Notice.priority() > 0);
    }
}
