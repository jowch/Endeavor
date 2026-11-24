use std::cell::RefCell;

use gpui::{Bounds, Pixels};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_DIFF};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;

pub use crate::layer::Hole;

#[derive(Default)]
struct Holes {
    open: Vec<(Hole, Bounds<Pixels>)>,
    dismiss: bool,
    window: Option<usize>,
    second: Option<usize>,
}

thread_local! {
    static HOLES: RefCell<Holes> = RefCell::default();
}

fn container(webview: &wry::WebView) -> Option<usize> {
    use wry::WebViewExtWindows;
    let mut parent = Default::default();
    unsafe { webview.controller().ParentWindow(&mut parent) }.ok()?;
    let hwnd = parent.0 as usize;
    (hwnd != 0).then_some(hwnd)
}

const ALL: i32 = 1 << 16;

fn apply(holes: &Holes) {
    let open: Vec<_> = holes.open.iter().map(|(_, b)| *b).collect();
    if let Some(second) = holes.second {
        unsafe { set_region(second as HWND, &open) };
    }
    let Some(hwnd) = holes.window else { return };
    let hwnd = hwnd as HWND;
    unsafe {
        set_region(hwnd, &open);
        if holes.dismiss {
            SetFocus(GetParent(hwnd));
        }
        EnableWindow(hwnd, (!holes.dismiss).into());
    }
}

unsafe fn set_region(hwnd: HWND, holes: &[Bounds<Pixels>]) {
    unsafe {
        if holes.is_empty() {
            SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
            return;
        }
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
        let region = CreateRectRgn(0, 0, ALL, ALL);
        for b in holes {
            let x = (f32::from(b.origin.x) * scale).floor() as i32;
            let y = (f32::from(b.origin.y) * scale).floor() as i32;
            let right = (f32::from(b.origin.x + b.size.width) * scale).ceil() as i32;
            let bottom = (f32::from(b.origin.y + b.size.height) * scale).ceil() as i32;
            let hole = CreateRectRgn(x, y, right, bottom);
            CombineRgn(region, region, hole, RGN_DIFF);
            DeleteObject(hole);
        }
        if SetWindowRgn(hwnd, region, 1) == 0 {
            DeleteObject(region);
        }
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

pub fn window_of(webview: &wry::WebView) -> Option<usize> {
    container(webview)
}

pub fn set_second(other: Option<&wry::WebView>) {
    HOLES.with_borrow_mut(|holes| {
        holes.second = other.and_then(container);
        if let Some(second) = holes.second {
            unsafe { EnableWindow(second as HWND, 0) };
        }
        apply(holes);
    });
}

pub fn has_second() -> bool {
    HOLES.with_borrow(|holes| holes.second.is_some())
}
