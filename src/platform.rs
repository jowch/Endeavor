//! What the app does differently on macOS, Linux and Windows, outside GPUI and
//! wry: showing a file, reading system settings, and hosting the notebook's web
//! view. On Linux the web view is WebKitGTK in an X11 child window, so GTK has
//! to be started and its events run. `src/linux/` has the Linux versions of the
//! macOS-only fixes. Windows has only stubs so far (docs/windows.md).

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

/// A `secondary-` shortcut as the app shows it: "⌘N" on macOS, "Ctrl+N" on
/// Linux. `shortcut!(shift "K")` adds Shift.
#[cfg(target_os = "macos")]
macro_rules! shortcut {
    ($key:literal) => {
        concat!("⌘", $key)
    };
    (shift $key:literal) => {
        concat!("⌘⇧", $key)
    };
}

#[cfg(not(target_os = "macos"))]
macro_rules! shortcut {
    ($key:literal) => {
        concat!("Ctrl+", $key)
    };
    (shift $key:literal) => {
        concat!("Ctrl+Shift+", $key)
    };
}

pub(crate) use shortcut;

/// The menu item that runs `reveal` on a notebook, and on a session's folder.
pub const REVEAL: &str = if cfg!(target_os = "macos") { "Reveal in Finder" } else { "Show in Files" };
pub const REVEAL_FOLDER: &str = if cfg!(target_os = "macos") { "Reveal folder in Finder" } else { "Show folder in Files" };
/// The button that shows the log files' folder, and its accessible name.
pub const SHOW_LOGS: (&str, &str) = if cfg!(target_os = "macos") { ("Show in Finder", "Show log files in Finder") } else { ("Show in Files", "Show log files in Files") };

/// Settings' Appearance choice that follows the system's light or dark setting.
pub const MATCH_SYSTEM: &str = if cfg!(target_os = "macos") { "Match macOS" } else { "Match system" };

/// Show `path` in Finder, selected; on Linux, open its folder in the file
/// manager; on Windows, show it selected in Explorer.
pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    // Explorer reads `/select,` and the quoted path as one argument, which std's quoting would break.
    #[cfg(windows)]
    let _ = std::os::windows::process::CommandExt::raw_arg(&mut std::process::Command::new("explorer"), format!("/select,\"{}\"", path.display())).spawn();
    #[cfg(target_os = "linux")]
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

/// Not ported: Windows needs SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION).
#[cfg(windows)]
pub fn reduces_motion() -> bool {
    false
}

/// `reduces_motion` again each time the setting changes, for the life of the app.
#[cfg(target_os = "macos")]
pub fn watch_reduce_motion() -> futures::channel::mpsc::UnboundedReceiver<bool> {
    use block2::RcBlock;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification: *mut AnyObject;
    }
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let changed = RcBlock::new(move |_: *mut AnyObject| {
        let _ = tx.unbounded_send(reduces_motion());
    });
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let center: *mut AnyObject = msg_send![workspace, notificationCenter];
        let main: *mut AnyObject = msg_send![class!(NSOperationQueue), mainQueue];
        // The center keeps the observer for as long as the app runs.
        let _: *mut AnyObject = msg_send![center, addObserverForName: NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification, object: std::ptr::null_mut::<AnyObject>(), queue: main, usingBlock: &*changed];
    }
    rx
}

#[cfg(target_os = "linux")]
pub fn watch_reduce_motion() -> futures::channel::mpsc::UnboundedReceiver<bool> {
    use gtk::prelude::GtkSettingsExt;
    let (tx, rx) = futures::channel::mpsc::unbounded();
    if let Some(settings) = gtk::Settings::default() {
        settings.connect_gtk_enable_animations_notify(move |s| {
            let _ = tx.unbounded_send(!s.is_gtk_enable_animations());
        });
    }
    rx
}

/// Not ported: Windows needs WM_SETTINGCHANGE for SPI_SETCLIENTAREAANIMATION.
#[cfg(windows)]
pub fn watch_reduce_motion() -> futures::channel::mpsc::UnboundedReceiver<bool> {
    futures::channel::mpsc::unbounded().1
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

#[cfg(not(target_os = "macos"))]
pub fn bring_all_to_front(cx: &mut gpui::App) {
    cx.activate(true);
}

/// On macOS AppKit keeps the web view in step with the window by itself.
/// Windows has nothing to hook yet: WebView2's keys and focus aren't ported.
#[cfg(not(target_os = "linux"))]
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

#[cfg(windows)]
pub fn init(_: &mut gpui::App) {}

/// GPUI's application. On Windows it draws without DirectComposition: GPUI
/// makes its composition target topmost, which covers child windows, so the
/// notebook's WebView2 window would be there but never seen.
#[cfg(not(windows))]
pub fn application() -> gpui::Application {
    gpui_platform::application()
}

#[cfg(windows)]
pub fn application() -> gpui::Application {
    // Read once, while GPUI's Windows platform starts; set only for that, so
    // Julia, Node and the agent don't inherit it. A value already set wins.
    const VAR: &str = "GPUI_DISABLE_DIRECT_COMPOSITION";
    let ours = std::env::var_os(VAR).is_none();
    // SAFETY: no other thread runs yet; GPUI starts its threads in application().
    if ours {
        unsafe { std::env::set_var(VAR, "1") };
    }
    let app = gpui_platform::application();
    if ours {
        unsafe { std::env::remove_var(VAR) };
    }
    app
}

/// The window the notebook's web view goes in, as wry wants it.
#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "macos"))]
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

/// Not ported: menus over the notebook need a hole cut in WebView2's window
/// with SetWindowRgn, or the web view hidden while one is open. Until then they
/// show under the notebook.
#[cfg(windows)]
pub mod overlay {
    use gpui::{Bounds, Pixels};

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

    pub fn set_dismiss_on_click(_: &wry::WebView, _: bool) {}

    pub fn set_dimmed(_: &wry::WebView, _: bool) {}

    pub fn set_hole(_: &wry::WebView, _: Hole, _: Option<Bounds<Pixels>>) {}
}

/// WebView2's side of the notebook. Not ported: its process ending
/// (`ProcessFailed`), find in the page, and whether it has the keyboard.
#[cfg(windows)]
pub mod webcontent {
    use futures::channel::oneshot;

    pub fn on_process_ended(_: &wry::WebView, _: impl Fn() + 'static) {}

    pub fn url(webview: &wry::WebView) -> String {
        webview.url().unwrap_or_default()
    }

    /// Nothing is found until find is ported.
    pub fn find(_: &wry::WebView, _: &str, _: bool) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(false);
        rx
    }

    pub fn clear_find(_: &wry::WebView) {}

    pub fn give_keyboard(webview: &wry::WebView) {
        let _ = webview.focus();
    }

    pub fn has_keyboard(_: &wry::WebView) -> bool {
        false
    }
}
