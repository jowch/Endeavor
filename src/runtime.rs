//! The Julia runtime (Pluto + EndeavorRuntime, runtime/boot.jl), reached through
//! the endeavor-remote helper (docs/remote-sessions.md): the app runs it as a
//! child, and the helper attaches to the runtime in the state folder or starts
//! one. The webview and the agent reach the runtime through the app's local
//! listener, whose two loopback ports stay the same for the whole launch.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use wire::relay::Mux;
use wire::{Frame, Target, ToApp, ToHelper};

use crate::splash::{Progress, Step};

/// Julia needed by runtime/Project.toml's `[sources]` section.
const MIN_JULIA: (u32, u32) = (1, 11);

/// The app's loopback ports for Pluto and the bridge. Each connection is
/// relayed to the runtime of the moment; with none, it's closed.
pub struct Listener {
    ports: [u16; 2],
    current: Mutex<Option<Arc<Mux>>>,
}

impl Listener {
    pub fn start() -> Result<Arc<Listener>, String> {
        let pluto = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let bridge = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let ports = [&pluto, &bridge].map(|l| l.local_addr().unwrap().port());
        let listener = Arc::new(Listener { ports, current: Mutex::new(None) });
        for (socket, target) in [(pluto, Target::Pluto), (bridge, Target::Bridge)] {
            let listener = listener.clone();
            std::thread::spawn(move || {
                for connection in socket.incoming().map_while(Result::ok) {
                    let _ = connection.set_nodelay(true);
                    let mux = listener.current.lock().unwrap().clone();
                    if let Some(mux) = mux {
                        let _ = mux.open(target, connection);
                    }
                }
            });
        }
        Ok(listener)
    }

    /// The bridge URL the agent's MCP config carries; the same for the whole launch.
    pub fn mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/sse", self.ports[1])
    }

    pub fn bridge_port(&self) -> u16 {
        self.ports[1]
    }

    fn pluto_url(&self, secret: &str) -> String {
        format!("http://127.0.0.1:{}/?secret={secret}", self.ports[0])
    }

    fn forget(&self, mux: &Arc<Mux>) {
        let mut current = self.current.lock().unwrap();
        if current.as_ref().is_some_and(|m| Arc::ptr_eq(m, mux)) {
            *current = None;
        }
    }
}

/// A connected runtime.
pub struct Runtime {
    pub pluto_url: String,
    pub mcp_url: String,
    /// It was already running (kept after the last quit); this launch didn't start it.
    pub reattached: bool,
    mux: Arc<Mux>,
    /// Signalled once the helper process has exited.
    helper_exited: mpsc::Receiver<()>,
    /// The app is stopping or leaving this runtime, so its end is no news.
    leaving: Arc<AtomicBool>,
}

/// Why a connected runtime went away.
pub enum Notice {
    Died(String),
    /// Another client took the runtime over.
    Replaced,
    /// The helper failed or vanished.
    Lost(String),
}

impl Runtime {
    /// Stop Julia and wait until the helper is done with it (blocks up to ~30 s).
    pub fn stop(self) {
        self.leave(ToHelper::Stop);
    }

    /// Leave Julia running and wait until the helper has gone.
    pub fn detach(self) {
        self.leave(ToHelper::Detach);
    }

    fn leave(self, message: ToHelper) {
        self.leaving.store(true, Ordering::SeqCst);
        let _ = self.mux.send(&message.frame());
        let _ = self.helper_exited.recv_timeout(Duration::from_secs(30));
    }

    /// The app is quitting: leave Julia running for the next launch, or stop it.
    /// The helper does the rest after the app has gone.
    pub fn quit(&self, keep_running: bool) {
        self.leaving.store(true, Ordering::SeqCst);
        let message = if keep_running { ToHelper::Detach } else { ToHelper::Stop };
        let _ = self.mux.send(&message.frame());
    }
}

/// The Julia the app installs on first run (design doc §11), pinned with the
/// official tarballs' SHA-256 and size (bump all three per release).
pub const JULIA_VERSION: &str = "1.12.6";
#[cfg(target_arch = "aarch64")]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/mac/aarch64/1.12/julia-1.12.6-macaarch64.tar.gz",
    "277d82fbd2eda99d0963b3e41f3dc979d7486f181399f8430fb637318ccd6a31",
    231_027_185,
);
#[cfg(target_arch = "x86_64")]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/mac/x64/1.12/julia-1.12.6-mac64.tar.gz",
    "1a70b7c606d6bac38a246e722369e5b30914dccf9378499d2712fb3bd282642c",
    271_518_180,
);


