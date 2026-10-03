//! The Julia runtime (Pluto + EndeavorRuntime, runtime/boot.jl), reached through
//! the endeavor-remote helper (docs/remote-sessions.md): the app runs it as a
//! child on This Mac, and over ssh on a server (remote.rs). The helper answers
//! file requests from the start, and attaches to the runtime in its state
//! folder or starts one when asked. The webview and the agent reach a host's
//! runtime through that host's local listener, whose two loopback ports stay
//! the same for the whole launch.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use wire::files;
use wire::slurm::JobRequest;
use wire::relay::Mux;
use wire::{Frame, Target, ToApp, ToHelper};

use crate::pluto::Bridge;
use crate::splash::{Progress, Step};

/// Julia needed by runtime/Project.toml's `[sources]` section.
const MIN_JULIA: (u32, u32) = (1, 11);

/// How long a connection that arrives while the runtime is away waits for it
/// to come back, so a short drop goes unnoticed.
const HOLD: Duration = Duration::from_secs(if cfg!(test) { 0 } else { 2 });

/// The app's loopback ports for Pluto and a host's bridge. Each connection is
/// relayed to the runtime of the moment.
pub struct Listener {
    ports: [u16; 2],
    /// The host, as the agent is told it.
    name: String,
    upstream: Mutex<Upstream>,
    changed: Condvar,
    /// What the app knows of the runtime and its sessions, to hold back what
    /// a runtime from an older build can't do (`older_runtime`).
    watch: Mutex<Watch>,
    /// Said when the runtime's build is known.
    known: Condvar,
}

#[derive(Default)]
struct Watch {
    /// Whether the runtime is from another build than the app; none until it says.
    older: Option<bool>,
    /// Each session's mode (`Session::guard_mode`), by its key.
    policies: HashMap<String, &'static str>,
}

/// How long a call that an older runtime couldn't carry out waits for the
/// runtime to say which build it is, after the app attaches to it.
const BUILD_WAIT: Duration = Duration::from_secs(2);

/// Where a listener's connections go.
enum Upstream {
    /// No runtime yet: a connection is closed.
    None,
    Up { mux: Arc<Mux>, mcp: wire::McpTransport, token: String },
    /// The runtime was up and the app is getting it back. A connection waits
    /// for it up to `HOLD`; then an MCP request on the bridge is answered with
    /// `why` (`endeavor_remote::serve_unreachable`), and anything else closed.
    Away { mcp: wire::McpTransport, token: String, why: String },
}

impl Listener {
    pub fn start(name: &str) -> Result<Arc<Listener>, String> {
        let pluto = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let bridge = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let ports = [&pluto, &bridge].map(|l| l.local_addr().unwrap().port());
        let listener = Arc::new(Listener {
            ports,
            name: name.to_owned(),
            upstream: Mutex::new(Upstream::None),
            changed: Condvar::new(),
            watch: Mutex::default(),
            known: Condvar::new(),
        });
        for (socket, target) in [(pluto, Target::Pluto), (bridge, Target::Bridge)] {
            let listener = listener.clone();
            std::thread::spawn(move || {
                for connection in socket.incoming().map_while(Result::ok) {
                    let listener = listener.clone();
                    std::thread::spawn(move || listener.route(target, connection));
                }
            });
        }
        Ok(listener)
    }

    fn route(&self, target: Target, connection: TcpStream) {
        let _ = connection.set_nodelay(true);
        let upstream = self.upstream.lock().unwrap();
        let (upstream, _) = self.changed.wait_timeout_while(upstream, HOLD, |u| matches!(u, Upstream::Away { .. })).unwrap();
        match &*upstream {
            Upstream::Up { mux, .. } if target == Target::Bridge && self.watch.lock().unwrap().older != Some(false) => {
                let mux = mux.clone();
                drop(upstream);
                let Ok((ours, theirs)) = loopback_pair() else { return };
                if mux.open(target, theirs).is_ok() {
                    let _ = endeavor_remote::serve_guarded(connection, ours, &|session, tool, arguments| self.refusal(session, tool, arguments));
                }
            }
            Upstream::Up { mux, .. } => {
                let mux = mux.clone();
                drop(upstream);
                let _ = mux.open(target, connection);
            }
            Upstream::Away { mcp: wire::McpTransport::Http, token, why } if target == Target::Bridge => {
                let (token, why) = (token.clone(), why.clone());
                drop(upstream);
                let _ = connection.set_read_timeout(Some(Duration::from_secs(10)));
                let _ = endeavor_remote::serve_unreachable(connection, &token, &why);
            }
            Upstream::Away { .. } | Upstream::None => {}
        }
    }

