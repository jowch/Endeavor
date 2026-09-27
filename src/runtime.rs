//! The Julia runtime (Pluto + EndeavorRuntime, runtime/boot.jl), reached through
//! the endeavor-remote helper (docs/remote-sessions.md): the app runs it as a
//! child on This Mac, and over ssh on a server (remote.rs). The helper answers
//! file requests from the start, and attaches to the runtime in its state
//! folder or starts one when asked. The webview and the agent reach a host's
//! runtime through that host's local listener, whose two loopback ports stay
//! the same for the whole launch.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use wire::files;
use wire::relay::Mux;
use wire::{Frame, Target, ToApp, ToHelper};

use crate::pluto::Bridge;
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
    fn mcp_url(&self) -> String {
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

/// A runtime the app is attached to, as its host's listener serves it.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub pluto_url: String,
    pub bridge: Bridge,
    /// It was already running (kept after the last quit, or on a server); this
    /// connect didn't start it.
    pub reattached: bool,
    /// The machine it runs on.
    pub node: String,
}

/// Why a runtime went away.
#[derive(Debug)]
pub enum Notice {
    /// Julia exited; the helper is still connected, so it can start again.
    Died(String),
    /// Another client took the runtime over.
    Replaced,
    /// The helper failed or the connection to it closed.
    Lost(String),
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

/// Run This Mac's helper and wait for its hello. `keep_running` leaves the
/// runtime running if the app goes away without quitting; `progress` hears
/// about Julia's install.
pub fn connect(keep_running: bool, progress: &dyn Fn(Progress)) -> Result<(Channel, Hello), String> {
    let julia = julia_binary(&|detail, fraction| progress(Progress { fraction, ..Progress::new(Step::Julia, detail) }))?;
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
    let hello = channel.wait_hello(|| "Endeavor's runtime helper stopped unexpectedly. Show logs has the details.".into())?;
    Ok((channel, hello))
}

/// Start This Mac's runtime on `channel` (or attach to the one running) and
/// relay `listener` to it. `progress` hears Julia's log while it starts;
/// `notice` hears if it goes away later.
pub fn start_local(channel: &Channel, listener: &Arc<Listener>, progress: &dyn Fn(Progress), notice: impl FnOnce(Notice) + Send + 'static) -> Result<Runtime, String> {
    progress(Progress::new(Step::Packages, "Starting Julia…"));
    let runtime = channel.start_runtime(listener, &mut |message| {
        if let ToApp::Progress { line } = message {
            eprintln!("{line}");
            // Package installs and precompiles show on the setup screen.
            let text = line.trim_start_matches(['┌', '│', '└', ' ']).trim();
            if !text.is_empty() {
                progress(Progress { log: true, ..Progress::new(Step::Packages, text) });
            }
        }
    }, notice)?;
    let how = if runtime.reattached { "Reattached to" } else { "Started" };
    let log = crate::install::app_dir().map(|d| d.join("runtime/runtime.log").display().to_string()).unwrap_or_default();
    eprintln!("{how} Julia on {}; its log is {log}", runtime.node);
    Ok(runtime)
}

/// What the helper says as soon as it runs.
#[derive(Clone, Debug)]
pub struct Hello {
    pub node: String,
    /// The home folder on its machine.
    pub home: PathBuf,
}

/// The app's end of a helper's stdin/stdout, on This Mac or over ssh. It lasts
/// as long as the helper: runtimes start, die and stop on it.
pub struct Channel {
    mux: Arc<Mux>,
    /// Where control messages other than file replies go: to whoever waits on
    /// the helper now (its hello, a start, a stop, or the runtime's watcher).
    /// Replacing it ends the previous listener's wait.
    sink: Arc<Mutex<Option<mpsc::Sender<ToApp>>>>,
    hello: Mutex<Option<mpsc::Receiver<ToApp>>>,
    files: Arc<Mutex<HashMap<u32, mpsc::Sender<files::Reply>>>>,
    next_file: AtomicU32,
    helper_exited: Mutex<mpsc::Receiver<()>>,
    /// The listener relaying to this channel, which forgets it when it ends.
    listener: Arc<Mutex<Option<Arc<Listener>>>>,
    /// Set when the app stops the current runtime or leaves the helper, so its
    /// watcher keeps quiet.
    leaving: Mutex<Arc<AtomicBool>>,
}

impl Channel {
    /// `output` is the helper's stdout, read up to its first frame.
    pub fn open(mut helper: Child, input: impl Write + Send + 'static, output: impl Read + Send + 'static) -> Channel {
        let mux = Mux::new(input);
        let (hello_tx, hello) = mpsc::channel::<ToApp>();
        let sink = Arc::new(Mutex::new(Some(hello_tx)));
        let (exited_tx, helper_exited) = mpsc::channel();
        let listener: Arc<Mutex<Option<Arc<Listener>>>> = Arc::default();
        let files: Arc<Mutex<HashMap<u32, mpsc::Sender<files::Reply>>>> = Arc::default();
        std::thread::spawn({
            let (mux, sink, listener, files) = (mux.clone(), sink.clone(), listener.clone(), files.clone());
            move || {
                let result = mux.run(
                    output,
                    // The app opens every stream; the helper never asks to.
                    |mux, id, _| drop(mux.send(&Frame::Close { id })),
                    |json| match serde_json::from_slice(json) {
                        Ok(ToApp::Files { id, reply }) => {
                            if let Some(waiting) = files.lock().unwrap().remove(&id) {
                                let _ = waiting.send(reply);
                            }
                        }
                        Ok(message) => {
                            if let Some(sink) = &*sink.lock().unwrap() {
                                let _ = sink.send(message);
                            }
                        }
                        Err(e) => eprintln!("endeavor-remote sent an unreadable message: {e}"),
                    },
                );
                if let Some(listener) = listener.lock().unwrap().take() {
                    listener.forget(&mux);
                }
                // Whoever waits hears the end.
                sink.lock().unwrap().take();
                files.lock().unwrap().clear();
                let status = helper.wait().map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                eprintln!("endeavor-remote exited ({status}){}", result.err().map(|e| format!(": {e}")).unwrap_or_default());
                let _ = exited_tx.send(());
            }
        });
        Channel {
            mux,
            sink,
            hello: Mutex::new(Some(hello)),
            files,
            next_file: AtomicU32::new(0),
            helper_exited: Mutex::new(helper_exited),
            listener,
            leaving: Mutex::default(),
        }
    }

