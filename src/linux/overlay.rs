//! A GPUI menu over the notebook (macOS: `src/overlay.rs`). The web view is an
//! X child window on top of everything GPUI draws, so while a menu is open the
//! window gets a hole where the menu is, with the X Shape extension: its
//! bounding shape leaves the hole out, so GPUI shows through, and clicks there
//! reach GPUI's window below.

use std::cell::RefCell;
use std::ffi::{c_int, c_ulong};

use gpui::{Bounds, Pixels};
use gtk::gdk;
use gtk::prelude::*;

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

#[derive(Default)]
struct Holes {
    open: Vec<(Hole, Bounds<Pixels>)>,
    /// A menu or popover is open somewhere: every click on the web view goes
    /// to GPUI, so its click-outside handling can close it.
    dismiss: bool,
    /// wry's container window around the web view, which the shape is set on.
    window: Option<gdk::Window>,
}

thread_local! {
    static HOLES: RefCell<Holes> = RefCell::default();
}

/// wry's X window for the web view. wry places it in GDK's units, taking them
/// to be GPUI's logical pixels.
fn container(webview: &wry::WebView) -> Option<gdk::Window> {
    use wry::WebViewExtUnix;
    webview.webview().toplevel()?.window()
}

fn xrect(b: &Bounds<Pixels>, scale: f32) -> x11::xlib::XRectangle {
    let (x, y) = ((f32::from(b.origin.x) * scale).floor(), (f32::from(b.origin.y) * scale).floor());
    let right = (f32::from(b.origin.x + b.size.width) * scale).ceil();
    let bottom = (f32::from(b.origin.y + b.size.height) * scale).ceil();
    x11::xlib::XRectangle { x: x as i16, y: y as i16, width: (right - x) as u16, height: (bottom - y) as u16 }
}

// From X11/extensions/shape.h.
const SHAPE_BOUNDING: c_int = 0;
const SHAPE_INPUT: c_int = 2;
const SHAPE_SET: c_int = 0;
const SHAPE_SUBTRACT: c_int = 3;
const UNSORTED: c_int = 0;

#[link(name = "Xext")]
unsafe extern "C" {
    fn XShapeCombineRectangles(display: *mut x11::xlib::Display, window: c_ulong, kind: c_int, x: c_int, y: c_int, rects: *mut x11::xlib::XRectangle, n: c_int, op: c_int, ordering: c_int);
    fn XShapeCombineMask(display: *mut x11::xlib::Display, window: c_ulong, kind: c_int, x: c_int, y: c_int, mask: c_ulong, op: c_int);
}

/// Set `kind` of the shape of `window`: everything, nothing, or everything
/// but `holes` (in X's pixels).
unsafe fn set_shape(display: *mut x11::xlib::Display, window: c_ulong, kind: c_int, shape: Shape) {
    unsafe {
        match shape {
            Shape::Whole => XShapeCombineMask(display, window, kind, 0, 0, 0, SHAPE_SET),
            Shape::Empty => XShapeCombineRectangles(display, window, kind, 0, 0, std::ptr::null_mut(), 0, SHAPE_SET, UNSORTED),
            Shape::Holes(holes) => {
                // Past any window's size, so a resize keeps the rest whole.
                let mut all = x11::xlib::XRectangle { x: 0, y: 0, width: u16::MAX, height: u16::MAX };
                XShapeCombineRectangles(display, window, kind, 0, 0, &mut all, 1, SHAPE_SET, UNSORTED);
                let mut holes = holes.to_vec();
                XShapeCombineRectangles(display, window, kind, 0, 0, holes.as_mut_ptr(), holes.len() as c_int, SHAPE_SUBTRACT, UNSORTED);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Shape<'a> {
    Whole,
    Empty,
    Holes(&'a [x11::xlib::XRectangle]),
}

fn apply(holes: &Holes) {
    let (Some(window), Some(display)) = (&holes.window, super::xdisplay()) else { return };
    let Some(xid) = super::xid_of(window) else { return };
    let scale = window.scale_factor() as f32;
    let rects: Vec<_> = holes.open.iter().map(|(_, b)| xrect(b, scale)).collect();
    let shape = if rects.is_empty() { Shape::Whole } else { Shape::Holes(&rects) };
    unsafe {
        set_shape(display, xid, SHAPE_BOUNDING, shape);
        set_shape(display, xid, SHAPE_INPUT, if holes.dismiss { Shape::Empty } else { shape });
        x11::xlib::XFlush(display);
    }
}

fn update(webview: Option<&wry::WebView>, change: impl FnOnce(&mut Holes) -> bool) {
    HOLES.with_borrow_mut(|holes| {
        if holes.window.is_none() {
            holes.window = webview.and_then(container);
        }
        if change(holes) {
            apply(holes);
        }
    });
}

/// A menu or popover open anywhere in the window, so a click on the web view
/// should reach GPUI even outside any hole (`Workspace::dismissible_open`).
pub fn set_dismiss_on_click(webview: &wry::WebView, active: bool) {
    update(Some(webview), |holes| std::mem::replace(&mut holes.dismiss, active) != active);
}

/// No see-through child windows without a compositor, so the web view isn't
/// dimmed behind a panel on Linux.
pub fn set_dimmed(_: &wry::WebView, _: bool) {}

/// Cut `owner`'s hole in the web view at `hole` (relative to the web view's
/// top-left corner), or close it with None. Other holes stay.
pub fn set_hole(webview: &wry::WebView, owner: Hole, hole: Option<Bounds<Pixels>>) {
    update(Some(webview), |holes| {
        let before = holes.open.clone();
        holes.open.retain(|(o, _)| *o != owner);
        holes.open.extend(hole.map(|b| (owner, b)));
        holes.open != before
    });
}
