//! A GPUI menu over the notebook. The notebook's web view is a native view on top
//! of everything GPUI draws, so a menu drawn there would be hidden and its clicks
//! would go to the page. While one is open, the web view gets a hole where the
//! menu is: its layer is masked there, and clicks there go to GPUI.

use std::sync::{Mutex, OnceLock};

use gpui::{Bounds, Pixels};
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::ffi::class_replaceMethod;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{class, msg_send, sel};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

#[repr(C)]
struct CGColor {
    _private: [u8; 0],
}
unsafe impl RefEncode for CGColor {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[]));
}

impl CGRect {
    pub(crate) fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        CGRect { origin: CGPoint { x, y }, size: CGSize { width: width.max(0.), height: height.max(0.) } }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.origin.x && x < self.origin.x + self.size.width && y >= self.origin.y && y < self.origin.y + self.size.height
    }
}

/// The open hole, in the web view's coordinates from its top-left corner, and
/// the web view (an address: it lives as long as the app).
#[derive(Clone, Copy)]
struct Hole {
    rect: CGRect,
    view: usize,
}

static HOLE: Mutex<Option<Hole>> = Mutex::new(None);

type SendEvent = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
static GPUI_SEND_EVENT: OnceLock<SendEvent> = OnceLock::new();

// NSEventType values.
const LEFT_DOWN: usize = 1;
const LEFT_UP: usize = 2;
const RIGHT_DOWN: usize = 3;
const RIGHT_UP: usize = 4;
const LEFT_DRAGGED: usize = 6;

/// Clicks in the hole go straight to GPUI's view. The web view claims them
/// otherwise, even when it declines to hit-test there (WebKit's gesture
/// recognizers take mouse-downs over the whole view).
unsafe extern "C-unwind" fn send_event(window: *mut AnyObject, cmd: Sel, event: *mut AnyObject) {
    unsafe {
        let hole = *HOLE.lock().unwrap();
        let kind: usize = msg_send![event, type];
        if let Some(hole) = hole
            && [LEFT_DOWN, LEFT_UP, RIGHT_DOWN, RIGHT_UP, LEFT_DRAGGED].contains(&kind)
        {
            let view = hole.view as *mut AnyObject;
            let in_window: CGPoint = msg_send![event, locationInWindow];
            let local: CGPoint = msg_send![view, convertPoint: in_window, fromView: std::ptr::null_mut::<AnyObject>()];
            let flipped: Bool = msg_send![view, isFlipped];
            let bounds: CGRect = msg_send![view, bounds];
            let y = if flipped.as_bool() { local.y } else { bounds.size.height - local.y };
            if hole.rect.contains(local.x, y) {
                let gpui: *mut AnyObject = msg_send![view, superview];
                match kind {
                    LEFT_DOWN => msg_send![gpui, mouseDown: event],
                    LEFT_UP => msg_send![gpui, mouseUp: event],
                    RIGHT_DOWN => msg_send![gpui, rightMouseDown: event],
                    RIGHT_UP => msg_send![gpui, rightMouseUp: event],
                    _ => msg_send![gpui, mouseDragged: event],
                }
                return;
            }
        }
        if let Some(gpui) = GPUI_SEND_EVENT.get() {
            gpui(window, cmd, event);
        }
    }
}

/// Route GPUI's window events through `send_event`. Idempotent.
fn install_send_event() {
    if GPUI_SEND_EVENT.get().is_some() {
        return;
    }
    let Some(window) = AnyClass::get(c"GPUIWindow") else { return };
    let sel = sel!(sendEvent:);
    let Some(method) = window.instance_method(sel) else { return };
    unsafe {
        let _ = GPUI_SEND_EVENT.set(std::mem::transmute::<Imp, SendEvent>(method.implementation()));
        let ours: Imp = std::mem::transmute(send_event as SendEvent);
        class_replaceMethod(window as *const AnyClass as *mut AnyClass, sel, ours, c"v@:@".as_ptr());
    }
}

/// Everything in `bounds` but `hole` (both from the top-left), as a mask layer.
unsafe fn mask_around(bounds: CGRect, hole: CGRect, flipped: bool) -> *mut AnyObject {
    unsafe {
        let (w, h) = (bounds.size.width, bounds.size.height);
        let (x, y, hw, hh) = (hole.origin.x, hole.origin.y, hole.size.width, hole.size.height);
        let around = [
            CGRect::new(0., 0., w, y),
            CGRect::new(0., y + hh, w, h - y - hh),
            CGRect::new(0., y, x, hh),
            CGRect::new(x + hw, y, w - x - hw, hh),
        ];
        let mask: *mut AnyObject = msg_send![class!(CALayer), layer];
        let _: () = msg_send![mask, setFrame: bounds];
        let black: *mut AnyObject = msg_send![class!(NSColor), blackColor];
        let black: *const CGColor = msg_send![black, CGColor];
        for mut rect in around {
            if !flipped {
                rect.origin.y = h - rect.origin.y - rect.size.height;
            }
            let part: *mut AnyObject = msg_send![class!(CALayer), layer];
            let _: () = msg_send![part, setFrame: rect];
            let _: () = msg_send![part, setBackgroundColor: black];
            let _: () = msg_send![mask, addSublayer: part];
        }
        mask
    }
}

/// Cut a hole in the web view at `hole` (relative to the web view's top-left
/// corner), or close it with None.
pub fn set_hole(webview: &wry::WebView, hole: Option<Bounds<Pixels>>) {
    use wry::WebViewExtMacOS;
    let rect = hole.map(|b| CGRect::new(f64::from(b.origin.x), f64::from(b.origin.y), f64::from(b.size.width), f64::from(b.size.height)));
    let view = webview.webview();
    let view: *mut AnyObject = &*view as *const _ as *mut AnyObject;
    {
        let mut current = HOLE.lock().unwrap();
        if current.map(|hole| hole.rect) == rect {
            return;
        }
        *current = rect.map(|rect| Hole { rect, view: view as usize });
    }
    install_send_event();
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        if layer.is_null() {
            return;
        }
        let _: () = msg_send![class!(CATransaction), begin];
        let _: () = msg_send![class!(CATransaction), setDisableActions: true];
        let mask = match rect {
            None => std::ptr::null_mut(),
            Some(rect) => {
                let bounds: CGRect = msg_send![layer, bounds];
                let flipped: Bool = msg_send![layer, isGeometryFlipped];
                mask_around(bounds, rect, flipped.as_bool())
            }
        };
        let _: () = msg_send![layer, setMask: mask];
        let _: () = msg_send![class!(CATransaction), commit];
    }
}

/// Close the hole if it's still the one at `hole` (a tooltip's, say, and not
/// a menu's opened since).
pub fn close_hole_at(hole: Bounds<Pixels>) {
    let rect = CGRect::new(f64::from(hole.origin.x), f64::from(hole.origin.y), f64::from(hole.size.width), f64::from(hole.size.height));
    let Some(open) = *HOLE.lock().unwrap() else { return };
    if open.rect != rect {
        return;
    }
    *HOLE.lock().unwrap() = None;
    unsafe {
        let layer: *mut AnyObject = msg_send![open.view as *mut AnyObject, layer];
        if !layer.is_null() {
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
            let _: () = msg_send![layer, setMask: std::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![class!(CATransaction), commit];
        }
    }
}
