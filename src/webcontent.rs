//! The notebook's web view beyond what wry offers: hearing that WebKit's web
//! content process (the one running the page) ended, WebKit's own find in the
//! page, and whether the web view has the keyboard.

use std::cell::{Cell, RefCell};

use block2::RcBlock;
use futures::channel::oneshot;
use objc2::ffi::{class_addMethod, object_getClass};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{class, msg_send, sel};

thread_local! {
    static ENDED: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None);
    /// The text last found, so the same text goes on from its match.
    static FOUND: RefCell<Option<String>> = const { RefCell::new(None) };
}

unsafe extern "C-unwind" fn process_did_terminate(_: *mut AnyObject, _: Sel, _: *mut AnyObject) {
    ENDED.with(|ended| {
        if let Some(ended) = ended.borrow().as_ref() {
            ended();
        }
    });
}

/// Call `ended` when the page's web content process ends (a crash, or macOS
/// ending it for memory), which leaves the web view blank. Having this handler
/// also stops WebKit reloading the page by itself; `ended` does that.
///
/// wry's navigation delegate has no such method, so it's added to the
/// delegate's class, and the delegate set again: WebKit reads which methods a
/// delegate has when it's set.
pub fn on_process_ended(webview: &wry::WebView, ended: impl Fn() + 'static) {
    use wry::WebViewExtMacOS;
    ENDED.with(|slot| *slot.borrow_mut() = Some(Box::new(ended)));
    let view = webview.webview();
    unsafe {
        let delegate: *mut AnyObject = msg_send![&*view, navigationDelegate];
        if delegate.is_null() {
            eprintln!("The notebook's web view has no navigation delegate; a crash of its page won't be noticed");
            return;
        }
        let class = object_getClass(delegate) as *mut AnyClass;
        let imp: Imp = std::mem::transmute(process_did_terminate as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject));
        // NO when the class has it already (a second web view): the handler above is shared.
        let _ = class_addMethod(class, sel!(webViewWebContentProcessDidTerminate:), imp, c"v@:@".as_ptr());
        let _: () = msg_send![&*view, setNavigationDelegate: delegate];
    }
}

/// Find `text` in the page with WebKit's find (macOS 13+): it selects the
/// match and scrolls to it. Searching the same text again goes on from the
/// match; a new text starts from the top. The receiver gets whether there was
/// a match. WebKit gives no count.
pub fn find(webview: &wry::WebView, text: &str, backwards: bool) -> oneshot::Receiver<bool> {
    let new = FOUND.with(|f| f.replace(Some(text.to_owned())).as_deref() != Some(text));
    if new {
        clear_selection(webview);
    }
    search(webview, text, backwards)
}

fn search(webview: &wry::WebView, text: &str, backwards: bool) -> oneshot::Receiver<bool> {
    use wry::WebViewExtMacOS;
    let (tx, rx) = oneshot::channel();
    let Ok(text) = std::ffi::CString::new(text) else { return rx };
    let view = webview.webview();
    let tx = Cell::new(Some(tx));
    unsafe {
        let supported: Bool = msg_send![&*view, respondsToSelector: sel!(findString:withConfiguration:completionHandler:)];
        let Some(config): Option<Retained<AnyObject>> = supported.as_bool().then(|| msg_send![class!(WKFindConfiguration), new]).flatten() else {
            let _ = tx.take().map(|tx| tx.send(false));
            return rx;
        };
        let _: () = msg_send![&*config, setBackwards: Bool::new(backwards)];
        let _: () = msg_send![&*config, setCaseSensitive: Bool::NO];
        let _: () = msg_send![&*config, setWraps: Bool::YES];
        let string: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()];
        let handler = RcBlock::new(move |result: *mut AnyObject| {
            let found = !result.is_null() && {
                let found: Bool = msg_send![result, matchFound];
                found.as_bool()
            };
            if let Some(tx) = tx.take() {
                let _ = tx.send(found);
            }
        });
        let _: () = msg_send![&*view, findString: string, withConfiguration: &*config, completionHandler: &*handler];
    }
    rx
}

/// The page's address, or "" when WebKit has none: after the web content
/// process ended, until the next load. wry's `url()` panics then.
pub fn url(webview: &wry::WebView) -> String {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    let url: *mut AnyObject = unsafe { msg_send![&*view, URL] };
    if url.is_null() { String::new() } else { webview.url().unwrap_or_default() }
}

/// With no selection, WebKit's next find starts from the top.
fn clear_selection(webview: &wry::WebView) {
    let _ = webview.evaluate_script("window.getSelection()?.removeAllRanges()");
}

/// Take away find's highlight and selection. WebKit has no public call to
/// hide its highlight, but hides it when a search finds nothing: this
/// searches for a character no page has.
pub fn clear_find(webview: &wry::WebView) {
    FOUND.with(|f| f.take());
    drop(search(webview, "\u{FDD0}", false));
    clear_selection(webview);
}

/// Give the web view the keyboard.
pub fn give_keyboard(webview: &wry::WebView) {
    let _ = webview.focus();
}

/// The keyboard is in the web view: it, or a view inside it, is its window's first responder.
pub fn has_keyboard(webview: &wry::WebView) -> bool {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    let target = Retained::as_ptr(&view) as *const AnyObject;
    unsafe {
        let window: *mut AnyObject = msg_send![&*view, window];
        if window.is_null() {
            return false;
        }
        let mut responder: *mut AnyObject = msg_send![window, firstResponder];
        while !responder.is_null() {
            if std::ptr::eq(responder, target) {
                return true;
            }
            let is_view: Bool = msg_send![responder, isKindOfClass: class!(NSView)];
            if !is_view.as_bool() {
                return false;
            }
            responder = msg_send![responder, superview];
        }
        false
    }
}
