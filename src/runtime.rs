//! The Julia runtime (Pluto + EndeavorRuntime, runtime/boot.jl), reached through
//! the `endeavor` helper (docs/remote-sessions.md): the app runs it as a
//! child on This Mac, and over ssh on a server (remote.rs). The helper answers
//! file requests from the start, and attaches to the runtime in its state
//! folder or starts one when asked. The webview, the agent and the app reach a
//! host's runtime through that host's local listener, whose one loopback port
//! stays the same for the whole launch.

use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use endeavor_mcp::client::{self, StartError, StartOptions};
use wire::ToApp;
use wire::slurm::JobRequest;

pub use endeavor_mcp::client::{Channel, Hello, Notice};

use crate::older_runtime::Version;
use crate::pluto::Bridge;
use crate::splash::{Progress, Step};

/// Julia needed by runtime/Project.toml's `[sources]` section.
const MIN_JULIA: (u32, u32) = (1, 11);

/// A host's listener (`endeavor_mcp::client::Listener`): the app's loopback
/// port for its runtime, for Pluto's page, the agent's MCP and the app's
/// calls. Each connection is relayed to the runtime of the moment. It refuses
/// the code runs of a runtime too old to ask before a run
/// (`older_runtime::refusal`).
pub struct Listener {
    relay: Arc<client::Listener>,
    version: Arc<Known>,
}

/// The attached runtime's version (`older_runtime::Version`); none while a
/// start is under way.
#[derive(Default)]
struct Known {
    version: Mutex<Option<Version>>,
    changed: Condvar,
}

/// How long a call waits for the version of a runtime being attached: only
/// from when the listener relays to it until `start_runtime` returns.
const VERSION_WAIT: Duration = Duration::from_secs(2);

impl Known {
    fn set(&self, version: Option<Version>) {
        *self.version.lock().unwrap() = version;
        self.changed.notify_all();
    }

    fn refusal(&self, tool: &str, arguments: &serde_json::Value) -> Option<String> {
        if !endeavor_mcp::runs_code(tool, arguments) {
            return None;
        }
        let version = self.version.lock().unwrap();
        let (version, _) = self.changed.wait_timeout_while(version, VERSION_WAIT, |v| v.is_none()).unwrap();
        crate::older_runtime::refusal((*version)?, tool, arguments)
    }
}

impl Listener {
    pub fn start(name: &str) -> Result<Arc<Listener>, String> {
        let version = Arc::new(Known::default());
        let known = version.clone();
        let relay = client::Listener::with_refuse(name, Box::new(move |_session, tool, arguments| known.refusal(tool, arguments)))?;
        Ok(Arc::new(Listener { relay, version }))
    }

    /// This Mac's Julia is restarting (Settings → Restart Julia).
    pub fn restarting(&self) {
        self.relay.restarting();
    }

    /// The restart `restarting` announced didn't work out: Julia didn't come back.
    pub fn restart_failed(&self) {
        self.relay.restart_failed();
    }

    /// The user stopped or disconnected `self`'s host on purpose: nothing
    /// will reconnect it by itself, unlike a drop.
    pub fn disconnected(&self) {
        self.relay.disconnected();
    }
}

/// A runtime the app is attached to, as its host's listener serves it.
#[derive(Clone, Debug)]
pub struct Runtime {
    /// Pluto's start page, with the token that lets the web view in.
    pub page_url: String,
    pub bridge: Bridge,
    /// Its process, which tells it from the host's next runtime (the URLs
    /// and the token stay the same).
    pub pid: u32,
    /// It was already running (kept after the last quit, or on a server); this
    /// connect didn't start it.
    pub reattached: bool,
    /// The machine it runs on.
    pub node: String,
    /// The cluster job it runs in.
    pub job: Option<wire::slurm::Job>,
    /// How the app can use it: as it is, or with a note (`older_runtime`).
    pub version: Version,
}

impl From<client::Runtime> for Runtime {
    fn from(runtime: client::Runtime) -> Runtime {
        let bridge = Bridge { url: runtime.mcp_url, token: runtime.token };
        let version = Version::of(runtime.build.as_deref(), runtime.interface, crate::remote::build());
        Runtime { page_url: runtime.page_url, bridge, pid: runtime.pid, reattached: runtime.reattached, node: runtime.node, job: runtime.job, version }
    }
}

