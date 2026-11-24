use std::cell::RefCell;
use std::ffi::{c_int, c_ulong};

use gpui::{Bounds, Pixels};
use gtk::gdk;
use gtk::prelude::*;

pub use crate::layer::Hole;

#[derive(Default)]
struct Holes {
    open: Vec<(Hole, Bounds<Pixels>)>,
    dismiss: bool,
    window: Option<gdk::Window>,
    second: Option<gdk::Window>,
}

thread_local! {
    static HOLES: RefCell<Holes> = RefCell::default();
}

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

unsafe fn set_shape(display: *mut x11::xlib::Display, window: c_ulong, kind: c_int, shape: Shape) {
    unsafe {
        match shape {
            Shape::Whole => XShapeCombineMask(display, window, kind, 0, 0, 0, SHAPE_SET),
            Shape::Empty => XShapeCombineRectangles(display, window, kind, 0, 0, std::ptr::null_mut(), 0, SHAPE_SET, UNSORTED),
            Shape::Holes(holes) => {
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
    let Some(display) = super::xdisplay() else { return };
    for (window, clicks) in [(&holes.window, !holes.dismiss), (&holes.second, false)] {
        let Some((window, xid)) = window.as_ref().and_then(|w| Some((w, super::xid_of(w)?))) else { continue };
        let scale = window.scale_factor() as f32;
        let rects: Vec<_> = holes.open.iter().map(|(_, b)| xrect(b, scale)).collect();
        let shape = if rects.is_empty() { Shape::Whole } else { Shape::Holes(&rects) };
        unsafe {
            set_shape(display, xid, SHAPE_BOUNDING, shape);
            set_shape(display, xid, SHAPE_INPUT, if clicks { shape } else { Shape::Empty });
        }
    }
    unsafe { x11::xlib::XFlush(display) };
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

pub fn set_dismiss_on_click(webview: &wry::WebView, active: bool) {
    update(Some(webview), |holes| std::mem::replace(&mut holes.dismiss, active) != active);
}

pub fn set_dimmed(_: &wry::WebView, _: bool) {}

pub fn set_hole(webview: &wry::WebView, owner: Hole, hole: Option<Bounds<Pixels>>) {
    update(Some(webview), |holes| {
        let before = holes.open.clone();
        holes.open.retain(|(o, _)| *o != owner);
        holes.open.extend(hole.map(|b| (owner, b)));
        holes.open != before
    });
}

pub fn set_second(other: Option<&wry::WebView>) {
    HOLES.with_borrow_mut(|holes| {
        holes.second = other.and_then(container);
        apply(holes);
    });
}

pub fn has_second() -> bool {
    HOLES.with_borrow(|holes| holes.second.is_some())
}

pub fn window_of(webview: &wry::WebView) -> Option<gdk::Window> {
    container(webview)
}

pub fn viewable(window: &gdk::Window) -> bool {
    let (Some(display), Some(xid)) = (super::xdisplay(), super::xid_of(window)) else { return false };
    let mut attributes = std::mem::MaybeUninit::<x11::xlib::XWindowAttributes>::zeroed();
    unsafe { x11::xlib::XGetWindowAttributes(display, xid, attributes.as_mut_ptr()) != 0 && attributes.assume_init().map_state == x11::xlib::IsViewable }
}