/// The julia binary to run: the user's (Settings), else the app's own,
/// downloaded and verified on first run. `progress` hears how that's going.
fn julia_binary(progress: &dyn Fn(String, Option<f32>)) -> Result<String, String> {
    if let Some(julia) = crate::settings::Settings::load().julia {
        return Ok(julia.display().to_string());
    }
    let dir = crate::install::app_dir()?.join(format!("julia-{JULIA_VERSION}"));
    let bin = dir.join("bin/julia");
    if !bin.exists() {
        crate::install::tarball(&dir, &format!("Julia {JULIA_VERSION}"), &format!("julia-{JULIA_VERSION}"), JULIA_TARBALL, progress)?;
    }
    Ok(bin.display().to_string())
}

/// The helper, next to the app's own executable (`cargo test` runs from target/*/deps).
pub fn helper_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no executable folder")?;
    [Some(dir), dir.parent()]
        .into_iter()
        .flatten()
        .map(|d| d.join("endeavor-remote"))
        .find(|p| p.exists())
        .ok_or_else(|| format!("Endeavor's runtime helper (endeavor-remote) is missing from {}. Reinstall Endeavor.", dir.display()))
}

/// Connect to the runtime on This Mac, starting it if it isn't running, and
/// relay `listener` to it. Blocks until it's ready. `keep_running` leaves it
/// running if the app goes away without quitting; `notices` hears if it goes
/// away later; `progress` hears about setup: Julia's install, then its log.
pub fn connect(
    listener: Arc<Listener>,
    keep_running: bool,
    notices: UnboundedSender<Notice>,
    progress: UnboundedSender<Progress>,
) -> Result<Runtime, String> {
    let julia = julia_binary(&|detail, fraction| {
        let _ = progress.unbounded_send(Progress { fraction, ..Progress::new(Step::Julia, detail) });
    })?;
    let _ = progress.unbounded_send(Progress::new(Step::Packages, "Starting Julia…"));
    check_version(&julia)?;
    let app_dir = crate::install::app_dir()?;
    let state_dir = app_dir.join("runtime");
    // Trailing ':' stacks the default depots (~/.julia) read-only behind ours.
    let depot = format!("{}/depot:", app_dir.display());

    let mut command = Command::new(helper_binary()?);
    command
        .args(["connect", "--state-dir"])
        .arg(&state_dir)
        .args(["--julia", &julia, "--runtime"])
        .arg(crate::install::resources().join("runtime"))
        .args(["--depot", &depot])
        // This Mac's state folder is its own, so another node name means a renamed Mac.
        .arg("--any-node");
    if !keep_running {
        command.arg("--quit-with-client");
    }
    // Its own process group: a Ctrl-C meant for the app in a terminal must not
    // kill the helper before it can stop Julia.
    let mut helper = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("Couldn't start Endeavor's runtime helper: {e}"))?;

    let stdin = helper.stdin.take().unwrap();
    let stdout = helper.stdout.take().unwrap();
    let channel = Channel::open(helper, stdin, stdout);
    let hello = channel.wait_hello(
        |message| {
            if let ToApp::Progress { line } = message {
                eprintln!("{line}");
                // Package installs and precompiles show on the setup screen.
                let text = line.trim_start_matches(['┌', '│', '└', ' ']).trim();
                if !text.is_empty() {
                    let _ = progress.unbounded_send(Progress { log: true, ..Progress::new(Step::Packages, text) });
                }
            }
        },
        || "Endeavor's runtime helper stopped unexpectedly. Show logs has the details.".into(),
    )?;
    let how = if hello.reattached { "Reattached to" } else { "Started" };
    eprintln!("{how} Julia on {} (pid {}); its log is {}", hello.node, hello.pid, state_dir.join("runtime.log").display());
    crate::pluto::set_bridge_token(&hello.token);
    Ok(channel.attach(&listener, &hello, notices))
}

/// What the helper said once the runtime was up.
pub struct Hello {
    pub node: String,
    pub pid: u32,
    pub token: String,
    pub pluto_secret: String,
    pub reattached: bool,
}

/// The app's end of a helper's stdin/stdout, on This Mac or over ssh.
pub struct Channel {
    mux: Arc<Mux>,
    control: mpsc::Receiver<ToApp>,
    helper_exited: mpsc::Receiver<()>,
    /// The listener relaying to this channel, which forgets it when it ends.
    listener: Arc<Mutex<Option<Arc<Listener>>>>,
}

