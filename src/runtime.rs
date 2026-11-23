use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use endeavor_mcp::client::{self, StartError, StartOptions};
use wire::ToApp;
use wire::backend::Backend;
use wire::slurm::JobRequest;

pub use endeavor_mcp::client::{Channel, Hello, Notice};

use crate::older_runtime::Version;
use crate::pluto::Bridge;
use crate::splash::{Progress, Step};

pub use endeavor_mcp::julia::{Chosen as ChosenJulia, JULIA_VERSION, check_chosen, min_version as min_julia};

pub struct Listener {
    relay: Arc<client::Listener>,
    version: Arc<Known>,
}

#[derive(Default)]
struct Known {
    version: Mutex<Option<Version>>,
    changed: Condvar,
    host: String,
    kinds: Mutex<HashMap<String, Backend>>,
}

const VERSION_WAIT: Duration = Duration::from_secs(2);

impl Known {
    fn set(&self, version: Option<Version>) {
        *self.version.lock().unwrap() = version;
        self.changed.notify_all();
    }

    fn refusal(&self, session: &str, tool: &str, arguments: &serde_json::Value) -> Option<String> {
        let kind = self.kinds.lock().unwrap().get(session).copied().unwrap_or(Backend::Pluto);
        if !endeavor_mcp::runs_code(tool, arguments) && (tool != "new_notebook" || kind == Backend::Pluto) {
            return None;
        }
        let version = self.version.lock().unwrap();
        let (version, _) = self.changed.wait_timeout_while(version, VERSION_WAIT, |v| v.is_none()).unwrap();
        crate::older_runtime::refusal((*version)?, &self.host, kind, tool, arguments)
    }
}

impl Listener {
    pub fn start(name: &str) -> Result<Arc<Listener>, String> {
        let version = Arc::new(Known { host: name.to_owned(), ..Known::default() });
        let known = version.clone();
        let relay = client::Listener::with_refuse(name, Box::new(move |session, tool, arguments| known.refusal(session, tool, arguments)))?;
        Ok(Arc::new(Listener { relay, version }))
    }

    pub fn session_kind(&self, key: u64, kind: Backend) {
        self.version.kinds.lock().unwrap().insert(key.to_string(), kind);
    }

    pub fn set_pictures(&self, pictures: Box<endeavor_mcp::Pictures>) {
        self.relay.set_pictures(pictures);
    }

    pub fn set_restart(&self, ask: Box<endeavor_mcp::AskRestart>) {
        self.relay.set_restart(ask);
    }

    pub fn restarting(&self) {
        self.relay.restarting();
    }

    pub fn restart_failed(&self) {
        self.relay.restart_failed();
    }

    pub fn disconnected(&self) {
        self.relay.disconnected();
    }
}

#[derive(Clone, Debug)]
pub struct Runtime {
    pub page_url: String,
    pub bridge: Bridge,
    pub pid: u32,
    pub reattached: bool,
    pub node: String,
    pub job: Option<wire::slurm::Job>,
    pub version: Version,
}

impl From<client::Runtime> for Runtime {
    fn from(runtime: client::Runtime) -> Runtime {
        let bridge = Bridge { url: runtime.mcp_url, token: runtime.token, page_token: runtime.page_token };
        let version = Version::of(runtime.build.as_deref(), runtime.interface, crate::remote::build());
        Runtime { page_url: runtime.page_url, bridge, pid: runtime.pid, reattached: runtime.reattached, node: runtime.node, job: runtime.job, version }
    }
}

pub fn start_runtime(channel: &Channel, listener: &Arc<Listener>, job: Option<JobRequest>, on_message: &mut dyn FnMut(ToApp), notice: impl FnOnce(Notice) + Send + 'static) -> Result<Runtime, String> {
    let options = StartOptions { job, install: true, ..StartOptions::default() };
    let before = listener.version.version.lock().unwrap().take();
    let started = channel.start_runtime(&listener.relay, &options, on_message, notice).map(Runtime::from).map_err(StartError::message);
    listener.version.set(started.as_ref().map_or(before, |runtime| Some(runtime.version)));
    started
}

fn julia_arg() -> String {
    let source = endeavor_mcp::julia::choose(&crate::settings::Settings::load().julia_preference(), None);
    #[cfg(not(windows))]
    if source == endeavor_mcp::julia::Source::Auto || source == endeavor_mcp::julia::Source::Own {
        move_old_julia();
    }
    source.value().unwrap_or_else(|| "auto".into())
}

#[cfg(not(windows))]
fn own_julia_dir() -> PathBuf {
    endeavor_mcp::paths::Env::here().server_root().join(format!("julia-{JULIA_VERSION}"))
}

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
        Err(e) => eprintln!("Couldn't move {} to {}: {e}", old.display(), new.display()),
    }
}

pub const HELPER_FLAG: &str = "--helper";

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

pub fn helper_program() -> Result<PathBuf, String> {
    if cfg!(test) {
        return helper_binary();
    }
    std::env::current_exe().map_err(|e| format!("Couldn't find Endeavor's own program: {e}"))
}

static STARTED_WITH: Mutex<Option<String>> = Mutex::new(None);

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
        .args(client::Server { r: crate::settings::Settings::load().r, ..Default::default() }.r_args())
        .arg("--own-r")
        .arg("--any-node");
    if let Some(build) = crate::remote::build() {
        command.args(["--build", build]);
    }
    command.arg("--exit-idle-after-client");
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

pub fn state_dir() -> PathBuf {
    endeavor_mcp::paths::Env::here().state_dir()
}

