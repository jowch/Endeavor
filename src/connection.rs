//! The app's line to each host's runtime: This Mac's connects and starts at
//! launch; a server's connects when it's picked in the Where chip (its files
//! can be browsed then) and starts Julia when a session on it needs one. Each
//! host keeps its own local listener, so its sessions' MCP URL survives
//! reconnects and restarts.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::hosts::HostId;
use crate::pluto::{self, Bridge};
use crate::remote::{self, Askpass, Cancel, Event};
use crate::runtime::{self, Channel, Hello, Listener, Notice, Runtime};
use crate::session::{Session, Stopped};
use crate::splash::{Progress, Step};
use crate::turtle::{self, Pose};
use crate::{Workspace, theme};

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
    pub channel: Option<Arc<Channel>>,
    pub hello: Option<Hello>,
    pub runtime: Option<Runtime>,
    /// Start Julia once connected (a session is waiting for it).
    start_when_connected: bool,
    /// ssh may ask again (a reconnect inside it, a key's passphrase), so it lives as long as the connection.
    _askpass: Option<Askpass>,
    cancel: Arc<Cancel>,
    pub steps: Steps,
    /// Open notebooks (id, path) as last seen, to reopen after a restart.
    pub last_notebooks: Vec<(String, String)>,
    /// The runtime's notebook list as last pushed (`list_notebooks` shape).
    pub notebooks: serde_json::Value,
    /// Per-notebook cell states as last pushed ({notebook_id: [state]}).
    pub cells: serde_json::Value,
    /// Paths the runtime listed as stopped for being idle, as last pushed.
    pub idle_stopped: HashSet<String>,
    /// Bumped when the runtime starts or goes, so an old runtime's event reader stops.
    watching: Arc<AtomicU64>,
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
    Connected(Result<(Arc<Channel>, Hello, Option<Askpass>), String>),
    Started(Result<Runtime, String>),
    Notice(Notice),
}

impl Connection {
    fn new(id: u64, status: Status, steps: Steps) -> Connection {
        Connection {
            status,
            id,
            channel: None,
            hello: None,
            runtime: None,
            start_when_connected: false,
            _askpass: None,
            cancel: Arc::default(),
            steps,
            last_notebooks: Vec::new(),
            notebooks: serde_json::Value::Null,
            cells: serde_json::Value::Null,
            idle_stopped: HashSet::new(),
            watching: Arc::default(),
        }
    }

    pub fn bridge(&self) -> Option<Bridge> {
        self.runtime.as_ref().map(|r| r.bridge.clone())
    }

    /// Stop following the runtime that's going; `last_notebooks` stays for the reopen.
    fn forget_runtime(&mut self) {
        self.runtime = None;
        self.watching.fetch_add(1, Ordering::SeqCst);
        self.notebooks = serde_json::Value::Null;
        self.cells = serde_json::Value::Null;
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

    /// The runtime of the session `key`'s host.
    pub fn session_bridge(&self, key: u64) -> Option<Bridge> {
        let session = self.sessions.iter().find(|s| s.key == key)?;
        self.bridge(&session.place.host)
    }

    fn listener(&mut self, host: &HostId) -> Result<Arc<Listener>, String> {
        if let Some(listener) = self.listeners.get(host) {
            return Ok(listener.clone());
        }
        let listener = Listener::start()?;
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
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
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
        }
        let cancel = connection.cancel.clone();
        self.connections.insert(host.clone(), connection);
        let (tx, rx) = futures::channel::mpsc::unbounded::<Update>();
        match host {
            HostId::ThisMac => {
                let keep_running = self.settings.keep_running;
                std::thread::spawn(move || {
                    let progress = |p: Progress| drop(tx.unbounded_send(Update::Local(p)));
                    let result = runtime::connect(keep_running, &progress).map(|(channel, hello)| (Arc::new(channel), hello, None));
                    let _ = tx.unbounded_send(Update::Connected(result));
                });
            }
            HostId::Server(server_id) => {
                let Some(server) = self.hosts.server(server_id).cloned() else {
                    return self.on_update(host.clone(), id, Update::Connected(Err("This server was removed.".into())), cx);
                };
                let questions = self.questions_tx.clone();
                std::thread::spawn(move || {
                    let on = |event| drop(tx.unbounded_send(Update::Event(event)));
                    let result = Askpass::start(server.name.clone(), questions).and_then(|askpass| {
                        let transport = remote::Transport::for_server(&server);
                        remote::connect(&server, &transport, Some(&askpass), &cancel, &on).map(|(channel, hello)| (Arc::new(channel), hello, Some(askpass)))
                    });
                    let _ = tx.unbounded_send(Update::Connected(result));
                });
            }
        }
        self.follow(host.clone(), id, rx, cx);
        cx.notify();
    }