impl Channel {
    /// `output` is the helper's stdout, read up to its first frame.
    pub fn open(mut helper: Child, input: impl Write + Send + 'static, output: impl Read + Send + 'static) -> Channel {
        let mux = Mux::new(input);
        let (control_tx, control) = mpsc::channel::<ToApp>();
        let (exited_tx, helper_exited) = mpsc::channel();
        let listener: Arc<Mutex<Option<Arc<Listener>>>> = Arc::default();
        std::thread::spawn({
            let (mux, listener) = (mux.clone(), listener.clone());
            move || {
                let result = mux.run(
                    output,
                    // The app opens every stream; the helper never asks to.
                    |mux, id, _| drop(mux.send(&Frame::Close { id })),
                    |json| match serde_json::from_slice(json) {
                        Ok(message) => drop(control_tx.send(message)),
                        Err(e) => eprintln!("endeavor-remote sent an unreadable message: {e}"),
                    },
                );
                if let Some(listener) = listener.lock().unwrap().take() {
                    listener.forget(&mux);
                }
                let status = helper.wait().map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                eprintln!("endeavor-remote exited ({status}){}", result.err().map(|e| format!(": {e}")).unwrap_or_default());
                let _ = exited_tx.send(());
            }
        });
        Channel { mux, control, helper_exited, listener }
    }

    /// Wait until the runtime is up. `on_message` hears `Progress` and
    /// `FoundJulia` meanwhile; `vanished` says why when the helper just ends.
    pub fn wait_hello(&self, mut on_message: impl FnMut(ToApp), vanished: impl FnOnce() -> String) -> Result<Hello, String> {
        loop {
            match self.control.recv() {
                Ok(message @ (ToApp::Progress { .. } | ToApp::FoundJulia { .. })) => on_message(message),
                Ok(ToApp::Hello { token, pluto_secret, reattached, node, pid, .. }) => {
                    return Ok(Hello { node, pid, token, pluto_secret, reattached });
                }
                Ok(ToApp::Died { status, log_tail }) => {
                    return Err(format!("Julia stopped before Pluto was ready ({status}). {}", diagnose(&log_tail)));
                }
                Ok(ToApp::Error { message }) => return Err(message),
                Ok(ToApp::Replaced) => return Err("Another connection took Julia over while it was starting.".into()),
                Err(_) => return Err(vanished()),
            }
        }
    }

    /// Relay `listener`'s connections to this runtime from now on.
    pub fn attach(self, listener: &Arc<Listener>, hello: &Hello, notices: UnboundedSender<Notice>) -> Runtime {
        *self.listener.lock().unwrap() = Some(listener.clone());
        *listener.current.lock().unwrap() = Some(self.mux.clone());
        let leaving = Arc::new(AtomicBool::new(false));
        forward_notices(self.control, notices, leaving.clone());
        Runtime {
            pluto_url: listener.pluto_url(&hello.pluto_secret),
            mcp_url: listener.mcp_url(),
            reattached: hello.reattached,
            mux: self.mux,
            helper_exited: self.helper_exited,
            leaving,
        }
    }
}

/// After `Hello`: the first word of the runtime going away, unless the app is
/// the one leaving it.
fn forward_notices(control: mpsc::Receiver<ToApp>, notices: UnboundedSender<Notice>, leaving: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let notice = loop {
            match control.recv() {
                Ok(ToApp::Died { status, log_tail }) => {
                    // A crash's log is just Pluto's startup banner: say why only if we know.
                    let hint = hint(&log_tail.join("\n")).map(|h| format!(" {h}")).unwrap_or_default();
                    break Notice::Died(format!("Julia exited ({status}).{hint}"));
                }
                Ok(ToApp::Replaced) => break Notice::Replaced,
                Ok(ToApp::Error { message }) => break Notice::Lost(message),
                Ok(_) => continue,
                Err(_) => break Notice::Lost("The connection to Julia closed unexpectedly.".into()),
            }
        };
        if !leaving.load(Ordering::SeqCst) {
            let _ = notices.unbounded_send(notice);
        }
    });
}

fn check_version(julia: &str) -> Result<(), String> {
    let output = Command::new(julia).arg("--version").output().map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            format!(
                "Julia wasn't found at `{julia}`. In Settings, choose a julia binary \
                 or switch back to Endeavor's own Julia."
            )
        } else {
            format!("Couldn't run {julia}: {e}")
        }
    })?;
    let text = String::from_utf8_lossy(&output.stdout);
    match parse_version(&text) {
        Some(v) if v < MIN_JULIA => Err(format!(
            "Endeavor needs Julia {}.{} or newer; `{julia}` is {}.{}. Update it (e.g. `juliaup update`) \
             or choose a newer julia in Settings.",
            MIN_JULIA.0, MIN_JULIA.1, v.0, v.1
        )),
        _ => Ok(()),
    }
}