/// Start the runtime on `channel` (or attach to the one running) and relay
/// `listener`'s connections to it from now on; on a cluster, `job` is what to
/// submit. Blocks until it's ready; `on_message` hears `Progress`, `Found`,
/// `Submitted` and `Queued` meanwhile, and `notice` the first word of the
/// runtime going away later, unless the app is the one stopping it. The helper
/// installs what the start needs without asking, as it always has.
pub fn start_runtime(channel: &Channel, listener: &Arc<Listener>, job: Option<JobRequest>, on_message: &mut dyn FnMut(ToApp), notice: impl FnOnce(Notice) + Send + 'static) -> Result<Runtime, String> {
    let options = StartOptions { job, install: true, ..StartOptions::default() };
    // A call that reaches the new runtime before this returns waits for its version.
    let before = listener.version.version.lock().unwrap().take();
    let started = channel.start_runtime(&listener.relay, &options, on_message, notice).map(Runtime::from).map_err(StartError::message);
    // A start that failed left the runtime before it attached, if any.
    listener.version.set(started.as_ref().map_or(before, |runtime| Some(runtime.version)));
    started
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

/// `endeavor --helper ARGS…` runs as the helper, like `endeavor ARGS…` on a server (see main).
pub const HELPER_FLAG: &str = "--helper";

/// The helper binary as cargo builds it, next to the app's own executable
/// (`cargo test` runs from target/*/deps): what a macOS server is sent from a
/// source checkout. The app has it in `helpers/` with the Linux ones
/// (`remote::helper_for`). This Mac runs the app itself as its helper instead
/// (`helper_command`).
pub fn helper_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no executable folder")?;
    [Some(dir), dir.parent()]
        .into_iter()
        .flatten()
        .map(|d| d.join("endeavor-helper"))
        .find(|p| p.exists())
        .ok_or_else(|| format!("Endeavor's runtime helper (endeavor-helper) is missing from {}.", dir.display()))
}

