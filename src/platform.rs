use std::path::Path;

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

#[cfg(target_os = "macos")]
macro_rules! keys {
    ($key:literal) => {
        concat!("cmd-", $key)
    };
}

#[cfg(not(target_os = "macos"))]
macro_rules! keys {
    ($key:literal) => {
        concat!("ctrl-", $key)
    };
}

pub(crate) use keys;

pub const REVEAL: &str = if cfg!(target_os = "macos") { "Reveal in Finder" } else if cfg!(windows) { "Show in File Explorer" } else { "Show in Files" };
pub const REVEAL_FOLDER: &str = if cfg!(target_os = "macos") { "Reveal folder in Finder" } else if cfg!(windows) { "Show folder in File Explorer" } else { "Show folder in Files" };
pub const SHOW_LOGS: (&str, &str) = if cfg!(target_os = "macos") {
    ("Show in Finder", "Show log files in Finder")
} else if cfg!(windows) {
    ("Show in File Explorer", "Show log files in File Explorer")
} else {
    ("Show in Files", "Show log files in Files")
};

pub const MATCH_SYSTEM: &str = if cfg!(target_os = "macos") { "Match macOS" } else { "Match system" };

pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    if let Err(e) = std::os::windows::process::CommandExt::raw_arg(&mut std::process::Command::new("explorer"), format!("/select,\"{}\"", path.display())).spawn() {
        eprintln!("Couldn't show {} in File Explorer: {e}", path.display());
    }
    #[cfg(target_os = "linux")]
    if let Some(folder) = if path.is_dir() { Some(path) } else { path.parent() } {
        let _ = opener("xdg-open").arg(folder).spawn().map(|mut c| std::thread::spawn(move || c.wait()));
    }
}

#[cfg(target_os = "linux")]
pub const WAYLAND_DISPLAY_KEPT: &str = "ENDEAVOR_WAYLAND_DISPLAY";

#[cfg(target_os = "linux")]
pub fn wayland_display() -> Option<String> {
    std::env::var(WAYLAND_DISPLAY_KEPT).ok().filter(|display| !display.is_empty())
}

#[cfg(unix)]
pub fn opener(program: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(program);
    #[cfg(target_os = "linux")]
    if let Some(display) = std::env::var_os(WAYLAND_DISPLAY_KEPT) {
        command.env("WAYLAND_DISPLAY", display);
    }
    command
}

pub fn open_url(url: &str, cx: &gpui::App) {
    #[cfg(target_os = "linux")]
    if let Ok(mut child) = opener("xdg-open").arg(url).spawn() {
        std::thread::spawn(move || child.wait());
        return;
    }
    cx.open_url(url);
}

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

#[cfg(target_os = "linux")]
pub fn reduces_motion() -> bool {
    use gtk::prelude::GtkSettingsExt;
    gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations())
}

#[cfg(windows)]
pub fn reduces_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
    let mut animate: windows_sys::core::BOOL = 1;
    let read = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut animate as *mut windows_sys::core::BOOL).cast(), 0) };
    read != 0 && animate == 0
}

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

#[cfg(windows)]
pub fn watch_reduce_motion() -> futures::channel::mpsc::UnboundedReceiver<bool> {
    use std::cell::RefCell;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW, WM_SETTINGCHANGE, WNDCLASSW, WS_OVERLAPPED};
    thread_local! {
        static WATCH: RefCell<Option<(futures::channel::mpsc::UnboundedSender<bool>, bool)>> = const { RefCell::new(None) };
    }
    unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if message == WM_SETTINGCHANGE {
            let now = reduces_motion();
            WATCH.with(|watch| {
                if let Some((tx, sent)) = watch.borrow_mut().as_mut()
                    && *sent != now
                {
                    *sent = now;
                    let _ = tx.unbounded_send(now);
                }
            });
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let started = std::thread::Builder::new().name("reduce-motion".into()).spawn(move || unsafe {
        WATCH.with(|watch| *watch.borrow_mut() = Some((tx, reduces_motion())));
        let instance = GetModuleHandleW(std::ptr::null());
        let class = windows_sys::w!("EndeavorSettingsWatch");
        let wc = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance, lpszClassName: class, ..std::mem::zeroed() };
        if RegisterClassW(&wc) == 0 {
            eprintln!("Couldn't watch the Animation effects setting: RegisterClassW failed");
            return;
        }
        let hwnd = CreateWindowExW(0, class, std::ptr::null(), WS_OVERLAPPED, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
        if hwnd.is_null() {
            eprintln!("Couldn't watch the Animation effects setting: CreateWindowExW failed");
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }
    });
    if let Err(e) = started {
        eprintln!("Couldn't watch the Animation effects setting: {e}");
    }
    rx
}

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

