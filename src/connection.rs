//! The app's line to each host's runtime: This Mac's connects and starts at
//! launch; a server's connects when it's picked in the Where chip (its files
//! can be browsed then) and starts Julia when a session on it needs one. Each
//! host keeps its own local listener, so its sessions' MCP URL survives
//! reconnects and restarts.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use endeavor_mcp::client::{self, SessionEvent, State as LineState, Status as LineStatus};
use futures::StreamExt;
use gpui::prelude::FluentBuilder as _;
use gpui::*;

use wire::files::{Reply, Request, RuntimeState};
use wire::slurm::JobRequest;

use crate::host_list::HostState;
use crate::hosts::{HostId, Server};
use crate::pluto::{self, Bridge};
use crate::remote::{self, Event};
use crate::runtime::{self, Channel, Hello, Listener, Notice, Runtime};
use crate::session::{Session, Stopped};
use crate::splash::{Progress, Step};
use crate::turtle::{self, Pose};
use crate::{Workspace, theme};
use crate::theme::FocusRing as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    /// Connected, with no runtime yet: its files can be browsed.
    Browsing,
    Starting,
    Ready,
    /// Julia exited or couldn't start; still connected, so Start tries again.
    Died(String),
    /// Another connection took the runtime over.
    Replaced,
    /// Couldn't connect, or the connection closed.
    Failed(String),
}

/// One host's connection, and what the app follows of its runtime.
pub struct Connection {
    pub status: Status,
    /// Which connect this is: a thread still reporting on an older one is ignored.
    id: u64,
    /// This Mac's helper. A server's is its `Line`'s.
    pub channel: Option<Arc<Channel>>,
    /// A server's line is connected (its helper is up).
    linked: bool,
    pub hello: Option<Hello>,
    pub runtime: Option<Runtime>,
    /// Start Julia once connected (a session is waiting for it).
    start_when_connected: bool,
    /// On a cluster: the job the next start submits (the resources of the
    /// session that asked for it); else the cluster's defaults.
    pub job_request: Option<JobRequest>,
    /// Cancel was pressed while Julia started (a job waited in the queue).
    cancelling: bool,
    /// Stop Julia once connected (Stop in Settings' host list).
    stop_when_connected: bool,
    /// Start Julia again once the Stop under way ends (`restart_host`).
    start_after_stop: bool,
    /// Bumped by each check and stop, so an older check's answer is dropped.
    check: u64,
    /// A Stop is under way.
    pub stopping: bool,
    /// What the last check found running there, while connected without a
    /// runtime; Err if the check failed.
    pub found: Option<Result<RuntimeState, String>>,
    /// On a cluster, the job Julia is starting in.
    pub job: Option<String>,
    pub steps: Steps,
    /// Open notebooks (id, path) as last seen, to reopen after a restart.
    pub last_notebooks: Vec<(String, String)>,
    /// The notebooks the user had let run when they restarted or repaired
    /// Julia, with each file's modification time then: they reopen running if
    /// the file is unchanged. After a crash, Restart Julia fills it (with no
    /// time: Julia's gone, so it can't be read) for the notebooks that were
    /// running; any other start leaves it empty, so they reopen in safe preview.
    pub resume: Vec<(String, Option<f64>)>,
    /// Connects and starts that failed in a row, across reconnects.
    pub failures: u32,
    /// Why Restart Julia's stop failed, said once the start that follows it is ready.
    pub not_restarted: Option<String>,
    /// The runtime's notebook list as last pushed (`list_notebooks` shape).
    pub notebooks: serde_json::Value,
    /// Per-notebook cell states as last pushed ({notebook_id: [state]}).
    pub cells: serde_json::Value,
    /// Paths the runtime listed as stopped for being idle, as last pushed.
    pub idle_stopped: HashSet<String>,
    /// Bumped when the runtime starts or goes, so an old runtime's event reader stops.
    watching: Arc<AtomicU64>,
    /// A server's connection dropped by itself (not a Stop, disconnect or
    /// remove) and hasn't come back, or a session waiting to open there
    /// couldn't reach it: Endeavor tries again, its sessions' messages wait,
    /// and it shows as "Can't reach". None for any other connect that never
    /// got through (a bad host name while adding it, a wrong password): that
    /// stays "Not connected", with Reconnect.
    pub lost: Option<Lost>,
    /// Julia exited by itself (not a Stop, a restart or a quit), and hasn't
    /// been started since.
    pub crashed: bool,
    /// How the app can use its runtime (`older_runtime`); none until a
    /// runtime is ready.
    pub older: Option<crate::older_runtime::Version>,
}

/// A dropped server connection.
#[derive(Clone)]
pub struct Lost {
    /// The dropped runtime's Pluto address (`http://host:port/`), if its
    /// notebook was showing: that page stays, read-only.
    page: Option<String>,
    /// Its status when it dropped: the reconnect starts Julia again if it ran
    /// or was starting, and shows it stopped if it was.
    was: Status,
}

impl Lost {
    fn had_julia(&self) -> bool {
        matches!(self.was, Status::Starting | Status::Ready)
    }
}

/// The starting pane's step log: what's done, what's under way (since when),
/// and the latest line of Julia's output.
#[derive(Clone, Debug)]
pub struct Steps {
    pub done: Vec<String>,
    pub current: String,
    pub since: Instant,
    pub detail: Option<String>,
    /// Julia was found or reattached to, so only starting it is left.
    found_julia: bool,
}

impl Steps {
    fn new(current: impl Into<String>) -> Steps {
        Steps { done: Vec::new(), current: current.into(), since: Instant::now(), detail: None, found_julia: false }
    }

    fn advance(&mut self, done: impl Into<String>, next: impl Into<String>) {
        self.done.push(done.into());
        self.now(next);
    }

    fn now(&mut self, current: impl Into<String>) {
        let current = current.into();
        if current != self.current {
            self.current = current;
            self.since = Instant::now();
            self.detail = None;
        }
    }

    /// What's left after the current step, shown faint.
    pub fn pending(&self) -> Vec<&'static str> {
        let mut left = Vec::new();
        if !self.current.starts_with("Starting Julia") {
            left.push("Start Julia");
        }
        left.push("Open notebook");
        left
    }

    /// A line of Julia's download or boot log.
    fn log(&mut self, line: &str) {
        let text = line.trim_start_matches(['┌', '│', '└', ' ']).trim();
        if text.is_empty() {
            return;
        }
        if text.starts_with("Downloading Julia") || text.starts_with("Checking Julia") || text.starts_with("Unpacking Julia") {
            self.now(text.trim_end_matches(|c: char| c.is_ascii_digit() || c == '%' || c == ' ').trim_end_matches('…').to_owned() + "…");
            self.detail = percent(text).map(str::to_owned);
            return;
        }
        if self.found_julia && ["Precompiling", "Installing", "Resolving", "Updating", "Downloaded", "Instantiating"].iter().any(|w| text.contains(w)) {
            self.now("Installing packages");
        }
        self.detail = Some(text.to_owned());
    }
}

/// The last word of a downloading line: "Downloading Julia 1.12.6… 42%" -> "42%".
fn percent(line: &str) -> Option<&str> {
    line.rsplit(' ').next().filter(|w| w.ends_with('%'))
}

/// How a connect or start went, from its thread.
enum Update {
    Event(Event),
    /// This Mac's setup progress.
    Local(Progress),
    /// The helper is up: This Mac's channel (a server's goes through its `Line`), and its hello.
    Connected(Result<(Option<Arc<Channel>>, Hello), String>),
    Started(Result<Runtime, String>),
    Notice(Notice),
}

impl Connection {
    fn new(id: u64, status: Status, steps: Steps) -> Connection {
        Connection {
            status,
            id,
            channel: None,
            linked: false,
            hello: None,
            runtime: None,
            start_when_connected: false,
            job_request: None,
            cancelling: false,
            stop_when_connected: false,
            start_after_stop: false,
            check: 0,
            stopping: false,
            found: None,
            job: None,
            steps,
            last_notebooks: Vec::new(),
            resume: Vec::new(),
            failures: 0,
            not_restarted: None,
            notebooks: serde_json::Value::Null,
            cells: serde_json::Value::Null,
            idle_stopped: HashSet::new(),
            watching: Arc::default(),
            older: None,
            lost: None,
            crashed: false,
        }
    }

    pub fn bridge(&self) -> Option<Bridge> {
        self.runtime.as_ref().map(|r| r.bridge.clone())
    }

    /// The helper is up: This Mac's channel, or a server's line.
    fn connected(&self) -> bool {
        self.channel.is_some() || self.linked
    }

    /// The helper went: there is nothing to talk to until the next connect.
    fn unlink(&mut self) {
        self.channel = None;
        self.linked = false;
    }

    /// Dropped, and between tries to reconnect.
    pub fn waiting_to_reconnect(&self) -> bool {
        self.lost.is_some() && matches!(self.status, Status::Failed(_))
    }

    /// A connect that didn't get through because the server is out of reach,
    /// while it was already lost or a session waits to open there: it stays
    /// lost, to be tried again, and starts Julia once back if this connect was
    /// to. False, and nothing changes, for any other failure.
    fn keep_trying(&mut self, error: &str, session_waits: bool) -> bool {
        if !crate::offline::unreachable(error) || (self.lost.is_none() && !session_waits) {
            return false;
        }
        let was = if self.start_when_connected { Status::Starting } else { Status::Browsing };
        self.lost.get_or_insert(Lost { page: None, was });
        self.status = Status::Failed(error.to_owned());
        true
    }

    /// Stop following the runtime that's going; `last_notebooks` stays for the reopen.
    /// Returns it, for `Workspace::close_page`.
    #[must_use]
    fn forget_runtime(&mut self) -> Option<Runtime> {
        self.watching.fetch_add(1, Ordering::SeqCst);
        self.notebooks = serde_json::Value::Null;
        self.cells = serde_json::Value::Null;
        self.runtime.take()
    }
}

impl Workspace {
    pub fn connection(&self, host: &HostId) -> Option<&Connection> {
        self.connections.get(host)
    }

    pub fn status(&self, host: &HostId) -> Option<&Status> {
        self.connections.get(host).map(|c| &c.status)
    }

    /// The bridge of `host`'s runtime, while it's ready.
    pub fn bridge(&self, host: &HostId) -> Option<Bridge> {
        self.connections.get(host).and_then(Connection::bridge)
    }

