//! Endeavor: app-owned Julia running Pluto + EndeavorRuntime, the live Pluto frontend in a
//! child webview, and ACP agent sessions (a session bar, one chat pane) wired to the
//! same Pluto session over MCP.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

mod webkeys;
mod agent;
mod annotate;
mod celldiff;
mod gate;
mod install;
mod logs;
mod new_session;
mod notebook_files;
mod outbox;
mod overlay;
mod pluto;
mod runtime;
mod session;
mod settings;
mod splash;
mod theme;
mod turtle;
mod when;

use agent::{AgentEvent, Command};
use agent_client_protocol::schema::v1::{ContentBlock, PermissionOptionKind, SessionId, SessionInfo, TextContent};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::radio::Radio;
use gpui_component::{Root, Sizable, Theme, ThemeConfig, ThemeMode};
use gpui_wry::WebView;
use outbox::Queued;
use new_session::{Draft, Glyph, NotebookChoice, glyph, menu_row};
use raw_window_handle::HasWindowHandle;
use runtime::Runtime;
use session::{Effect, Session, Stopped, folder_name};
use settings::{Appearance, NotebookTheme, Settings};
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

actions!(endeavor, [Interrupt, ToggleAnnotation, CycleMode, ToggleSidebar, OpenSettings, Quit, ZoomIn, ZoomOut, ZoomReset]);

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
        .border_color(theme::text_faint())
        .text_size(theme::size_meta_small())
        .when(checked, |d| d.bg(theme::accent()).border_color(theme::accent()).child("✓"));
    div().id(id).flex().items_center().gap_2().cursor_pointer().child(mark).child(label)
}

/// The range a divider can drag the panes to (defaults: settings::Layout).
const SIDEBAR_RANGE: (f32, f32) = (180., 400.);
const CHAT_MIN: f32 = 320.;
const NOTEBOOK_MIN: f32 = 360.;

#[derive(Clone, Copy, PartialEq)]
enum Divider {
    Sidebar,
    Chat,
}

/// The ⌘B button: a small drawn sidebar glyph. It sits in a header, so it stops
/// the mouse-down that would otherwise start a window move.
fn sidebar_toggle(cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("sidebar-toggle")
        .p(px(4.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::row_active()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(&ToggleSidebar, window, cx)))
        .child(
            div()
                .w(px(15.))
                .h(px(12.))
                .rounded(px(3.))
                .border_1()
                .border_color(theme::text_muted())
                .child(div().w(px(5.)).h_full().border_r_1().border_color(theme::text_muted())),
        )
}

/// The composer's context ring: a 14px circle filled clockwise by the share of
/// the context window used; orange once it's nearly full.
fn context_ring(fraction: f32) -> impl IntoElement {
    const SIZE: f32 = 14.;
    const WIDTH: f32 = 2.;
    let fraction = fraction.clamp(0., 1.);
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let r = px((SIZE - WIDTH) / 2.);
            // ponytail: arcs as 48-segment polylines; arc_to if it ever looks faceted.
            let arc = |to: f32| {
                let mut path = PathBuilder::stroke(px(WIDTH));
                let steps = ((48. * to).ceil() as usize).max(1);
                for i in 0..=steps {
                    let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * to * i as f32 / steps as f32;
                    let point = point(center.x + r * angle.cos(), center.y + r * angle.sin());
                    if i == 0 { path.move_to(point) } else { path.line_to(point) }
                }
                path.build().ok()
            };
            if let Some(track) = arc(1.) {
                window.paint_path(track, theme::border());
            }
            if fraction > 0.
                && let Some(used) = arc(fraction)
            {
                window.paint_path(used, if fraction >= 0.8 { theme::accent_text() } else { theme::text_secondary() });
            }
        },
    )
    .size(px(SIZE))
}

/// A 24px composer-toolbar button.
fn tool_button(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.))
        .flex()
        .items_center()
        .px(px(5.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::row_active()))
}
/// Room the traffic lights take at the start of a header.
const TRAFFIC_LIGHTS: f32 = 84.;

/// A 44px column header: the window's drag area (the title bar is transparent),
/// double-click zooms like a title bar.
fn column_header(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(44.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .on_mouse_down(MouseButton::Left, |e, window, _| {
            if e.click_count >= 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
}

/// A 28px sidebar row (sessions, "New session").
fn sidebar_row(id: ElementId, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2()
        .px(px(10.))
        .rounded(px(4.))
        .cursor_pointer()
        .text_color(if active { theme::text_row_active() } else { theme::text_muted() })
        .when(active, |d| d.bg(theme::row_active()))
        .hover(|s| s.bg(theme::row_active()))
}

/// Past sessions listed per folder before "Show more".
const PAST_SHOWN: usize = 8;

/// A session in the sidebar: an open one, or a past one listed under its folder.
#[derive(Clone, PartialEq)]
enum Row {
    Open(u64),
    Past(SessionId, PathBuf),
}

#[derive(Clone, Copy, PartialEq)]
enum RowAction {
    Rename,
    Reveal,
    Archive,
    Unarchive,
    Close,
    Delete,
}

impl RowAction {
    const ALL: [RowAction; 6] = [RowAction::Rename, RowAction::Reveal, RowAction::Archive, RowAction::Unarchive, RowAction::Close, RowAction::Delete];

    fn label(self) -> &'static str {
        match self {
            RowAction::Rename => "Rename",
            RowAction::Reveal => "Reveal folder in Finder",
            RowAction::Archive => "Archive",
            RowAction::Unarchive => "Unarchive",
            RowAction::Close => "Close",
            RowAction::Delete => "Delete…",
        }
    }

    /// The key that picks it while the menu is open (`Keystroke::key`), and how
    /// the menu shows that key.
    fn shortcut(self) -> (&'static str, &'static str) {
        match self {
            RowAction::Rename => ("r", "R"),
            RowAction::Reveal => ("f", "F"),
            RowAction::Archive | RowAction::Unarchive => ("a", "A"),
            RowAction::Close => ("c", "C"),
            RowAction::Delete => ("backspace", "⌫"),
        }
    }

    /// A row's menu; Delete comes last, after a separator. `archived` is None for
    /// a session the agent hasn't given an id yet, which can't be archived.
    fn for_row(row: &Row, archived: Option<bool>) -> Vec<RowAction> {
        Self::ALL
            .into_iter()
            .filter(|action| match action {
                RowAction::Archive => archived == Some(false),
                RowAction::Unarchive => archived == Some(true),
                RowAction::Close => matches!(row, Row::Open(_)),
                _ => true,
            })
            .collect()
    }
}

/// What a menu is for: a sidebar row (⋮), or an open session's notebook (⋯).
#[derive(Clone, PartialEq)]
enum MenuTarget {
    Row(Row),
    Notebook(u64),
}

#[derive(Clone, Copy, PartialEq)]
enum NotebookAction {
    Reveal,
    NewSession,
    Stop,
}

impl NotebookAction {
    fn label(self) -> &'static str {
        match self {
            NotebookAction::Reveal => "Reveal in Finder",
            NotebookAction::NewSession => "Open in a new session…",
            NotebookAction::Stop => "Stop notebook",
        }
    }

    fn shortcut(self) -> (&'static str, &'static str) {
        match self {
            NotebookAction::Reveal => ("f", "F"),
            NotebookAction::NewSession => ("n", "N"),
            NotebookAction::Stop => ("s", "S"),
        }
    }

    /// The notebook's menu; Stop comes last, after a separator, once it's open and running.
    fn for_notebook(running: bool) -> Vec<NotebookAction> {
        let mut actions = vec![NotebookAction::Reveal, NotebookAction::NewSession];
        if running {
            actions.push(NotebookAction::Stop);
        }
        actions
    }
}

/// A menu item, with what it acts on.
#[derive(Clone, PartialEq)]
enum MenuPick {
    Row(Row, RowAction),
    Notebook(u64, NotebookAction),
}

impl MenuPick {
    fn label(&self) -> &'static str {
        match self {
            MenuPick::Row(_, action) => action.label(),
            MenuPick::Notebook(_, action) => action.label(),
        }
    }

    fn shortcut(&self) -> (&'static str, &'static str) {
        match self {
            MenuPick::Row(_, action) => action.shortcut(),
            MenuPick::Notebook(_, action) => action.shortcut(),
        }
    }

    /// Shown in the danger colour, after a separator.
    fn danger(&self) -> bool {
        matches!(self, MenuPick::Row(_, RowAction::Delete) | MenuPick::Notebook(_, NotebookAction::Stop))
    }
}

/// An open ⋮ / ⋯ menu.
struct PopupMenu {
    target: MenuTarget,
    /// Where a right-click opened it; None hangs it under the ⋮ button.
    at: Option<Point<Pixels>>,
    /// The item picked with the arrow keys or the pointer.
    selected: Option<usize>,
    focus: FocusHandle,
    /// Focus to give back when the menu closes.
    restore: Option<FocusHandle>,
}