    fn attach(&self, mux: Arc<Mux>, mcp: wire::McpTransport, token: String) {
        self.watch.lock().unwrap().older = None;
        *self.upstream.lock().unwrap() = Upstream::Up { mux, mcp, token };
        self.changed.notify_all();
    }

    /// The runtime said which build it came from: whether that's another than the app's.
    pub fn runtime_build(&self, older: bool) {
        self.watch.lock().unwrap().older = Some(older);
        self.known.notify_all();
    }

    /// Session `key`'s mode, as `Session::guard_mode` names it.
    pub fn set_mode(&self, key: u64, mode: &'static str) {
        self.watch.lock().unwrap().policies.insert(key.to_string(), mode);
    }

    /// Why the agent's call to `tool` from session `session` is refused here:
    /// something the runtime, from an older build, can't do safely. A runtime
    /// that hasn't said which build it is yet counts as older once it has had
    /// time to say.
    fn refusal(&self, session: &str, tool: &str, arguments: &serde_json::Value) -> Option<String> {
        let watch = self.watch.lock().unwrap();
        let why = crate::older_runtime::refusal(watch.policies.get(session).copied().unwrap_or("ask"), tool, arguments)?;
        let (watch, _) = self.known.wait_timeout_while(watch, BUILD_WAIT, |w| w.older.is_none()).unwrap();
        (watch.older != Some(false)).then_some(why)
    }

    /// The runtime is away while `why` holds, if it was up.
    fn away(&self, why: String, only: Option<&Arc<Mux>>) {
        let mut upstream = self.upstream.lock().unwrap();
        if let Upstream::Up { mux, mcp, token } = &*upstream
            && only.is_none_or(|only| Arc::ptr_eq(only, mux))
        {
            *upstream = Upstream::Away { mcp: *mcp, token: token.clone(), why };
        }
    }

    /// `why` replaces an already-away runtime's reason, once more is known (a
    /// restart's outcome, say). No-op if it came back up, or was never away:
    /// callers only reach for this once `away` (or `restarting`) already ran.
    fn still_away(&self, why: String) {
        let mut upstream = self.upstream.lock().unwrap();
        if let Upstream::Away { mcp, token, .. } = &*upstream {
            *upstream = Upstream::Away { mcp: *mcp, token: token.clone(), why };
        }
    }

    /// This Mac's Julia is restarting (Settings → Restart Julia).
    pub fn restarting(&self) {
        self.away(format!("Endeavor is restarting Julia on {}. Try again in a moment.", self.name), None);
    }

    /// The restart `restarting` announced didn't work out: Julia didn't come back.
    pub fn restart_failed(&self) {
        self.still_away(format!("Julia on {} couldn't start. Use Restart Julia to try again.", self.name));
    }

    /// The user stopped or disconnected `self`'s host on purpose: nothing
    /// will reconnect it by itself, unlike a drop (`forget`).
    pub fn disconnected(&self) {
        self.away(format!("Endeavor isn't connected to {}. Reconnect it to use its notebook again.", self.name), None);
    }

    /// The bridge URL the agent's MCP config carries; the same for the whole
    /// launch. `/mcp` for a core that speaks Streamable HTTP, else the
    /// deprecated SSE transport (`/sse`) a runtime from before that still serves.
    fn mcp_url(&self, mcp: wire::McpTransport) -> String {
        let path = match mcp { wire::McpTransport::Http => "mcp", wire::McpTransport::Sse => "sse" };
        format!("http://127.0.0.1:{}/{path}", self.ports[1])
    }

