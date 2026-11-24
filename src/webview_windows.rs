use std::cell::{Cell, RefCell};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use futures::channel::oneshot;
use gpui::{AnyWindowHandle, App, AsyncApp, Keystroke, Modifiers};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{AcceleratorKeyPressedEventHandler, FindMatchCountChangedEventHandler, FindStartCompletedHandler, FocusChangedEventHandler, ProcessFailedEventHandler};
use windows_core::{HSTRING, Interface};
use wry::WebViewExtWindows;

const APP_FIRST: [&str; 10] = ["ctrl-q", "ctrl-,", "ctrl-b", "ctrl-=", "ctrl--", "ctrl-0", "ctrl-shift-e", "ctrl-f", "ctrl-g", "ctrl-shift-g"];

thread_local! {
    static FOCUSED: Cell<bool> = const { Cell::new(false) };
    static HAD_KEYBOARD: Cell<bool> = const { Cell::new(false) };
    static FINDING: RefCell<Finding> = RefCell::new(Finding::default());
}

const SETTLE_MS: u32 = 400;

#[derive(Default)]
struct Finding {
    text: Option<String>,
    answer: Option<oneshot::Sender<bool>>,
    find: Option<ICoreWebView2Find>,
    timer: usize,
}

impl Finding {
    fn stop_timer(&mut self) {
        if self.timer != 0 {
            // SAFETY: our own thread timer, on the thread that set it.
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::KillTimer(std::ptr::null_mut(), self.timer) };
            self.timer = 0;
        }
    }
}

fn answer(found: bool) {
    if let Some(tx) = FINDING.with(|f| f.borrow_mut().answer.take()) {
        let _ = tx.send(found);
    }
}

fn settle() {
    unsafe extern "system" fn settled(_: windows_sys::Win32::Foundation::HWND, _: u32, id: usize, _: u32) {
        // SAFETY: our own thread timer.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::KillTimer(std::ptr::null_mut(), id) };
        let find = FINDING.with(|f| {
            let mut f = f.borrow_mut();
            if f.timer == id {
                f.timer = 0;
            }
            f.find.clone()
        });
        if let Some(find) = find {
            answer(match_count(&find) > 0);
        }
    }
    FINDING.with(|f| {
        let mut f = f.borrow_mut();
        f.stop_timer();
        // SAFETY: a thread timer on GPUI's thread, whose message loop runs `settled`.
        f.timer = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetTimer(std::ptr::null_mut(), 0, SETTLE_MS, Some(settled)) };
    });
}

pub fn attach(webview: &wry::WebView, window: &gpui::Window, cx: &mut App) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let controller = webview.controller();
    let mut token = 0i64;
    // SAFETY: COM calls on the UI thread that made the controller; the handlers live as long as it does.
    unsafe {
        let _ = controller.add_GotFocus(&FocusChangedEventHandler::create(Box::new(|_, _| {
            FOCUSED.with(|f| f.set(true));
            Ok(())
        })), &mut token);
        let page = webview.webview();
        let _ = controller.add_LostFocus(&FocusChangedEventHandler::create(Box::new(move |_, _| {
            FOCUSED.with(|f| f.set(false));
            let _ = page.ExecuteScript(&HSTRING::from(SAVE_FOCUS), None);
            Ok(())
        })), &mut token);
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
    let gpui = handle.hwnd.get() as usize;
    let window = window.window_handle();
    let (tx, rx) = unbounded::<Keystroke>();
    forward(window, gpui, rx, cx);
    // SAFETY: as above.
    unsafe {
        let _ = controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                    return Ok(());
                }
                let mut key = 0u32;
                args.VirtualKey(&mut key)?;
                if let Some(keystroke) = keystroke(key).filter(app_first) {
                    args.SetHandled(true)?;
                    let _ = tx.unbounded_send(keystroke);
                }
                Ok(())
            })),
            &mut token,
        );
    }
}

fn forward(handle: AnyWindowHandle, gpui: usize, mut keys: UnboundedReceiver<Keystroke>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(keystroke) = keys.next().await {
            let _ = handle.update(cx, |_, window, cx| {
                let before = window.focused(cx);
                window.dispatch_keystroke(keystroke, cx);
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        if window.focused(cx) != before {
                            // SAFETY: GPUI's own window, on its thread.
                            unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(gpui as _) };
                        }
                    });
                });
            });
        }
    })
    .detach();
}