    /// Control messages from now on, to the returned receiver only.
    fn subscribe(&self) -> mpsc::Receiver<ToApp> {
        let (tx, rx) = mpsc::channel();
        let mut sink = self.sink.lock().unwrap();
        // A channel whose helper is gone keeps no sink, so the receiver ends at once.
        if sink.is_some() {
            *sink = Some(tx);
        }
        rx
    }

    /// Wait for the helper's hello; `vanished` says why when the helper just ends.
    pub fn wait_hello(&self, vanished: impl FnOnce() -> String) -> Result<Hello, String> {
        let Some(hello) = self.hello.lock().unwrap().take() else { return Err("Already said hello.".into()) };
        match hello.recv() {
            Ok(ToApp::Hello { node, home, .. }) => Ok(Hello { node, home: PathBuf::from(home) }),
            Ok(ToApp::Error { message }) => Err(message),
            Ok(other) => Err(format!("Endeavor's helper said {other:?} before hello.")),
            Err(_) => Err(vanished()),
        }
    }

    /// Ask the helper about its machine's files.
    pub fn files(&self, request: files::Request) -> Result<files::Reply, String> {
        let id = self.next_file.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.files.lock().unwrap().insert(id, tx);
        self.mux.send(&ToHelper::Files { id, request }.frame()).map_err(|_| "The connection closed.".to_owned())?;
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(files::Reply::Error { message }) => Err(message),
            Ok(reply) => Ok(reply),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.files.lock().unwrap().remove(&id);
                Err("The server took too long to answer.".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("The connection closed.".into()),
        }
    }

