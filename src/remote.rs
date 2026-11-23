use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use endeavor_mcp::client::{self, Asker, Auth, Session};
use futures::channel::mpsc::UnboundedSender;
use wire::askpass::{Ask, Kind};

use crate::hosts::Server;

pub use endeavor_mcp::client::{Cancel, Event};

pub fn transport(server: &Server) -> client::Transport {
    #[cfg(debug_assertions)]
    {
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

pub fn client_server(server: &Server) -> Result<client::Server, String> {
    serde_json::to_value(server).and_then(serde_json::from_value).map_err(|e| format!("Couldn't read the settings of {}: {e}", server.name))
}

pub fn asker(server: &Server, questions: UnboundedSender<Question>, sign_in: SignIn) -> Result<(Asker, Auth), String> {
    let host = server.name.clone();
    let source = sign_in.id;
    let asker = Asker::start(sign_in.answering(move |ask, retry| {
        let (reply, answer) = mpsc::channel();
        questions.unbounded_send(Question { ask, host: host.clone(), retry, source, asked: Instant::now(), reply }).map_err(|_| Gone)?;
        answer.recv().map_err(|_| Gone)
    }))?;
    let auth = Auth::Env(asker.env(&crate::runtime::helper_program()?));
    Ok((asker, auth))
}

pub struct Gone;

pub fn gave_up(host: &str) -> String {
    format!("{host} stopped waiting for the sign-in.")
}

#[derive(Clone)]
pub struct SignIn {
    pub id: u64,
    state: Arc<Mutex<Remembered>>,
}

#[derive(Default)]
struct Remembered {
    declined: Option<Instant>,
    answered: Option<(String, Instant)>,
}

const DECLINED_FOR: Duration = Duration::from_secs(60);

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
                Err(Gone) => state.answered = None,
                _ => {}
            }
            answer.ok().flatten()
        }
    }

    pub fn forget(&self) {
        *self.state.lock().unwrap() = Remembered::default();
    }
}

fn messages() -> client::Messages {
    client::Messages { no_run_gate: Some(crate::older_runtime::no_run_gate), ..client::Messages::default() }
}

pub fn open(server: &Server, auth: Auth, socket: Option<std::net::TcpListener>, on_event: client::OnEvent) -> Result<Session, String> {
    let mut config = client::Config::new(client_server(server)?, |os: &str, arch: &str| helper_for(os, arch));
    config.transport = transport(server);
    let [_, state] = server.launcher();
    config.state = state;
    config.allow_install = true;
    config.exit_idle = false;
    config.julia_when_needed = true;
    config.auth = auth;
    config.messages = messages();
    config.on_event = on_event;
    config.socket = socket;
    Session::new(config)
}

pub fn test(server: &Server, auth: Auth, cancel: &Cancel, on: &dyn Fn(Event)) -> Result<(), String> {
    let [_, state] = server.launcher();
    let helper = |os: &str, arch: &str| helper_for(os, arch);
    let options = client::Options { auth, root: String::new(), state, depot: String::new(), exit_idle: false, julia_when_needed: false, allow_install: true, launcher: None, helper: &helper };
    client::test(&client_server(server)?, &transport(server), &options, cancel, on)
}

pub fn version() -> Result<String, String> {
    let mut hash = wire::tree::Fnv::default();
    hash.add(endeavor_mcp::embedded::RUNTIME_VERSION.as_bytes());
    for helper in bundled_helpers() {
        hash.add(&std::fs::read(&helper).map_err(|e| format!("{}: {e}", helper.display()))?);
    }
    Ok(format!("{}-{:016x}", env!("CARGO_PKG_VERSION"), hash.0))
}

