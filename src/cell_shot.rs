//! The agent's picture of one cell's output (`view_cell_output`) when the
//! runtime has none: an HTML or JS output (a widget, an interactive plot) as
//! Pluto draws it. The listener asks for it (`endeavor_mcp::Snapshot`), and the
//! app takes it from a second web view of the same notebook, behind the user's
//! one, scrolled to the cell. The user's view never moves. A rect off screen in
//! the user's own view comes back blank; a second view scrolled to it draws
//! HTML and WebGL alike, even a canvas that drew once (Mac spikes,
//! notes/html-output-images-plan.md).
//!
//! The second view is another Pluto client of the notebook. It loads the page
//! as a reload would, so it shows the outputs as Pluto draws them fresh: what
//! the user did inside a widget (a selection) isn't in the picture. It has none
//! of the user's view's setup (page.js), so Pluto there loads JuliaMono from its
//! CDN: offline, code in a picture is in another monospace font.
//!
//! On Linux the second view is an X child window stacked under the user's
//! one: WebKitGTK draws a hidden view's snapshot too, but runs no animation
//! frames there, so a plot drawn from one would be blank.
//!
//! On Windows the second view is a child window under the user's one, in the
//! same place and with the same holes, and the picture is WebView2's
//! (`snapshot_windows.rs`).

use std::time::Duration;

use crate::hosts::HostId;

/// Why there's no picture while the pane isn't on screen.
const NOT_ON_SCREEN: &str = "the notebook pane isn't on screen";

/// One picture the listener waits for.
pub struct Ask {
    pub host: HostId,
    pub notebook: String,
    pub cell: String,
    /// When the listener asked: once it has stopped waiting (`WAIT`), the ask is dropped.
    pub asked: std::time::Instant,
    /// The picture; None when there's none.
    pub reply: std::sync::mpsc::Sender<Option<endeavor_mcp::Picture>>,
}

/// How long the listener waits for a picture. Then the agent gets the
/// runtime's answer, as it would without the app.
const WAIT: Duration = Duration::from_secs(10);

/// How long the app tries for one: under `WAIT`, and long enough for the
/// second view's first load of a notebook.
pub const DEADLINE: Duration = Duration::from_secs(8);

/// A second view is let go after this long without a picture, so it isn't a
/// Pluto client for the rest of the day.
pub const IDLE: Duration = Duration::from_secs(300);

/// The listener's side: each picture is asked of the app on `asks`, and waited for.
pub fn snapshot(host: HostId, asks: futures::channel::mpsc::UnboundedSender<Ask>) -> Box<endeavor_mcp::Pictures> {
    let asks = std::sync::Mutex::new(asks);
    Box::new(move |_session, notebook, cell| {
        let (reply, answer) = std::sync::mpsc::channel();
        let ask = Ask { host: host.clone(), notebook: notebook.to_owned(), cell: cell.to_owned(), asked: std::time::Instant::now(), reply };
        asks.lock().unwrap().unbounded_send(ask).ok()?;
        answer.recv_timeout(WAIT).ok().flatten()
    })
}

/// The page script that finds the cell's output and scrolls it into view:
/// "scrolled", "busy" (it is running or queued), "empty" (nothing to see: a
/// cell that hasn't run, or safe preview's "Code not executed"), or "absent"
/// (the page hasn't drawn the cell yet). An output shorter than the window
/// goes to the middle, clear of Pluto's bars at the top; a taller one to the top.
/// `cell` is a UUID (the library checked it).
pub fn find_script(cell: &str) -> String {
    format!(
        r#"(() => {{
  const out = document.querySelector('pluto-cell[id="{cell}"] > pluto-output');
  if (!out) return "absent";
  const c = out.parentElement.classList;
  if (!out.firstElementChild || c.contains("no_output_yet") || out.querySelector(".safe-preview-output") || out.getBoundingClientRect().height < 2) return "empty";
  if (c.contains("running") || c.contains("queued")) return "busy";
  out.scrollIntoView({{ block: out.getBoundingClientRect().height < innerHeight * 0.8 ? "center" : "start" }});
  return "scrolled";
}})()"#
    )
}

