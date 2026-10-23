//! Linux: the notebook's web view is WebKitGTK in an X11 child window of
//! GPUI's window. X gives each window its own keyboard focus and its own
//! region on screen, so the app moves focus and cuts holes itself, where
//! macOS does it with AppKit (`webkeys.rs`, `overlay.rs`).

pub mod gtk_loop;
pub mod overlay;
pub mod webcontent;
pub mod webkeys;

use std::cell::RefCell;

use gpui::{App, IntoElement, MouseDownEvent, Styled, Window, canvas};
use gtk::glib::Propagation;
use gtk::prelude::*;

/// GDK's own Xlib connection, so our requests queue behind GDK's.
pub(super) fn xdisplay() -> Option<*mut x11::xlib::Display> {
    use gtk::glib::Cast;
    use gtk::glib::translate::ToGlibPtr;
    let display = gtk::gdk::Display::default()?.downcast::<gdkx11::X11Display>().ok()?;
    let raw: *mut gdkx11::ffi::GdkX11Display = display.to_glib_none().0;
    Some(unsafe { gdkx11::ffi::gdk_x11_display_get_xdisplay(raw) } as *mut x11::xlib::Display)
}

/// GPUI's X window.
fn gpui_xid(window: &Window) -> Option<u64> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Xcb(xcb) => Some(xcb.window.get().into()),
        RawWindowHandle::Xlib(xlib) => Some(xlib.window),
        _ => None,
    }
}

pub(super) fn xid_of(window: &gtk::gdk::Window) -> Option<u64> {
    use gtk::glib::Cast;
    Some(window.clone().downcast::<gdkx11::X11Window>().ok()?.xid())
}

thread_local! {
    /// The notebook's web view, to tell GTK when the keyboard leaves it.
    static WEB_VIEW: RefCell<Option<webkit2gtk::WebView>> = const { RefCell::new(None) };
}

/// Keyboard handling for the notebook's web view in `window`. The order counts:
/// GTK runs key handlers in the order they're connected.
pub fn attach(webview: &wry::WebView, window: &Window, cx: &mut App) {
    use wry::WebViewExtUnix;
    let Some(gpui) = gpui_xid(window) else { return };
    WEB_VIEW.with(|v| v.replace(Some(webview.webview())));
    focus_on_click(webview);
    keys_follow_focus(webview, gpui);
    webkeys::forward_app_keys(webview, window.window_handle(), gpui, cx);
}

/// Drawn in the workspace every frame. A click GPUI gets was outside the web
/// view (or in a hole cut in it), so the keyboard goes to GPUI's window.
/// gpui-wry answers the same click by focusing the web view's container
/// window, so this runs after it: deferred, and on GDK's connection so the X
/// server sees it second. And GTK runs once the frame has moved the web view.
/// When another app's window takes the keyboard, the page loses it too.
pub fn web_view_hooks() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |_, _, window, _| {
            gtk_loop::wake();
            // GPUI's window is inactive while the web view has the keyboard too.
            if !window.is_window_active() && !web_view_has_keyboard() {
                web_view_focused(false);
            }
            let Some(xid) = gpui_xid(window) else { return };
            window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx: &mut App| {
                if phase.capture() {
                    cx.defer(move |_| focus_gpui(xid));
                }
            });
        },
    )
    .absolute()
}

/// A click in the web view gives it the keyboard: WebKit's own X window, as
/// wry's container window around it doesn't take keys.
fn focus_on_click(webview: &wry::WebView) {
    use wry::WebViewExtUnix;
    webview.webview().connect_button_press_event(move |view, _| {
        focus_web(view);
        Propagation::Proceed
    });
}

/// The keyboard to the web view: X input focus to WebKit's own window, and
/// GTK told as well.
pub(super) fn focus_web(view: &webkit2gtk::WebView) {
    if let Some(xid) = view.window().as_ref().and_then(xid_of) {
        focus(xid);
    }
    web_view_focused(true);
}