/// The ⋮ button at a row's end: shown while the pointer is over the row
/// (`group`), and always on the active row or while its menu is open.
fn more_button(id: impl Into<ElementId>, group: SharedString, shown: bool) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label("Session actions")
        .relative()
        .flex_shrink_0()
        .size(px(24.))
        .mr(px(-6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .font_family(theme::MONO)
        .text_size(theme::size_subhead())
        .text_color(if shown { theme::text_muted().into() } else { gpui::transparent_black() })
        .group_hover(group, |s| s.text_color(theme::text_muted()))
        .hover(|s| s.text_color(theme::text_primary()))
        .child("⋮")
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
    /// The new-session screen's choices.
    draft: Draft,
    /// Working folders, most recent first (persisted).
    recent: Vec<PathBuf>,
    /// Past sessions per folder, from the agent's history.
    past: HashMap<PathBuf, Vec<SessionInfo>>,
    /// Ids of sessions Endeavor created (persisted). Only these are listed: Claude
    /// Code's history for a folder also holds CLI sessions, which aren't ours.
    ours: HashSet<String>,
    /// Session names the user gave, by session id (persisted).
    titles: HashMap<String, String>,
    /// Ids of archived sessions (persisted): hidden from the sidebar's Active view.
    archived: HashSet<String>,
    /// The sidebar's Active / All filter menu is open.
    filter_menu: bool,
    /// Each session's notebook file, by session id (persisted), for reopening it
    /// with the session: one the app opened isn't in the agent's history.
    session_notebooks: HashMap<String, String>,
    /// The session being renamed, and its name box.
    renaming: Option<(Row, Entity<InputState>)>,
    menu: Option<PopupMenu>,
    /// Folders showing all their past sessions, not just the newest.
    expanded: HashSet<PathBuf>,
    settings: Settings,
    /// The divider being dragged.
    resizing: Option<Divider>,
    /// The composer's open picker: a config option id ("model", "effort").
    picker: Option<&'static str>,
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
    /// Per-notebook cell states as last pushed ({notebook_id: [state]}).
    cells: serde_json::Value,
    /// The composer's placeholder as last set (it changes while Claude works).
    placeholder: &'static str,
    /// Bumped when Julia boots or dies, so an old runtime's event reader stops.
    runtime_generation: Arc<AtomicU64>,
    died_tx: UnboundedSender<String>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (page_tx, mut page_rx) = futures::channel::mpsc::unbounded::<String>();
        let webview = cx.new(|cx| {
            let handle = window.window_handle().expect("window handle");
            let webview = wry::WebViewBuilder::new()
                .with_devtools(true)
                .with_initialization_script(&annotate::script())
                // The bundled JuliaMono for Pluto's page, which otherwise loads it from a CDN.
                .with_custom_protocol("endeavor".into(), |_, request| {
                    let font = match request.uri().path() {
                        "/fonts/JuliaMono-Regular.ttf" => Some(theme::JULIA_MONO_REGULAR),
                        "/fonts/JuliaMono-Bold.ttf" => Some(theme::JULIA_MONO_BOLD),
                        "/fonts/JuliaMono-RegularItalic.ttf" => Some(theme::JULIA_MONO_ITALIC),
                        _ => None,
                    };
                    let response = wry::http::Response::builder().header("Access-Control-Allow-Origin", "*");
                    match font {
                        Some(bytes) => response.header("Content-Type", "font/ttf").body(bytes.into()),
                        None => response.status(404).body(Vec::new().into()),
                    }
                    .expect("static response")
                })
                .with_ipc_handler(move |request| {
                    let _ = page_tx.unbounded_send(request.into_body());
                })
                .build_as_child(&handle)
                .expect("child webview");
            webkeys::fix_key_handling();
            webkeys::allow_pinch_zoom(&webview);
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
                .placeholder("Type / for commands")
                .submit_on_enter(true)
                .auto_grow(1, 8)
        });
        cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            // The slash-command menu follows what's typed.
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
            if let InputEvent::PressEnter { secondary, shift: false } = event {
                // An empty box answers a pending approval: ⏎ allow, ⌘⏎ allow and stop asking.
                if input.read(cx).value().trim().is_empty()
                    && let Some(key) = this.active
                    && this.session_mut(key).is_some_and(|s| s.pending_permission().is_some())
                {
                    this.with_session(key, cx, |s| {
                        s.answer_pending(PermissionOptionKind::AllowOnce, *secondary);
                    });
                    return;
                }
                if this.active.is_some() {
                    this.submit(*secondary, window, cx);
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
        let recent = load_recent();
        let draft = Draft::new(new_session::default_folder(&recent), window, cx);
        let mut this = Self {
            webview,
            input,
            sessions: Vec::new(),
            active: None,
            next_key: 1,
            draft,
            recent,
            past: HashMap::new(),
            ours: load_json("sessions.json"),
            titles: load_json("titles.json"),
            archived: load_json("archived.json"),
            filter_menu: false,
            session_notebooks: load_json("notebooks.json"),
            renaming: None,
            menu: None,
            expanded: HashSet::new(),
            settings: Settings::load(),
            settings_open: false,
            resizing: None,
            picker: None,
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
            cells: serde_json::Value::Null,
            placeholder: "Type / for commands",
            runtime_generation: Arc::new(AtomicU64::new(0)),
            died_tx,
        };
        settings::set_webview_appearance(this.webview.read(cx).raw(), this.settings.appearance);
        // Julia boots while the user picks a folder on the new-session screen.
        this.boot(None, cx);
        this.scan_notebooks(cx);
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

    /// Start a session with the new-session screen's choices, and the input's
    /// text (if any) as its first message. A chosen notebook opens in safe preview.
    fn start_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popover(window, cx);
        let cwd = self.draft.folder.clone();
        let _ = std::fs::create_dir_all(&cwd);
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
        let existing = match &self.draft.notebook {
            NotebookChoice::New => None,
            NotebookChoice::Existing(path) => Some(path.display().to_string()),
        };
        let context = existing.as_ref().map(|path| {
            session.open_on_start(path.clone());
            ContentBlock::Text(TextContent::new(format!(
                "[Endeavor] The user started this session on the Pluto notebook {path}, which is open in the \
                 notebook pane in safe preview (nothing has run). Unless they say otherwise, \"the notebook\" \
                 means this one; list_notebooks gives its id."
            )))
        });
        self.sessions.push(session);
        if let Some(path) = existing {
            self.bind_notebook(key, path, cx);
        }
        self.draft.notebook = NotebookChoice::New;
        self.draft.preview = None;
        self.activate(key, cx);
        self.send(key, context, false, window, cx);
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
        let notebook = self.session_notebooks.get(&info.session_id.to_string()).cloned();
        let mut session = Session::loading(key, info.session_id, info.cwd, title);
        if let Some(path) = &notebook {
            session.open_on_start(path.clone());
        }
        session.named = named.is_some();
        session.run_without_asking = self.settings.run_without_asking;
        self.sessions.push(session);
        if let Some(path) = notebook {
            self.bind_notebook(key, path, cx);
        }
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
        if self.renaming.as_ref().is_some_and(|(row, _)| *row == Row::Open(key)) {
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

    /// Delete a session's history (Claude Code's transcript of it), closing it first
    /// if it's open. A session the agent hasn't started yet just closes.
    fn delete_session(&mut self, row: Row, cx: &mut Context<Self>) {
        let id = match row {
            Row::Open(key) => {
                let id = self.sessions.iter().find(|s| s.key == key).and_then(|s| s.id.clone());
                self.close_session(key, cx);
                id
            }
            Row::Past(id, _) => Some(id),
        };
        let Some(id) = id else { return };
        let _ = self.agent_tx.unbounded_send(Command::DeleteSession(id.clone()));
        for past in self.past.values_mut() {
            past.retain(|info| info.session_id != id);
        }
        if self.renaming.as_ref().is_some_and(|(row, _)| matches!(row, Row::Past(p, _) if *p == id)) {
            self.renaming = None;
        }
        self.ours.remove(&id.to_string());
        save_json("sessions.json", &self.ours);
        if self.titles.remove(&id.to_string()).is_some() {
            save_json("titles.json", &self.titles);
        }
        if self.archived.remove(&id.to_string()) {
            save_json("archived.json", &self.archived);
        }
        if self.session_notebooks.remove(&id.to_string()).is_some() {
            save_json("notebooks.json", &self.session_notebooks);
        }
        cx.notify();
    }

    fn row_session_id(&self, row: &Row) -> Option<SessionId> {
        match row {
            Row::Open(key) => self.sessions.iter().find(|s| s.key == *key).and_then(|s| s.id.clone()),
            Row::Past(id, _) => Some(id.clone()),
        }
    }

    fn row_actions(&self, row: &Row) -> Vec<RowAction> {
        let archived = self.row_session_id(row).map(|id| self.archived.contains(&id.to_string()));
        RowAction::for_row(row, archived)
    }

    /// Archive a session (closing it if it's open), or bring it back.
    fn set_archived(&mut self, row: Row, archive: bool, cx: &mut Context<Self>) {
        let Some(id) = self.row_session_id(&row) else { return };
        let changed = if archive { self.archived.insert(id.to_string()) } else { self.archived.remove(&id.to_string()) };
        if changed {
            save_json("archived.json", &self.archived);
        }
        if let (true, Row::Open(key)) = (archive, row) {
            self.close_session(key, cx);
        }
        cx.notify();
    }

    fn set_show_archived(&mut self, show: bool, cx: &mut Context<Self>) {
        self.filter_menu = false;
        self.update_settings(cx, |s| s.show_archived = show);
    }

    /// A row's name as the sidebar shows it.
    fn row_title(&self, row: &Row) -> Option<String> {
        match row {
            Row::Open(key) => self.sessions.iter().find(|s| s.key == *key).map(|s| s.title.clone()),
            Row::Past(id, folder) => {
                let info = self.past.get(folder)?.iter().find(|info| info.session_id == *id)?;
                Some(self.titles.get(&id.to_string()).cloned().or(info.title.clone()).unwrap_or_else(|| "Earlier session".into()))
            }
        }
    }

    fn menu_picks(&self, target: &MenuTarget) -> Vec<MenuPick> {
        match target {
            MenuTarget::Row(row) => self.row_actions(row).into_iter().map(|action| MenuPick::Row(row.clone(), action)).collect(),
            MenuTarget::Notebook(key) => {
                let running = self.sessions.iter().find(|s| s.key == *key).is_some_and(|s| s.notebook.is_some() && s.stopped.is_none());
                NotebookAction::for_notebook(running).into_iter().map(|action| MenuPick::Notebook(*key, action)).collect()
            }
        }
    }

    fn open_menu(&mut self, target: MenuTarget, at: Option<Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        let restore = match self.menu.take() {
            Some(menu) => menu.restore,
            None => window.focused(cx),
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.menu = Some(PopupMenu { target, at, selected: None, focus, restore });
        cx.notify();
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(restore) = self.menu.take().and_then(|menu| menu.restore) {
            window.focus(&restore, cx);
        }
        cx.notify();
    }

    fn pick(&mut self, pick: MenuPick, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        match pick {
            MenuPick::Row(row, action) => self.row_action(row, action, window, cx),
            MenuPick::Notebook(key, action) => self.notebook_action(key, action, cx),
        }
    }

    fn notebook_action(&mut self, key: u64, action: NotebookAction, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let Some(path) = session.notebook_path.clone() else { return };
        match action {
            NotebookAction::Reveal => {
                let _ = std::process::Command::new("open").arg("-R").arg(&path).spawn();
            }
            NotebookAction::NewSession => {
                let folder = session.cwd.clone();
                self.new_session_on(folder, PathBuf::from(path), cx);
            }
            NotebookAction::Stop => self.stop_notebook(path, cx),
        }
    }

    fn row_action(&mut self, row: Row, action: RowAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            RowAction::Rename => self.start_rename(row, window, cx),
            RowAction::Reveal => {
                let folder = match &row {
                    Row::Open(key) => self.sessions.iter().find(|s| s.key == *key).map(|s| s.cwd.clone()),
                    Row::Past(_, folder) => Some(folder.clone()),
                };
                if let Some(folder) = folder {
                    let _ = std::process::Command::new("open").arg("-R").arg(folder).spawn();
                }
            }
            RowAction::Archive => self.set_archived(row, true, cx),
            RowAction::Unarchive => self.set_archived(row, false, cx),
            RowAction::Close => {
                if let Row::Open(key) = row {
                    self.close_session(key, cx);
                }
            }
            RowAction::Delete => {
                let Some(title) = self.row_title(&row) else { return };
                let answer = window.prompt(
                    PromptLevel::Warning,
                    &format!("Delete “{title}”?"),
                    Some("This permanently deletes the conversation, including its Claude Code history. Notebooks and other files it made stay on disk."),
                    // Cancel first: NSAlert gives the first button Return, and a cancel
                    // button trades it for Escape, so no key deletes by accident.
                    &[PromptButton::cancel("Cancel"), PromptButton::new("Delete")],
                    cx,
                );
                cx.spawn(async move |this, cx| {
                    if answer.await == Ok(1) {
                        let _ = this.update(cx, |this, cx| this.delete_session(row, cx));
                    }
                })
                .detach();
            }
        }
    }

    fn start_rename(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.row_title(&row) else { return };
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
        self.renaming = Some((row, input));
        cx.notify();
    }

    /// Keep the typed name (an empty one leaves the title as it was).
    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some((row, input)) = self.renaming.take() else { return };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return cx.notify();
        }
        let id = match row {
            Row::Open(key) => self.session_mut(key).and_then(|session| {
                session.title = name.clone();
                session.named = true;
                session.id.as_ref().map(ToString::to_string)
            }),
            Row::Past(id, _) => Some(id.to_string()),
        };
        if let Some(id) = id {
            self.titles.insert(id, name);
            save_json("titles.json", &self.titles);
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

    /// Open a session's notebook file in the current Pluto (reusing it if it's
    /// already open; otherwise running it only if `run`) and point the session, the
    /// pane if active, and any session whose copy was stopped, at it.
    fn open_for_session(&mut self, key: u64, path: String, run: bool, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        let opened = cx.background_executor().spawn({
            let path = path.clone();
            async move {
                let listed = pluto::call_tool(&mcp_url, "list_notebooks", serde_json::json!({})).ok();
                let open = listed.as_ref().and_then(|l| l.as_array()?.iter().find(|nb| nb["path"] == path.as_str()).cloned());
                let nb = match open {
                    Some(nb) => nb,
                    None => pluto::call_tool(&mcp_url, "open_notebook", serde_json::json!({ "path": path, "run_notebook": run })).ok()?,
                };
                nb["notebook_id"].as_str().map(str::to_owned)
            }
        });
        cx.spawn(async move |this, cx| {
            // ponytail: a notebook file that's gone just leaves the pane where it is.
            let Some(id) = opened.await else { return };
            let _ = this.update(cx, |this, cx| {
                let keys: Vec<u64> = this
                    .sessions
                    .iter()
                    .filter(|s| s.key == key || (s.stopped.is_some() && s.notebook_path.as_deref() == Some(path.as_str())))
                    .map(|s| s.key)
                    .collect();
                for key in keys {
                    this.apply_effects(key, vec![Effect::ShowNotebook { id: id.clone(), path: Some(path.clone()) }], cx);
                }
            });
        })
        .detach();
    }

    /// Record a session's one notebook file: the runtime holds the session to it,
    /// and notebooks.json reopens it with the session.
    fn bind_notebook(&mut self, key: u64, path: String, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(key) else { return };
        session.notebook_path = Some(path.clone());
        if let Some(id) = session.id.as_ref().map(ToString::to_string)
            && self.session_notebooks.get(&id) != Some(&path)
        {
            self.session_notebooks.insert(id, path.clone());
            save_json("notebooks.json", &self.session_notebooks);
        }
        self.send_binding(key, path, cx);
    }

    fn send_binding(&self, key: u64, path: String, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        // ponytail: a failed send leaves the session unbound until its first open binds it.
        cx.background_executor().spawn(async move { pluto::set_notebook(&mcp_url, key, &path) }).detach();
    }

    /// Stop the notebook at `path` (Pluto shuts it down); every session on it
    /// shows it stopped, with Start.
    fn stop_notebook(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        let stop = cx.background_executor().spawn({
            let path = path.clone();
            async move { pluto::stop_notebook(&mcp_url, &path) }
        });
        cx.spawn(async move |this, cx| {
            let result = stop.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    // Not open (already stopped elsewhere): Start reopens it in safe preview.
                    Ok(safe_preview) => {
                        let stopped = Stopped { safe_preview: safe_preview.unwrap_or(true) };
                        for session in this.sessions.iter_mut().filter(|s| s.notebook_path.as_deref() == Some(path.as_str())) {
                            session.notebook = None;
                            session.stopped = Some(stopped);
                        }
                    }
                    Err(e) => this.status = format!("⚠ Couldn't stop {}: {e}", folder_name(Path::new(&path))).into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Start a stopped notebook again, the way it was running.
    fn start_notebook(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some((path, stopped)) = self.session_mut(key).and_then(|s| s.notebook_path.clone().zip(s.stopped)) else { return };
        self.open_for_session(key, path, !stopped.safe_preview, cx);
    }

    /// Show a session; the notebook pane follows it to the notebook it last viewed
    /// (without one, the pane draws a stand-in over the hidden web view).
    fn activate(&mut self, key: u64, cx: &mut Context<Self>) {
        self.active = Some(key);
        self.settings_open = false;
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
                Effect::ShowNotebook { id, path } => {
                    if let Some(session) = self.session_mut(key) {
                        session.notebook = Some(id.clone());
                        session.stopped = None;
                    }
                    if let Some(path) = path {
                        self.bind_notebook(key, path, cx);
                    }
                    if self.active == Some(key) {
                        self.load_notebook(&id, cx);
                    }
                }
                Effect::CheckRunState => self.check_run_state(key, cx),
                Effect::SetPolicy(policy) => self.send_policy(key, policy, cx),
                Effect::PreviewRun { ix, tool, input } => {
                    let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { continue };
                    let task = cx.background_executor().spawn(async move { pluto::run_preview(&mcp_url, &tool, &input) });
                    cx.spawn(async move |this, cx| match task.await {
                        Ok(preview) => {
                            let _ = this.update(cx, |this, cx| this.with_session(key, cx, |s| s.set_preview(ix, preview)));
                        }
                        // The card falls back to "Run code?".
                        Err(e) => eprintln!("run preview: {e}"),
                    })
                    .detach();
                }
                Effect::SetConfig(id_, value) => {
                    if let Some(id) = self.session_mut(key).and_then(|s| s.id.clone()) {
                        let _ = self.agent_tx.unbounded_send(Command::SetConfig(id, id_, value));
                    }
                }
                Effect::SetMode(mode) => {
                    if let Some(id) = self.session_mut(key).and_then(|s| s.id.clone()) {
                        let _ = self.agent_tx.unbounded_send(Command::SetMode(id, mode));
                    }
                }
                Effect::ReopenNotebook(path) => {
                    self.bind_notebook(key, path.clone(), cx);
                    self.open_for_session(key, path, false, cx);
                }
            }
        }
        cx.notify();
    }

    /// Which notebook the user is looking at, so "the notebook" is unambiguous.
    /// Also remembered as the active session's notebook.
    fn viewing_context(&mut self, cx: &mut Context<Self>) -> Option<ContentBlock> {
        let url = self.webview.read(cx).raw().url().unwrap_or_default();
        let id = viewed_notebook_id(&url)?.to_owned();
        let mut edits = Vec::new();
        if let Some(session) = self.active.and_then(|key| self.session_mut(key)) {
            session.notebook = Some(id.clone());
            edits = std::mem::take(&mut session.user_edits);
        }
        let mut text = format!(
            "[Endeavor] The user is viewing Pluto notebook {id} in the notebook pane. \
             Unless they say otherwise, \"the notebook\" means this one."
        );
        if !edits.is_empty() {
            let cells: Vec<String> = edits.iter().map(|(cell, name)| name.as_ref().map_or(cell.clone(), |n| format!("`{n}` ({cell})"))).collect();
            text += &format!(" Since you last heard, the user edited these cells: {}. Re-read them before relying on their code.", cells.join(", "));
        }
        Some(ContentBlock::Text(TextContent::new(text)))
    }

    fn submit(&mut self, now: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active else { return };
        if self.input.read(cx).value().trim().is_empty() {
            return;
        }
        let context = self.viewing_context(cx);
        self.send(key, context, now, window, cx);
    }

    /// Send the input's text (if any) to a session, after `context`.
    fn send(&mut self, key: u64, context: Option<ContentBlock>, now: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut blocks: Vec<_> = context.into_iter().collect();
        blocks.push(ContentBlock::Text(TextContent::new(text.clone())));
        let Some(session) = self.session_mut(key) else { return };
        let effects = session.submit(Queued::new(text.clone(), Some(text), blocks), now);
        self.apply_effects(key, effects, cx);
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
    }

    fn send_policy(&self, key: u64, policy: &'static str, cx: &mut Context<Self>) {
        let Some(mcp_url) = self.runtime.as_ref().map(|r| r.mcp_url.clone()) else { return };
        // ponytail: a failed send leaves the runtime's policy stale until the next change.
        cx.background_executor().spawn(async move { pluto::set_policy(&mcp_url, key, policy) }).detach();
    }

    /// ⇧⇥: the active session's next mode (e.g. default → plan → auto).
    fn open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        cx.notify();
    }

    /// "/" at the start of the box lists the agent's commands matching what follows;
    /// a click fills in the command.
    fn render_commands(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        const SHOWN: usize = 8;
        let text = self.input.read(cx).value().to_string();
        let typed = text.strip_prefix('/').filter(|t| !t.contains(char::is_whitespace))?;
        let matches: Vec<_> = session.commands.iter().filter(|c| c.name.starts_with(typed)).take(SHOWN).collect();
        if matches.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_col()
                .p(px(4.))
                .rounded(px(8.))
                .border_1()
                .border_color(theme::composer_edge())
                .bg(theme::bg_raised())
                .children(matches.into_iter().enumerate().map(|(i, command)| {
                    let name = command.name.clone();
                    div()
                        .id(ElementId::NamedInteger("command".into(), i as u64))
                        .flex()
                        .gap_3()
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::row_active()))
                        .child(div().flex_shrink_0().font_family(theme::MONO).text_size(theme::size_code()).child(format!("/{}", command.name)))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(theme::text_muted()).child(command.description.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.input.update(cx, |s, cx| s.set_value(format!("/{name} "), window, cx));
                            // The box keeps focus; put the caret after the command.
                            window.dispatch_action(Box::new(gpui_component::input::MoveToEnd), cx);
                            cx.notify();
                        }))
                }))
                .into_any_element(),
        )
    }

    /// The model / effort list, opening upward from the composer toolbar.
    fn render_picker(&self, session: &Session, id: &'static str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (current, options) = session.config_choices(id)?;
        let key = session.key;
        Some(
            div()
                .id("picker")
                // Clicks stop here instead of reaching the transcript underneath.
                .occlude()
                .absolute()
                .right(px(16.))
                // Above the box, not over it.
                .bottom(px(84.))
                .w(px(260.))
                .p(px(4.))
                .flex()
                .flex_col()
                .rounded(px(8.))
                .border_1()
                .border_color(theme::composer_edge())
                .bg(theme::bg_raised())
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.picker = None;
                    cx.notify();
                }))
                .children(options.into_iter().enumerate().map(|(i, option)| {
                    let chosen = option.value == current;
                    let value = option.value.clone();
                    div()
                        .id(ElementId::NamedInteger("pick".into(), i as u64))
                        .flex()
                        .gap_2()
                        .px(px(8.))
                        .py(px(5.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::row_active()))
                        .child(div().w(px(10.)).flex_shrink_0().text_color(theme::accent_text()).child(if chosen { "✓" } else { "" }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(option.name.clone())
                                .children(option.description.clone().map(|d| div().text_size(theme::size_meta()).text_color(theme::text_muted()).child(d))),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.picker = None;
                            let effects = this.session_mut(key).map(|s| s.set_config(id, value.clone())).unwrap_or_default();
                            this.settings.agent_config.insert(id.to_string(), value.to_string());
                            this.settings.save();
                            this.apply_effects(key, effects, cx);
                            cx.notify();
                        }))
                }))
                .into_any_element(),
        )
    }

    /// Zoom the notebook by `step` (or back to 100%), kept in settings.
    fn zoom(&mut self, step: f64, reset: bool, cx: &mut Context<Self>) {
        let current = if self.settings.zoom > 0. { self.settings.zoom } else { 1. };
        self.settings.zoom = if reset { 1. } else { (current * step).clamp(0.5, 3.) };
        self.settings.save();
        let _ = self.webview.read(cx).raw().zoom(self.settings.zoom);
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.settings.layout.sidebar_open = !self.settings.layout.sidebar_open;
        self.settings.save();
        cx.notify();
    }

    /// A 1px column divider with a wider invisible grip for dragging. The grip
    /// sits left of the line: the notebook's web view covers anything to its right.
    fn divider(&self, which: Divider, color: Rgba, cx: &mut Context<Self>) -> impl IntoElement {
        let id = if which == Divider::Sidebar { "sidebar-divider" } else { "chat-divider" };
        div().w(px(1.)).h_full().flex_shrink_0().bg(color).relative().child(
            div()
                .id(id)
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-6.))
                .w(px(7.))
                .cursor(CursorStyle::ResizeLeftRight)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.resizing = Some(which);
                        cx.stop_propagation();
                    }),
                ),
        )
    }

    /// Drags go on reaching us over the notebook: AppKit sends them to the view
    /// that got the mouse-down.
    fn drag_divider(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = self.resizing else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.resizing = None;
            return;
        }
        let x = e.position.x.as_f32();
        match which {
            // Dragging well past the minimum collapses the sidebar, like ⌘B.
            Divider::Sidebar if x < SIDEBAR_RANGE.0 / 2. => self.settings.layout.sidebar_open = false,
            Divider::Sidebar => (self.settings.layout.sidebar_open, self.settings.layout.sidebar_width) = (true, x.clamp(SIDEBAR_RANGE.0, SIDEBAR_RANGE.1)),
            Divider::Chat => {
                let start = if self.settings.layout.sidebar_open { self.settings.layout.sidebar_width + 1. } else { 0. };
                let max = (window.viewport_size().width.as_f32() - start - NOTEBOOK_MIN).max(CHAT_MIN);
                self.settings.layout.chat_width = (x - start).clamp(CHAT_MIN, max);
            }
        }
        cx.notify();
    }

    fn cycle_mode(&mut self, _: &CycleMode, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active else { return };
        if let Some(effects) = self.session_mut(key).map(Session::cycle_mode) {
            self.apply_effects(key, effects, cx);
        }
    }

    fn interrupt(&mut self, _: &Interrupt, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter_menu {
            self.filter_menu = false;
            return cx.notify();
        }
        if self.draft.popover.is_some() {
            return self.close_popover(window, cx);
        }
        let Some(key) = self.active else { return };
        // Esc denies a pending approval before it stops the turn.
        if let Some(s) = self.session_mut(key)
            && s.answer_pending(PermissionOptionKind::RejectOnce, false)
        {
            cx.notify();
            return;
        }
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
                    setup.fail(e.clone());
                }
                for session in &mut self.sessions {
                    session.note(format!("⚠ Agent stopped: {e}"));
                }
            }
            AgentEvent::Started { key, result } => {
                let Some(session) = self.session_mut(key) else { return };
                match result {
                    Ok(started) => {
                        let id = started.id.clone();
                        if self.ours.insert(id.to_string()) {
                            save_json("sessions.json", &self.ours);
                        }
                        // Renamed before the agent assigned an id.
                        if let Some(name) = self.session_mut(key).filter(|s| s.named).map(|s| s.title.clone()) {
                            self.titles.insert(id.to_string(), name);
                            save_json("titles.json", &self.titles);
                        }
                        let picked = self.settings.agent_config.clone();
                        let Some(session) = self.session_mut(key) else { return };
                        let queued = session.started(started);
                        // The user's last picks, ahead of a queued first message; the model
                        // first, since effort's choices depend on it.
                        let mut effects = Vec::new();
                        for id in ["model", "effort"] {
                            let Some(value) = picked.get(id) else { continue };
                            let offered = session.config_choices(id).filter(|(current, options)| {
                                current.to_string() != *value && options.iter().any(|o| o.value.to_string() == *value)
                            });
                            if offered.is_some() {
                                effects.extend(session.set_config(id, value.clone().into()));
                            }
                        }
                        effects.extend(queued);
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
            Some(annotate::Message::Ready) => {
                self.push_cells(cx);
                self.apply_look(cx);
            }
            Some(annotate::Message::Mode(on)) => self.annotating = on,
            Some(annotate::Message::Annotation(a)) => {
                let Some(key) = self.active else { return };
                let comment = if a.comment.is_empty() { "(no comment)" } else { a.comment.as_str() };
                let attached = a.attachment.as_ref().map(|(label, _)| format!("\n📎 {label}")).unwrap_or_default();
                let label = format!("✎ {} cell{}: {comment}{attached}", a.cells.len(), if a.cells.len() > 1 { "s" } else { "" });
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

    /// Cmd+Shift+K from the panel (the page handles it when the notebook has focus).
    fn toggle_annotation(&mut self, _: &ToggleAnnotation, _: &mut Window, cx: &mut Context<Self>) {
        let enable = !self.annotating;
        if enable {
            // Move keyboard focus into the notebook so the comment box takes typing.
            let _ = self.webview.read(cx).raw().focus();
        }
        self.send_to_page(&serde_json::json!({ "type": "annotate", "on": enable }), cx);
    }

    /// A message to the page script (frontend/src/bridge.ts, `ToPage`).
    fn send_to_page(&self, msg: &serde_json::Value, cx: &mut Context<Self>) {
        // JSON is a JS expression, so the message goes in as a literal.
        let _ = self.webview.read(cx).raw().evaluate_script(&format!("window.__endeavor && window.__endeavor.receive({msg})"));
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
        let muted = theme::text_muted();
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .bg(if primary { theme::accent() } else { theme::bg_raised() })
                .child(label)
        };
        let waiting = self.signing_in.then(|| div().text_size(theme::size_meta()).text_color(muted).child("Waiting for you to finish in your browser…"));
        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded_md()
                .bg(theme::bg_card())
                .child(div().text_size(theme::size_subhead()).font_weight(FontWeight::MEDIUM).child("Sign in to Claude"))
                .child(div().text_size(theme::size_meta()).text_color(muted).child("Endeavor runs Claude Code with your account. Sign-in opens in your browser."))
                .child(button("sign-in-claude", "Claude subscription", true).on_click(cx.listener(|this, _, _, cx| this.sign_in(false, cx))))
                .child(
                    button("sign-in-console", "Anthropic Console (API billing)", false)
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in(true, cx))),
                )
                .children(waiting)
                .children(self.sign_in_error.clone().map(|e| div().text_size(theme::size_meta()).text_color(theme::danger()).child(e)))
                .into_any_element(),
        )
    }

    /// Retry the failed setup step: start Julia again, or the agent once Julia is up.
    pub fn retry_setup(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = &mut self.setup else { return };
        setup.clear_error();
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
                    setup.fail(e);
                }
                return cx.notify();
            }
        };
        self.webview.update(cx, |w, _| w.load_url(&runtime.pluto_url));
        let mcp_url = runtime.mcp_url.clone();
        self.runtime = Some(runtime);
        self.follow_folder();
        // A new runtime knows no session's notebook.
        let bound: Vec<(u64, String)> = self.sessions.iter().filter_map(|s| Some((s.key, s.notebook_path.clone()?))).collect();
        for (key, path) in bound {
            self.send_binding(key, path, cx);
        }
        if let Some(commands) = self.agent_rx.take() {
            self.on_progress(Progress::new(Step::Agent, "Pluto ready · starting Claude…"), cx);
            self.start_agent(mcp_url.clone(), commands, cx);
        } else {
            self.status = "Julia restarted.".into();
            // Captures the notebooks to reopen before the new list starts arriving.
            self.reopen_notebooks(cx);
            // The new runtime starts with every session on "ask".
            let planning: Vec<u64> = self.sessions.iter().filter(|s| s.policy() == "plan").map(|s| s.key).collect();
            for key in planning {
                self.send_policy(key, "plan", cx);
            }
        }
        self.watch_notebooks(mcp_url, cx);
        cx.notify();
    }

    fn on_runtime_died(&mut self, reason: String, cx: &mut Context<Self>) {
        self.last_ports = self.runtime.take().map(|r| r.ports);
        // Stop following the dead runtime; `last_notebooks` stays for the reopen.
        self.runtime_generation.fetch_add(1, Ordering::SeqCst);
        self.notebooks = serde_json::Value::Null;
        self.cells = serde_json::Value::Null;
        self.status = format!("⚠ {reason}\nNotebook tools are unavailable until Julia restarts.").into();
        cx.notify();
    }

    /// Follow the runtime's notebook list (pushed on every change) for the
    /// end-of-turn run check and for reopening notebooks after a crash.
    fn watch_notebooks(&self, mcp_url: String, cx: &mut Context<Self>) {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<serde_json::Value>();
        let generation = self.runtime_generation.clone();
        let mine = generation.fetch_add(1, Ordering::SeqCst) + 1;
        // A long-lived blocking read: its own thread, not the executor's pool. If the
        // stream drops it reconnects, until this runtime is replaced or dies.
        std::thread::spawn(move || {
            while generation.load(Ordering::SeqCst) == mine {
                let _ = pluto::watch_notebooks(&mcp_url, |event| {
                    let _ = tx.unbounded_send(event);
                });
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        cx.spawn(async move |this, cx| {
            while let Some(mut event) = rx.next().await {
                let updated = this.update(cx, |this, cx| {
                    this.remember_notebooks(event["notebooks"].take());
                    for (notebook, cell, name) in pluto::user_edits(&this.cells, &event["cells"]) {
                        for session in this.sessions.iter_mut().filter(|s| s.notebook.as_deref() == Some(notebook.as_str())) {
                            session.note_user_edit(cell.clone(), name.clone());
                        }
                    }
                    this.cells = event["cells"].take();
                    this.push_cells(cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The notebook's appearance and theme, from Settings.
    fn apply_look(&self, cx: &mut Context<Self>) {
        settings::set_webview_appearance(self.webview.read(cx).raw(), self.settings.appearance);
        if self.settings.zoom > 0. {
            let _ = self.webview.read(cx).raw().zoom(self.settings.zoom);
        }
        self.send_to_page(&serde_json::json!({ "type": "theme", "name": self.settings.notebook_theme.name() }), cx);
    }

    /// Mark the shown notebook's cells in the page (unrun, author).
    fn push_cells(&self, cx: &mut Context<Self>) {
        let url = self.webview.read(cx).raw().url().unwrap_or_default();
        let Some(id) = viewed_notebook_id(&url) else { return };
        let cells = self.cells.get(id).cloned().unwrap_or_else(|| serde_json::json!([]));
        self.send_to_page(&serde_json::json!({ "type": "cells", "cells": cells }), cx);
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

    /// A session's sidebar row: right-click opens its menu, and it stays lit while
    /// the menu is open.
    fn session_row(&self, row: Row, group: SharedString, active: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let menu_open = self.menu.as_ref().is_some_and(|menu| menu.target == MenuTarget::Row(row.clone()));
        sidebar_row(ElementId::Name(group.clone()), active)
            .group(group)
            .when(menu_open, |d| d.bg(theme::row_active()))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.open_menu(MenuTarget::Row(row.clone()), Some(e.position), window, cx)),
            )
    }

    /// A row's title, or its name box while it's being renamed.
    fn row_label(&self, row: &Row, title: String) -> Div {
        match &self.renaming {
            Some((renaming, input)) if renaming == row => div().flex_1().child(Input::new(input).xsmall().text_size(theme::size_body())),
            _ => div().flex_1().overflow_hidden().whitespace_nowrap().child(title),
        }
    }

    /// A row's ⋮ button, and its menu while open.
    fn row_more(&self, row: Row, group: SharedString, active: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let menu = self.menu.as_ref().filter(|menu| menu.target == MenuTarget::Row(row.clone()));
        more_button(ElementId::Name(format!("{group}-more").into()), group, active || menu.is_some())
            .children(menu.map(|menu| self.render_menu(menu, cx)))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.open_menu(MenuTarget::Row(row.clone()), None, window, cx);
            }))
    }

    /// A menu, under its ⋮ / ⋯ button or at the pointer that right-clicked.
    fn render_menu(&self, menu: &PopupMenu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let items = self.menu_picks(&menu.target).into_iter().enumerate().flat_map(|(i, pick)| {
            let danger = pick.danger();
            let separator = danger.then(|| div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::composer_edge()).into_any_element());
            let (label, shortcut) = (pick.label(), pick.shortcut().1);
            let item = div()
                .id(ElementId::NamedInteger("menu-item".into(), i as u64))
                .role(Role::MenuItem)
                .aria_label(label)
                .flex()
                .items_center()
                .justify_between()
                .px(px(8.))
                .py(px(4.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(danger, |d| d.text_color(theme::danger()))
                .when(menu.selected == Some(i), |d| d.bg(theme::composer_edge()))
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if let Some(menu) = this.menu.as_mut().filter(|menu| menu.selected != Some(i)) {
                        menu.selected = Some(i);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.pick(pick.clone(), window, cx);
                }))
                .child(label)
                .child(div().text_size(theme::size_meta()).text_color(theme::text_faint()).child(shortcut))
                .into_any_element();
            separator.into_iter().chain([item])
        });
        // Over the notebook, the web view (a native view on top) lets the menu show through.
        let hole = matches!(menu.target, MenuTarget::Notebook(_)).then(|| {
            let webview = self.webview.read(cx);
            let (handle, under) = (webview.handle(), webview.bounds());
            canvas(move |bounds, _, _| overlay::set_hole(handle.raw(), Some(Bounds { origin: bounds.origin - under.origin, size: bounds.size })), |_, _, _, _| ()).absolute().size_full()
        });
        let body = div()
            .id("popup-menu")
            .role(Role::Menu)
            .track_focus(&menu.focus)
            .occlude()
            .w(px(210.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(theme::composer_edge())
            .bg(theme::bg_raised())
            .font_family(theme::SANS)
            .text_size(theme::size_body())
            .text_color(theme::text_primary())
            .on_action(cx.listener(|this, _: &Interrupt, window, cx| this.close_menu(window, cx)))
            .on_key_down(cx.listener(Self::menu_key))
            .children(hole)
            .children(items);
        let placed = match menu.at {
            Some(at) => anchored().position(at),
            None => anchored().anchor(Anchor::TopRight),
        };
        // The wrapper puts an un-positioned menu under the button's right edge.
        div().absolute().top(px(28.)).right_0().child(deferred(placed.child(body)).with_priority(1))
    }

    fn menu_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.menu.as_ref().map(|menu| menu.target.clone()) else { return };
        let picks = self.menu_picks(&target);
        if !e.keystroke.modifiers.modified()
            && let Some(pick) = picks.iter().find(|p| p.shortcut().0 == e.keystroke.key)
        {
            cx.stop_propagation();
            return self.pick(pick.clone(), window, cx);
        }
        let Some(menu) = self.menu.as_mut() else { return };
        let n = picks.len();
        match e.keystroke.key.as_str() {
            "down" => menu.selected = Some(menu.selected.map_or(0, |i| (i + 1) % n)),
            "up" => menu.selected = Some(menu.selected.map_or(n - 1, |i| (i + n - 1) % n)),
            "enter" | "space" => {
                if let Some(i) = menu.selected {
                    self.pick(picks[i].clone(), window, cx);
                }
            }
            "escape" => self.close_menu(window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn render_session_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
                        let active = self.active == Some(key) && !self.settings_open;
                        let group: SharedString = format!("session-{key}").into();
                        let title = self.row_label(&Row::Open(key), s.title.clone());
                        let archived = s.id.as_ref().is_some_and(|id| self.archived.contains(&id.to_string()));
                        // Status at the row's end: a ring waits for you, a dot is working.
                        let mark = if s.needs_approval() {
                            Some(div().size(px(6.)).rounded_full().border_1().border_color(theme::accent()).into_any_element())
                        } else if s.outbox.busy {
                            Some(div().size(px(6.)).rounded_full().bg(theme::accent()).into_any_element())
                        } else if archived {
                            Some(glyph(Glyph::Archive, theme::text_section()).into_any_element())
                        } else {
                            None
                        };
                        self.session_row(Row::Open(key), group.clone(), active, cx)
                            .when(s.failed.is_some(), |d| d.text_color(theme::text_section()))
                            .child(title)
                            .children(mark)
                            .child(self.row_more(Row::Open(key), group, active, cx))
                            // Double-click renames.
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                if e.click_count() >= 2 {
                                    this.start_rename(Row::Open(key), window, cx);
                                } else {
                                    this.activate(key, cx);
                                }
                            }))
                    })
                    .collect();
                // Past sessions not already open, newest first; the newest few unless expanded.
                let is_open = |info: &SessionInfo| self.sessions.iter().any(|s| s.id.as_ref() == Some(&info.session_id));
                let is_archived = |info: &SessionInfo| self.archived.contains(&info.session_id.to_string());
                let shown = |info: &SessionInfo| self.ours.contains(&info.session_id.to_string()) && (self.settings.show_archived || !is_archived(info));
                let all: Vec<_> = self.past.get(folder).into_iter().flatten().filter(|info| !is_open(info) && shown(info)).collect();
                let expanded = self.expanded.contains(folder);
                let limit = if expanded { all.len() } else { PAST_SHOWN };
                let past: Vec<_> = all
                    .iter()
                    .take(limit)
                    .enumerate()
                    .map(|(i, info)| {
                        let row = Row::Past(info.session_id.clone(), folder.clone());
                        let title = self.row_label(&row, self.row_title(&row).unwrap_or_default());
                        let group: SharedString = format!("past-{}-{i}", folder.display()).into();
                        let open = (*info).clone();
                        let archived = is_archived(info);
                        self.session_row(row.clone(), group.clone(), false, cx)
                            .when(archived, |d| d.text_color(theme::text_section()))
                            .child(title)
                            .when(archived, |d| d.child(glyph(Glyph::Archive, theme::text_section())))
                            .child(self.row_more(row.clone(), group, false, cx))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.renaming.as_ref().is_some_and(|(renaming, _)| *renaming == row) {
                                    this.open_past(open.clone(), cx);
                                }
                            }))
                    })
                    .collect();
                let more = (all.len() > PAST_SHOWN).then(|| {
                    let label = if expanded { "Show fewer".to_string() } else { format!("Show {} more", all.len() - PAST_SHOWN) };
                    let folder = folder.clone();
                    sidebar_row(ElementId::Name(format!("more-{}", folder.display()).into()), false)
                        .text_color(theme::text_faint())
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
                    .child(div().mt(px(18.)).px(px(10.)).pb_1().text_size(theme::size_meta_small()).text_color(theme::text_section()).child(folder_name(folder)))
                    .children(open)
                    .children(past)
                    .children(more)
            })
            .collect();

        div()
            .w(px(self.settings.layout.sidebar_width))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .px(px(6.))
            .pb(px(10.))
            .bg(theme::bg_sidebar())
            // The traffic lights sit in this header (see TitlebarOptions in main()).
            .child(column_header("sidebar-header").mx(px(-6.)).justify_end().px(px(10.)).child(sidebar_toggle(cx)))
            .child(
                sidebar_row("new-session".into(), false)
                    .text_color(theme::text_new())
                    .child(div().text_color(theme::text_faint()).child("+"))
                    .child(div().flex_1().child("New session"))
                    .child(self.render_filter_button(cx))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.active = None;
                        this.settings_open = false;
                        this.scan_notebooks(cx);
                        cx.notify();
                    })),
            )
            .child(div().id("sessions").flex_1().overflow_y_scroll().flex().flex_col().children(groups))
            .children(self.render_sign_in(cx))
            .when(self.runtime.is_none() && !self.starting, |d| {
                d.child(
                    sidebar_row("restart".into(), false)
                        .text_color(theme::accent_text())
                        .child("↻ Restart Julia")
                        .on_click(cx.listener(|this, _, _, cx| this.boot(this.last_ports, cx))),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .pt(px(6.))
                    .px_1()
                    .child(div().flex_1().pl(px(6.)).text_size(theme::size_meta()).text_color(theme::text_section()).child(self.status.clone()))
                    .child(
                        div()
                            .id("settings")
                            .size(px(28.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_size(px(16.))
                            .text_color(if self.settings_open { theme::text_primary() } else { theme::text_faint() })
                            .when(self.settings_open, |d| d.bg(theme::row_active()))
                            .hover(|s| s.text_color(theme::text_primary()))
                            .child("⚙")
                            .on_click(cx.listener(|this, _, window, cx| this.open_settings(&OpenSettings, window, cx))),
                    ),
            )
    }

    /// The funnel at the end of the "New session" row, and its Active / All menu.
    fn render_filter_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let all = self.settings.show_archived;
        let color = if all { theme::accent_text() } else { theme::text_faint() };
        let item = |id: &'static str, checked: bool, label: &'static str, show: bool| {
            menu_row(id, checked, false)
                .role(Role::MenuItemRadio)
                .aria_label(label)
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_show_archived(show, cx);
                }))
        };
        let menu = self.filter_menu.then(|| {
            let body = div()
                .id("filter-menu")
                .role(Role::Menu)
                .occlude()
                .w(px(210.))
                .p(px(4.))
                .flex()
                .flex_col()
                .rounded(px(8.))
                .border_1()
                .border_color(theme::composer_edge())
                .bg(theme::bg_raised())
                .font_family(theme::SANS)
                .text_size(theme::size_body())
                .text_color(theme::text_primary())
                .child(item("filter-active", !all, "Active", false))
                .child(item("filter-all", all, "All, including archived", true));
            div().absolute().top(px(26.)).right_0().child(deferred(anchored().anchor(Anchor::TopRight).child(body)).with_priority(1))
        });
        div()
            .id("sidebar-filter")
            .role(Role::Button)
            .aria_label("Show sessions")
            .relative()
            .flex_shrink_0()
            .size(px(24.))
            .mr(px(-6.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when(self.filter_menu, |d| d.bg(theme::bg_raised()))
            .hover(|s| s.bg(theme::bg_raised()))
            .child(glyph(Glyph::Funnel, color.into()))
            .children(menu)
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.filter_menu = !this.filter_menu;
                cx.notify();
            }))
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let muted = theme::text_muted();
        let s = &self.settings;
        let heading = |text: &'static str| div().mt_2().text_size(theme::size_subhead()).font_weight(FontWeight::MEDIUM).child(text);
        let note = |text: String| div().pl_6().text_size(theme::size_meta()).text_color(muted).child(text);
        let own = s.julia.is_none();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
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
                "New sessions start as if you'd chosen \"Always this session\". Applies to new and reopened sessions.".into(),
            ))
            .child(heading("Appearance"))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .children([(Appearance::Dark, "Dark"), (Appearance::Light, "Light"), (Appearance::System, "Match system")].map(
                        |(value, label)| {
                            Radio::new(label).text_size(theme::size_body()).checked(s.appearance == value).label(label).on_click(cx.listener(move |this, _, _, cx| {
                                this.update_settings(cx, |s| s.appearance = value);
                                this.apply_look(cx);
                            }))
                        },
                    )),
            )
            .child(note("Only the notebook follows it for now; Endeavor itself stays dark.".into()))
            .child(heading("Notebook theme"))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .children([(NotebookTheme::Endeavor, "Endeavor"), (NotebookTheme::Pluto, "Pluto")].map(|(value, label)| {
                        Radio::new(label).text_size(theme::size_body()).checked(s.notebook_theme == value).label(label).on_click(cx.listener(move |this, _, _, cx| {
                            this.update_settings(cx, |s| s.notebook_theme = value);
                            this.apply_look(cx);
                        }))
                    })),
            )
            .child(note("Endeavor matches the notebook to the app in dark mode; in light mode both use Pluto's light theme.".into()))
            .child(heading("Julia"))
            .child(
                Radio::new("julia-own")
                    .text_size(theme::size_body())
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
                            .text_size(theme::size_body())
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
                            .bg(theme::bg_raised())
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
                    .bg(theme::bg_raised())
                    .child("Show logs")
                    .on_click(|_, _, _| logs::reveal()),
            )
            .child(note("The log of this run and the one before, to attach to a bug report.".into()))
    }

    /// The notebook's ⋯ button in its header, and its menu while open.
    fn notebook_more(&self, key: u64, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let menu = self.menu.as_ref().filter(|menu| menu.target == MenuTarget::Notebook(key));
        div()
            .id("notebook-more")
            .role(Role::Button)
            .aria_label("Notebook")
            .relative()
            .flex_shrink_0()
            .size(px(24.))
            .mr(px(-6.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(theme::size_subhead())
            .text_color(if menu.is_some() { theme::text_primary() } else { theme::text_muted() })
            .when(menu.is_some(), |d| d.bg(theme::row_active()))
            .hover(|s| s.bg(theme::row_active()).text_color(theme::text_primary()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.open_menu(MenuTarget::Notebook(key), None, window, cx);
            }))
            .child("⋯")
            .children(menu.map(|menu| self.render_menu(menu, cx)))
    }

    /// The notebook pane drawn natively while a session has no notebook to show:
    /// before Claude creates it, while it opens, and once it's stopped. None shows
    /// the web view (Pluto's start page never shows for a session).
    fn notebook_stand_in(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = || div().flex().items_baseline().text_color(theme::text_muted());
        let Some(path) = session.notebook_path.as_deref() else {
            if session.notebook.is_some() {
                return None;
            }
            let folder = new_session::file_name(folder_name(&session.cwd));
            return Some(new_session::turtle_pane().child(line().child("Claude will create the notebook in ").child(folder).child(".")).into_any_element());
        };
        let file = new_session::file_name(folder_name(Path::new(path)));
        if session.stopped.is_some() {
            let key = session.key;
            let start = div()
                .id("start-notebook")
                .role(Role::Button)
                .px_3()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .bg(theme::accent())
                .text_color(theme::text_primary())
                .child("Start")
                .on_click(cx.listener(move |this, _, _, cx| this.start_notebook(key, cx)));
            return Some(new_session::turtle_pane().child(line().child(file).child(" is stopped.")).child(start).into_any_element());
        }
        if session.notebook.is_some() {
            return None;
        }
        Some(new_session::turtle_pane().child(line().child("Opening ").child(file).child("…")).into_any_element())
    }

    fn render_chat(&self, session: &Session, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let key = session.key;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .children(session.failed.as_ref().map(|failure| {
                div()
                    .m_3()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::danger())
                    .child(failure.message.clone())
                    .when(failure.can_copy, |d| {
                        d.child(
                            div()
                                .id("open-copy")
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(theme::accent())
                                .child("Open a copy")
                                .on_click(cx.listener(move |this, _, _, cx| this.open_copy(key, cx))),
                        )
                    })
            }))
            .child(session::render_transcript(session, cx))
            .children(session::render_activity(session))
            .child(
                div()
                    .px_4()
                    .pb(px(11.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .relative()
                    .children(self.picker.and_then(|id| self.render_picker(session, id, cx)))
                    .children(self.render_commands(session, cx))
                    .children(session::render_pinned_plan(session, cx))
                    .children(session::render_approval(session, cx))
                    .child(session::render_queue(session, cx))
                    // One-line box: Enter sends; the glyph becomes Stop while Claude works.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_h(px(38.))
                            .px(px(10.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme::composer_edge())
                            .bg(theme::bg_card())
                            .child(div().flex_1().child(Textarea::new(&self.input).appearance(false).text_size(theme::size_body())))
                            .child(if session.outbox.busy && session.id.is_some() {
                                div()
                                    .id("stop")
                                    .size(px(20.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .bg(theme::bg_raised())
                                    .text_size(theme::size_meta_small())
                                    .child("■")
                                    .on_click(cx.listener(|this, _, window, cx| this.interrupt(&Interrupt, window, cx)))
                                    .into_any_element()
                            } else {
                                div().text_color(theme::text_faint()).child("↵").into_any_element()
                            }),
                    )
                    // Toolbar under the box: point, mode · model, effort, context.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .h(px(24.))
                            .text_size(theme::size_meta())
                            .text_color(theme::text_new())
                            .child(
                                tool_button("point")
                                    .when(self.annotating, |d| d.text_color(theme::accent_text()))
                                    .child("↖ Point")
                                    .on_click(cx.listener(|this, _, window, cx| this.toggle_annotation(&ToggleAnnotation, window, cx))),
                            )
                            .children(session.mode_name().map(|name| {
                                tool_button("mode")
                                    .when(name.to_lowercase().contains("plan"), |d| d.text_color(theme::accent_text()))
                                    .child(name)
                                    .on_click(cx.listener(|this, _, window, cx| this.cycle_mode(&CycleMode, window, cx)))
                            }))
                            .child(div().flex_1())
                            .children(["model", "effort"].map(|id| {
                                session.config_label(id).map(|label| {
                                    tool_button(id)
                                        .text_color(theme::text_secondary())
                                        .when(self.picker == Some(id), |d| d.bg(theme::row_active()))
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.picker = if this.picker == Some(id) { None } else { Some(id) };
                                            cx.notify();
                                        }))
                                })
                            }).into_iter().flatten())
                            .children(session.usage.filter(|(_, size)| *size > 0).map(|(used, size)| {
                                // Shown on hover beside the ring: a tooltip would open under the notebook.
                                div()
                                    .id("context")
                                    .group("context")
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .px(px(5.))
                                    .child(
                                        div()
                                            .text_color(gpui::transparent_black())
                                            .group_hover("context", |s| s.text_color(theme::text_muted()))
                                            .child(format!("{}% context", used * 100 / size)),
                                    )
                                    .child(context_ring(used as f32 / size as f32))
                            })),
                    ),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active.and_then(|key| self.sessions.iter().position(|s| s.key == key));
        let stand_in = active.and_then(|ix| self.notebook_stand_in(&self.sessions[ix], cx));
        // The web view is a native view over the window: hidden behind the setup
        // screen, and wherever the notebook pane is drawn natively.
        let show_webview = self.setup.is_none() && active.is_some() && stand_in.is_none();
        if self.webview.read(cx).visible() != show_webview {
            self.webview.update(cx, |w, _| if show_webview { w.show() } else { w.hide() });
        }
        if !self.menu.as_ref().is_some_and(|m| matches!(m.target, MenuTarget::Notebook(_))) {
            overlay::set_hole(self.webview.read(cx).raw(), None);
        }
        if let Some(setup) = &self.setup {
            let sign_in = self.render_sign_in(cx);
            let retry = cx.listener(|this, _, _, cx| this.retry_setup(cx));
            return div().size_full().bg(theme::bg_page()).text_color(theme::text_primary()).text_size(theme::size_body()).child(splash::render(setup, sign_in, retry, cx)).into_any_element();
        }
        let working = active.is_some_and(|ix| self.sessions[ix].outbox.busy);
        let placeholder = match active {
            None => "What do you want to work on?",
            Some(_) if working => "Queue a message, or ⌘⏎ to steer",
            Some(_) => "Type / for commands",
        };
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            self.input.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        let chat = match active {
            _ if self.settings_open => self.render_settings(cx).into_any_element(),
            Some(ix) => self.render_chat(&self.sessions[ix], cx).into_any_element(),
            None => self.render_new_session(cx).into_any_element(),
        };
        // Chat header: the session and its folder; the notebook header: its file.
        let (title, folder) = match active {
            _ if self.settings_open => ("Settings".into(), None),
            Some(ix) => (self.sessions[ix].title.clone(), Some(folder_name(&self.sessions[ix].cwd))),
            None => ("New session".into(), None),
        };
        let chat_header = column_header("chat-header")
            .when(!self.settings.layout.sidebar_open, |d| d.pl(px(TRAFFIC_LIGHTS)).child(sidebar_toggle(cx)))
            .child(div().overflow_hidden().whitespace_nowrap().child(title))
            .children(folder.map(|f| {
                div().px(px(6.)).rounded(px(3.)).bg(theme::bg_tag()).text_color(theme::text_tag()).font_family(theme::MONO).text_size(theme::size_meta_small()).child(f)
            }));
        let notebook_header = column_header("notebook-header").map(|d| match active {
            None => d.child(self.draft_pane_header()),
            Some(ix) => match self.sessions[ix].notebook_path.as_deref() {
                Some(path) => d.child(new_session::notebook_title(Path::new(path))).child(div().flex_1()).child(self.notebook_more(self.sessions[ix].key, cx)),
                None => d,
            },
        });
        let notebook = match (active, stand_in) {
            (None, _) => self.render_draft_pane(),
            (Some(_), Some(stand_in)) => stand_in,
            (Some(_), None) => self.webview.clone().into_any_element(),
        };
        div()
            .key_context("Workspace")
            .on_action(cx.listener(Self::interrupt))
            .on_action(cx.listener(Self::toggle_annotation))
            .on_action(cx.listener(Self::cycle_mode))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::open_settings))
            .on_mouse_move(cx.listener(Self::drag_divider))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if this.resizing.take().is_some() {
                        this.settings.save();
                    }
                }),
            )
            .flex()
            .size_full()
            .bg(theme::bg_page())
            .text_color(theme::text_primary())
            .text_size(theme::size_body())
            .line_height(theme::line_body())
            .when(self.settings.layout.sidebar_open, |d| d.child(self.render_session_bar(cx)).child(self.divider(Divider::Sidebar, theme::sidebar_edge(), cx)))
            .child(
                div()
                    .w(px(self.settings.layout.chat_width))
                    .flex_shrink_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(chat_header)
                    .child(chat),
            )
            .child(self.divider(Divider::Chat, theme::divider(), cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(notebook_header)
                    .child(div().flex_1().min_h_0().child(notebook)),
            )
            // A click outside a chip's menu only closes it.
            .when(self.draft.popover.is_some() && active.is_none(), |d| {
                d.child(
                    div()
                        .id("chip-menu-backdrop")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_popover(window, cx))),
                )
            })
            .when(self.filter_menu, |d| {
                d.child(
                    div()
                        .id("filter-menu-backdrop")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                            this.filter_menu = false;
                            cx.notify();
                        })),
                )
            })
            // A click outside the menu only closes it, as with a native menu.
            .when(self.menu.is_some(), |d| {
                d.child(
                    div()
                        .id("menu-backdrop")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_menu(window, cx)))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, cx| this.close_menu(window, cx))),
                )
            })
            .into_any_element()
    }
}

