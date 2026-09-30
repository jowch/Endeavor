//! What the app does differently on macOS and Linux, outside GPUI and wry:
//! showing a file, reading system settings, and hosting the notebook's web view.
//! On Linux the web view is WebKitGTK in an X11 child window, so GTK has to be
//! started and its events run. `src/linux/` has the Linux versions of the
//! macOS-only fixes.

use std::path::Path;

/// The computer the app runs on, as the app names it: "This Mac" on macOS,
/// "This computer" on Linux. `this_computer!(lower)` is the same inside a
/// sentence ("on this Mac"). A macro, so `concat!` can put it in fixed text.
#[cfg(target_os = "macos")]
macro_rules! this_computer {
    () => {
        "This Mac"
    };
    (lower) => {
        "this Mac"
    };
}

#[cfg(not(target_os = "macos"))]
macro_rules! this_computer {
    () => {
        "This computer"
    };
    (lower) => {
        "this computer"
    };
}

pub(crate) use this_computer;

/// The menu item that runs `reveal` on a notebook, and on a session's folder.
pub const REVEAL: &str = if cfg!(target_os = "macos") { "Reveal in Finder" } else { "Show in Files" };
pub const REVEAL_FOLDER: &str = if cfg!(target_os = "macos") { "Reveal folder in Finder" } else { "Show folder in Files" };
/// The button that shows the log files' folder, and its accessible name.
pub const SHOW_LOGS: (&str, &str) = if cfg!(target_os = "macos") { ("Show in Finder", "Show log files in Finder") } else { ("Show in Files", "Show log files in Files") };

/// Settings' Appearance choice that follows the system's light or dark setting.
pub const MATCH_SYSTEM: &str = if cfg!(target_os = "macos") { "Match macOS" } else { "Match system" };

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

/// Run from source (`cargo run`), the app has no bundle to take its Dock icon
/// from, so it sets the icon itself.
#[cfg(target_os = "macos")]
pub fn init(_: &mut gpui::App) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    let bundled = std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"));
    if bundled {
        return;
    }
    let Ok(path) = std::ffi::CString::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icon/Endeavor.icns")) else { return };
    unsafe {
        let path: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: path.as_ptr()];
        let image: *mut AnyObject = msg_send![class!(NSImage), alloc];
        let image: *mut AnyObject = msg_send![image, initWithContentsOfFile: path];
        if image.is_null() {
            return;
        }
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setApplicationIconImage: image];
    }
}

/// Window ▸ Bring All to Front.
#[cfg(target_os = "macos")]
pub fn bring_all_to_front(_: &mut gpui::App) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, arrangeInFront: std::ptr::null_mut::<AnyObject>()];
    }
}

#[cfg(target_os = "linux")]
pub fn bring_all_to_front(cx: &mut gpui::App) {
    cx.activate(true);
}

/// On macOS AppKit keeps the web view in step with the window by itself.
#[cfg(target_os = "macos")]
pub fn web_view_hooks() -> impl gpui::IntoElement {
    gpui::Empty
}

#[cfg(target_os = "linux")]
pub use crate::linux::web_view_hooks;

/// Start GTK and keep its events flowing from GPUI's main loop.
#[cfg(target_os = "linux")]
pub fn init(cx: &mut gpui::App) {
    if let Err(e) = gtk::init() {
        eprintln!("GTK didn't start, so the notebook can't show: {e}");
        return;
    }
    crate::linux::gtk_loop::start(cx);
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

/// Put `message` above the file picker that is opening (GPUI's picker takes
/// only its button's label). The panel opens on the next turn of the main
/// loop, so this looks for it shortly after.
#[cfg(target_os = "macos")]
pub fn set_open_panel_message(message: &'static str, cx: &mut gpui::App) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    cx.spawn(async move |cx| {
        for _ in 0..20 {
            cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
            let Ok(text) = std::ffi::CString::new(message) else { return };
            let found = unsafe {
                let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                let windows: *mut AnyObject = msg_send![app, windows];
                let count: usize = msg_send![windows, count];
                let mut found = false;
                for i in 0..count {
                    let window: *mut AnyObject = msg_send![windows, objectAtIndex: i];
                    let is_panel: bool = msg_send![window, isKindOfClass: class!(NSOpenPanel)];
                    if is_panel {
                        let text: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()];
                        let _: () = msg_send![window, setMessage: text];
                        found = true;
                    }
                }
                found
            };
            if found {
                return;
            }
        }
    })
    .detach();
}

#[cfg(target_os = "linux")]
pub fn set_open_panel_message(_: &'static str, _: &mut gpui::App) {}

#[cfg(not(target_os = "macos"))]
pub mod snapshot {
    use futures::channel::oneshot;

    /// No picture of the notebook here yet: a drawn box goes as its cells.
    pub fn png(_: &wry::WebView, _: [f64; 4]) -> oneshot::Receiver<Option<Vec<u8>>> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(None);
        rx
    }
}

#[cfg(not(target_os = "macos"))]
pub mod dialogs {
    /// The page's alert(), confirm() and prompt(): WebKitGTK's own dialogs, untested (docs/linux.md).
    pub fn show_page_dialogs(_: &wry::WebView) {}
}
