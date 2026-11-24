use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{AnyWindowHandle, App, AsyncApp, Keystroke, Modifiers};
use gtk::gdk;
use gtk::glib::Propagation;
use gtk::prelude::*;

const APP_FIRST: [&str; 7] = ["ctrl-q", "ctrl-,", "ctrl-b", "ctrl-=", "ctrl--", "ctrl-0", "ctrl-shift-e"];

fn keystroke(event: &gdk::EventKey) -> Option<Keystroke> {
    let key = event.keyval().to_lower().to_unicode().filter(|c| !c.is_control())?;
    let state = event.state();
    let modifiers = Modifiers {
        control: state.contains(gdk::ModifierType::CONTROL_MASK),
        alt: state.contains(gdk::ModifierType::MOD1_MASK),
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
        platform: state.contains(gdk::ModifierType::SUPER_MASK),
        function: false,
    };
    Some(Keystroke { modifiers, key: key.to_string(), key_char: None })
}

fn app_first(keystroke: &Keystroke) -> bool {
    APP_FIRST.iter().filter_map(|s| Keystroke::parse(s).ok()).any(|k| k.modifiers == keystroke.modifiers && k.key == keystroke.key)
}

pub fn forward_app_keys(webview: &wry::WebView, handle: AnyWindowHandle, gpui: u64, cx: &mut App) {
    use wry::WebViewExtUnix;
    let (tx, mut rx) = unbounded::<Keystroke>();
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(keystroke) = rx.next().await {
            let _ = handle.update(cx, |_, window, cx| {
                let before = window.focused(cx);
                window.dispatch_keystroke(keystroke, cx);
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        if window.focused(cx) != before {
                            super::focus_gpui(gpui);
                        }
                    });
                });
            });
        }
    })
    .detach();
    let send = |tx: &UnboundedSender<Keystroke>, keystroke| {
        let _ = tx.unbounded_send(keystroke);
        Propagation::Stop
    };
    let view = webview.webview();
    let first = tx.clone();
    view.connect_key_press_event(move |_, event| match keystroke(event) {
        Some(k) if app_first(&k) => send(&first, k),
        _ => Propagation::Proceed,
    });
    if let Some(parent) = view.parent() {
        parent.connect_key_press_event(move |_, event| match keystroke(event) {
            Some(k) if k.modifiers.control => send(&tx, k),
            _ => Propagation::Proceed,
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_app_first_key_parses() {
        for key in super::APP_FIRST {
            let keystroke = gpui::Keystroke::parse(key).unwrap_or_else(|_| panic!("{key} doesn't parse"));
            assert!(keystroke.modifiers.control, "{key} has no Ctrl");
        }
    }
}
