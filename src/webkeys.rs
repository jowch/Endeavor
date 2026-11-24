use std::sync::OnceLock;

use objc2::ffi::class_replaceMethod;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{msg_send, sel};

type KeyEquivalent = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> Bool;

static GPUI_HANDLER: OnceLock<KeyEquivalent> = OnceLock::new();

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
        let Some(method) = AnyClass::get(c"WKWebView").and_then(|c| c.instance_method(cmd)) else { return Bool::NO };
        let webkit: KeyEquivalent = std::mem::transmute(method.implementation());
        webkit(this, cmd, event)
    }
}

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

pub fn allow_pinch_zoom(webview: &wry::WebView) {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    unsafe {
        let _: () = msg_send![&*view, setAllowsMagnification: true];
    }
}

pub fn no_new_windows() {
    type WillOpenMenu = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject);
    const DEAD: [&str; 3] = [
        "WKMenuItemIdentifierOpenImageInNewWindow",
        "WKMenuItemIdentifierOpenMediaInNewWindow",
        "WKMenuItemIdentifierOpenFrameInNewWindow",
    ];
    unsafe extern "C-unwind" fn will_open_menu(this: *mut AnyObject, cmd: Sel, menu: *mut AnyObject, event: *mut AnyObject) {
        unsafe {
            if let Some(method) = AnyClass::get(c"WKWebView").and_then(|c| c.instance_method(cmd)) {
                let webkit: WillOpenMenu = std::mem::transmute(method.implementation());
                webkit(this, cmd, menu, event);
            }
            if menu.is_null() {
                return;
            }
            let items: *mut AnyObject = msg_send![menu, itemArray];
            let count: usize = msg_send![items, count];
            for i in 0..count {
                let item: *mut AnyObject = msg_send![items, objectAtIndex: i];
                let id = crate::dialogs::string_of(msg_send![item, identifier]);
                if DEAD.contains(&id.as_str()) {
                    let _: () = msg_send![menu, removeItem: item];
                }
            }
        }
    }
    let Some(wry) = AnyClass::get(c"WryWebView") else { return };
    unsafe {
        let ours: Imp = std::mem::transmute(will_open_menu as WillOpenMenu);
        objc2::ffi::class_addMethod(wry as *const AnyClass as *mut AnyClass, sel!(willOpenMenu:withEvent:), ours, c"v@:@@".as_ptr());
    }
}