    pub fn bridge_port(&self) -> u16 {
        self.ports[1]
    }

    fn pluto_url(&self, secret: &str) -> String {
        format!("http://127.0.0.1:{}/?secret={secret}", self.ports[0])
    }

    /// `mux`'s helper has gone; the app reconnects by itself.
    fn forget(&self, mux: &Arc<Mux>) {
        self.away(format!("Endeavor lost the connection to {} and is reconnecting by itself. Try again in a moment.", self.name), Some(mux));
    }
}

/// Two ends of a loopback connection: one to read and write here, one for the relay.
fn loopback_pair() -> std::io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let ours = TcpStream::connect(listener.local_addr()?)?;
    let (theirs, _) = listener.accept()?;
    let _ = ours.set_nodelay(true);
    let _ = theirs.set_nodelay(true);
    Ok((ours, theirs))
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
    /// The cluster job it runs in.
    pub job: Option<wire::slurm::Job>,
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
/// official tarballs' (Windows: zip's) SHA-256 and size (bump all three per release).
pub const JULIA_VERSION: &str = "1.12.6";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/mac/aarch64/1.12/julia-1.12.6-macaarch64.tar.gz",
    "277d82fbd2eda99d0963b3e41f3dc979d7486f181399f8430fb637318ccd6a31",
    231_027_185,
);
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/mac/x64/1.12/julia-1.12.6-mac64.tar.gz",
    "1a70b7c606d6bac38a246e722369e5b30914dccf9378499d2712fb3bd282642c",
    271_518_180,
);
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/linux/aarch64/1.12/julia-1.12.6-linux-aarch64.tar.gz",
    "029b93b857bd0ffd627f9a8580d3bbaa63daf008d7b7aed02fbceb8fd57c4899",
    306_918_080,
);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/linux/x64/1.12/julia-1.12.6-linux-x86_64.tar.gz",
    "bbabf3bef19421a9dbd24a767d807606ab85e444323b5a1c73ffe293fa3d079a",
    289_794_236,
);
// There is no Windows ARM64 build of Julia 1.12, so Windows on ARM gets the
// x64 one, which it runs under emulation.
#[cfg(windows)]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/winnt/x64/1.12/julia-1.12.6-win64.zip",
    "a63d991976e6893f508c512e3dc7bca1836c1a1f6ad1f3e4aedec159b6733e89",
    275_091_967,
);


/// The julia binary to run: the user's (Settings) while it's there and is
/// Julia, else the app's own, downloaded and verified on first run.
/// `progress` hears how that's going.
fn julia_binary(progress: &dyn Fn(String, Option<f32>)) -> Result<String, String> {
    if let Some(julia) = crate::settings::Settings::load().julia {
        match check_chosen(&julia) {
            ChosenJulia::Usable(_) => return Ok(julia.display().to_string()),
            problem => eprintln!("The chosen Julia {} can't be used ({problem:?}); using Endeavor's.", julia.display()),
        }
    }
    let dir = crate::install::app_dir()?.join(format!("julia-{JULIA_VERSION}"));
    let bin = dir.join("bin").join(format!("julia{}", std::env::consts::EXE_SUFFIX));
    if !bin.exists() {
        crate::install::tarball(&dir, &format!("Julia {JULIA_VERSION}"), &format!("julia-{JULIA_VERSION}"), JULIA_TARBALL, progress)?;
    }
    Ok(bin.display().to_string())
}

/// `endeavor --helper ARGS…` runs `endeavor-remote ARGS…` (see main).
pub const HELPER_FLAG: &str = "--helper";

/// The helper binary next to the app's own executable (`cargo test` runs from
/// target/*/deps): what a macOS server is sent. This Mac runs the app itself
/// as its helper instead (`helper_command`).
pub fn helper_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no executable folder")?;
    [Some(dir), dir.parent()]
        .into_iter()
        .flatten()
        .map(|d| d.join("endeavor-remote"))
        .find(|p| p.exists())
        .ok_or_else(|| format!("Endeavor's runtime helper (endeavor-remote) is missing from {}.", dir.display()))
}

