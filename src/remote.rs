//! Reaching a runtime on a server over SSH (docs/remote-sessions.md): one
//! system `ssh` runs a short bootstrap script that installs the helper and
//! `runtime/` if this app version's aren't there yet, then becomes the helper,
//! whose frames use the rest of ssh's stdin/stdout. From there on it's the same
//! channel as This Mac's (`runtime::Channel`).
//!
//! ssh's password, two-factor and host-key prompts come to the app through the
//! helper's askpass mode and a private Unix socket (`Askpass`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use wire::ToApp;
use wire::slurm::JobRequest;
use wire::askpass::{Answer, Ask, SOCKET_ENV};

use crate::hosts::Server;
use crate::runtime::{Channel, Hello, Listener, Notice, Runtime};

/// What happened so far while connecting, for Test connection's steps.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The bootstrap script answered: ssh is in.
    Connected { os: String, arch: String },
    /// The helper and runtime for this app version were just installed (or already there).
    Helper { installed: bool },
    FoundJulia { path: String, version: String },
    /// A line of Julia's download or the runtime's boot log.
    Progress(String),
    /// A cluster job for Julia was submitted.
    Submitted { job: String, summary: String },
    /// It waits in the queue (Slurm's state and reason).
    Queued { state: String, reason: String },
    /// The runtime is up (or was already running) and its bridge answered through the app's listener.
    Started { node: String, reattached: bool },
    /// Test connection on a cluster: what Slurm says there.
    Slurm(wire::slurm::Scheduler),
    /// Test connection is done with it: stopped, or left running as it was found.
    Finished { stopped: bool },
}

/// How the bootstrap script reaches the server.
#[derive(Clone, Debug)]
pub enum Transport {
    Ssh { host: String, port: Option<u16> },
    /// Debug builds and tests: the script through a local `sh`, parsed the way a
    /// login shell on the server parses ssh's command. `ask` first runs a
    /// made-up prompt through askpass, as ssh would.
    Shell { env: Vec<(String, String)>, ask: Option<String> },
}

impl Transport {
    /// In debug builds the SSH hosts `local-test`, `local-test-password` and
    /// `local-test-hostkey` connect to This Mac through `sh` instead.
    pub fn for_server(server: &Server) -> Transport {
        #[cfg(debug_assertions)]
        {
            let ask = match server.ssh_host.as_str() {
                "local-test" => Some(None),
                "local-test-password" => Some(Some(FAKE_PASSWORD.to_owned())),
                "local-test-hostkey" => Some(Some(FAKE_HOST_KEY.to_owned())),
                _ => None,
            };
            if let Some(ask) = ask {
                return Transport::Shell { env: Vec::new(), ask };
            }
        }
        Transport::Ssh { host: server.ssh_host.clone(), port: server.port }
    }

    fn command(&self, script: &str, askpass: Option<&Askpass>) -> Result<Command, String> {
        let remote = format!("sh -c '{script}'");
        let mut command = match self {
            Transport::Ssh { host, port } => {
                let mut command = Command::new("ssh");
                command.args(["-T", "-o", "ServerAliveInterval=15", "-o", "ConnectTimeout=20", "-o", "ForwardX11=no"]);
                if let Some(port) = port {
                    command.arg("-p").arg(port.to_string());
                }
                command.arg("--").arg(host).arg(remote);
                command
            }
            Transport::Shell { env, ask } => {
                let mut command = Command::new("sh");
                let first = ask.as_ref().map(|a| format!("{a} && ")).unwrap_or_default();
                command.arg("-c").arg(format!("{first}{remote}")).envs(env.iter().map(|(k, v)| (k, v)));
                command
            }
        };
        if let Some(askpass) = askpass {
            askpass.set_env(&mut command)?;
        }
        Ok(command)
    }

    fn host(&self) -> &str {
        match self {
            Transport::Ssh { host, .. } => host,
            Transport::Shell { .. } => "this Mac (local test)",
        }
    }
}

#[cfg(debug_assertions)]
const FAKE_PASSWORD: &str =
    r#""$SSH_ASKPASS" "local-test's password: " >/dev/null || { echo "local-test: Permission denied (publickey,password)." >&2; exit 255; }"#;
#[cfg(debug_assertions)]
const FAKE_HOST_KEY: &str = r#"[ "$("$SSH_ASKPASS" "The authenticity of host 'local-test (127.0.0.1)' can't be established.
ED25519 key fingerprint is SHA256:q0dTLnd9XJbDBwu3vYQfZkLr0oTqPqJ8Ne2yTnA3o1E.
This key is not known by any other names.
Are you sure you want to continue connecting (yes/no/[fingerprint])? ")" = yes ] || { echo "Host key verification failed." >&2; exit 255; }"#;

