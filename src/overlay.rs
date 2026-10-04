//! A GPUI menu over the notebook. The notebook's web view is a native view on top
//! of everything GPUI draws, so a menu drawn there would be hidden and its clicks
//! would go to the page. While one is open, the web view gets a hole where the
//! menu is: its layer is masked there, and clicks there go to GPUI. A menu, a
//! tip and a tooltip can each have a hole at the same time.

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

#[repr(C)]
struct CGPath {
    _private: [u8; 0],
}
unsafe impl RefEncode for CGPath {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPathCreateMutable() -> *mut CGPath;
    fn CGPathMoveToPoint(path: *mut CGPath, transform: *const std::ffi::c_void, x: f64, y: f64);
    fn CGPathAddLineToPoint(path: *mut CGPath, transform: *const std::ffi::c_void, x: f64, y: f64);
    fn CGPathCloseSubpath(path: *mut CGPath);
    fn CGPathRelease(path: *mut CGPath);
}

impl CGRect {
    fn overlaps(&self, other: &CGRect) -> bool {
        self.origin.x < other.origin.x + other.size.width
            && other.origin.x < self.origin.x + self.size.width
            && self.origin.y < other.origin.y + other.size.height
            && other.origin.y < self.origin.y + self.size.height
    }

    pub(crate) fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        CGRect { origin: CGPoint { x, y }, size: CGSize { width: width.max(0.), height: height.max(0.) } }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.origin.x && x < self.origin.x + self.size.width && y >= self.origin.y && y < self.origin.y + self.size.height
    }
}

/// What a hole in the web view is for; each has at most one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hole {
    Menu,
    Tip,
    Tooltip,
    /// The Settings panel.
    Settings,
    /// The confirm dialog.
    Confirm,
    /// A one-off failure's notice.
    Notice,
}

impl Hole {
    /// The corner radius of what the hole is cut for, so the notebook shows
    /// right up to its rounded corners.
    fn radius(self) -> f64 {
        match self {
            Hole::Menu | Hole::Tip => 8.,
            Hole::Tooltip => 5.,
            Hole::Notice => 10.,
            Hole::Confirm => 12.,
            Hole::Settings => 14.,
        }
    }
}

/// The open holes, in the web view's coordinates from its top-left corner, and
/// the web view (an address: it lives as long as the app).
struct Holes {
    rects: Vec<(Hole, CGRect)>,
    view: usize,
    /// A menu or popover is open somewhere (not necessarily over the web
    /// view): a mouse-down anywhere on the web view, not only in a hole,
    /// goes to GPUI too, so its own click-outside handling can close it.
    dismiss: bool,
}

