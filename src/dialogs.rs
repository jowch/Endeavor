//! The notebook page's alert(), confirm() and prompt(). WebKit shows them only
//! through its UI delegate, and wry's delegate doesn't implement them, so they
//! did nothing (confirm() answered false, prompt() null): Pluto's "Delete these
//! cells?" and its export warnings among them. They show as sheets on the window.

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char};

use block2::{Block, RcBlock};
use objc2::ffi::class_addMethod;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{class, msg_send, sel};

use crate::overlay::CGRect;

type Alert = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject, *mut Block<dyn Fn()>);
type Confirm = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject, *mut Block<dyn Fn(Bool)>);
type Prompt =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject, *mut AnyObject, *mut Block<dyn Fn(*mut AnyObject)>);

/// NSAlertFirstButtonReturn.
const FIRST_BUTTON: isize = 1000;

unsafe fn string_of(ns: *mut AnyObject) -> String {
    if ns.is_null() {
        return String::new();
    }
    unsafe {
        let utf8: *const c_char = msg_send![ns, UTF8String];
        if utf8.is_null() { String::new() } else { CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}

unsafe fn ns_string(text: &str) -> Retained<AnyObject> {
    let text = CString::new(text.replace('\0', "")).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

/// An alert with the page's message (its first line as the heading) and `buttons`, the first the default.
unsafe fn alert(message: *mut AnyObject, buttons: &[&str]) -> Retained<AnyObject> {
    unsafe {
        let text = string_of(message);
        let (heading, rest) = text.trim().split_once('\n').unwrap_or((text.trim(), ""));
        let alert: Retained<AnyObject> = msg_send![class!(NSAlert), new];
        let _: () = msg_send![&*alert, setMessageText: &*ns_string(heading.trim())];
        let _: () = msg_send![&*alert, setInformativeText: &*ns_string(rest.trim())];
        for button in buttons {
            let _: *mut AnyObject = msg_send![&*alert, addButtonWithTitle: &*ns_string(button)];
        }
        alert
    }
}

/// Show `alert` as a sheet on the web view's window (or on its own when there's
/// none), then call `answer` with whether its first button was chosen.
unsafe fn show(web_view: *mut AnyObject, alert: Retained<AnyObject>, answer: impl Fn(bool) + 'static) {
    unsafe {
        let window: *mut AnyObject = msg_send![web_view, window];
        if window.is_null() {
            let response: isize = msg_send![&*alert, runModal];
            return answer(response == FIRST_BUTTON);
        }
        let handler = RcBlock::new(move |response: isize| answer(response == FIRST_BUTTON));
        let _: () = msg_send![&*alert, beginSheetModalForWindow: window, completionHandler: &*handler];
    }
}

/// WebKit's completion handler, to call once, later.
unsafe fn keep<F: ?Sized>(handler: *mut Block<F>) -> Cell<Option<RcBlock<F>>> {
    Cell::new(unsafe { RcBlock::copy(handler) })
}

unsafe extern "C-unwind" fn run_alert(_: *mut AnyObject, _: Sel, web_view: *mut AnyObject, message: *mut AnyObject, _: *mut AnyObject, done: *mut Block<dyn Fn()>) {
    unsafe {
        let done = keep(done);
        show(web_view, alert(message, &["OK"]), move |_| {
            if let Some(done) = done.take() {
                done.call(());
            }
        });
    }
}

unsafe extern "C-unwind" fn run_confirm(_: *mut AnyObject, _: Sel, web_view: *mut AnyObject, message: *mut AnyObject, _: *mut AnyObject, done: *mut Block<dyn Fn(Bool)>) {
    unsafe {
        let done = keep(done);
        show(web_view, alert(message, &["OK", "Cancel"]), move |ok| {
            if let Some(done) = done.take() {
                done.call((Bool::new(ok),));
            }
        });
    }
}

unsafe extern "C-unwind" fn run_prompt(
    _: *mut AnyObject,
    _: Sel,
    web_view: *mut AnyObject,
    message: *mut AnyObject,
    default: *mut AnyObject,
    _: *mut AnyObject,
    done: *mut Block<dyn Fn(*mut AnyObject)>,
) {
    unsafe {
        let done = keep(done);
        let alert = alert(message, &["OK", "Cancel"]);
        let field: Retained<AnyObject> = msg_send![class!(NSTextField), textFieldWithString: &*ns_string(&string_of(default))];
        let _: () = msg_send![&*field, setFrame: CGRect::new(0., 0., 300., 24.)];
        let _: () = msg_send![&*alert, setAccessoryView: &*field];
        let window: *mut AnyObject = msg_send![&*alert, window];
        let _: () = msg_send![window, setInitialFirstResponder: &*field];
        let typed = field.clone();
        show(web_view, alert, move |ok| {
            if let Some(done) = done.take() {
                let text: *mut AnyObject = if ok { msg_send![&*typed, stringValue] } else { std::ptr::null_mut() };
                done.call((text,));
            }
        });
    }
}

/// Answer the page's dialogs. Call once the web view exists (wry registers its
/// delegate class then). Idempotent.
pub fn show_page_dialogs(webview: &wry::WebView) {
    use wry::WebViewExtMacOS;
    let Some(class) = AnyClass::get(c"WryWebViewUIDelegate") else { return };
    let alert_sel = sel!(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:);
    if class.instance_method(alert_sel).is_some() {
        return;
    }
    let class = class as *const AnyClass as *mut AnyClass;
    unsafe {
        let methods: [(Sel, Imp, &CStr); 3] = [
            (alert_sel, std::mem::transmute::<Alert, Imp>(run_alert), c"v@:@@@@?"),
            (
                sel!(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:),
                std::mem::transmute::<Confirm, Imp>(run_confirm),
                c"v@:@@@@?",
            ),
            (
                sel!(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:),
                std::mem::transmute::<Prompt, Imp>(run_prompt),
                c"v@:@@@@@?",
            ),
        ];
        for (sel, imp, types) in methods {
            class_addMethod(class, sel, imp, types.as_ptr());
        }
        // WebKit reads which methods its delegate has when the delegate is set.
        let view = webview.webview();
        let delegate: *mut AnyObject = msg_send![&*view, UIDelegate];
        let _: () = msg_send![&*view, setUIDelegate: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![&*view, setUIDelegate: delegate];
    }
}
