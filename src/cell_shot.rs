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
// ponytail: Mac only. Windows (WebView2's captureScreenshot) and Linux (WebKitGTK's snapshot) come later.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

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
    /// The PNG, in base64; None when there's no picture.
    pub reply: std::sync::mpsc::Sender<Option<String>>,
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
pub fn snapshot(host: HostId, asks: futures::channel::mpsc::UnboundedSender<Ask>) -> Box<endeavor_mcp::Snapshot> {
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

/// The page script that measures the output: its rect and the viewport's size, in CSS pixels;
/// "covered" if something else on the page (a bar, a popup) is drawn over the middle of what's in view.
pub fn measure_script(cell: &str) -> String {
    format!(
        r#"(() => {{
  const out = document.querySelector('pluto-cell[id="{cell}"] > pluto-output');
  if (!out) return null;
  const r = out.getBoundingClientRect();
  const top = Math.max(r.top, 0), bottom = Math.min(r.bottom, innerHeight);
  const over = document.elementFromPoint(r.left + r.width / 2, (top + bottom) / 2);
  if (bottom > top && over && !out.contains(over)) return "covered";
  return [r.left, r.top, r.width, r.height, innerWidth, innerHeight];
}})()"#
    )
}

/// The part of the output inside the viewport, as x, y, width, height, from
/// `measure_script`'s answer; None if there's none worth a picture.
pub fn visible_rect(measured: &str) -> Option<[f64; 4]> {
    let v: Vec<f64> = serde_json::from_str(measured).ok()?;
    let [left, top, width, height, view_width, view_height] = v[..] else { return None };
    let (x, y) = (left.max(0.), top.max(0.));
    let (w, h) = ((left + width).min(view_width) - x, (top + height).min(view_height) - y);
    (w >= 2. && h >= 2.).then_some([x, y, w, h])
}