/// This Mac's helper: the app itself in helper mode, named `endeavor` in its
/// argv[0] like a server's helper, with `--helper` telling it from the app in
/// `ps`. A test binary can't act as the helper, so tests run the built one.
pub fn helper_command() -> Result<Command, String> {
    if cfg!(test) {
        return Ok(Command::new(helper_binary()?));
    }
    let mut command = Command::new(helper_program()?);
    #[cfg(unix)]
    command.arg0("endeavor");
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
        .arg(crate::install::runtime()?)
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
    // kill the helper before it can stop Julia. On Windows the helper is the
    // app's own GUI program, so it opens no console window either way.
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
    stop_recorded_in(&state_dir);
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

/// `endeavor.exe --stop-runtime`: the Windows installer stops a runtime kept
/// running before it replaces or removes the exe, which that runtime's core
/// runs from. Its open notebooks are already saved (Pluto saves on each change).
pub const STOP_FLAG: &str = "--stop-runtime";

/// Stop the runtime recorded in This Mac's state folder, if one still runs.
pub fn stop_recorded() {
    if let Ok(app_dir) = crate::install::app_dir() {
        stop_recorded_in(&app_dir.join("runtime"));
    }
}

fn stop_recorded_in(state_dir: &Path) {
    let recorded = std::fs::read_to_string(state_dir.join("runtime.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    if let Some(recorded) = recorded
        && let Some(pid) = recorded["pid"].as_i64().and_then(|p| i32::try_from(p).ok()).filter(|&p| p > 1)
    {
        stop_group(pid, recorded["started"].as_u64());
    }
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
    endeavor_mcp::end_recorded_runtime(pid, started);
}

/// Start This Mac's runtime on `channel` (or attach to the one running) and
/// relay `listener` to it. `progress` hears Julia's log while it starts;
/// `notice` hears if it goes away later.
pub fn start_local(channel: &Channel, listener: &Arc<Listener>, progress: &dyn Fn(Progress), notice: impl FnOnce(Notice) + Send + 'static) -> Result<Runtime, String> {
    progress(Progress::new(Step::Packages, "Starting Julia…"));
    let runtime = start_runtime(channel, listener, None, &mut |message| {
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

fn check_version(julia: &str) -> Result<(), String> {
    let mut command = Command::new(julia);
    endeavor_mcp::client::no_window(&mut command);
    let output = command.arg("--version").output().map_err(|e| {
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
    let mut command = Command::new(path);
    endeavor_mcp::client::no_window(&mut command);
    match command.arg("--version").output() {
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

#[cfg(test)]
mod tests {
    use super::{ChosenJulia, chosen_from_version, depot_list, parse_version};

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
    fn parses_julia_version() {
        assert_eq!(parse_version("julia version 1.12.6\n"), Some((1, 12)));
        assert_eq!(parse_version("julia version 1.10.0-rc1"), Some((1, 10)));
        assert!(parse_version("julia version 1.10.0").unwrap() < (1, 11));
        assert_eq!(parse_version("garbage"), None);
    }

    fn ready(build: Option<&str>, interface: Option<u32>) -> super::Runtime {
        let runtime = endeavor_mcp::client::Runtime {
            port: 1,
            token: "t".into(),
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            page_url: "http://127.0.0.1:1/".into(),
            pid: 2,
            reattached: true,
            node: "lab".into(),
            job: None,
            remote_port: None,
            build: build.map(str::to_owned),
            interface,
        };
        runtime.into()
    }

    #[test]
    fn a_ready_runtime_says_how_the_app_can_use_it() {
        use crate::older_runtime::Version;
        use endeavor_mcp::CORE_INTERFACE;
        assert_eq!(ready(Some("0.1.0-0000000000000000"), Some(CORE_INTERFACE)).version, Version::Usable);
        assert_eq!(ready(Some("0.1.0-0000000000000000"), None).version, Version::Other(None));
        assert_eq!(ready(None, Some(CORE_INTERFACE + 1)).version, Version::Other(Some(CORE_INTERFACE + 1)));
        assert_eq!(ready(None, None).version, Version::NoRunGate);
    }

    #[test]
    fn the_listener_refuses_code_runs_only_on_a_runtime_with_no_run_gate() {
        use crate::older_runtime::Version;
        let listener = super::Listener::start("lab-server").unwrap();
        let refused = |tool: &str| listener.version.refusal(tool, &serde_json::json!({ "cell_id": "a" })).is_some();
        for (build, interface) in [(None, None), (Some("0.1.0-0000000000000000"), None), (None, Some(endeavor_mcp::CORE_INTERFACE + 1))] {
            let version = ready(build, interface).version;
            listener.version.set(Some(version));
            assert_eq!(refused("execute_cell"), version == Version::NoRunGate, "{build:?} {interface:?}");
            assert!(!refused("read_cell"));
        }
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
    let (heard_tx, heard) = std::sync::mpsc::channel();
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
    assert_eq!(second.page_url, first.page_url, "the web view's URL survives a restart too");
    assert_ne!(second.pid, first.pid, "a new runtime");
    let list = crate::pluto::call_tool(&second.bridge, "list_notebooks", serde_json::json!({})).unwrap();
    println!("restarted; list_notebooks = {list}");
    channel.stop().unwrap();
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
    // Its group can be empty a moment before the child is ours to reap (seen on macOS), so poll.
    let stopped = (0..50).any(|_| {
        let gone = julia.try_wait().unwrap().is_some();
        if !gone {
            std::thread::sleep(Duration::from_millis(100));
        }
        gone
    });
    assert!(stopped, "the recorded runtime was stopped");
    for kept in ["runtime/token", "runtime/runtime.log", "depot/compiled/v1.12/Pluto/a.ji", "depot/packages/Pluto/x/src/Pluto.jl", "sessions.json"] {
        assert!(app.join(kept).exists(), "{kept} stays");
    }
    assert_eq!(clear_state_in(&app).unwrap(), Vec::<PathBuf>::new(), "nothing left to clear");
    let _ = std::fs::remove_dir_all(&app);
}

// What the Windows installer relies on before it replaces endeavor.exe: the
// recorded core is ended, and a pid reused by a later process isn't.
#[cfg(all(test, windows))]
#[test]
fn stop_recorded_ends_the_recorded_process_only() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};
    let state = std::env::temp_dir().join(format!("endeavor-stop-{}", std::process::id()));
    std::fs::create_dir_all(&state).unwrap();
    let mut core = Command::new("ping").args(["-n", "120", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
    let mut times = [FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }; 4];
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: four FILETIMEs to write into; a handle to our own child.
    assert_ne!(unsafe { GetProcessTimes(core.as_raw_handle(), created, exited, kernel, user) }, 0);
    let started = (u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime);
    let pid = core.id();
    let record = |started: u64| std::fs::write(state.join("runtime.json"), format!(r#"{{"pid": {pid}, "started": {started}}}"#)).unwrap();

    record(started + 1);
    stop_recorded_in(&state);
    assert!(core.try_wait().unwrap().is_none(), "another process with the recorded pid is left alone");
    record(started);
    stop_recorded_in(&state);
    assert!(core.try_wait().unwrap().is_some(), "the recorded process was stopped");
    let _ = std::fs::remove_dir_all(&state);
}

#[cfg(test)]
#[test]
fn a_missing_julia_points_to_settings() {
    let err = check_version("/nonexistent/julia").unwrap_err();
    println!("{err}");
    assert!(err.contains("wasn't found") && err.contains("Settings"));
}

/// The cloud VMs' setup script installs this Julia and the pinned Rust too (docs/cloud.md).
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[test]
fn the_cloud_setup_script_pins_the_same_julia() {
    let script = include_str!("../scripts/cloud-setup.sh");
    let (url, sha, _) = JULIA_TARBALL;
    let rust = include_str!("../rust-toolchain.toml").lines().find_map(|l| l.strip_prefix("channel = ")).unwrap().trim_matches('"');
    for line in [format!("JULIA_VERSION={JULIA_VERSION}"), format!("JULIA_URL={url}"), format!("JULIA_SHA256={sha}"), format!("RUST_TOOLCHAIN={rust}")] {
        assert!(script.lines().any(|l| l == line), "scripts/cloud-setup.sh should have `{line}`");
    }
}