/// This app's helper and runtime, as the name of their folder on a server:
/// the app version plus a hash of what gets installed, so edits reinstall.
pub fn version() -> Result<String, String> {
    let mut hash = Fnv::default();
    for (path, contents, _) in runtime_files()? {
        hash.add(path.as_bytes());
        hash.add(&contents);
    }
    for helper in bundled_helpers() {
        hash.add(&std::fs::read(&helper).map_err(|e| format!("{}: {e}", helper.display()))?);
    }
    Ok(format!("{}-{:016x}", env!("CARGO_PKG_VERSION"), hash.0))
}

/// FNV-1a: stable across builds, unlike std's hasher.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn add(&mut self, bytes: &[u8]) {
        for &b in bytes.iter().chain(&[0xff]) {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// The script ssh runs on the server, as one line: the server's login shell
/// (sh, bash, zsh, fish or csh) gets it inside single quotes, so it holds no
/// quote, backslash, `!` or newline.
///
/// It prints `ENDEAVOR <os> <arch> <have|need>`, reads four lines (the
/// helper's Julia flag and its value, its launcher, and its state folder's
/// name), and if it needs the install, a byte count and then that many bytes
/// of tar. Then it becomes the helper.
pub fn bootstrap_script(version: &str) -> String {
    [
        &format!("v={version}"),
        r#"c="$HOME/.cache/endeavor""#,
        r#"d="$c/$v""#,
        r#"if [ -x "$d/endeavor-remote" ] && [ -f "$d/runtime/boot.jl" ]; then s=have; else s=need; fi"#,
        r#"echo "ENDEAVOR $(uname -s) $(uname -m) $s""#,
        r#"read -r jf && read -r jv && read -r ln && read -r sd || exit 1"#,
        r#"if [ $s = need ]; then read -r n || exit 1; t="$d.part.$$"; rm -rf "$t"; mkdir -p "$t" && head -c "$n" | (cd "$t" && tar xf -) || { rm -rf "$t"; echo "Endeavor: installing into $d failed" >&2; exit 1; }; rm -rf "$d"; mv "$t" "$d"; fi"#,
        r#"exec "$d/endeavor-remote" connect --state-dir "$c/$sd" --launcher "$ln" "$jf" "$jv" --runtime "$d/runtime" --depot "$c/depot:""#,
    ]
    .join("; ")
}

/// The files of `runtime/` as (path in the tar, contents, executable), sorted.
fn runtime_files() -> Result<Vec<(String, Vec<u8>, bool)>, String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>, bool)>) -> Result<(), String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
            let meta = std::fs::metadata(entry.path()).map_err(|e| e.to_string())?;
            if meta.is_dir() {
                walk(&entry.path(), &name, out)?;
            } else {
                let contents = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
                out.push((name, contents, meta.permissions().mode() & 0o111 != 0));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(&crate::install::resources().join("runtime"), "runtime", &mut files)?;
    Ok(files)
}

/// Folders holding helpers built for other platforms (scripts/build-helpers.sh):
/// inside Endeavor.app, or target/helpers in the source tree.
fn helper_dirs() -> Vec<PathBuf> {
    let resources = crate::install::resources();
    vec![resources.join("helpers"), resources.join("target/helpers")]
}

fn bundled_helpers() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = helper_dirs()
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .map(|platform| platform.path().join("endeavor-remote"))
        .filter(|p| p.is_file())
        .collect();
    found.extend(crate::runtime::helper_binary());
    found.sort();
    found
}

/// The helper binary for a server that says `uname -s` = `os`, `uname -m` = `arch`.
fn helper_for(os: &str, arch: &str) -> Result<PathBuf, String> {
    let arch = if arch == "arm64" { "aarch64" } else { arch };
    let platform = format!("{}-{arch}", os.to_lowercase());
    if let Some(helper) = helper_dirs().iter().map(|d| d.join(&platform).join("endeavor-remote")).find(|p| p.is_file()) {
        return Ok(helper);
    }
    if (os.to_lowercase().as_str(), arch) == HERE
        && let Ok(helper) = crate::runtime::helper_binary()
    {
        return Ok(helper);
    }
    Err(missing_helper(os, arch, crate::install::bundled()))
}

/// This Mac's platform as `helper_for` names a server's.
const HERE: (&str, &str) = (if cfg!(target_os = "macos") { "darwin" } else { std::env::consts::OS }, std::env::consts::ARCH);

const BUILD_HELPERS: &str = "scripts/build-helpers.sh";
const NO_HELPER: &str = " has no runtime helper for ";
const DOWNLOAD_AGAIN: &str = "Download Endeavor again";