    /// Start Julia on a connected host (or attach to the one running there).
    pub fn start_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Ok(listener) = self.listener(host) else { return };
        let Some(connection) = self.connections.get_mut(host) else { return };
        if !matches!(connection.status, Status::Browsing | Status::Died(_)) {
            return;
        }
        let Some(channel) = connection.channel.clone() else { return };
        connection.status = Status::Starting;
        connection.steps = match host {
            HostId::ThisMac => Steps::new("Starting Julia"),
            HostId::Server(_) => Steps { done: vec![format!("Connected to {}", self.hosts.name(host))], ..Steps::new("Finding Julia") },
        };
        let id = connection.id;
        let (tx, rx) = futures::channel::mpsc::unbounded::<Update>();
        let notices = tx.clone();
        let local = *host == HostId::ThisMac;
        std::thread::spawn(move || {
            let notice = move |notice| drop(notices.unbounded_send(Update::Notice(notice)));
            let result = if local {
                let progress = |p: Progress| drop(tx.unbounded_send(Update::Local(p)));
                runtime::start_local(&channel, &listener, &progress, notice)
            } else {
                let on = |event| drop(tx.unbounded_send(Update::Event(event)));
                remote::start(&channel, &listener, &on, notice)
            };
            let _ = tx.unbounded_send(Update::Started(result));
        });
        self.follow(host.clone(), id, rx, cx);
        cx.notify();
    }

    /// Stop This Mac's Julia and start it again (Settings → Restart Julia).
    pub fn restart_local(&mut self, cx: &mut Context<Self>) {
        let host = HostId::ThisMac;
        let Some(connection) = self.connections.get_mut(&host) else { return self.connect_host(&host, true, cx) };
        let Some(channel) = connection.channel.clone().filter(|_| connection.status == Status::Ready) else { return };
        connection.forget_runtime();
        connection.status = Status::Starting;
        connection.steps = Steps::new("Stopping Julia");
        self.status = "Restarting Julia…".into();
        let stop = cx.background_executor().spawn(async move { channel.stop() });
        cx.spawn(async move |this, cx| {
            stop.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(connection) = this.connections.get_mut(&host) {
                    connection.status = Status::Died(String::new());
                }
                this.start_host(&host, cx);
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
                if this.update(cx, |this, cx| this.on_update(host.clone(), id, update, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_update(&mut self, host: HostId, id: u64, update: Update, cx: &mut Context<Self>) {
        let local = host == HostId::ThisMac;
        let name = self.hosts.name(&host);
        let Some(connection) = self.connections.get_mut(&host).filter(|c| c.id == id) else { return };
        match update {
            Update::Event(Event::Connected { .. }) => connection.steps.advance(format!("Connected to {name}"), "Checking Endeavor's helper"),
            Update::Event(Event::Helper { installed }) => {
                connection.steps.now(if installed { "Installed Endeavor's helper" } else { "Connecting" });
            }
            Update::Event(Event::FoundJulia { version, .. }) => {
                connection.steps.found_julia = true;
                connection.steps.advance(format!("Julia {version}"), "Starting Julia");
            }
            Update::Event(Event::Progress(line)) => connection.steps.log(&line),
            Update::Event(Event::Started { .. } | Event::Finished { .. }) => {}
            Update::Local(p) => {
                if connection.status == Status::Starting && p.log {
                    connection.steps.found_julia = true;
                    connection.steps.log(&p.detail);
                }
                if matches!(connection.status, Status::Connecting | Status::Starting) {
                    self.on_progress(p, cx);
                }
            }
            Update::Connected(Ok((channel, hello, askpass))) => {
                connection.channel = Some(channel);
                connection.hello = Some(hello);
                connection._askpass = askpass;
                connection.status = Status::Browsing;
                if !local && connection.steps.done.is_empty() {
                    connection.steps.advance(format!("Connected to {name}"), "Finding Julia");
                }
                let start = connection.start_when_connected;
                self.on_connected(&host, cx);
                if start {
                    self.start_host(&host, cx);
                }
            }
            Update::Connected(Err(e)) => {
                connection.status = Status::Failed(e.clone());
                if local {
                    self.status = format!("⚠ {e}").into();
                    if let Some(setup) = &mut self.setup {
                        setup.fail(e);
                    }
                }
            }
            Update::Started(Ok(runtime)) => {
                connection.steps.found_julia = true;
                let started = if runtime.reattached { format!("Julia running on {}", runtime.node) } else { "Started Julia".to_owned() };
                connection.steps.advance(started, "Opening the notebook");
                connection.status = Status::Ready;
                connection.runtime = Some(runtime);
                self.on_ready(&host, cx);
            }
            Update::Started(Err(e)) => {
                connection.status = Status::Died(e.clone());
                if local {
                    self.status = format!("⚠ {e}").into();
                    if let Some(setup) = &mut self.setup {
                        setup.fail(e);
                    }
                }
            }
            Update::Notice(notice) => {
                connection.forget_runtime();
                connection.status = match notice {
                    Notice::Died(reason) => Status::Died(reason),
                    Notice::Replaced => {
                        connection.channel = None;
                        Status::Replaced
                    }
                    Notice::Lost(reason) => {
                        connection.channel = None;
                        Status::Failed(reason)
                    }
                };
                if local {
                    self.status = match &connection.status {
                        Status::Replaced => "⚠ Another connection took over Julia.\nNotebook tools are unavailable until you reconnect.".into(),
                        Status::Died(reason) => format!("⚠ Julia on This Mac stopped. {reason}\nNotebook tools are unavailable until Julia restarts."),
                        Status::Failed(reason) => format!("⚠ {reason}\nNotebook tools are unavailable until Julia restarts."),
                        _ => String::new(),
                    }
                    .into();
                }
            }
        }
        cx.notify();
    }

    /// Connected, before any runtime: the new-session screen can use its files.
    fn on_connected(&mut self, host: &HostId, cx: &mut Context<Self>) {
        if self.draft.host == *host {
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
        let Some(bridge) = self.bridge(host) else { return };
        let reattached = self.connections.get(host).and_then(|c| c.runtime.as_ref()).is_some_and(|r| r.reattached);
        if *host == HostId::ThisMac {
            if let Some(commands) = self.agent_rx.take() {
                self.on_progress(Progress::new(Step::Agent, "Pluto ready · starting Claude…"), cx);
                self.start_agent(commands, cx);
            } else {
                self.status = if reattached { "Reconnected to Julia." } else { "Julia restarted." }.into();
            }
        }
        self.send_idle_limit(host, cx);
        if self.active_session().is_some_and(|s| s.place.host == *host) {
            self.follow_folder(cx);
        }
        // A new runtime knows no session's notebook or policy.
        let on_host: Vec<&Session> = self.sessions.iter().filter(|s| s.place.host == *host).collect();
        let bound: Vec<(u64, String)> = on_host.iter().filter_map(|s| Some((s.key, s.notebook_path.clone()?))).collect();
        let folders: Vec<(u64, std::path::PathBuf)> = on_host.iter().map(|s| (s.key, s.place.path.clone())).collect();
        let planning: Vec<u64> = on_host.iter().filter(|s| s.policy() == "plan").map(|s| s.key).collect();
        let waiting: Vec<u64> = on_host.iter().filter(|s| s.agent_waiting).map(|s| s.key).collect();
        for (key, path) in bound {
            let bridge = bridge.clone();
            cx.background_executor().spawn(async move { pluto::set_notebook(&bridge, key, &path) }).detach();
        }
        for (key, folder) in folders {
            let bridge = bridge.clone();
            cx.background_executor().spawn(async move { pluto::set_session_folder(&bridge, key, &folder) }).detach();
        }
        for key in planning {
            self.send_policy(key, "plan", cx);
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
        std::thread::spawn(move || {
            while watching.load(Ordering::SeqCst) == mine {
                let _ = pluto::watch_notebooks(&bridge, |event| {
                    let _ = tx.unbounded_send(event);
                });
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
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
        for (path, hours, safe_preview) in &stops {
            if connection.idle_stopped.contains(path) {
                continue;
            }
            let stopped = Stopped { safe_preview: *safe_preview, idle_hours: Some(*hours) };
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
        connection.notebooks = list;
        for (notebook, cell, name) in pluto::user_edits(&connection.cells, &event["cells"]) {
            for session in self.sessions.iter_mut().filter(|s| s.notebook.as_deref() == Some(notebook.as_str())) {
                session.note_user_edit(cell.clone(), name.clone());
            }
        }
        connection.cells = event["cells"].take();
        self.push_cells(cx);
    }

    /// Reopen the notebooks that were open in `host`'s last runtime (unless this
    /// one has them open already, as after a reconnect), and point each session
    /// (and the pane) at the reopened copy. Pluto saves on every change, so the
    /// files are current.
    fn reopen_notebooks(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(connection) = self.connections.get(host) else { return };
        let Some(bridge) = connection.bridge() else { return };
        let reattached = connection.runtime.as_ref().is_some_and(|r| r.reattached);
        let before = connection.last_notebooks.clone();
        if before.is_empty() {
            return;
        }
        let paths: Vec<String> = before.iter().map(|(_, p)| p.clone()).collect();
        let reopen = cx.background_executor().spawn(async move {
            let listed = pluto::call_tool(&bridge, "list_notebooks", serde_json::json!({})).ok();
            let open = |path: &str| listed.as_ref()?.as_array()?.iter().find(|nb| nb["path"] == path).cloned();
            paths
                .into_iter()
                .filter_map(|path| {
                    let result = match open(&path) {
                        Some(nb) => nb,
                        None => pluto::call_tool(&bridge, "open_notebook", serde_json::json!({ "path": path })).ok()?,
                    };
                    Some((result["notebook_id"].as_str()?.to_owned(), path))
                })
                .collect::<Vec<_>>()
        });
        let host = host.clone();
        cx.spawn(async move |this, cx| {
            let reopened = reopen.await;
            let _ = this.update(cx, |this, cx| {
                let new_id = |old: &str| {
                    let path = before.iter().find(|(id, _)| id == old).map(|(_, p)| p)?;
                    reopened.iter().find(|(_, p)| p == path).map(|(id, _)| id.clone())
                };
                for session in this.sessions.iter_mut().filter(|s| s.place.host == host) {
                    session.notebook = session.notebook.as_deref().and_then(new_id);
                }
                if let Some((host, id)) = this.active_session().and_then(|s| Some((s.place.host.clone(), s.notebook.clone()?))) {
                    this.load_notebook(&host, &id, cx);
                }
                if host == HostId::ThisMac && !reopened.is_empty() && !reattached {
                    this.status = format!("Julia restarted; reopened {} notebook(s) in safe preview.", reopened.len()).into();
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

    /// Leave a host (its server was removed): its runtime keeps running.
    pub fn disconnect_host(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let Some(mut connection) = self.connections.remove(host) else { return };
        connection.forget_runtime();
        if let Some(channel) = connection.channel.take() {
            cx.background_executor().spawn(async move { channel.detach() }).detach();
        }
        cx.notify();
    }

    /// The app is quitting: servers' runtimes keep running (their idle stop
    /// covers them); This Mac's as Settings says. The helpers do the rest.
    pub fn quit_runtimes(&self) {
        for (host, connection) in &self.connections {
            if let Some(channel) = &connection.channel {
                channel.quit(*host != HostId::ThisMac || self.settings.keep_running);
            }
        }
    }

    /// The notebook pane while `host` isn't ready: its state, and what to do about it.
    pub fn host_pane(&self, host: &HostId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let connection = self.connections.get(host);
        let status = connection.map_or(Status::Connecting, |c| c.status.clone());
        let name = self.hosts.name(host);
        let action = |id: &'static str, label: &'static str, host: HostId, start: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .px_3()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .bg(theme::accent())
                .text_color(theme::text_primary())
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if start {
                        this.start_host(&host, cx);
                    } else {
                        this.connect_host(&host, true, cx);
                    }
                }))
        };
        let message = |text: String| div().max_w(px(420.)).text_center().text_size(theme::size_meta()).text_color(theme::text_muted()).child(text);
        let resting = crate::new_session::turtle_pane();
        let pane = match status {
            Status::Ready => return None,
            Status::Connecting | Status::Browsing | Status::Starting => {
                let steps = connection.map_or_else(|| Steps::new(format!("Connecting to {name}")), |c| c.steps.clone());
                return Some(starting_pane(&steps).into_any_element());
            }
            Status::Died(reason) => {
                let headline = if reason.is_empty() { format!("Julia on {name} is stopped.") } else { format!("Julia on {name} stopped.") };
                resting.child(div().text_color(theme::text_muted()).child(headline)).when(!reason.is_empty(), |d| d.child(message(reason))).child(action("host-start", "Start Julia", host.clone(), true))
            }
            Status::Replaced => resting
                .child(div().text_color(theme::text_muted()).child(format!("Another connection took over Julia on {name}.")))
                .child(action("host-reconnect", "Reconnect", host.clone(), false)),
            Status::Failed(reason) => resting
                .child(div().text_color(theme::text_muted()).child(format!("Not connected to {name}.")))
                .child(message(reason))
                .child(action("host-reconnect", "Reconnect", host.clone(), false)),
        };
        Some(pane.into_any_element())
    }
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

#[cfg(test)]
mod tests {
    use super::{Steps, elapsed, percent};
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
    fn elapsed_reads_like_a_clock() {
        let ago = |s: u64| Instant::now() - Duration::from_secs(s);
        assert_eq!(elapsed(ago(12)), "0:12");
        assert_eq!(elapsed(ago(62)), "1:02");
        assert_eq!(elapsed(ago(3723)), "1:02:03");
    }
}
