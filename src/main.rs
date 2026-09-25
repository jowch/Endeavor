//! Endeavor: app-owned Julia running Pluto + EndeavorRuntime, the live Pluto frontend in a
//! child webview, and ACP agent sessions (a session bar, one chat pane) wired to the
//! same Pluto session over MCP.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod agent;
mod annotate;
mod celldiff;
mod gate;
mod install;
mod logs;
mod outbox;
mod pluto;
mod runtime;
mod session;
mod settings;
mod splash;

use agent::{AgentEvent, Command};
use agent_client_protocol::schema::v1::{ContentBlock, SessionId, SessionInfo, TextContent};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::radio::Radio;
use gpui_component::{Root, Sizable, Theme, ThemeMode};
use gpui_wry::WebView;
use outbox::Queued;
use raw_window_handle::HasWindowHandle;
use runtime::Runtime;
use session::{Effect, Session, folder_name};
use settings::Settings;
use splash::{Progress, Setup, Step};

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

/// A small JSON file in Endeavor's Application Support folder.
fn app_file(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Application Support/endeavor").join(name))
}

fn load_json<T: serde::de::DeserializeOwned + Default>(name: &str) -> T {
    let text = app_file(name).and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_json(name: &str, value: &impl serde::Serialize) {
    // ponytail: best effort; losing these lists only costs a folder pick or a filter.
    if let (Some(file), Ok(json)) = (app_file(name), serde_json::to_string_pretty(value)) {
        let _ = std::fs::create_dir_all(file.parent().unwrap()).and_then(|_| std::fs::write(file, json));
    }
}

/// A checkbox row. Drawn here: gpui-component's Checkbox needs an icon asset set
/// for its check mark, which the app doesn't ship.
fn check_row(id: &'static str, checked: bool, label: &'static str) -> Stateful<Div> {
    let mark = div()
        .size_4()
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .border_1()
        .border_color(rgb(0x6a6a6a))
        .text_xs()
        .when(checked, |d| d.bg(rgb(0x2f5d3a)).border_color(rgb(0x2f5d3a)).child("✓"));
    div().id(id).flex().items_center().gap_2().cursor_pointer().child(mark).child(label)
}

/// Past sessions listed per folder before "Show more".
const PAST_SHOWN: usize = 8;

/// A small button that appears when the pointer is over its row (`group`).
fn hover_button(
    id: impl Into<ElementId>,
    group: SharedString,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_1()
        .text_color(gpui::transparent_black())
        .group_hover(group, |s| s.text_color(rgb(0x8a8a8a)))
        .hover(|s| s.text_color(rgb(0xe0e0e0)))
        .child(label)
        .on_click(on_click)
}

/// Recently used working folders, most recent first, kept across launches.
fn load_recent() -> Vec<PathBuf> {
    load_json::<Vec<PathBuf>>("recent.json").into_iter().filter(|p| p.is_dir()).collect()
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
    /// Ids of sessions Endeavor created (persisted). Only these are listed: Claude
    /// Code's history for a folder also holds CLI sessions, which aren't ours.
    ours: HashSet<String>,
    /// Session names the user gave, by session id (persisted).
    titles: HashMap<String, String>,
    /// The open session being renamed, and its name box.
    renaming: Option<(u64, Entity<InputState>)>,
    /// A past session whose delete button was clicked once; the second click deletes.
    confirm_delete: Option<SessionId>,
    /// Folders showing all their past sessions, not just the newest.
    expanded: HashSet<PathBuf>,
    settings: Settings,
    /// The Settings screen is in the chat pane.
    settings_open: bool,
    /// First launch: the setup screen covers the window until setup finishes.
    setup: Option<Setup>,
    /// The agent connected (setup's last step).
    agent_ready: bool,
    /// Claude Code's sign-in state, checked when the agent starts.
    signed_in: Option<bool>,
    /// A browser sign-in is under way.
    signing_in: bool,
    sign_in_error: Option<String>,
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
    /// The runtime's notebook list as last pushed (`list_notebooks` shape).
    notebooks: serde_json::Value,
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

        // Tick the "Working · 12s" timers once a second while any session is busy.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok(busy) = this.update(cx, |this, _| this.sessions.iter().any(|s| s.busy_since.is_some())) else { break };
            if busy {
                let _ = this.update(cx, |_, cx| cx.notify());
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
            ours: load_json("sessions.json"),
            titles: load_json("titles.json"),
            renaming: None,
            confirm_delete: None,
            expanded: HashSet::new(),
            settings: Settings::load(),
            settings_open: false,
            setup: Setup::needed().then(Setup::default),
            agent_ready: false,
            signed_in: None,
            signing_in: false,
            sign_in_error: None,
            agent_tx,
            agent_rx: Some(agent_rx),
            status: "".into(),
            annotating: false,
            runtime: None,
            starting: false,
            last_ports: None,
            last_notebooks: Vec::new(),
            notebooks: serde_json::Value::Null,
            died_tx,
        };
        // The webview is a native view over the window; hide it behind the setup screen.
        if this.setup.is_some() {
            let _ = this.webview.read(cx).raw().set_visible(false);
        }
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

    fn update_settings(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings);
        self.settings.save();
        cx.notify();
    }

    fn choose_julia(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use this julia".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = picked.await {
                let _ = this.update(cx, |this, cx| this.update_settings(cx, |s| s.julia = paths.pop()));
            }
        })
        .detach();
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
        save_json("recent.json", &self.recent);
        if !self.past.contains_key(&cwd) {
            let _ = self.agent_tx.unbounded_send(Command::ListSessions { cwd: cwd.clone() });
        }
        let _ = self.agent_tx.unbounded_send(Command::NewSession { key, cwd: cwd.clone() });
        let mut session = Session::new(key, cwd);
        session.run_without_asking = self.settings.run_without_asking;
        self.sessions.push(session);
        self.active = Some(key);
        self.follow_folder();
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
        let named = self.titles.get(&info.session_id.to_string()).cloned();
        let title = named.clone().or(info.title.clone()).unwrap_or_else(|| "Earlier session".into());
        let _ = self.agent_tx.unbounded_send(Command::LoadSession { key, id: info.session_id.clone(), cwd: info.cwd.clone() });
        let mut session = Session::loading(key, info.session_id, info.cwd, title);
        session.named = named.is_some();
        session.run_without_asking = self.settings.run_without_asking;
        self.sessions.push(session);
        self.activate(key, cx);
    }

    /// Stop an open session; it goes back to its folder's history.
    fn close_session(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.sessions.iter().position(|s| s.key == key) else { return };
        let session = self.sessions.remove(ix);
        if let Some(id) = session.id {
            let _ = self.agent_tx.unbounded_send(Command::CloseSession(id));
        }
        let _ = self.agent_tx.unbounded_send(Command::ListSessions { cwd: session.cwd });
        if self.renaming.as_ref().is_some_and(|(k, _)| *k == key) {
            self.renaming = None;
        }
        if self.active == Some(key) {
            match self.sessions.get(ix.min(self.sessions.len().saturating_sub(1))).map(|s| s.key) {
                Some(next) => self.activate(next, cx),
                None => self.active = None,
            }
        }
        cx.notify();
    }

    /// Delete a past session's history; the first click only asks to confirm.
    fn delete_past(&mut self, id: SessionId, cwd: PathBuf, cx: &mut Context<Self>) {
        if self.confirm_delete.as_ref() != Some(&id) {
            self.confirm_delete = Some(id);
            return cx.notify();
        }
        self.confirm_delete = None;
        let _ = self.agent_tx.unbounded_send(Command::DeleteSession(id.clone()));
        if let Some(past) = self.past.get_mut(&cwd) {
            past.retain(|info| info.session_id != id);
        }
        self.ours.remove(&id.to_string());
        save_json("sessions.json", &self.ours);
        if self.titles.remove(&id.to_string()).is_some() {
            save_json("titles.json", &self.titles);
        }
        cx.notify();
    }

    fn start_rename(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.sessions.iter().find(|s| s.key == key).map(|s| s.title.clone()) else { return };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.finish_rename(cx);
            }
        })
        .detach();
        self.renaming = Some((key, input));
        cx.notify();
    }

    /// Keep the typed name (an empty one leaves the title as it was).
    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some((key, input)) = self.renaming.take() else { return };
        let name = input.read(cx).value().trim().to_string();
        if let (false, Some(session)) = (name.is_empty(), self.session_mut(key)) {
            session.title = name.clone();
            session.named = true;
            if let Some(id) = session.id.as_ref().map(ToString::to_string) {
                self.titles.insert(id, name);
                save_json("titles.json", &self.titles);
            }
        }
        cx.notify();
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
        self.settings_open = false;
        self.confirm_delete = None;
        self.follow_folder();
        if let Some(notebook) = self.active_session().and_then(|s| s.notebook.clone()) {
            self.load_notebook(&notebook, cx);
        }
        cx.notify();
    }

    /// Pluto's new notebooks start unsaved; point its "Save notebook" suggestion
    /// at the active session's folder (the page picks it up on its next load).
    fn follow_folder(&mut self) {
        let Some(cwd) = self.active_session().map(|s| s.cwd.clone()) else { return };
        if let Some(runtime) = &mut self.runtime {
            runtime.suggest_folder(&cwd);
        }
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
                self.agent_ready = true;
                self.finish_setup(cx);
                for cwd in &self.recent {
                    let _ = self.agent_tx.unbounded_send(Command::ListSessions { cwd: cwd.clone() });
                }
            }
            AgentEvent::Setup(p) => self.on_progress(p, cx),
            AgentEvent::SignedIn(signed_in) => {
                self.signed_in = Some(signed_in);
                if !signed_in {
                    self.on_progress(Progress::new(Step::Claude, "Sign in to continue"), cx);
                    self.status = "Not signed in to Claude.".into();
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
                if let Some(setup) = &mut self.setup {
                    setup.error = Some(e.clone());
                }
                for session in &mut self.sessions {
                    session.note(format!("⚠ Agent stopped: {e}"));
                }
            }
            AgentEvent::Started { key, result } => {
                let Some(session) = self.session_mut(key) else { return };
                match result {
                    Ok(id) => {
                        if self.ours.insert(id.to_string()) {
                            save_json("sessions.json", &self.ours);
                        }
                        // Renamed before the agent assigned an id.
                        if let Some(name) = self.session_mut(key).filter(|s| s.named).map(|s| s.title.clone()) {
                            self.titles.insert(id.to_string(), name);
                            save_json("titles.json", &self.titles);
                        }
                        let Some(session) = self.session_mut(key) else { return };
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
        // A statement block, not `({ … })`, which would parse as an object literal.
        let _ = self.webview.read(cx).raw().evaluate_script(&format!("if (window.__annotate) {{ {js} }}"));
    }

    // -----------------------------------------------------------------------
    // Julia runtime
    // -----------------------------------------------------------------------

    /// Start Julia, or restart it on the previous ports.
    fn boot(&mut self, ports: Option<[u16; 2]>, cx: &mut Context<Self>) {
        self.starting = true;
        self.status = if ports.is_some() { "Restarting Julia…" } else { "Starting Julia…" }.into();
        let died = self.died_tx.clone();
        // First run downloads Julia, then instantiates + precompiles (~1 min); later launches are seconds.
        let (progress_tx, mut progress) = futures::channel::mpsc::unbounded::<Progress>();
        let boot = cx.background_executor().spawn(async move { runtime::start(ports, died, progress_tx) });
        cx.spawn(async move |this, cx| {
            while let Some(p) = progress.next().await {
                // Julia's log keeps coming after it's up; only show it while starting.
                let _ = this.update(cx, |this, cx| {
                    if this.starting {
                        this.on_progress(p, cx);
                    }
                });
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = boot.await;
            let _ = this.update(cx, |this, cx| this.on_booted(result, cx));
        })
        .detach();
        cx.notify();
    }

    /// Setup is under way: the status line, and the setup screen on first launch.
    fn on_progress(&mut self, p: Progress, cx: &mut Context<Self>) {
        if !p.log {
            self.status = p.detail.clone().into();
        }
        if let Some(setup) = &mut self.setup {
            setup.apply(p);
        }
        cx.notify();
    }

    /// Setup is done once the agent is up and Claude is signed in.
    fn finish_setup(&mut self, cx: &mut Context<Self>) {
        if self.setup.is_some() && self.agent_ready && self.signed_in != Some(false) {
            self.setup = None;
            Setup::finish();
            let _ = self.webview.read(cx).raw().set_visible(true);
            cx.notify();
        }
    }

    fn sign_in(&mut self, console: bool, cx: &mut Context<Self>) {
        self.signing_in = true;
        self.sign_in_error = None;
        let signing = cx.background_executor().spawn(async move { agent::sign_in(console) });
        cx.spawn(async move |this, cx| {
            let result = signing.await;
            let _ = this.update(cx, |this, cx| {
                this.signing_in = false;
                match result {
                    Ok(()) => {
                        this.signed_in = Some(true);
                        this.status = "Signed in to Claude.".into();
                        this.finish_setup(cx);
                    }
                    Err(e) => this.sign_in_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Shown while Claude isn't signed in: on the setup screen, else in the session bar.
    fn render_sign_in(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.signed_in != Some(false) {
            return None;
        }
        let muted = rgb(0x8a8a8a);
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .text_sm()
                .bg(if primary { rgb(0x2f5d3a) } else { rgb(0x3a3a3c) })
                .child(label)
        };
        let waiting = self.signing_in.then(|| div().text_xs().text_color(muted).child("Waiting for you to finish in your browser…"));
        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded_md()
                .bg(rgb(0x252526))
                .child(div().text_sm().child("Sign in to Claude"))
                .child(div().text_xs().text_color(muted).child("Endeavor runs Claude Code with your account. Sign-in opens in your browser."))
                .child(button("sign-in-claude", "Claude subscription", true).on_click(cx.listener(|this, _, _, cx| this.sign_in(false, cx))))
                .child(
                    button("sign-in-console", "Anthropic Console (API billing)", false)
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in(true, cx))),
                )
                .children(waiting)
                .children(self.sign_in_error.clone().map(|e| div().text_xs().text_color(rgb(0xd16969)).child(e)))
                .into_any_element(),
        )
    }

    /// Retry the failed setup step: start Julia again, or the agent once Julia is up.
    pub fn retry_setup(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = &mut self.setup else { return };
        setup.error = None;
        match self.runtime.as_ref().map(|r| r.mcp_url.clone()) {
            None if !self.starting => self.boot(None, cx),
            None => {}
            Some(mcp_url) => {
                // The failed agent thread dropped its command channel; start with a new one.
                let (tx, rx) = futures::channel::mpsc::unbounded();
                self.agent_tx = tx;
                self.start_agent(mcp_url, rx, cx);
            }
        }
        cx.notify();
    }

    fn start_agent(&mut self, mcp_url: String, commands: UnboundedReceiver<Command>, cx: &mut Context<Self>) {
        let mut events = agent::start(mcp_url, commands);
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_booted(&mut self, result: Result<Runtime, String>, cx: &mut Context<Self>) {
        self.starting = false;
        let runtime = match result {
            Ok(runtime) => runtime,
            Err(e) => {
                self.status = format!("⚠ {e}").into();
                if let Some(setup) = &mut self.setup {
                    setup.error = Some(e);
                }
                return cx.notify();
            }
        };
        self.webview.update(cx, |w, _| w.load_url(&runtime.pluto_url));
        let mcp_url = runtime.mcp_url.clone();
        self.runtime = Some(runtime);
        self.follow_folder();
        if let Some(commands) = self.agent_rx.take() {
            self.on_progress(Progress::new(Step::Agent, "Pluto ready · starting Claude…"), cx);
            self.start_agent(mcp_url.clone(), commands, cx);
        } else {
            self.status = "Julia restarted.".into();
            // Captures the notebooks to reopen before the new list starts arriving.
            self.reopen_notebooks(cx);
        }
        self.watch_notebooks(mcp_url, cx);
        cx.notify();
    }

    fn on_runtime_died(&mut self, reason: String, cx: &mut Context<Self>) {
        self.last_ports = self.runtime.take().map(|r| r.ports);
        self.status = format!("⚠ {reason}\nNotebook tools are unavailable until Julia restarts.").into();
        cx.notify();
    }

    /// Follow the runtime's notebook list (pushed on every change) for the
    /// end-of-turn run check and for reopening notebooks after a crash.
    fn watch_notebooks(&self, mcp_url: String, cx: &mut Context<Self>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<serde_json::Value>();
        // A long-lived blocking read: its own thread, not the executor's pool. It ends
        // when Julia goes away; the next boot starts a new one.
        std::thread::spawn(move || {
            let _ = pluto::watch_notebooks(&mcp_url, |list| {
                let _ = tx.unbounded_send(list);
            });
        });
        cx.spawn(async move |this, cx| {
            while let Some(list) = rx.next().await {
                if this.update(cx, |this, _| this.remember_notebooks(list)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn remember_notebooks(&mut self, list: serde_json::Value) {
        self.last_notebooks = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|nb| Some((nb["notebook_id"].as_str()?.to_owned(), nb["path"].as_str()?.to_owned())))
            .collect();
        self.notebooks = list;
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
        // ponytail: warns about every open notebook, not only the ones this session touched.
        let warnings = pluto::run_warnings(&self.notebooks);
        self.with_session(key, cx, |s| warnings.into_iter().for_each(|w| s.note(format!("⚠ {w}"))));
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
                        let row: SharedString = format!("session-{key}").into();
                        let title = match &self.renaming {
                            Some((k, input)) if *k == key => div().flex_1().child(Input::new(input).xsmall()),
                            _ => div().flex_1().overflow_hidden().child(s.title.clone()),
                        };
                        div()
                            .id(ElementId::NamedInteger("session".into(), key))
                            .group(row.clone())
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_sm()
                            .when(self.active == Some(key), |d| d.bg(rgb(0x2d2d30)))
                            .child(div().text_color(color).child(dot))
                            .child(title)
                            .child(hover_button(("close", key), row, "×", cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_session(key, cx);
                            })))
                            // Double-click renames.
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                if e.click_count() >= 2 {
                                    this.start_rename(key, window, cx);
                                } else {
                                    this.activate(key, cx);
                                }
                            }))
                    })
                    .collect();
                // Past sessions not already open, newest first; the newest few unless expanded.
                let is_open = |info: &SessionInfo| self.sessions.iter().any(|s| s.id.as_ref() == Some(&info.session_id));
                let shown = |info: &SessionInfo| self.ours.contains(&info.session_id.to_string());
                let all: Vec<_> = self.past.get(folder).into_iter().flatten().filter(|info| !is_open(info) && shown(info)).collect();
                let expanded = self.expanded.contains(folder);
                let limit = if expanded { all.len() } else { PAST_SHOWN };
                let past: Vec<_> = all
                    .iter()
                    .take(limit)
                    .enumerate()
                    .map(|(i, info)| {
                        let id = info.session_id.to_string();
                        let title = self.titles.get(&id).cloned().or(info.title.clone()).unwrap_or_else(|| "Earlier session".into());
                        let date = info.updated_at.as_deref().map(|d| d.chars().take(10).collect::<String>()).unwrap_or_default();
                        let row: SharedString = format!("past-{}-{i}", folder.display()).into();
                        let confirming = self.confirm_delete.as_ref() == Some(&info.session_id);
                        let (open, delete, cwd) = ((*info).clone(), info.session_id.clone(), folder.clone());
                        let delete = cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.delete_past(delete.clone(), cwd.clone(), cx);
                        });
                        div()
                            .id(ElementId::Name(row.clone()))
                            .group(row.clone())
                            .flex()
                            .gap_2()
                            .px_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(muted)
                            .child(div().flex_1().overflow_hidden().child(title))
                            .child(date)
                            .map(|d| {
                                if confirming {
                                    let id = ElementId::Name(format!("{row}-confirm").into());
                                    d.child(div().id(id).text_color(rgb(0xd16969)).child("Delete?").on_click(delete))
                                } else {
                                    d.child(hover_button(ElementId::Name(format!("{row}-delete").into()), row, "×", delete))
                                }
                            })
                            .on_click(cx.listener(move |this, _, _, cx| this.open_past(open.clone(), cx)))
                    })
                    .collect();
                let more = (all.len() > PAST_SHOWN).then(|| {
                    let label = if expanded { "Show fewer".to_string() } else { format!("Show {} more", all.len() - PAST_SHOWN) };
                    let folder = folder.clone();
                    div()
                        .id(ElementId::Name(format!("more-{}", folder.display()).into()))
                        .px_2()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(rgb(0x6a8fb5))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&folder) {
                                this.expanded.insert(folder.clone());
                            }
                            cx.notify();
                        }))
                });
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().px_2().text_xs().text_color(muted).child(folder_name(folder)))
                    .children(open)
                    .children(past)
                    .children(more)
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
                        this.settings_open = false;
                        cx.notify();
                    })),
            )
            .child(div().id("sessions").flex_1().overflow_y_scroll().flex().flex_col().gap_3().children(groups))
            .child(
                div()
                    .id("settings")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(muted)
                    .when(self.settings_open, |d| d.bg(rgb(0x2d2d30)).text_color(rgb(0xdddddd)))
                    .child("⚙ Settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_open = true;
                        cx.notify();
                    })),
            )
            .child(div().text_xs().text_color(muted).child(self.status.clone()))
            .children(self.render_sign_in(cx))
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

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = rgb(0x8a8a8a);
        let s = &self.settings;
        let heading = |text: &'static str| div().mt_2().text_sm().child(text);
        let note = |text: String| div().pl_6().text_xs().text_color(muted).child(text);
        let own = s.julia.is_none();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .child(div().text_lg().child("Settings"))
            .child(heading("Claude"))
            .child(check_row("personal-claude", s.personal_claude, "Use my Claude Code setup").on_click(
                cx.listener(|this, _, _, cx| this.update_settings(cx, |s| s.personal_claude = !s.personal_claude)),
            ))
            .child(note(
                "Adds your user settings and MCP servers to Endeavor's plugin and the project's settings. Applies to new sessions."
                    .into(),
            ))
            .child(check_row("run-without-asking", s.run_without_asking, "Run notebook code without asking").on_click(
                cx.listener(|this, _, _, cx| this.update_settings(cx, |s| s.run_without_asking = !s.run_without_asking)),
            ))
            .child(note(
                "New sessions start as if you'd chosen \"Allow & stop asking\". Applies to new and reopened sessions.".into(),
            ))
            .child(heading("Julia"))
            .child(
                Radio::new("julia-own")
                    .checked(own)
                    .label(format!("Endeavor's Julia ({})", runtime::JULIA_VERSION))
                    .on_click(cx.listener(|this, _, _, cx| this.update_settings(cx, |s| s.julia = None))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Radio::new("julia-mine")
                            .checked(!own)
                            .label("My julia")
                            .on_click(cx.listener(|this, _, _, cx| this.choose_julia(cx))),
                    )
                    .child(
                        div()
                            .id("choose-julia")
                            .px_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_sm()
                            .bg(rgb(0x3a3a3c))
                            .child("Choose…")
                            .on_click(cx.listener(|this, _, _, cx| this.choose_julia(cx))),
                    ),
            )
            .children(s.julia.as_ref().map(|p| note(p.display().to_string())))
            // ponytail: no live switch; restarting Julia under running sessions needs
            // the old process gone before its ports are reused.
            .child(note("Takes effect the next time Julia starts (relaunching Endeavor, or Restart Julia).".into()))
            .child(heading("Troubleshooting"))
            .child(
                div()
                    .id("show-logs")
                    .self_start()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_sm()
                    .bg(rgb(0x3a3a3c))
                    .child("Show logs")
                    .on_click(|_, _, _| logs::reveal()),
            )
            .child(note("The log of this run and the one before, to attach to a bug report.".into()))
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
            .children(session::render_activity(session))
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
                            // Wrap rather than clip when the chips and Stop don't fit.
                            .flex_wrap()
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
        if let Some(setup) = &self.setup {
            let sign_in = self.render_sign_in(cx);
            return div().size_full().bg(rgb(0x1e1e1e)).text_color(rgb(0xdddddd)).child(splash::render(setup, sign_in, cx)).into_any_element();
        }
        let chat = match self.active.and_then(|key| self.sessions.iter().position(|s| s.key == key)) {
            _ if self.settings_open => self.render_settings(cx).into_any_element(),
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
            .into_any_element()
    }
}

fn main() {
    // Claude Code runs the plugin's execution-gate hook as `endeavor hook-pretool`.
    if std::env::args().nth(1).as_deref() == Some("hook-pretool") {
        gate::run_pretool_hook();
    }
    logs::start();
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