pub fn build() -> Option<&'static str> {
    static BUILD: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| version().map_err(|e| eprintln!("Couldn't work out this build's version: {e}")).ok()).as_deref()
}

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
    match endeavor_mcp::embedded::RELEASE_KEY {
        Some(key) if supported(os, arch) => {
            let cache = crate::install::app_dir()?.join("helpers");
            let removed = removed_build(os, arch);
            endeavor_mcp::release_helper(&os.to_lowercase(), arch, key, &cache, &|_, _, _| removed.clone()).map_err(|e| if e == removed { e } else { download_failed(os, arch, &e) })
        }
        _ => Err(missing_helper(os, arch, crate::install::bundled())),
    }
}

fn supported(os: &str, arch: &str) -> bool {
    (os.eq_ignore_ascii_case("linux") || os.eq_ignore_ascii_case("darwin")) && matches!(arch, "x86_64" | "aarch64")
}

fn is_this_mac(os: &str, arch: &str) -> bool {
    (os.to_lowercase().as_str(), arch) == HERE && os.eq_ignore_ascii_case("darwin")
}

fn platform_words(os: &str, arch: &str) -> String {
    if os.eq_ignore_ascii_case("darwin") { format!("macOS {}", if arch == "aarch64" { "arm64" } else { arch }) } else { format!("{os} {arch}") }
}

fn download_failed(os: &str, arch: &str, why: &str) -> String {
    format!("Couldn't download the runtime helper for {} servers, which Endeavor needs the first time it connects to one. Check that this computer is online, then connect again. ({why})", platform_words(os, arch))
}

fn removed_build(os: &str, arch: &str) -> String {
    format!("This copy of Endeavor{TOO_OLD}{} servers. Restart Endeavor to finish updating, or {DOWNLOAD_AGAIN} to get a newer one.", platform_words(os, arch))
}

const HERE: (&str, &str) = (if cfg!(target_os = "macos") { "darwin" } else { std::env::consts::OS }, std::env::consts::ARCH);

const HELPERS_SCRIPT: &str = "scripts/helpers.sh";
const NO_HELPER: &str = " has no runtime helper for ";
const DOWNLOAD_AGAIN: &str = "Download Endeavor again";
const TOO_OLD: &str = " is too old to download the runtime helper for ";

