//! Reaching a runtime on a server over SSH (docs/remote-sessions.md), through
//! EndeavorMCP's `client::Session`: one per server, which signs in with the
//! system `ssh`, installs the helper this app carries for the server's
//! platform, starts Julia (or a Slurm job for it) or attaches to the one
//! running, and gets the connection back when it drops. Its loopback port, for
//! Pluto's page and the agent's MCP, stays the same through all of it.
//!
//! ssh's password, two-factor and host-key prompts come to the app through the
//! helper's askpass mode and the library's loopback `Asker`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use endeavor_mcp::client::{self, Asker, Auth, Session};
use futures::channel::mpsc::UnboundedSender;
use wire::askpass::{Ask, Kind};

use crate::hosts::Server;

pub use endeavor_mcp::client::{Cancel, Event};

/// How the bootstrap script reaches `server`: ssh, or in debug builds, for the
/// SSH hosts `local-test`, `local-test-password` and `local-test-hostkey`, This
/// Mac through `sh`.
pub fn transport(server: &Server) -> client::Transport {
    #[cfg(debug_assertions)]
    {
        // While the file ENDEAVOR_TEST_UNREACHABLE names exists, `local-test`
        // fails to connect as a server out of reach does.
        let unreachable = std::env::var("ENDEAVOR_TEST_UNREACHABLE").ok().map(|file| {
            format!("{{ test ! -e '{file}' || {{ echo 'ssh: connect to host {} port 22: Operation timed out' >&2; exit 255; }}; }}", server.name)
        });
        let ask = match server.ssh_host.as_str() {
            "local-test" => Some(unreachable),
            "local-test-password" => Some(Some(FAKE_PASSWORD.to_owned())),
            "local-test-hostkey" => Some(Some(FAKE_HOST_KEY.to_owned())),
            _ => None,
        };
        if let Some(ask) = ask {
            return client::Transport::Shell { env: Vec::new(), ask };
        }
    }
    client::Transport::Ssh { host: server.ssh_host.clone(), port: server.port }
}

#[cfg(debug_assertions)]
const FAKE_PASSWORD: &str =
    r#""$SSH_ASKPASS" "local-test's password: " >/dev/null || { echo "local-test: Permission denied (publickey,password)." >&2; exit 255; }"#;
#[cfg(debug_assertions)]
const FAKE_HOST_KEY: &str = r#"[ "$("$SSH_ASKPASS" "The authenticity of host 'local-test (127.0.0.1)' can't be established.
ED25519 key fingerprint is SHA256:q0dTLnd9XJbDBwu3vYQfZkLr0oTqPqJ8Ne2yTnA3o1E.
This key is not known by any other names.
Are you sure you want to continue connecting (yes/no/[fingerprint])? ")" = yes ] || { echo "Host key verification failed." >&2; exit 255; }"#;

/// The library's record of `server`: the same JSON as the app's `hosts.json`
/// entry. An entry the library can't read is an error, not a blank server.
pub fn client_server(server: &Server) -> Result<client::Server, String> {
    serde_json::to_value(server).and_then(serde_json::from_value).map_err(|e| format!("Couldn't read the settings of {}: {e}", server.name))
}

/// The askpass for `server`'s ssh: each prompt goes to the app's modal as a
/// `Question`, and ssh waits for the answer. It works on every platform.
pub fn asker(server: &Server, questions: UnboundedSender<Question>, sign_in: SignIn) -> Result<(Asker, Auth), String> {
    let host = server.name.clone();
    let source = sign_in.id;
    let asker = Asker::start(sign_in.answering(move |ask, retry| {
        let (reply, answer) = mpsc::channel();
        questions.unbounded_send(Question { ask, host: host.clone(), retry, source, asked: Instant::now(), reply }).map_err(|_| Gone)?;
        // A question taken down unanswered (`Question` dropped) is Gone.
        answer.recv().map_err(|_| Gone)
    }))?;
    let auth = Auth::Env(asker.env(&crate::runtime::helper_program()?));
    Ok((asker, auth))
}

/// A question taken down without the user's answer: its sign-in ended.
pub struct Gone;

/// Why a connect ended while its question waited for the user: the askpass
/// saw the server close the connection (`askpass_watch`). ssh's own words
/// then are those of the try before, such as "Permission denied".
pub fn gave_up(host: &str) -> String {
    format!("{host} stopped waiting for the sign-in.")
}

