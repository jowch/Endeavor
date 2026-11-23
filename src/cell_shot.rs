use std::time::Duration;

use crate::hosts::HostId;

const NOT_ON_SCREEN: &str = "the notebook pane isn't on screen";

pub struct Ask {
    pub host: HostId,
    pub notebook: String,
    pub cell: String,
    pub asked: std::time::Instant,
    pub reply: std::sync::mpsc::Sender<Option<endeavor_mcp::Picture>>,
}

const WAIT: Duration = Duration::from_secs(10);

pub const DEADLINE: Duration = Duration::from_secs(8);

pub const IDLE: Duration = Duration::from_secs(300);

pub fn snapshot(host: HostId, asks: futures::channel::mpsc::UnboundedSender<Ask>) -> Box<endeavor_mcp::Pictures> {
    let asks = std::sync::Mutex::new(asks);
    Box::new(move |_session, notebook, cell| {
        let (reply, answer) = std::sync::mpsc::channel();
        let ask = Ask { host: host.clone(), notebook: notebook.to_owned(), cell: cell.to_owned(), asked: std::time::Instant::now(), reply };
        asks.lock().unwrap().unbounded_send(ask).ok()?;
        answer.recv_timeout(WAIT).ok().flatten()
    })
}

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

pub type Cut = Option<(u32, u32)>;