/// The page script that measures the output: its rect and the viewport's size, in CSS pixels, and
/// its whole height. First it hides what floats over the page (Pluto's bars, its Live docs button):
/// this view is only for pictures (#163). A bar still over the top of the page is cut off the top
/// of a tall output; "covered" if something else on the page (a popup) is drawn over the middle of
/// what's left in view. A hit on an element around the output counts as the output: the point fell
/// through a layer of it that takes no clicks (`pointer-events: none`).
pub fn measure_script(cell: &str) -> String {
    format!(
        r#"(() => {{
  const out = document.querySelector('pluto-cell[id="{cell}"] > pluto-output');
  if (!out) return null;
  for (const el of document.body.querySelectorAll("*")) {{
    if (el.contains(out) || out.contains(el)) continue;
    const position = getComputedStyle(el).position;
    if (position === "fixed" || position === "sticky") {{
      el.style.setProperty("opacity", "0", "important");
      el.style.setProperty("pointer-events", "none", "important");
    }}
  }}
  const r = out.getBoundingClientRect();
  const x = r.left + r.width / 2, bottom = Math.min(r.bottom, innerHeight);
  let top = Math.max(r.top, 0);
  while (top < bottom) {{
    const at = document.elementFromPoint(x, top + 1);
    if (!at || out.contains(at) || at.contains(out)) break;
    const bar = at.getBoundingClientRect().bottom;
    top = bar > top ? bar : top + 4;
  }}
  const over = document.elementFromPoint(x, (top + bottom) / 2);
  if (top >= bottom || (over && !out.contains(over) && !over.contains(out))) return "covered";
  return [r.left, top, r.width, r.bottom - top, innerWidth, innerHeight, r.height];
}})()"#
    )
}

/// When a picture stops short of its output: (the height shown, the output's whole height).
pub type Cut = Option<(u32, u32)>;

/// A picture being taken: the PNG and its cut, or why there's none.
type ShotJob = futures::future::LocalBoxFuture<'static, Result<(Vec<u8>, Cut), &'static str>>;

/// The part of the output inside the viewport, as x, y, width, height, from
/// `measure_script`'s answer; None if there's none worth a picture. With it, when the output is
/// taller than that part: (the height shown, the output's whole height), for the agent (#163).
pub fn visible_rect(measured: &str) -> Option<([f64; 4], Cut)> {
    let v: Vec<f64> = serde_json::from_str(measured).ok()?;
    let [left, top, width, height, view_width, view_height, whole] = v[..] else { return None };
    let (x, y) = (left.max(0.), top.max(0.));
    let (w, h) = ((left + width).min(view_width) - x, (top + height).min(view_height) - y);
    let cut = (whole - h >= 2.).then(|| (h.round() as u32, whole.round() as u32));
    (w >= 2. && h >= 2.).then_some(([x, y, w, h], cut))
}

#[cfg(target_os = "macos")]
pub use mac::Shooter;
#[cfg(target_os = "linux")]
pub use linux::Shooter;
#[cfg(windows)]
pub use windows::Shooter;

#[cfg(target_os = "macos")]
mod mac {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use wry::WebViewExtMacOS;

    use crate::overlay::CGRect;

    /// The second web view, the page it has loaded, and the user's view it hides behind.
    pub struct Shooter {
        pub view: wry::WebView,
        /// Cleared when its web content process ends, so the next picture loads the page again.
        pub url: std::rc::Rc<std::cell::RefCell<String>>,
        main: Retained<AnyObject>,
    }

    impl Shooter {
        pub fn new(window: &gpui::Window, main: &wry::WebView) -> Option<Shooter> {
            let view = wry::WebViewBuilder::new().with_url("about:blank").build_as_child(&crate::platform::webview_parent(window)).ok()?;
            let _ = view.set_visible(false);
            let url = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            // macOS ends a hidden view's process first when memory is short. That is this view's
            // own: the user's notebook is fine and isn't loaded again.
            let page = url.clone();
            crate::webcontent::on_process_ended(&view, move || {
                eprintln!("The second web view's web content process ended; the next cell picture loads its page again");
                page.borrow_mut().clear();
            });
            let main = unsafe { Retained::cast_unchecked(main.webview()) };
            crate::overlay::set_second(Some(&view));
            Some(Shooter { view, url, main })
        }

        /// Show `url`, unless it is the page already loaded. Whether it loads now.
        pub fn load(&self, url: &str) -> bool {
            if *self.url.borrow() == url {
                return false;
            }
            let _ = self.view.load_url(url);
            *self.url.borrow_mut() = url.to_owned();
            true
        }