/// What one connection's sign-in remembers between ssh's questions.
///
/// A Cancel: ssh takes a cancelled password or code as a wrong one and asks
/// again, up to three times; the askpass's exit status doesn't stop it (only a
/// key's passphrase is given up at once). So for a while after a Cancel, ssh's
/// next password questions are cancelled without asking, until the user
/// connects again (`forget`).
///
/// An answer: ssh asks the same question again only when the answer didn't
/// work, and words it the same both times, so the same question soon after an
/// answer is a retry (`Question::retry`).
#[derive(Clone)]
pub struct SignIn {
    /// Tells this sign-in's questions from another's (`Question::source`).
    pub id: u64,
    state: Arc<Mutex<Remembered>>,
}

#[derive(Default)]
struct Remembered {
    declined: Option<Instant>,
    /// The secret question last answered, and when.
    answered: Option<(String, Instant)>,
}

/// How long a Cancel answers ssh's repeats of the question.
const DECLINED_FOR: Duration = Duration::from_secs(60);

/// How soon after an answer the same question means the answer didn't work.
/// ssh asks again within seconds. A new connect asks again only after the
/// user's Reconnect, or, on a line that dropped, after the session's pause
/// between tries (from 1 s): when a try with the right answer fails for
/// another reason (the helper, say), the next try's question within this
/// time wrongly says the answer didn't work. That needs a failure right after
/// signing in, so it is left.
const RETRY_WITHIN: Duration = Duration::from_secs(30);

impl Default for SignIn {
    fn default() -> SignIn {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        SignIn { id: NEXT.fetch_add(1, Ordering::Relaxed), state: Arc::default() }
    }
}

impl SignIn {
    fn answering(self, ask_user: impl Fn(Ask, bool) -> Result<Option<String>, Gone> + Send + Sync + 'static) -> impl Fn(Ask) -> Option<String> + Send + Sync + 'static {
        move |ask| {
            let secret = ask.kind == Kind::Secret;
            let retry = {
                let state = self.state.lock().unwrap();
                if secret && state.declined.is_some_and(|at| at.elapsed() < DECLINED_FOR) {
                    return None;
                }
                secret && state.answered.as_ref().is_some_and(|(prompt, at)| *prompt == ask.prompt && at.elapsed() < RETRY_WITHIN)
            };
            let prompt = ask.prompt.clone();
            let answer = ask_user(ask, retry);
            let mut state = self.state.lock().unwrap();
            match &answer {
                Ok(Some(_)) if secret => state.answered = Some((prompt, Instant::now())),
                Ok(None) if secret => {
                    state.declined = Some(Instant::now());
                    state.answered = None;
                }
                // Taken down because the connect ended: not the user's Cancel.
                Err(Gone) => state.answered = None,
                _ => {}
            }
            answer.ok().flatten()
        }
    }

    /// The user is connecting again: ask them again.
    pub fn forget(&self) {
        *self.state.lock().unwrap() = Remembered::default();
    }
}

/// The listener's words while a server's runtime is away, and its refusal of
/// code runs on a runtime too old to ask the user before a run
/// (`older_runtime::no_run_gate`). Without that refusal set here the library
/// lets such a runtime run code.
fn messages() -> client::Messages {
    client::Messages { no_run_gate: Some(crate::older_runtime::no_run_gate), ..client::Messages::default() }
}

/// A `Session` for `server`, connecting in a thread of its own: the helper is
/// installed without asking, Julia keeps running when the app goes, and its
/// state folder is the one this app has always used there, so a runtime that
/// already runs is found. `on_event` hears its steps and trouble.
pub fn open(server: &Server, auth: Auth, on_event: client::OnEvent) -> Result<Session, String> {
    let mut config = client::Config::new(client_server(server)?, |os: &str, arch: &str| helper_for(os, arch));
    config.transport = transport(server);
    let [_, state] = server.launcher();
    config.state = state;
    config.allow_install = true;
    config.exit_idle = false;
    config.auth = auth;
    config.messages = messages();
    config.on_event = on_event;
    Session::new(config)
}

/// Test connection: connect, check the runtime answers through a listener, then
/// stop it (or leave it running if it already was). On a cluster, only ask
/// Slurm about itself: starting Julia there means a job.
pub fn test(server: &Server, auth: Auth, cancel: &Cancel, on: &dyn Fn(Event)) -> Result<(), String> {
    let [_, state] = server.launcher();
    let helper = |os: &str, arch: &str| helper_for(os, arch);
    let options = client::Options { auth, root: String::new(), state, depot: String::new(), exit_idle: false, allow_install: true, launcher: None, helper: &helper };
    client::test(&client_server(server)?, &transport(server), &options, cancel, on)
}

/// This app's helper and runtime, as the name of their folder on a server:
/// the app version plus a hash of what gets installed, so edits reinstall.
pub fn version() -> Result<String, String> {
    let mut hash = wire::tree::Fnv::default();
    hash.add(endeavor_mcp::embedded::RUNTIME_VERSION.as_bytes());
    for helper in bundled_helpers() {
        hash.add(&std::fs::read(&helper).map_err(|e| format!("{}: {e}", helper.display()))?);
    }
    Ok(format!("{}-{:016x}", env!("CARGO_PKG_VERSION"), hash.0))
}

/// `version()`, worked out once: the build a runtime this app starts
/// reports, which the app compares with a runtime's own (`older_runtime`).
pub fn build() -> Option<&'static str> {
    static BUILD: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| version().map_err(|e| eprintln!("Couldn't work out this build's version: {e}")).ok()).as_deref()
}

