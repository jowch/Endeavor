//! What the app does differently on macOS and Linux, outside GPUI and wry:
//! showing a file, reading system settings, and hosting the notebook's web view.
//! On Linux the web view is WebKitGTK in an X11 child window, so GTK has to be
//! started and its events pumped; the macOS-only fixes (`overlay`, `webkeys`)
//! have no Linux version yet and are no-ops there.

use std::path::Path;

/// Show `path` in Finder, selected; on Linux, open its folder in the file manager.
pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(not(target_os = "macos"))]
    if let Some(folder) = if path.is_dir() { Some(path) } else { path.parent() } {
        let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
    }
}

/// Accessibility's Reduce motion, which GPUI doesn't read itself.
#[cfg(target_os = "macos")]
pub fn reduces_motion() -> bool {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let reduce: Bool = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
        reduce.as_bool()
    }
}

/// GTK's "enable animations" setting, turned off.
#[cfg(target_os = "linux")]
pub fn reduces_motion() -> bool {
    use gtk::prelude::GtkSettingsExt;
    gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations())
}

/// Before anything touches GTK (the web view, `reduces_motion`).
#[cfg(target_os = "macos")]
pub fn init(_: &mut gpui::App) {}

/// Start GTK and keep its events flowing from GPUI's main loop.
#[cfg(target_os = "linux")]
pub fn init(cx: &mut gpui::App) {
    if let Err(e) = gtk::init() {
        eprintln!("GTK didn't start, so the notebook can't show: {e}");
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            cx.background_executor().timer(std::time::Duration::from_millis(8)).await;
        }
    })
    .detach();
}

/// The window the notebook's web view goes in, as wry wants it.
#[cfg(target_os = "macos")]
pub fn webview_parent(window: &gpui::Window) -> raw_window_handle::WindowHandle<'_> {
    raw_window_handle::HasWindowHandle::window_handle(window).expect("window handle")
}

#[cfg(target_os = "linux")]
pub struct XlibParent(raw_window_handle::RawWindowHandle);

#[cfg(target_os = "linux")]
impl raw_window_handle::HasWindowHandle for XlibParent {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: the GPUI window outlives the web view built from this handle.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.0) })
    }
}

/// wry takes only an Xlib window handle, and GPUI gives an XCB one: the same
/// X window either way.
#[cfg(target_os = "linux")]
pub fn webview_parent(window: &gpui::Window) -> XlibParent {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle, XlibWindowHandle};
    let raw = HasWindowHandle::window_handle(window).expect("window handle").as_raw();
    XlibParent(match raw {
        RawWindowHandle::Xcb(xcb) => RawWindowHandle::Xlib(XlibWindowHandle::new(xcb.window.get() as _)),
        other => other,
    })
}

#[cfg(not(target_os = "macos"))]
pub mod overlay {
    use gpui::{Bounds, Pixels};

    /// Menus over the notebook: on Linux the web view's X11 window still covers them.
    pub fn set_hole(_: &wry::WebView, _: Option<Bounds<Pixels>>) {}
}

#[cfg(not(target_os = "macos"))]
pub mod webkeys {
    pub fn fix_key_handling() {}
    pub fn allow_pinch_zoom(_: &wry::WebView) {}
}
