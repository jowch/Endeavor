use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use endeavor_mcp::client::{self, SessionEvent, State as LineState, Status as LineStatus};
use futures::StreamExt;
use gpui::prelude::FluentBuilder as _;
use gpui::*;

use wire::files::{Reply, Request, RuntimeState};
use wire::backend::Backend;
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
use crate::theme::TextButton as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    Browsing,
    Starting,
    Ready,
    Died(String),
    Failed(String),
}

pub struct Connection {
    pub status: Status,
    id: u64,
    pub channel: Option<Arc<Channel>>,
    linked: bool,
    pub hello: Option<Hello>,
    pub runtime: Option<Runtime>,
    start_when_connected: bool,
    pub job_request: Option<JobRequest>,
    cancelling: bool,
    stop_when_connected: bool,
    start_after_stop: bool,
    check: u64,
    pub stopping: bool,
    pub found: Option<Result<RuntimeState, String>>,
    pub job: Option<String>,
    pub queue_reason: Option<String>,
    pub steps: Steps,
    pub last_notebooks: Vec<(String, String)>,
    pub resume: Vec<(String, Option<f64>)>,
    pub failures: u32,
    pub not_restarted: Option<String>,
    pub notebooks: serde_json::Value,
    pub cells: serde_json::Value,
    pub idle_stopped: HashSet<String>,
    pub engine_ended: pluto::Endings,
    watching: Arc<AtomicU64>,
    pub lost: Option<Lost>,
    dropped: Option<(u32, String)>,
    pub crashed: bool,
    pub older: Option<crate::older_runtime::Version>,
    pub julia: Option<Steps>,
    julia_waits: u32,
    pub julia_failed: Option<String>,
    pub julia_status: Option<pluto::JuliaStatus>,
    pub julia_started: Option<pluto::Started>,
    pub r: Option<Steps>,
    r_waits: u32,
    pub r_failed: Option<String>,
    pub r_status: Option<pluto::RStatus>,
    pub r_started: Option<pluto::Started>,
}

#[derive(Clone)]
pub struct Lost {
    page: Option<String>,
    was: Status,
}

impl Lost {
    fn had_julia(&self) -> bool {
        matches!(self.was, Status::Starting | Status::Ready)
    }
}

const DOWNLOADING_UV: &str = "Downloading uv";

#[derive(Clone, Debug)]
pub struct Steps {
    pub done: Vec<String>,
    pub current: String,
    pub since: Instant,
    pub detail: Option<String>,
    found_julia: bool,
    for_r: bool,
}

impl Steps {
    fn new(current: impl Into<String>) -> Steps {
        Steps { done: Vec::new(), current: current.into(), since: Instant::now(), detail: None, found_julia: false, for_r: false }
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

    pub fn pending(&self) -> Vec<&'static str> {
        let mut left = Vec::new();
        if self.current == pluto::OPENING {
            return left;
        }
        if !self.for_r && !self.current.starts_with("Starting Julia") {
            left.push("Start Julia");
        }
        left.push("Open notebook");
        left
    }

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

impl Steps {
    fn opening(&mut self) {
        if self.current == pluto::OPENING {
            return;
        }
        let done = self.current.trim_end_matches('…').replacen("Starting", "Started", 1).replacen("Installing", "Installed", 1);
        self.advance(done, pluto::OPENING);
        self.detail = Some(if self.for_r { "The first notebook can take a minute to open." } else { "The first notebook after Julia starts takes a while to compile." }.into());
    }

    fn first(step: &str, start: impl FnOnce() -> Steps) -> Steps {
        let mut steps = start();
        if step == pluto::OPENING {
            steps.now(pluto::OPENING);
        }
        steps
    }

    fn r() -> Steps {
        Steps { for_r: true, ..Steps::new("Starting R") }
    }

    fn r_step(&mut self, step: &str) {
        if step == pluto::OPENING {
            return self.opening();
        }
        let next = format!("{step}…");
        if next == self.current {
            return;
        }
        if self.current.starts_with("Installing") {
            let done = self.current.trim_end_matches('…').replacen("Installing", "Installed", 1);
            self.advance(done, next);
        } else {
            self.now(next);
        }
        self.detail = step.starts_with("Installing").then(|| "The first time takes a few minutes.".into());
    }

    fn julia(&mut self, step: &str) {
        if step == pluto::OPENING {
            return self.opening();
        }
        if step.starts_with("Downloading Julia") || step.starts_with("Checking Julia") || step.starts_with("Unpacking Julia") {
            return self.log(step);
        }
        match step.strip_suffix(" is starting and loading Pluto") {
            Some(julia) => {
                if !self.current.starts_with("Starting Julia") && self.current.ends_with("Julia…") {
                    let done = self.current.trim_end_matches('…').replace("Downloading", "Downloaded").replace("Unpacking", "Unpacked").replace("Checking", "Checked");
                    self.advance(done, format!("Starting {julia} and Pluto"));
                } else {
                    self.now(format!("Starting {julia} and Pluto"));
                }
                self.detail = Some("The first start installs Pluto's packages.".into());
            }
            None if step == "Julia is starting" => self.now("Starting Julia"),
            None => self.detail = Some(step.to_owned()),
        }
    }
}

impl Steps {
    fn julia_status(&mut self, step: &str, quiet: u64) {
        self.julia(step);
        if quiet >= 120 {
            self.detail = Some(format!("No sign of progress for {} minutes.", quiet / 60));
        }
    }
}

fn percent(line: &str) -> Option<&str> {
    line.rsplit(' ').next().filter(|w| w.ends_with('%'))
}

enum Update {
    Event(Event),
    Local(Progress),
    Connected(Result<(Option<Arc<Channel>>, Hello), String>),
    Started(Result<Runtime, String>),
    Notice(Notice),
}

impl Connection {
    fn r_status_now(&mut self, status: pluto::RStatus) -> bool {
        if self.r_status.as_ref() == Some(&status) {
            return false;
        }
        match &status {
            pluto::RStatus::Starting { step, .. } => {
                self.r_failed = None;
                self.r.get_or_insert_with(Steps::r).r_step(step);
            }
            other => {
                if self.r_waits == 0 {
                    self.r = None;
                }
                if matches!(other, pluto::RStatus::Ready) {
                    self.r_failed = None;
                }
                if let Some(failure) = r_failure(self.r_status.as_ref(), other) {
                    self.r_failed = Some(failure);
                }
            }
        }
        self.r_status = Some(status);
        true
    }

