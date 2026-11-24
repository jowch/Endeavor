pub mod gtk_loop;
pub mod overlay;
pub mod snapshot;
pub mod webcontent;
pub mod webkeys;

use std::cell::{Cell, RefCell};

use gpui::{App, IntoElement, MouseDownEvent, Styled, Window, canvas};
use gtk::glib::Propagation;
use gtk::prelude::*;

pub(super) fn xdisplay() -> Option<*mut x11::xlib::Display> {
    use gtk::glib::Cast;
    use gtk::glib::translate::ToGlibPtr;
    let display = gtk::gdk::Display::default()?.downcast::<gdkx11::X11Display>().ok()?;
    let raw: *mut gdkx11::ffi::GdkX11Display = display.to_glib_none().0;
    Some(unsafe { gdkx11::ffi::gdk_x11_display_get_xdisplay(raw) } as *mut x11::xlib::Display)
}

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
    static WEB_VIEW: RefCell<Option<webkit2gtk::WebView>> = const { RefCell::new(None) };
    static HAD_KEYBOARD: Cell<bool> = const { Cell::new(false) };
}

pub fn attach(webview: &wry::WebView, window: &Window, cx: &mut App) {
    use wry::WebViewExtUnix;
    let Some(gpui) = gpui_xid(window) else { return };
    WEB_VIEW.with(|v| v.replace(Some(webview.webview())));
    focus_on_click(webview);
    no_new_windows(webview);
    keys_follow_focus(webview, gpui);
    webkeys::forward_app_keys(webview, window.window_handle(), gpui, cx);
}

pub fn web_view_hooks() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |_, _, window, _| {
            gtk_loop::wake();
            if !window.is_window_active() && gtk_focused() && !web_view_has_keyboard() {
                web_view_focused(false);
            }
            let Some(xid) = gpui_xid(window) else { return };
            window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx: &mut App| {
                if phase.capture() {
                    HAD_KEYBOARD.with(|h| h.set(gtk_focused()));
                    cx.defer(move |_| focus_gpui(xid));
                }
            });
        },
    )
    .absolute()
}

fn no_new_windows(webview: &wry::WebView) {
    use webkit2gtk::{ContextMenuAction, ContextMenuExt, ContextMenuItemExt, WebViewExt};
    use wry::WebViewExtUnix;
    webview.webview().connect_context_menu(|_, menu, _, _| {
        for item in menu.items() {
            if matches!(
                item.stock_action(),
                ContextMenuAction::OpenImageInNewWindow
                    | ContextMenuAction::OpenFrameInNewWindow
                    | ContextMenuAction::OpenVideoInNewWindow
                    | ContextMenuAction::OpenAudioInNewWindow
            ) {
                menu.remove(&item);
            }
        }
        false
    });
}

fn focus_on_click(webview: &wry::WebView) {
    use wry::WebViewExtUnix;
    webview.webview().connect_button_press_event(move |view, _| {
        focus_web(view);
        Propagation::Proceed
    });
}

pub(super) fn focus_web(view: &webkit2gtk::WebView) {
    if let Some(xid) = view.window().as_ref().and_then(xid_of) {
        focus(xid);
    }
    web_view_focused(true);
}

pub(super) fn web_view_had_keyboard() -> bool {
    gtk_focused() || HAD_KEYBOARD.with(Cell::get)
}

fn gtk_focused() -> bool {
    WEB_VIEW.with(|v| v.borrow().as_ref().and_then(|view| view.toplevel()).and_then(|w| w.downcast::<gtk::Window>().ok()).is_some_and(|w| w.has_toplevel_focus()))
}

fn web_view_has_keyboard() -> bool {
    WEB_VIEW.with(|v| v.borrow().as_ref().and_then(|view| view.window()).as_ref().and_then(xid_of).is_some_and(focused))
}

pub(super) fn focus_gpui(gpui: u64) {
    focus(gpui);
    web_view_focused(false);
}

fn web_view_focused(focused: bool) {
    use gtk::glib::translate::{ToGlibPtr, ToGlibPtrMut};
    let Some(view) = WEB_VIEW.with(|v| v.borrow().clone()) else { return };
    let Some(toplevel) = view.toplevel().and_then(|w| w.downcast::<gtk::Window>().ok()) else { return };
    if toplevel.has_toplevel_focus() == focused {
        return;
    }
    let Some(gdk_window) = toplevel.window() else { return };
    if focused {
        view.grab_focus();
    }
    let mut event = gtk::gdk::Event::new(gtk::gdk::EventType::FocusChange);
    unsafe {
        let focus: *mut gtk::gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
        let focus = focus as *mut gtk::gdk::ffi::GdkEventFocus;
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
