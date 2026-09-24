//! Spike (design doc §10.1 + §11): app-owned Julia running Pluto + PlutoMCP,
//! with the live Pluto frontend embedded as a child webview beside a native panel.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};

use gpui::*;
use gpui_wry::WebView;
use raw_window_handle::HasWindowHandle;

struct Runtime {
    // Holding the child holds its stdin; boot.jl exits when stdin closes.
    _julia: Child,
    pluto_url: String,
    mcp_url: String,
}

/// Start the app-owned Julia and block until boot.jl reports `READY`.
fn start_runtime() -> Result<Runtime, String> {
    // ponytail: dev-tree paths; resolve from the .app bundle's resources when packaging.
    let root = env!("CARGO_MANIFEST_DIR");
    // ponytail: ENDEAVOR_JULIA is the "use my Julia" opt-in; the managed download (§11) comes next.
    let julia = std::env::var("ENDEAVOR_JULIA").unwrap_or_else(|_| "julia".into());
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    // Trailing ':' stacks the default depots (~/.julia) read-only behind ours.
    let depot = format!("{home}/Library/Application Support/endeavor/depot:");

    // Hold both listeners at once so the OS can't hand out the same port twice.
    let pluto = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let mcp = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let ports = [&pluto, &mcp].map(|l| l.local_addr().unwrap().port().to_string());
    drop((pluto, mcp));

    let mut child = Command::new(&julia)
        .arg(format!("--project={root}/runtime"))
        .arg(format!("{root}/runtime/boot.jl"))
        .args(ports)
        .env("JULIA_DEPOT_PATH", depot)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("couldn't start {julia}: {e}"))?;

    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    for line in lines.by_ref() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some((pluto_url, mcp_url)) = line.strip_prefix("READY ").and_then(|r| r.split_once(' ')) {
            let (pluto_url, mcp_url) = (pluto_url.to_owned(), mcp_url.to_owned());
            // Keep draining stdout so a chatty Julia never blocks on a full pipe.
            std::thread::spawn(move || lines.map_while(Result::ok).for_each(|l| println!("{l}")));
            return Ok(Runtime { _julia: child, pluto_url, mcp_url });
        }
        println!("{line}");
    }
    Err("Julia exited before reporting READY (see stderr)".into())
}

struct Workspace {
    webview: Entity<WebView>,
    status: SharedString,
    _runtime: Option<Runtime>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let webview = cx.new(|cx| {
            let handle = window.window_handle().expect("window handle");
            let webview = wry::WebViewBuilder::new()
                .with_devtools(true)
                .build_as_child(&handle)
                .expect("child webview");
            WebView::new(webview, window, cx)
        });

        // First run instantiates + precompiles (~1 min); later launches are seconds.
        let boot = cx.background_executor().spawn(async { start_runtime() });
        cx.spawn(async move |this, cx| {
            let result = boot.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(runtime) => {
                        this.webview.update(cx, |w, _| w.load_url(&runtime.pluto_url));
                        this.status = format!("Pluto ready\nMCP: {}", runtime.mcp_url).into();
                        this._runtime = Some(runtime);
                    }
                    Err(e) => this.status = format!("Runtime failed: {e}").into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            webview,
            status: "Starting Julia…".into(),
            _runtime: None,
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xdddddd))
            .child(
                div()
                    .w(px(360.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .border_r_1()
                    .border_color(rgb(0x333333))
                    .child("Agent panel")
                    .child(div().text_sm().text_color(rgb(0x999999)).child(self.status.clone())),
            )
            .child(div().flex_1().h_full().child(self.webview.clone()))
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Workspace::new(window, cx)),
        )
        .unwrap();
        // Quitting closes Julia's stdin, which shuts the runtime down.
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}