    /// Start the runtime (or attach to the running one) and relay `listener`'s
    /// connections to it from now on. Blocks until it's ready; `on_message`
    /// hears `Progress` and `FoundJulia` meanwhile, and `notice` the first word
    /// of the runtime going away later, unless the app is the one stopping it.
    pub fn start_runtime(
        &self,
        listener: &Arc<Listener>,
        on_message: &mut dyn FnMut(ToApp),
        notice: impl FnOnce(Notice) + Send + 'static,
    ) -> Result<Runtime, String> {
        let events = self.subscribe();
        let leaving = Arc::new(AtomicBool::new(false));
        *self.leaving.lock().unwrap() = leaving.clone();
        self.mux.send(&ToHelper::StartRuntime { job: None }.frame()).map_err(|_| "The connection to Endeavor's helper closed.".to_owned())?;
        let runtime = loop {
            match events.recv() {
                Ok(message @ (ToApp::Progress { .. } | ToApp::FoundJulia { .. } | ToApp::Submitted { .. } | ToApp::Queued { .. })) => on_message(message),
                Ok(ToApp::Ready { node, token, pluto_secret, reattached, .. }) => {
                    *self.listener.lock().unwrap() = Some(listener.clone());
                    *listener.current.lock().unwrap() = Some(self.mux.clone());
                    let bridge = Bridge { url: listener.mcp_url(), token };
                    break Runtime { pluto_url: listener.pluto_url(&pluto_secret), bridge, reattached, node };
                }
                Ok(ToApp::StartFailed { message } | ToApp::Error { message }) => return Err(message),
                Ok(ToApp::Died { status, log_tail }) => {
                    let how = died_reason(&status, &[]);
                    return Err(format!("Julia stopped before Pluto was ready. {how}{}{}", if how.is_empty() { "" } else { " " }, diagnose(&log_tail)));
                }
                Ok(ToApp::Stopped) => return Err("Julia was stopped while it started.".into()),
                Ok(ToApp::Replaced) => return Err("Another connection took Julia over while it was starting.".into()),
                Ok(ToApp::Hello { .. } | ToApp::Files { .. }) => {}
                Err(_) => return Err("The connection to Endeavor's helper closed.".into()),
            }
        };
        std::thread::spawn(move || {
            let heard = loop {
                match events.recv() {
                    Ok(ToApp::Died { status, log_tail }) => break Notice::Died(died_reason(&status, &log_tail)),
                    Ok(ToApp::Replaced) => break Notice::Replaced,
                    Ok(ToApp::Error { message }) => break Notice::Lost(message),
                    Ok(_) => continue,
                    Err(_) => break Notice::Lost("The connection to Julia closed unexpectedly.".into()),
                }
            };
            if !leaving.load(Ordering::SeqCst) {
                notice(heard);
            }
        });
        Ok(runtime)
    }

    fn leave(&self) {
        self.leaving.lock().unwrap().store(true, Ordering::SeqCst);
    }

    /// Stop the runtime and wait until it's gone (blocks up to ~30 s). The
    /// helper stays connected.
    pub fn stop(&self) {
        self.leave();
        let events = self.subscribe();
        if self.mux.send(&ToHelper::Stop.frame()).is_ok() {
            while let Ok(message) = events.recv_timeout(Duration::from_secs(30)) {
                if message == ToApp::Stopped {
                    break;
                }
            }
        }
    }

    /// Leave the runtime running and wait until the helper has gone.
    pub fn detach(&self) {
        self.leave();
        let _ = self.mux.send(&ToHelper::Detach.frame());
        let _ = self.helper_exited.lock().unwrap().recv_timeout(Duration::from_secs(30));
    }

