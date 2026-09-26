//! Keys in the notebook's web view. WebKit handles arrow keys (and other function
//! keys) as key equivalents, but two things keep them from it: GPUI's root view
//! answers every key equivalent itself without asking its subviews, and wry makes
//! a child web view refuse them all (so menu shortcuts fire). Refused, arrow keys
//! fall through to text input and type private-use glyphs, breaking CodeMirror's
//! cursor keys. So: while the keyboard is in the web view, GPUI's view first
//! offers the key to it; the web view gives the app's menu first pick (⌘C, ⌘Q,
//! ⌘B, …) and hands the rest to WebKit's own handling.

use std::sync::OnceLock;

use objc2::ffi::class_replaceMethod;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{msg_send, sel};

type KeyEquivalent = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool;

/// GPUI's own handler, called for keys the web view doesn't take.
static GPUI_HANDLER: OnceLock<KeyEquivalent> = OnceLock::new();

/// Is the window's keyboard focus in a web view? Returns that view.
unsafe fn focused_web_view(view: *mut AnyObject) -> Option<*mut AnyObject> {
    unsafe {
        let webkit = AnyClass::get(c"WKWebView")?;
        let window: *mut AnyObject = msg_send![view, window];
        if window.is_null() {
            return None;
        }
        let mut responder: *mut AnyObject = msg_send![window, firstResponder];
        while !responder.is_null() {
            let found: Bool = msg_send![responder, isKindOfClass: webkit];
            if found.as_bool() {
                return Some(responder);
            }
            let is_view: Bool = msg_send![responder, respondsToSelector: sel!(superview)];
            if !is_view.as_bool() {
                return None;
            }
            responder = msg_send![responder, superview];
        }
        None
    }
}

unsafe extern "C-unwind" fn gpui_view_key_equivalent(this: *mut AnyObject, cmd: Sel, event: *mut AnyObject) -> Bool {
    unsafe {
        if let Some(web_view) = focused_web_view(this) {
            let taken: Bool = msg_send![web_view, performKeyEquivalent: event];
            if taken.as_bool() {
                return Bool::YES;
            }
        }
        match GPUI_HANDLER.get() {
            Some(gpui) => gpui(this, cmd, event),
            None => Bool::NO,
        }
    }
}

unsafe extern "C-unwind" fn web_view_key_equivalent(this: *mut AnyObject, cmd: Sel, event: *mut AnyObject) -> Bool {
    unsafe {
        let app: *mut AnyObject = msg_send![AnyClass::get(c"NSApplication").expect("AppKit"), sharedApplication];
        let menu: *mut AnyObject = msg_send![app, mainMenu];
        if !menu.is_null() {
            let taken: Bool = msg_send![menu, performKeyEquivalent: event];
            if taken.as_bool() {
                return Bool::YES;
            }
        }
        // WKWebView's own implementation (what wry's override skips).
        let Some(method) = AnyClass::get(c"WKWebView").and_then(|c| c.instance_method(cmd)) else { return Bool::NO };
        let webkit: KeyEquivalent = std::mem::transmute(method.implementation());
        webkit(this, cmd, event)
    }
}

/// Call once the web view exists (wry registers its class then). Idempotent.
pub fn fix_key_handling() {
    let (Some(gpui), Some(wry)) = (AnyClass::get(c"GPUIView"), AnyClass::get(c"WryWebView")) else { return };
    if GPUI_HANDLER.get().is_some() {
        return;
    }
    let sel = sel!(performKeyEquivalent:);
    unsafe {
        let ours: Imp = std::mem::transmute(gpui_view_key_equivalent as KeyEquivalent);
        if let Some(previous) = class_replaceMethod(gpui as *const AnyClass as *mut AnyClass, sel, ours, c"B@:@".as_ptr()) {
            let _ = GPUI_HANDLER.set(std::mem::transmute::<Imp, KeyEquivalent>(previous));
        }
        let ours: Imp = std::mem::transmute(web_view_key_equivalent as KeyEquivalent);
        class_replaceMethod(wry as *const AnyClass as *mut AnyClass, sel, ours, c"B@:@".as_ptr());
    }
}

/// Trackpad pinch zooms the notebook (WebKit's magnification, separate from ⌘=).
pub fn allow_pinch_zoom(webview: &wry::WebView) {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    unsafe {
        let _: () = msg_send![&*view, setAllowsMagnification: true];
    }
}