/// This Mac's helper: the app itself in helper mode, named `endeavor-remote`
/// in its argv[0] so `ps` and `pgrep -x endeavor` tell it from the app. A
/// test binary can't act as the helper, so tests run the built one.
pub fn helper_command() -> Result<Command, String> {
    if cfg!(test) {
        return Ok(Command::new(helper_binary()?));
    }
    let mut command = Command::new(helper_program()?);
    #[cfg(unix)]
    command.arg0("endeavor-remote");
    command.arg(HELPER_FLAG);
    Ok(command)
}

/// The program ssh runs as its askpass: the app, which acts as the helper's
/// askpass mode when started with its socket in the environment (see main).
pub fn helper_program() -> Result<PathBuf, String> {
    if cfg!(test) {
        return helper_binary();
    }
    std::env::current_exe().map_err(|e| format!("Couldn't find Endeavor's own program: {e}"))
}

/// Run This Mac's helper and wait for its hello. `keep_running` leaves the
/// runtime running if the app goes away without quitting; `progress` hears
/// about Julia's install.
pub fn connect(keep_running: bool, progress: &dyn Fn(Progress)) -> Result<(Channel, Hello), String> {
    let julia = julia_binary(&|detail, fraction| progress(Progress { fraction, ..Progress::new(Step::Julia, detail) }))?;
    check_version(&julia)?;
    let app_dir = crate::install::app_dir()?;
    let state_dir = app_dir.join("runtime");
    let depot = depot_list(&app_dir);

    let mut command = helper_command()?;
    command
        .args(["connect", "--state-dir"])
        .arg(&state_dir)
        .args(["--julia", &julia, "--runtime"])
        .arg(crate::install::resources().join("runtime"))
        .args(["--depot", &depot])
        // This Mac's state folder is its own, so another node name means a renamed Mac.
        .arg("--any-node");
    if let Some(build) = crate::remote::build() {
        command.args(["--build", build]);
    }
    if !keep_running {
        command.arg("--quit-with-client");
    }
    // Its own process group: a Ctrl-C meant for the app in a terminal must not
    // kill the helper before it can stop Julia.
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP);
    let mut helper = command.spawn().map_err(|e| format!("Couldn't start Endeavor's runtime helper: {e}"))?;
    let stdin = helper.stdin.take().unwrap();
    let stdout = helper.stdout.take().unwrap();
    let channel = Channel::open(helper, stdin, stdout);
    let hello = channel.wait_hello(|| "Endeavor's runtime helper stopped unexpectedly. Show logs has the details.".into())?;
    Ok((channel, hello))
}

/// JULIA_DEPOT_PATH for This Mac's runtime: the app's depot, then an empty
/// entry, which stacks the default depots (~/.julia) read-only behind ours.
fn depot_list(app_dir: &Path) -> String {
    format!("{}{}", app_dir.join("depot").display(), if cfg!(windows) { ';' } else { ':' })
}

/// Repair runtime: stop the runtime recorded in This Mac's state folder if one
/// still runs (no helper holds it by now), and remove what can go stale: the
/// state and lock files, and the precompiled EndeavorRuntime. The bridge token
/// (the agent's MCP config carries it), the log, the depot's packages and
/// Julia stay. Returns what it removed.
pub fn clear_state() -> Result<Vec<PathBuf>, String> {
    clear_state_in(&crate::install::app_dir()?)
}