/// Why a server can't be set up, and what to do: servers of a platform
/// Endeavor supports need its helper built (in a source checkout) or come
/// with a fresh download (in the app).
fn missing_helper(os: &str, arch: &str, bundled: bool) -> String {
    let linux = os.eq_ignore_ascii_case("linux") && matches!(arch, "x86_64" | "aarch64");
    let mac = (os.to_lowercase().as_str(), arch) == HERE && os.eq_ignore_ascii_case("darwin");
    if !linux && !mac {
        return format!("Endeavor can't run on {os} {arch} servers.");
    }
    let platform = if mac { format!("macOS {}", if arch == "aarch64" { "arm64" } else { arch }) } else { format!("{os} {arch}") };
    let build = if linux { BUILD_HELPERS } else { "cargo build" };
    if bundled {
        format!("This copy of Endeavor{NO_HELPER}{platform} servers. {DOWNLOAD_AGAIN} to get one.")
    } else {
        format!("This build of Endeavor{NO_HELPER}{platform} servers. Build it with {build} in Endeavor's source folder, then connect again.")
    }
}

/// What the app can offer for a connection that failed for want of a helper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HelperFix {
    /// Open Endeavor's website to download it again.
    Download,
    /// Copy the command that builds it in the source folder.
    Build(&'static str),
}

impl HelperFix {
    pub fn of(reason: &str) -> Option<HelperFix> {
        if !reason.contains(NO_HELPER) {
            None
        } else if reason.contains(DOWNLOAD_AGAIN) {
            Some(HelperFix::Download)
        } else if reason.contains(BUILD_HELPERS) {
            Some(HelperFix::Build(BUILD_HELPERS))
        } else {
            Some(HelperFix::Build("cargo build"))
        }
    }

    /// The command to paste in a terminal, from the source folder.
    pub fn command(self) -> Option<String> {
        match self {
            HelperFix::Download => None,
            HelperFix::Build(build) => Some(format!("cd '{}' && {build}", env!("CARGO_MANIFEST_DIR"))),
        }
    }
}

/// A tar stream (ustar) of the helper as `endeavor-remote` plus `runtime/`.
fn install_tar(helper: &Path) -> Result<Vec<u8>, String> {
    let helper_bytes = std::fs::read(helper).map_err(|e| format!("{}: {e}", helper.display()))?;
    let mut entries = vec![("endeavor-remote".to_owned(), helper_bytes, true)];
    entries.extend(runtime_files()?);
    let mut tar = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    for (path, contents, executable) in &entries {
        // Parent folders first, each once.
        let mut at = 0;
        while let Some(slash) = path[at..].find('/') {
            let dir = &path[..at + slash];
            if !dirs.iter().any(|d| d == dir) {
                dirs.push(dir.to_owned());
                tar_entry(&mut tar, &format!("{dir}/"), b"", 0o755, b'5')?;
            }
            at += slash + 1;
        }
        tar_entry(&mut tar, path, contents, if *executable { 0o755 } else { 0o644 }, b'0')?;
    }
    tar.extend([0; 1024]);
    Ok(tar)
}

fn tar_entry(tar: &mut Vec<u8>, path: &str, contents: &[u8], mode: u32, kind: u8) -> Result<(), String> {
    let mut header = [0u8; 512];
    let (prefix, name) = match path.len() {
        0..=100 => ("", path),
        _ => {
            let split = path[..path.len().min(156)].rfind('/').filter(|&i| path.len() - i - 1 <= 100);
            let split = split.ok_or_else(|| format!("{path} is too long to install"))?;
            (&path[..split], &path[split + 1..])
        }
    };
    let field = |header: &mut [u8; 512], at: usize, bytes: &[u8]| header[at..at + bytes.len()].copy_from_slice(bytes);
    field(&mut header, 0, name.as_bytes());
    field(&mut header, 100, format!("{mode:07o}\0").as_bytes());
    field(&mut header, 108, b"0000000\0");
    field(&mut header, 116, b"0000000\0");
    field(&mut header, 124, format!("{:011o}\0", contents.len()).as_bytes());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    field(&mut header, 136, format!("{now:011o}\0").as_bytes());
    field(&mut header, 148, b"        ");
    header[156] = kind;
    field(&mut header, 257, b"ustar\0");
    field(&mut header, 263, b"00");
    field(&mut header, 345, prefix.as_bytes());
    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    field(&mut header, 148, format!("{sum:06o}\0 ").as_bytes());
    tar.extend(header);
    tar.extend(contents);
    tar.resize(tar.len().div_ceil(512) * 512, 0);
    Ok(())
}

/// Lets another thread give up on a connect, killing its ssh.
#[derive(Default)]
pub struct Cancel {
    pid: Mutex<Option<u32>>,
    cancelled: AtomicBool,
}

impl Cancel {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(pid) = *self.pid.lock().unwrap() {
            kill_group(pid);
        }
    }

    fn started(&self, pid: u32) -> bool {
        *self.pid.lock().unwrap() = Some(pid);
        !self.cancelled.load(Ordering::SeqCst)
    }
}

