//! Linux's `src/webcontent.rs`: WebKitGTK's web process ending, its find
//! controller, and whether the web view has the keyboard.

use std::cell::RefCell;
use std::collections::VecDeque;

use futures::channel::oneshot;
use webkit2gtk::{FindControllerExt, FindOptions, WebViewExt};

/// How many matches WebKitGTK looks for; the bar doesn't show the count.
const MAX_MATCHES: u32 = 1000;

#[derive(Default)]
struct Finding {
    /// The text last searched, so the same text goes to the next match.
    text: Option<String>,
    /// Searches waiting for the find controller's answer, oldest first: it
    /// answers each search once, in order, and the answer doesn't say which.
    answers: VecDeque<oneshot::Sender<bool>>,
    connected: bool,
}

thread_local! {
    static FINDING: RefCell<Finding> = RefCell::new(Finding::default());
}

fn answer(found: bool) {
    if let Some(tx) = FINDING.with(|f| f.borrow_mut().answers.pop_front()) {
        let _ = tx.send(found);
    }
}

pub fn on_process_ended(webview: &wry::WebView, ended: impl Fn() + 'static) {
    use wry::WebViewExtUnix;
    webview.webview().connect_web_process_terminated(move |_, reason| {
        eprintln!("WebKitGTK's web process ended: {reason:?}");
        ended();
    });
}

pub fn url(webview: &wry::WebView) -> String {
    webview.url().unwrap_or_default()
}

pub fn find(webview: &wry::WebView, text: &str, backwards: bool) -> oneshot::Receiver<bool> {
    use wry::WebViewExtUnix;
    let (tx, rx) = oneshot::channel();
    let Some(controller) = webview.webview().find_controller() else {
        let _ = tx.send(false);
        return rx;
    };
    let same = FINDING.with(|f| {
        let mut f = f.borrow_mut();
        if !f.connected {
            f.connected = true;
            controller.connect_found_text(|_, _| answer(true));
            controller.connect_failed_to_find_text(|_| answer(false));
        }
        f.answers.push_back(tx);
        let same = f.text.as_deref() == Some(text);
        f.text = Some(text.to_owned());
        same
    });
    match (same, backwards) {
        (true, false) => controller.search_next(),
        (true, true) => controller.search_previous(),
        (false, _) => {
            let mut options = FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND;
            if backwards {
                options |= FindOptions::BACKWARDS;
            }
            controller.search(text, options.bits(), MAX_MATCHES);
        }
    }
    rx
}

pub fn clear_find(webview: &wry::WebView) {
    use wry::WebViewExtUnix;
    // A search finish cuts short may never answer: its waiters go.
    FINDING.with(|f| {
        let mut f = f.borrow_mut();
        f.text = None;
        f.answers.clear();
    });
    if let Some(controller) = webview.webview().find_controller() {
        controller.search_finish();
    }
}

/// Give the web view the keyboard: X input focus to WebKit's own window.
pub fn give_keyboard(webview: &wry::WebView) {
    use gtk::prelude::WidgetExt;
    use wry::WebViewExtUnix;
    if let Some(xid) = webview.webview().window().as_ref().and_then(super::xid_of) {
        super::focus(xid);
    }
}

pub fn has_keyboard(webview: &wry::WebView) -> bool {
    use gtk::prelude::WidgetExt;
    use wry::WebViewExtUnix;
    webview.webview().window().as_ref().and_then(super::xid_of).is_some_and(super::focused)
}