/// Accessibility's Reduce motion, which GPUI doesn't read itself.
fn system_reduces_motion() -> bool {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let reduce: Bool = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
        reduce.as_bool()
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
        theme::load_fonts(cx);
        // Theme::change applies these before building the component defaults from them.
        let ui = Theme::global_mut(cx);
        ui.dark_theme = std::rc::Rc::new(ThemeConfig {
            font_family: Some(theme::SANS.into()),
            mono_font_family: Some(theme::MONO.into()),
            mono_font_size: Some(f32::from(theme::size_code())),
            ..(*ui.dark_theme).clone()
        });
        cx.set_reduce_motion(system_reduces_motion());
        // Input consumes Escape only when it has something to dismiss; otherwise it reaches us.
        cx.bind_keys([
            KeyBinding::new("escape", Interrupt, None),
            KeyBinding::new("cmd-shift-k", ToggleAnnotation, None),
            // Registered after gpui-component's, so it beats the text box's own ⇧⇥ (outdent).
            KeyBinding::new("shift-tab", CycleMode, Some("Input")),
            KeyBinding::new("shift-tab", CycleMode, None),
            KeyBinding::new("cmd-b", ToggleSidebar, Some("Input")),
            KeyBinding::new("cmd-b", ToggleSidebar, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-=", ZoomIn, None),
            KeyBinding::new("cmd--", ZoomOut, None),
            KeyBinding::new("cmd-0", ZoomReset, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        // Edit's items send the native cut:/copy:/paste:/selectAll: selectors, which
        // the notebook's web view needs for the clipboard; in our own text boxes
        // they become the input's actions.
        use gpui_component::input::{Copy, Cut, Paste, SelectAll};
        let menu = |name: &str, items| Menu { name: name.to_string().into(), items, disabled: false };
        cx.set_menus(vec![
            menu("Endeavor", vec![MenuItem::action("Settings…", OpenSettings), MenuItem::separator(), MenuItem::action("Quit Endeavor", Quit)]),
            menu(
                "Edit",
                vec![
                    MenuItem::os_action("Cut", Cut, OsAction::Cut),
                    MenuItem::os_action("Copy", Copy, OsAction::Copy),
                    MenuItem::os_action("Paste", Paste, OsAction::Paste),
                    MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                ],
            ),
            menu(
                "View",
                vec![
                    MenuItem::action("Toggle Sidebar", ToggleSidebar),
                    MenuItem::separator(),
                    MenuItem::action("Zoom In", ZoomIn),
                    MenuItem::action("Zoom Out", ZoomOut),
                    MenuItem::action("Actual Size", ZoomReset),
                ],
            ),
        ]);
        let bounds = Bounds::centered(None, size(px(1560.), px(900.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // No title bar: each column has its own 44px header, the traffic
                // lights sit in the sidebar's.
                titlebar: Some(TitlebarOptions {
                    title: Some("Endeavor".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(16.))),
                }),
                ..Default::default()
            },
            |window, cx| {
                Theme::change(ThemeMode::Dark, Some(window), cx);
                #[cfg(debug_assertions)]
                if let Some(preview) = splash::preview::open(cx) {
                    return cx.new(|cx| Root::new(preview, window, cx));
                }
                let workspace = cx.new(|cx| Workspace::new(window, cx));
                // Menu items and shortcuts pressed while the notebook has the keyboard reach
                // no focused GPUI element; these app-wide handlers forward them.
                let ws = workspace.downgrade();
                cx.on_action(move |_: &OpenSettings, cx| {
                    ws.update(cx, |this, cx| {
                        this.settings_open = true;
                        cx.notify();
                    })
                    .ok();
                });
                // The notebook's zoom, from anywhere (the notebook usually has the keyboard).
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ZoomIn, cx| drop(ws.update(cx, |this, cx| this.zoom(1.1, false, cx))));
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ZoomOut, cx| drop(ws.update(cx, |this, cx| this.zoom(1. / 1.1, false, cx))));
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ZoomReset, cx| drop(ws.update(cx, |this, cx| this.zoom(1., true, cx))));
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ToggleSidebar, cx| {
                    ws.update(cx, |this, cx| {
                        this.settings.layout.sidebar_open = !this.settings.layout.sidebar_open;
                        this.settings.save();
                        cx.notify();
                    })
                    .ok();
                });
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
    use super::{MenuPick, NotebookAction, Row, RowAction, viewed_notebook_id};

    #[test]
    fn notebook_id_from_pluto_url() {
        let id = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let url = format!("http://127.0.0.1:1234/edit?secret=s3cr3t&id={id}");
        assert_eq!(viewed_notebook_id(&url), Some(id));
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/?secret=s3cr3t"), None);
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/edit?id=../../secret"), None);
    }

    #[test]
    fn row_menu_items() {
        let labels = |row: &Row, archived| RowAction::for_row(row, archived).into_iter().map(RowAction::label).collect::<Vec<_>>();
        let past = Row::Past("s1".to_string().into(), "/tmp".into());
        assert_eq!(labels(&Row::Open(1), Some(false)), ["Rename", "Reveal folder in Finder", "Archive", "Close", "Delete…"]);
        assert_eq!(labels(&Row::Open(1), None), ["Rename", "Reveal folder in Finder", "Close", "Delete…"]);
        assert_eq!(labels(&past, Some(true)), ["Rename", "Reveal folder in Finder", "Unarchive", "Delete…"]);
    }

    #[test]
    fn notebook_menu_items() {
        let labels = |running| NotebookAction::for_notebook(running).into_iter().map(NotebookAction::label).collect::<Vec<_>>();
        assert_eq!(labels(true), ["Reveal in Finder", "Open in a new session…", "Stop notebook"]);
        assert_eq!(labels(false), ["Reveal in Finder", "Open in a new session…"]);
        assert!(MenuPick::Notebook(1, NotebookAction::Stop).danger());
        assert!(!MenuPick::Notebook(1, NotebookAction::Reveal).danger());
    }
}