/// Folders holding helpers for servers, one folder per platform: inside
/// Endeavor.app (the Linux ones and this Mac's), or target/helpers in the
/// source tree (the Linux ones, from scripts/helpers.sh).
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
        .map(|platform| platform.path().join("endeavor"))
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
    if let Some(helper) = helper_dirs().iter().map(|d| d.join(&platform).join("endeavor")).find(|p| p.is_file()) {
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

const HELPERS_SCRIPT: &str = "scripts/helpers.sh";
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
    let fix = if linux { format!("Get it with {HELPERS_SCRIPT}") } else { "Build it with cargo build".into() };
    if bundled {
        format!("This copy of Endeavor{NO_HELPER}{platform} servers. {DOWNLOAD_AGAIN} to get one.")
    } else {
        format!("This build of Endeavor{NO_HELPER}{platform} servers. {fix} in Endeavor's source folder, then connect again.")
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
        } else if reason.contains(HELPERS_SCRIPT) {
            Some(HelperFix::Build(HELPERS_SCRIPT))
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

/// ssh asking the user something (password, two-factor code, host key), for the app to answer.
pub struct Question {
    pub ask: Ask,
    /// The server's name, for the modal's title.
    pub host: String,
    /// ssh asks again: the last answer didn't work.
    pub retry: bool,
    /// The `SignIn` it belongs to.
    pub source: u64,
    asked: Instant,
    reply: mpsc::Sender<Option<String>>,
}

impl Question {
    /// `None` cancels. A question dropped unanswered was taken down (`Gone`).
    pub fn answer(self, text: Option<String>) {
        let _ = self.reply.send(text);
    }

    /// Asked by sign-in `source`, and before `before` if that is given.
    pub fn belongs_to(&self, source: u64, before: Option<Instant>) -> bool {
        self.source == source && before.is_none_or(|before| self.asked < before)
    }

    /// The line over the question on a retry.
    pub fn retry_line(&self) -> Option<&'static str> {
        if !self.retry {
            return None;
        }
        let prompt = self.ask.prompt.to_lowercase();
        Some(if prompt.contains("passphrase") {
            "That passphrase didn't work. Try again."
        } else if prompt.contains("password") {
            "That password didn't work. Try again."
        } else {
            "That didn't work. Try again."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_record_reads_the_same_to_the_library() {
        let server = Server { id: "lab".into(), name: "Lab".into(), ssh_host: "jc@lab".into(), port: Some(2222), julia: Some("module load julia".into()), r: Some("module load R".into()), ..Server::default() };
        let theirs = client_server(&server).unwrap();
        assert_eq!((theirs.id.as_str(), theirs.name.as_str(), theirs.ssh_host.as_str(), theirs.port, theirs.julia.as_deref()), ("lab", "Lab", "jc@lab", Some(2222), Some("module load julia")));
        assert_eq!(theirs.julia_args(), server.julia_args(), "the helper gets the same Julia");
        assert_eq!(theirs.r_args(), ["--r-shell", "module load R"], "and the same R");
    }

    fn password() -> Ask {
        Ask { kind: Kind::Secret, prompt: "jc@lab's password:".into() }
    }

    #[test]
    fn a_cancelled_password_isnt_asked_again_when_ssh_retries() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let sign_in = SignIn::default();
        let answer = sign_in.clone().answering({
            let asked = asked.clone();
            move |ask: Ask, _| {
                asked.lock().unwrap().push(ask.prompt.clone());
                Ok(if ask.kind == Kind::YesNo { Some("yes".into()) } else { None })
            }
        });
        assert_eq!(answer(password()), None, "the user's Cancel");
        assert_eq!(answer(password()), None);
        assert_eq!(answer(password()), None);
        assert_eq!(asked.lock().unwrap().len(), 1, "ssh's two retries aren't shown");
        assert_eq!(answer(Ask { kind: Kind::YesNo, prompt: "Are you sure (yes/no)?".into() }), Some("yes".into()), "other questions still are");
        sign_in.forget();
        assert_eq!(answer(password()), None);
        assert_eq!(asked.lock().unwrap().len(), 3, "a new connect asks again");
    }

    #[test]
    fn the_same_question_after_an_answer_says_the_answer_didnt_work() {
        let retries = Arc::new(Mutex::new(Vec::new()));
        let sign_in = SignIn::default();
        let answer = sign_in.clone().answering({
            let retries = retries.clone();
            move |_, retry| {
                retries.lock().unwrap().push(retry);
                Ok(Some("hunter2".into()))
            }
        });
        answer(password());
        answer(password());
        answer(Ask { kind: Kind::Secret, prompt: "Verification code:".into() });
        answer(password());
        sign_in.forget();
        answer(password());
        assert_eq!(*retries.lock().unwrap(), [false, true, false, false, false], "only the same question again, and not after a new connect");
    }

    #[test]
    fn a_question_taken_down_isnt_a_cancel() {
        let shown = Arc::new(Mutex::new(0));
        let answer = SignIn::default().answering({
            let shown = shown.clone();
            move |_, retry| {
                let mut shown = shown.lock().unwrap();
                *shown += 1;
                assert!(!retry);
                if *shown == 1 { Err(Gone) } else { Ok(Some("hunter2".into())) }
            }
        });
        assert_eq!(answer(password()), None, "ssh gets no answer");
        assert_eq!(answer(password()), Some("hunter2".into()), "the next connect's question is shown, not cancelled");
        assert_eq!(*shown.lock().unwrap(), 2);
    }

    #[test]
    fn a_try_that_ended_takes_down_only_the_questions_it_asked() {
        let (reply, _) = mpsc::channel();
        let asked = Instant::now();
        let question = Question { ask: password(), host: "Lab".into(), retry: false, source: 7, asked, reply };
        assert!(question.belongs_to(7, None), "the sign-in ended");
        assert!(question.belongs_to(7, Some(asked + Duration::from_millis(1))), "asked before its try ended");
        assert!(!question.belongs_to(7, Some(asked)), "the next try's question stays");
        assert!(!question.belongs_to(8, None), "another sign-in's stays");
    }

    #[test]
    fn a_retry_says_what_didnt_work() {
        let (reply, _) = mpsc::channel();
        let question = |prompt: &str, retry| Question { ask: Ask { kind: Kind::Secret, prompt: prompt.into() }, host: "Lab".into(), retry, source: 0, asked: Instant::now(), reply: reply.clone() };
        assert_eq!(question("jc@lab's password:", false).retry_line(), None);
        assert_eq!(question("jc@lab's password:", true).retry_line(), Some("That password didn't work. Try again."));
        assert_eq!(question("Enter passphrase for key '/Users/jc/.ssh/id_ed25519':", true).retry_line(), Some("That passphrase didn't work. Try again."));
        assert_eq!(question("Verification code:", true).retry_line(), Some("That didn't work. Try again."));
    }

    #[test]
    fn the_listener_refuses_code_runs_on_a_runtime_too_old_to_ask() {
        // The library leaves the refusal off unless a client sets it (EndeavorMCP #43).
        let words = messages().no_run_gate.expect("the app sets the refusal");
        assert!(words("lab").starts_with("ArgumentError: older_runtime::Julia on lab was started by a version of Endeavor too old"), "{}", words("lab"));
        assert!(words("lab").contains("tell the user to restart Julia"), "{}", words("lab"));
    }

    #[test]
    fn a_missing_helper_says_how_to_get_one() {
        let linux = missing_helper("Linux", "x86_64", false);
        assert_eq!(linux, "This build of Endeavor has no runtime helper for Linux x86_64 servers. Get it with scripts/helpers.sh in Endeavor's source folder, then connect again.");
        assert_eq!(HelperFix::of(&linux), Some(HelperFix::Build("scripts/helpers.sh")));
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
}