#[cfg(target_os = "macos")]
pub use mac::Shooter;

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
        pub url: std::cell::RefCell<String>,
        main: Retained<AnyObject>,
    }

    impl Shooter {
        pub fn new(window: &gpui::Window, main: &wry::WebView) -> Option<Shooter> {
            let view = wry::WebViewBuilder::new().with_url("about:blank").build_as_child(&crate::platform::webview_parent(window)).ok()?;
            let _ = view.set_visible(false);
            let main = unsafe { Retained::cast_unchecked(main.webview()) };
            Some(Shooter { view, url: std::cell::RefCell::new(String::new()), main })
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

        /// Whether the user's view is shown whole, in a window on screen and not covered: behind
        /// anything else the second view would show, and a hidden window draws nothing. Settings,
        /// a dialog, a menu or a notice over the pane doesn't hide the user's view: it dims it
        /// (`overlay::set_dimmed`) or cuts holes in it (`overlay::set_hole`, a layer mask), and the
        /// second view would show through both.
        pub fn pane_shows(&self) -> bool {
            unsafe {
                let hidden: bool = msg_send![&*self.main, isHiddenOrHasHiddenAncestor];
                let window: *mut AnyObject = msg_send![&*self.main, window];
                if hidden || window.is_null() {
                    return false;
                }
                let alpha: f64 = msg_send![&*self.main, alphaValue];
                let layer: *mut AnyObject = msg_send![&*self.main, layer];
                let holes: *mut AnyObject = if layer.is_null() { std::ptr::null_mut() } else { msg_send![layer, mask] };
                if alpha < 1. || !holes.is_null() {
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
                if now != frame {
                    let _: () = msg_send![&*ours, setFrame: frame];
                }
                let superview: *mut AnyObject = msg_send![&*self.main, superview];
                let behind: *mut AnyObject = msg_send![&*ours, superview];
                if !superview.is_null() && behind != superview {
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
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_the_part_of_the_output_in_view() {
        assert_eq!(visible_rect("[20, 0, 600, 300, 900, 1200]"), Some([20., 0., 600., 300.]));
        assert_eq!(visible_rect("[20, 900, 600, 900, 900, 1200]"), Some([20., 900., 600., 300.]), "cut at the bottom");
        assert_eq!(visible_rect("[-10, -5, 600, 300, 400, 1200]"), Some([0., 0., 400., 295.]), "and at the edges");
        assert_eq!(visible_rect("[20, 0, 600, 0, 900, 1200]"), None, "an empty output");
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
            ask.reply.send(Some("cGljdHVyZQ==".into())).unwrap();
            // The app went away: no answer.
            drop(taken);
        });
        assert_eq!(snapshot("7", "nb", "cell").as_deref(), Some("cGljdHVyZQ=="));
        answering.join().unwrap();
        assert_eq!(snapshot("7", "nb", "cell"), None);
    }
}

/// The second web view while it is kept, and how the last picture went (the state dump's `cell_shot`).
#[derive(Default)]
pub struct Second {
    #[cfg(target_os = "macos")]
    shooter: Option<std::rc::Rc<Shooter>>,
    #[cfg(target_os = "macos")]
    used: Option<std::time::Instant>,
    pub last: Option<&'static str>,
}

impl Second {
    /// The second view, for the state dump: "none" (not made, or let go), "hidden", or "shown"
    /// (behind the user's view, while a picture is taken).
    pub fn view(&self) -> &'static str {
        #[cfg(target_os = "macos")]
        if let Some(shooter) = &self.shooter {
            return if shooter.shown() { "shown" } else { "hidden" };
        }
        "none"
    }
}

impl crate::Workspace {
    /// Take the picture `ask` waits for, one at a time. What happened.
    pub(crate) fn shoot_cell(&mut self, ask: Ask, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> futures::future::LocalBoxFuture<'static, &'static str> {
        #[cfg(target_os = "macos")]
        return match self.cell_shot_job(&ask, window, cx) {
            Ok(job) => Box::pin(async move {
                let (png, outcome) = match job.await {
                    Ok(png) => (Some(crate::attach::base64(&png)), "picture"),
                    Err(why) => (None, why),
                };
                let _ = ask.reply.send(png);
                outcome
            }),
            Err(why) => {
                let _ = ask.reply.send(None);
                Box::pin(async move { why })
            }
        };
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (window, cx, ask.reply.send(None));
            Box::pin(async { "no snapshots here" })
        }
    }

    /// After a picture: the outcome for the state dump, and the second view goes once it's idle.
    pub(crate) fn cell_shot_done(&mut self, outcome: &'static str, cx: &mut gpui::Context<Self>) {
        self.second.last = Some(outcome);
        #[cfg(target_os = "macos")]
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(IDLE).await;
            let _ = this.update(cx, |this, _| {
                if this.second.used.is_some_and(|used| used.elapsed() >= IDLE) {
                    this.second = Second { last: this.second.last, ..Second::default() };
                }
            });
        })
        .detach();
        #[cfg(not(target_os = "macos"))]
        let _ = cx;
    }

    #[cfg(target_os = "macos")]
    fn cell_shot_job(&mut self, ask: &Ask, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Result<futures::future::LocalBoxFuture<'static, Result<Vec<u8>, &'static str>>, &'static str> {
        use wire::backend::Backend;
        if self.notebook_kind(&ask.host, &ask.notebook) != Backend::Pluto {
            return Err("not a Pluto notebook");
        }
        let runtime = self.connection(&ask.host).and_then(|c| c.runtime.as_ref()).ok_or("no runtime")?;
        let url = Backend::Pluto.notebook_url(&runtime.page_url, &ask.notebook);
        if ask.asked.elapsed() >= WAIT {
            return Err("the agent stopped waiting");
        }
        if self.second.shooter.is_none() {
            self.second.shooter = Shooter::new(window, self.webview.read(cx).raw()).map(std::rc::Rc::new);
        }
        let shooter = self.second.shooter.clone().ok_or("no second web view")?;
        // ponytail: only while the notebook pane is on screen (`keep_behind`).
        if !shooter.keep_behind() {
            return Err(NOT_ON_SCREEN);
        }
        let fresh = shooter.load(&url);
        self.second.used = Some(std::time::Instant::now());
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
/// pane is checked at each step: once it isn't on screen (Settings opened), the second view hides,
/// within a step (at most about a second), and there is no picture.
#[cfg(target_os = "macos")]
async fn take(shooter: &Shooter, cell: &str, fresh: bool, executor: &gpui::BackgroundExecutor) -> Result<Vec<u8>, &'static str> {
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
    let rect = visible_rect(&measured).ok_or("the output is empty")?;
    let shot = crate::snapshot::png(&shooter.view, rect);
    let timeout = executor.timer(Duration::from_secs(3));
    let png = match futures::future::select(shot, timeout).await {
        futures::future::Either::Left((png, _)) => png.ok().flatten(),
        futures::future::Either::Right(_) => None,
    };
    png.ok_or("WebKit couldn't draw the output")
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
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nX-Endeavor-Session: {key}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{call}",
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
