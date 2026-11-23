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

/// Which Julia runs, and the checks of a chosen one, are EndeavorMCP's (`julia`): the app and a
/// server pick Julia one way.
pub use endeavor_mcp::julia::{Chosen as ChosenJulia, JULIA_VERSION, check_chosen, min_version as min_julia};

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

    /// The agent's `view_cell_output` gets `pictures`' picture when the runtime has none
    /// (`cell_shot`), told when it stops short of the output.
    pub fn set_pictures(&self, pictures: Box<endeavor_mcp::Pictures>) {
        self.relay.set_pictures(pictures);
    }

    /// The agent can ask the user to restart the runtime (`restart_ask`).
    pub fn set_restart(&self, ask: Box<endeavor_mcp::AskRestart>) {
        self.relay.set_restart(ask);
    }

    /// Julia is restarting (Settings → Restart Julia, or `restart_host`).
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
        let bridge = Bridge { url: runtime.mcp_url, token: runtime.token, page_token: runtime.page_token };
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



/// The core's `--julia`, as EndeavorMCP chooses it from Settings (`julia::choose`): the chosen
/// Julia while it's there and is Julia; else, unless Settings ask for Endeavor's, the user's own
/// (the login shell's, juliaup's) when it's new enough; else Endeavor's own, the pinned Julia, which
/// the core installs the first time a Julia notebook needs it (with juliaup where there is juliaup,
/// always on Windows).
fn julia_arg() -> String {
    let source = endeavor_mcp::julia::choose(&crate::settings::Settings::load().julia_preference(), None);
    #[cfg(not(windows))]
    if source == endeavor_mcp::julia::Source::Auto || source == endeavor_mcp::julia::Source::Own {
        move_old_julia();
    }
    source.value().unwrap_or_else(|| "auto".into())
}

/// Where the core keeps the Julia it downloads (EndeavorMCP's `julia::download_dir`).
#[cfg(not(windows))]
fn own_julia_dir() -> PathBuf {
    endeavor_mcp::paths::Env::here().server_root().join(format!("julia-{JULIA_VERSION}"))
}

/// The Julia an older Endeavor downloaded into the app's folder becomes the
/// core's own (`~/.cache/endeavor/julia-<version>`), so it isn't downloaded
/// again. Only while no runtime runs, since one may be running that Julia.
#[cfg(not(windows))]
fn move_old_julia() {
    let Ok(app_dir) = crate::install::app_dir() else { return };
    let old = app_dir.join(format!("julia-{JULIA_VERSION}"));
    let new = own_julia_dir();
    if !old.join("bin/julia").is_file() || new.exists() || app_dir.join("runtime/runtime.json").exists() {
        return;
    }
    let moved = new.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::rename(&old, &new));
    match moved {
        Ok(()) => eprintln!("Moved Endeavor's Julia {JULIA_VERSION} to {}", new.display()),
        // Another disk, say: the core downloads its own when it needs it, and the old one goes after the next start.
        Err(e) => eprintln!("Couldn't move {} to {}: {e}", old.display(), new.display()),
    }
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

/// The `--julia` the last `connect` gave the helper: what a runtime it starts runs (`auto`: the core's choice).
static STARTED_WITH: Mutex<Option<String>> = Mutex::new(None);