    fn waits(&mut self, kind: Backend) -> &mut u32 {
        match kind {
            Backend::Pluto => &mut self.julia_waits,
            Backend::Ember | Backend::Marimo => &mut self.r_waits,
        }
    }

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
            queue_reason: None,
            steps,
            last_notebooks: Vec::new(),
            resume: Vec::new(),
            failures: 0,
            not_restarted: None,
            notebooks: serde_json::Value::Null,
            cells: serde_json::Value::Null,
            idle_stopped: HashSet::new(),
            engine_ended: HashSet::new(),
            watching: Arc::default(),
            older: None,
            julia: None,
            julia_waits: 0,
            julia_failed: None,
            julia_status: None,
            julia_started: None,
            r: None,
            r_waits: 0,
            r_failed: None,
            r_status: None,
            r_started: None,
            lost: None,
            dropped: None,
            crashed: false,
        }
    }

    pub fn bridge(&self) -> Option<Bridge> {
        self.runtime.as_ref().map(|r| r.bridge.clone())
    }

    fn connected(&self) -> bool {
        self.channel.is_some() || self.linked
    }

    fn lose(&mut self, reason: String, shown: bool) {
        let was = self.status.clone();
        let gone = self.forget_runtime();
        let page = gone.as_ref().and_then(|r| r.page_url.split_once('?')).map(|(origin, _)| origin.to_owned()).filter(|_| shown);
        self.dropped = gone.filter(|_| was == Status::Ready).map(|r| (r.pid, r.node));
        self.unlink();
        self.status = Status::Failed(reason);
        self.lost = Some(Lost { page, was });
    }

    fn unlink(&mut self) {
        self.channel = None;
        self.linked = false;
    }

    pub fn waiting_to_reconnect(&self) -> bool {
        self.lost.is_some() && matches!(self.status, Status::Failed(_))
    }

    fn keep_trying(&mut self, error: &str, session_waits: bool) -> bool {
        if !crate::offline::unreachable(error) || (self.lost.is_none() && !session_waits) {
            return false;
        }
        let was = if self.start_when_connected { Status::Starting } else { Status::Browsing };
        self.lost.get_or_insert(Lost { page: None, was });
        self.status = Status::Failed(error.to_owned());
        true
    }

    fn back_to_a_queue(&mut self) -> Option<String> {
        self.lost.take().and_then(|lost| lost.page)
    }

    fn same_julia(&self, runtime: &Runtime) -> bool {
        let ready = self.runtime.as_ref().filter(|_| self.status == Status::Ready).map(|r| (r.pid, &r.node));
        let was = ready.or(self.dropped.as_ref().map(|(pid, node)| (*pid, node)));
        was.is_some_and(|(pid, node)| pid == runtime.pid && *node == runtime.node)
    }

    #[must_use]
    fn forget_runtime(&mut self) -> Option<Runtime> {
        self.watching.fetch_add(1, Ordering::SeqCst);
        self.julia = None;
        self.julia_failed = None;
        self.julia_status = None;
        self.julia_started = None;
        self.r = None;
        self.r_failed = None;
        self.r_status = None;
        self.r_started = None;
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

    pub fn bridge(&self, host: &HostId) -> Option<Bridge> {
        self.connections.get(host).and_then(Connection::bridge)
    }

    pub fn crashed_host(&self) -> Option<String> {
        let mut crashed: Vec<&HostId> = self.connections.iter().filter(|(_, c)| c.crashed && matches!(c.status, Status::Died(_))).map(|(h, _)| h).collect();
        crashed.sort_by_key(|h| **h != HostId::ThisMac);
        crashed.first().map(|host| match host {
            HostId::ThisMac => crate::platform::this_computer!().to_string(),
            host => self.hosts.name(host),
        })
    }

    pub fn session_bridge(&self, key: u64) -> Option<Bridge> {
        let session = self.sessions.iter().find(|s| s.key == key)?;
        self.bridge(&session.place.host)
    }

    pub(crate) fn note_session_kind(&self, host: &HostId, key: u64, kind: Backend) {
        if let Some(listener) = self.listeners.get(host) {
            listener.session_kind(key, kind);
        }
    }

    fn close_page(&mut self, gone: Option<Runtime>, cx: &mut Context<Self>) {
        let Some((origin, _)) = gone.as_ref().and_then(|r| r.page_url.split_once('?')) else { return };
        self.blank_page(origin, cx);
    }

    fn blank_page(&mut self, origin: &str, cx: &mut Context<Self>) {
        let shown = crate::webcontent::url(self.webview.read(cx).raw());
        if !origin.is_empty() && shown.starts_with(origin) {
            self.webview.update(cx, |w, _| w.load_url("about:blank"));
        }
    }

    fn shows_page_of(&self, host: &HostId, cx: &App) -> bool {
        let Some((origin, _)) = self.connections.get(host).and_then(|c| c.runtime.as_ref()).and_then(|r| r.page_url.split_once('?')) else { return false };
        crate::webcontent::url(self.webview.read(cx).raw()).starts_with(origin)
    }

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
        listener.set_pictures(crate::cell_shot::snapshot(host.clone(), self.cell_shots.clone()));
        listener.set_restart(crate::restart_ask::hook(host.clone(), self.restart_asks.clone()));
        self.listeners.insert(host.clone(), listener.clone());
        Ok(listener)
    }

    pub fn connect_host(&mut self, host: &HostId, start: bool, cx: &mut Context<Self>) {
        if self.missing_files.is_some() {
            return;
        }
        match self.connections.get_mut(host).map(|c| (c.status.clone(), c)) {
            Some((Status::Connecting, connection)) => {
                connection.start_when_connected |= start;
                return;
            }
            Some((Status::Failed(_), _)) | None => {}
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
            HostId::ThisMac => Steps::new("Starting the notebook runtime"),
            HostId::Server(_) => Steps::new(format!("Connecting to {name}")),
        };
        let mut connection = Connection::new(id, Status::Connecting, first);
        connection.start_when_connected = start;
        if let Some(old) = self.connections.remove(host) {
            connection.last_notebooks = old.last_notebooks;
            connection.resume = old.resume;
            connection.failures = old.failures;
            connection.lost = old.lost;
            connection.dropped = old.dropped;
        }
        self.connections.insert(host.clone(), connection);
        match host {
            HostId::ThisMac => {
                let (tx, rx) = futures::channel::mpsc::unbounded::<Update>();
                self.settings_checks.local_julia = Some(self.settings.julia_preference());
                self.check_users_julia(cx);
                self.settings_checks.local_r = Some(self.settings.r.clone());
                std::thread::spawn(move || {
                    let result = runtime::connect().map(|(channel, hello)| (Arc::new(channel), hello));
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

    fn open_line(&mut self, host: &HostId, server: &Server, cx: &mut Context<Self>) -> Result<(), String> {
        let mut socket = None;
        if let Some(line) = self.lines.get(host) {
            if same_connection(&line.server, server) {
                line.sign_in.forget();
                let session = line.session.clone();
                let status = session.status();
                if matches!(status.state, LineState::Failed(_)) && !signed_in(&status) {
                    session.request(&client::Want::Attach { install: true });
                } else if !signed_in(&status) {
                    session.retry_now();
                }
                let status = session.status();
                let id = self.connections.get(host).map_or(0, |c| c.id);
                for change in changes(&LineStatus { state: LineState::Connecting, hello: None, ..status.clone() }, &status) {
                    self.on_change(host, id, change, cx);
                }
                return Ok(());
            }
            socket = line.session.hand_over_port();
            self.close_line(host, cx);
        }
        let (tx, rx) = futures::channel::mpsc::unbounded::<LineUpdate>();
        let sign_in = remote::SignIn::default();
        let (asker, auth) = remote::asker(server, self.questions_tx.clone(), sign_in.clone())?;
        let events = tx.clone();
        let session = Arc::new(remote::open(server, auth, socket, Box::new(move |event| drop(events.unbounded_send(LineUpdate::Event(event)))))?);
        let closed: Arc<AtomicBool> = Arc::default();
        std::thread::spawn({
            let (session, closed) = (session.clone(), closed.clone());
            move || {
                let mut last = session.status();
                let _ = tx.unbounded_send(LineUpdate::Status(Box::new(last.clone()), Instant::now()));
                while !closed.load(Ordering::SeqCst) {
                    let now = session.wait_for(Duration::from_secs(1), |now| *now != last);
                    if now != last && !closed.load(Ordering::SeqCst) {
                        if tx.unbounded_send(LineUpdate::Status(Box::new(now.clone()), Instant::now())).is_err() {
                            break;
                        }
                        last = now;
                    }
                }
            }
        });
        let generation = next_connect_id();
        self.lines.insert(host.clone(), Line { session, _asker: asker, sign_in, server: server.clone(), closed, last: None, generation, held: None });
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

    fn close_line(&mut self, host: &HostId, cx: &mut Context<Self>) {
        if let Some(line) = self.lines.remove(host) {
            let _ = self.drop_asks(line.sign_in.id, None, cx);
            drop(line.close());
        }
    }

    fn on_line(&mut self, host: &HostId, generation: u64, update: LineUpdate, cx: &mut Context<Self>) {
        if !self.lines.get(host).is_some_and(|l| l.generation == generation) {
            return;
        }
        let Some(id) = self.connections.get(host).map(|c| c.id) else { return };
        match update {
            LineUpdate::Event(SessionEvent::Step(event)) => self.on_update(host.clone(), id, Update::Event(event), cx),
            LineUpdate::Event(SessionEvent::Trouble(text)) => eprintln!("{}: {text}", self.hosts.name(host)),
            LineUpdate::Event(_) => {}
            LineUpdate::Status(now, seen) => {
                let Some(line) = self.lines.get_mut(host) else { return };
                let now = *now;
                let last = line.last.replace(now.clone()).unwrap_or(LineStatus { state: LineState::Connecting, hello: None, ..now.clone() });
                let (source, ssh_host) = (line.sign_in.id, line.server.ssh_host.clone());
                let gave_up = try_ended(&last, &now) && self.drop_asks(source, Some(seen), cx);
                for mut change in changes(&last, &now) {
                    match &mut change {
                        Change::ConnectFailed(why) if gave_up => *why = remote::gave_up(&ssh_host),
                        Change::Connected(_) => {
                            if let Some(line) = self.lines.get(host) {
                                line.sign_in.forget();
                            }
                        }
                        _ => {}
                    }
                    if self.hold(host, generation, &change, cx) {
                        continue;
                    }
                    self.on_change(host, id, change, cx);
                }
            }
        }
    }

    fn hold(&mut self, host: &HostId, generation: u64, change: &Change, cx: &mut Context<Self>) -> bool {
        let ready_pid = self.connections.get(host).filter(|c| c.status == Status::Ready).and_then(|c| c.runtime.as_ref()).map(|r| r.pid);
        let Some(line) = self.lines.get_mut(host) else { return false };
        match (&mut line.held, change) {
            (Some(held), Change::Started(runtime)) if runtime.pid == held.pid => {
                line.held = None;
                true
            }
            (Some(held), Change::Connected(_) | Change::Starting) => {
                held.since.push(change.clone());
                true
            }
            (Some(_), _) => {
                self.release(host, generation, cx);
                false
            }
            (None, Change::Lost(why)) => {
                let Some(pid) = ready_pid else { return false };
                let id = next_connect_id();
                line.held = Some(Held { id, pid, why: why.clone(), since: Vec::new() });
                let host = host.clone();
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(LOST_GRACE).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.lines.get(&host).and_then(|l| l.held.as_ref()).is_some_and(|h| h.id == id) {
                            this.release(&host, generation, cx);
                        }
                        this.sync_holds(cx);
                    });
                })
                .detach();
                true
            }
            (None, _) => false,
        }
    }

    fn release(&mut self, host: &HostId, generation: u64, cx: &mut Context<Self>) {
        let Some(held) = self.lines.get_mut(host).filter(|l| l.generation == generation).and_then(|l| l.held.take()) else { return };
        let Some(id) = self.connections.get(host).map(|c| c.id) else { return };
        for change in std::iter::once(Change::Lost(held.why)).chain(held.since) {
            self.on_change(host, id, change, cx);
        }
    }

    fn on_change(&mut self, host: &HostId, id: u64, change: Change, cx: &mut Context<Self>) {
        let update = match change {
            Change::Connected(hello) => Update::Connected(Ok((None, hello_of(&hello)))),
            Change::ConnectFailed(why) => Update::Connected(Err(why)),
            Change::Lost(why) => Update::Notice(Notice::Lost(why)),
            Change::Started(runtime) => Update::Started(Ok(runtime_of(runtime))),
            Change::StartFailed(why) => Update::Started(Err(why)),
            Change::Died(why) => Update::Notice(Notice::Died(why)),
            Change::Stopped => match self.connections.get(host) {
                Some(c) if c.status == Status::Starting => Update::Started(Err(String::new())),
                _ => return,
            },
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

    pub fn start_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
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
        connection.queue_reason = None;
        connection.steps = match host {
            HostId::ThisMac => Steps::new("Starting the notebook runtime"),
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
            (_, _, Some(session)) => session.request(&client::Want::Start { job, install: true }),
            _ => {}
        }
        cx.notify();
    }

    pub fn restart_local(&mut self, cx: &mut Context<Self>) {
        if self.julia_changed() || self.r_changed() {
            return self.reconnect_local(false, cx);
        }
        let host = HostId::ThisMac;
        let Some(connection) = self.connections.get_mut(&host) else { return self.connect_host(&host, true, cx) };
        let Some(channel) = connection.channel.clone().filter(|_| connection.status == Status::Ready) else { return };
        let running = connection.bridge().map(|bridge| (bridge, connection.notebooks.clone()));
        let gone = connection.forget_runtime();
        connection.status = Status::Starting;
        connection.steps = Steps::new("Stopping notebooks");
        let listener = self.listeners.get(&host).cloned();
        self.close_page(gone, cx);
        self.status = "Restarting notebooks…".into();
        let stop = cx.background_executor().spawn(async move {
            let resume = running.map(|(bridge, notebooks)| running_files(&bridge, &notebooks)).unwrap_or_default();
            if let Some(listener) = listener {
                listener.restarting();
            }
            (resume, channel.stop())
        });
        cx.spawn(async move |this, cx| {
            let (resume, stopped) = stop.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(connection) = this.connections.get_mut(&host) {
                    connection.status = Status::Died(String::new());
                    connection.resume = resume;
                    connection.not_restarted = stopped.err();
                }
                this.start_host(&host, cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub fn fixes(&self, host: &HostId, reason: &str) -> Vec<Fix> {
        let repair = *host == HostId::ThisMac && self.connections.get(host).is_some_and(|c| c.failures > REPAIR_AFTER);
        let helper = remote::HelperFix::of(reason).map(|fix| match fix.command() {
            Some(command) => Fix::CopyCommand(command),
            None => Fix::Download,
        });
        repair.then_some(Fix::Repair).into_iter().chain(helper).collect()
    }

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
            .button_text(fix.label())
            .on_click(cx.listener(move |this, _, _, cx| this.apply_fix(&fix, cx)))
    }

    pub fn apply_fix(&mut self, fix: &Fix, cx: &mut Context<Self>) {
        match fix {
            Fix::Repair => self.repair_local(cx),
            Fix::Download => crate::platform::open_url(crate::about::WEBSITE, cx),
            Fix::CopyCommand(command) => {
                cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                self.status = "Copied the command; paste it in Terminal.".into();
                cx.notify();
            }
        }
    }

    pub fn repair_local(&mut self, cx: &mut Context<Self>) {
        self.reconnect_local(true, cx);
    }

    fn reconnect_local(&mut self, clear: bool, cx: &mut Context<Self>) {
        let host = HostId::ThisMac;
        let (channel, running, gone) = match self.connections.get_mut(&host) {
            Some(c) if c.status == Status::Connecting => return,
            Some(c) => {
                let running = c.bridge().map(|bridge| (bridge, c.notebooks.clone()));
                c.id = next_connect_id();
                let gone = c.forget_runtime();
                c.status = Status::Starting;
                c.steps = Steps::new(if clear { "Clearing what Julia saved" } else { "Stopping notebooks" });
                c.stopping = false;
                (c.channel.take(), running, gone)
            }
            None => (None, None, None),
        };
        self.close_page(gone, cx);
        self.settings_checks.repairing = clear;
        self.status = if clear { "Repairing Julia…" } else { "Restarting notebooks…" }.into();
        let work = cx.background_executor().spawn(async move {
            let resume = running.map(|(bridge, notebooks)| running_files(&bridge, &notebooks)).unwrap_or_default();
            if let Some(channel) = channel {
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
                if let Some(page) = connection.back_to_a_queue() {
                    self.blank_page(&page, cx);
                }
            }
            Update::Event(Event::Queued { job, state, reason }) if state == "RUNNING" => {
                connection.job.get_or_insert(job);
                connection.queue_reason = None;
                connection.steps.advance(format!("Got a node: {reason}"), "Starting Julia");
            }
            Update::Event(Event::Queued { job, reason, .. }) => {
                connection.job.get_or_insert(job);
                connection.steps.now("Waiting for a node");
                connection.steps.detail = wire::slurm::reason_text(&reason).map(|r| format!("Slurm: {r}"));
                connection.queue_reason = Some(reason);
                if let Some(page) = connection.back_to_a_queue() {
                    self.blank_page(&page, cx);
                }
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
                let keeps_page = start && connection.lost.as_ref().is_some_and(|l| l.page.is_some());
                let back = if keeps_page { None } else { connection.lost.take() };
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
            Update::Started(Ok(runtime)) if !local && connection.same_julia(&runtime) => {
                let watched = connection.runtime.is_some();
                connection.lost = None;
                connection.dropped = None;
                connection.job = None;
                connection.failures = 0;
                connection.steps.found_julia = true;
                connection.status = Status::Ready;
                connection.runtime = Some(runtime);
                if !watched {
                    self.watch_notebooks(&host, cx);
                    self.warn_before_job_ends(&host, cx);
                }
                let shown = self.active_session().filter(|s| s.place.host == host).and_then(|s| s.notebook.clone());
                let origin = self.connection(&host).and_then(|c| c.runtime.as_ref()).and_then(|r| Some(r.page_url.split_once('?')?.0.to_owned()));
                if let (Some(id), Some(origin)) = (shown, origin)
                    && load_again(&crate::webcontent::url(self.webview.read(cx).raw()), &origin, &self.page, &id)
                {
                    self.load_notebook(&host, &id, cx);
                }
                let waiting: Vec<u64> = self.sessions.iter().filter(|s| s.place.host == host && s.agent_waiting).map(|s| s.key).collect();
                for key in waiting {
                    self.request_agent(key, cx);
                }
            }
            Update::Started(Ok(runtime)) => {
                connection.lost = None;
                connection.dropped = None;
                connection.job = None;
                connection.failures = 0;
                connection.steps.found_julia = true;
                let started = match (runtime.reattached, local) {
                    (true, true) => "The notebook runtime was running".to_owned(),
                    (false, true) => "Started the notebook runtime".to_owned(),
                    (true, false) => format!("Julia running on {}", runtime.node),
                    (false, false) => "Started Julia".to_owned(),
                };
                connection.steps.advance(started, "Opening the notebook");
                connection.status = Status::Ready;
                connection.runtime = Some(runtime);
                self.on_ready(&host, cx);
                self.warn_before_job_ends(&host, cx);
            }
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
                }
                if let Some(listener) = self.listeners.get(&host) {
                    listener.restart_failed();
                }
            }
            Update::Notice(Notice::Lost(_)) if !connection.connected() => {}
            Update::Notice(Notice::Lost(reason)) if !local => {
                connection.lose(reason, shown);
                if self.offline_since().is_none() {
                    self.retry_lost(host.clone(), std::time::Duration::from_secs(2), cx);
                }
            }
            Update::Notice(notice) => {
                let crashed = matches!(notice, Notice::Died(_)).then(|| crate::crash::running_notebooks(&connection.notebooks));
                connection.dropped = None;
                let gone = connection.forget_runtime();
                connection.status = match notice {
                    Notice::Died(reason) => {
                        connection.crashed = true;
                        Status::Died(reason)
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

    pub fn settings_saved(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let HostId::Server(id) = host else { return };
        let (Some(line), Some(server)) = (self.lines.get(host), self.hosts.server(id)) else { return };
        if same_connection(&line.server, server) {
            return;
        }
        let name = self.hosts.name(host);
        let shown = self.shows_page_of(host, cx);
        let Some(connection) = self.connections.get_mut(host) else { return };
        let start = connection.start_when_connected;
        if connection.connected() {
            connection.lose(format!("Connecting to {name} with its new settings"), shown);
        } else if connection.status == Status::Connecting {
            connection.status = Status::Failed(String::new());
        } else {
            return;
        }
        let julia = start || connection.lost.as_ref().is_some_and(Lost::had_julia);
        self.connect_host(host, julia, cx);
    }

    pub fn reconnect_lost(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let julia = self.connections.get(host).and_then(|c| c.lost.as_ref()).is_some_and(Lost::had_julia);
        self.connect_host(host, julia, cx);
    }

    fn on_connected(&mut self, host: &HostId, check: bool, cx: &mut Context<Self>) {
        if check && *host != HostId::ThisMac {
            self.check_host(host, cx);
        }
        self.fetch_partitions(host, cx);
        if self.draft.host == *host {
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

    fn on_ready(&mut self, host: &HostId, cx: &mut Context<Self>) {
        if *host == HostId::ThisMac {
            self.settings_checks.repairing = false;
        }
        let Some(bridge) = self.bridge(host) else { return };
        let reattached = self.connections.get(host).and_then(|c| c.runtime.as_ref()).is_some_and(|r| r.reattached);
        if *host == HostId::ThisMac {
            let agent = self.setup.as_ref().and_then(|s| s.agent);
            if let Some(agent) = agent
                && let Some(commands) = self.links.get_mut(agent).rx.take()
            {
                self.on_progress(Progress::new(Step::Agent, format!("Notebook runtime ready · starting {}…", agent.name())), cx);
                self.start_agent(agent, commands, cx);
            } else if let Some(e) = self.connections.get_mut(host).and_then(|c| c.not_restarted.take()) {
                eprintln!("Restart Julia: {e}");
                self.status = not_restarted(&e).into();
            } else if self.this_mac_was_ready {
                self.status = if reattached { "Reconnected to your notebooks." } else { "Notebooks restarted." }.into();
            } else {
                self.status = "Ready.".into();
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
        let on_host: Vec<&Session> = self.sessions.iter().filter(|s| s.place.host == *host).collect();
        let bound: Vec<(u64, String)> = on_host.iter().filter_map(|s| Some((s.key, s.notebook_path.clone()?))).collect();
        let folders: Vec<(u64, String, Backend)> = on_host.iter().map(|s| (s.key, s.place.path.clone(), s.kind)).collect();
        let policies: Vec<(u64, &'static str, bool)> = on_host.iter().map(|s| (s.key, s.policy(), s.edits_ask())).collect();
        let waiting: Vec<u64> = on_host.iter().filter(|s| s.agent_waiting).map(|s| s.key).collect();
        for (key, path) in bound {
            let bridge = bridge.clone();
            cx.background_executor().spawn(async move { pluto::set_notebook(&bridge, key, &path) }).detach();
        }
        for (key, folder, kind) in folders {
            self.note_session_kind(host, key, kind);
            let bridge = bridge.clone();
            cx.background_executor().spawn(async move { pluto::set_session_folder(&bridge, key, &folder, kind) }).detach();
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

    fn watch_notebooks(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get(host) else { return };
        let Some(bridge) = connection.bridge() else { return };
        let julia_status = connection.runtime.as_ref().is_some_and(|r| r.version.has_julia_status());
        let r_status = connection.runtime.as_ref().is_some_and(|r| r.version.has_r_status());
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<serde_json::Value>();
        let watching = connection.watching.clone();
        let mine = watching.fetch_add(1, Ordering::SeqCst) + 1;
        if julia_status {
            self.watch_julia(host, bridge.clone(), watching.clone(), mine, cx);
        }
        if r_status {
            self.watch_r(host, bridge.clone(), watching.clone(), mine, cx);
        }
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

    fn watch_julia(&mut self, host: &HostId, bridge: Bridge, watching: Arc<AtomicU64>, mine: u64, cx: &mut Context<Self>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<(pluto::JuliaStatus, Option<pluto::Started>)>();
        let current = watching.clone();
        std::thread::spawn(move || {
            while current.load(Ordering::SeqCst) == mine {
                let status = pluto::julia_status(&bridge);
                let starting = matches!(status, Ok((pluto::JuliaStatus::Starting { .. }, _)));
                if let Ok(status) = status
                    && tx.unbounded_send(status).is_err()
                {
                    return;
                }
                std::thread::sleep(Duration::from_secs(if starting { 1 } else { 3 }));
            }
        });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            while let Some(status) = rx.next().await {
                if watching.load(Ordering::SeqCst) != mine {
                    break;
                }
                if this.update(cx, |this, cx| this.on_julia_status(&host, status.0, status.1, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_julia_status(&mut self, host: &HostId, status: pluto::JuliaStatus, started: Option<pluto::Started>, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get_mut(host) else { return };
        if connection.julia_started != started {
            connection.julia_started = started;
            cx.notify();
        }
        if connection.julia_status.as_ref() == Some(&status) {
            return;
        }
        let was_starting = matches!(connection.julia_status, Some(pluto::JuliaStatus::Starting { .. }));
        let failure = new_failure(connection.julia_status.as_ref(), &status).map(str::to_owned);
        match &status {
            pluto::JuliaStatus::Starting { step, quiet } => {
                connection.julia_failed = None;
                connection.julia.get_or_insert_with(|| Steps::new("Starting Julia")).julia_status(step, *quiet);
            }
            other => {
                if connection.julia_waits == 0 {
                    connection.julia = None;
                }
                if matches!(other, pluto::JuliaStatus::Ready) {
                    connection.julia_failed = None;
                }
                if failure.is_some() {
                    connection.julia_failed = failure;
                }
            }
        }
        let ended = was_starting && !matches!(status, pluto::JuliaStatus::Starting { .. });
        connection.julia_status = Some(status);
        if ended && *host == HostId::ThisMac && self.settings_panel.is_some() {
            self.check_own_julia(cx);
        }
        cx.notify();
    }

    fn watch_r(&mut self, host: &HostId, bridge: Bridge, watching: Arc<AtomicU64>, mine: u64, cx: &mut Context<Self>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<(pluto::RStatus, Option<pluto::Started>)>();
        let current = watching.clone();
        std::thread::spawn(move || {
            while current.load(Ordering::SeqCst) == mine {
                let status = pluto::r_status(&bridge);
                let starting = matches!(status, Ok((pluto::RStatus::Starting { .. }, _)));
                if let Ok(status) = status
                    && tx.unbounded_send(status).is_err()
                {
                    return;
                }
                std::thread::sleep(Duration::from_secs(if starting { 1 } else { 3 }));
            }
        });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            while let Some(status) = rx.next().await {
                if watching.load(Ordering::SeqCst) != mine {
                    break;
                }
                if this.update(cx, |this, cx| this.on_r_status(&host, status.0, status.1, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_r_status(&mut self, host: &HostId, status: pluto::RStatus, started: Option<pluto::Started>, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get_mut(host) else { return };
        let moved = connection.r_started != started;
        connection.r_started = started;
        if connection.r_status_now(status) || moved {
            cx.notify();
        }
    }

    fn on_notebooks_event(&mut self, host: &HostId, mut event: serde_json::Value, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get_mut(host) else { return };
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
        let (new_ended, ended) = pluto::new_endings(&connection.engine_ended, &event);
        connection.engine_ended = ended;
        let list = event["notebooks"].take();
        if list.as_array().is_some_and(|l| l.iter().any(|nb| nb["path"].as_str().is_some_and(|p| p.ends_with(".jl")))) {
            connection.julia_failed = None;
        }
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
        let now_running = pluto::newly_allowed(&connection.notebooks, &list);
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
        let python_pages: Vec<u64> = self.sessions.iter().filter(|s| s.place.host == *host && s.kind == Backend::Marimo && s.notebook_path.as_ref().is_some_and(|p| now_running.contains(p))).map(|s| s.key).collect();
        for key in python_pages {
            self.reload_notebook(key, cx);
        }
        for (path, cell) in exits {
            self.notebook_stopped((host.clone(), path), cell, cx);
        }
        for (path, language, why) in new_ended {
            let path = self.sessions.iter().filter(|s| s.place.host == *host).filter_map(|s| s.notebook_path.as_deref()).find(|own| same_file(host, own, &path)).map_or(path.clone(), str::to_owned);
            let at = (host.clone(), path);
            self.crashes.ended(at.clone(), language, why);
            self.notebook_stopped(at, None, cx);
        }
    }

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
        let steps = [self.julia_steps(host, Backend::Pluto, cx), self.julia_steps(host, Backend::Ember, cx)];
        let reopen = cx.background_executor().spawn(async move {
            let failed = std::cell::RefCell::new([None, None]);
            let unexplained = std::cell::RefCell::new(Vec::new());
            let list = || pluto::call_tool(&bridge, "list_notebooks", serde_json::json!({})).ok();
            let find = |listed: &Option<serde_json::Value>, path: &str| listed.as_ref()?.as_array()?.iter().find(|nb| nb["path"] == path).cloned();
            let listed = list();
            let reopened = paths
                .into_iter()
                .filter_map(|path| {
                    let result = match find(&listed, &path) {
                        Some(nb) => nb,
                        None => {
                            let run = resume.iter().any(|(p, modified)| *p == path && modified.is_none_or(|m| pluto::file_info(&bridge, &path) == Ok(Some(m))));
                            let r = crate::new_session::kind_of_path(&path) == Backend::Ember;
                            match pluto::until_julia(|| pluto::open_notebook(&bridge, &path, run), &|step| steps[r as usize].step(step)) {
                                Ok(nb) => nb,
                                Err(e) => find(&list(), &path).or_else(|| {
                                    eprintln!("Couldn't reopen {path}: {e}");
                                    if pane_failure(crate::new_session::kind_of_path(&path), &e).is_some() {
                                        failed.borrow_mut()[r as usize].get_or_insert(e);
                                    } else {
                                        unexplained.borrow_mut().push((path.clone(), e));
                                    }
                                    None
                                })?,
                            }
                        }
                    };
                    let previewed = result["execution_allowed"] == false;
                    Some((result["notebook_id"].as_str()?.to_owned(), path, previewed))
                })
                .collect::<Vec<_>>();
            (reopened, failed.into_inner(), unexplained.into_inner())
        });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            let (reopened, [julia_failed, r_failed], unexplained) = reopen.await;
            let _ = this.update(cx, |this, cx| {
                this.julia_answer(&host, Backend::Pluto, julia_failed.as_deref(), cx);
                this.julia_answer(&host, Backend::Ember, r_failed.as_deref(), cx);
            });
            let previewed = reopened.iter().filter(|(.., previewed)| *previewed).count();
            let reopened: Vec<(String, String)> = reopened.into_iter().map(|(id, path, _)| (id, path)).collect();
            let _ = this.update(cx, |this, cx| {
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
                    if own.is_some() {
                        session.open_failed = None;
                    }
                    if let Some(path) = own.filter(|_| session.notebook.is_none()) {
                        let failed = unexplained.iter().find(|(p, _)| p == path).cloned();
                        session.open_failed = failed.clone();
                        lost.push((session.key, failed));
                    }
                }
                if this.notice.as_ref().is_some_and(|n| matches!(&n.retry, Some(crate::notice::Retry::OpenNotebook(k, _)) if this.sessions.iter().any(|s| s.key == *k && s.place.host == host && s.open_failed.is_none()))) {
                    this.notice = None;
                }
                for (key, failed) in lost {
                    this.check_missing(key, failed, cx);
                }
                if let Some((host, id)) = this.active_session().and_then(|s| Some((s.place.host.clone(), s.notebook.clone()?))) {
                    this.load_notebook(&host, &id, cx);
                }
                if host == HostId::ThisMac && !reopened.is_empty() && !reattached {
                    this.status = match previewed {
                        0 => format!("Notebooks restarted; {} reopened.", reopened.len()),
                        n => format!("Notebooks restarted; {} reopened, {n} in safe preview.", reopened.len()),
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

    pub fn send_idle_limit(&self, host: &HostId, cx: &mut Context<Self>) {
        let Some(bridge) = self.bridge(host) else { return };
        let limit = match host {
            HostId::Server(id) => self.hosts.server(id).and_then(|s| s.idle_stop),
            HostId::ThisMac => None,
        };
        let hours = f64::from(limit.unwrap_or(self.settings.idle_stop).hours());
        #[cfg(debug_assertions)]
        let hours = std::env::var("ENDEAVOR_TEST_IDLE_SECS").ok().and_then(|s| s.parse::<f64>().ok()).map_or(hours, |secs| secs / 3600.0);
        cx.background_executor().spawn(async move { pluto::set_idle_limit(&bridge, hours) }).detach();
    }

    pub fn job_end(&self, host: &HostId) -> Option<(u64, bool)> {
        let connection = self.connections.get(host).filter(|c| c.status == Status::Ready)?;
        let at = connection.runtime.as_ref()?.job.as_ref()?.ends_at?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Some((at, at.saturating_sub(now) <= job_warning_window().as_secs()))
    }

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

    pub fn cancel_start(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(helper) = self.helper(host) else { return };
        let Some(connection) = self.connections.get_mut(host).filter(|c| c.status == Status::Starting) else { return };
        connection.cancelling = true;
        connection.steps.now("Cancelling the job");
        cx.background_executor().spawn(async move { helper.stop().map_err(|e| eprintln!("Cancelling the start: {e}")) }).detach();
        cx.notify();
    }

    pub fn check_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let helper = self.helper(host);
        let Some(connection) = self.connections.get_mut(host).filter(|c| matches!(c.status, Status::Browsing | Status::Died(_))) else {
            if matches!(self.status(host), None | Some(Status::Failed(_))) {
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

    pub fn stop_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        self.stop_host_then(host, false, cx);
    }

    fn stop_host_then(&mut self, host: &HostId, start: bool, cx: &mut Context<Self>) {
        match self.status(host) {
            Some(Status::Browsing | Status::Starting | Status::Ready | Status::Died(_)) => {}
            Some(Status::Connecting) => {
                if let Some(connection) = self.connections.get_mut(host) {
                    connection.stop_when_connected = true;
                }
                return cx.notify();
            }
            None | Some(Status::Failed(_)) => {
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
        let running = connection.bridge().filter(|_| start && connection.status == Status::Ready).map(|bridge| (bridge, connection.notebooks.clone()));
        let gone = if connection.status == Status::Starting {
            connection.cancelling = true;
            None
        } else {
            connection.status = Status::Died(String::new());
            if let Some(listener) = self.listeners.get(host).filter(|_| running.is_none()) {
                listener.disconnected();
            }
            connection.forget_runtime()
        };
        connection.stop_when_connected = false;
        connection.dropped = None;
        connection.stopping = true;
        connection.found = None;
        connection.check += 1;
        connection.start_after_stop = start;
        let id = connection.id;
        self.close_page(gone, cx);
        if *host == HostId::ThisMac {
            self.status = "Stopping notebooks…".into();
        }
        let listener = self.listeners.get(host).filter(|_| start).cloned();
        let stop = cx.background_executor().spawn(async move {
            let resume = running.map(|(bridge, notebooks)| running_files(&bridge, &notebooks)).unwrap_or_default();
            if let Some(listener) = listener {
                listener.restarting();
            }
            (resume, helper.stop())
        });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            let (resume, stopped) = stop.await;
            let _ = this.update(cx, |this, cx| {
                let mut restart = false;
                if let Some(connection) = this.connections.get_mut(&host).filter(|c| c.id == id) {
                    connection.stopping = false;
                    if let Err(e) = stopped {
                        connection.start_after_stop = false;
                        this.status = not_stopped(&this.hosts.name(&host), &e).into();
                        if host == HostId::ThisMac {
                            this.own_julia_after_stop(false, cx);
                            this.own_r_after_stop(false, cx);
                        }
                        return this.check_host(&host, cx);
                    }
                    connection.found = Some(Ok(RuntimeState::NotRunning));
                    restart = std::mem::take(&mut connection.start_after_stop);
                    if restart {
                        connection.resume = resume;
                    }
                }
                if restart {
                    return this.start_host(&host, cx);
                }
                if host == HostId::ThisMac {
                    this.status = concat!("Notebooks on ", crate::platform::this_computer!(), " are stopped.").into();
                    this.own_julia_after_stop(true, cx);
                    this.own_r_after_stop(true, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn restart_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        self.stop_host_then(host, true, cx);
    }

    pub fn signing_in(&self, host: &HostId) -> bool {
        self.lines.get(host).is_some_and(|line| self.asks.iter().any(|ask| ask.question().source == line.sign_in.id))
    }

    pub fn host_state(&self, host: &HostId) -> HostState {
        let Some(c) = self.connections.get(host) else { return HostState::Unknown };
        if c.stopping {
            return HostState::Stopping;
        }
        if c.lost.is_some() {
            return if self.signing_in(host) { HostState::SigningIn } else { HostState::Lost };
        }
        match &c.status {
            Status::Connecting => HostState::Connecting,
            Status::Browsing => match &c.found {
                None => HostState::Checking,
                Some(Ok(found)) => HostState::from(found),
                Some(Err(_)) => HostState::Connected,
            },
            Status::Died(_) => match &c.found {
                Some(Ok(found @ (RuntimeState::Running { .. } | RuntimeState::Queued { .. } | RuntimeState::Starting))) => HostState::from(found),
                _ => HostState::NotRunning,
            },
            Status::Starting => match &c.job {
                Some(job) => HostState::Queued { job: job.clone(), starting: c.steps.current != "Waiting for a node", reason: c.queue_reason.clone() },
                None => HostState::Starting,
            },
            Status::Ready => HostState::Running {
                notebooks: c.notebooks.as_array().map(Vec::len),
                job: c.runtime.as_ref().and_then(|r| r.job.as_ref()).map(|j| (j.id.clone(), j.ends_at)),
            },
            Status::Failed(e) => HostState::Failed(e.clone()),
        }
    }

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
        self.close_line(host, cx);
        cx.notify();
    }

    pub fn quit_runtimes(&mut self) {
        for connection in self.connections.values() {
            if let Some(channel) = &connection.channel {
                channel.quit(true);
            }
        }
        let closing: Vec<_> = self.lines.drain().map(|(_, line)| line.close()).collect();
        let until = Instant::now() + Duration::from_secs(2);
        while closing.iter().any(|c| !c.is_finished()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn julia_steps(&mut self, host: &HostId, kind: Backend, cx: &mut Context<Self>) -> JuliaSteps {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<String>();
        if let Some(connection) = self.connections.get_mut(host) {
            *connection.waits(kind) += 1;
        }
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            while let Some(step) = rx.next().await {
                let heard = this.update(cx, |this, cx| {
                    if let Some(connection) = this.connections.get_mut(&host) {
                        match kind {
                            Backend::Pluto => {
                                connection.julia_failed = None;
                                connection.julia.get_or_insert_with(|| Steps::first(&step, || Steps::new("Starting Julia"))).julia(&step);
                            }
                            Backend::Ember | Backend::Marimo => {
                                connection.r_failed = None;
                                connection.r.get_or_insert_with(|| Steps::first(&step, Steps::r)).r_step(&step);
                            }
                        }
                    }
                    cx.notify();
                });
                if heard.is_err() {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(connection) = this.connections.get_mut(&host) {
                    let waits = connection.waits(kind);
                    *waits = waits.saturating_sub(1);
                    if *waits == 0 {
                        match kind {
                            Backend::Pluto => connection.julia = None,
                            Backend::Ember | Backend::Marimo => connection.r = None,
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        JuliaSteps(tx)
    }

    pub fn julia_ahead(&self, host: &HostId) -> Option<String> {
        self.connections.get(host)?.julia.as_ref().map(ahead_line)
    }

    pub fn r_ahead(&self, host: &HostId) -> Option<String> {
        self.connections.get(host)?.r.as_ref().map(ahead_line)
    }

    pub fn app_waits(&self, host: &HostId, kind: Backend) -> u32 {
        self.connections.get(host).map_or(0, |c| match kind {
            Backend::Pluto => c.julia_waits,
            Backend::Ember | Backend::Marimo => c.r_waits,
        })
    }

    pub fn julia_answer(&mut self, host: &HostId, kind: Backend, error: Option<&str>, cx: &mut Context<Self>) {
        let failed = error.and_then(|e| pane_failure(kind, e));
        let Some(connection) = self.connections.get_mut(host) else { return };
        if failed.is_some() || error.is_none() {
            match kind {
                Backend::Pluto => connection.julia_failed = failed,
                Backend::Ember | Backend::Marimo => connection.r_failed = failed,
            }
            cx.notify();
        }
    }

    fn retry_julia(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let mut start = None;
        if let Some(connection) = self.connections.get_mut(host) {
            connection.julia_failed = None;
            if connection.julia_status.is_some() {
                start = connection.bridge();
                connection.julia_status = None;
                connection.julia.get_or_insert_with(|| Steps::new("Starting Julia"));
            }
        }
        let waiting = self.active_session().filter(|s| s.place.host == *host && s.notebook.is_none() && s.stopped.is_none() && !s.missing);
        let open = waiting.and_then(|s| Some((s.key, s.notebook_path.clone()?)));
        match start {
            Some(bridge) => {
                let started = cx.background_executor().spawn(async move { pluto::start_julia(&bridge) });
                let host = host.clone();
                cx.spawn(async move |this, cx| {
                    let started = started.await;
                    let _ = this.update(cx, |this, cx| {
                        match started {
                            Ok(status) => {
                                let started = this.connections.get(&host).and_then(|c| c.julia_started.clone());
                                this.on_julia_status(&host, status, started, cx)
                            }
                            Err(e) => this.julia_answer(&host, Backend::Pluto, Some(&format!("julia_failed::{e}")), cx),
                        }
                        if let Some((key, path)) = open {
                            this.open_for_session(key, path, false, cx);
                        }
                    });
                })
                .detach();
            }
            None => {
                if let Some((key, path)) = open {
                    self.open_for_session(key, path, false, cx);
                }
            }
        }
        cx.notify();
    }

    fn retry_r(&mut self, host: &HostId, install: Option<Backend>, cx: &mut Context<Self>) {
        let bridge = self.connections.get_mut(host).and_then(|connection| {
            connection.r_failed = None;
            connection.bridge()
        });
        let Some(again) = self.r_retry_target(host) else { return cx.notify() };
        match bridge.zip(install) {
            Some((bridge, kind)) => {
                let allowed = cx.background_executor().spawn(async move { if kind == Backend::Marimo { pluto::allow_uv_install(&bridge) } else { pluto::allow_r_install(&bridge) } });
                let host = host.clone();
                cx.spawn(async move |this, cx| {
                    let allowed = allowed.await;
                    let _ = this.update(cx, |this, cx| match allowed {
                        Ok(()) => {
                            if let Some(connection) = this.connections.get_mut(&host).filter(|_| kind == Backend::Marimo) {
                                connection.r.get_or_insert_with(Steps::r).r_step(DOWNLOADING_UV);
                            }
                            this.open_again(again, cx)
                        }
                        Err(e) if kind == Backend::Marimo => this.julia_answer(&host, kind, Some(&format!("python_failed::Couldn't download uv: {e}")), cx),
                        Err(e) => this.julia_answer(&host, kind, Some(&format!("r_failed::Couldn't install R: {e}")), cx),
                    });
                })
                .detach();
            }
            None => self.open_again(again, cx),
        }
        cx.notify();
    }

    fn r_retry_target(&self, host: &HostId) -> Option<(u64, Option<String>)> {
        let waiting = self.active_session().filter(|s| s.place.host == *host && waits_for_notebook(s));
        waiting.map(|s| (s.key, s.notebook_path.clone()))
    }

    fn open_again(&mut self, (key, path): (u64, Option<String>), cx: &mut Context<Self>) {
        match path {
            Some(path) => self.open_for_session(key, path, false, cx),
            None => self.new_notebook_here(key, cx),
        }
    }

    pub fn host_pane_state(&self, host: &HostId, kind: Backend, cx: &App) -> Option<HostPane> {
        let connection = self.connections.get(host);
        if let Some(lost) = connection.and_then(|c| c.lost.as_ref()) {
            let shown = crate::webcontent::url(self.webview.read(cx).raw());
            return (!lost.page.as_deref().is_some_and(|page| shown.starts_with(page))).then_some(HostPane::Lost);
        }
        Some(match connection.map_or(Status::Connecting, |c| c.status.clone()) {
            Status::Ready => match (kind, connection) {
                (Backend::Pluto, Some(Connection { julia_failed: Some(why), .. })) => HostPane::JuliaFailed(why.clone()),
                (Backend::Pluto, Some(Connection { julia: Some(steps), .. })) => HostPane::JuliaStarting(steps.current.clone()),
                (Backend::Ember | Backend::Marimo, Some(Connection { r_failed: Some(why), .. })) if is_python_failure(why) == (kind == Backend::Marimo) => HostPane::RFailed(why.clone()),
                (Backend::Ember | Backend::Marimo, Some(Connection { r: Some(steps), .. })) => HostPane::RStarting(steps.current.clone()),
                _ => return None,
            },
            Status::Connecting | Status::Browsing | Status::Starting => HostPane::Starting,
            Status::Died(_) if connection.is_some_and(|c| c.stopping) => HostPane::Stopping,
            Status::Died(reason) if connection.is_some_and(|c| c.crashed) => HostPane::Crashed(reason),
            Status::Died(reason) => HostPane::NotRunning(reason),
            Status::Failed(reason) => HostPane::NotConnected(reason),
        })
    }

    pub fn host_pane(&self, host: &HostId, kind: Backend, start: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.host_pane_state(host, kind, cx)?;
        let connection = self.connections.get(host);
        let name = self.hosts.mid_sentence_name(host);
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
                .button_text(label)
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
                if start {
                    let text = if offline { format!("Endeavor reconnects to {name} when you're back online.") } else { format!("Endeavor reconnects to {name} by itself.") };
                    resting.child(div().text_color(theme::text_muted()).child(text))
                } else {
                    let text = if offline { "Endeavor reconnects when you're back online." } else { "Endeavor reconnects by itself." };
                    let host = host.clone();
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
            HostPane::JuliaStarting(_) => return connection.and_then(|c| c.julia.as_ref()).map(|steps| starting_pane(steps).into_any_element()),
            HostPane::RStarting(_) => return connection.and_then(|c| c.r.as_ref()).map(|steps| starting_pane(steps).into_any_element()),
            HostPane::JuliaFailed(reason) => {
                let host = host.clone();
                let retry = div()
                    .id("julia-retry")
                    .role(Role::Button)
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(theme::accent())
                    .text_color(gpui::white())
                    .button_text("Try again")
                    .on_click(cx.listener(move |this, _, _, cx| this.retry_julia(&host, cx)));
                let heading = if reason == pluto::OPEN_STUCK { format!("The notebook didn't open on {name}.") } else { format!("Julia couldn't start on {name}.") };
                resting.child(div().text_color(theme::text_muted()).child(heading)).child(message(pluto::julia_failed_words(&reason))).child(retry)
            }
            HostPane::RFailed(error) => {
                let host_id = host.clone();
                let shows = r_failed_shows(host, &error, self.hosts.downloads_own_uv(host), own_r_offered());
                let button = |label: &'static str, install: Option<Backend>| {
                    let host = host_id.clone();
                    div()
                        .id(shows.button())
                        .role(Role::Button)
                        .px_3()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(theme::accent())
                        .text_color(gpui::white())
                        .button_text(label)
                        .on_click(cx.listener(move |this, _, _, cx| this.retry_r(&host, install, cx)))
                };
                match &shows {
                    RFailedShows::InstallUv => {
                        let words = format!("Python notebooks need uv. Endeavor can download its own, just for Endeavor. It changes nothing else on {name}.");
                        resting.child(div().text_color(theme::text_muted()).child(format!("No uv on {name}"))).child(message(words)).child(button("Install uv", Some(Backend::Marimo)))
                    }
                    RFailedShows::PythonFailed(why) => {
                        let heading = if *why == pluto::OPEN_STUCK { format!("The notebook didn't open on {name}.") } else { format!("Python notebooks couldn't start on {name}.") };
                        resting.child(div().text_color(theme::text_muted()).child(heading)).child(message(pluto::julia_failed_words(why))).child(button("Try again", None))
                    }
                    RFailedShows::InstallR(item) => {
                        let size = item.size_mb.map(|mb| format!(", about {mb} MB")).unwrap_or_default();
                        let words = format!(
                            "Endeavor can install its own {}{size}, just for Endeavor. It changes nothing else on {}.",
                            item.name,
                            crate::platform::this_computer!(lower)
                        );
                        resting.child(div().text_color(theme::text_muted()).child(format!("No R on {name}"))).child(message(words)).child(button("Install R", Some(Backend::Ember)))
                    }
                    RFailedShows::RFailed(why) => {
                        let heading = if *why == pluto::OPEN_STUCK { format!("The notebook didn't open on {name}.") } else { format!("R notebooks couldn't start on {name}.") };
                        resting.child(div().text_color(theme::text_muted()).child(heading)).child(message(pluto::julia_failed_words(why))).child(button("Try again", None))
                    }
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
                        .aria_label(format!("Cancel connecting to {name}"))
                        .on_click(cx.listener(move |this, _, _, cx| this.cancel_start(&host, cx)))
                });
                return Some(starting_pane(&steps).children(cancel).into_any_element());
            }
            HostPane::Stopping => resting.child(div().text_color(theme::text_muted()).child(format!("Stopping {} on {name}…", crate::notebook_pane::engine_name(kind)))),
            HostPane::NotRunning(reason) => return Some(self.julia_stopped_page(host, kind, &reason, cx)),
            HostPane::Crashed(reason) => return Some(self.julia_crashed_page(host, kind, &reason, cx)),
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

pub fn starting_ahead(state: &HostPane, waits: u32) -> bool {
    waits == 0 && matches!(state, HostPane::JuliaStarting(_) | HostPane::RStarting(_))
}

fn ahead_line(steps: &Steps) -> String {
    match steps.detail.as_deref() {
        Some(pct) if pct.ends_with('%') => format!("{} {pct}", steps.current),
        Some(stall) if stall.starts_with("No sign of progress") => format!("{}. {stall}", steps.current.trim_end_matches('…')),
        _ => steps.current.clone(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HostPane {
    Lost,
    Starting,
    JuliaStarting(String),
    JuliaFailed(String),
    RStarting(String),
    RFailed(String),
    Stopping,
    NotRunning(String),
    Crashed(String),
    NotConnected(String),
}

fn is_python_failure(error: &str) -> bool {
    pluto::python_failed(error).is_some()
}

#[derive(Debug, PartialEq)]
pub enum RFailedShows<'a> {
    InstallUv,
    PythonFailed(&'a str),
    InstallR(wire::Item),
    RFailed(&'a str),
}

impl RFailedShows<'_> {
    pub fn button(&self) -> &'static str {
        match self {
            RFailedShows::InstallUv => "uv-install",
            RFailedShows::PythonFailed(_) => "python-retry",
            RFailedShows::InstallR(_) => "r-install",
            RFailedShows::RFailed(_) => "r-retry",
        }
    }
}

pub fn r_failed_shows<'a>(host: &HostId, error: &'a str, own_uv_there: bool, own_r: Option<wire::Item>) -> RFailedShows<'a> {
    let here = *host == HostId::ThisMac;
    if own_uv_there && error.starts_with("python_not_found::") {
        return RFailedShows::InstallUv;
    }
    if let Some(why) = pluto::python_failed(error) {
        return RFailedShows::PythonFailed(why);
    }
    match own_r.filter(|_| here && error.starts_with("r_not_found::")) {
        Some(item) => RFailedShows::InstallR(item),
        None => RFailedShows::RFailed(pluto::r_failed(error).unwrap_or(error)),
    }
}

pub fn own_r_offered() -> Option<wire::Item> {
    own_r_item().filter(|_| endeavor_mcp::r::own_installed().is_none())
}

pub fn own_r_item() -> Option<wire::Item> {
    static OFFERED: std::sync::OnceLock<Option<wire::Item>> = std::sync::OnceLock::new();
    OFFERED.get_or_init(endeavor_mcp::r::own_offered).clone()
}

pub fn pane_failure(kind: Backend, error: &str) -> Option<String> {
    match kind {
        Backend::Pluto => pluto::julia_failed(error).or_else(|| pluto::open_stuck(error)).map(str::to_owned),
        Backend::Ember => pluto::r_failed(error).map(|_| error.to_owned()).or_else(|| pluto::open_stuck(error).map(|why| format!("r_failed::{why}"))),
        Backend::Marimo => pluto::python_failed(error).map(|_| error.to_owned()).or_else(|| pluto::open_stuck(error).map(|why| format!("python_failed::{why}"))),
    }
}

#[derive(Clone)]
pub struct JuliaSteps(futures::channel::mpsc::UnboundedSender<String>);

impl JuliaSteps {
    pub fn step(&self, step: &str) {
        let _ = self.0.unbounded_send(step.to_owned());
    }
}

pub(crate) struct Line {
    session: Arc<client::Session>,
    _asker: client::Asker,
    sign_in: remote::SignIn,
    server: Server,
    closed: Arc<AtomicBool>,
    last: Option<LineStatus>,
    generation: u64,
    held: Option<Held>,
}

struct Held {
    id: u64,
    pid: u32,
    why: String,
    since: Vec<Change>,
}

const LOST_GRACE: Duration = Duration::from_secs(2);

impl Line {
    fn close(self) -> std::thread::JoinHandle<()> {
        self.closed.store(true, Ordering::SeqCst);
        let Line { session, _asker, .. } = self;
        std::thread::spawn(move || {
            session.close();
            drop(_asker);
        })
    }
}

enum LineUpdate {
    Event(SessionEvent),
    Status(Box<LineStatus>, Instant),
}

fn try_ended(last: &LineStatus, now: &LineStatus) -> bool {
    !signed_in(now) && now.step != last.step
}

#[derive(Clone, Debug, PartialEq)]
enum Change {
    Connected(client::HelloInfo),
    ConnectFailed(String),
    Lost(String),
    Starting,
    Started(client::RuntimeInfo),
    StartFailed(String),
    Died(String),
    Stopped,
}

fn signed_in(status: &LineStatus) -> bool {
    status.state != LineState::Connecting && status.hello.as_ref().is_some_and(|h| !h.node.is_empty())
}

#[derive(PartialEq)]
enum Phase<'a> {
    Idle,
    Gone(&'a str),
    Starting,
    Ready(&'a client::RuntimeInfo),
    Failed(&'a str),
}

fn phase(status: &LineStatus) -> Phase<'_> {
    match &status.state {
        LineState::Connecting | LineState::Connected => Phase::Idle,
        LineState::NothingRunning => Phase::Gone(status.step.as_deref().unwrap_or("")),
        LineState::Starting { .. } | LineState::Queued(_) => Phase::Starting,
        LineState::Ready(runtime) => Phase::Ready(runtime),
        LineState::NeedsInstall(_) => Phase::Failed(status.step.as_deref().unwrap_or("")),
        LineState::Failed(why) => Phase::Failed(why),
    }
}

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
        (Phase::Ready(was), Phase::Ready(runtime)) if (was.pid, &was.node) == (runtime.pid, &runtime.node) => {}
        (_, Phase::Ready(runtime)) => {
            if !matches!(last.state, LineState::Starting { .. } | LineState::Queued(_)) {
                out.push(Change::Starting);
            }
            out.push(Change::Started(runtime.clone()));
        }
        (Phase::Idle | Phase::Gone(_) | Phase::Failed(_), Phase::Starting) => out.push(Change::Starting),
        (Phase::Ready(_), Phase::Starting) => {}
        (Phase::Ready(_), Phase::Failed(why) | Phase::Gone(why)) => out.push(match &now.gone {
            Some(client::Gone::Died(reason)) => Change::Died(reason.clone()),
            None => Change::Died(why.to_owned()),
        }),
        (Phase::Starting | Phase::Ready(_), Phase::Idle) => out.push(Change::Stopped),
        (Phase::Starting, Phase::Gone(_)) if now.gone.is_none() => out.push(Change::Stopped),
        (Phase::Starting, Phase::Failed(why) | Phase::Gone(why)) => out.push(Change::StartFailed(why.to_owned())),
        (Phase::Idle | Phase::Gone(_), Phase::Failed(why)) => out.push(Change::StartFailed(why.to_owned())),
        (Phase::Idle | Phase::Gone(_) | Phase::Failed(_), Phase::Idle | Phase::Gone(_)) | (Phase::Failed(_), Phase::Failed(_)) | (Phase::Starting, Phase::Starting) => {}
    }
    out
}

pub fn waits_for_notebook(s: &crate::session::Session) -> bool {
    s.notebook.is_none() && s.stopped.is_none() && !s.missing
}

fn hello_of(hello: &client::HelloInfo) -> Hello {
    let launcher = match hello.launcher.as_deref() {
        Some("slurm") => Some(client::Launcher::Slurm),
        Some("process") => Some(client::Launcher::Process),
        _ => None,
    };
    Hello { protocol: wire::PROTOCOL, node: hello.node.clone(), home: hello.home.clone(), slurm: hello.slurm, uploads: hello.uploads, launcher }
}

fn runtime_of(runtime: client::RuntimeInfo) -> Runtime {
    let version = crate::older_runtime::Version::of_server(&runtime);
    Runtime {
        page_url: runtime.page_url,
        bridge: Bridge { url: runtime.mcp_url, page_token: if runtime.page_token.is_empty() { runtime.token.clone() } else { runtime.page_token }, token: runtime.token },
        pid: runtime.pid,
        reattached: runtime.reattached,
        node: runtime.node,
        job: runtime.job,
        version,
    }
}

fn same_connection(a: &Server, b: &Server) -> bool {
    (&a.ssh_host, a.port, &a.julia, &a.r, a.cluster.is_some()) == (&b.ssh_host, b.port, &b.julia, &b.r, b.cluster.is_some())
}

#[derive(Clone)]
pub enum Helper {
    Local(Arc<Channel>),
    Line(Arc<client::Session>),
}

impl Helper {
    pub fn files(&self, request: Request) -> Result<Reply, String> {
        match self {
            Helper::Local(channel) => channel.files(request),
            Helper::Line(session) => session.files(request, Duration::ZERO).unwrap_or_else(|| Err(session.status().state.error().map_or_else(|| "Endeavor isn't connected to this server yet.".to_owned(), str::to_owned))),
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        match self {
            Helper::Local(channel) => channel.force_stop(),
            Helper::Line(session) => session.force_stop(),
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

fn running_files(bridge: &Bridge, notebooks: &serde_json::Value) -> Vec<(String, Option<f64>)> {
    let allowed = notebooks.as_array().into_iter().flatten().filter(|nb| nb["execution_allowed"] == true);
    allowed.filter_map(|nb| nb["path"].as_str()).filter_map(|path| Some((path.to_owned(), Some(pluto::file_info(bridge, path).ok()??)))).collect()
}

fn next_connect_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

const JULIA_NOT_RUNNING: &str = "Julia not running";

const REPAIR_AFTER: u32 = 2;

pub const REPAIR_NOTE: &str = "Repair stops Julia, clears its saved state and compiled runtime, and starts it again. Notebooks, packages and settings stay.";

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

fn job_warning_window() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(secs) = std::env::var("ENDEAVOR_JOB_WARNING_SECONDS").ok().and_then(|v| v.parse().ok()) {
        return Duration::from_secs(secs);
    }
    Duration::from_secs(15 * 60)
}

pub fn elapsed(since: Instant) -> String {
    let s = since.elapsed().as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

const WALKER_R: f32 = 22.;

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

fn walker() -> Canvas<()> {
    canvas(
        |_, _, _| (),
        |bounds, _, window, cx| {
            let t = if cx.reduce_motion() {
                0.
            } else {
                crate::motion::loop_clock(window, cx)
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

fn not_restarted(why: &str) -> String {
    format!("Julia didn't restart, so Endeavor is still using the Julia that was running. {}", stop_reason(why))
}

fn not_stopped(host: &str, why: &str) -> String {
    match why.strip_prefix("Julia was not stopped: ") {
        Some(rest) => format!("Notebooks on {host} were not stopped: {rest}"),
        None => format!("Couldn't stop notebooks on {host}: {why}"),
    }
}

fn stop_reason(why: &str) -> String {
    let rest = why.strip_prefix("Julia was not stopped: ").unwrap_or(why);
    let mut chars = rest.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn load_again(at: &str, origin: &str, page: &crate::annotate::PageState, notebook: &str) -> bool {
    !at.starts_with(origin) || (page.notebook == notebook && !page.connected && !page.unsaved)
}

fn new_failure<'a>(before: Option<&pluto::JuliaStatus>, now: &'a pluto::JuliaStatus) -> Option<&'a str> {
    match now {
        pluto::JuliaStatus::Failed { message, .. } if !matches!(before, Some(pluto::JuliaStatus::Failed { .. })) => Some(message),
        _ => None,
    }
}

fn r_failure(before: Option<&pluto::RStatus>, now: &pluto::RStatus) -> Option<String> {
    let pluto::RStatus::Failed { code, .. } = now else { return None };
    new_failure(before, now).map(|message| format!("{}::{message}", if code == "r_not_found" { "r_not_found" } else { "r_failed" }))
}

pub(crate) fn same_file(host: &HostId, a: &str, b: &str) -> bool {
    a == b || (*host == HostId::ThisMac && matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b))
}

#[cfg(test)]
mod tests {
    use super::{Change, Connection, HostPane, LineState, LineStatus, Status, Steps, changes, client, elapsed, ahead_line, load_again, new_failure, not_restarted, r_failure, not_stopped, pane_failure, percent, runtime_of, same_connection, starting_ahead, try_ended};
    use crate::hosts::Server;
    use wire::backend::Backend;
    use crate::pluto;

    #[test]
    fn the_pane_shows_only_its_own_kinds_failures() {
        let julia = "julia_failed::Julia stopped while starting (exit 1).";
        assert_eq!(pane_failure(Backend::Pluto, julia).as_deref(), Some("Julia stopped while starting (exit 1)."));
        assert_eq!(pane_failure(Backend::Pluto, "open_stuck::It's still opening.").as_deref(), Some("It's still opening."));
        assert_eq!(pane_failure(Backend::Pluto, "r_failed::R stopped while starting"), None);
        assert_eq!(pane_failure(Backend::Pluto, "invalid_path::Give an absolute path"), None);
        assert_eq!(pane_failure(Backend::Pluto, "The connection to the runtime closed."), None);
        assert_eq!(pane_failure(Backend::Ember, "r_not_found::No R here").as_deref(), Some("r_not_found::No R here"));
        assert_eq!(pane_failure(Backend::Ember, "open_stuck::It's still opening.").as_deref(), Some("r_failed::It's still opening."));
        assert_eq!(pane_failure(Backend::Ember, julia), None);
        let no_uv = "python_not_found::Python notebooks need uv, which isn't installed on this computer.";
        assert_eq!(pane_failure(Backend::Marimo, no_uv).as_deref(), Some(no_uv));
        assert_eq!(pane_failure(Backend::Marimo, "open_stuck::It's still opening.").as_deref(), Some("python_failed::It's still opening."));
        assert_eq!(pane_failure(Backend::Marimo, "r_failed::R stopped while starting"), None);
        assert!(super::is_python_failure(no_uv) && !super::is_python_failure("r_not_found::No R here"));
    }

    #[test]
    fn a_missing_uv_offers_endeavors_own_only_where_there_is_one() {
        use super::{RFailedShows, r_failed_shows};
        use crate::hosts::HostId;
        let no_uv = "python_not_found::Python notebooks need uv, which isn't installed on this computer. Install uv (https://docs.astral.sh/uv/getting-started/installation/).";
        let here = HostId::ThisMac;
        let lab = HostId::Server("lab".into());
        assert_eq!(r_failed_shows(&here, no_uv, true, None), RFailedShows::InstallUv);
        assert_eq!(r_failed_shows(&lab, no_uv, true, None).button(), "uv-install");
        let without_own_uv = r_failed_shows(&here, no_uv, false, None);
        assert_eq!(without_own_uv.button(), "python-retry");
        let RFailedShows::PythonFailed(words) = without_own_uv else { panic!("{without_own_uv:?}") };
        assert!(words.starts_with("Python notebooks need uv") && words.contains("docs.astral.sh/uv"));
        assert!(matches!(r_failed_shows(&lab, no_uv, false, None), RFailedShows::PythonFailed(_)));
        assert_eq!(r_failed_shows(&here, "python_failed::Couldn't download uv: offline", true, None), RFailedShows::PythonFailed("Couldn't download uv: offline"));
    }

    #[test]
    fn a_missing_r_offers_endeavors_own_only_on_this_computer() {
        use super::{RFailedShows, r_failed_shows};
        use crate::hosts::HostId;
        let item = wire::Item { kind: "r".into(), name: "R 4.5".into(), size_mb: Some(90), place: None };
        let no_r = "r_not_found::No R here";
        assert_eq!(r_failed_shows(&HostId::ThisMac, no_r, true, Some(item.clone())), RFailedShows::InstallR(item.clone()));
        assert_eq!(r_failed_shows(&HostId::ThisMac, no_r, true, None), RFailedShows::RFailed("No R here"));
        assert_eq!(r_failed_shows(&HostId::Server("lab".into()), no_r, true, Some(item)).button(), "r-retry");
        assert_eq!(r_failed_shows(&HostId::ThisMac, "r_failed::R stopped", true, None), RFailedShows::RFailed("R stopped"));
    }

    #[test]
    fn a_start_ahead_leaves_new_notebook_showing() {
        let julia = HostPane::JuliaStarting("Installing packages".into());
        assert!(starting_ahead(&julia, 0));
        assert!(!starting_ahead(&julia, 1), "a click's call waits on it");
        assert!(!starting_ahead(&HostPane::JuliaFailed("Couldn't download Julia.".into()), 0), "a failure keeps Try again");
        assert!(!starting_ahead(&HostPane::Starting, 0), "the runtime itself starting");
        let r = HostPane::RStarting("Installing Ember for R notebooks…".into());
        assert!(starting_ahead(&r, 0), "R's setup for someone else, as Julia's");
        assert!(!starting_ahead(&r, 1));
        assert!(!starting_ahead(&HostPane::RFailed("r_failed::Couldn't install Ember".into()), 0));
    }

    #[test]
    fn r_status_puts_the_setup_and_a_failure_on_the_pane_once() {
        use crate::pluto::JuliaStatus::{Failed, NotStarted, Ready, Starting};
        let mut c = Connection::new(1, Status::Ready, Steps::new("Starting the notebook runtime"));
        let failed = Failed { code: "r_failed".into(), message: "Couldn't install Ember".into() };
        assert!(c.r_status_now(NotStarted));
        assert!(!c.r_status_now(NotStarted), "nothing new");
        assert_eq!((c.r.is_none(), c.r_failed.is_none()), (true, true));

        assert!(c.r_status_now(Starting { step: "Installing Ember for R notebooks".into(), quiet: 0 }));
        assert_eq!(c.r.as_ref().map(|s| s.current.as_str()), Some("Installing Ember for R notebooks…"));
        assert_eq!(c.r.as_ref().and_then(|s| s.detail.as_deref()), Some("The first time takes a few minutes."));
        assert!(c.r_status_now(failed.clone()));
        assert_eq!(c.r_failed.as_deref(), Some("r_failed::Couldn't install Ember"), "the pane says why");
        assert!(c.r.is_none(), "no call of the app's waits");

        c.r_failed = None;
        assert!(!c.r_status_now(failed.clone()));
        assert_eq!(c.r_failed, None, "stays dismissed");

        c.r_failed = Some("r_failed::old".into());
        assert!(c.r_status_now(Starting { step: "Updating Ember".into(), quiet: 0 }));
        assert_eq!(c.r_failed, None);
        assert_eq!(c.r.as_ref().and_then(|s| s.detail.as_deref()), None, "an update isn't a first time");
        c.r_failed = Some("r_failed::old".into());
        assert!(c.r_status_now(Ready));
        assert_eq!((c.r.is_none(), c.r_failed.is_none()), (true, true));

        c.r_waits = 1;
        assert!(c.r_status_now(Starting { step: "Starting R".into(), quiet: 0 }));
        assert!(c.r_status_now(failed));
        assert!(c.r.is_some(), "the app's call still waits");
        assert_eq!(c.r_failed.as_deref(), Some("r_failed::Couldn't install Ember"));
    }

    #[test]
    fn the_start_ahead_line_keeps_progress_and_stalls() {
        let mut steps = Steps::new("Starting Julia");
        steps.julia_status("Downloading Julia 1.12.6… 42%", 0);
        assert_eq!(ahead_line(&steps), "Downloading Julia 1.12.6… 42%");
        steps.julia_status("Julia 1.12.6 is starting and loading Pluto", 0);
        assert_eq!(ahead_line(&steps), "Starting Julia 1.12.6 and Pluto");
        steps.julia_status("Julia 1.12.6 is starting and loading Pluto", 180);
        assert_eq!(ahead_line(&steps), "Starting Julia 1.12.6 and Pluto. No sign of progress for 3 minutes.");
        steps.log("Precompiling Pluto");
        assert_eq!(ahead_line(&steps), "Starting Julia 1.12.6 and Pluto", "the log's lines stay out");
    }

    fn line(state: LineState, node: &str) -> LineStatus {
        let hello = Some(client::HelloInfo { node: node.into(), home: "/home/me".into(), ..Default::default() });
        LineStatus { machine: "lab".into(), name: "lab".into(), state, step: None, hello, job: None, gone: None }
    }

    #[test]
    fn a_start_that_fails_between_two_polls_still_shows_why() {
        use crate::pluto::JuliaStatus::{Failed, NotStarted, Ready, Starting};
        let failed = Failed { code: "julia_download_failed".into(), message: "Couldn't download Julia.".into() };
        assert_eq!(new_failure(Some(&NotStarted), &failed), Some("Couldn't download Julia."), "offline: never seen starting");
        assert_eq!(new_failure(Some(&Starting { step: "Downloading".into(), quiet: 0 }), &failed), Some("Couldn't download Julia."));
        assert_eq!(new_failure(None, &failed), Some("Couldn't download Julia."), "a first look after attaching");
        assert_eq!(new_failure(Some(&failed), &failed), None, "said once");
        assert_eq!(new_failure(Some(&NotStarted), &Ready), None);
    }

    #[test]
    fn r_setup_the_runtime_reports_shows_its_steps_and_a_failure_the_pane_reads() {
        use crate::pluto::JuliaStatus::{Failed, NotStarted, Ready, Starting};
        let starting = |step: &str| Starting { step: step.into(), quiet: 0 };
        let mut steps = Steps::r();
        steps.r_step("Installing Endeavor's own R 4.6.1");
        steps.r_step("Installing Ember for R notebooks");
        steps.r_step("Starting R");
        assert_eq!(steps.done, vec!["Installed Endeavor's own R 4.6.1", "Installed Ember for R notebooks"]);
        assert_eq!(steps.current, "Starting R…");
        let failed = Failed { code: "r_failed".into(), message: "Couldn't install Ember".into() };
        assert_eq!(r_failure(Some(&starting("Starting R")), &failed).as_deref(), Some("r_failed::Couldn't install Ember"));
        assert_eq!(pluto::r_failed(&r_failure(None, &failed).unwrap()), Some("Couldn't install Ember"), "the pane reads it as an open's error");
        let no_r = Failed { code: "r_not_found".into(), message: "Couldn't find R".into() };
        assert_eq!(r_failure(Some(&NotStarted), &no_r).as_deref(), Some("r_not_found::Couldn't find R"), "the pane offers Install R");
        assert_eq!(r_failure(Some(&failed), &failed), None, "said once");
        assert_eq!(r_failure(Some(&starting("Starting R")), &Ready), None);
        assert_eq!(pluto::RStatus::from(&serde_json::json!({ "state": "ready", "updating": true })), Ok(Ready));
    }

    #[test]
    fn a_failed_try_while_reconnecting_ends_its_sign_in() {
        let step = |state: LineState, step: &str| LineStatus { step: Some(step.into()), hello: None, ..line(state, "") };
        let trying = step(LineState::Connecting, "Connecting to lab");
        let failed = step(LineState::Connecting, "Lost the connection to lab: The connection to lab ended: the server stopped waiting for the sign-in.");
        let again = step(LineState::Connecting, "Connecting to lab");
        assert!(changes(&trying, &failed).is_empty(), "no ConnectFailed while it tries again");
        assert!(try_ended(&trying, &failed), "but its questions go");
        assert!(try_ended(&failed, &again));
        assert!(!try_ended(&trying, &trying.clone()));
        let gave_up = step(LineState::Failed("refused".into()), "refused");
        assert!(try_ended(&trying, &gave_up), "a first connect's failure too");
        let connected = LineStatus { step: Some("Signed in to lab".into()), ..line(LineState::Connected, "lab") };
        assert!(!try_ended(&trying, &connected), "signing in isn't an end");
    }

    #[test]
    fn exports_use_the_pages_token_and_an_older_runtimes_bearer_token() {
        assert_eq!(runtime_of(runtime(7)).bridge.page_token, "p");
        assert_eq!(runtime_of(client::RuntimeInfo { page_token: String::new(), ..runtime(7) }).bridge.page_token, "t");
    }

    fn runtime(pid: u32) -> client::RuntimeInfo {
        client::RuntimeInfo {
            port: 1,
            token: "t".into(),
            page_token: "p".into(),
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            page_url: "http://127.0.0.1:1/?token=p".into(),
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
        assert!(changes(&connected, &line(LineState::NothingRunning, "lab")).is_empty());
        let nothing = LineStatus { step: Some("No runtime is running on lab".into()), ..line(LineState::NothingRunning, "lab") };
        assert_eq!(changes(&starting, &nothing), [Change::Stopped]);
        let went = LineStatus { gone: Some(client::Gone::Died("out of memory".into())), step: Some("out of memory".into()), ..line(LineState::NothingRunning, "lab") };
        assert_eq!(changes(&starting, &went), [Change::StartFailed("out of memory".into())]);
        assert_eq!(changes(&line(LineState::NothingRunning, "lab"), &starting), [Change::Starting]);
        let refused = "Julia was not stopped: it is still starting. Try again once it is up, or force the stop to cancel the start.";
        assert_eq!(changes(&starting, &line(LineState::Failed(refused.into()), "lab")), [Change::StartFailed(refused.into())]);
        assert!(changes(&ready, &line(LineState::Ready(client::RuntimeInfo { reattached: true, ..runtime(7) }), "lab")).is_empty());
        assert_eq!(changes(&ready, &line(LineState::Ready(runtime(8)), "lab")), [Change::Starting, Change::Started(runtime(8))], "another Julia");
    }

    #[test]
    fn a_server_connection_that_gets_back_to_its_julia_carries_on() {
        let mut c = Connection::new(1, Status::Failed("Can't reach lab".into()), Steps::new("Connecting to lab"));
        c.dropped = Some((7, "lab".into()));
        assert!(c.same_julia(&runtime_of(runtime(7))), "however long the drop");
        assert!(!c.same_julia(&runtime_of(runtime(8))), "a new Julia reopens its notebooks");
        assert!(!c.same_julia(&runtime_of(client::RuntimeInfo { node: "node2".into(), ..runtime(7) })), "a pid on another node is another Julia");
        c.dropped = None;
        assert!(!c.same_julia(&runtime_of(runtime(7))), "nothing dropped, nothing to carry on");
        c.status = Status::Ready;
        c.runtime = Some(runtime_of(runtime(7)));
        assert!(c.same_julia(&runtime_of(client::RuntimeInfo { reattached: true, ..runtime(7) })), "seen again while ready");
    }

    #[test]
    fn only_what_the_line_uses_counts_as_a_change_of_connection() {
        let server = Server { ssh_host: "lab".into(), cluster: Some(crate::hosts::Cluster::default()), ..Default::default() };
        let mut resources = server.clone();
        resources.cluster.as_mut().unwrap().account = Some("other".into());
        resources.name = "Lab".into();
        assert!(same_connection(&server, &resources), "a cluster's job settings are read at each start");
        for changed in [Server { port: Some(2222), ..server.clone() }, Server { cluster: None, ..server.clone() }, Server { julia: Some("/opt/julia".into()), ..server.clone() }] {
            assert!(!same_connection(&server, &changed), "{changed:?}");
        }
    }

    #[test]
    fn a_connection_let_go_for_new_settings_carries_on_on_the_same_julia_as_after_a_drop() {
        let mut c = Connection::new(1, Status::Ready, Steps::new("Connecting to lab"));
        c.runtime = Some(runtime_of(runtime(7)));
        c.linked = true;
        c.lose("Connecting to lab with its new settings".into(), true);
        assert_eq!(c.status, Status::Failed("Connecting to lab with its new settings".into()));
        assert!(!c.connected() && c.runtime.is_none());
        assert!(c.same_julia(&runtime_of(runtime(7))), "the line that gets back to it carries on");
        let lost = c.lost.as_ref().unwrap();
        assert!(lost.had_julia() && lost.page.as_deref() == Some("http://127.0.0.1:1/"), "Julia is started (attached) again, and the page stays up meanwhile");
    }

    #[test]
    fn back_on_the_same_julia_a_page_still_reconnecting_loads_again_unless_it_has_edits() {
        let origin = "http://127.0.0.1:4100/edit";
        let at = "http://127.0.0.1:4100/edit?id=nb";
        let page = crate::annotate::PageState { notebook: "nb".into(), connected: false, ..Default::default() };
        assert!(load_again("about:blank", origin, &page, "nb"), "the page was let go");
        assert!(load_again(at, origin, &page, "nb"), "still waiting to reconnect");
        assert!(!load_again(at, origin, &crate::annotate::PageState { connected: true, ..page.clone() }, "nb"), "already live");
        assert!(!load_again(at, origin, &crate::annotate::PageState { unsaved: true, ..page.clone() }, "nb"), "an edit not yet run");
        assert!(!load_again(at, origin, &page, "other"), "the page is another notebook's");
    }

    #[test]
    fn a_servers_julia_that_goes_by_itself_is_told_apart_from_a_drop() {
        let ready = line(LineState::Ready(runtime(7)), "lab");
        let mut died = line(LineState::NothingRunning, "lab");
        died.step = Some("Julia on lab stopped. It ran out of memory.".into());
        died.gone = Some(client::Gone::Died("It ran out of memory.".into()));
        assert_eq!(changes(&ready, &died), [Change::Died("It ran out of memory.".into())]);
        died.step = Some("Julia on lab ended: out of memory.".into());
        assert_eq!(changes(&ready, &died), [Change::Died("It ran out of memory.".into())]);
        let failed = line(LineState::Failed("Julia on lab couldn't start.".into()), "lab");
        assert_eq!(changes(&ready, &failed), [Change::Died("Julia on lab couldn't start.".into())]);
        let mut dropped = line(LineState::Connecting, "lab");
        dropped.step = Some("Lost the connection to lab; connecting again".into());
        assert_eq!(changes(&ready, &dropped), [Change::Lost("Lost the connection to lab; connecting again".into())]);
        assert!(changes(&dropped, &line(LineState::Connecting, "")).is_empty(), "nor while it tries again");
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
        assert_eq!(not_stopped("lab", "Julia was not stopped: it is still running."), "Notebooks on lab were not stopped: it is still running.");
        assert_eq!(not_stopped("lab", "The connection to Endeavor's helper closed."), "Couldn't stop notebooks on lab: The connection to Endeavor's helper closed.");
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
    fn a_notebook_still_opening_is_its_own_step() {
        let mut steps = Steps::new("Starting Julia");
        steps.julia("Julia 1.12.6 is starting and loading Pluto");
        steps.julia(pluto::OPENING);
        assert_eq!((steps.done.as_slice(), steps.current.as_str()), (["Started Julia 1.12.6 and Pluto".to_owned()].as_slice(), pluto::OPENING));
        assert!(steps.pending().is_empty(), "the open is the step under way");
        assert!(steps.detail.as_deref().is_some_and(|d| !d.contains('/')), "no path: {:?}", steps.detail);
        let steps = Steps::first(pluto::OPENING, || Steps::new("Starting Julia"));
        assert_eq!((steps.done.len(), steps.current.as_str()), (0, pluto::OPENING));
        let mut r = Steps::first("Installing Ember", Steps::r);
        r.r_step("Installing Ember");
        r.r_step(pluto::OPENING);
        assert_eq!((r.done.as_slice(), r.current.as_str()), (["Installed Ember".to_owned()].as_slice(), pluto::OPENING));
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
