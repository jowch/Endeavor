//! Endeavor: app-owned Julia running Pluto + PlutoMCP, the live Pluto frontend in a
//! child webview, and ACP agent sessions (a session bar, one chat pane) wired to the
//! same Pluto session over MCP.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod agent;
mod annotate;
mod celldiff;
mod gate;
mod outbox;
mod pluto;
mod runtime;
mod session;

use agent::{AgentEvent, Command};
use agent_client_protocol::schema::v1::{ContentBlock, SessionInfo, TextContent};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{InputEvent, Textarea, TextareaState};
use gpui_component::{Root, Theme, ThemeMode};
use gpui_wry::WebView;
use outbox::Queued;
use raw_window_handle::HasWindowHandle;
use runtime::Runtime;
use session::{Effect, Session, folder_name};

/// Notebook id from a Pluto `/edit?id=…` URL. Only the id is used: the URL also
/// carries Pluto's secret, which must never reach the agent.
fn viewed_notebook_id(url: &str) -> Option<&str> {
    let (path, query) = url.split_once('?')?;
    if !path.ends_with("/edit") {
        return None;
    }
    let id = query.split('&').find_map(|kv| kv.strip_prefix("id="))?;
    annotate::is_uuid(id).then_some(id)
}

actions!(endeavor, [Interrupt, ToggleAnnotation]);

/// Recently used working folders, most recent first, kept across launches.
fn recent_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Application Support/endeavor/recent.json"))
}

fn load_recent() -> Vec<PathBuf> {
    let text = recent_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    serde_json::from_str::<Vec<PathBuf>>(&text).unwrap_or_default().into_iter().filter(|p| p.is_dir()).collect()
}

fn save_recent(recent: &[PathBuf]) {
    // ponytail: best effort; losing the recent list only costs a folder pick.
    if let (Some(file), Ok(json)) = (recent_file(), serde_json::to_string_pretty(recent)) {
        let _ = std::fs::create_dir_all(file.parent().unwrap()).and_then(|_| std::fs::write(file, json));
    }
}

pub struct Workspace {
    webview: Entity<WebView>,
    /// The chat box; on the new-session screen it holds the optional first message.
    pub input: Entity<TextareaState>,
    sessions: Vec<Session>,
    /// The session in the chat pane; None shows the new-session screen.
    active: Option<u64>,
    next_key: u64,
    /// New-session screen: the chosen working folder.
    new_cwd: Option<PathBuf>,
    /// Working folders, most recent first (persisted).
    recent: Vec<PathBuf>,
    /// Past sessions per folder, from the agent's history.
    past: HashMap<PathBuf, Vec<SessionInfo>>,
    agent_tx: UnboundedSender<Command>,
    /// Handed to the agent thread once Julia is up (it needs the MCP URL).
    agent_rx: Option<UnboundedReceiver<Command>>,
    /// App-level status (Julia, agent connection), shown under the session bar.
    status: SharedString,
    annotating: bool,
    runtime: Option<Runtime>,
    /// Booting or restarting Julia.
    starting: bool,
    /// Ports of the runtime that died, reused on restart so the agent reconnects.
    last_ports: Option<[u16; 2]>,
    /// Open notebooks (id, path) as last seen, to reopen after a restart.
    last_notebooks: Vec<(String, String)>,
    died_tx: UnboundedSender<String>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (page_tx, mut page_rx) = futures::channel::mpsc::unbounded::<String>();
        let webview = cx.new(|cx| {
            let handle = window.window_handle().expect("window handle");
            let webview = wry::WebViewBuilder::new()
                .with_devtools(true)
                .with_initialization_script(annotate::SCRIPT)
                .with_ipc_handler(move |request| {
                    let _ = page_tx.unbounded_send(request.into_body());
                })
                .build_as_child(&handle)
                .expect("child webview");
            WebView::new(webview, window, cx)
        });

        cx.spawn(async move |this, cx| {
            while let Some(body) = page_rx.next().await {
                if this.update(cx, |this, cx| this.on_page_message(&body, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        // Enter sends (queued while Claude works), Cmd+Enter sends now, Shift+Enter is a newline.
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask Claude about the notebook…")
                .submit_on_enter(true)
                .auto_grow(1, 8)
        });
        cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary, shift: false } = event {
                if this.active.is_some() {
                    this.submit(input, *secondary, window, cx);
                } else {
                    this.start_session(window, cx);
                }
            }
        })
        .detach();