/// Run This Mac's helper and wait for its hello. Julia itself is installed and
/// started the first time a Julia notebook needs it, not with the runtime
/// (`--julia-when-needed`).
pub fn connect() -> Result<(Channel, Hello), String> {
    let julia = julia_arg();
    *STARTED_WITH.lock().unwrap() = Some(julia.clone());
    let app_dir = crate::install::app_dir()?;
    retire_old_state(&app_dir);
    let state_dir = state_dir();
    let depot = depot_list(&app_dir);

    let mut command = helper_command()?;
    command
        .args(["connect", "--state-dir"])
        .arg(&state_dir)
        .args(["--julia", &julia, "--julia-when-needed", "--runtime"])
        .arg(crate::install::runtime()?)
        .args(["--depot", &depot])
        // Read as a server's R setting is: a path, or a shell line. R starts with the first R notebook.
        .args(client::Server { r: crate::settings::Settings::load().r, ..Default::default() }.r_args())
        // This computer, and only it, may be offered Endeavor's own R when none is found (a Mac).
        .arg("--own-r")
        // The state folder is named after this computer, so a record in it with
        // another node name is this computer's from before it was renamed.
        .arg("--any-node");
    if let Some(build) = crate::remote::build() {
        command.args(["--build", build]);
    }
    // However the app goes (a quit, a crash, a kill), the runtime stays up until
    // its notebooks are idle, then ends once none is open (#274).
    command.arg("--exit-idle-after-client");
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

/// This Mac's state folder: the one the plugin and `endeavor serve` use here
/// (EndeavorMCP's `paths`), so the app and the plugin find one runtime and share
/// it rather than each starting its own on the same notebook (EndeavorMCP #110).
pub fn state_dir() -> PathBuf {
    endeavor_mcp::paths::Env::here().state_dir()
}

/// Where versions before the shared folder kept This Mac's runtime.
fn old_state_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("runtime")
}

/// A runtime an older version kept running in the app's own folder is stopped
/// and the old folder removed, once, so it doesn't run on beside the shared one.
/// Its notebooks were saved as they changed, and the app opens them again in
/// the new runtime. The app stops it itself rather than through the helper's
/// `stop`, which refuses a record made under another host name (a renamed Mac),
/// and only a process that is still that runtime.
fn retire_old_state(app_dir: &Path) {
    retire_old_state_in(&old_state_dir(app_dir), &stop_old);
}

/// Stop the runtime recorded in the old folder `old`, if it still runs from
/// there; whether its process group is gone now.
fn stop_old(old: &Path) -> bool {
    let Some(pid) = record(old).and_then(|r| r["pid"].as_u64()) else { return true };
    if runs_from(pid, old) {
        stop_recorded_in(old);
    } else {
        // A pid the system has given another process since, or a runtime this check can't see.
        eprintln!("The runtime recorded in {} (pid {pid}) isn't running from there; not stopping that process.", old.display());
    }
    i32::try_from(pid).is_ok_and(group_gone)
}

/// Whether no process of the group `pgid` runs any more: none is left (ESRCH),
/// or only zombies are.
#[cfg(unix)]
fn group_gone(pgid: i32) -> bool {
    // SAFETY: signal 0 only checks.
    if pgid <= 1 || unsafe { libc::kill(-pgid, 0) } != 0 {
        return pgid > 1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    }
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pgid=,stat="]).stderr(Stdio::null()).output() else { return false };
    !String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next().and_then(|g| g.parse::<i32>().ok()) == Some(pgid) && fields.next().is_some_and(|stat| !stat.starts_with('Z'))
    })
}

/// Windows can't tell here, so the old folder stays (only its record goes).
#[cfg(windows)]
fn group_gone(_pgid: i32) -> bool {
    false
}

/// Whether process `pid` is a runtime started with `state_dir`, so a pid the
/// system gave another process since isn't ended. `-ww`: the whole command line. On Windows `stop_group`
/// checks the recorded start time instead.
#[cfg(unix)]
fn runs_from(pid: u64, state_dir: &Path) -> bool {
    let args = Command::new("ps").args(["-ww", "-p", &pid.to_string(), "-o", "args="]).stderr(Stdio::null()).output();
    let Ok(out) = args else { return false };
    let (args, flag) = (String::from_utf8_lossy(&out.stdout), format!("--state-dir {}", state_dir.display()));
    // The folder ends there: not a longer path that starts with it.
    args.match_indices(&flag).any(|(at, _)| args[at + flag.len()..].chars().next().is_none_or(char::is_whitespace))
}

#[cfg(windows)]
fn runs_from(_pid: u64, _state_dir: &Path) -> bool {
    true
}

