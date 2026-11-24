use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use futures::channel::oneshot;
use objc2::ffi::{class_addMethod, object_getClass};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{class, msg_send, sel};

thread_local! {
    static ENDED: RefCell<Vec<(usize, Rc<dyn Fn()>)>> = const { RefCell::new(Vec::new()) };
    static FOUND: RefCell<Option<String>> = const { RefCell::new(None) };
}

unsafe extern "C-unwind" fn process_did_terminate(_: *mut AnyObject, _: Sel, view: *mut AnyObject) {
    let ended = ENDED.with(|ended| ended.borrow().iter().find(|(v, _)| *v == view as usize).map(|(_, f)| f.clone()));
    match ended {
        Some(ended) => ended(),
        None => eprintln!("The web content process of a web view the app doesn't know ended"),
    }
}

fn key(webview: &wry::WebView) -> usize {
    use wry::WebViewExtMacOS;
    Retained::as_ptr(&webview.webview()) as usize
}

pub fn on_process_ended(webview: &wry::WebView, ended: impl Fn() + 'static) {
    use wry::WebViewExtMacOS;
    let id = key(webview);
    let ended: Rc<dyn Fn()> = Rc::new(ended);
    ENDED.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.retain(|(v, _)| *v != id);
        slot.push((id, ended));
    });
    let view = webview.webview();
    unsafe {
        let delegate: *mut AnyObject = msg_send![&*view, navigationDelegate];
        if delegate.is_null() {
            eprintln!("The notebook's web view has no navigation delegate; a crash of its page won't be noticed");
            return;
        }
        let class = object_getClass(delegate) as *mut AnyClass;
        let imp: Imp = std::mem::transmute(process_did_terminate as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject));
        let _ = class_addMethod(class, sel!(webViewWebContentProcessDidTerminate:), imp, c"v@:@".as_ptr());
        let _: () = msg_send![&*view, setNavigationDelegate: delegate];
    }
}

pub fn forget(webview: &wry::WebView) {
    let id = key(webview);
    ENDED.with(|slot| slot.borrow_mut().retain(|(v, _)| *v != id));
}

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

pub fn url(webview: &wry::WebView) -> String {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    let url: *mut AnyObject = unsafe { msg_send![&*view, URL] };
    if url.is_null() { String::new() } else { webview.url().unwrap_or_default() }
}

fn clear_selection(webview: &wry::WebView) {
    let _ = webview.evaluate_script("window.getSelection()?.removeAllRanges()");
}

pub fn clear_find(webview: &wry::WebView) {
    FOUND.with(|f| f.take());
    drop(search(webview, "\u{FDD0}", false));
    clear_selection(webview);
}

pub fn give_keyboard(webview: &wry::WebView) {
    let _ = webview.focus();
}

pub fn had_keyboard(webview: &wry::WebView) -> bool {
    has_keyboard(webview)
}

pub fn edit(_: &wry::WebView, _: gpui::OsAction) {}

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
