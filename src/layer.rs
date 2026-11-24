use gpui::*;

use crate::overlay;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hole {
    Menu,
    Tip,
    Tooltip,
    Settings,
    Confirm,
    Notice,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Layer {
    Notice,
    Popover,
    Settings,
    SettingsMenu,
    ServerDialog,
    ServerDialogMenu,
    LoginNodeWarning,
    Askpass,
    Confirm,
    NoticeOverSettings,
}

impl Layer {
    fn priority(self) -> usize {
        self as usize + 1
    }
}

pub fn place(layer: Layer, element: impl IntoElement) -> Deferred {
    deferred(element).with_priority(layer.priority())
}

pub fn cut(body: impl IntoElement, hole: Hole, webview: Option<&gpui_wry::WebView>) -> Div {
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
        assert!(Layer::Popover > Layer::Notice);
        assert!(Layer::SettingsMenu > Layer::Settings);
        assert!(Layer::ServerDialogMenu > Layer::ServerDialog);
        assert!(Layer::Confirm > Layer::SettingsMenu);
        assert!(Layer::NoticeOverSettings > Layer::Confirm);
        assert!(Layer::Notice.priority() > 0);
    }
}