/// ssh runs in its own process group, which ends with it: the askpass it may
/// be waiting on, or the local test's shells.
fn kill_group(pid: u32) {
    // SAFETY: plain syscall.
    unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
}

/// Run the bootstrap on `server` and wait for the helper's hello. The runtime
/// starts later, when the channel is asked to (`Channel::start_runtime`).
pub fn connect(server: &Server, transport: &Transport, askpass: Option<&Askpass>, cancel: &Cancel, on: &dyn Fn(Event)) -> Result<(Channel, Hello), String> {
    let version = version()?;
    let mut command = transport.command(&bootstrap_script(&version), askpass)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("Couldn't run ssh: {e}"))?;
    if !cancel.started(child.id()) {
        kill_group(child.id());
    }
    let stderr = Stderr::collect(child.stderr.take().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let host = transport.host();
    let give_up = |mut child: std::process::Child, why: Option<String>| {
        kill_group(child.id());
        let status = child.wait().ok();
        why.unwrap_or_else(|| explain(host, &stderr.finish(), status, cancel.cancelled.load(Ordering::SeqCst)))
    };

    // Login scripts may print things before the script's own line.
    let (os, arch, have) = loop {
        let mut line = String::new();
        match stdout.read_line(&mut line) {
            Ok(0) | Err(_) => return Err(give_up(child, None)),
            Ok(_) => {
                let words: Vec<&str> = line.split_whitespace().collect();
                if let ["ENDEAVOR", os, arch, have @ ("have" | "need")] = words[..] {
                    break (os.to_owned(), arch.to_owned(), have == "have");
                }
                eprintln!("{host}: {}", line.trim_end());
            }
        }
    };
    on(Event::Connected { os: os.clone(), arch: arch.clone() });

    let install = match have {
        true => None,
        false => match helper_for(&os, &arch).and_then(|helper| install_tar(&helper)) {
            Ok(tar) => Some(tar),
            Err(e) => return Err(give_up(child, Some(e))),
        },
    };
    let [flag, value] = server.julia_args();
    let [launcher, state] = server.launcher();
    let mut preamble = format!("{flag}\n{value}\n{launcher}\n{state}\n").into_bytes();
    if let Some(tar) = &install {
        preamble.extend(format!("{}\n", tar.len()).as_bytes());
        preamble.extend(tar);
    }
    if stdin.write_all(&preamble).and_then(|_| stdin.flush()).is_err() {
        return Err(give_up(child, None));
    }
    on(Event::Helper { installed: install.is_some() });

    let channel = Channel::open(child, stdin, stdout);
    let hello = channel.wait_hello(|| explain(host, &stderr.finish(), None, cancel.cancelled.load(Ordering::SeqCst)))?;
    Ok((channel, hello))
}

/// Start the runtime on a connected server's channel (on a cluster, `job` is
/// what to submit); `on` hears Julia being found, the job queueing, and its
/// log. `notice` hears if the runtime goes away later.
pub fn start(channel: &Channel, listener: &Arc<Listener>, job: Option<JobRequest>, on: &dyn Fn(Event), notice: impl FnOnce(Notice) + Send + 'static) -> Result<Runtime, String> {
    let runtime = channel.start_runtime(
        listener,
        job,
        &mut |message| match message {
            ToApp::Progress { line } => on(Event::Progress(line)),
            ToApp::FoundJulia { path, version } => on(Event::FoundJulia { path, version }),
            ToApp::Submitted { job, summary } => on(Event::Submitted { job, summary }),
            ToApp::Queued { state, reason, .. } => on(Event::Queued { state, reason }),
            _ => {}
        },
        notice,
    )?;
    on(Event::Started { node: runtime.node.clone(), reattached: runtime.reattached });
    Ok(runtime)
}

/// Test connection: connect, check the runtime answers through the app's
/// listener, then stop it (or leave it running if it already was). On a
/// cluster, only ask Slurm about itself: starting Julia there means a job.
pub fn test(server: &Server, askpass: Option<&Askpass>, cancel: &Cancel, on: &dyn Fn(Event)) -> Result<(), String> {
    let transport = Transport::for_server(server);
    let (channel, _) = connect(server, &transport, askpass, cancel, on)?;
    if server.cluster.is_some() {
        let reply = channel.files(wire::files::Request::Slurm);
        channel.detach();
        return match reply? {
            wire::files::Reply::Slurm { scheduler } => {
                on(Event::Slurm(scheduler));
                Ok(())
            }
            other => Err(format!("The helper answered {other:?}.")),
        };
    }
    let listener = test_listener()?;
    let runtime = start(&channel, &listener, None, on, |_| {})?;
    let answered = bridge_ping(listener.bridge_port(), &runtime.bridge.token);
    if runtime.reattached {
        channel.detach();
    } else {
        channel.stop();
        channel.detach();
    }
    answered.map_err(|e| format!("Julia started on {}, but it didn't answer through Endeavor's connection ({e}).", runtime.node))?;
    on(Event::Finished { stopped: !runtime.reattached });
    Ok(())
}