        /// Whether the user's view is shown, not dimmed, in a window on screen and not covered:
        /// behind anything else the second view would show, and a hidden window draws nothing.
        /// Settings or a dialog over the pane doesn't hide the user's view but dims it
        /// (`overlay::set_dimmed`), and the second view would show through. The holes a tip, a
        /// menu or a notice cuts in it are cut in the second view too, at once
        /// (`overlay::set_second`).
        pub fn pane_shows(&self) -> bool {
            unsafe {
                let hidden: bool = msg_send![&*self.main, isHiddenOrHasHiddenAncestor];
                let window: *mut AnyObject = msg_send![&*self.main, window];
                if hidden || window.is_null() {
                    return false;
                }
                let alpha: f64 = msg_send![&*self.main, alphaValue];
                if alpha < 1. {
                    return false;
                }
                // NSWindowOcclusionStateVisible.
                let state: usize = msg_send![window, occlusionState];
                state & (1 << 1) != 0
            }
        }

        /// Put the second view exactly behind the user's view, where it draws but can't be seen,
        /// and keep it there: called again as the picture is taken, it follows the pane's
        /// frame. False, with the second view hidden, once the pane isn't on screen.
        pub fn keep_behind(&self) -> bool {
            if !self.pane_shows() {
                self.hide();
                return false;
            }
            unsafe {
                let ours = self.view.webview();
                let frame: CGRect = msg_send![&*self.main, frame];
                let now: CGRect = msg_send![&*ours, frame];
                if now != frame || !self.shown() {
                    let _: () = msg_send![&*ours, setFrame: frame];
                    // Its holes' mask is drawn for its bounds.
                    crate::overlay::set_second(Some(&self.view));
                }
                // Directly under the user's view among its siblings. Both are built on the same
                // parent, so sharing a superview isn't enough: the second view, added last, starts
                // on top (#259). Moved only when it isn't there, before it's shown.
                let superview: *mut AnyObject = msg_send![&*self.main, superview];
                if !superview.is_null() && !just_below(superview, &*ours, &*self.main) {
                    // NSWindowBelow.
                    let _: () = msg_send![superview, addSubview: &*ours, positioned: -1isize, relativeTo: &*self.main];
                }
            }
            if !self.shown() {
                // The theme may have changed since the last picture.
                crate::settings::set_webview_appearance(&self.view, crate::theme::is_light());
                let _ = self.view.set_visible(true);
            }
            true
        }

        pub fn shown(&self) -> bool {
            unsafe {
                let hidden: bool = msg_send![&*self.view.webview(), isHidden];
                !hidden
            }
        }

        pub fn hide(&self) {
            let _ = self.view.set_visible(false);
        }
    }

    /// Whether `ours` is a subview of `superview` directly below `main` (the next one in front of
    /// it, in `subviews`' back-to-front order).
    unsafe fn just_below<A: objc2::Message, B: objc2::Message>(superview: *mut AnyObject, ours: &A, main: &B) -> bool {
        unsafe {
            let subviews: *mut AnyObject = msg_send![superview, subviews];
            let ours: usize = msg_send![subviews, indexOfObjectIdenticalTo: ours];
            let main: usize = msg_send![subviews, indexOfObjectIdenticalTo: main];
            // NSNotFound is NSIntegerMax.
            ours != isize::MAX as usize && ours.checked_add(1) == Some(main)
        }
    }