    /// The app is quitting: leave Julia running, or stop it. The helper does
    /// the rest after the app has gone.
    pub fn quit(&self, keep_running: bool) {
        self.leave();
        let message = if keep_running { ToHelper::Detach } else { ToHelper::Stop };
        let _ = self.mux.send(&message.frame());
    }
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

/// Why Julia stopped, in plain words, for "Julia on <host> stopped.": how a
/// cluster job ended (the helper's sentence), else how the process exited,
/// plus a cause from its log if one is recognized. A crash's log is mostly
/// Pluto's startup banner, so only a recognized cause is shown. Empty when
/// nothing is known.
pub fn died_reason(status: &str, log_tail: &[String]) -> String {
    let how = if status.ends_with('.') {
        Some(status.to_owned())
    } else if let Some(code) = status.strip_prefix("exit status: ") {
        Some(format!("It exited with code {code}."))
    } else if let Some(signal) = status.strip_prefix("signal: ") {
        let memory = if signal.starts_with("9 ") { ", perhaps for using too much memory" } else { "" };
        Some(format!("It was killed (signal {signal}){memory}."))
    } else {
        None
    };
    how.into_iter().chain(hint(&log_tail.join("\n")).map(str::to_owned)).collect::<Vec<_>>().join(" ")
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
    use super::{diagnose, died_reason, parse_version};

    #[test]
    fn says_plainly_why_julia_stopped() {
        assert_eq!(died_reason("exited", &[]), "");
        assert_eq!(died_reason("exit status: 3", &[]), "It exited with code 3.");
        assert_eq!(died_reason("signal: 9 (SIGKILL)", &[]), "It was killed (signal 9 (SIGKILL)), perhaps for using too much memory.");
        assert_eq!(died_reason("Its Slurm job reached its time limit.", &[]), "Its Slurm job reached its time limit.");
        assert!(died_reason("exited", &["IOError: listen: address already in use (EADDRINUSE)".into()]).contains("port"));
    }

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
    let listener = Listener::start().unwrap();
    let (channel, _) = connect(false, &|_| {}).expect("connect");
    let (heard_tx, heard) = mpsc::channel();
    let first = start_local(&channel, &listener, &|_| {}, move |notice| drop(heard_tx.send(notice))).expect("start");
    let list = crate::pluto::call_tool(&first.bridge, "list_notebooks", serde_json::json!({})).unwrap();
    println!("connected (reattached: {}); list_notebooks = {list}", first.reattached);
    // SIGKILL, like a crash or OOM kill.
    let state = std::fs::read_to_string(crate::install::app_dir().unwrap().join("runtime/runtime.json")).unwrap();
    let pid = serde_json::from_str::<serde_json::Value>(&state).unwrap()["pid"].to_string();
    Command::new("kill").args(["-9", &pid]).status().unwrap();
    match heard.recv_timeout(Duration::from_secs(30)).expect("death reported") {
        Notice::Died(reason) => {
            println!("died: {reason}");
            assert!(!reason.contains("Julia exited"));
        }
        other => panic!("expected Died, got {other:?}"),
    }

    // The helper stayed: the same channel starts a new one.
    let second = start_local(&channel, &listener, &|_| {}, |_| {}).expect("start again");
    assert!(!second.reattached);
    assert_eq!(second.bridge, first.bridge, "agent's MCP URL and token must survive a restart");
    assert_ne!(second.pluto_url, first.pluto_url, "new Pluto secret");
    let list = crate::pluto::call_tool(&second.bridge, "list_notebooks", serde_json::json!({})).unwrap();
    println!("restarted; list_notebooks = {list}");
    channel.stop();
}

#[cfg(test)]
#[test]
fn a_missing_julia_points_to_settings() {
    let err = check_version("/nonexistent/julia").unwrap_err();
    println!("{err}");
    assert!(err.contains("wasn't found") && err.contains("Settings"));
}