/// One listener for every Test connection (they run one at a time).
fn test_listener() -> Result<Arc<Listener>, String> {
    static LISTENER: OnceLock<Arc<Listener>> = OnceLock::new();
    if let Some(listener) = LISTENER.get() {
        return Ok(listener.clone());
    }
    let listener = Listener::start()?;
    Ok(LISTENER.get_or_init(|| listener).clone())
}

/// The bridge's `ping`, through a local port.
fn bridge_ping(port: u16, token: &str) -> Result<(), String> {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    socket.set_read_timeout(Some(Duration::from_secs(20))).map_err(|e| e.to_string())?;
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
    write!(
        socket,
        "POST /call HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|e| e.to_string())?;
    let mut status = String::new();
    BufReader::new(socket).read_line(&mut status).map_err(|e| e.to_string())?;
    match status.split_whitespace().nth(1) {
        Some("200") => Ok(()),
        Some(code) => Err(format!("HTTP {code}")),
        None => Err("no answer".into()),
    }
}

/// ssh's stderr: logged, and its last lines kept to say what went wrong.
struct Stderr {
    lines: Arc<Mutex<Vec<String>>>,
    done: Mutex<mpsc::Receiver<()>>,
}

impl Stderr {
    fn collect(stderr: impl Read + Send + 'static) -> Stderr {
        let lines: Arc<Mutex<Vec<String>>> = Arc::default();
        let (done_tx, done) = mpsc::channel();
        std::thread::spawn({
            let lines = lines.clone();
            move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("ssh: {line}");
                    let mut lines = lines.lock().unwrap();
                    lines.push(line);
                    let excess = lines.len().saturating_sub(40);
                    lines.drain(..excess);
                }
                let _ = done_tx.send(());
            }
        });
        Stderr { lines, done: Mutex::new(done) }
    }

    /// Everything it said, once it's done (or after a short wait).
    fn finish(&self) -> Vec<String> {
        let _ = self.done.lock().unwrap().recv_timeout(Duration::from_secs(2));
        self.lines.lock().unwrap().clone()
    }
}

/// What went wrong, in plain words, from ssh's stderr.
fn explain(host: &str, stderr: &[String], status: Option<ExitStatus>, cancelled: bool) -> String {
    if cancelled {
        return "Cancelled.".into();
    }
    let said = |needle: &str| stderr.iter().any(|l| l.contains(needle));
    let last = stderr.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty());
    if said("Could not resolve hostname") {
        format!("Couldn't find a server called {host}. Check the SSH host.")
    } else if said("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        format!("{host}'s identity (host key) changed since the last connection. If the server was reinstalled, remove its old key with `ssh-keygen -R {host}` in Terminal; otherwise ask its administrator.")
    } else if said("Host key verification failed") {
        format!("{host}'s identity (host key) wasn't confirmed, so Endeavor didn't connect.")
    } else if said("Permission denied") {
        format!("{host} refused the sign-in. Check the user name, and your key or password.")
    } else if said("Connection refused") {
        format!("{host} refused the connection. Check the host name and port, and that it accepts SSH.")
    } else if said("timed out") || said("Operation timed out") {
        format!("{host} didn't answer (the connection timed out). Check the host name, and that you're on a network that can reach it (a VPN, perhaps).")
    } else if said("No route to host") || said("Network is unreachable") {
        format!("Couldn't reach {host} from this network.")
    } else if let Some(line) = last {
        format!("The connection to {host} ended: {line}")
    } else {
        let status = status.map(|s| format!(" ({s})")).unwrap_or_default();
        format!("The connection to {host} ended before Endeavor could start{status}.")
    }
}

/// ssh asking the user something (password, two-factor code, host key), for the app to answer.
pub struct Question {
    pub ask: Ask,
    /// The server's name, for the modal's title.
    pub host: String,
    reply: mpsc::Sender<Option<String>>,
}

impl Question {
    /// `None` cancels.
    pub fn answer(self, text: Option<String>) {
        let _ = self.reply.send(text);
    }
}

/// A private Unix socket that the helper's askpass mode sends ssh's prompts to,
/// for one connect. The folder is 0700 and the socket 0600, so only this user
/// can ask or answer; both go away when this is dropped.
pub struct Askpass {
    dir: PathBuf,
    socket: PathBuf,
    stop: Arc<AtomicBool>,
}