fn clear_state_in(app_dir: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let state_dir = app_dir.join("runtime");
    let recorded = std::fs::read_to_string(state_dir.join("runtime.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    if let Some(recorded) = recorded
        && let Some(pid) = recorded["pid"].as_i64().and_then(|p| i32::try_from(p).ok()).filter(|&p| p > 1)
    {
        stop_group(pid, recorded["started"].as_u64());
    }
    let mut stale: Vec<PathBuf> = ["runtime.json", "runtime.json.tmp", "lock"].iter().map(|f| state_dir.join(f)).collect();
    if let Ok(versions) = std::fs::read_dir(app_dir.join("depot/compiled")) {
        stale.extend(versions.flatten().map(|v| v.path().join("EndeavorRuntime")));
    }
    let mut removed = Vec::new();
    for path in stale {
        let result = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match result {
            Ok(()) => removed.push(path),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Couldn't remove {}: {e}", path.display())),
        }
    }
    Ok(removed)
}

/// The runtime runs in its own session, so its recorded pid (the core's, or
/// Julia's for a runtime an older helper started) is its group's: end the
/// group (the core, Julia and its notebook workers), politely first.
#[cfg(unix)]
fn stop_group(pid: i32, _started: Option<u64>) {
    // SAFETY (both): plain syscalls; a group that's gone only returns ESRCH.
    let alive = || unsafe { libc::kill(-pid, 0) } == 0;
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        unsafe { libc::kill(-pid, signal) };
        for _ in 0..50 {
            if !alive() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Windows: the core recorded as `pid`, started at `started`, and with it
/// Julia and its workers (its Job Object).
#[cfg(windows)]
fn stop_group(pid: i32, started: Option<u64>) {
    endeavor_remote::end_recorded_runtime(pid, started);
}

/// Start This Mac's runtime on `channel` (or attach to the one running) and
/// relay `listener` to it. `progress` hears Julia's log while it starts;
/// `notice` hears if it goes away later.
pub fn start_local(channel: &Channel, listener: &Arc<Listener>, progress: &dyn Fn(Progress), notice: impl FnOnce(Notice) + Send + 'static) -> Result<Runtime, String> {
    progress(Progress::new(Step::Packages, "Starting Julia…"));
    let runtime = channel.start_runtime(listener, None, &mut |message| {
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
    /// Slurm's commands are there: probably a cluster's login node.
    pub slurm: bool,
    /// It can save attached files into a session's folder (`files::Request::Write`).
    pub uploads: bool,
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
    /// Set once the helper has exited (or its end of the channel closed).
    ended: Arc<(Mutex<bool>, Condvar)>,
    /// The app let the helper go (a detach or a quit), so its end is no drop.
    left: AtomicBool,
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
        let ended: Arc<(Mutex<bool>, Condvar)> = Arc::default();
        let listener: Arc<Mutex<Option<Arc<Listener>>>> = Arc::default();
        let files: Arc<Mutex<HashMap<u32, mpsc::Sender<files::Reply>>>> = Arc::default();
        std::thread::spawn({
            let (mux, sink, listener, files, ended) = (mux.clone(), sink.clone(), listener.clone(), files.clone(), ended.clone());
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
                *ended.0.lock().unwrap() = true;
                ended.1.notify_all();
            }
        });
        Channel {
            mux,
            sink,
            hello: Mutex::new(Some(hello)),
            files,
            next_file: AtomicU32::new(0),
            ended,
            left: AtomicBool::new(false),
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
            Ok(ToApp::Hello { node, home, slurm, uploads, .. }) => Ok(Hello { node, home: PathBuf::from(home), slurm, uploads }),
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
    /// connections to it from now on; on a cluster, `job` is what to submit.
    /// Blocks until it's ready; `on_message` hears `Progress`, `FoundJulia`,
    /// `Submitted` and `Queued` meanwhile, and `notice` the first word of the
    /// runtime going away later, unless the app is the one stopping it.
    pub fn start_runtime(
        &self,
        listener: &Arc<Listener>,
        job: Option<JobRequest>,
        on_message: &mut dyn FnMut(ToApp),
        notice: impl FnOnce(Notice) + Send + 'static,
    ) -> Result<Runtime, String> {
        let events = self.subscribe();
        let leaving = Arc::new(AtomicBool::new(false));
        *self.leaving.lock().unwrap() = leaving.clone();
        self.mux.send(&ToHelper::StartRuntime { job }.frame()).map_err(|_| "The connection to Endeavor's helper closed.".to_owned())?;
        let runtime = loop {
            match events.recv() {
                Ok(message @ (ToApp::Progress { .. } | ToApp::FoundJulia { .. } | ToApp::Submitted { .. } | ToApp::Queued { .. })) => on_message(message),
                Ok(ToApp::Ready { node, token, pluto_secret, reattached, job, mcp, .. }) => {
                    *self.listener.lock().unwrap() = Some(listener.clone());
                    listener.attach(self.mux.clone(), mcp, token.clone());
                    let bridge = Bridge { url: listener.mcp_url(mcp), token, transport: mcp };
                    break Runtime { pluto_url: listener.pluto_url(&pluto_secret), bridge, reattached, node, job };
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
                    // The helper ended: `closed` tells of that.
                    Err(_) => return,
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
        self.left.store(true, Ordering::SeqCst);
        let _ = self.mux.send(&ToHelper::Detach.frame());
        self.wait_end(Some(Duration::from_secs(30)));
    }

    /// Block until the helper has gone, or `timeout` passes.
    fn wait_end(&self, timeout: Option<Duration>) {
        let (ended, changed) = &*self.ended;
        let ended = ended.lock().unwrap();
        match timeout {
            Some(timeout) => drop(changed.wait_timeout_while(ended, timeout, |ended| !*ended)),
            None => drop(changed.wait_while(ended, |ended| !*ended)),
        }
    }

    /// Block until the helper has gone, whether or not Julia runs: `Lost` if
    /// it went by itself (ssh or the helper exited, or the network dropped and
    /// ssh's keepalive gave up), None if the app let it go.
    pub fn closed(&self) -> Option<Notice> {
        self.wait_end(None);
        (!self.left.load(Ordering::SeqCst)).then(|| Notice::Lost("The connection closed unexpectedly.".into()))
    }

    /// The app is quitting: leave Julia running, or stop it. The helper does
    /// the rest after the app has gone.
    pub fn quit(&self, keep_running: bool) {
        self.leave();
        self.left.store(true, Ordering::SeqCst);
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

/// What a julia the user chose in Settings turned out to be.
#[derive(Clone, Debug, PartialEq)]
pub enum ChosenJulia {
    /// It runs and is new enough; its version, e.g. "1.11.5".
    Usable(String),
    /// Nothing is at the path any more.
    Gone,
    /// Something is there, but it didn't answer as Julia does.
    NotJulia,
    /// Julia, but older than Endeavor needs; its version.
    TooOld(String),
}

/// Run the chosen file once to read its version (blocking; Julia answers in well under a second).
pub fn check_chosen(path: &Path) -> ChosenJulia {
    if !path.is_file() {
        return ChosenJulia::Gone;
    }
    match Command::new(path).arg("--version").output() {
        Ok(out) if out.status.success() => chosen_from_version(&String::from_utf8_lossy(&out.stdout)),
        _ => ChosenJulia::NotJulia,
    }
}

/// `julia --version`'s answer, as a check of the chosen file.
fn chosen_from_version(text: &str) -> ChosenJulia {
    let Some(version) = text.trim().strip_prefix("julia version ") else { return ChosenJulia::NotJulia };
    match parse_version(text) {
        Some(v) if v < MIN_JULIA => ChosenJulia::TooOld(version.to_owned()),
        Some(_) => ChosenJulia::Usable(version.to_owned()),
        None => ChosenJulia::NotJulia,
    }
}

/// The oldest Julia Endeavor runs, for saying so ("1.11").
pub fn min_julia() -> String {
    format!("{}.{}", MIN_JULIA.0, MIN_JULIA.1)
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
    use super::{ChosenJulia, chosen_from_version, depot_list, diagnose, died_reason, parse_version};

    #[test]
    fn the_depot_list_stacks_the_default_depots_behind_ours() {
        #[cfg(windows)]
        assert_eq!(depot_list(std::path::Path::new(r"C:\Users\jc\AppData\Local\Endeavor")), r"C:\Users\jc\AppData\Local\Endeavor\depot;");
        #[cfg(not(windows))]
        assert_eq!(depot_list(std::path::Path::new("/Users/jc/Library/Application Support/endeavor")), "/Users/jc/Library/Application Support/endeavor/depot:");
    }

    #[test]
    fn a_chosen_file_is_julia_when_it_says_so() {
        assert_eq!(chosen_from_version("julia version 1.11.5\n"), ChosenJulia::Usable("1.11.5".into()));
        assert_eq!(chosen_from_version("julia version 1.10.4\n"), ChosenJulia::TooOld("1.10.4".into()));
        assert_eq!(chosen_from_version("Python 3.12.1\n"), ChosenJulia::NotJulia);
        assert_eq!(chosen_from_version(""), ChosenJulia::NotJulia);
    }

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

    /// A listener for lab-server whose runtime spoke `mcp` and has gone away.
    fn away(mcp: wire::McpTransport) -> std::sync::Arc<super::Listener> {
        let listener = super::Listener::start("lab-server").unwrap();
        let mux = wire::relay::Mux::new(std::io::sink());
        listener.attach(mux.clone(), mcp, "secret".into());
        listener.forget(&mux);
        listener
    }

    /// The raw HTTP response to POST `body` to `/mcp` on the listener's bridge port.
    fn post(listener: &super::Listener, token: &str, body: &str) -> String {
        use std::io::{Read, Write};
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", listener.bridge_port())).unwrap();
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        let _ = socket.read_to_string(&mut response);
        response
    }

    #[test]
    fn a_tool_call_while_the_server_is_away_fails_with_a_reason_to_retry() {
        let response = post(&away(wire::McpTransport::Http), "secret", r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_notebooks","arguments":{}}}"#);
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert_eq!(
            body,
            r#"{"id":7,"jsonrpc":"2.0","result":{"content":[{"text":"Endeavor lost the connection to lab-server and is reconnecting by itself. Try again in a moment.","type":"text"}],"isError":true}}"#
        );
    }

    #[test]
    fn a_notification_while_the_server_is_away_is_accepted() {
        let response = post(&away(wire::McpTransport::Http), "secret", r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert!(response.starts_with("HTTP/1.1 202 Accepted\r\n"), "{response}");
    }

    #[test]
    fn a_ping_while_the_server_is_away_is_answered() {
        let response = post(&away(wire::McpTransport::Http), "secret", r#"{"jsonrpc":"2.0","id":"p","method":"ping"}"#);
        assert!(response.ends_with("\r\n\r\n{\"id\":\"p\",\"jsonrpc\":\"2.0\",\"result\":{}}"), "{response}");
    }

    #[test]
    fn malformed_input_while_the_server_is_away_is_a_bad_request() {
        let listener = away(wire::McpTransport::Http);
        assert!(post(&listener, "secret", "{not json").starts_with("HTTP/1.1 400 Bad Request\r\n"));
        assert!(post(&listener, "wrong", r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).starts_with("HTTP/1.1 401 Unauthorized\r\n"));
    }

    #[test]
    fn an_sse_runtime_away_closes_connections_as_before() {
        assert_eq!(post(&away(wire::McpTransport::Sse), "secret", r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#), "");
    }

    #[test]
    fn a_runtime_from_an_older_build_runs_nothing_in_ask_to_run() {
        use std::io::{Read, Write};
        let listener = super::Listener::start("lab-server").unwrap();
        listener.attach(wire::relay::Mux::new(std::io::sink()), wire::McpTransport::Http, "secret".into());
        listener.runtime_build(true);
        listener.set_mode(7, "ask");
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", listener.bridge_port())).unwrap();
        let body = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"execute_cell","arguments":{"cell_id":"a"}}}"#;
        let request = format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Endeavor-Session: 7\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        socket.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        let _ = socket.read_to_string(&mut response);
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(response.contains(r#"\"error\":\"older_runtime\""#) && response.contains(r#""isError":true"#), "{response}");
    }

    #[test]
    fn restarting_julia_says_so() {
        let listener = super::Listener::start("This Mac").unwrap();
        listener.attach(wire::relay::Mux::new(std::io::sink()), wire::McpTransport::Http, "secret".into());
        listener.restarting();
        let response = post(&listener, "secret", r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_notebooks"}}"#);
        assert!(response.contains(r#""text":"Endeavor is restarting Julia on This Mac. Try again in a moment.""#), "{response}");
    }

    #[test]
    fn a_restart_that_fails_says_so_instead_of_restarting_forever() {
        let listener = super::Listener::start("This Mac").unwrap();
        listener.attach(wire::relay::Mux::new(std::io::sink()), wire::McpTransport::Http, "secret".into());
        // Before restarting() ran, there's nothing to correct: still up, a no-op.
        listener.restart_failed();
        assert!(matches!(&*listener.upstream.lock().unwrap(), super::Upstream::Up { .. }));
        listener.restarting();
        listener.restart_failed();
        let response = post(&listener, "secret", r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_notebooks"}}"#);
        assert!(response.contains(r#""text":"Julia on This Mac couldn't start. Use Restart Julia to try again.""#), "{response}");
    }

    #[test]
    fn stopping_on_purpose_says_so_not_that_it_reconnects_by_itself() {
        let listener = super::Listener::start("lab-server").unwrap();
        let mux = wire::relay::Mux::new(std::io::sink());
        listener.attach(mux.clone(), wire::McpTransport::Http, "secret".into());
        listener.disconnected();
        let response = post(&listener, "secret", r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_notebooks"}}"#);
        assert!(
            response.contains(r#""text":"Endeavor isn't connected to lab-server. Reconnect it to use its notebook again.""#),
            "{response}"
        );
        // The drop that follows a deliberate stop doesn't overwrite that with "reconnecting by itself".
        listener.forget(&mux);
        let after = post(&listener, "secret", r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_notebooks"}}"#);
        assert_eq!(after, response);
    }
}

/// Live check of the recovery path through the helper (slow; starts Julia twice;
/// stop any Endeavor first, since it shares the state folder):
/// `cargo test -- --ignored live_die_and_restart --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn live_die_and_restart() {
    let listener = Listener::start("test").unwrap();
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

#[cfg(all(test, unix))]
#[test]
fn repair_clears_stale_state_and_keeps_the_rest() {
    let app = std::env::temp_dir().join(format!("endeavor-repair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&app);
    let write = |path: &str| {
        let path = app.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    };
    // A recorded runtime that still runs, in its own group like the core's (with Julia in it).
    let mut julia = Command::new("sleep").arg("600").process_group(0).spawn().unwrap();
    std::fs::create_dir_all(app.join("runtime")).unwrap();
    std::fs::write(app.join("runtime/runtime.json"), format!(r#"{{"pid": {}}}"#, julia.id())).unwrap();
    for kept in ["runtime/token", "runtime/runtime.log", "depot/compiled/v1.12/Pluto/a.ji", "depot/packages/Pluto/x/src/Pluto.jl", "sessions.json"] {
        write(kept);
    }
    write("runtime/lock");
    write("depot/compiled/v1.12/EndeavorRuntime/b.ji");

    let removed = clear_state_in(&app).unwrap();
    let removed: Vec<String> = removed.iter().map(|p| p.strip_prefix(&app).unwrap().display().to_string()).collect();
    assert_eq!(removed, ["runtime/runtime.json", "runtime/lock", "depot/compiled/v1.12/EndeavorRuntime"]);
    assert!(julia.try_wait().unwrap().is_some(), "the recorded runtime was stopped");
    for kept in ["runtime/token", "runtime/runtime.log", "depot/compiled/v1.12/Pluto/a.ji", "depot/packages/Pluto/x/src/Pluto.jl", "sessions.json"] {
        assert!(app.join(kept).exists(), "{kept} stays");
    }
    assert_eq!(clear_state_in(&app).unwrap(), Vec::<PathBuf>::new(), "nothing left to clear");
    let _ = std::fs::remove_dir_all(&app);
}

#[cfg(test)]
#[test]
fn a_missing_julia_points_to_settings() {
    let err = check_version("/nonexistent/julia").unwrap_err();
    println!("{err}");
    assert!(err.contains("wasn't found") && err.contains("Settings"));
}
