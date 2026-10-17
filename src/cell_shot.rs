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
//! the user did inside a widget (a selection) isn't in the picture.
// ponytail: Mac only. Windows (WebView2's captureScreenshot) and Linux (WebKitGTK's snapshot) come later.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::time::Duration;

use crate::hosts::HostId;

/// One picture the listener waits for.
pub struct Ask {
    pub host: HostId,
    pub notebook: String,
    pub cell: String,
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
        let ask = Ask { host: host.clone(), notebook: notebook.to_owned(), cell: cell.to_owned(), reply };
        asks.lock().unwrap().unbounded_send(ask).ok()?;
        answer.recv_timeout(WAIT).ok().flatten()
    })
}

/// The page script that finds the cell's output and scrolls it to the top:
/// "scrolled", "busy" (it is running or queued), "empty" (nothing to see: a
/// cell that hasn't run), or "absent" (the page hasn't drawn the cell yet).
/// `cell` is a UUID (the library checked it).
pub fn find_script(cell: &str) -> String {
    format!(
        r#"(() => {{
  const out = document.querySelector('pluto-cell[id="{cell}"] > pluto-output');
  if (!out) return "absent";
  if (!out.firstElementChild || out.getBoundingClientRect().height < 2) return "empty";
  const c = out.parentElement.classList;
  if (c.contains("running") || c.contains("queued")) return "busy";
  out.scrollIntoView({{ block: "start" }});
  return "scrolled";
}})()"#
    )
}

/// The page script that measures the output: its rect and the viewport's size, in CSS pixels.
pub fn measure_script(cell: &str) -> String {
    format!(
        r#"(() => {{
  const out = document.querySelector('pluto-cell[id="{cell}"] > pluto-output');
  if (!out) return null;
  const r = out.getBoundingClientRect();
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
    use objc2::runtime::AnyObject;
    use wry::WebViewExtMacOS;

    /// The second web view, and the page it has loaded.
    pub struct Shooter {
        pub view: wry::WebView,
        pub url: std::cell::RefCell<String>,
    }

    impl Shooter {
        pub fn new(window: &gpui::Window) -> Option<Shooter> {
            let view = wry::WebViewBuilder::new().with_url("about:blank").build_as_child(&crate::platform::webview_parent(window)).ok()?;
            let _ = view.set_visible(false);
            crate::settings::set_webview_appearance(&view, crate::theme::is_light());
            Some(Shooter { view, url: std::cell::RefCell::new(String::new()) })
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

        /// Put the second view exactly behind `main`, the user's view, where it draws but can't be seen.
        pub fn place(&self, main: &wry::WebView) {
            if let Ok(bounds) = main.bounds() {
                let _ = self.view.set_bounds(bounds);
            }
            unsafe {
                let (ours, theirs) = (self.view.webview(), main.webview());
                let superview: *mut AnyObject = msg_send![&*theirs, superview];
                if !superview.is_null() {
                    // NSWindowBelow: in the same superview, this only reorders.
                    let _: () = msg_send![superview, addSubview: &*ours, positioned: -1isize, relativeTo: &*theirs];
                }
            }
            let _ = self.view.set_visible(true);
        }

        pub fn hide(&self) {
            let _ = self.view.set_visible(false);
        }
    }

    /// Whether the window of `main` is on screen and not covered: a window
    /// that isn't draws nothing, and its snapshots come back blank.
    pub fn window_shows(main: &wry::WebView) -> bool {
        unsafe {
            let view = main.webview();
            let window: *mut AnyObject = msg_send![&*view, window];
            if window.is_null() {
                return false;
            }
            // NSWindowOcclusionStateVisible.
            let state: usize = msg_send![window, occlusionState];
            state & (1 << 1) != 0
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::window_shows;

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
        // ponytail: only while the notebook pane is on screen. Behind any other screen the second
        // view would show, and in a hidden window it draws nothing.
        let main = self.webview.read(cx);
        if !main.visible() || !window_shows(main.raw()) {
            return Err("the notebook pane isn't on screen");
        }
        if self.second.shooter.is_none() {
            self.second.shooter = Shooter::new(window).map(std::rc::Rc::new);
        }
        let shooter = self.second.shooter.clone().ok_or("no second web view")?;
        let fresh = shooter.load(&url);
        shooter.place(main.raw());
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

/// Wait for the cell's output in the second view, scroll it into view, and take its picture.
#[cfg(target_os = "macos")]
async fn take(shooter: &Shooter, cell: &str, fresh: bool, executor: &gpui::BackgroundExecutor) -> Result<Vec<u8>, &'static str> {
    let deadline = std::time::Instant::now() + DEADLINE;
    loop {
        match eval(&shooter.view, &find_script(cell), executor).await.as_deref() {
            Some("\"scrolled\"") => break,
            // An empty output gives the agent a blank picture: the runtime's answer says more.
            Some("\"empty\"") => return Err("the output is empty"),
            _ => {}
        }
        if std::time::Instant::now() > deadline {
            return Err("the cell's output wasn't drawn in time");
        }
        executor.timer(Duration::from_millis(200)).await;
    }
    // Two frames after the scroll; and on a page just loaded, time for a widget that draws after it (WebGL).
    executor.timer(Duration::from_millis(if fresh { 800 } else { 100 })).await;
    let measured = eval(&shooter.view, &measure_script(cell), executor).await.ok_or("the output couldn't be measured")?;
    let rect = visible_rect(&measured).ok_or("the output is empty")?;
    crate::snapshot::png(&shooter.view, rect).await.ok().flatten().ok_or("WebKit couldn't draw the output")
}

/// The JSON of what `js` gives in `view`; None if it fails or takes over a second.
#[cfg(target_os = "macos")]
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