    impl Drop for Shooter {
        fn drop(&mut self) {
            crate::overlay::set_second(None);
            crate::webcontent::forget(&self.view);
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use gtk::gdk;
    use gtk::prelude::*;

    use crate::overlay;

    /// The second web view, the page it has loaded, and the user's view's X window it hides behind.
    pub struct Shooter {
        pub view: wry::WebView,
        /// Cleared when its web content process ends, so the next picture loads the page again.
        pub url: std::rc::Rc<std::cell::RefCell<String>>,
        main: gdk::Window,
    }

    impl Shooter {
        pub fn new(window: &gpui::Window, main: &wry::WebView) -> Option<Shooter> {
            let main = overlay::window_of(main)?;
            let parent = crate::platform::webview_parent(window);
            let view = wry::WebViewBuilder::new().with_url("about:blank").build_as_child(&parent).ok()?;
            // Shown to GTK from the start, and hidden by unmapping its X window: shown again
            // through GTK, a view keeps the size GTK picks for it, not the pane's.
            overlay::window_of(&view)?.hide();
            let url = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            let page = url.clone();
            crate::webcontent::on_process_ended(&view, move || {
                eprintln!("The second web view's web content process ended; the next cell picture loads its page again");
                page.borrow_mut().clear();
            });
            // The user's view's holes are cut in it as they open and close.
            overlay::set_second(Some(&view));
            Some(Shooter { view, url, main })
        }

        /// Show `url`, unless it is the page already loaded. Whether it loads now.
        pub fn load(&self, url: &str) -> bool {
            if *self.url.borrow() == url {
                return false;
            }
            let _ = self.view.load_url(url);
            *self.url.borrow_mut() = url.to_owned();
            true
        }

        /// Whether the user's view is mapped in a mapped window. Behind anything else the second
        /// view would show. X has no occlusion here, and a covered window still draws.
        pub fn pane_shows(&self) -> bool {
            overlay::viewable(&self.main)
        }

        /// Put the second view exactly behind the user's view, at the bottom of GPUI's child
        /// windows, and keep it there: called again as the picture
        /// is taken, it follows the pane. False, with the second view hidden, once the pane isn't
        /// on screen.
        pub fn keep_behind(&self) -> bool {
            if !self.pane_shows() {
                self.hide();
                return false;
            }
            let Some(ours) = overlay::window_of(&self.view) else { return false };
            if !overlay::has_second() {
                overlay::set_second(Some(&self.view));
            }
            // As wry places the user's view: GDK's units, which wry takes to be logical pixels.
            // Placed and lowered before it's mapped, so it never shows over anything.
            use wry::WebViewExtUnix;
            let (x, y, width, height) = self.main.geometry();
            let laid_out = (self.view.webview().allocated_width(), self.view.webview().allocated_height());
            if ours.geometry() != (x, y, width, height) || laid_out != (width, height) {
                let _ = self.view.set_bounds(wry::Rect { position: wry::dpi::LogicalPosition::new(x, y).into(), size: wry::dpi::LogicalSize::new(width, height).into() });
            }
            ours.lower();
            if !self.shown() {
                // The theme may have changed since the last picture.
                crate::settings::set_webview_appearance(&self.view, crate::theme::is_light());
                ours.show_unraised();
            }
            true
        }

        pub fn shown(&self) -> bool {
            overlay::window_of(&self.view).is_some_and(|w| overlay::viewable(&w))
        }

        pub fn hide(&self) {
            if let Some(ours) = overlay::window_of(&self.view) {
                ours.hide();
            }
        }
    }

    impl Drop for Shooter {
        fn drop(&mut self) {
            overlay::set_second(None);
        }
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::MapWindowPoints;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetClientRect, GetParent, IsIconic, IsWindowVisible, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow};
    use wry::WebViewExtWindows;

    use crate::overlay;

    /// The second web view, the page it has loaded, and the user's view's window it hides behind.
    pub struct Shooter {
        pub view: wry::WebView,
        /// Cleared when its web content process ends, so the next picture loads the page again.
        pub url: std::rc::Rc<std::cell::RefCell<String>>,
        main: usize,
    }

    /// `hwnd`'s client area in its parent's client coordinates, in physical pixels.
    fn placed(hwnd: usize) -> Option<RECT> {
        let hwnd = hwnd as HWND;
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe {
            if GetClientRect(hwnd, &mut rect) == 0 {
                return None;
            }
            let mut corner = [POINT { x: 0, y: 0 }];
            MapWindowPoints(hwnd, GetParent(hwnd), corner.as_mut_ptr(), 1);
            Some(RECT { left: corner[0].x, top: corner[0].y, right: corner[0].x + rect.right, bottom: corner[0].y + rect.bottom })
        }
    }

    impl Shooter {
        pub fn new(window: &gpui::Window, main: &wry::WebView) -> Option<Shooter> {
            let main = overlay::window_of(main)?;
            let view = wry::WebViewBuilder::new().with_url("about:blank").with_visible(false).build_as_child(&crate::platform::webview_parent(window)).ok()?;
            let url = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            let page = url.clone();
            crate::webcontent::on_process_ended(&view, move || {
                eprintln!("The second web view's web content process ended; the next cell picture loads its page again");
                page.borrow_mut().clear();
            });
            // The user's view's holes are cut in it as they open and close.
            overlay::set_second(Some(&view));
            Some(Shooter { view, url, main })
        }

        /// Show `url`, unless it is the page already loaded. Whether it loads now.
        pub fn load(&self, url: &str) -> bool {
            if *self.url.borrow() == url {
                return false;
            }
            let _ = self.view.load_url(url);
            *self.url.borrow_mut() = url.to_owned();
            true
        }

        /// Whether the user's view is shown in a window that isn't minimized. Behind anything
        /// else the second view would show. A covered child window still draws.
        pub fn pane_shows(&self) -> bool {
            let main = self.main as HWND;
            unsafe { IsWindowVisible(main) != 0 && IsIconic(GetAncestor(main, GA_ROOT)) == 0 }
        }

        /// Put the second view exactly behind the user's view, just under it in the window's
        /// children, and keep it there: called again as the picture
        /// is taken, it follows the pane. False, with the second view hidden, once the pane isn't
        /// on screen.
        pub fn keep_behind(&self) -> bool {
            if !self.pane_shows() {
                self.hide();
                return false;
            }
            let (Some(ours), Some(want)) = (overlay::window_of(&self.view), placed(self.main)) else { return false };
            if !overlay::has_second() {
                overlay::set_second(Some(&self.view));
            }
            let now = placed(ours);
            if now.is_none_or(|r| (r.left, r.top, r.right, r.bottom) != (want.left, want.top, want.right, want.bottom)) {
                let _ = self.view.set_bounds(wry::Rect {
                    position: wry::dpi::PhysicalPosition::new(want.left, want.top).into(),
                    size: wry::dpi::PhysicalSize::new(want.right - want.left, want.bottom - want.top).into(),
                });
            }
            // Just under the user's view, before it's shown, so it never shows over anything.
            unsafe { SetWindowPos(ours as HWND, self.main as HWND, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
            if !self.shown() {
                // The theme may have changed since the last picture.
                crate::settings::set_webview_appearance(&self.view, crate::theme::is_light());
                // Not wry's set_visible, whose SW_SHOW may activate: shown without
                // activating, so the keyboard stays in the user's cell.
                unsafe { ShowWindow(ours as HWND, SW_SHOWNA) };
                let _ = unsafe { self.view.controller().SetIsVisible(true) };
            }
            true
        }

        pub fn shown(&self) -> bool {
            overlay::window_of(&self.view).is_some_and(|w| unsafe { IsWindowVisible(w as HWND) } != 0)
        }

        pub fn hide(&self) {
            let _ = self.view.set_visible(false);
        }
    }

    impl Drop for Shooter {
        fn drop(&mut self) {
            overlay::set_second(None);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_the_part_of_the_output_in_view() {
        assert_eq!(visible_rect("[20, 0, 600, 300, 900, 1200, 300]"), Some(([20., 0., 600., 300.], None)));
        assert_eq!(visible_rect("[20, 900, 600, 900, 900, 1200, 900]"), Some(([20., 900., 600., 300.], Some((300, 900)))), "cut at the bottom, and said so");
        assert_eq!(visible_rect("[-10, -5, 600, 300, 400, 1200, 305]"), Some(([0., 0., 400., 295.], Some((295, 305)))), "and at the edges");
        assert_eq!(visible_rect("[0, 0, 600, 855, 900, 855, 920]"), Some(([0., 0., 600., 855.], Some((855, 920)))), "a tall output, from its top (the Mac check)");
        assert_eq!(visible_rect("[0, 40, 600, 300, 900, 1200, 300.5]"), Some(([0., 40., 600., 300.], None)), "half a pixel isn't a cut");
        assert_eq!(visible_rect("[20, 0, 600, 0, 900, 1200, 0]"), None, "an empty output");
        assert_eq!(visible_rect("[20, 0, 600, 300, 900, 1200]"), None, "an answer from before the whole height");
        assert_eq!(visible_rect("null"), None);
        assert_eq!(visible_rect("\"absent\""), None);
    }

    #[test]
    fn the_listener_gets_the_app_s_answer_or_none() {
        let (asks, mut taken) = futures::channel::mpsc::unbounded();
        let snapshot = snapshot(HostId::ThisMac, asks);
        let answering = std::thread::spawn(move || {
            let ask = futures::executor::block_on(futures::StreamExt::next(&mut taken)).unwrap();
            assert_eq!((ask.notebook.as_str(), ask.cell.as_str()), ("nb", "cell"));
            ask.reply.send(Some(endeavor_mcp::Picture { png: "cGljdHVyZQ==".into(), cut: Some((855, 920)) })).unwrap();
            // The app went away: no answer.
            drop(taken);
        });
        assert_eq!(snapshot("7", "nb", "cell"), Some(endeavor_mcp::Picture { png: "cGljdHVyZQ==".into(), cut: Some((855, 920)) }));
        answering.join().unwrap();
        assert_eq!(snapshot("7", "nb", "cell"), None);
    }
}

/// The second web view while it is kept, and how the last picture went (the state dump's `cell_shot`).
#[derive(Default)]
pub struct Second {
    shooter: Option<std::rc::Rc<Shooter>>,
    used: Option<std::time::Instant>,
    pub last: Option<&'static str>,
}

impl Second {
    /// The second view, for the state dump: "none" (not made, or let go), "hidden", or "shown"
    /// (behind the user's view, while a picture is taken).
    pub fn view(&self) -> &'static str {
        if let Some(shooter) = &self.shooter {
            return if shooter.shown() { "shown" } else { "hidden" };
        }
        "none"
    }

    /// Hide the second view, if it's shown, as soon as the user's view isn't on screen (hidden,
    /// dimmed behind Settings, its window minimized): it is never seen without the user's view in
    /// front of it. Called as the window draws, where the user's view is shown and hidden. The
    /// picture being taken then stops at its next step.
    pub fn follow_pane(&self) {
        if let Some(shooter) = &self.shooter
            && shooter.shown()
            && !shooter.pane_shows()
        {
            shooter.hide();
        }
    }
}

impl crate::Workspace {
    /// Take the picture `ask` waits for, one at a time. What happened.
    pub(crate) fn shoot_cell(&mut self, ask: Ask, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> futures::future::LocalBoxFuture<'static, &'static str> {
        match self.cell_shot_job(&ask, window, cx) {
            Ok(job) => Box::pin(async move {
                let (png, outcome) = match job.await {
                    Ok((png, cut)) => (Some(endeavor_mcp::Picture { png: crate::attach::base64(&png), cut }), if cut.is_some() { "picture, cut" } else { "picture" }),
                    Err(why) => (None, why),
                };
                let _ = ask.reply.send(png);
                outcome
            }),
            Err(why) => {
                let _ = ask.reply.send(None);
                Box::pin(async move { why })
            }
        }
    }

    /// After a picture: the outcome for the state dump, and the second view goes once it's idle.
    pub(crate) fn cell_shot_done(&mut self, outcome: &'static str, cx: &mut gpui::Context<Self>) {
        self.second.last = Some(outcome);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(IDLE).await;
            let _ = this.update(cx, |this, _| {
                if this.second.used.is_some_and(|used| used.elapsed() >= IDLE) {
                    this.second = Second { last: this.second.last, ..Second::default() };
                }
            });
        })
        .detach();
    }

    fn cell_shot_job(&mut self, ask: &Ask, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Result<ShotJob, &'static str> {
        use wire::backend::Backend;
        if self.notebook_kind(&ask.host, &ask.notebook) != Backend::Pluto {
            return Err("not a Pluto notebook");
        }
        let runtime = self.connection(&ask.host).and_then(|c| c.runtime.as_ref()).ok_or("no runtime")?;
        let url = Backend::Pluto.notebook_url(&runtime.page_url, &ask.notebook, "");
        if ask.asked.elapsed() >= WAIT {
            return Err("the agent stopped waiting");
        }
        if self.second.shooter.is_none() {
            self.second.shooter = Shooter::new(window, self.webview.read(cx).raw()).map(std::rc::Rc::new);
        }
        let shooter = self.second.shooter.clone().ok_or("no second web view")?;
        // Counts even when refused, so the idle second view is let go.
        self.second.used = Some(std::time::Instant::now());
        // ponytail: only while the notebook pane is on screen (`keep_behind`).
        if !shooter.keep_behind() {
            return Err(NOT_ON_SCREEN);
        }
        let fresh = shooter.load(&url);
        let executor = cx.background_executor().clone();
        let cell = ask.cell.clone();
        Ok(Box::pin(async move {
            let png = take(&shooter, &cell, fresh, &executor).await;
            shooter.hide();
            png
        }))
    }
}

/// Wait for the cell's output in the second view, scroll it into view, and take its picture. The
/// pane is checked between steps, and there is no picture once it isn't on screen. The second
/// view itself hides at once, as the user's view does (`Second::follow_pane`), and has the user's
/// view's holes as they open (`overlay::set_second`): the checks here only place it.
async fn take(shooter: &Shooter, cell: &str, fresh: bool, executor: &gpui::BackgroundExecutor) -> Result<(Vec<u8>, Cut), &'static str> {
    let deadline = std::time::Instant::now() + DEADLINE;
    let behind = || if shooter.keep_behind() { Ok(()) } else { Err(NOT_ON_SCREEN) };
    loop {
        let found = eval(&shooter.view, &find_script(cell), executor).await;
        behind()?;
        match found.as_deref() {
            Some("\"scrolled\"") => break,
            // An empty output gives the agent a blank picture: the runtime's answer says more.
            Some("\"empty\"") => return Err("the output is empty"),
            _ => {}
        }
        if std::time::Instant::now() > deadline {
            return Err("the cell's output wasn't drawn in time");
        }
        executor.timer(Duration::from_millis(200)).await;
        behind()?;
    }
    // Two frames after the scroll; and on a page just loaded, time for a widget that draws after it (WebGL).
    executor.timer(Duration::from_millis(if fresh { 800 } else { 100 })).await;
    behind()?;
    let measured = eval(&shooter.view, &measure_script(cell), executor).await.ok_or("the output couldn't be measured")?;
    behind()?;
    if measured == "\"covered\"" {
        return Err("the output is covered on the page");
    }
    let (rect, cut) = visible_rect(&measured).ok_or("the output is empty")?;
    let shot = crate::snapshot::png(&shooter.view, rect);
    let timeout = executor.timer(Duration::from_secs(3));
    let png = match futures::future::select(shot, timeout).await {
        futures::future::Either::Left((png, _)) => png.ok().flatten(),
        futures::future::Either::Right(_) => None,
    };
    png.map(|png| (png, cut)).ok_or("WebKit couldn't draw the output")
}

/// The JSON of what `js` gives in `view`; None if it fails or takes over a second.
async fn eval(view: &wry::WebView, js: &str, executor: &gpui::BackgroundExecutor) -> Option<String> {
    let (tx, rx) = futures::channel::oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    view.evaluate_script_with_callback(js, move |value| {
        if let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(value);
        }
    })
    .ok()?;
    let timeout = executor.timer(Duration::from_secs(1));
    match futures::future::select(rx, timeout).await {
        futures::future::Either::Left((value, _)) => value.ok(),
        futures::future::Either::Right(_) => None,
    }
}

/// Debug builds: while the file `ENDEAVOR_TEST_VIEW_CELL` names holds "<notebook id> <cell id>", the
/// app makes the agent's `view_cell_output` call for the open session on This Mac, through its own
/// listener as the agent would, and writes the result's text to `<file>.json` and its PNG, if
/// any, to `<file>.png`. Then it deletes the file. For checking the picture without an agent turn.
#[cfg(debug_assertions)]
pub fn test_view_cell(workspace: &crate::Workspace) -> Option<()> {
    let path = std::env::var_os("ENDEAVOR_TEST_VIEW_CELL")?;
    let asked = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let (notebook, cell) = asked.trim().split_once(' ')?;
    let key = workspace.active?;
    let bridge = workspace.connection(&HostId::ThisMac)?.runtime.as_ref()?.bridge.clone();
    let call = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "view_cell_output", "arguments": { "notebook_id": notebook, "cell_id": cell.trim() } } }).to_string();
    std::thread::spawn(move || {
        let out = |ext: &str| std::path::PathBuf::from(format!("{}.{ext}", path.to_string_lossy()));
        let result = view(&bridge, key, &call).unwrap_or_else(|why| serde_json::json!({ "failed": why }));
        let content = &result["content"];
        if let Some(png) = content[1]["data"].as_str().and_then(crate::attach::unbase64) {
            let _ = std::fs::write(out("png"), png);
        }
        let text = content[0]["text"].as_str().map_or_else(|| result.to_string(), str::to_owned);
        let _ = std::fs::write(out("json"), text);
    });
    Some(())
}