/// `retire_old_state` for the folder `old`, where `stop` stops the runtime
/// recorded there and says whether it's gone. The record goes either way, so
/// this happens once; the folder (token, locks, log) goes only with the runtime.
fn retire_old_state_in(old: &Path, stop: &dyn Fn(&Path) -> bool) {
    if !old.join("runtime.json").exists() {
        return;
    }
    if stop(old) {
        // Nothing uses the old folder now: its record, token, locks and log go.
        if let Err(e) = std::fs::remove_dir_all(old) {
            eprintln!("Couldn't remove {}: {e}", old.display());
        }
    } else {
        eprintln!("The runtime kept in {} may still be running; removing only its record.", old.display());
        for file in ["runtime.json", "runtime.json.tmp"] {
            match std::fs::remove_file(old.join(file)) {
                Err(e) if e.kind() != ErrorKind::NotFound => eprintln!("Couldn't remove {}: {e}", old.join(file).display()),
                _ => {}
            }
        }
    }
    eprintln!("This Mac's runtime is now in {}, shared with the plugin.", state_dir().display());
}

/// `runtime.json` in `state_dir`, if there is one.
fn record(state_dir: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(state_dir.join("runtime.json")).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// A runtime `endeavor mcp` or `serve` started records its notebooks folder; one the app started has none.
fn started_by_plugin(record: &serde_json::Value) -> bool {
    record.get("folder").is_some_and(|folder| !folder.is_null())
}

/// JULIA_DEPOT_PATH for This Mac's runtime: the app's depot, then an empty
/// entry, behind which EndeavorMCP puts the user's own depot (~/.julia) when
/// Julia starts (`paths::with_user_depot`), so their registries and packages are found.
fn depot_list(app_dir: &Path) -> String {
    format!("{}{}", app_dir.join("depot").display(), if cfg!(windows) { ';' } else { ':' })
}

/// Repair runtime: stop the runtime recorded in This Mac's state folder if one
/// still runs (no helper holds it by now), and remove what can go stale: the
/// state and lock files, and the precompiled EndeavorRuntime. The bridge token
/// (the agent's MCP config carries it), the log, the depot's packages and
/// Julia stay. Returns what it removed.
pub fn clear_state() -> Result<Vec<PathBuf>, String> {
    clear_state_in(&state_dir(), &crate::install::app_dir()?)
}

fn clear_state_in(state_dir: &Path, app_dir: &Path) -> Result<Vec<PathBuf>, String> {
    stop_recorded_in(state_dir);
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

/// Stop the runtimes the app's exe runs, if they still run: the one recorded in
/// This Mac's state folder, unless the plugin started it from its own binary,
/// and one an older version kept in the app's folder.
pub fn stop_recorded() {
    if let Ok(app_dir) = crate::install::app_dir() {
        stop_recorded_in(&old_state_dir(&app_dir));
    }
    let state_dir = state_dir();
    if record(&state_dir).is_some_and(|r| !started_by_plugin(&r)) {
        stop_recorded_in(&state_dir);
    }
}

fn stop_recorded_in(state_dir: &Path) {
    if let Some(recorded) = record(state_dir)
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
/// relay `listener` to it. Julia isn't part of it: it starts when a Julia
/// notebook first needs it. `progress` hears the start; `notice` hears if the
/// runtime goes away later.
pub fn start_local(channel: &Channel, listener: &Arc<Listener>, progress: &dyn Fn(Progress), notice: impl FnOnce(Notice) + Send + 'static) -> Result<Runtime, String> {
    progress(Progress::new(Step::Runtime, "Starting the notebook runtime…"));
    let runtime = start_runtime(channel, listener, None, &mut |message| {
        if let ToApp::Progress { line } = message {
            eprintln!("{line}");
        }
    }, notice)?;
    // A fresh start means no runtime runs a Julia from the app's own folder:
    // remove those, in the background, since deleting 300 MB can take a while
    // on Windows. The core keeps its own Julia elsewhere (juliaup's on
    // Windows), so only a Julia chosen in Settings stays.
    if !runtime.reattached {
        std::thread::spawn(|| {
            let chosen = crate::settings::Settings::load().julia;
            let started = STARTED_WITH.lock().unwrap().clone();
            let keep: Vec<&Path> = chosen.as_deref().into_iter().chain(started.as_deref().map(Path::new)).collect();
            crate::install::remove_all_versions("julia-", &keep);
        });
    }
    let how = if runtime.reattached { "Reattached to" } else { "Started" };
    let log = state_dir().join("runtime.log").display().to_string();
    eprintln!("{how} the runtime on {}; its log is {log}", runtime.node);
    Ok(runtime)
}

#[cfg(test)]
mod tests {
    use super::depot_list;

    #[test]
    fn the_depot_list_asks_for_the_users_depot_behind_ours() {
        #[cfg(windows)]
        assert_eq!(depot_list(std::path::Path::new(r"C:\Users\jc\AppData\Local\Endeavor")), r"C:\Users\jc\AppData\Local\Endeavor\depot;");
        #[cfg(not(windows))]
        assert_eq!(depot_list(std::path::Path::new("/Users/jc/Library/Application Support/endeavor")), "/Users/jc/Library/Application Support/endeavor/depot:");
    }

    fn ready(build: Option<&str>, interface: Option<u32>) -> super::Runtime {
        let runtime = endeavor_mcp::client::Runtime {
            port: 1,
            token: "t".into(),
            page_token: "p".into(),
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
    let (channel, _) = connect().expect("connect");
    let (heard_tx, heard) = std::sync::mpsc::channel();
    let first = start_local(&channel, &listener, &|_| {}, move |notice| drop(heard_tx.send(notice))).expect("start");
    let list = crate::pluto::call_tool(&first.bridge, "list_notebooks", serde_json::json!({})).unwrap();
    println!("connected (reattached: {}); list_notebooks = {list}", first.reattached);
    // SIGKILL, like a crash or OOM kill.
    let state = std::fs::read_to_string(state_dir().join("runtime.json")).unwrap();
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

#[cfg(test)]
#[test]
fn an_old_runtime_is_stopped_once_and_its_folder_removed() {
    let old = std::env::temp_dir().join(format!("endeavor-retire-{}", std::process::id()));
    let stops = std::cell::Cell::new(0);
    let counted = |gone: bool| {
        let stops = &stops;
        move |_: &Path| -> bool {
            stops.set(stops.get() + 1);
            gone
        }
    };
    let fill = || {
        std::fs::create_dir_all(&old).unwrap();
        for file in ["runtime.json", "start.lock", "starting.lock", "token", "page_token", "runtime.log"] {
            std::fs::write(old.join(file), "{}").unwrap();
        }
    };
    std::fs::create_dir_all(&old).unwrap();
    retire_old_state_in(&old, &counted(true));
    assert_eq!(stops.get(), 0, "nothing recorded, nothing to stop");
    fill();
    retire_old_state_in(&old, &counted(false));
    assert!(!old.join("runtime.json").exists(), "the record goes, so this happens once");
    assert!(old.join("start.lock").exists() && old.join("runtime.log").exists(), "the folder stays while the runtime may run");
    retire_old_state_in(&old, &counted(true));
    assert_eq!(stops.get(), 1, "once");
    fill();
    retire_old_state_in(&old, &counted(true));
    assert_eq!(stops.get(), 2);
    assert!(!old.exists(), "the old folder goes, locks and all");
}

/// Wait until `ps` shows `pid` started with `--state-dir dir`: on macOS it can
/// show a child just spawned with the parent's command line until it execs.
#[cfg(all(test, unix))]
fn wait_runs_from(pid: u32, dir: &Path) {
    if !(0..50).any(|_| runs_from(u64::from(pid), dir) || { std::thread::sleep(Duration::from_millis(100)); false }) {
        let ps = Command::new("ps").args(["-ww", "-p", &pid.to_string(), "-o", "args="]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        panic!("pid {pid} isn't shown started with {}: ps says {ps:?}", dir.display());
    }
}

#[cfg(all(test, unix))]
#[test]
fn only_a_runtime_started_with_the_old_folder_is_taken_for_it() {
    let old = std::env::temp_dir().join(format!("endeavor-runs-from-{}", std::process::id()));
    let mut core = Command::new("sh").args(["-c", "sleep 60; :", "core", "--state-dir"]).arg(&old).spawn().unwrap();
    let mut other = Command::new("sleep").arg("60").spawn().unwrap();
    wait_runs_from(core.id(), &old);
    assert!(!runs_from(u64::from(other.id()), &old), "a pid another process has now");
    assert!(!runs_from(u64::from(core.id()), &old.join("x")), "started with another folder");
    let longer = PathBuf::from(format!("{}-files", old.display()));
    let mut near = Command::new("sh").args(["-c", "sleep 60; :", "core", "--state-dir"]).arg(&longer).spawn().unwrap();
    wait_runs_from(near.id(), &longer);
    assert!(!runs_from(u64::from(near.id()), &old), "a folder whose name starts with it");
    let _ = near.kill();
    let _ = near.wait();
    for child in [&mut core, &mut other] {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(all(test, unix))]
#[test]
fn the_old_folder_goes_only_once_its_runtime_is_gone() {
    use std::os::unix::process::CommandExt as _;
    let old = std::env::temp_dir().join(format!("endeavor-stop-old-{}", std::process::id()));
    let record = |pid: u32| {
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("runtime.json"), format!(r#"{{"pid": {pid}}}"#)).unwrap();
        std::fs::write(old.join("start.lock"), "").unwrap();
    };
    // A live pid that isn't a runtime started from there: not stopped, and the folder stays.
    let mut other = Command::new("sleep").arg("60").process_group(0).spawn().unwrap();
    record(other.id());
    retire_old_state_in(&old, &stop_old);
    assert!(other.try_wait().unwrap().is_none(), "another process isn't stopped");
    assert!(!old.join("runtime.json").exists() && old.join("start.lock").exists(), "only the record goes");
    let _ = other.kill();
    let _ = other.wait();
    // The old runtime, which isn't the app's child (launchd or init adopted it), leading its own
    // group: stopped, and the folder goes with it.
    let spawn = Command::new("sh")
        .args(["-c", r#"perl -e 'setpgrp(0,0); exec @ARGV' sh -c "sleep 60; :" core --state-dir "$1" </dev/null >/dev/null 2>&1 & echo $!"#, "sh"])
        .arg(&old).output().unwrap();
    let core: u32 = String::from_utf8_lossy(&spawn.stdout).trim().parse().unwrap();
    wait_runs_from(core, &old);
    record(core);
    retire_old_state_in(&old, &stop_old);
    assert!(!old.exists(), "the folder goes with the runtime");
    // Linux: a group left with only a zombie counts as gone (a container's PID 1 may never reap).
    #[cfg(target_os = "linux")]
    {
        let mut child = Command::new("sh").args(["-c", "sleep 60; :", "core", "--state-dir"]).arg(&old).process_group(0).spawn().unwrap();
        wait_runs_from(child.id(), &old);
        record(child.id());
        retire_old_state_in(&old, &stop_old);
        assert!(!old.exists(), "the folder goes while the stopped runtime is an unreaped zombie");
        let _ = child.wait();
    }
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

    let removed = clear_state_in(&app.join("runtime"), &app).unwrap();
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
    assert_eq!(clear_state_in(&app.join("runtime"), &app).unwrap(), Vec::<PathBuf>::new(), "nothing left to clear");
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

/// The cloud VMs' setup script installs this Julia and the pinned Rust too (docs/cloud.md).
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[test]
fn the_cloud_setup_script_pins_the_same_julia() {
    let script = include_str!("../scripts/cloud-setup.sh");
    let (url, sha, _) = endeavor_mcp::julia::tarball().unwrap();
    let rust = include_str!("../rust-toolchain.toml").lines().find_map(|l| l.strip_prefix("channel = ")).unwrap().trim_matches('"');
    for line in [format!("JULIA_VERSION={JULIA_VERSION}"), format!("JULIA_URL={url}"), format!("JULIA_SHA256={sha}"), format!("RUST_TOOLCHAIN={rust}")] {
        assert!(script.lines().any(|l| l == line), "scripts/cloud-setup.sh should have `{line}`");
    }
}