        let (died_tx, mut died_rx) = futures::channel::mpsc::unbounded::<String>();
        cx.spawn(async move |this, cx| {
            while let Some(reason) = died_rx.next().await {
                if this.update(cx, |this, cx| this.on_runtime_died(reason, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        // Keep the open-notebook list fresh so a restart can reopen them.
        // ponytail: 10 s poll of a loopback call; a Pluto open/close event would be exact.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(10)).await;
            let Ok(url) = this.update(cx, |this, _| this.runtime.as_ref().map(|r| r.mcp_url.clone())) else { break };
            let Some(url) = url else { continue };
            let list = cx
                .background_executor()
                .spawn(async move { pluto::call_tool(&url, "list_notebooks", serde_json::json!({})) })
                .await;
            if let Ok(list) = list {
                let _ = this.update(cx, |this, _| this.remember_notebooks(&list));
            }
        })
        .detach();

        let (agent_tx, agent_rx) = futures::channel::mpsc::unbounded();
        let mut recent = load_recent();
        if recent.is_empty() {
            recent.extend(std::env::current_dir().ok());
        }
        let mut this = Self {
            webview,
            input,
            sessions: Vec::new(),
            active: None,
            next_key: 1,
            new_cwd: recent.first().cloned(),
            recent,
            past: HashMap::new(),
            agent_tx,
            agent_rx: Some(agent_rx),
            status: "".into(),
            annotating: false,
            runtime: None,
            starting: false,
            last_ports: None,
            last_notebooks: Vec::new(),
            died_tx,
        };
        // Julia boots while the user picks a folder on the new-session screen.
        this.boot(None, cx);
        this
    }

    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    pub fn session_mut(&mut self, key: u64) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.key == key)
    }

    /// Run `f` on a session and repaint (for clicks in the transcript).
    pub fn with_session(&mut self, key: u64, cx: &mut Context<Self>, f: impl FnOnce(&mut Session)) {
        if let Some(session) = self.session_mut(key) {
            f(session);
            cx.notify();
        }
    }