    /// A host whose Julia stopped by itself and hasn't started since, by name
    /// ("This Mac", a server's), for the sidebar's status line.
    pub fn crashed_host(&self) -> Option<String> {
        let mut crashed: Vec<&HostId> = self.connections.iter().filter(|(_, c)| c.crashed && matches!(c.status, Status::Died(_))).map(|(h, _)| h).collect();
        crashed.sort_by_key(|h| **h != HostId::ThisMac);
        crashed.first().map(|host| match host {
            HostId::ThisMac => crate::platform::this_computer!().to_string(),
            host => self.hosts.name(host),
        })
    }

    /// The runtime of the session `key`'s host.
    pub fn session_bridge(&self, key: u64) -> Option<Bridge> {
        let session = self.sessions.iter().find(|s| s.key == key)?;
        self.bridge(&session.place.host)
    }

    /// Take `gone`'s notebook page out of the web view. Left there, it keeps
    /// reconnecting to its port, where the host's next runtime listens, which
    /// doesn't have the notebook the page shows.
    fn close_page(&mut self, gone: Option<Runtime>, cx: &mut Context<Self>) {
        let Some((origin, _)) = gone.as_ref().and_then(|r| r.page_url.split_once('?')) else { return };
        self.blank_page(origin, cx);
    }

    /// Take down the page if it's from `origin` (a runtime that went away).
    fn blank_page(&mut self, origin: &str, cx: &mut Context<Self>) {
        let shown = crate::webcontent::url(self.webview.read(cx).raw());
        if !origin.is_empty() && shown.starts_with(origin) {
            self.webview.update(cx, |w, _| w.load_url("about:blank"));
        }
    }

    /// The web view shows a page of `host`'s runtime.
    fn shows_page_of(&self, host: &HostId, cx: &App) -> bool {
        let Some((origin, _)) = self.connections.get(host).and_then(|c| c.runtime.as_ref()).and_then(|r| r.page_url.split_once('?')) else { return false };
        crate::webcontent::url(self.webview.read(cx).raw()).starts_with(origin)
    }

    /// What answers for `host`'s runtime while it's connected: This Mac's helper, or a server's line.
    pub fn helper(&self, host: &HostId) -> Option<Helper> {
        let connection = self.connections.get(host)?;
        if let Some(channel) = &connection.channel {
            return Some(Helper::Local(channel.clone()));
        }
        let line = self.lines.get(host).filter(|_| connection.linked)?;
        Some(Helper::Line(line.session.clone()))
    }

    fn listener(&mut self, host: &HostId) -> Result<Arc<Listener>, String> {
        if let Some(listener) = self.listeners.get(host) {
            return Ok(listener.clone());
        }
        let listener = Listener::start(&self.hosts.name(host))?;
        self.listeners.insert(host.clone(), listener.clone());
        Ok(listener)
    }

    /// Connect to `host` unless it's connected or connecting; `start` also
    /// starts Julia once it is.
    pub fn connect_host(&mut self, host: &HostId, start: bool, cx: &mut Context<Self>) {
        match self.connections.get_mut(host).map(|c| (c.status.clone(), c)) {
            Some((Status::Connecting, connection)) => {
                connection.start_when_connected |= start;
                return;
            }
            Some((Status::Failed(_) | Status::Replaced, _)) | None => {}
            Some(_) => {
                if start {
                    self.start_host(host, cx);
                }
                return;
            }
        }
        let id = next_connect_id();
        let name = self.hosts.name(host);
        let first = match host {
            HostId::ThisMac => Steps::new("Starting Julia"),
            HostId::Server(_) => Steps::new(format!("Connecting to {name}")),
        };
        let mut connection = Connection::new(id, Status::Connecting, first);
        connection.start_when_connected = start;
        // A reconnect keeps what the runtime last had open, to reopen it.
        if let Some(old) = self.connections.remove(host) {
            connection.last_notebooks = old.last_notebooks;
            connection.resume = old.resume;
            connection.failures = old.failures;
            connection.lost = old.lost;
        }
        self.connections.insert(host.clone(), connection);
        match host {
            HostId::ThisMac => {
                let (tx, rx) = futures::channel::mpsc::unbounded::<Update>();
                let keep_running = self.settings.keep_running;
                // The helper runs this Julia for as long as it lives (runtime::connect).
                self.settings_checks.local_julia = Some(self.settings.julia.clone());
                std::thread::spawn(move || {
                    let progress = |p: Progress| drop(tx.unbounded_send(Update::Local(p)));
                    let result = runtime::connect(keep_running, &progress).map(|(channel, hello)| (Arc::new(channel), hello));
                    report_until_closed(&tx, result);
                });
                self.follow(host.clone(), id, rx, cx);
            }
            HostId::Server(server_id) => {
                let Some(server) = self.hosts.server(server_id).cloned() else {
                    return self.on_update(host.clone(), id, Update::Connected(Err("This server was removed.".into())), cx);
                };
                if let Err(e) = self.open_line(host, &server, cx) {
                    return self.on_update(host.clone(), id, Update::Connected(Err(e)), cx);
                }
            }
        }
        cx.notify();
    }