type ShotJob = futures::future::LocalBoxFuture<'static, Result<(Vec<u8>, Cut), &'static str>>;

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

    pub struct Shooter {
        pub view: wry::WebView,
        pub url: std::rc::Rc<std::cell::RefCell<String>>,
        main: Retained<AnyObject>,
    }

    impl Shooter {
        pub fn new(window: &gpui::Window, main: &wry::WebView) -> Option<Shooter> {
            let view = wry::WebViewBuilder::new().with_url("about:blank").build_as_child(&crate::platform::webview_parent(window)).ok()?;
            let _ = view.set_visible(false);
            let url = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            let page = url.clone();
            crate::webcontent::on_process_ended(&view, move || {
                eprintln!("The second web view's web content process ended; the next cell picture loads its page again");
                page.borrow_mut().clear();
            });
            let main = unsafe { Retained::cast_unchecked(main.webview()) };
            crate::overlay::set_second(Some(&view));
            Some(Shooter { view, url, main })
        }

        pub fn load(&self, url: &str) -> bool {
            if *self.url.borrow() == url {
                return false;
            }
            let _ = self.view.load_url(url);
            *self.url.borrow_mut() = url.to_owned();
            true
        }

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
                let state: usize = msg_send![window, occlusionState];
                state & (1 << 1) != 0
            }
        }

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
                    crate::overlay::set_second(Some(&self.view));
                }
                let superview: *mut AnyObject = msg_send![&*self.main, superview];
                if !superview.is_null() && !just_below(superview, &*ours, &*self.main) {
                    let _: () = msg_send![superview, addSubview: &*ours, positioned: -1isize, relativeTo: &*self.main];
                }
            }
            if !self.shown() {
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

    unsafe fn just_below<A: objc2::Message, B: objc2::Message>(superview: *mut AnyObject, ours: &A, main: &B) -> bool {
        unsafe {
            let subviews: *mut AnyObject = msg_send![superview, subviews];
            let ours: usize = msg_send![subviews, indexOfObjectIdenticalTo: ours];
            let main: usize = msg_send![subviews, indexOfObjectIdenticalTo: main];
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

    pub struct Shooter {
        pub view: wry::WebView,
        pub url: std::rc::Rc<std::cell::RefCell<String>>,
        main: gdk::Window,
    }

    impl Shooter {
        pub fn new(window: &gpui::Window, main: &wry::WebView) -> Option<Shooter> {
            let main = overlay::window_of(main)?;
            let parent = crate::platform::webview_parent(window);
            let view = wry::WebViewBuilder::new().with_url("about:blank").build_as_child(&parent).ok()?;
            overlay::window_of(&view)?.hide();
            let url = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
            let page = url.clone();
            crate::webcontent::on_process_ended(&view, move || {
                eprintln!("The second web view's web content process ended; the next cell picture loads its page again");
                page.borrow_mut().clear();
            });
            overlay::set_second(Some(&view));
            Some(Shooter { view, url, main })
        }

        pub fn load(&self, url: &str) -> bool {
            if *self.url.borrow() == url {
                return false;
            }
            let _ = self.view.load_url(url);
            *self.url.borrow_mut() = url.to_owned();
            true
        }

        pub fn pane_shows(&self) -> bool {
            overlay::viewable(&self.main)
        }

        pub fn keep_behind(&self) -> bool {
            if !self.pane_shows() {
                self.hide();
                return false;
            }
            let Some(ours) = overlay::window_of(&self.view) else { return false };
            if !overlay::has_second() {
                overlay::set_second(Some(&self.view));
            }
            use wry::WebViewExtUnix;
            let (x, y, width, height) = self.main.geometry();
            let laid_out = (self.view.webview().allocated_width(), self.view.webview().allocated_height());
            if ours.geometry() != (x, y, width, height) || laid_out != (width, height) {
                let _ = self.view.set_bounds(wry::Rect { position: wry::dpi::LogicalPosition::new(x, y).into(), size: wry::dpi::LogicalSize::new(width, height).into() });
            }
            ours.lower();
            if !self.shown() {
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

    pub struct Shooter {
        pub view: wry::WebView,
        pub url: std::rc::Rc<std::cell::RefCell<String>>,
        main: usize,
    }

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
            overlay::set_second(Some(&view));
            Some(Shooter { view, url, main })
        }

        pub fn load(&self, url: &str) -> bool {
            if *self.url.borrow() == url {
                return false;
            }
            let _ = self.view.load_url(url);
            *self.url.borrow_mut() = url.to_owned();
            true
        }

        pub fn pane_shows(&self) -> bool {
            let main = self.main as HWND;
            unsafe { IsWindowVisible(main) != 0 && IsIconic(GetAncestor(main, GA_ROOT)) == 0 }
        }

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
            unsafe { SetWindowPos(ours as HWND, self.main as HWND, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
            if !self.shown() {
                crate::settings::set_webview_appearance(&self.view, crate::theme::is_light());
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
            drop(taken);
        });
        assert_eq!(snapshot("7", "nb", "cell"), Some(endeavor_mcp::Picture { png: "cGljdHVyZQ==".into(), cut: Some((855, 920)) }));
        answering.join().unwrap();
        assert_eq!(snapshot("7", "nb", "cell"), None);
    }
}

#[derive(Default)]
pub struct Second {
    shooter: Option<std::rc::Rc<Shooter>>,
    used: Option<std::time::Instant>,
    pub last: Option<&'static str>,
}

impl Second {
    pub fn view(&self) -> &'static str {
        if let Some(shooter) = &self.shooter {
            return if shooter.shown() { "shown" } else { "hidden" };
        }
        "none"
    }

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
        let url = Backend::Pluto.notebook_url(&runtime.page_url, &ask.notebook);
        if ask.asked.elapsed() >= WAIT {
            return Err("the agent stopped waiting");
        }
        if self.second.shooter.is_none() {
            self.second.shooter = Shooter::new(window, self.webview.read(cx).raw()).map(std::rc::Rc::new);
        }
        let shooter = self.second.shooter.clone().ok_or("no second web view")?;
        self.second.used = Some(std::time::Instant::now());
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

async fn take(shooter: &Shooter, cell: &str, fresh: bool, executor: &gpui::BackgroundExecutor) -> Result<(Vec<u8>, Cut), &'static str> {
    let deadline = std::time::Instant::now() + DEADLINE;
    let behind = || if shooter.keep_behind() { Ok(()) } else { Err(NOT_ON_SCREEN) };
    loop {
        let found = eval(&shooter.view, &find_script(cell), executor).await;
        behind()?;
        match found.as_deref() {
            Some("\"scrolled\"") => break,
            Some("\"empty\"") => return Err("the output is empty"),
            _ => {}
        }
        if std::time::Instant::now() > deadline {
            return Err("the cell's output wasn't drawn in time");
        }
        executor.timer(Duration::from_millis(200)).await;
        behind()?;
    }
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