    fn active_session(&self) -> Option<&Session> {
        self.active.and_then(|key| self.sessions.iter().find(|s| s.key == key))
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = picked.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.new_cwd = Some(path);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Start a session in the chosen folder, with the input's text (if any) as its first message.
    fn start_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cwd) = self.new_cwd.clone() else { return };
        let key = self.next_key;
        self.next_key += 1;
        self.recent.retain(|p| p != &cwd);
        self.recent.insert(0, cwd.clone());
        save_recent(&self.recent);
        if !self.past.contains_key(&cwd) {
            let _ = self.agent_tx.unbounded_send(Command::ListSessions { cwd: cwd.clone() });
        }
        let _ = self.agent_tx.unbounded_send(Command::NewSession { key, cwd: cwd.clone() });
        self.sessions.push(Session::new(key, cwd));
        self.active = Some(key);
        let text = self.input.read(cx).value().trim().to_string();
        if !text.is_empty() {
            self.submit(&self.input.clone(), false, window, cx);
        }
        cx.notify();
    }

    /// Reopen a past session (or switch to it if it's already open).
    fn open_past(&mut self, info: SessionInfo, cx: &mut Context<Self>) {
        if let Some(key) = self.sessions.iter().find(|s| s.id.as_ref() == Some(&info.session_id)).map(|s| s.key) {
            return self.activate(key, cx);
        }
        let key = self.next_key;
        self.next_key += 1;
        let title = info.title.clone().unwrap_or_else(|| "Earlier session".into());
        let _ = self.agent_tx.unbounded_send(Command::LoadSession { key, id: info.session_id.clone(), cwd: info.cwd.clone() });
        self.sessions.push(Session::loading(key, info.session_id, info.cwd, title));
        self.activate(key, cx);
    }

    /// Continue a session that couldn't be reopened (e.g. live in the CLI) as a copy.
    fn open_copy(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(key) else { return };
        let cwd = session.cwd.clone();
        if let Some(source) = session.reopen_as_copy() {
            let _ = self.agent_tx.unbounded_send(Command::ForkSession { key, source, cwd });
        }
        cx.notify();
    }

    /// Open a reopened session's notebook file in the current Pluto (reusing it if
    /// it's already open) and point the session, and the pane if active, at it.
    fn reopen_for_session(&mut self, key: u64, path: String, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        let opened = cx.background_executor().spawn(async move {
            let listed = pluto::call_tool(&mcp_url, "list_notebooks", serde_json::json!({})).ok();
            let open = listed.as_ref().and_then(|l| l.as_array()?.iter().find(|nb| nb["path"] == path.as_str()).cloned());
            let nb = match open {
                Some(nb) => nb,
                None => pluto::call_tool(&mcp_url, "open_notebook", serde_json::json!({ "path": path })).ok()?,
            };
            nb["notebook_id"].as_str().map(str::to_owned)
        });
        cx.spawn(async move |this, cx| {
            // ponytail: a notebook file that's gone just leaves the pane where it is.
            let Some(id) = opened.await else { return };
            let _ = this.update(cx, |this, cx| this.apply_effects(key, vec![Effect::ShowNotebook(id)], cx));
        })
        .detach();
    }

    /// Show a session; the notebook pane follows it to the notebook it last viewed.
    fn activate(&mut self, key: u64, cx: &mut Context<Self>) {
        self.active = Some(key);
        if let Some(notebook) = self.active_session().and_then(|s| s.notebook.clone()) {
            self.load_notebook(&notebook, cx);
        }
        cx.notify();
    }

    fn apply_effects(&mut self, key: u64, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Send(turn) => {
                    if let Some(id) = self.session_mut(key).and_then(|s| s.id.clone()) {
                        let _ = self.agent_tx.unbounded_send(Command::Turn(id, turn));
                    }
                }
                Effect::ShowNotebook(id) => {
                    if let Some(session) = self.session_mut(key) {
                        session.notebook = Some(id.clone());
                    }
                    if self.active == Some(key) {
                        self.load_notebook(&id, cx);
                    }
                }
                Effect::CheckRunState => self.check_run_state(key, cx),
                Effect::ReopenNotebook(path) => self.reopen_for_session(key, path, cx),
            }
        }
        cx.notify();
    }

    /// Which notebook the user is looking at, so "the notebook" is unambiguous.
    /// Also remembered as the active session's notebook.
    fn viewing_context(&mut self, cx: &mut Context<Self>) -> Option<ContentBlock> {
        let url = self.webview.read(cx).raw().url().unwrap_or_default();
        let id = viewed_notebook_id(&url)?.to_owned();
        if let Some(session) = self.active.and_then(|key| self.session_mut(key)) {
            session.notebook = Some(id.clone());
        }
        Some(ContentBlock::Text(TextContent::new(format!(
            "[Endeavor] The user is viewing Pluto notebook {id} in the notebook pane. \
             Unless they say otherwise, \"the notebook\" means this one."
        ))))
    }

    fn submit(&mut self, input: &Entity<TextareaState>, now: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active else { return };
        let text = input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut blocks: Vec<_> = self.viewing_context(cx).into_iter().collect();
        blocks.push(ContentBlock::Text(TextContent::new(text.clone())));
        let Some(session) = self.session_mut(key) else { return };
        let effects = session.submit(Queued::new(text.clone(), Some(text), blocks), now);
        self.apply_effects(key, effects, cx);
        input.update(cx, |s, cx| s.set_value("", window, cx));
    }

    fn interrupt(&mut self, _: &Interrupt, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active else { return };
        if let Some(effect) = self.active_session().and_then(Session::interrupt) {
            self.apply_effects(key, vec![effect], cx);
        }
    }

    fn on_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        match event {
            AgentEvent::Ready => {
                self.status = "Claude connected.".into();
                for cwd in &self.recent {
                    let _ = self.agent_tx.unbounded_send(Command::ListSessions { cwd: cwd.clone() });
                }
            }
            AgentEvent::Listed { cwd, sessions } => {
                self.past.insert(cwd, sessions);
            }
            AgentEvent::Forked { key, id } => {
                if let Some(session) = self.session_mut(key) {
                    session.id = Some(id);
                }
            }
            AgentEvent::Failed(e) => {
                self.status = format!("⚠ Agent stopped: {e}").into();
                for session in &mut self.sessions {
                    session.note(format!("⚠ Agent stopped: {e}"));
                }
            }
            AgentEvent::Started { key, result } => {
                let Some(session) = self.session_mut(key) else { return };
                match result {
                    Ok(id) => {
                        let effects = session.started(id);
                        self.apply_effects(key, effects, cx);
                    }
                    Err(e) => session.fail(&e),
                }
            }
            AgentEvent::Session(id, event) => {
                let Some(session) = self.sessions.iter_mut().find(|s| s.id.as_ref() == Some(&id)) else { return };
                let key = session.key;
                let effects = session.apply(event);
                self.apply_effects(key, effects, cx);
            }
        }
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Notebook pane and annotation mode
    // -----------------------------------------------------------------------

    fn load_notebook(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(runtime) = &self.runtime else { return };
        // pluto_url is `http://host:port/?secret=…`; keep the secret app-side.
        let url = runtime.pluto_url.replacen("/?", &format!("/edit?id={id}&"), 1);
        self.webview.update(cx, |w, _| w.load_url(&url));
    }

    fn on_page_message(&mut self, body: &str, cx: &mut Context<Self>) {
        match annotate::parse(body) {
            Some(annotate::Message::Mode(on)) => self.annotating = on,
            Some(annotate::Message::Annotation(a)) => {
                let Some(key) = self.active else { return };
                let comment = if a.comment.is_empty() { "(no comment)" } else { a.comment.as_str() };
                let label = format!("✎ {} cell{}: {comment}", a.cells.len(), if a.cells.len() > 1 { "s" } else { "" });
                let mut blocks: Vec<_> = self.viewing_context(cx).into_iter().collect();
                blocks.extend(annotate::prompt_blocks(std::slice::from_ref(&a)));
                let Some(session) = self.session_mut(key) else { return };
                let effects = session.submit(Queued::new(label, None, blocks), a.now);
                self.apply_effects(key, effects, cx);
            }
            None => return,
        }
        cx.notify();
    }

    /// Cmd+Shift+E from the panel (the page handles it when the notebook has focus).
    fn toggle_annotation(&mut self, _: &ToggleAnnotation, _: &mut Window, cx: &mut Context<Self>) {
        let enable = !self.annotating;
        if enable {
            // Move keyboard focus into the notebook so the comment box takes typing.
            let _ = self.webview.read(cx).raw().focus();
        }
        self.page_script(if enable { "__annotate.set(true)" } else { "__annotate.set(false)" }, cx);
    }

    fn page_script(&self, js: &str, cx: &mut Context<Self>) {
        let _ = self.webview.read(cx).raw().evaluate_script(&format!("window.__annotate && ({{ {js} }})"));
    }

    // -----------------------------------------------------------------------
    // Julia runtime
    // -----------------------------------------------------------------------

    /// Start Julia, or restart it on the previous ports.
    fn boot(&mut self, ports: Option<[u16; 2]>, cx: &mut Context<Self>) {
        self.starting = true;
        self.status = if ports.is_some() { "Restarting Julia…" } else { "Starting Julia…" }.into();
        let died = self.died_tx.clone();
        // First run instantiates + precompiles (~1 min); later launches are seconds.
        let boot = cx.background_executor().spawn(async move { runtime::start(ports, died) });
        cx.spawn(async move |this, cx| {
            let result = boot.await;
            let _ = this.update(cx, |this, cx| this.on_booted(result, cx));
        })
        .detach();
        cx.notify();
    }

    fn on_booted(&mut self, result: Result<Runtime, String>, cx: &mut Context<Self>) {
        self.starting = false;
        let runtime = match result {
            Ok(runtime) => runtime,
            Err(e) => {
                self.status = format!("⚠ {e}").into();
                return cx.notify();
            }
        };
        self.webview.update(cx, |w, _| w.load_url(&runtime.pluto_url));
        let mcp_url = runtime.mcp_url.clone();
        self.runtime = Some(runtime);
        if let Some(commands) = self.agent_rx.take() {
            self.status = "Pluto ready · connecting to Claude…".into();
            let mut events = agent::start(mcp_url, commands);
            cx.spawn(async move |this, cx| {
                while let Some(event) = events.next().await {
                    if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        } else {
            self.status = "Julia restarted.".into();
            self.reopen_notebooks(cx);
        }
        cx.notify();
    }

    fn on_runtime_died(&mut self, reason: String, cx: &mut Context<Self>) {
        self.last_ports = self.runtime.take().map(|r| r.ports);
        self.status = format!("⚠ {reason}\nNotebook tools are unavailable until Julia restarts.").into();
        cx.notify();
    }

    fn remember_notebooks(&mut self, list: &serde_json::Value) {
        self.last_notebooks = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|nb| Some((nb["notebook_id"].as_str()?.to_owned(), nb["path"].as_str()?.to_owned())))
            .collect();
    }

    /// Reopen the notebooks that were open when Julia died, and point each session
    /// (and the pane) at the reopened copy. Pluto saves on every change, so the
    /// files are current.
    fn reopen_notebooks(&mut self, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        let before = self.last_notebooks.clone();
        let paths: Vec<String> = before.iter().map(|(_, p)| p.clone()).collect();
        let reopen = cx.background_executor().spawn(async move {
            paths
                .into_iter()
                .filter_map(|path| {
                    let result = pluto::call_tool(&mcp_url, "open_notebook", serde_json::json!({ "path": path })).ok()?;
                    Some((result["notebook_id"].as_str()?.to_owned(), path))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let reopened = reopen.await;
            let _ = this.update(cx, |this, cx| {
                let new_id = |old: &str| {
                    let path = before.iter().find(|(id, _)| id == old).map(|(_, p)| p)?;
                    reopened.iter().find(|(_, p)| p == path).map(|(id, _)| id.clone())
                };
                for session in &mut this.sessions {
                    session.notebook = session.notebook.as_deref().and_then(new_id);
                }
                if let Some(id) = this.active_session().and_then(|s| s.notebook.clone()) {
                    this.load_notebook(&id, cx);
                }
                if !reopened.is_empty() {
                    this.status = format!("Julia restarted; reopened {} notebook(s) in safe preview.", reopened.len()).into();
                }
                this.last_notebooks = reopened;
                cx.notify();
            });
        })
        .detach();
    }

    /// After a session goes idle, say if it left edited cells unrun or still running.
    fn check_run_state(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        let list = cx
            .background_executor()
            .spawn(async move { pluto::call_tool(&mcp_url, "list_notebooks", serde_json::json!({})) });
        cx.spawn(async move |this, cx| {
            // ponytail: a failed check stays silent; it's advisory, and a dead runtime reports itself.
            let Ok(list) = list.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.remember_notebooks(&list);
                // ponytail: warns about every open notebook, not only the ones this session touched.
                let warnings = pluto::run_warnings(&list);
                this.with_session(key, cx, |s| warnings.into_iter().for_each(|w| s.note(format!("⚠ {w}"))));
            });
        })
        .detach();
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn render_session_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = rgb(0x8a8a8a);
        // Folders: recent ones, then any other folder with an open session.
        let mut folders: Vec<&PathBuf> = self.recent.iter().collect();
        for s in &self.sessions {
            if !folders.contains(&&s.cwd) {
                folders.push(&s.cwd);
            }
        }
        let groups: Vec<_> = folders
            .into_iter()
            .map(|folder| {
                let open: Vec<_> = self
                    .sessions
                    .iter()
                    .filter(|s| &s.cwd == folder)
                    .map(|s| {
                        let key = s.key;
                        let (dot, color) = if s.failed.is_some() {
                            ("×", rgb(0x6a6a6a))
                        } else if s.needs_approval() {
                            ("!", rgb(0xd16969))
                        } else if s.outbox.busy {
                            ("●", rgb(0xc8a040))
                        } else {
                            ("○", rgb(0x6a6a6a))
                        };
                        div()
                            .id(ElementId::NamedInteger("session".into(), key))
                            .flex()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_sm()
                            .when(self.active == Some(key), |d| d.bg(rgb(0x2d2d30)))
                            .child(div().text_color(color).child(dot))
                            .child(div().flex_1().overflow_hidden().child(s.title.clone()))
                            .on_click(cx.listener(move |this, _, _, cx| this.activate(key, cx)))
                    })
                    .collect();
                // Past sessions not already open, newest first.
                // ponytail: capped at 8 per folder; add "show more" if people want older ones.
                let is_open = |info: &SessionInfo| self.sessions.iter().any(|s| s.id.as_ref() == Some(&info.session_id));
                let past: Vec<_> = self
                    .past
                    .get(folder)
                    .into_iter()
                    .flatten()
                    .filter(|info| !is_open(info))
                    .take(8)
                    .enumerate()
                    .map(|(i, info)| {
                        let title = info.title.clone().unwrap_or_else(|| "Earlier session".into());
                        let date = info.updated_at.as_deref().map(|d| d.chars().take(10).collect::<String>()).unwrap_or_default();
                        let info = info.clone();
                        div()
                            .id(ElementId::Name(format!("past-{}-{i}", folder.display()).into()))
                            .flex()
                            .gap_2()
                            .px_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(muted)
                            .child(div().flex_1().overflow_hidden().child(title))
                            .child(date)
                            .on_click(cx.listener(move |this, _, _, cx| this.open_past(info.clone(), cx)))
                    })
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().px_2().text_xs().text_color(muted).child(folder_name(folder)))
                    .children(open)
                    .children(past)
            })
            .collect();

        div()
            .w(px(220.))
            .h_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_2()
            .border_r_1()
            .border_color(rgb(0x333333))
            .bg(rgb(0x191919))
            .child(
                div()
                    .id("new-session")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_sm()
                    .bg(rgb(0x2d2d30))
                    .child("+ New session")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.active = None;
                        cx.notify();
                    })),
            )
            .child(div().id("sessions").flex_1().overflow_y_scroll().flex().flex_col().gap_3().children(groups))
            .child(div().text_xs().text_color(muted).child(self.status.clone()))
            .when(self.runtime.is_none() && !self.starting, |d| {
                d.child(
                    div()
                        .id("restart")
                        .px_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_sm()
                        .bg(rgb(0x3a3a3c))
                        .child("↻ Restart Julia")
                        .on_click(cx.listener(|this, _, _, cx| this.boot(this.last_ports, cx))),
                )
            })
    }

    fn render_new_session(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = rgb(0x8a8a8a);
        let folder = self.new_cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Choose a folder…".into());
        let recent: Vec<_> = self
            .recent
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let path = path.clone();
                div()
                    .id(("recent", i))
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(muted)
                    .when(self.new_cwd.as_ref() == Some(&path), |d| d.text_color(rgb(0xdddddd)).bg(rgb(0x2d2d30)))
                    .child(format!("{}  ·  {}", folder_name(&path), path.display()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.new_cwd = Some(path.clone());
                        cx.notify();
                    }))
            })
            .collect();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(div().text_lg().child("New session"))
            .child(div().text_sm().text_color(muted).child("Working folder: where Claude works, whose CLAUDE.md applies, and where new notebooks go."))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().p_1().rounded_sm().bg(rgb(0x252526)).text_sm().overflow_hidden().child(folder))
                    .child(
                        div()
                            .id("choose-folder")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_sm()
                            .bg(rgb(0x3a3a3c))
                            .child("Choose…")
                            .on_click(cx.listener(|this, _, _, cx| this.choose_folder(cx))),
                    ),
            )
            .when(!recent.is_empty(), |d| d.child(div().text_xs().text_color(muted).child("Recent")).children(recent))
            .child(div().flex_1())
            .child(div().text_sm().text_color(muted).child("First message (optional)"))
            .child(Textarea::new(&self.input))
            .child(
                div()
                    .id("start-session")
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(if self.new_cwd.is_some() { rgb(0x2f5d3a) } else { rgb(0x3a3a3c) })
                    .child("Start session  ↩")
                    .on_click(cx.listener(|this, _, window, cx| this.start_session(window, cx))),
            )
    }

    fn render_chat(&self, session: &Session, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let key = session.key;
        let button = |id: &'static str| div().id(id).px_2().rounded_sm().cursor_pointer().text_sm();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x333333))
                    .text_sm()
                    .child(session.title.clone())
                    .child(div().text_xs().text_color(rgb(0x8a8a8a)).child(session.cwd.display().to_string())),
            )
            .children(session.failed.as_ref().map(|failure| {
                div()
                    .m_3()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0xd16969))
                    .text_sm()
                    .child(failure.message.clone())
                    .when(failure.can_copy, |d| {
                        d.child(
                            div()
                                .id("open-copy")
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(0x2f5d3a))
                                .child("Open a copy")
                                .on_click(cx.listener(move |this, _, _, cx| this.open_copy(key, cx))),
                        )
                    })
            }))
            .child(session::render_transcript(session, cx))
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(0x333333))
                    .child(session::render_queue(session, cx))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button("annotate-toggle")
                                    .when(self.annotating, |d| d.bg(rgb(0x8a6d1f)))
                                    .child(if self.annotating { "◉ Annotating (⌘⇧E exits)" } else { "◎ Annotate (⌘⇧E)" })
                                    .on_click(cx.listener(|this, _, window, cx| this.toggle_annotation(&ToggleAnnotation, window, cx))),
                            )
                            .when(session.run_without_asking, |d| {
                                d.child(
                                    button("ask-again")
                                        .bg(rgb(0x3a3a3c))
                                        .child("▶ Runs without asking ✕")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.with_session(key, cx, |s| {
                                                s.run_without_asking = false;
                                                s.note("Will ask before running code again.");
                                            })
                                        })),
                                )
                            })
                            .child(div().flex_1())
                            .when(session.outbox.busy && session.id.is_some(), |d| {
                                d.child(
                                    button("stop")
                                        .bg(rgb(0x5d2f2f))
                                        .child("■ Stop (Esc)")
                                        .on_click(cx.listener(|this, _, window, cx| this.interrupt(&Interrupt, window, cx))),
                                )
                            }),
                    )
                    .child(Textarea::new(&self.input)),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = match self.active.and_then(|key| self.sessions.iter().position(|s| s.key == key)) {
            Some(ix) => self.render_chat(&self.sessions[ix], cx).into_any_element(),
            None => self.render_new_session(cx).into_any_element(),
        };
        div()
            .key_context("Workspace")
            .on_action(cx.listener(Self::interrupt))
            .on_action(cx.listener(Self::toggle_annotation))
            .flex()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xdddddd))
            .child(self.render_session_bar(cx))
            .child(
                div()
                    .w(px(420.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(rgb(0x333333))
                    .child(chat),
            )
            .child(div().flex_1().h_full().child(self.webview.clone()))
    }
}