/// "julia version 1.12.6" -> (1, 12)
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().rsplit(' ').next()?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Plain-language cause for common failures in Julia's log, if recognized.
fn hint(log: &str) -> Option<&'static str> {
    if ["Could not resolve host", "failed to clone", "Couldn't connect", "network"]
        .iter()
        .any(|p| log.contains(p))
    {
        Some("It couldn't download packages; check the internet connection (the first launch installs Pluto and its packages).")
    } else if log.contains("Unsatisfiable requirements") {
        Some("Package versions in Endeavor's runtime environment conflict.")
    } else if log.contains("EADDRINUSE") || log.contains("Address already in use") {
        Some("A port it needs is already in use.")
    } else {
        None
    }
}

/// Plain-language cause for common failures, else the end of Julia's log.
fn diagnose(tail: &[String]) -> String {
    let hint = hint(&tail.join("\n"));
    let recent: Vec<&str> = tail.iter().rev().take(12).rev().map(String::as_str).collect();
    match hint {
        Some(hint) => hint.to_string(),
        None if recent.is_empty() => "It printed nothing.".to_string(),
        None => format!("Last output:\n{}", recent.join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::{diagnose, parse_version};

    #[test]
    fn parses_julia_version() {
        assert_eq!(parse_version("julia version 1.12.6\n"), Some((1, 12)));
        assert_eq!(parse_version("julia version 1.10.0-rc1"), Some((1, 10)));
        assert!(parse_version("julia version 1.10.0").unwrap() < (1, 11));
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn diagnoses_common_failures_or_shows_the_log() {
        let lines = |s: &str| s.lines().map(String::from).collect::<Vec<_>>();
        assert!(diagnose(&lines("ERROR: Could not resolve host: github.com")).contains("internet"));
        assert!(diagnose(&lines("ERROR: Unsatisfiable requirements detected")).contains("conflict"));
        assert!(diagnose(&lines("IOError: listen: address already in use (EADDRINUSE)")).contains("port"));
        let other = diagnose(&lines("ERROR: LoadError: boom\nStacktrace: …"));
        assert!(other.starts_with("Last output:") && other.contains("boom"));
        assert_eq!(diagnose(&[]), "It printed nothing.");
    }
}

/// Live check of the recovery path through the helper (slow; starts Julia twice;
/// stop any Endeavor first, since it shares the state folder):
/// `cargo test -- --ignored live_die_and_restart --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn live_die_and_restart() {
    use futures::StreamExt;
    let listener = Listener::start().unwrap();
    let (notices, mut heard) = futures::channel::mpsc::unbounded();
    let first = connect(listener.clone(), false, notices.clone(), futures::channel::mpsc::unbounded().0).expect("connect");
    let list = crate::pluto::call_tool(&first.mcp_url, "list_notebooks", serde_json::json!({})).unwrap();
    println!("connected (reattached: {}); list_notebooks = {list}", first.reattached);
    // SIGKILL, like a crash or OOM kill.
    let state = std::fs::read_to_string(crate::install::app_dir().unwrap().join("runtime/runtime.json")).unwrap();
    let pid = serde_json::from_str::<serde_json::Value>(&state).unwrap()["pid"].to_string();
    Command::new("kill").args(["-9", &pid]).status().unwrap();
    match futures::executor::block_on(heard.next()).expect("death reported") {
        Notice::Died(reason) => {
            println!("died: {reason}");
            assert!(reason.starts_with("Julia exited"));
        }
        _ => panic!("expected Died"),
    }

    let second = connect(listener, false, notices, futures::channel::mpsc::unbounded().0).expect("connect again");
    assert!(!second.reattached);
    assert_eq!(second.mcp_url, first.mcp_url, "agent's MCP URL must survive a restart");
    assert_ne!(second.pluto_url, first.pluto_url, "new Pluto secret");
    let list = crate::pluto::call_tool(&second.mcp_url, "list_notebooks", serde_json::json!({})).unwrap();
    println!("restarted; list_notebooks = {list}");
    second.stop();
}

#[cfg(test)]
#[test]
fn a_missing_julia_points_to_settings() {
    let err = check_version("/nonexistent/julia").unwrap_err();
    println!("{err}");
    assert!(err.contains("wasn't found") && err.contains("Settings"));
}