fn missing_helper(os: &str, arch: &str, bundled: bool) -> String {
    if !supported(os, arch) {
        return format!("Endeavor can't run on {os} {arch} servers.");
    }
    let platform = platform_words(os, arch);
    if bundled && cfg!(windows) {
        return format!("This copy of Endeavor can't get the runtime helper for {platform} servers: it was built without the key for downloading one.");
    }
    if bundled {
        return format!("This copy of Endeavor{NO_HELPER}{platform} servers. {DOWNLOAD_AGAIN} to get one.");
    }
    let fix = if is_this_mac(os, arch) {
        "Build it with cargo build"
    } else if os.eq_ignore_ascii_case("linux") {
        &format!("Get it with {HELPERS_SCRIPT}")
    } else {
        return format!("This build of Endeavor{NO_HELPER}{platform} servers, and can't download one: only Endeavor's own builds can.");
    };
    format!("This build of Endeavor{NO_HELPER}{platform} servers. {fix} in Endeavor's source folder, then connect again.")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HelperFix {
    Download,
    Build(&'static str),
}

impl HelperFix {
    pub fn of(reason: &str) -> Option<HelperFix> {
        if reason.contains(TOO_OLD) {
            Some(HelperFix::Download)
        } else if !reason.contains(NO_HELPER) {
            None
        } else if reason.contains(DOWNLOAD_AGAIN) {
            Some(HelperFix::Download)
        } else if reason.contains(HELPERS_SCRIPT) {
            Some(HelperFix::Build(HELPERS_SCRIPT))
        } else if reason.contains("cargo build") {
            Some(HelperFix::Build("cargo build"))
        } else {
            None
        }
    }

    pub fn command(self) -> Option<String> {
        match self {
            HelperFix::Download => None,
            HelperFix::Build(build) => Some(format!("cd '{}' && {build}", env!("CARGO_MANIFEST_DIR"))),
        }
    }
}

pub struct Question {
    pub ask: Ask,
    pub host: String,
    pub retry: bool,
    pub source: u64,
    asked: Instant,
    reply: mpsc::Sender<Option<String>>,
}

impl Question {
    pub fn answer(self, text: Option<String>) {
        let _ = self.reply.send(text);
    }

    pub fn belongs_to(&self, source: u64, before: Option<Instant>) -> bool {
        self.source == source && before.is_none_or(|before| self.asked < before)
    }

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
        let words = messages().no_run_gate.expect("the app sets the refusal");
        assert!(words("lab").starts_with("ArgumentError: older_runtime::Julia on lab was started by a version of Endeavor too old"), "{}", words("lab"));
        assert!(words("lab").contains("call request_restart"), "{}", words("lab"));
    }

    #[test]
    fn a_missing_helper_says_how_to_get_one() {
        let linux = missing_helper("Linux", "x86_64", false);
        assert_eq!(linux, "This build of Endeavor has no runtime helper for Linux x86_64 servers. Get it with scripts/helpers.sh in Endeavor's source folder, then connect again.");
        assert_eq!(HelperFix::of(&linux), Some(HelperFix::Build("scripts/helpers.sh")));
        let app = missing_helper("Linux", "aarch64", true);
        if cfg!(windows) {
            assert_eq!(app, "This copy of Endeavor can't get the runtime helper for Linux aarch64 servers: it was built without the key for downloading one.");
            assert_eq!(HelperFix::of(&app), None);
        } else {
            assert_eq!(app, "This copy of Endeavor has no runtime helper for Linux aarch64 servers. Download Endeavor again to get one.");
            assert_eq!(HelperFix::of(&app), Some(HelperFix::Download));
        }
        assert_eq!(HelperFix::Download.command(), None);
        assert_eq!(missing_helper("Linux", "riscv64", false), "Endeavor can't run on Linux riscv64 servers.");
        assert_eq!(missing_helper("FreeBSD", "x86_64", true), "Endeavor can't run on FreeBSD x86_64 servers.");
        let other = if HERE == ("darwin", "aarch64") { "x86_64" } else { "aarch64" };
        let mac = missing_helper("Darwin", other, false);
        assert_eq!(mac, format!("This build of Endeavor has no runtime helper for {} servers, and can't download one: only Endeavor's own builds can.", platform_words("Darwin", other)));
        assert_eq!(HelperFix::of(&mac), None);
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            let mac = missing_helper("Darwin", "aarch64", false);
            assert_eq!(mac, "This build of Endeavor has no runtime helper for macOS arm64 servers. Build it with cargo build in Endeavor's source folder, then connect again.");
            assert_eq!(HelperFix::of(&mac).and_then(HelperFix::command), Some(format!("cd '{}' && cargo build", env!("CARGO_MANIFEST_DIR"))));
        }
    }

    #[test]
    fn an_old_build_says_to_update() {
        let old = removed_build("Linux", "x86_64");
        assert_eq!(old, "This copy of Endeavor is too old to download the runtime helper for Linux x86_64 servers. Restart Endeavor to finish updating, or Download Endeavor again to get a newer one.");
        assert_eq!(HelperFix::of(&old), Some(HelperFix::Download));
        let offline = download_failed("Darwin", "x86_64", "Couldn't download https://example (curl: (6) Could not resolve host).");
        assert_eq!(offline, "Couldn't download the runtime helper for macOS x86_64 servers, which Endeavor needs the first time it connects to one. Check that this computer is online, then connect again. (Couldn't download https://example (curl: (6) Could not resolve host).)");
        assert_eq!(HelperFix::of(&offline), None);
        assert_eq!(removed_build("Darwin", "x86_64"), "This copy of Endeavor is too old to download the runtime helper for macOS x86_64 servers. Restart Endeavor to finish updating, or Download Endeavor again to get a newer one.");
    }
}