fn main() {
    // Claude Code runs the plugin's execution-gate hook as `endeavor hook-pretool`.
    if std::env::args().nth(1).as_deref() == Some("hook-pretool") {
        gate::run_pretool_hook();
    }
    gpui_platform::application().run(|cx: &mut App| {
        gpui_component::init(cx);
        // Input consumes Escape only when it has something to dismiss; otherwise it reaches us.
        cx.bind_keys([
            KeyBinding::new("escape", Interrupt, None),
            KeyBinding::new("cmd-shift-e", ToggleAnnotation, None),
        ]);
        let bounds = Bounds::centered(None, size(px(1560.), px(900.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                Theme::change(ThemeMode::Dark, Some(window), cx);
                let workspace = cx.new(|cx| Workspace::new(window, cx));
                cx.new(|cx| Root::new(workspace, window, cx))
            },
        )
        .unwrap();
        // Quitting closes Julia's stdin, which shuts the runtime down.
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::viewed_notebook_id;

    #[test]
    fn notebook_id_from_pluto_url() {
        let id = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let url = format!("http://127.0.0.1:1234/edit?secret=s3cr3t&id={id}");
        assert_eq!(viewed_notebook_id(&url), Some(id));
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/?secret=s3cr3t"), None);
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/edit?id=../../secret"), None);
    }
}
