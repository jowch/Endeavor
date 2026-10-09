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
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;

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
    /// wry's container window around WebView2's, which the region is set on.
    window: Option<usize>,
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
    let Some(hwnd) = holes.window else { return };
    let hwnd = hwnd as HWND;
    unsafe {
        if holes.open.is_empty() {
            SetWindowRgn(hwnd, std::ptr::null_mut(), 1);
        } else {
            // The region is in the window's physical pixels, from its top-left corner.
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.;
            let region = CreateRectRgn(0, 0, ALL, ALL);
            for (_, b) in &holes.open {
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
        EnableWindow(hwnd, (!holes.dismiss).into());
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

/// The web view isn't dimmed behind a panel on Windows, as on Linux.
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
