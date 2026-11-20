//! A GPUI menu over the notebook (macOS: `src/overlay.rs`, Linux:
//! `src/linux/overlay.rs`). WebView2 is a child window on top of everything
//! GPUI draws, so while a menu is open the web view's window gets a hole where
//! the menu is: its window region leaves the hole out, so GPUI shows through
//! and clicks there reach GPUI's window below. A window region can't let the
//! notebook show while its clicks go elsewhere, so while a menu or popover is
//! open the web view's window is disabled instead, and Windows gives its
//! clicks to GPUI's window.

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
    /// A menu or popover is open somewhere: every click on the web view goes
    /// to GPUI, so its click-outside handling can close it.
    dismiss: bool,
    /// wry's container window around WebView2's, which the region is set on.
    window: Option<usize>,
    /// The cell picture's second web view's container window, while there is one (`set_second`).
    second: Option<usize>,
}

thread_local! {
    static HOLES: RefCell<Holes> = RefCell::default();
}

/// wry's container window: WebView2's windows are inside it, so its region
/// and its being disabled apply to them too.
fn container(webview: &wry::WebView) -> Option<usize> {
    use wry::WebViewExtWindows;
    let mut parent = Default::default();
    unsafe { webview.controller().ParentWindow(&mut parent) }.ok()?;
    let hwnd = parent.0 as usize;
    (hwnd != 0).then_some(hwnd)
}

/// Past any window's size, so a resize keeps the rest whole.
const ALL: i32 = 1 << 16;

fn apply(holes: &Holes) {
    let open: Vec<_> = holes.open.iter().map(|(_, b)| *b).collect();
    // The second view has the same holes, so GPUI shows through both. It never takes a click.
    if let Some(second) = holes.second {
        unsafe { set_region(second as HWND, &open) };
    }
    let Some(hwnd) = holes.window else { return };
    let hwnd = hwnd as HWND;
    unsafe {
        set_region(hwnd, &open);
        // A disabled window keeps the keyboard if it has it, and then keys go
        // nowhere, so GPUI's window takes it first.
        if holes.dismiss {
            SetFocus(GetParent(hwnd));
        }
        EnableWindow(hwnd, (!holes.dismiss).into());
    }
}

/// `hwnd`'s region: all of it but `holes`.
unsafe fn set_region(hwnd: HWND, holes: &[Bounds<Pixels>]) {
    unsafe {
        if holes.is_empty() {
            SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
            return;
        }
        // The region is in the window's physical pixels, from its top-left corner.
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
        // The window owns the region once it's set.
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

/// A menu or popover open anywhere in the window, so a click on the web view
/// should reach GPUI even outside any hole (`Workspace::dismissible_open`).
pub fn set_dismiss_on_click(webview: &wry::WebView, active: bool) {
    update(Some(webview), |holes| std::mem::replace(&mut holes.dismiss, active) != active);
}

/// As on Linux, the web view isn't dimmed behind a panel.
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

/// wry's container window around `webview`, for `cell_shot` to place the second view.
pub fn window_of(webview: &wry::WebView) -> Option<usize> {
    container(webview)
}

/// The cell picture's second web view (`cell_shot`), kept just under the user's at the same
/// place, or None once it goes. Every hole cut in the user's view is cut in it too, in the same
/// call, so it never shows where GPUI draws a menu, a notice or a tip. It never takes a click.
pub fn set_second(other: Option<&wry::WebView>) {
    HOLES.with_borrow_mut(|holes| {
        holes.second = other.and_then(container);
        if let Some(second) = holes.second {
            unsafe { EnableWindow(second as HWND, 0) };
        }
        apply(holes);
    });
}

/// Whether the second view is registered (`set_second`). It isn't when its window wasn't
/// there yet as it was made, and `cell_shot` registers it again then.
pub fn has_second() -> bool {
    HOLES.with_borrow(|holes| holes.second.is_some())
}