fn old_state_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("runtime")
}

fn retire_old_state(app_dir: &Path) {
    retire_old_state_in(&old_state_dir(app_dir), &stop_old);
}

fn stop_old(old: &Path) -> bool {
    let Some(pid) = record(old).and_then(|r| r["pid"].as_u64()) else { return true };
    if runs_from(pid, old) {
        stop_recorded_in(old);
    } else {
        eprintln!("The runtime recorded in {} (pid {pid}) isn't running from there; not stopping that process.", old.display());
    }
    i32::try_from(pid).is_ok_and(group_gone)
}

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

#[cfg(windows)]
fn group_gone(_pgid: i32) -> bool {
    false
}

#[cfg(unix)]
fn runs_from(pid: u64, state_dir: &Path) -> bool {
    let args = Command::new("ps").args(["-ww", "-p", &pid.to_string(), "-o", "args="]).stderr(Stdio::null()).output();
    let Ok(out) = args else { return false };
    let (args, flag) = (String::from_utf8_lossy(&out.stdout), format!("--state-dir {}", state_dir.display()));
    args.match_indices(&flag).any(|(at, _)| args[at + flag.len()..].chars().next().is_none_or(char::is_whitespace))
}

#[cfg(windows)]
fn runs_from(_pid: u64, _state_dir: &Path) -> bool {
    true
}

fn retire_old_state_in(old: &Path, stop: &dyn Fn(&Path) -> bool) {
    if !old.join("runtime.json").exists() {
        return;
    }
    if stop(old) {
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

fn record(state_dir: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(state_dir.join("runtime.json")).ok().and_then(|t| serde_json::from_str(&t).ok())
}

fn started_by_plugin(record: &serde_json::Value) -> bool {
    record.get("folder").is_some_and(|folder| !folder.is_null())
}

fn depot_list(app_dir: &Path) -> String {
    format!("{}{}", app_dir.join("depot").display(), if cfg!(windows) { ';' } else { ':' })
}

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

pub const STOP_FLAG: &str = "--stop-runtime";

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

#[cfg(windows)]
fn stop_group(pid: i32, started: Option<u64>) {
    endeavor_mcp::end_recorded_runtime(pid, started);
}

pub fn start_local(channel: &Channel, listener: &Arc<Listener>, progress: &dyn Fn(Progress), notice: impl FnOnce(Notice) + Send + 'static) -> Result<Runtime, String> {
    progress(Progress::new(Step::Runtime, "Starting the notebook runtime…"));
    let runtime = start_runtime(channel, listener, None, &mut |message| {
        if let ToApp::Progress { line } = message {
            eprintln!("{line}");
        }
    }, notice)?;
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
        let refused = |tool: &str| listener.version.refusal("1", tool, &serde_json::json!({ "cell_id": "a" })).is_some();
        for (build, interface) in [(None, None), (Some("0.1.0-0000000000000000"), None), (None, Some(endeavor_mcp::CORE_INTERFACE + 1))] {
            let version = ready(build, interface).version;
            listener.version.set(Some(version));
            assert_eq!(refused("execute_cell"), version == Version::NoRunGate, "{build:?} {interface:?}");
            assert!(!refused("read_cell"));
        }
    }

    #[test]
    fn the_listener_refuses_a_pathless_new_notebook_in_an_r_session_on_a_runtime_without_session_kinds() {
        use crate::older_runtime::Version;
        use serde_json::json;
        use wire::backend::Backend;
        let listener = super::Listener::start("lab-server").unwrap();
        listener.session_kind(1, Backend::Ember);
        listener.session_kind(2, Backend::Pluto);
        let refused = |session: &str, arguments: serde_json::Value| listener.version.refusal(session, "new_notebook", &arguments);
        listener.version.set(Some(Version::Other(Some(5))));
        let why = refused("1", json!({})).expect("an R session's pathless new_notebook");
        assert!(why.contains("Endeavor on lab-server") && why.contains("ending in .R") && why.contains("request_restart"), "{why}");
        assert!(refused("1", json!({ "path": "a.R" })).is_none(), "a path says the kind itself");
        assert!(refused("2", json!({})).is_none(), "a Julia session's notebook is Julia's anyway");
        assert!(refused("3", json!({})).is_none(), "a session whose kind the app never said");
        listener.version.set(Some(Version::Other(Some(6))));
        assert!(refused("1", json!({})).is_none(), "the runtime knows session kinds");
        listener.version.set(Some(Version::Usable));
        assert!(refused("1", json!({})).is_none());
    }
}

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
    let mut other = Command::new("sleep").arg("60").process_group(0).spawn().unwrap();
    record(other.id());
    retire_old_state_in(&old, &stop_old);
    assert!(other.try_wait().unwrap().is_none(), "another process isn't stopped");
    assert!(!old.join("runtime.json").exists() && old.join("start.lock").exists(), "only the record goes");
    let _ = other.kill();
    let _ = other.wait();
    let spawn = Command::new("sh")
        .args(["-c", r#"perl -e 'setpgrp(0,0); exec @ARGV' sh -c "sleep 60; :" core --state-dir "$1" </dev/null >/dev/null 2>&1 & echo $!"#, "sh"])
        .arg(&old).output().unwrap();
    let core: u32 = String::from_utf8_lossy(&spawn.stdout).trim().parse().unwrap();
    wait_runs_from(core, &old);
    record(core);
    retire_old_state_in(&old, &stop_old);
    assert!(!old.exists(), "the folder goes with the runtime");
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
