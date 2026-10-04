//! The app's shortcuts while the notebook has the keyboard. GTK sends keys to
//! the web view, not to GPUI, so they are handed to GPUI here, in the same
//! order as on macOS (`src/webkeys.rs`): the menu bar's shortcuts before the
//! page sees them, other Ctrl shortcuts only when the page doesn't use them.

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{AnyWindowHandle, App, AsyncApp, Keystroke, Modifiers};
use gtk::gdk;
use gtk::glib::Propagation;
use gtk::prelude::*;

/// On macOS these are menu items, which take their keys before the page does.
const APP_FIRST: [&str; 7] = ["secondary-q", "secondary-,", "secondary-b", "secondary-=", "secondary--", "secondary-0", "secondary-shift-e"];

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

/// Send `webview`'s app shortcuts to the GPUI key handling of `handle`'s window. A shortcut
/// that moves GPUI's focus (Ctrl+, focuses Settings' search) brings the keyboard
/// to GPUI's X window, `gpui`, too.
pub fn forward_app_keys(webview: &wry::WebView, handle: AnyWindowHandle, gpui: u64, cx: &mut App) {
    use wry::WebViewExtUnix;
    let (tx, mut rx) = unbounded::<Keystroke>();
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(keystroke) = rx.next().await {
            let _ = handle.update(cx, |_, window, cx| {
                let before = window.focused(cx);
                window.dispatch_keystroke(keystroke, cx);
                // After the action's own deferred work.
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        if window.focused(cx) != before {
                            super::focus(gpui);
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
    // WebKit hands a key the page didn't handle on to the web view's parent.
    if let Some(parent) = view.parent() {
        parent.connect_key_press_event(move |_, event| match keystroke(event) {
            Some(k) if k.modifiers.control => send(&tx, k),
            _ => Propagation::Proceed,
        });
    }
}