#[cfg(target_os = "macos")]
pub fn web_view_hooks() -> impl gpui::IntoElement {
    gpui::Empty
}

#[cfg(windows)]
static WAS_HOVERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
pub fn web_view_hooks() -> impl gpui::IntoElement {
    use gpui::Styled;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    gpui::canvas(
        |_, _, _| (),
        |_, _, window, cx| {
            let Ok(handle) = window.window_handle() else { return };
            let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
            let hwnd = handle.hwnd.get() as usize;
            window.on_mouse_event(move |_: &gpui::MouseDownEvent, phase, _, _| {
                if phase.capture() {
                    webcontent::note_click();
                    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(hwnd as _) };
                }
            });
            let hovered = window.is_window_hovered();
            if !hovered && WAS_HOVERED.swap(false, std::sync::atomic::Ordering::Relaxed) {
                window.defer(cx, |window, cx| {
                    let away = gpui::MouseMoveEvent { position: gpui::point(gpui::px(-1.), gpui::px(-1.)), pressed_button: None, modifiers: window.modifiers() };
                    window.dispatch_event(gpui::PlatformInput::MouseMove(away), cx);
                });
            }
            WAS_HOVERED.store(hovered, std::sync::atomic::Ordering::Relaxed);
        },
    )
    .absolute()
}

#[cfg(target_os = "linux")]
pub use crate::linux::web_view_hooks;

#[cfg(target_os = "linux")]
pub fn init(cx: &mut gpui::App) {
    gtk::gdk::set_allowed_backends("x11");
    if let Err(e) = gtk::init() {
        eprintln!("GTK didn't start, so the notebook can't show: {e}");
        return;
    }
    crate::linux::gtk_loop::start(cx);
}

#[cfg(windows)]
pub fn init(_: &mut gpui::App) {
    let name: Vec<u16> = "EndeavorApp\0".encode_utf16().collect();
    // SAFETY: a null-terminated wide name; the handle stays open for the app's life.
    unsafe { windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
}

#[cfg(windows)]
pub fn no_web_view(error: &wry::Error) -> ! {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    eprintln!("The notebook's web view couldn't start: {error}");
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let text = wide(&format!(
        "Endeavor's notebook view couldn't start, so Endeavor can't open.\n\nUsually the Microsoft Edge WebView2 Runtime is missing. Install it from https://developer.microsoft.com/microsoft-edge/webview2/ and open Endeavor again. If it's installed, install Endeavor again, so its folder can be written to.\n\n({error})"
    ));
    let title = wide("Endeavor");
    // SAFETY: null-terminated wide strings that outlive the call; no owner window.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR) };
    std::process::exit(1)
}

#[cfg(not(windows))]
pub fn application() -> gpui::Application {
    gpui_platform::application()
}

#[cfg(windows)]
pub fn application() -> gpui::Application {
    const VAR: &str = "GPUI_DISABLE_DIRECT_COMPOSITION";
    let ours = std::env::var_os(VAR).is_none();
    // SAFETY: on Windows, std's set_var and remove_var are safe even with other
    // threads running (GPUI may have started some by the time we remove it).
    if ours {
        unsafe { std::env::set_var(VAR, "1") };
    }
    let app = gpui_platform::application();
    if ours {
        unsafe { std::env::remove_var(VAR) };
    }
    app
}

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

#[cfg(target_os = "linux")]
pub fn webview_parent(window: &gpui::Window) -> XlibParent {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle, XlibWindowHandle};
    let raw = HasWindowHandle::window_handle(window).expect("window handle").as_raw();
    XlibParent(match raw {
        RawWindowHandle::Xcb(xcb) => RawWindowHandle::Xlib(XlibWindowHandle::new(xcb.window.get() as _)),
        other => other,
    })
}

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

#[cfg(windows)]
#[path = "snapshot_windows.rs"]
pub mod snapshot;

#[cfg(not(target_os = "macos"))]
pub mod dialogs {
    pub fn show_page_dialogs(_: &wry::WebView) {}
}

#[cfg(windows)]
#[path = "overlay_windows.rs"]
pub mod overlay;

#[cfg(windows)]
#[path = "webview_windows.rs"]
pub mod webcontent;