    /// `host`'s line, opened unless one for `server` as it is now is open and
    /// still trying: one that gave up, or was made from the server's old
    /// connection settings, is closed and a new one opened. A line that is
    /// connected already reports it again, to the connection just made.
    fn open_line(&mut self, host: &HostId, server: &Server, cx: &mut Context<Self>) -> Result<(), String> {
        if let Some(line) = self.lines.get(host) {
            let status = line.session.status();
            let gave_up = matches!(status.state, LineState::Failed(_)) && !signed_in(&status);
            if !gave_up && same_connection(&line.server, server) {
                // What it already reported is told again, to the connection just made.
                let id = self.connections.get(host).map_or(0, |c| c.id);
                for change in changes(&LineStatus { state: LineState::Connecting, hello: None, ..status.clone() }, &status) {
                    self.on_change(host, id, change, cx);
                }
                return Ok(());
            }
            self.close_line(host);
        }
        let (tx, rx) = futures::channel::mpsc::unbounded::<LineUpdate>();
        let (asker, auth) = remote::asker(server, self.questions_tx.clone())?;
        let events = tx.clone();
        let session = Arc::new(remote::open(server, auth, Box::new(move |event| drop(events.unbounded_send(LineUpdate::Event(event)))))?);
        let closed: Arc<AtomicBool> = Arc::default();
        // Every change of the session's state, in order; it says few of them with an event.
        std::thread::spawn({
            let (session, closed) = (session.clone(), closed.clone());
            move || {
                let mut last = session.status();
                let _ = tx.unbounded_send(LineUpdate::Status(Box::new(last.clone())));
                while !closed.load(Ordering::SeqCst) {
                    let now = session.wait_for(Duration::from_secs(1), |now| *now != last);
                    if now != last && !closed.load(Ordering::SeqCst) {
                        if tx.unbounded_send(LineUpdate::Status(Box::new(now.clone()))).is_err() {
                            break;
                        }
                        last = now;
                    }
                }
            }
        });
        let generation = next_connect_id();
        self.lines.insert(host.clone(), Line { session, _asker: asker, server: server.clone(), closed, last: None, generation });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            let mut rx = rx;
            while let Some(update) = rx.next().await {
                let applied = this.update(cx, |this, cx| {
                    this.on_line(&host, generation, update, cx);
                    this.sync_holds(cx);
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
        Ok(())
    }

    /// Close `host`'s line, if it has one: the helper is let go, and Julia keeps running there.
    fn close_line(&mut self, host: &HostId) {
        if let Some(line) = self.lines.remove(host) {
            drop(line.close());
        }
    }

    /// What `host`'s line (of `generation`) heard, applied to its connection of the moment.
    fn on_line(&mut self, host: &HostId, generation: u64, update: LineUpdate, cx: &mut Context<Self>) {
        if !self.lines.get(host).is_some_and(|l| l.generation == generation) {
            return;
        }
        let Some(id) = self.connections.get(host).map(|c| c.id) else { return };
        match update {
            LineUpdate::Event(SessionEvent::Step(event)) => self.on_update(host.clone(), id, Update::Event(event), cx),
            LineUpdate::Event(SessionEvent::Trouble(text)) => eprintln!("{}: {text}", self.hosts.name(host)),
            LineUpdate::Event(_) => {}
            LineUpdate::Status(now) => {
                let Some(line) = self.lines.get_mut(host) else { return };
                let now = *now;
                let last = line.last.replace(now.clone()).unwrap_or(LineStatus { state: LineState::Connecting, hello: None, ..now.clone() });
                for change in changes(&last, &now) {
                    self.on_change(host, id, change, cx);
                }
            }
        }
    }

    /// One change of a server's line, as the connection's update it stands for.
    fn on_change(&mut self, host: &HostId, id: u64, change: Change, cx: &mut Context<Self>) {
        let update = match change {
            Change::Connected(hello) => Update::Connected(Ok((None, hello_of(&hello)))),
            Change::ConnectFailed(why) => Update::Connected(Err(why)),
            Change::Lost(why) => Update::Notice(Notice::Lost(why)),
            Change::Started(runtime) => Update::Started(Ok(runtime_of(runtime))),
            Change::StartFailed(why) => Update::Started(Err(why)),
            Change::Died(why) => Update::Notice(Notice::Died(why)),
            Change::Replaced => Update::Notice(Notice::Replaced),
            // The app's own stop, or Cancel: a start that it cut short ends as cancelled.
            Change::Stopped => match self.connections.get(host) {
                Some(c) if c.status == Status::Starting => Update::Started(Err(String::new())),
                _ => return,
            },
            // The line went on with Julia by itself, as after a reconnect.
            Change::Starting => {
                let name = self.hosts.name(host);
                let Some(connection) = self.connections.get_mut(host).filter(|c| c.id == id && matches!(c.status, Status::Browsing | Status::Died(_))) else { return };
                connection.status = Status::Starting;
                connection.crashed = false;
                connection.steps = Steps { done: vec![format!("Connected to {name}")], ..Steps::new("Finding Julia") };
                return cx.notify();
            }
        };
        self.on_update(host.clone(), id, update, cx);
    }

    /// Start Julia on a connected host (or attach to the one running there).
    pub fn start_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        // A cluster restarted from its pane runs the shown session's resources.
        let shown = self.active_session().filter(|s| s.place.host == *host).and_then(|s| s.resources.clone());
        let line = self.lines.get(host).map(|l| l.session.clone());
        let listener = match host {
            HostId::ThisMac => match self.listener(host) {
                Ok(listener) => Some(listener),
                Err(_) => return,
            },
            HostId::Server(_) => None,
        };
        let Some(connection) = self.connections.get_mut(host) else { return };
        if !matches!(connection.status, Status::Browsing | Status::Died(_)) || connection.stopping {
            return;
        }
        let channel = connection.channel.clone();
        if channel.is_none() && line.is_none() {
            return;
        }
        connection.status = Status::Starting;
        connection.crashed = false;
        connection.cancelling = false;
        connection.found = None;
        connection.job = None;
        connection.steps = match host {
            HostId::ThisMac => Steps::new("Starting Julia"),
            HostId::Server(_) => Steps { done: vec![format!("Connected to {}", self.hosts.name(host))], ..Steps::new("Finding Julia") },
        };
        let job = match host {
            HostId::Server(id) => self.hosts.server(id).and_then(|s| s.cluster.as_ref()).map(|c| connection.job_request.take().unwrap_or_else(|| c.job(shown.as_ref().unwrap_or(&c.resources)))),
            HostId::ThisMac => None,
        };
        let id = connection.id;
        match (channel, listener, line) {
            (Some(channel), Some(listener), _) => {
                let (tx, rx) = futures::channel::mpsc::unbounded::<Update>();
                let notices = tx.clone();
                std::thread::spawn(move || {
                    let notice = move |notice| drop(notices.unbounded_send(Update::Notice(notice)));
                    let progress = |p: Progress| drop(tx.unbounded_send(Update::Local(p)));
                    let result = runtime::start_local(&channel, &listener, &progress, notice);
                    let _ = tx.unbounded_send(Update::Started(result));
                });
                self.follow(host.clone(), id, rx, cx);
            }
            // The line says how it goes (`on_line`). Julia is installed without asking, as it always was.
            (_, _, Some(session)) => session.request(&client::Want::Start { job, install: true }),
            _ => {}
        }
        cx.notify();
    }

    /// Stop This Mac's Julia and start it again (Settings → Restart Julia).
    /// Another Julia chosen since it started needs a new helper, which a
    /// reconnect starts with it.
    pub fn restart_local(&mut self, cx: &mut Context<Self>) {
        if self.julia_changed() {
            return self.reconnect_local(false, cx);
        }
        let host = HostId::ThisMac;
        let Some(connection) = self.connections.get_mut(&host) else { return self.connect_host(&host, true, cx) };
        let Some(channel) = connection.channel.clone().filter(|_| connection.status == Status::Ready) else { return };
        let running = connection.bridge().map(|bridge| (bridge, connection.notebooks.clone()));
        let gone = connection.forget_runtime();
        connection.status = Status::Starting;
        connection.steps = Steps::new("Stopping Julia");
        if let Some(listener) = self.listeners.get(&host) {
            listener.restarting();
        }
        self.close_page(gone, cx);
        self.status = "Restarting Julia…".into();
        let stop = cx.background_executor().spawn(async move {
            let resume = running.map(|(bridge, notebooks)| running_files(&bridge, &notebooks)).unwrap_or_default();
            (resume, channel.stop())
        });
        cx.spawn(async move |this, cx| {
            let (resume, stopped) = stop.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(connection) = this.connections.get_mut(&host) {
                    connection.status = Status::Died(String::new());
                    connection.resume = resume;
                    // Julia that didn't stop is still running: the start attaches to it again, and says why.
                    connection.not_restarted = stopped.err();
                }
                this.start_host(&host, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// What to offer beside Retry for `host`'s trouble: Repair once This
    /// Mac's has outlasted Retry twice, and for a server with no helper for
    /// its platform, how to get one.
    pub fn fixes(&self, host: &HostId, reason: &str) -> Vec<Fix> {
        let repair = *host == HostId::ThisMac && self.connections.get(host).is_some_and(|c| c.failures > REPAIR_AFTER);
        let helper = remote::HelperFix::of(reason).map(|fix| match fix.command() {
            Some(command) => Fix::CopyCommand(command),
            None => Fix::Download,
        });
        repair.then_some(Fix::Repair).into_iter().chain(helper).collect()
    }

    /// A fix's button: `small` beside the new-session screen's notice, else a
    /// secondary one beside a pane's primary action.
    pub fn fix_button(&self, fix: Fix, small: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(fix.id())
            .role(Role::Button)
            .flex_shrink_0()
            .when(small, |d| d.px(px(8.)).rounded(px(4.)))
            .when(!small, |d| d.px_3().py_1().rounded_sm())
            .cursor_pointer()
            .bg(theme::bg_raised())
            .text_color(theme::text_primary())
            .child(fix.label())
            .on_click(cx.listener(move |this, _, _, cx| this.apply_fix(&fix, cx)))
    }

    pub fn apply_fix(&mut self, fix: &Fix, cx: &mut Context<Self>) {
        match fix {
            Fix::Repair => self.repair_local(cx),
            Fix::Download => cx.open_url(crate::about::WEBSITE),
            Fix::CopyCommand(command) => {
                cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                self.status = "Copied the command; paste it in Terminal.".into();
                cx.notify();
            }
        }
    }

    /// Settings → Repair Julia, or Repair beside This Mac's error: stop
    /// Julia and the helper, clear the runtime's state that can go stale
    /// (`runtime::clear_state`), then connect and start again.
    pub fn repair_local(&mut self, cx: &mut Context<Self>) {
        self.reconnect_local(true, cx);
    }

    /// Stop This Mac's Julia and its helper, clear the runtime's state if
    /// `clear`, then connect and start again.
    fn reconnect_local(&mut self, clear: bool, cx: &mut Context<Self>) {
        let host = HostId::ThisMac;
        let (channel, running, gone) = match self.connections.get_mut(&host) {
            // Still installing Julia: nothing to repair yet.
            Some(c) if c.status == Status::Connecting => return,
            Some(c) => {
                let running = c.bridge().map(|bridge| (bridge, c.notebooks.clone()));
                // Updates from the connect or start under way are dropped from now on.
                c.id = next_connect_id();
                let gone = c.forget_runtime();
                c.status = Status::Starting;
                c.steps = Steps::new(if clear { "Clearing what Julia saved" } else { "Stopping Julia" });
                c.stopping = false;
                (c.channel.take(), running, gone)
            }
            None => (None, None, None),
        };
        self.close_page(gone, cx);
        self.settings_checks.repairing = clear;
        self.status = if clear { "Repairing Julia…" } else { "Restarting Julia…" }.into();
        let work = cx.background_executor().spawn(async move {
            let resume = running.map(|(bridge, notebooks)| running_files(&bridge, &notebooks)).unwrap_or_default();
            if let Some(channel) = channel {
                // A Julia that doesn't stop here is stopped by its recorded group below, on a repair.
                if let Err(e) = channel.stop() {
                    eprintln!("Stopping Julia: {e}");
                }
                channel.detach();
            }
            (if clear { runtime::clear_state() } else { Ok(Vec::new()) }, resume)
        });
        cx.spawn(async move |this, cx| {
            let (cleared, resume) = work.await;
            let _ = this.update(cx, |this, cx| {
                match cleared {
                    Ok(paths) if clear => eprintln!("Repair Julia cleared {paths:?}"),
                    Ok(_) => {}
                    Err(e) => eprintln!("Repair Julia: {e}"),
                }
                if let Some(connection) = this.connections.get_mut(&host) {
                    connection.status = Status::Failed(String::new());
                    connection.failures = 0;
                    connection.resume = resume;
                }
                this.connect_host(&host, true, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Make sure `host`'s Julia is running or on its way (a session needs it).
    pub fn ensure_runtime(&mut self, host: &HostId, cx: &mut Context<Self>) {
        match self.status(host) {
            Some(Status::Ready | Status::Starting) => {}
            Some(Status::Browsing | Status::Died(_)) => self.start_host(host, cx),
            _ => self.connect_host(host, true, cx),
        }
    }

    fn follow(&mut self, host: HostId, id: u64, mut rx: futures::channel::mpsc::UnboundedReceiver<Update>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(update) = rx.next().await {
                let applied = this.update(cx, |this, cx| {
                    this.on_update(host.clone(), id, update, cx);
                    // A server's sessions wait while it reconnects, and go once it's back.
                    this.sync_holds(cx);
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_update(&mut self, host: HostId, id: u64, update: Update, cx: &mut Context<Self>) {
        let local = host == HostId::ThisMac;
        let name = self.hosts.name(&host);
        let cluster = self.is_cluster(&host);
        let shown = !local && matches!(update, Update::Notice(Notice::Lost(_))) && self.shows_page_of(&host, cx);
        let session_waits = !local && self.sessions.iter().any(|s| s.place.host == host && (s.opening() || s.agent_waiting));
        let Some(connection) = self.connections.get_mut(&host).filter(|c| c.id == id) else { return };
        match update {
            Update::Event(Event::Connected { .. }) => connection.steps.advance(format!("Connected to {name}"), "Checking Endeavor's helper"),
            Update::Event(Event::Helper { installed }) => {
                connection.steps.now(if installed { "Installed Endeavor's helper" } else { "Connecting" });
            }
            Update::Event(Event::Found { version, .. }) => {
                connection.steps.found_julia = true;
                connection.steps.advance(format!("Julia {version}"), if cluster { "Submitting a job" } else { "Starting Julia" });
            }
            Update::Event(Event::Submitted { job, summary }) => {
                connection.job = Some(job.clone());
                connection.steps.found_julia = true;
                connection.steps.advance(format!("Submitted job {job} ({summary})"), "Waiting for a node");
            }
            Update::Event(Event::Queued { job, state, reason }) if state == "RUNNING" => {
                connection.job.get_or_insert(job);
                connection.steps.advance(format!("Got a node: {reason}"), "Starting Julia");
            }
            Update::Event(Event::Queued { job, reason, .. }) => {
                // A job this start found waiting, not one it submitted.
                connection.job.get_or_insert(job);
                connection.steps.now("Waiting for a node");
                connection.steps.detail = wire::slurm::reason_text(&reason).map(|r| format!("Slurm: {r}"));
            }
            Update::Event(Event::Progress(line)) => connection.steps.log(&line),
            Update::Event(Event::Started { .. } | Event::Finished { .. } | Event::Slurm(_)) => {}
            Update::Local(p) => {
                if connection.status == Status::Starting && p.log {
                    connection.steps.found_julia = true;
                    connection.steps.log(&p.detail);
                }
                if matches!(connection.status, Status::Connecting | Status::Starting) {
                    self.on_progress(p, cx);
                }
            }
            Update::Connected(Ok((channel, hello))) => {
                connection.linked = channel.is_none();
                connection.channel = channel;
                connection.hello = Some(hello);
                connection.status = Status::Browsing;
                if !local && connection.steps.done.is_empty() {
                    connection.steps.advance(format!("Connected to {name}"), "Finding Julia");
                }
                let (start, stop) = (connection.start_when_connected, connection.stop_when_connected);
                // Back as it was; one that starts Julia stays lost until it's up.
                let back = if start { None } else { connection.lost.take() };
                if let Some(Lost { was: Status::Died(reason), .. }) = back {
                    connection.status = Status::Died(reason);
                }
                self.on_connected(&host, !stop && !start, cx);
                if stop {
                    self.stop_host(&host, cx);
                } else if start {
                    self.start_host(&host, cx);
                }
            }
            Update::Connected(Err(e)) if connection.keep_trying(&e, session_waits) => {
                self.retry_lost(host.clone(), crate::offline::RETRY_EVERY, cx);
            }
            Update::Connected(Err(e)) => {
                if let Some(page) = connection.lost.take().and_then(|l| l.page) {
                    self.blank_page(&page, cx);
                }
                let Some(connection) = self.connections.get_mut(&host).filter(|c| c.id == id) else { return };
                connection.status = Status::Failed(e.clone());
                connection.failures += 1;
                if local {
                    self.status = JULIA_NOT_RUNNING.into();
                    if let Some(setup) = &mut self.setup {
                        setup.fail(e);
                    }
                }
            }
            Update::Started(Ok(runtime)) => {
                // The kept page gives way to the notebook reopened on the new connection.
                connection.lost = None;
                connection.job = None;
                connection.failures = 0;
                connection.steps.found_julia = true;
                let started = if runtime.reattached { format!("Julia running on {}", runtime.node) } else { "Started Julia".to_owned() };
                connection.steps.advance(started, "Opening the notebook");
                connection.status = Status::Ready;
                connection.runtime = Some(runtime);
                self.on_ready(&host, cx);
                self.warn_before_job_ends(&host, cx);
            }
            // The connection dropped under the start: it reconnects as lost.
            Update::Started(Err(_)) if connection.lost.is_some() && !connection.connected() => {}
            Update::Started(Err(_)) if connection.cancelling => {
                connection.cancelling = false;
                connection.job = None;
                connection.status = Status::Died(String::new());
            }
            Update::Started(Err(e)) => {
                if let Some(page) = connection.lost.take().and_then(|l| l.page) {
                    self.blank_page(&page, cx);
                }
                let Some(connection) = self.connections.get_mut(&host).filter(|c| c.id == id) else { return };
                connection.job = None;
                connection.not_restarted = None;
                connection.status = Status::Died(e.clone());
                connection.failures += 1;
                if local {
                    self.status = JULIA_NOT_RUNNING.into();
                    if let Some(setup) = &mut self.setup {
                        setup.fail(e);
                    }
                    if let Some(listener) = self.listeners.get(&host) {
                        listener.restart_failed();
                    }
                }
            }
            // Heard already, from the runtime or the helper's end.
            Update::Notice(Notice::Lost(_)) if !connection.connected() => {}
            // A server that drops reconnects by itself; its notebook, if showing, stays up read-only meanwhile.
            Update::Notice(Notice::Lost(reason)) if !local => {
                let was = connection.status.clone();
                let gone = connection.forget_runtime();
                let page = gone.as_ref().and_then(|r| r.page_url.split_once('?')).map(|(origin, _)| origin.to_owned()).filter(|_| shown);
                connection.unlink();
                connection.status = Status::Failed(reason);
                connection.lost = Some(Lost { page, was });
                // A blip often passes: the first try comes soon.
                if self.offline_since().is_none() {
                    self.retry_lost(host.clone(), std::time::Duration::from_secs(2), cx);
                }
            }
            Update::Notice(notice) => {
                // Heard only when Julia went by itself: the app's own stops leave first.
                let crashed = matches!(notice, Notice::Died(_)).then(|| crate::crash::running_notebooks(&connection.notebooks));
                let gone = connection.forget_runtime();
                connection.status = match notice {
                    Notice::Died(reason) => {
                        connection.crashed = true;
                        Status::Died(reason)
                    }
                    Notice::Replaced => {
                        connection.unlink();
                        Status::Replaced
                    }
                    Notice::Lost(reason) => {
                        connection.unlink();
                        Status::Failed(reason)
                    }
                };
                self.close_page(gone, cx);
                if local {
                    self.status = JULIA_NOT_RUNNING.into();
                }
                if let Some(running) = crashed {
                    self.julia_died(&host, running, cx);
                }
            }
        }
        cx.notify();
    }

    /// A dropped server's reconnect: it starts Julia again only if it ran.
    pub fn reconnect_lost(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let julia = self.connections.get(host).and_then(|c| c.lost.as_ref()).is_some_and(Lost::had_julia);
        self.connect_host(host, julia, cx);
    }

    /// Connected, before any runtime: the new-session screen can use its files,
    /// and (with `check`) the host list hears what runs there.
    fn on_connected(&mut self, host: &HostId, check: bool, cx: &mut Context<Self>) {
        if check && *host != HostId::ThisMac {
            self.check_host(host, cx);
        }
        self.fetch_partitions(host, cx);
        if self.draft.host == *host {
            // An open folder browser lists again, after a reconnect.
            if let Some(path) = self.draft.browser.as_ref().map(|b| b.path.clone()) {
                self.browse_to(path, cx);
            }
            if self.draft.folder.is_none() {
                self.draft.folder = self.connections.get(host).and_then(|c| c.hello.as_ref()).map(|h| h.home.clone());
            }
            self.scan_notebooks(cx);
            if let crate::new_session::NotebookChoice::Existing(path) = self.draft.notebook.clone() {
                self.choose_notebook(crate::new_session::NotebookChoice::Existing(path), cx);
            }
        }
    }

    /// `host`'s runtime is up: tell it what the app knows, follow its notebooks,
    /// and start the sessions waiting on it.
    fn on_ready(&mut self, host: &HostId, cx: &mut Context<Self>) {
        if *host == HostId::ThisMac {
            self.settings_checks.repairing = false;
        }
        let Some(bridge) = self.bridge(host) else { return };
        let reattached = self.connections.get(host).and_then(|c| c.runtime.as_ref()).is_some_and(|r| r.reattached);
        if *host == HostId::ThisMac {
            if let Some(commands) = self.links.get_mut(crate::agent::Agent::Claude).rx.take() {
                self.on_progress(Progress::new(Step::Agent, "Pluto ready · starting Claude…"), cx);
                self.start_agent(crate::agent::Agent::Claude, commands, cx);
            } else if let Some(e) = self.connections.get_mut(host).and_then(|c| c.not_restarted.take()) {
                eprintln!("Restart Julia: {e}");
                self.status = not_restarted(&e).into();
            } else if self.this_mac_was_ready {
                self.status = if reattached { "Reconnected to Julia." } else { "Julia restarted." }.into();
            } else {
                self.status = "Julia ready.".into();
            }
            self.this_mac_was_ready = true;
        }
        self.send_idle_limit(host, cx);
        if self.active_session().is_some_and(|s| s.place.host == *host) {
            self.follow_folder(cx);
        }
        if let Some(connection) = self.connections.get_mut(host) {
            connection.older = connection.runtime.as_ref().map(|r| r.version);
            if let Some(version) = connection.older {
                for session in self.sessions.iter_mut().filter(|s| s.place.host == *host && !s.agent_waiting) {
                    session.runtime_build(version);
                }
            }
        }
        // A new runtime knows no session's notebook or policy.
        let on_host: Vec<&Session> = self.sessions.iter().filter(|s| s.place.host == *host).collect();
        let bound: Vec<(u64, String)> = on_host.iter().filter_map(|s| Some((s.key, s.notebook_path.clone()?))).collect();
        let folders: Vec<(u64, std::path::PathBuf)> = on_host.iter().map(|s| (s.key, s.place.path.clone())).collect();
        let policies: Vec<(u64, &'static str, bool)> = on_host.iter().map(|s| (s.key, s.policy(), s.edits_ask())).collect();
        let waiting: Vec<u64> = on_host.iter().filter(|s| s.agent_waiting).map(|s| s.key).collect();
        for (key, path) in bound {
            let bridge = bridge.clone();
            cx.background_executor().spawn(async move { pluto::set_notebook(&bridge, key, &path) }).detach();
        }
        for (key, folder) in folders {
            let bridge = bridge.clone();
            cx.background_executor().spawn(async move { pluto::set_session_folder(&bridge, key, &folder) }).detach();
        }
        for (key, policy, edits) in policies {
            self.send_policy(key, policy, edits, cx);
        }
        self.reopen_notebooks(host, cx);
        self.watch_notebooks(host, cx);
        for key in waiting {
            self.request_agent(key, cx);
        }
        cx.notify();
    }

    /// Follow `host`'s notebook list (pushed on every change) for the end-of-turn
    /// run check, the idle stops and reopening notebooks after a crash.
    fn watch_notebooks(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get(host) else { return };
        let Some(bridge) = connection.bridge() else { return };
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<serde_json::Value>();
        let watching = connection.watching.clone();
        let mine = watching.fetch_add(1, Ordering::SeqCst) + 1;
        // A long-lived blocking read: its own thread, not the executor's pool. If the
        // stream drops it reconnects, until this runtime is replaced or dies.
        let current = watching.clone();
        std::thread::spawn(move || {
            while current.load(Ordering::SeqCst) == mine {
                let _ = pluto::watch_notebooks(&bridge, |event| {
                    let _ = tx.unbounded_send(event);
                });
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                // A runtime that's going still sends its list as it closes: it
                // must not replace what the next runtime reopens.
                if watching.load(Ordering::SeqCst) != mine {
                    break;
                }
                if this.update(cx, |this, cx| this.on_notebooks_event(&host, event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_notebooks_event(&mut self, host: &HostId, mut event: serde_json::Value, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get_mut(host) else { return };
        // A notebook the runtime just stopped for being idle shows stopped, with
        // Start, in every session on it. Only new entries count: a stale one must
        // not stop a notebook the user has started again since.
        let stops = pluto::idle_stopped(&event);
        let mut new_stops = Vec::new();
        for (path, hours, safe_preview) in &stops {
            if connection.idle_stopped.contains(path) {
                continue;
            }
            new_stops.push(path.clone());
            let stopped = Stopped { safe_preview: *safe_preview, idle_hours: Some(*hours), modified: None };
            for session in self.sessions.iter_mut().filter(|s| s.place.host == *host && s.notebook_path.as_deref() == Some(path.as_str())) {
                session.notebook = None;
                session.stopped = Some(stopped);
            }
        }
        connection.idle_stopped = stops.into_iter().map(|(path, ..)| path).collect();
        let list = event["notebooks"].take();
        connection.last_notebooks = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|nb| Some((nb["notebook_id"].as_str()?.to_owned(), nb["path"].as_str()?.to_owned())))
            .collect();
        let mut refreshed = false;
        for session in self.sessions.iter_mut().filter(|s| s.place.host == *host) {
            refreshed |= session.refresh_run_state(&list);
        }
        if refreshed {
            cx.notify();
        }
        let exits = crate::crash::new_exits(&connection.notebooks, &list, &event["cells"]);
        connection.notebooks = list;
        for (notebook, cell, name) in pluto::user_edits(&connection.cells, &event["cells"]) {
            for session in self.sessions.iter_mut().filter(|s| s.notebook.as_deref() == Some(notebook.as_str())) {
                session.note_user_edit(cell.clone(), name.clone());
            }
        }
        for (notebook, cells) in pluto::cells_that_ran(&connection.cells, &event["cells"]) {
            for session in self.sessions.iter_mut().filter(|s| s.notebook.as_deref() == Some(notebook.as_str())) {
                session.cells_ran(&cells);
            }
            cx.notify();
        }
        let asks = event["asks"].as_array().cloned().unwrap_or_default();
        let on_host: Vec<u64> = self.sessions.iter().filter(|s| s.place.host == *host).map(|s| s.key).collect();
        for key in on_host {
            let effects = self.session_mut(key).map(|s| s.runtime_asks_now(&asks)).unwrap_or_default();
            self.apply_effects(key, effects, cx);
        }
        let Some(connection) = self.connections.get_mut(host) else { return };
        connection.cells = event["cells"].take();
        self.push_cells(cx);
        for path in new_stops {
            self.note_stopped_file(host, path, cx);
        }
        self.crash_progress(host);
        for (path, cell) in exits {
            self.notebook_stopped((host.clone(), path), cell, cx);
        }
    }

    /// Reopen the notebooks that were open in `host`'s last runtime (unless this
    /// one has them open already, as after a reconnect), and point each session
    /// (and the pane) at the reopened copy. Pluto saves on every change, so the
    /// files are current. They open in safe preview, except the ones in
    /// `resume` whose files haven't changed since.
    ///
    /// Each session's own notebook reopens too, even if the last runtime's list
    /// had lost it: a restart during an earlier reopen leaves the list empty,
    /// and a session left without its notebook shows "Opening" for good.
    fn reopen_notebooks(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get_mut(host) else { return };
        let Some(runtime) = connection.runtime.as_ref() else { return };
        let (bridge, reattached, this_runtime) = (runtime.bridge.clone(), runtime.reattached, runtime.pid);
        let resume = std::mem::take(&mut connection.resume);
        let before = connection.last_notebooks.clone();
        let mut paths: Vec<String> = before.iter().map(|(_, p)| p.clone()).collect();
        for session in self.sessions.iter().filter(|s| s.place.host == *host && s.stopped.is_none() && !s.missing) {
            if let Some(path) = session.notebook_path.as_ref().filter(|p| !paths.contains(p)) {
                paths.push(path.clone());
            }
        }
        if paths.is_empty() {
            return;
        }
        let reopen = cx.background_executor().spawn(async move {
            let list = || pluto::call_tool(&bridge, "list_notebooks", serde_json::json!({})).ok();
            let find = |listed: &Option<serde_json::Value>, path: &str| listed.as_ref()?.as_array()?.iter().find(|nb| nb["path"] == path).cloned();
            let listed = list();
            paths
                .into_iter()
                .filter_map(|path| {
                    let result = match find(&listed, &path) {
                        Some(nb) => nb,
                        None => {
                            let run = resume.iter().any(|(p, modified)| *p == path && modified.is_none_or(|m| pluto::file_info(&bridge, &path) == Ok(Some(m))));
                            match pluto::call_tool(&bridge, "open_notebook", serde_json::json!({ "path": path, "run_notebook": run })) {
                                Ok(nb) => nb,
                                // Opened meanwhile (the session's own open, or Claude's).
                                Err(e) => find(&list(), &path).or_else(|| {
                                    eprintln!("Couldn't reopen {path}: {e}");
                                    None
                                })?,
                            }
                        }
                    };
                    let previewed = result["execution_allowed"] == false;
                    Some((result["notebook_id"].as_str()?.to_owned(), path, previewed))
                })
                .collect::<Vec<_>>()
        });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            let reopened = reopen.await;
            let previewed = reopened.iter().filter(|(.., previewed)| *previewed).count();
            let reopened: Vec<(String, String)> = reopened.into_iter().map(|(id, path, _)| (id, path)).collect();
            let _ = this.update(cx, |this, cx| {
                // The runtime went (a second restart) while this reopen ran: the next one's reopen does it.
                if this.connections.get(&host).and_then(|c| c.runtime.as_ref()).is_none_or(|r| r.pid != this_runtime) {
                    eprintln!("Dropped a reopen of {} notebook(s) on {}: its runtime has gone", reopened.len(), this.hosts.name(&host));
                    return;
                }
                let by_path = |path: &str| reopened.iter().find(|(_, p)| p == path).map(|(id, _)| id.clone());
                let new_id = |old: &str| by_path(before.iter().find(|(id, _)| id == old).map(|(_, p)| p)?);
                let mut lost = Vec::new();
                for session in this.sessions.iter_mut().filter(|s| s.place.host == host) {
                    let own = session.notebook_path.as_deref().filter(|_| session.stopped.is_none() && !session.missing);
                    session.notebook = own.and_then(by_path).or_else(|| session.notebook.as_deref().and_then(new_id));
                    if own.is_some() && session.notebook.is_none() {
                        lost.push(session.key);
                    }
                }
                // A file that's gone shows File not found.
                for key in lost {
                    this.check_missing(key, cx);
                }
                if let Some((host, id)) = this.active_session().and_then(|s| Some((s.place.host.clone(), s.notebook.clone()?))) {
                    this.load_notebook(&host, &id, cx);
                }
                if host == HostId::ThisMac && !reopened.is_empty() && !reattached {
                    this.status = match previewed {
                        0 => format!("Julia restarted; reopened {} notebook(s).", reopened.len()),
                        n => format!("Julia restarted; reopened {} notebook(s), {n} in safe preview.", reopened.len()),
                    }
                    .into();
                }
                if let Some(connection) = this.connections.get_mut(&host) {
                    connection.last_notebooks = reopened;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Stop open notebooks after the host's idle limit: a server's own, else Settings'.
    pub fn send_idle_limit(&self, host: &HostId, cx: &mut Context<Self>) {
        let Some(bridge) = self.bridge(host) else { return };
        let limit = match host {
            HostId::Server(id) => self.hosts.server(id).and_then(|s| s.idle_stop),
            HostId::ThisMac => None,
        };
        let hours = limit.unwrap_or(self.settings.idle_stop).hours();
        // ponytail: a failed send leaves the runtime on its default (48 hours) until the next start.
        cx.background_executor().spawn(async move { pluto::set_idle_limit(&bridge, hours) }).detach();
    }

    /// When the running cluster job on `host` ends (Unix seconds), and whether that's soon.
    pub fn job_end(&self, host: &HostId) -> Option<(u64, bool)> {
        let connection = self.connections.get(host).filter(|c| c.status == Status::Ready)?;
        let at = connection.runtime.as_ref()?.job.as_ref()?.ends_at?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Some((at, at.saturating_sub(now) <= job_warning_window().as_secs()))
    }

    /// "Job ends 18:40", for the notebook header of a session on a cluster.
    pub fn job_ends(&self, host: &HostId) -> Option<AnyElement> {
        let (at, soon) = self.job_end(host)?;
        Some(
            div()
                .flex_shrink_0()
                .mr(px(8.))
                .px(px(6.))
                .rounded(px(3.))
                .bg(theme::bg_tag())
                .text_size(theme::size_meta_small())
                .text_color(if soon { theme::accent_text() } else { theme::text_faint() })
                .child(format!("Job ends {}", crate::when::clock(at)))
                .into_any_element(),
        )
    }

    pub fn is_cluster(&self, host: &HostId) -> bool {
        matches!(host, HostId::Server(id) if self.hosts.server(id).is_some_and(|s| s.cluster.is_some()))
    }

    /// Cancel starting Julia on a cluster: the queued job is cancelled.
    pub fn cancel_start(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(helper) = self.helper(host) else { return };
        let Some(connection) = self.connections.get_mut(host).filter(|c| c.status == Status::Starting) else { return };
        connection.cancelling = true;
        connection.steps.now("Cancelling the job");
        cx.background_executor().spawn(async move { helper.stop().map_err(|e| eprintln!("Cancelling the start: {e}")) }).detach();
        cx.notify();
    }

    /// Ask a connected host what runs there (a runtime, or a cluster job for
    /// one), without taking it over. A host that isn't connected connects,
    /// which then asks.
    pub fn check_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let helper = self.helper(host);
        let Some(connection) = self.connections.get_mut(host).filter(|c| matches!(c.status, Status::Browsing | Status::Died(_))) else {
            if matches!(self.status(host), None | Some(Status::Failed(_) | Status::Replaced)) {
                self.connect_host(host, false, cx);
            }
            return;
        };
        let Some(helper) = helper.filter(|_| !connection.stopping) else { return };
        connection.found = None;
        connection.check += 1;
        let (id, check) = (connection.id, connection.check);
        let ask = cx.background_executor().spawn(async move { helper.files(Request::Runtime) });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            let reply = ask.await;
            let _ = this.update(cx, |this, cx| {
                let Some(connection) = this.connections.get_mut(&host).filter(|c| c.id == id && c.check == check) else { return };
                connection.found = Some(match reply {
                    Ok(Reply::Runtime { runtime }) => Ok(runtime),
                    Ok(other) => Err(format!("The helper answered {other:?}.")),
                    Err(e) => Err(e),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stop Julia on `host` (Settings' host list): its runtime, or on a
    /// cluster its job, queued or running, whether or not this app attached
    /// to it. A host that isn't connected connects first. Sessions on it then
    /// show Julia stopped, with Start.
    pub fn stop_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        match self.status(host) {
            Some(Status::Browsing | Status::Starting | Status::Ready | Status::Died(_)) => {}
            Some(Status::Connecting) => {
                if let Some(connection) = self.connections.get_mut(host) {
                    connection.stop_when_connected = true;
                }
                return cx.notify();
            }
            None | Some(Status::Failed(_) | Status::Replaced) => {
                self.connect_host(host, false, cx);
                if let Some(connection) = self.connections.get_mut(host) {
                    connection.stop_when_connected = true;
                }
                return;
            }
        }
        let helper = self.helper(host);
        let Some(connection) = self.connections.get_mut(host) else { return };
        let Some(helper) = helper.filter(|_| !connection.stopping) else { return };
        let gone = if connection.status == Status::Starting {
            // The start under way ends as cancelled (Update::Started).
            connection.cancelling = true;
            None
        } else {
            connection.status = Status::Died(String::new());
            if let Some(listener) = self.listeners.get(host) {
                listener.disconnected();
            }
            connection.forget_runtime()
        };
        connection.stop_when_connected = false;
        connection.stopping = true;
        connection.found = None;
        connection.check += 1;
        let id = connection.id;
        self.close_page(gone, cx);
        if *host == HostId::ThisMac {
            self.status = "Stopping Julia…".into();
        }
        let stop = cx.background_executor().spawn(async move { helper.stop() });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            let stopped = stop.await;
            let _ = this.update(cx, |this, cx| {
                let mut restart = false;
                if let Some(connection) = this.connections.get_mut(&host).filter(|c| c.id == id) {
                    connection.stopping = false;
                    if let Err(e) = stopped {
                        // It may still run: ask what does.
                        connection.start_after_stop = false;
                        this.status = not_stopped(&this.hosts.name(&host), &e).into();
                        return this.check_host(&host, cx);
                    }
                    connection.found = Some(Ok(RuntimeState::NotRunning));
                    restart = std::mem::take(&mut connection.start_after_stop);
                }
                if restart {
                    return this.start_host(&host, cx);
                }
                if host == HostId::ThisMac {
                    this.status = concat!("Julia on ", crate::platform::this_computer!(), " is stopped.").into();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stop Julia on `host` and start it again: for Julia an older Endeavor
    /// started, which this one can't attach to.
    pub fn restart_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        self.stop_host(host, cx);
        if let Some(connection) = self.connections.get_mut(host).filter(|c| c.stopping) {
            connection.start_after_stop = true;
        }
    }

    /// What runs on `host`, as far as the app knows, for the host list and the Where menu.
    pub fn host_state(&self, host: &HostId) -> HostState {
        let Some(c) = self.connections.get(host) else { return HostState::Unknown };
        if c.stopping {
            return HostState::Stopping;
        }
        if c.lost.is_some() {
            return HostState::Lost;
        }
        match &c.status {
            Status::Connecting => HostState::Connecting,
            Status::Browsing => match &c.found {
                None => HostState::Checking,
                Some(Ok(found)) => HostState::from(found),
                Some(Err(_)) => HostState::Connected,
            },
            // Started since by another app, as a check found.
            Status::Died(_) => match &c.found {
                Some(Ok(found @ (RuntimeState::Running { .. } | RuntimeState::Queued { .. } | RuntimeState::Starting))) => HostState::from(found),
                _ => HostState::NotRunning,
            },
            Status::Starting => match &c.job {
                Some(job) => HostState::Queued { job: job.clone(), starting: c.steps.current != "Waiting for a node" },
                None => HostState::Starting,
            },
            Status::Ready => HostState::Running {
                notebooks: c.notebooks.as_array().map(Vec::len),
                job: c.runtime.as_ref().and_then(|r| r.job.as_ref()).map(|j| (j.id.clone(), j.ends_at)),
            },
            Status::Replaced => HostState::Replaced,
            Status::Failed(e) => HostState::Failed(e.clone()),
        }
    }

    /// When a cluster job's end is near, say so in the chat of each session on
    /// it: the notebook file is saved, and a new job can run it again.
    fn warn_before_job_ends(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get(host) else { return };
        let Some(ends_at) = connection.runtime.as_ref().and_then(|r| r.job.as_ref()).and_then(|j| j.ends_at) else { return };
        let window = job_warning_window();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let wait = Duration::from_secs(ends_at.saturating_sub(window.as_secs()).saturating_sub(now));
        let (watching, mine) = (connection.watching.clone(), connection.watching.load(Ordering::SeqCst));
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            if watching.load(Ordering::SeqCst) != mine {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let name = this.hosts.name(&host);
                let left = ends_at.saturating_sub(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()));
                let minutes = left.div_ceil(60);
                let text = format!(
                    "⚠ The cluster job running Julia on {name} ends at {} (in {minutes} min), and its notebooks stop then. The notebook file is already saved; Start Julia afterwards runs it in a new job.",
                    crate::when::clock(ends_at)
                );
                for session in this.sessions.iter_mut().filter(|s| s.place.host == host) {
                    session.note(text.clone());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Leave a host (its server was removed): its runtime keeps running.
    pub fn disconnect_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(mut connection) = self.connections.remove(host) else { return };
        if let Some(listener) = self.listeners.get(host) {
            listener.disconnected();
        }
        let gone = connection.forget_runtime();
        self.close_page(gone, cx);
        if let Some(channel) = connection.channel.take() {
            cx.background_executor().spawn(async move { channel.detach() }).detach();
        }
        self.close_line(host);
        cx.notify();
    }

    /// The app is quitting: servers' runtimes keep running (their idle stop
    /// covers them); This Mac's as Settings says. The helpers do the rest.
    pub fn quit_runtimes(&mut self) {
        for (host, connection) in &self.connections {
            if let Some(channel) = &connection.channel {
                channel.quit(*host != HostId::ThisMac || self.settings.keep_running);
            }
        }
        // Each server's helper is let go, and Julia keeps running there; a
        // server slow to answer isn't waited for long.
        let closing: Vec<_> = self.lines.drain().map(|(_, line)| line.close()).collect();
        let until = Instant::now() + Duration::from_secs(2);
        while closing.iter().any(|c| !c.is_finished()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// What the notebook pane shows while `host` isn't ready; None once it is,
    /// and while a dropped server's page stays up, read-only, as it reconnects.
    pub fn host_pane_state(&self, host: &HostId, cx: &App) -> Option<HostPane> {
        let connection = self.connections.get(host);
        if let Some(lost) = connection.and_then(|c| c.lost.as_ref()) {
            let shown = crate::webcontent::url(self.webview.read(cx).raw());
            return (!lost.page.as_deref().is_some_and(|page| shown.starts_with(page))).then_some(HostPane::Lost);
        }
        Some(match connection.map_or(Status::Connecting, |c| c.status.clone()) {
            Status::Ready => return None,
            Status::Connecting | Status::Browsing | Status::Starting => HostPane::Starting,
            Status::Died(_) if connection.is_some_and(|c| c.stopping) => HostPane::Stopping,
            Status::Died(reason) if connection.is_some_and(|c| c.crashed) => HostPane::Crashed(reason),
            Status::Died(reason) => HostPane::NotRunning(reason),
            Status::Replaced => HostPane::Replaced,
            Status::Failed(reason) => HostPane::NotConnected(reason),
        })
    }

    /// The notebook pane while `host` isn't ready: its state, and what to do
    /// about it. `start`: Reconnect also starts Julia, as a session needs; the
    /// new-session screen only browses the host's files.
    pub fn host_pane(&self, host: &HostId, start: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.host_pane_state(host, cx)?;
        let connection = self.connections.get(host);
        let name = self.hosts.name(host);
        let action = |id: &'static str, label: &'static str, host: HostId, start_julia: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .px_3()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .bg(theme::accent())
                .text_color(gpui::white())
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if start_julia {
                        this.start_host(&host, cx);
                    } else {
                        this.connect_host(&host, start, cx);
                    }
                }))
        };
        let message = |text: String| div().max_w(px(420.)).text_center().text_size(theme::size_meta()).text_color(theme::text_muted()).child(text);
        let resting = crate::new_session::turtle_pane();
        let pane = match state {
            HostPane::Lost => {
                let offline = self.offline_since().is_some();
                // A session's pane has "Can't reach" above it already.
                if start {
                    let text = if offline { format!("Endeavor reconnects to {name} when you're back online.") } else { format!("Endeavor reconnects to {name} by itself.") };
                    resting.child(div().text_color(theme::text_muted()).child(text))
                } else {
                    let text = if offline { "Endeavor reconnects when you're back online." } else { "Endeavor reconnects by itself." };
                    let host = host.clone();
                    // Opens Settings at the server, where Try now and its connection settings are.
                    let title = div()
                        .id("cant-reach-host")
                        .role(Role::Link)
                        .aria_label(format!("Can't reach {name}: open Where notebooks run"))
                        .border_2()
                        .border_color(gpui::transparent_black())
                        .track_focus(&self.dialog_focus("cant-reach-host", cx))
                        .tab_stop(true)
                        .focus_ring()
                        .cursor_pointer()
                        .text_color(theme::text_primary())
                        .hover(|s| s.underline())
                        .child(format!("Can't reach {name}"))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_settings_at_host(&host, window, cx)));
                    resting.child(title).child(div().text_color(theme::text_muted()).child(text))
                }
            }
            HostPane::Starting => {
                let steps = connection.map_or_else(|| Steps::new(format!("Connecting to {name}")), |c| c.steps.clone());
                let starting = connection.is_some_and(|c| c.status == Status::Starting);
                let cancel = (starting && self.is_cluster(host) && connection.is_some_and(|c| !c.cancelling)).then(|| {
                    let host = host.clone();
                    div()
                        .id("cancel-start")
                        .role(Role::Button)
                        .mt(px(4.))
                        .cursor_pointer()
                        .underline()
                        .text_size(theme::size_meta())
                        .text_color(theme::text_muted())
                        .hover(|s| s.text_color(theme::text_primary()))
                        .child("Cancel")
                        .on_click(cx.listener(move |this, _, _, cx| this.cancel_start(&host, cx)))
                });
                return Some(starting_pane(&steps).children(cancel).into_any_element());
            }
            HostPane::Stopping => resting.child(div().text_color(theme::text_muted()).child(format!("Stopping Julia on {name}…"))),
            HostPane::NotRunning(reason) => return Some(self.julia_stopped_page(host, &reason, cx)),
            HostPane::Crashed(reason) => return Some(self.julia_crashed_page(host, &reason, cx)),
            HostPane::Replaced => resting
                .child(div().text_color(theme::text_muted()).child(format!("Another connection took over Julia on {name}.")))
                .child(action("host-reconnect", "Reconnect", host.clone(), false)),
            HostPane::NotConnected(reason) => {
                let fixes = self.fixes(host, &reason);
                let repair = fixes.contains(&Fix::Repair);
                resting
                    .child(div().text_color(theme::text_muted()).child(format!("Not connected to {name}.")))
                    .child(message(reason))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(action("host-reconnect", "Reconnect", host.clone(), false))
                            .children(fixes.into_iter().map(|fix| self.fix_button(fix, false, cx))),
                    )
                    .when(repair, |d| d.child(message(REPAIR_NOTE.into())))
            }
        };
        Some(pane.into_any_element())
    }
}

/// The notebook pane of a host that isn't ready.
#[derive(Clone, Debug, PartialEq)]
pub enum HostPane {
    /// A server's connection dropped by itself: "Can't reach", reconnecting.
    Lost,
    /// Connecting, or Julia starting: the step log.
    Starting,
    Stopping,
    /// "Julia isn't running", and why.
    NotRunning(String),
    /// Julia stopped by itself: "Julia stopped unexpectedly", and why.
    Crashed(String),
    /// Another connection took the runtime over.
    Replaced,
    /// "Not connected", and why.
    NotConnected(String),
}

/// Send how the connect went, then, if it got through, wait for the helper's
/// end: a drop is heard as `Notice::Lost` whether or not Julia runs.
/// A server's `client::Session`, with what the app keeps of it.
pub(crate) struct Line {
    session: Arc<client::Session>,
    /// ssh's prompts come through it while the session lives.
    _asker: client::Asker,
    /// The server as the line was opened for it.
    server: Server,
    /// Set once the line is closed: its watcher stops.
    closed: Arc<AtomicBool>,
    /// The status last applied (`on_line`).
    last: Option<LineStatus>,
    /// Tells this line's updates from a line opened after it for the same host.
    generation: u64,
}

impl Line {
    /// Let the helper go, end the connection and stop watching, in a thread:
    /// a close waits for a connect under way to be cut short.
    fn close(self) -> std::thread::JoinHandle<()> {
        self.closed.store(true, Ordering::SeqCst);
        let Line { session, _asker, .. } = self;
        std::thread::spawn(move || {
            session.close();
            drop(_asker);
        })
    }
}

/// What a server's line tells the app.
enum LineUpdate {
    Event(SessionEvent),
    Status(Box<LineStatus>),
}

/// One change of a server's line, in the app's terms (`changes`).
#[derive(Clone, Debug, PartialEq)]
enum Change {
    Connected(client::HelloInfo),
    ConnectFailed(String),
    /// The connection dropped; the line gets it back by itself.
    Lost(String),
    /// Julia went on starting by itself (as a reconnect does), or the app's start began.
    Starting,
    Started(client::RuntimeInfo),
    StartFailed(String),
    /// Julia stopped by itself; the reason, without the line's "Julia on X stopped."
    Died(String),
    Replaced,
    /// The app's stop or cancel ended Julia, or the start under way.
    Stopped,
}

/// Signed in and the helper up: the line is connected, whatever its runtime does.
fn signed_in(status: &LineStatus) -> bool {
    status.state != LineState::Connecting && status.hello.as_ref().is_some_and(|h| !h.node.is_empty())
}

/// What the runtime does, as far as `changes` tells states apart.
#[derive(PartialEq)]
enum Phase<'a> {
    Idle,
    Starting,
    Ready(&'a client::RuntimeInfo),
    Failed(&'a str),
}

fn phase(status: &LineStatus) -> Phase<'_> {
    match &status.state {
        LineState::Connecting | LineState::Connected => Phase::Idle,
        LineState::NothingRunning => Phase::Failed(status.step.as_deref().unwrap_or("")),
        LineState::Starting { .. } | LineState::Queued(_) => Phase::Starting,
        LineState::Ready(runtime) => Phase::Ready(runtime),
        LineState::NeedsInstall(_) => Phase::Failed(status.step.as_deref().unwrap_or("")),
        LineState::Failed(why) => Phase::Failed(why),
    }
}

/// How a server's line went from `last` to `now`, as the updates the app has
/// always had from its connects and starts. Between two looks the line may
/// pass through states unseen; what it ends in decides.
fn changes(last: &LineStatus, now: &LineStatus) -> Vec<Change> {
    let mut out = Vec::new();
    let (was_in, is_in) = (signed_in(last), signed_in(now));
    match (was_in, is_in) {
        (false, false) => {
            if let LineState::Failed(why) = &now.state
                && last.state != now.state
            {
                out.push(Change::ConnectFailed(why.clone()));
            }
            return out;
        }
        (true, false) => {
            let why = now.state.error().map(str::to_owned).or_else(|| now.step.clone()).unwrap_or_default();
            out.push(Change::Lost(why));
            return out;
        }
        (false, true) => out.push(Change::Connected(now.hello.clone().unwrap_or_default())),
        (true, true) => {}
    }
    let before = if was_in { phase(last) } else { Phase::Idle };
    let after = phase(now);
    if before == after {
        return out;
    }
    match (before, after) {
        (_, Phase::Ready(runtime)) => {
            if !matches!(last.state, LineState::Starting { .. } | LineState::Queued(_)) {
                out.push(Change::Starting);
            }
            out.push(Change::Started(runtime.clone()));
        }
        (Phase::Idle | Phase::Failed(_), Phase::Starting) => out.push(Change::Starting),
        (Phase::Ready(_), Phase::Starting) => {}
        (Phase::Ready(_), Phase::Failed(why)) if why == format!("Another connection took Julia on {} over.", now.name) => out.push(Change::Replaced),
        (Phase::Ready(_), Phase::Failed(why)) => {
            let prefix = format!("Julia on {} stopped.", now.name);
            out.push(Change::Died(why.strip_prefix(&prefix).unwrap_or(why).trim().to_owned()));
        }
        (Phase::Starting | Phase::Ready(_), Phase::Idle) => out.push(Change::Stopped),
        (Phase::Starting, Phase::Failed(why)) => out.push(Change::StartFailed(why.to_owned())),
        // A start that ended before it was seen to begin.
        (Phase::Idle, Phase::Failed(why)) => out.push(Change::StartFailed(why.to_owned())),
        (Phase::Failed(_), Phase::Failed(_) | Phase::Idle) | (Phase::Idle, Phase::Idle) | (Phase::Starting, Phase::Starting) => {}
    }
    out
}

/// The helper's hello, as the app's connections keep it.
fn hello_of(hello: &client::HelloInfo) -> Hello {
    let launcher = match hello.launcher.as_deref() {
        Some("slurm") => Some(client::Launcher::Slurm),
        Some("process") => Some(client::Launcher::Process),
        _ => None,
    };
    Hello { protocol: wire::PROTOCOL, node: hello.node.clone(), home: hello.home.clone().into(), slurm: hello.slurm, uploads: hello.uploads, launcher }
}

/// A server's runtime, as the app follows it.
fn runtime_of(runtime: client::RuntimeInfo) -> Runtime {
    let version = crate::older_runtime::Version::of_server(&runtime);
    Runtime {
        page_url: runtime.page_url,
        bridge: Bridge { url: runtime.mcp_url, token: runtime.token },
        pid: runtime.pid,
        reattached: runtime.reattached,
        node: runtime.node,
        job: runtime.job,
        version,
    }
}

/// The same connection to a server: a change in anything else (its name, its
/// idle stop) keeps the line open.
fn same_connection(a: &Server, b: &Server) -> bool {
    (&a.ssh_host, a.port, &a.julia, &a.cluster) == (&b.ssh_host, b.port, &b.julia, &b.cluster)
}

/// What answers for a host's runtime: This Mac's helper, or a server's line.
#[derive(Clone)]
pub enum Helper {
    Local(Arc<Channel>),
    Line(Arc<client::Session>),
}

impl Helper {
    /// Ask about the host's files. Blocks: call it off the main thread.
    pub fn files(&self, request: Request) -> Result<Reply, String> {
        match self {
            Helper::Local(channel) => channel.files(request),
            Helper::Line(session) => session.files(request, Duration::ZERO).unwrap_or_else(|| Err(session.status().state.error().map_or_else(|| "Endeavor isn't connected to this server yet.".to_owned(), str::to_owned))),
        }
    }

    /// Stop Julia there, or the start under way. Blocks.
    pub fn stop(&self) -> Result<(), String> {
        match self {
            Helper::Local(channel) => channel.stop(),
            Helper::Line(session) => session.stop(),
        }
    }
}

fn report_until_closed(tx: &futures::channel::mpsc::UnboundedSender<Update>, result: Result<(Arc<Channel>, Hello), String>) {
    let channel = result.as_ref().ok().map(|(channel, _)| channel.clone());
    let result = result.map(|(channel, hello)| (Some(channel), hello));
    let _ = tx.unbounded_send(Update::Connected(result));
    if let Some(notice) = channel.and_then(|channel| channel.closed()) {
        let _ = tx.unbounded_send(Update::Notice(notice));
    }
}

/// Of the open notebooks (`list_notebooks` shape), the ones the user has let
/// run, with each file's modification time now.
fn running_files(bridge: &Bridge, notebooks: &serde_json::Value) -> Vec<(String, Option<f64>)> {
    let allowed = notebooks.as_array().into_iter().flatten().filter(|nb| nb["execution_allowed"] == true);
    allowed.filter_map(|nb| nb["path"].as_str()).filter_map(|path| Some((path.to_owned(), Some(pluto::file_info(bridge, path).ok()??)))).collect()
}

/// A new `Connection::id`.
fn next_connect_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The sidebar's status while This Mac's Julia is down; why shows where it's
/// down (the new-session screen's notice, the notebook pane).
const JULIA_NOT_RUNNING: &str = "Julia not running";

/// Failures in a row before Repair shows beside Retry: the first, and Retry twice.
const REPAIR_AFTER: u32 = 2;

/// What Repair does, in a line (Settings, and beside This Mac's error).
pub const REPAIR_NOTE: &str = "Repair stops Julia, clears its saved state and compiled runtime, and starts it again. Notebooks, packages and settings stay.";

/// An action offered beside a connection's error.
#[derive(Clone, Debug, PartialEq)]
pub enum Fix {
    Repair,
    Download,
    CopyCommand(String),
}

impl Fix {
    pub fn id(&self) -> &'static str {
        match self {
            Fix::Repair => "fix-repair",
            Fix::Download => "fix-download",
            Fix::CopyCommand(_) => "fix-copy",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Fix::Repair => "Repair",
            Fix::Download => "Download Endeavor",
            Fix::CopyCommand(_) => "Copy command",
        }
    }
}

/// How long before a cluster job's end the chat warns: 15 minutes, or in a
/// debug build `ENDEAVOR_JOB_WARNING_SECONDS` (to try it on a short job).
fn job_warning_window() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(secs) = std::env::var("ENDEAVOR_JOB_WARNING_SECONDS").ok().and_then(|v| v.parse().ok()) {
        return Duration::from_secs(secs);
    }
    Duration::from_secs(15 * 60)
}

/// Elapsed time as "0:12" or "1:02:03".
pub fn elapsed(since: Instant) -> String {
    let s = since.elapsed().as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

const WALKER_R: f32 = 22.;

/// The starting pane: the slowly walking turtle, the current step with its
/// elapsed time, and the short step log. No progress bar.
fn starting_pane(steps: &Steps) -> Div {
    let row = |mark: AnyElement, text: String, color: Rgba| {
        div().flex().items_center().gap(px(10.)).child(div().w(px(12.)).flex().justify_center().child(mark)).child(div().text_color(color).child(text))
    };
    let done = steps.done.iter().map(|text| row(div().text_color(theme::text_muted()).child("✓").into_any_element(), text.clone(), theme::text_secondary()));
    let current = row(div().size(px(6.)).rounded_full().bg(theme::accent()).into_any_element(), steps.current.clone(), theme::text_primary());
    let pending = steps.pending().into_iter().map(|text| row(div().into_any_element(), text.to_owned(), theme::text_faint()));
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(14.))
        .child(walker().w(px(WALKER_R * 2.6)).h(px(WALKER_R * 1.3)))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(4.))
                .child(div().text_color(theme::text_primary()).child(steps.current.clone()))
                .child(div().text_color(theme::text_faint()).child(format!("· {}", elapsed(steps.since)))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .text_size(theme::size_meta())
                .children(done)
                .child(current)
                .children(pending),
        )
        .children(steps.detail.clone().map(|d| {
            div().max_w(px(420.)).overflow_hidden().whitespace_nowrap().text_ellipsis().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(d)
        }))
}

/// The turtle walking slowly in place; standing still under Reduce motion.
fn walker() -> Canvas<()> {
    canvas(
        |_, _, _| (),
        |bounds, _, window, cx| {
            let t = if cx.reduce_motion() {
                0.
            } else {
                window.request_animation_frame();
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0., |d| (d.as_millis() % 3_600_000) as f32 / 1000.)
            };
            let phase = t * std::f32::consts::TAU / 1.6;
            let moving = if cx.reduce_motion() { 0. } else { 1. };
            let pose = Pose {
                bob: -0.03 * phase.sin().abs() * moving,
                blink: turtle::blink_at(t, 3.3, 0.9),
                breathe: 0.018 * (t * std::f32::consts::TAU / 3.4).sin(),
                ..Pose::default()
            }
            .walk(phase, 0.1 * moving, 0.12 * moving);
            turtle::paint(window, point(bounds.left() + px(WALKER_R * 1.1), bounds.bottom()), WALKER_R, &pose);
        },
    )
}

/// The status line once Julia is back after Restart Julia's stop failed with `why`.
fn not_restarted(why: &str) -> String {
    format!("Julia didn't restart, so Endeavor is still using the Julia that was running. {}", stop_reason(why))
}

/// The status line when Stop failed on `host` with `why`.
fn not_stopped(host: &str, why: &str) -> String {
    match why.strip_prefix("Julia was not stopped: ") {
        Some(rest) => format!("Julia on {host} was not stopped: {rest}"),
        None => format!("Couldn't stop Julia on {host}: {why}"),
    }
}

/// The helper's "Julia was not stopped: <why>" as just the why, with a capital.
fn stop_reason(why: &str) -> String {
    let rest = why.strip_prefix("Julia was not stopped: ").unwrap_or(why);
    let mut chars = rest.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Change, Connection, LineState, LineStatus, Status, Steps, changes, client, elapsed, not_restarted, not_stopped, percent};

    fn line(state: LineState, node: &str) -> LineStatus {
        let hello = Some(client::HelloInfo { node: node.into(), home: "/home/me".into(), ..Default::default() });
        LineStatus { machine: "lab".into(), name: "lab".into(), state, step: None, hello, job: None }
    }

    fn runtime(pid: u32) -> client::RuntimeInfo {
        client::RuntimeInfo {
            port: 1,
            token: "t".into(),
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            page_url: "http://127.0.0.1:1/?token=t".into(),
            node: "lab".into(),
            pid,
            reattached: false,
            job: None,
            remote_port: None,
            build: None,
            interface: None,
        }
    }

    #[test]
    fn a_servers_line_says_what_its_connects_and_starts_always_said() {
        let connecting = line(LineState::Connecting, "");
        let connected = line(LineState::Connected, "lab");
        let starting = line(LineState::Starting { queue: None }, "lab");
        let ready = line(LineState::Ready(runtime(7)), "lab");
        assert!(changes(&connecting, &line(LineState::Connecting, "")).is_empty());
        assert!(matches!(&changes(&connecting, &connected)[..], [Change::Connected(h)] if h.node == "lab"));
        assert_eq!(changes(&connecting, &line(LineState::Failed("no route".into()), "")), [Change::ConnectFailed("no route".into())]);
        assert_eq!(changes(&connected, &starting), [Change::Starting]);
        assert_eq!(changes(&starting, &ready), [Change::Started(runtime(7))]);
        assert_eq!(changes(&connected, &ready), [Change::Starting, Change::Started(runtime(7))], "a start seen only once it was ready");
        assert_eq!(changes(&starting, &line(LineState::Failed("no Julia".into()), "lab")), [Change::StartFailed("no Julia".into())]);
        assert_eq!(changes(&starting, &connected), [Change::Stopped], "the app's cancel");
        assert_eq!(changes(&ready, &connected), [Change::Stopped], "the app's stop");
    }

    #[test]
    fn a_servers_julia_that_goes_by_itself_is_told_apart_from_a_takeover_and_a_drop() {
        let ready = line(LineState::Ready(runtime(7)), "lab");
        let mut died = line(LineState::NothingRunning, "lab");
        died.step = Some("Julia on lab stopped. It ran out of memory.".into());
        assert_eq!(changes(&ready, &died), [Change::Died("It ran out of memory.".into())]);
        let taken = line(LineState::Failed("Another connection took Julia on lab over.".into()), "lab");
        assert_eq!(changes(&ready, &taken), [Change::Replaced]);
        // A drop keeps the hello until the next connect begins.
        let mut dropped = line(LineState::Connecting, "lab");
        dropped.step = Some("Lost the connection to lab; connecting again".into());
        assert_eq!(changes(&ready, &dropped), [Change::Lost("Lost the connection to lab; connecting again".into())]);
        assert!(changes(&dropped, &line(LineState::Connecting, "")).is_empty(), "nor while it tries again");
        // Back, it goes on with Julia by itself.
        let back = changes(&line(LineState::Connecting, ""), &line(LineState::Ready(runtime(7)), "lab"));
        assert!(matches!(&back[..], [Change::Connected(_), Change::Starting, Change::Started(r)] if r.pid == 7));
    }

    #[test]
    fn a_stop_that_fails_says_what_still_runs() {
        assert_eq!(
            not_restarted("Julia was not stopped: it is still running."),
            "Julia didn't restart, so Endeavor is still using the Julia that was running. It is still running."
        );
        assert_eq!(
            not_restarted("Endeavor's helper didn't answer in 60 s, so Julia may not have stopped."),
            "Julia didn't restart, so Endeavor is still using the Julia that was running. Endeavor's helper didn't answer in 60 s, so Julia may not have stopped."
        );
        assert_eq!(not_stopped("lab", "Julia was not stopped: it is still running."), "Julia on lab was not stopped: it is still running.");
        assert_eq!(not_stopped("lab", "The connection to Endeavor's helper closed."), "Couldn't stop Julia on lab: The connection to Endeavor's helper closed.");
    }
    use std::time::{Duration, Instant};

    #[test]
    fn steps_follow_a_server_start() {
        let mut steps = Steps::new("Connecting to lab");
        steps.advance("Connected to lab", "Finding Julia");
        steps.log("Downloading Julia 1.12.6… 42%");
        assert_eq!(steps.current, "Downloading Julia 1.12.6…");
        assert_eq!(steps.pending(), ["Start Julia", "Open notebook"]);
        steps.found_julia = true;
        steps.advance("Julia 1.12.6", "Starting Julia");
        steps.log("│   Precompiling Pluto");
        assert_eq!((steps.current.as_str(), steps.detail.as_deref()), ("Installing packages", Some("Precompiling Pluto")));
        steps.log("[ Info: Runtime ready");
        assert_eq!(steps.detail.as_deref(), Some("[ Info: Runtime ready"));
        assert_eq!(steps.done, ["Connected to lab", "Julia 1.12.6"]);
        assert_eq!(percent("Downloading Julia 1.12.6… 42%"), Some("42%"));
    }

    #[test]
    fn a_server_out_of_reach_is_tried_again_while_a_session_waits_for_it() {
        let connecting = |start: bool| {
            let mut c = Connection::new(1, Status::Connecting, Steps::new("Connecting to lab"));
            c.start_when_connected = start;
            c
        };
        let timed_out = "ssh: connect to host lab port 22: Operation timed out";

        let mut reopening = connecting(true);
        assert!(reopening.keep_trying(timed_out, true));
        assert!(reopening.waiting_to_reconnect());
        assert!(reopening.lost.as_ref().unwrap().had_julia(), "Julia starts once it's back");

        let mut browsing = connecting(false);
        assert!(browsing.keep_trying(timed_out, true));
        assert!(!browsing.lost.as_ref().unwrap().had_julia());

        let mut nobody_waits = connecting(true);
        assert!(!nobody_waits.keep_trying(timed_out, false));
        assert!(nobody_waits.lost.is_none() && nobody_waits.status == Status::Connecting);

        let mut refused_sign_in = connecting(true);
        assert!(!refused_sign_in.keep_trying("lab: Permission denied (publickey).", true));
        assert!(refused_sign_in.lost.is_none());
    }

    #[test]
    fn elapsed_reads_like_a_clock() {
        let ago = |s: u64| Instant::now() - Duration::from_secs(s);
        assert_eq!(elapsed(ago(12)), "0:12");
        assert_eq!(elapsed(ago(62)), "1:02");
        assert_eq!(elapsed(ago(3723)), "1:02:03");
    }
}
