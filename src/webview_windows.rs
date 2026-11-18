//! Windows' `src/webcontent.rs`: WebView2's process ending, its find in the
//! page, whether it has the keyboard, and the app's shortcuts while it does.
//! Linux's versions are `src/linux/webcontent.rs` and `src/linux/webkeys.rs`.

use std::cell::{Cell, RefCell};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use futures::channel::oneshot;
use gpui::{AnyWindowHandle, App, AsyncApp, Keystroke, Modifiers};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{AcceleratorKeyPressedEventHandler, FindMatchCountChangedEventHandler, FindStartCompletedHandler, FocusChangedEventHandler, ProcessFailedEventHandler};
use windows_core::{HSTRING, Interface};
use wry::WebViewExtWindows;

/// The menu bar's shortcuts on macOS, which take their keys before the page
/// does (`src/webkeys.rs`), less ⌘M: Ctrl+M is Pluto's, and Windows has no
/// such shortcut. WebView2 doesn't say which keys the page used, so the
/// app's other Ctrl shortcuts stay with the page, unlike on Linux.
const APP_FIRST: [&str; 10] = ["secondary-q", "secondary-,", "secondary-b", "secondary-=", "secondary--", "secondary-0", "secondary-shift-e", "secondary-f", "secondary-g", "secondary-shift-g"];

thread_local! {
    /// WebView2 has the keyboard: its controller's GotFocus came after its LostFocus.
    static FOCUSED: Cell<bool> = const { Cell::new(false) };
    /// The web view had the keyboard when GPUI's window was last clicked.
    static HAD_KEYBOARD: Cell<bool> = const { Cell::new(false) };
    static FINDING: RefCell<Finding> = RefCell::new(Finding::default());
}

#[derive(Default)]
struct Finding {
    /// The text last searched, so the same text goes to the next match.
    text: Option<String>,
    /// The latest search's answer, until WebView2 has counted its matches.
    answer: Option<oneshot::Sender<bool>>,
    /// The page's find session, with our count handler on it.
    find: Option<ICoreWebView2Find>,
}

fn answer(found: bool) {
    if let Some(tx) = FINDING.with(|f| f.borrow_mut().answer.take()) {
        let _ = tx.send(found);
    }
}

/// Keyboard handling for the notebook's web view in `window`: track its focus,
/// and send the app's shortcuts to GPUI before the page sees them.
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
        let _ = controller.add_LostFocus(&FocusChangedEventHandler::create(Box::new(|_, _| {
            FOCUSED.with(|f| f.set(false));
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

/// Dispatch each shortcut to `window`'s GPUI key handling. A shortcut that
/// moves GPUI's focus (Ctrl+, focuses Settings' search, Ctrl+F the find bar)
/// brings the keyboard to GPUI's window, `gpui`, too.
fn forward(handle: AnyWindowHandle, gpui: usize, mut keys: UnboundedReceiver<Keystroke>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(keystroke) = keys.next().await {
            let _ = handle.update(cx, |_, window, cx| {
                let before = window.focused(cx);
                window.dispatch_keystroke(keystroke, cx);
                // After the action's own deferred work.
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

/// The keystroke for a virtual key with the modifiers held now, for the keys
/// the app's shortcuts use: letters, digits, `=`, `-` and `,`.
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

/// The page's renderer ended: a crash, or Windows ending it for memory.
/// Loading the page again starts a new one. The browser process ending takes
/// the whole web view with it, which a load doesn't bring back.
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

/// The page's find session, made once; None on a WebView2 runtime too old to have one.
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
                answer(count > 0);
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

/// The next match of `text` (or the previous one), highlighting them all with
/// WebView2's own find, without its find bar. Answers whether there's a match.
pub fn find(webview: &wry::WebView, text: &str, backwards: bool) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel();
    let Some(find) = find_session(webview) else {
        let _ = tx.send(false);
        return rx;
    };
    let same = FINDING.with(|f| {
        let mut f = f.borrow_mut();
        let same = f.text.as_deref() == Some(text);
        f.text = Some(text.to_owned());
        same
    });
    if same {
        // SAFETY: as in `attach`.
        let _ = unsafe { if backwards { find.FindPrevious() } else { find.FindNext() } };
        let _ = tx.send(match_count(&find) > 0);
        return rx;
    }
    // A newer search replaces the one still counting; its answer goes.
    FINDING.with(|f| f.borrow_mut().answer = Some(tx));
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
                    // Counted already; otherwise MatchCountChanged answers.
                    // ponytail: a search with no matches may only be answered by MatchCountChanged; untried on Windows.
                    if match_count(&session) > 0 {
                        answer(true);
                    }
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
        f.find.clone()
    });
    if let Some(find) = find {
        // SAFETY: as in `attach`.
        let _ = unsafe { find.Stop() };
    }
}

pub fn give_keyboard(webview: &wry::WebView) {
    let _ = webview.focus();
}

pub fn has_keyboard(_: &wry::WebView) -> bool {
    FOCUSED.with(Cell::get)
}

/// GPUI's window was clicked (`web_view_hooks`), before it takes the keyboard.
pub fn note_click() {
    HAD_KEYBOARD.with(|h| h.set(FOCUSED.with(Cell::get)));
}

/// The web view has the keyboard, or had it until GPUI's window was clicked.
pub fn had_keyboard(_: &wry::WebView) -> bool {
    FOCUSED.with(Cell::get) || HAD_KEYBOARD.with(Cell::get)
}

// ponytail: the app menu's Edit items act on GPUI's text boxes only; WebView2's clipboard commands aren't ported.
pub fn edit(_: &wry::WebView, _: gpui::OsAction) {}