fn key_down(key: i32) -> bool {
    // SAFETY: reads this thread's keyboard state, which WebView2's window shares.
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(key) < 0 }
}

fn keystroke(key: u32) -> Option<Keystroke> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let name = match key as u16 {
        k @ 0x41..=0x5A => char::from(k as u8).to_ascii_lowercase().to_string(),
        k @ 0x30..=0x39 => char::from(k as u8).to_string(),
        VK_NUMPAD0 => "0".into(),
        VK_OEM_PLUS | VK_ADD => "=".into(),
        VK_OEM_MINUS | VK_SUBTRACT => "-".into(),
        VK_OEM_COMMA => ",".into(),
        _ => return None,
    };
    let modifiers = Modifiers {
        control: key_down(VK_CONTROL as i32),
        alt: key_down(VK_MENU as i32),
        shift: key_down(VK_SHIFT as i32),
        platform: key_down(VK_LWIN as i32) || key_down(VK_RWIN as i32),
        function: false,
    };
    Some(Keystroke { modifiers, key: name, key_char: None })
}

fn app_first(keystroke: &Keystroke) -> bool {
    APP_FIRST.iter().filter_map(|s| Keystroke::parse(s).ok()).any(|k| k.modifiers == keystroke.modifiers && k.key == keystroke.key)
}

pub fn on_process_ended(webview: &wry::WebView, ended: impl Fn() + 'static) {
    let mut token = 0i64;
    // SAFETY: as in `attach`.
    unsafe {
        let _ = webview.webview().add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                eprintln!("A WebView2 process ended: kind {}", kind.0);
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
                    ended();
                }
                Ok(())
            })),
            &mut token,
        );
    }
}

pub fn url(webview: &wry::WebView) -> String {
    webview.url().unwrap_or_default()
}

fn find_session(webview: &wry::WebView) -> Option<ICoreWebView2Find> {
    if let Some(find) = FINDING.with(|f| f.borrow().find.clone()) {
        return Some(find);
    }
    // SAFETY: as in `attach`.
    let find = unsafe { webview.webview().cast::<ICoreWebView2_28>().ok()?.Find().ok()? };
    let mut token = 0i64;
    // SAFETY: as in `attach`.
    unsafe {
        let _ = find.add_MatchCountChanged(
            &FindMatchCountChangedEventHandler::create(Box::new(|find, _| {
                let mut count = 0;
                if let Some(find) = find {
                    find.MatchCount(&mut count)?;
                }
                if count > 0 {
                    answer(true);
                } else {
                    settle();
                }
                Ok(())
            })),
            &mut token,
        );
    }
    FINDING.with(|f| f.borrow_mut().find = Some(find.clone()));
    Some(find)
}

fn match_count(find: &ICoreWebView2Find) -> i32 {
    let mut count = 0;
    // SAFETY: as in `attach`.
    let _ = unsafe { find.MatchCount(&mut count) };
    count
}

pub fn find(webview: &wry::WebView, text: &str, backwards: bool) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel();
    let Some(find) = find_session(webview) else {
        let _ = tx.send(false);
        return rx;
    };
    let same = FINDING.with(|f| {
        let mut f = f.borrow_mut();
        let same = f.text.as_deref() == Some(text);
        if !same {
            f.stop_timer();
        }
        f.text = Some(text.to_owned());
        same
    });
    let counting = FINDING.with(|f| f.borrow_mut().answer.replace(tx).is_some());
    if same {
        // SAFETY: as in `attach`.
        let _ = unsafe { if backwards { find.FindPrevious() } else { find.FindNext() } };
        if !counting {
            answer(match_count(&find) > 0);
        }
        return rx;
    }
    let start = || -> windows_core::Result<()> {
        // SAFETY: as in `attach`.
        unsafe {
            let options = webview.environment().cast::<ICoreWebView2Environment15>()?.CreateFindOptions()?;
            options.SetFindTerm(&HSTRING::from(text))?;
            options.SetIsCaseSensitive(false)?;
            options.SetShouldHighlightAllMatches(true)?;
            options.SetSuppressDefaultFindDialog(true)?;
            let session = find.clone();
            find.Start(
                &options,
                &FindStartCompletedHandler::create(Box::new(move |result| {
                    result?;
                    if backwards {
                        session.FindPrevious()?;
                    }
                    settle();
                    Ok(())
                })),
            )
        }
    };
    if let Err(e) = start() {
        eprintln!("WebView2's find didn't start: {e}");
        answer(false);
    }
    rx
}