static HOLES: Mutex<Holes> = Mutex::new(Holes { rects: Vec::new(), view: 0, dismiss: false });

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
/// recognizers take mouse-downs over the whole view). While `dismiss` holds,
/// a mouse-down anywhere on the web view goes to GPUI the same way, so a menu
/// or popover open elsewhere in the window hears it and closes, the way a
/// native menu closes for a click anywhere outside it.
unsafe extern "C-unwind" fn send_event(window: *mut AnyObject, cmd: Sel, event: *mut AnyObject) {
    unsafe {
        let (rects, view, dismiss) = {
            let holes = HOLES.lock().unwrap();
            (holes.rects.iter().map(|(_, rect)| *rect).collect::<Vec<_>>(), holes.view, holes.dismiss)
        };
        let kind: usize = msg_send![event, type];
        let is_click = [LEFT_DOWN, LEFT_UP, RIGHT_DOWN, RIGHT_UP, LEFT_DRAGGED].contains(&kind);
        if view != 0 && is_click && (!rects.is_empty() || dismiss) {
            let view = view as *mut AnyObject;
            let in_window: CGPoint = msg_send![event, locationInWindow];
            let local: CGPoint = msg_send![view, convertPoint: in_window, fromView: std::ptr::null_mut::<AnyObject>()];
            let flipped: Bool = msg_send![view, isFlipped];
            let bounds: CGRect = msg_send![view, bounds];
            let y = if flipped.as_bool() { local.y } else { bounds.size.height - local.y };
            let in_hole = rects.iter().any(|rect| rect.contains(local.x, y));
            let in_view = local.x >= 0. && local.x < bounds.size.width && y >= 0. && y < bounds.size.height;
            let dismiss_click = dismiss && matches!(kind, LEFT_DOWN | RIGHT_DOWN) && in_view;
            if in_hole || dismiss_click {
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

/// Everything in a `w` by `h` area but `holes` (all from the top-left), as
/// rectangles: the cells of the grid their edges make that no hole covers.
fn around(w: f64, h: f64, holes: &[CGRect]) -> Vec<CGRect> {
    let edges = |ends: fn(&CGRect) -> [f64; 2], max: f64| {
        let mut at: Vec<f64> = holes.iter().flat_map(ends).map(|v| v.clamp(0., max)).chain([0., max]).collect();
        at.sort_by(f64::total_cmp);
        at.dedup();
        at
    };
    let xs = edges(|r| [r.origin.x, r.origin.x + r.size.width], w);
    let ys = edges(|r| [r.origin.y, r.origin.y + r.size.height], h);
    let mut cells = Vec::new();
    for x in xs.windows(2) {
        for y in ys.windows(2) {
            let (mid_x, mid_y) = ((x[0] + x[1]) / 2., (y[0] + y[1]) / 2.);
            if !holes.iter().any(|hole| hole.contains(mid_x, mid_y)) {
                cells.push(CGRect::new(x[0], y[0], x[1] - x[0], y[1] - y[0]));
            }
        }
    }
    cells
}

/// Everything in `bounds` but `holes` (all from the top-left, each with its
/// corner radius), as a mask layer.
unsafe fn mask_around(bounds: CGRect, holes: &[(CGRect, f64)], flipped: bool) -> *mut AnyObject {
    unsafe {
        let (w, h) = (bounds.size.width, bounds.size.height);
        let rects: Vec<CGRect> = holes.iter().map(|(rect, _)| *rect).collect();
        let around = around(w, h, &rects);
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
        let slivers = corner_slivers(holes);
        if !slivers.is_empty() {
            let path = CGPathCreateMutable();
            for sliver in &slivers {
                for (i, &(x, y)) in sliver.iter().enumerate() {
                    let y = if flipped { y } else { h - y };
                    if i == 0 {
                        CGPathMoveToPoint(path, std::ptr::null(), x, y);
                    } else {
                        CGPathAddLineToPoint(path, std::ptr::null(), x, y);
                    }
                }
                CGPathCloseSubpath(path);
            }
            let shape: *mut AnyObject = msg_send![class!(CAShapeLayer), layer];
            let _: () = msg_send![shape, setFrame: CGRect::new(0., 0., w, h)];
            let _: () = msg_send![shape, setFillColor: black];
            let _: () = msg_send![shape, setPath: path as *const CGPath];
            CGPathRelease(path);
            let _: () = msg_send![mask, addSublayer: shape];
        }
        mask
    }
}

/// The parts of each hole outside its rounded corners, as polygons from the
/// top-left: the web view shows there again. A corner that another hole
/// overlaps stays cut, since that hole's content is drawn there.
fn corner_slivers(holes: &[(CGRect, f64)]) -> Vec<Vec<(f64, f64)>> {
    const STEPS: usize = 8;
    let mut out = Vec::new();
    for (i, (rect, radius)) in holes.iter().enumerate() {
        let r = radius.min(rect.size.width / 2.).min(rect.size.height / 2.);
        if r <= 0. {
            continue;
        }
        let (x0, y0, x1, y1) = (rect.origin.x, rect.origin.y, rect.origin.x + rect.size.width, rect.origin.y + rect.size.height);
        for (px, py, sx, sy) in [(x0, y0, 1., 1.), (x1, y0, -1., 1.), (x0, y1, 1., -1.), (x1, y1, -1., -1.)] {
            let square = CGRect::new(px.min(px + sx * r), py.min(py + sy * r), r, r);
            let covered = holes.iter().enumerate().any(|(j, (other, _))| j != i && other.overlaps(&square));
            if covered {
                continue;
            }
            let (cx, cy) = (px + sx * r, py + sy * r);
            let mut points = vec![(px, py)];
            points.extend((0..=STEPS).map(|k| {
                let t = k as f64 / STEPS as f64 * std::f64::consts::FRAC_PI_2;
                (cx - sx * r * t.sin(), cy - sy * r * t.cos())
            }));
            out.push(points);
        }
    }
    out
}

fn cg_rect(b: Bounds<Pixels>) -> CGRect {
    CGRect::new(f64::from(b.origin.x), f64::from(b.origin.y), f64::from(b.size.width), f64::from(b.size.height))
}

/// A menu or popover open anywhere in the window, so a mouse-down on the web
/// view should reach GPUI even outside any hole: the shared click-outside
/// mechanism every menu and popover uses (`Workspace::dismissible_open`).
pub fn set_dismiss_on_click(webview: &wry::WebView, active: bool) {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    let view = &*view as *const _ as *mut AnyObject as usize;
    {
        let mut holes = HOLES.lock().unwrap();
        holes.view = view;
        holes.dismiss = active;
    }
    install_send_event();
}

/// Dim the web view as the scrim behind a panel dims the rest of the window:
/// drawn at 38% over the scrim, it matches a 62% black layer over it.
pub fn set_dimmed(webview: &wry::WebView, dimmed: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use wry::WebViewExtMacOS;
    static DIMMED: AtomicBool = AtomicBool::new(false);
    if DIMMED.swap(dimmed, Ordering::Relaxed) == dimmed {
        return;
    }
    let view = webview.webview();
    let view = &*view as *const _ as *mut AnyObject;
    let alpha: f64 = if dimmed { 0.38 } else { 1. };
    unsafe {
        let _: () = msg_send![view, setAlphaValue: alpha];
    }
}

/// Cut `owner`'s hole in the web view at `hole` (relative to the web view's
/// top-left corner), or close it with None. Other holes stay.
pub fn set_hole(webview: &wry::WebView, owner: Hole, hole: Option<Bounds<Pixels>>) {
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    let view = &*view as *const _ as *mut AnyObject as usize;
    let rects = {
        let mut holes = HOLES.lock().unwrap();
        let before = holes.rects.clone();
        holes.rects.retain(|(o, _)| *o != owner);
        holes.rects.extend(hole.map(|b| (owner, cg_rect(b))));
        if holes.rects == before {
            return;
        }
        holes.view = view;
        holes.rects.iter().map(|(owner, rect)| (*rect, owner.radius())).collect::<Vec<_>>()
    };
    install_send_event();
    unsafe { mask(view as *mut AnyObject, &rects) }
}

/// Mask the web view's layer around `holes`, or not at all when there are none.
unsafe fn mask(view: *mut AnyObject, holes: &[(CGRect, f64)]) {
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        if layer.is_null() {
            return;
        }
        let _: () = msg_send![class!(CATransaction), begin];
        let _: () = msg_send![class!(CATransaction), setDisableActions: true];
        let mask = if holes.is_empty() {
            std::ptr::null_mut()
        } else {
            let bounds: CGRect = msg_send![layer, bounds];
            let flipped: Bool = msg_send![layer, isGeometryFlipped];
            mask_around(bounds, holes, flipped.as_bool())
        };
        let _: () = msg_send![layer, setMask: mask];
        let _: () = msg_send![class!(CATransaction), commit];
    }
}

#[cfg(test)]
mod tests {
    use super::{CGRect, around, corner_slivers};

    fn area(rects: &[CGRect]) -> f64 {
        rects.iter().map(|r| r.size.width * r.size.height).sum()
    }

    #[test]
    fn the_mask_covers_everything_but_the_holes_even_where_they_overlap() {
        let tip = CGRect::new(100., 10., 400., 50.);
        let tooltip = CGRect::new(450., 0., 100., 20.);
        let visible = around(1000., 800., &[tip, tooltip]);
        // 400×50 and 100×20 less the 50×10 they share.
        assert_eq!(area(&visible), 1000. * 800. - 21_500.);
        assert!(visible.iter().all(|cell| !cell.contains(120., 30.) && !cell.contains(540., 5.)));
        assert!(visible.iter().any(|cell| cell.contains(50., 30.)));
        assert_eq!(area(&around(1000., 800., &[])), 1000. * 800.);
    }

    #[test]
    fn rounded_holes_give_the_corners_back_unless_another_hole_is_there() {
        let panel = CGRect::new(100., 100., 300., 200.);
        let slivers = corner_slivers(&[(panel, 10.)]);
        assert_eq!(slivers.len(), 4);
        let top_left = &slivers[0];
        assert_eq!((top_left[0], top_left[1], *top_left.last().unwrap()), ((100., 100.), (110., 100.), (100., 110.)));
        assert!(top_left.iter().all(|&(x, y)| (100. ..=110.).contains(&x) && (100. ..=110.).contains(&y)));
        let menu = CGRect::new(380., 90., 60., 40.);
        assert_eq!(corner_slivers(&[(panel, 10.), (menu, 0.)]).len(), 3, "the menu over the top-right corner keeps it cut");
        assert!(corner_slivers(&[(panel, 0.)]).is_empty());
    }

}