#[cfg(debug_assertions)]
fn view(bridge: &crate::pluto::Bridge, key: u64, call: &str) -> Result<serde_json::Value, String> {
    use std::io::{Read, Write};
    let address = bridge.url.trim_start_matches("http://").split('/').next().unwrap_or_default().to_owned();
    let mut stream = std::net::TcpStream::connect(&address).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nX-Endeavor-Session: {key}\r\nX-Endeavor-Client: app\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{call}",
        bridge.token,
        call.len()
    )
    .map_err(|e| e.to_string())?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;
    let (_, body) = response.split_once("\r\n\r\n").ok_or("malformed HTTP response")?;
    let message: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("bad JSON-RPC reply: {e}: {body}"))?;
    Ok(message["result"].clone())
}

/// Debug builds: the file `ENDEAVOR_TEST_ACTION` names holds one thing for the app to do as the
/// user would, so the Mac check needs no clicks. The app deletes the file, does it, and writes
/// what happened to `<file>.done`:
/// - `open <notebook path>`: start a session on This Mac in the notebook's folder, on that
///   notebook (it opens in safe preview), as the new-session screen does.
/// - `run`: the callout's Run notebook, for the notebook on screen.
/// - `settings` and `close settings`: open or close Settings (⌘, and Esc).
/// - `notice [error]`: the "Couldn't restart the notebook" notice, with `error` (`\n` for a
///   line break) or a long made-up stack trace under Details; `details` opens or closes them.
/// - `js <script>`: run the script in the user's notebook page; `.done` has the JSON of what it
///   gives once it has run (`null` for nothing).
#[cfg(debug_assertions)]
impl crate::Workspace {
    pub(crate) fn watch_test_actions(&mut self, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        let Some(path) = std::env::var_os("ENDEAVOR_TEST_ACTION").map(std::path::PathBuf::from) else { return };
        let done = std::path::PathBuf::from(format!("{}.done", path.to_string_lossy()));
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let Ok(action) = std::fs::read_to_string(&path) else { continue };
            let _ = std::fs::remove_file(&path);
            let done = done.clone();
            if this.update_in(cx, |this, window, cx| this.test_action(action.trim(), done, window, cx)).is_err() {
                return;
            }
        })
        .detach();
    }

    fn test_action(&mut self, action: &str, done: std::path::PathBuf, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        let (verb, rest) = action.split_once(' ').unwrap_or((action, ""));
        let said = match (verb, rest.trim()) {
            ("open", path) => {
                let folder = std::path::Path::new(path).parent().map(|p| p.to_string_lossy().into_owned());
                self.draft.host = HostId::ThisMac;
                self.draft.folder = folder;
                self.draft.notebook = crate::new_session::NotebookChoice::Existing(path.to_owned());
                self.input.update(cx, |s, cx| s.set_value("Look at this notebook.", window, cx));
                self.start_session(window, cx);
                "started".to_owned()
            }
            ("run", _) => match self.active_session().and_then(|s| s.notebook.clone()) {
                Some(notebook) => {
                    self.run_notebook(notebook, cx);
                    "running".to_owned()
                }
                None => "no notebook on screen".to_owned(),
            },
            ("settings", _) => {
                self.open_settings_last(window, cx);
                "open".to_owned()
            }
            ("close", "settings") => {
                self.close_settings(window, cx);
                "closed".to_owned()
            }
            ("notice", error) => {
                // A failure notice over the notebook's buttons, as a failed restart shows one.
                let error = if error.is_empty() { (1..=60).map(|i| format!("    [{i}] a stack frame that is long enough to wrap @ Base ./process.jl:{i}\n")).collect::<String>() } else { error.replace("\\n", "\n") };
                let key = self.active_session().map_or(0, |s| s.key);
                let notice = crate::notice::Notice::new(crate::notice::Spot::NotebookRight, "Couldn't restart the notebook", &format!("TaskFailedException\n\n{error}"), Some(crate::notice::Retry::RestartNotebook(key)));
                self.show_notice(notice, cx);
                "shown".to_owned()
            }
            ("details", _) => match &mut self.notice {
                Some(n) => {
                    n.details_open = !n.details_open;
                    cx.notify();
                    format!("{}", n.details_open)
                }
                None => "no notice".to_owned(),
            },
            ("js", script) => {
                let answer = done.clone();
                let asked = self.webview.read(cx).raw().evaluate_script_with_callback(script, move |value| {
                    let _ = std::fs::write(&answer, if value.is_empty() { "null".to_owned() } else { value });
                });
                match asked {
                    Ok(()) => return,
                    Err(e) => format!("couldn't run it: {e}"),
                }
            }
            _ => format!("unknown action: {action}"),
        };
        let _ = std::fs::write(&done, said);
    }
}