impl Askpass {
    pub fn start(host: String, questions: UnboundedSender<Question>) -> Result<Askpass, String> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("endeavor-askpass-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let socket = dir.join("s");
        let listener = UnixListener::bind(&socket).map_err(|e| format!("{}: {e}", socket.display()))?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        std::thread::spawn({
            let stop = stop.clone();
            move || {
                for connection in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(connection) = connection {
                        let (host, questions) = (host.clone(), questions.clone());
                        std::thread::spawn(move || answer_one(connection, host, &questions));
                    }
                }
            }
        });
        Ok(Askpass { dir, socket, stop })
    }

    fn set_env(&self, command: &mut Command) -> Result<(), String> {
        command
            .env("SSH_ASKPASS", crate::runtime::helper_program()?)
            .env("SSH_ASKPASS_REQUIRE", "force")
            // ssh before 8.4 only uses askpass with a DISPLAY; a Mac app has none.
            .env("DISPLAY", std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into()))
            .env(SOCKET_ENV, &self.socket);
        Ok(())
    }
}

impl Drop for Askpass {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn answer_one(connection: UnixStream, host: String, questions: &UnboundedSender<Question>) {
    let mut line = String::new();
    if BufReader::new(&connection).read_line(&mut line).is_err() {
        return;
    }
    let Ok(ask) = serde_json::from_str::<Ask>(&line) else { return };
    let (reply, answer) = mpsc::channel();
    let text = match questions.unbounded_send(Question { ask, host, reply }) {
        Ok(()) => answer.recv().ok().flatten(),
        Err(_) => None,
    };
    let mut out = serde_json::to_string(&Answer { text }).expect("serializable");
    out.push('\n');
    let _ = (&connection).write_all(out.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use wire::askpass::Kind;
    use wire::files;

    #[test]
    fn the_bootstrap_survives_any_login_shell() {
        let script = bootstrap_script("0.1.0-0123456789abcdef");
        for bad in ['\'', '\\', '!', '\n'] {
            assert!(!script.contains(bad), "{bad:?} in {script}");
        }
        // Through a login shell's parse (sh here), then sh: it runs and stops at the first read.
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!("sh -c '{script}'"))
            .env("HOME", std::env::temp_dir().join("endeavor-no-such-home"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.starts_with("ENDEAVOR ") && stdout.trim_end().ends_with(" need"), "{stdout}");
    }

    #[test]
    fn tar_holds_the_helper_and_runtime() {
        let tmp = std::env::temp_dir().join(format!("endeavor-tar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let helper = tmp.join("fake-helper");
        std::fs::write(&helper, "#!/bin/sh\necho hi\n").unwrap();
        let tar = install_tar(&helper).unwrap();
        assert_eq!(tar.len() % 512, 0);
        let mut untar = Command::new("tar").arg("xf").arg("-").current_dir(&tmp).stdin(Stdio::piped()).spawn().unwrap();
        untar.stdin.take().unwrap().write_all(&tar).unwrap();
        assert!(untar.wait().unwrap().success());
        let unpacked = tmp.join("endeavor-remote");
        assert_eq!(std::fs::read_to_string(&unpacked).unwrap(), "#!/bin/sh\necho hi\n");
        assert_eq!(std::fs::metadata(&unpacked).unwrap().permissions().mode() & 0o777, 0o755);
        for (path, contents, _) in runtime_files().unwrap() {
            assert_eq!(std::fs::read(tmp.join(&path)).unwrap(), contents, "{path}");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn long_paths_use_the_prefix_field() {
        let mut tar = Vec::new();
        let long = format!("{}/{}", "d".repeat(120), "f".repeat(60));
        tar_entry(&mut tar, &long, b"x", 0o644, b'0').unwrap();
        assert_eq!(&tar[..60], "f".repeat(60).as_bytes());
        assert_eq!(&tar[345..465], "d".repeat(120).as_bytes());
        assert!(tar_entry(&mut Vec::new(), &"x".repeat(300), b"", 0o644, b'0').is_err());
    }

    #[test]
    fn explains_ssh_failures_plainly() {
        let say = |line: &str| explain("lab", &[line.to_owned()], None, false);
        assert!(say("ssh: Could not resolve hostname lab: nodename nor servname provided").contains("Couldn't find a server called lab"));
        assert!(say("jc@lab: Permission denied (publickey,password).").contains("refused the sign-in"));
        assert!(say("ssh: connect to host lab port 22: Operation timed out").contains("timed out"));
        assert!(say("Host key verification failed.").contains("identity"));
        assert_eq!(say("bash: line 1: sh: command not found"), "The connection to lab ended: bash: line 1: sh: command not found");
        assert_eq!(explain("lab", &[], None, true), "Cancelled.");
    }

    /// A runtime as the helper sees one: a live pid on this node, and a bridge that answers `ping`.
    struct FakeRuntime {
        process: Arc<Mutex<std::process::Child>>,
    }

    impl FakeRuntime {
        fn start(state_dir: &Path, token: &str) -> FakeRuntime {
            let process = Command::new("sleep").arg("600").process_group(0).spawn().unwrap();
            let bridge = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = bridge.local_addr().unwrap().port();
            let pid = process.id() as i32;
            let process = Arc::new(Mutex::new(process));
            let p = process.clone();
            std::thread::spawn(move || {
                for mut socket in bridge.incoming().map_while(Result::ok) {
                    let p = p.clone();
                    std::thread::spawn(move || {
                        let mut request = Vec::new();
                        let mut buf = [0; 4096];
                        while let Ok(n) = socket.read(&mut buf) {
                            request.extend(&buf[..n]);
                            if n == 0 || String::from_utf8_lossy(&request).contains("\"params\"") {
                                break;
                            }
                        }
                        if String::from_utf8_lossy(&request).contains("endeavor/shutdown") {
                            let mut process = p.lock().unwrap();
                            let _ = process.kill();
                            let _ = process.wait();
                        }
                        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
                    });
                }
            });
            std::fs::create_dir_all(state_dir).unwrap();
            let state = serde_json::json!({
                "launcher": "process", "node": hostname(), "pid": pid, "pluto_port": port, "mcp_port": port,
                "token": token, "pluto_secret": "s3cret",
            });
            std::fs::write(state_dir.join("runtime.json"), state.to_string()).unwrap();
            FakeRuntime { process }
        }

        fn alive(&self) -> bool {
            self.process.lock().unwrap().try_wait().unwrap().is_none()
        }
    }

    impl Drop for FakeRuntime {
        fn drop(&mut self) {
            let mut process = self.process.lock().unwrap();
            let _ = process.kill();
            let _ = process.wait();
        }
    }

    fn hostname() -> String {
        let mut buf = [0u8; 256];
        // SAFETY: gethostname writes at most `len` bytes into `buf`.
        unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }

    fn temp_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("endeavor-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    fn events() -> (Arc<Mutex<Vec<Event>>>, impl Fn(Event)) {
        let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
        let s = seen.clone();
        (seen, move |e| s.lock().unwrap().push(e))
    }

    #[test]
    fn bootstrap_installs_then_reuses_the_helper_and_attaches() {
        let home = temp_home("bootstrap");
        let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let fake = FakeRuntime::start(&home.join(".cache/endeavor/state"), token);
        let transport = Transport::Shell { env: vec![("HOME".into(), home.display().to_string())], ask: None };
        let server = Server { ssh_host: "local-test".into(), ..Default::default() };

        let (seen, on) = events();
        let (channel, hello) = connect(&server, &transport, None, &Cancel::default(), &on).expect("first connect");
        assert_eq!((hello.node.as_str(), hello.home.as_path()), (hostname().as_str(), home.as_path()));
        let installed = home.join(".cache/endeavor").join(version().unwrap());
        assert!(installed.join("endeavor-remote").is_file() && installed.join("runtime/boot.jl").is_file());

        // Its files before any runtime: a folder under the (fake) home.
        std::fs::create_dir_all(home.join("decay-fits")).unwrap();
        std::fs::write(home.join("decay-fits/fit.jl"), "### A Pluto.jl notebook ###\n").unwrap();
        let listed = channel.files(files::Request::List { path: "~".into() }).expect("list");
        assert!(matches!(&listed, files::Reply::List { entries, .. } if entries.iter().any(|e| e.name == "decay-fits" && e.dir)), "{listed:?}");
        let found = channel.files(files::Request::Notebooks { path: "~/decay-fits".into() }).expect("scan");
        assert!(matches!(&found, files::Reply::Notebooks { found } if found.len() == 1), "{found:?}");
        assert!(channel.files(files::Request::Preview { path: "~/nope.jl".into() }).is_err());

        // Started on request, reachable through a listener like the app's.
        let listener = Listener::start().unwrap();
        let runtime = start(&channel, &listener, None, &on, |_| {}).expect("start");
        assert!(runtime.reattached);
        assert_eq!((runtime.bridge.token.as_str(), runtime.node.clone()), (token, hostname()));
        assert!(runtime.pluto_url.ends_with("/?secret=s3cret"));
        let seen = seen.lock().unwrap().clone();
        let uname = if cfg!(target_os = "macos") { "Darwin" } else { "Linux" };
        assert!(matches!(&seen[0], Event::Connected { os, .. } if os == uname), "{seen:?}");
        assert_eq!(seen[1], Event::Helper { installed: true });
        assert!(matches!(&seen[2], Event::Started { reattached: true, .. }), "{seen:?}");
        bridge_ping(listener.bridge_port(), token).expect("ping through the listener");
        channel.detach();
        assert!(fake.alive(), "detaching leaves it running");

        let (seen, on) = events();
        let (channel, _) = connect(&server, &transport, None, &Cancel::default(), &on).expect("second connect");
        assert_eq!(seen.lock().unwrap()[1], Event::Helper { installed: false });
        start(&channel, &listener, None, &on, |_| {}).expect("start again");
        channel.stop();
        assert!(!fake.alive(), "Stop reaches the runtime's bridge");
        // The helper stays connected after a stop.
        assert!(channel.files(files::Request::List { path: "~".into() }).is_ok());
        channel.detach();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_server_without_a_helper_build_is_refused_plainly() {
        let home = temp_home("platform");
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let uname = bin.join("uname");
        std::fs::write(&uname, "#!/bin/sh\n[ \"$1\" = -s ] && echo Plan9 || echo mips\n").unwrap();
        std::fs::set_permissions(&uname, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        let transport = Transport::Shell { env: vec![("HOME".into(), home.display().to_string()), ("PATH".into(), path)], ask: None };
        let (_, on) = events();
        let err = connect(&Server::default(), &transport, None, &Cancel::default(), &on).err().expect("refused");
        assert_eq!(err, "Endeavor can't run on Plan9 mips servers.");
        assert_eq!(HelperFix::of(&err), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_missing_helper_says_how_to_get_one() {
        let linux = missing_helper("Linux", "x86_64", false);
        assert_eq!(linux, "This build of Endeavor has no runtime helper for Linux x86_64 servers. Build it with scripts/build-helpers.sh in Endeavor's source folder, then connect again.");
        assert_eq!(HelperFix::of(&linux), Some(HelperFix::Build("scripts/build-helpers.sh")));
        let app = missing_helper("Linux", "aarch64", true);
        assert_eq!(app, "This copy of Endeavor has no runtime helper for Linux aarch64 servers. Download Endeavor again to get one.");
        assert_eq!(HelperFix::of(&app), Some(HelperFix::Download));
        assert_eq!(HelperFix::Download.command(), None);
        assert_eq!(missing_helper("Linux", "riscv64", false), "Endeavor can't run on Linux riscv64 servers.");
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            let mac = missing_helper("Darwin", "aarch64", false);
            assert_eq!(mac, "This build of Endeavor has no runtime helper for macOS arm64 servers. Build it with cargo build in Endeavor's source folder, then connect again.");
            assert_eq!(HelperFix::of(&mac).and_then(HelperFix::command), Some(format!("cd '{}' && cargo build", env!("CARGO_MANIFEST_DIR"))));
            assert_eq!(missing_helper("Darwin", "x86_64", true), "Endeavor can't run on Darwin x86_64 servers.");
        }
    }

    /// The app's side of askpass, answering from a thread as the modal would.
    fn answering(answer: Option<&'static str>) -> (Askpass, Arc<Mutex<Vec<Ask>>>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Question>();
        let asked: Arc<Mutex<Vec<Ask>>> = Arc::default();
        let a = asked.clone();
        std::thread::spawn(move || {
            while let Some(question) = futures::executor::block_on(futures::StreamExt::next(&mut rx)) {
                a.lock().unwrap().push(question.ask.clone());
                assert_eq!(question.host, "lab-server");
                question.answer(answer.map(String::from));
            }
        });
        (Askpass::start("lab-server".into(), tx).unwrap(), asked)
    }

    #[test]
    fn askpass_round_trip() {
        let (askpass, asked) = answering(Some("hunter2"));
        let helper = crate::runtime::helper_binary().unwrap();
        let run = |prompt: &str, askpass: &Askpass| {
            let mut command = Command::new(&helper);
            askpass.set_env(&mut command).unwrap();
            command.arg(prompt).output().unwrap()
        };
        let mode = std::fs::metadata(&askpass.socket).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let out = run("jc@lab's password: ", &askpass);
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hunter2\n");
        assert_eq!(asked.lock().unwrap()[0], Ask { kind: Kind::Secret, prompt: "jc@lab's password:".into() });

        let (cancelling, _) = answering(None);
        let out = run("Verification code: ", &cancelling);
        assert!(!out.status.success() && out.stdout.is_empty());

        let dir = askpass.dir.clone();
        drop(askpass);
        assert!(!dir.exists(), "the socket's folder goes with it");
    }

    #[test]
    fn a_cancelled_password_prompt_ends_the_connect() {
        let home = temp_home("askpass-cancel");
        let (askpass, asked) = answering(None);
        let transport = Transport::Shell { env: vec![("HOME".into(), home.display().to_string())], ask: Some(FAKE_PASSWORD.into()) };
        let (_, on) = events();
        let err = connect(&Server::default(), &transport, Some(&askpass), &Cancel::default(), &on).err().expect("refused");
        assert!(err.contains("refused the sign-in"), "{err}");
        assert_eq!(asked.lock().unwrap()[0].prompt, "local-test's password:");
        let _ = std::fs::remove_dir_all(&home);
    }
}
