//! The notebook's web view beyond what wry offers: hearing that WebKit's web
//! content process (the one running the page) ended, and the page's address,
//! which WebKit drops when that happens.

use std::cell::RefCell;

use objc2::ffi::{class_addMethod, object_getClass};
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{msg_send, sel};

thread_local! {
    static ENDED: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None);
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

/// The page's address, or "" when WebKit has none: after the web content
/// process ended, until the next load. wry's `url()` panics then.
pub fn url(webview: &wry::WebView) -> String {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    let url: *mut AnyObject = unsafe { msg_send![&*view, URL] };
    if url.is_null() { String::new() } else { webview.url().unwrap_or_default() }
}