pub fn clear_find(_: &wry::WebView) {
    let find = FINDING.with(|f| {
        let mut f = f.borrow_mut();
        f.text = None;
        f.answer = None;
        f.stop_timer();
        f.find.clone()
    });
    if let Some(find) = find {
        // SAFETY: as in `attach`.
        let _ = unsafe { find.Stop() };
    }
}

thread_local! {
    static FOCUS_NEXT: RefCell<Option<(ICoreWebView2Controller, ICoreWebView2)>> = const { RefCell::new(None) };
}

const SAVE_FOCUS: &str = "(() => { const e = document.activeElement, s = getSelection(); \
window.__endeavorFocus = e && e !== document.body ? [e, s.rangeCount ? s.getRangeAt(0).cloneRange() : null] : null })()";

const RESTORE_FOCUS: &str = "(() => { const f = window.__endeavorFocus; if (!f || !f[0].isConnected) return; \
if (document.activeElement !== f[0]) f[0].focus(); if (f[1]) { const s = getSelection(); s.removeAllRanges(); s.addRange(f[1]) } })()";

pub fn give_keyboard(webview: &wry::WebView) {
    unsafe extern "system" fn focus_now(_: windows_sys::Win32::Foundation::HWND, _: u32, id: usize, _: u32) {
        // SAFETY: our own thread timer.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::KillTimer(std::ptr::null_mut(), id) };
        if let Some((controller, page)) = FOCUS_NEXT.with(|f| f.borrow_mut().take()) {
            // SAFETY: as in `attach`.
            unsafe {
                let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
                let _ = page.ExecuteScript(&HSTRING::from(RESTORE_FOCUS), None);
            }
        }
    }
    FOCUS_NEXT.with(|f| f.replace(Some((webview.controller(), webview.webview()))));
    // SAFETY: a thread timer on GPUI's thread, whose message loop runs `focus_now`.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetTimer(std::ptr::null_mut(), 0, 10, Some(focus_now)) };
}

pub fn has_keyboard(_: &wry::WebView) -> bool {
    FOCUSED.with(Cell::get)
}

pub fn note_click() {
    HAD_KEYBOARD.with(|h| h.set(FOCUSED.with(Cell::get)));
}

pub fn had_keyboard(_: &wry::WebView) -> bool {
    FOCUSED.with(Cell::get) || HAD_KEYBOARD.with(Cell::get)
}

pub fn edit(webview: &wry::WebView, action: gpui::OsAction) {
    let (key, command) = match action {
        gpui::OsAction::Cut => ('X', "cut"),
        gpui::OsAction::Copy => ('C', "copy"),
        gpui::OsAction::Paste => ('V', "paste"),
        gpui::OsAction::SelectAll => ('A', "selectAll"),
        _ => return,
    };
    let page = webview.webview();
    let lower = key.to_ascii_lowercase();
    let down = format!(r#"{{"type":"keyDown","modifiers":2,"key":"{lower}","code":"Key{key}","windowsVirtualKeyCode":{},"commands":["{command}"]}}"#, key as u32);
    let up = format!(r#"{{"type":"keyUp","modifiers":2,"key":"{lower}","code":"Key{key}","windowsVirtualKeyCode":{}}}"#, key as u32);
    for params in [down, up] {
        // SAFETY: a COM call on the UI thread that made the web view.
        if let Err(e) = unsafe { page.CallDevToolsProtocolMethod(&HSTRING::from("Input.dispatchKeyEvent"), &HSTRING::from(params), None) } {
            eprintln!("Edit menu {command} in the notebook: {e}");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_app_first_key_parses() {
        for key in super::APP_FIRST {
            let keystroke = gpui::Keystroke::parse(key).unwrap_or_else(|_| panic!("{key} doesn't parse"));
            assert!(keystroke.modifiers.control, "{key} has no Ctrl");
        }
    }
}