/// Whether X input focus is on the web view's own window.
fn web_view_has_keyboard() -> bool {
    WEB_VIEW.with(|v| v.borrow().as_ref().and_then(|view| view.window()).as_ref().and_then(xid_of).is_some_and(focused))
}

/// The keyboard back to GPUI's window, `gpui`.
pub(super) fn focus_gpui(gpui: u64) {
    focus(gpui);
    web_view_focused(false);
}

/// Tell GTK whether the web view has the keyboard. WebKit takes the page's
/// focus from wry's GTK window around the web view, but GDK only follows X
/// focus for top-level windows, and that one sits in an X child window of
/// GPUI's. So GTK never sees it focused, and the page always thinks it's in
/// the background: editors draw no text cursor (#145), and paste does nothing
/// (#147). The window gets the focus event a window manager would give it.
fn web_view_focused(focused: bool) {
    use gtk::glib::translate::{ToGlibPtr, ToGlibPtrMut};
    let Some(view) = WEB_VIEW.with(|v| v.borrow().clone()) else { return };
    let Some(toplevel) = view.toplevel().and_then(|w| w.downcast::<gtk::Window>().ok()) else { return };
    if toplevel.has_toplevel_focus() == focused {
        return;
    }
    let Some(gdk_window) = toplevel.window() else { return };
    if focused {
        // The widget the window's focus goes to.
        view.grab_focus();
    }
    let mut event = gtk::gdk::Event::new(gtk::gdk::EventType::FocusChange);
    unsafe {
        let focus: *mut gtk::gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
        let focus = focus as *mut gtk::gdk::ffi::GdkEventFocus;
        // The event's free unrefs it.
        (*focus).window = gdk_window.to_glib_full();
        (*focus).send_event = 1;
        (*focus).in_ = focused as i16;
    }
    toplevel.send_focus_change(&event);
}

fn focus(xid: u64) {
    let Some(display) = xdisplay() else { return };
    unsafe {
        x11::xlib::XSetInputFocus(display, xid, x11::xlib::RevertToParent, x11::xlib::CurrentTime);
        x11::xlib::XFlush(display);
    }
}

fn focused(xid: u64) -> bool {
    let Some(display) = xdisplay() else { return false };
    let (mut window, mut revert) = (0, 0);
    unsafe { x11::xlib::XGetInputFocus(display, &mut window, &mut revert) };
    window == xid
}

/// Send `event` on to GPUI's window as the X key event it came from.
fn resend_key(event: &gtk::gdk::EventKey, to: u64) {
    let Some(display) = xdisplay() else { return };
    let press = event.event_type() == gtk::gdk::EventType::KeyPress;
    unsafe {
        let mut key: x11::xlib::XKeyEvent = std::mem::zeroed();
        key.type_ = if press { x11::xlib::KeyPress } else { x11::xlib::KeyRelease };
        key.send_event = x11::xlib::True;
        key.display = display;
        key.window = to;
        key.root = x11::xlib::XDefaultRootWindow(display);
        key.time = event.time().into();
        key.state = event.state().bits();
        key.keycode = event.hardware_keycode().into();
        key.same_screen = x11::xlib::True;
        let mask = if press { x11::xlib::KeyPressMask } else { x11::xlib::KeyReleaseMask };
        let mut event = x11::xlib::XEvent { key };
        x11::xlib::XSendEvent(display, to, x11::xlib::False, mask, &mut event);
        x11::xlib::XFlush(display);
    }
}

/// X sends keys to the window under the pointer when it's inside the focused
/// window, so with GPUI focused the web view still gets keys while the pointer
/// is over it. Those go on to GPUI.
fn keys_follow_focus(webview: &wry::WebView, gpui: u64) {
    use wry::WebViewExtUnix;
    let view = webview.webview();
    let pass_on = move |_: &_, event: &gtk::gdk::EventKey| {
        if focused(gpui) {
            resend_key(event, gpui);
            Propagation::Stop
        } else {
            Propagation::Proceed
        }
    };
    view.connect_key_press_event(pass_on);
    view.connect_key_release_event(pass_on);
}
