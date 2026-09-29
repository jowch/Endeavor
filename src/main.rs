//! Endeavor: app-owned Julia running Pluto + EndeavorRuntime, the live Pluto frontend in a
//! child webview, and ACP agent sessions (a session bar, one chat pane) wired to the
//! same Pluto session over MCP.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[cfg(target_os = "macos")]
mod dialogs;
#[cfg(target_os = "macos")]
mod webkeys;
mod about;
mod agent;
mod annotate;
mod attach;
mod celldiff;
mod composer;
mod connection;
#[cfg(debug_assertions)]
mod debug_state;
mod details;
mod gate;
mod host_list;
mod hosts;
mod install;
mod logs;
mod new_session;
mod network;
mod notebook_pane;
mod offline;
mod outbox;
#[cfg(target_os = "macos")]
mod overlay;
mod platform;
mod pluto;
mod remote;
mod resources;
mod runs;
mod runtime;
mod server_dialog;
mod session;
mod settings;
mod signin;
#[cfg(target_os = "macos")]
mod snapshot;
mod splash;
mod theme;
mod tips;
mod turtle;
mod when;
#[cfg(not(target_os = "macos"))]
use platform::{dialogs, overlay, snapshot, webkeys};

use agent::{AgentEvent, Command};
use agent_client_protocol::schema::v1::{ContentBlock, PermissionOptionKind, SessionId, SessionInfo, TextContent};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState, TextareaState};
use gpui_component::radio::Radio;
use gpui_component::{Root, Sizable, Theme, ThemeConfig, ThemeMode};
use gpui_wry::WebView;
use hosts::{HostId, Place};
use outbox::Queued;
use new_session::{Draft, Glyph, NotebookChoice, glyph, menu_row};
use notebook_pane::NotebookAction;
use session::{Effect, Session, Stopped, folder_name};
use settings::{Appearance, IdleStop, NotebookTheme, Settings};
use splash::{Progress, Setup};
use wire::backend::Backend;

/// Notebook id from a notebook page's URL. Only the id is used: the URL also
/// carries the notebook server's secret, which must never reach the agent.
fn viewed_notebook_id(url: &str) -> Option<&str> {
    Backend::Pluto.notebook_id(url).filter(|id| annotate::is_uuid(id))
}

actions!(
    endeavor,
    [
        Interrupt,
        ToggleAnnotation,
        CycleMode,
        ToggleSidebar,
        OpenSettings,
        Quit,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ShowAbout,
        Minimize,
        ZoomWindow,
        BringAllToFront,
        OpenHelp,
        ReportIssue
    ]
);

/// A small JSON file in Endeavor's Application Support folder.
fn app_file(name: &str) -> Option<PathBuf> {
    Some(install::app_dir().ok()?.join(name))
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
/// The window can't shrink below every column at its minimum, plus the two dividers.
const WINDOW_MIN: Size<Pixels> = Size { width: px(SIDEBAR_RANGE.0 + CHAT_MIN + NOTEBOOK_MIN + 2.), height: px(600.) };

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
        .role(Role::Button)
        .aria_label("Toggle sidebar")
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
        .role(Role::Button)
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

/// The status at an open session's sidebar row end.
#[derive(Clone, Copy, PartialEq)]
enum RowMark {
    /// A ring: it waits for your answer.
    NeedsApproval,
    /// The orbit: Claude is working.
    Working,
    Archived,
}

/// A session in the sidebar: an open one, or a past one listed under its folder.
#[derive(Clone, PartialEq)]
enum Row {
    Open(u64),
    Past(SessionId, Place),
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
    /// Finder can only reveal This Mac's folders.
    fn for_row(row: &Row, archived: Option<bool>, local: bool) -> Vec<RowAction> {
        Self::ALL
            .into_iter()
            .filter(|action| match action {
                RowAction::Reveal => local,
                RowAction::Archive => archived == Some(false),
                RowAction::Unarchive => archived == Some(true),
                RowAction::Close => matches!(row, Row::Open(_)),
                _ => true,
            })
            .collect()
    }
}

/// What a menu is for: a sidebar row (⋮), an open session's notebook (⋮), or
/// its Share button.
#[derive(Clone, PartialEq)]
enum MenuTarget {
    Row(Row),
    Notebook(u64),
    Share(u64),
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

    /// Shown in the danger colour.
    fn danger(&self) -> bool {
        match self {
            MenuPick::Row(_, action) => *action == RowAction::Delete,
            MenuPick::Notebook(_, action) => action.danger(),
        }
    }

    /// Items in one group sit together, with a separator between groups.
    fn group(&self) -> u8 {
        match self {
            MenuPick::Row(_, action) => (*action == RowAction::Delete) as u8,
            MenuPick::Notebook(_, action) => action.group(),
        }
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

/// Recently used working folders on every host, most recent first, kept across
/// launches. Only This Mac's can be checked for still being there.
fn load_recent() -> Vec<Place> {
    load_json::<Vec<Place>>("recent.json").into_iter().filter(|p| p.host != HostId::ThisMac || p.path.is_dir()).collect()
}

/// sessions.json: the sessions Endeavor created, with where each works. Before
/// servers it was a list of ids, all This Mac's (their folder is their cwd).
fn load_ours() -> HashMap<String, Place> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Places(HashMap<String, Place>),
        Ids(Vec<String>),
    }
    match load_json::<Option<Saved>>("sessions.json") {
        Some(Saved::Places(places)) => places,
        Some(Saved::Ids(ids)) => ids.into_iter().map(|id| (id, Place::local(PathBuf::new()))).collect(),
        None => HashMap::new(),
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
    /// The new-session screen's choices.
    draft: Draft,
    /// Working folders on every host, most recent first (persisted).
    recent: Vec<Place>,
    /// Past sessions per folder, from the agent's history.
    past: HashMap<Place, Vec<SessionInfo>>,
    /// Sessions Endeavor created, by id, with their host and folder (persisted).
    /// Only these are listed: Claude Code's history for a folder also holds CLI
    /// sessions, which aren't ours. A This Mac entry's folder is its cwd.
    ours: HashMap<String, Place>,
    /// Session names the user gave, by session id (persisted).
    titles: HashMap<String, String>,
    /// Ids of archived sessions (persisted): hidden from the sidebar's Active view.
    archived: HashSet<String>,
    /// The sidebar's Active / All filter menu is open.
    filter_menu: bool,
    /// Each session's notebook file, by session id (persisted), for reopening it
    /// with the session: one the app opened isn't in the agent's history.
    session_notebooks: HashMap<String, Place>,
    /// The session being renamed, and its name box.
    renaming: Option<(Row, Entity<InputState>)>,
    menu: Option<PopupMenu>,
    /// Folders showing all their past sessions, not just the newest.
    expanded: HashSet<Place>,
    settings: Settings,
    /// The divider being dragged.
    resizing: Option<Divider>,
    /// The chat box's chips, mentions and menus (the box itself is `input`).
    composer: composer::Composer,
    /// A sent chip's popover.
    chip_popover: Option<composer::ChipPopover>,
    /// The agent's config options (model, effort) as the last session offered
    /// them (persisted), so the new-session screen can offer them too.
    agent_options: Vec<agent_client_protocol::schema::v1::SessionConfigOption>,
    /// The Settings screen is in the chat pane.
    settings_open: bool,
    /// First launch: the setup screen covers the window until setup finishes.
    setup: Option<Setup>,
    /// The agent connected (setup's last step).
    agent_ready: bool,
    /// The agent stopped with an error (often a failed adapter install); About offers Update.
    agent_failed: bool,
    /// Claude Code's sign-in: checked when the agent starts and when the window
    /// comes back to the front, and lost when a turn fails for want of it.
    account: signin::Account,
    /// When the sign-in was last checked on coming to the front.
    sign_in_checked: Option<std::time::Instant>,
    /// Since when the network has been unreachable (None: online).
    offline_since: Option<std::time::Instant>,
    /// Try now is looking at the network.
    probing: bool,
    agent_tx: UnboundedSender<Command>,
    /// Handed to the agent thread once This Mac's Julia is up (setup's order).
    agent_rx: Option<UnboundedReceiver<Command>>,
    /// App-level status (Julia, agent connection), shown under the session bar.
    status: SharedString,
    annotating: bool,
    /// "Two ways to add your file" shows over the composer (a "Uses your file"
    /// example was clicked and the tip isn't done yet).
    file_tip: bool,
    /// Each host's connection and runtime.
    connections: HashMap<HostId, connection::Connection>,
    /// Each host's loopback ports for the webview and the agent, relayed to its
    /// runtime of the moment; kept for the whole launch.
    listeners: HashMap<HostId, Arc<runtime::Listener>>,
    /// The composer's placeholder as last set (it changes while Claude works).
    placeholder: &'static str,
    /// Servers sessions can run on (persisted in hosts.json).
    hosts: hosts::Hosts,
    /// Adding a server, or its settings.
    server_dialog: Option<server_dialog::ServerDialog>,
    /// ssh's prompts waiting for an answer, the first on screen.
    asks: VecDeque<server_dialog::AskModal>,
    /// Each cluster session's resources, by session id (persisted), for the
    /// job that runs it after a reopen.
    session_resources: HashMap<String, wire::slurm::Resources>,
    /// Each session's mode, by session id (persisted), to start it in again after a reopen.
    session_modes: HashMap<String, session::Mode>,
    /// Starting a session on a server that looks like a cluster's login node
    /// waits on this question (the server's id).
    login_node_warning: Option<String>,
    /// Servers the user chose to use as plain servers anyway, this launch.
    login_node_ok: HashSet<String>,
    questions_tx: UnboundedSender<remote::Question>,
    /// The shown notebook's state as its page reports it (safe preview, busy work, the drawer).
    page: annotate::PageState,
    /// The last `context` message sent to the page (resent when it changes).
    page_context: String,
    /// The notebook being renamed in its header, and its name box.
    notebook_rename: Option<(u64, Entity<InputState>)>,
    /// The session whose job resources are open from the Julia-not-running page's gear.
    pane_resources: Option<u64>,
    pane_partition_menu: bool,
    /// A state dump waiting for the page's part (debug_state.rs).
    #[cfg(debug_assertions)]
    page_debug: Option<futures::channel::oneshot::Sender<serde_json::Value>>,
    /// Modal dialog buttons' Tab-stop handles, keyed by their static id: dialogs
    /// (server_dialog.rs) are rebuilt fresh each render, so their handles live here.
    dialog_focus: std::cell::RefCell<HashMap<&'static str, FocusHandle>>,
    /// Past (not open) sidebar rows' Tab-stop handles, keyed by session id: unlike
    /// an open session, a past row has no `Session` to hold its own handle.
    past_row_focus: std::cell::RefCell<HashMap<SessionId, FocusHandle>>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (page_tx, mut page_rx) = futures::channel::mpsc::unbounded::<String>();
        let webview = cx.new(|cx| {
            let handle = platform::webview_parent(window);
            let webview = wry::WebViewBuilder::new()
                // wry's url() panics on a web view that has never loaded a page.
                .with_url("about:blank")
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
            dialogs::show_page_dialogs(&webview);
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
                .auto_grow(1, 10)
        });
        composer::subscribe(&input, window, cx);
        cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            // The slash-command menu follows what's typed.
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
            if let InputEvent::PressEnter { secondary, shift: false } = event {
                // An empty box answers the sign-in card: ⏎ signs in again.
                if input.read(cx).value().trim().is_empty() && this.sign_in_again(cx) {
                    return;
                }
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
                } else if !this.composer_empty(cx) {
                    this.start_session(window, cx);
                }
            }
        })
        .detach();

        let (questions_tx, mut questions) = futures::channel::mpsc::unbounded::<remote::Question>();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(question) = questions.next().await {
                if this.update_in(cx, |this, window, cx| this.on_question(question, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        // The helpers finish the job after the app is gone.
        cx.on_app_quit(|this, _| {
            this.quit_runtimes();
            async {}
        })
        .detach();

        // Tick the "Working · 12s" and "Starting Julia · 0:12" timers once a second.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok(busy) = this.update(cx, |this, _| {
                this.sessions.iter().any(|s| s.busy_since.is_some())
                    || this.connections.values().any(|c| matches!(c.status, connection::Status::Connecting | connection::Status::Starting))
            }) else {
                break;
            };
            if busy {
                let _ = this.update(cx, |_, cx| cx.notify());
            }
        })
        .detach();

        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.recheck_sign_in(cx);
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
            // Keys name sessions to the runtime, which can outlive a launch (Keep notebooks
            // running): starting from the clock keeps them from matching an earlier launch's.
            next_key: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_millis() as u64),
            draft,
            recent,
            past: HashMap::new(),
            ours: load_ours(),
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
            composer: composer::Composer::new(cx),
            chip_popover: None,
            agent_options: load_json("agent-options.json"),
            setup: Setup::needed().then(Setup::default),
            agent_ready: false,
            agent_failed: false,
            account: signin::Account::Unknown,
            sign_in_checked: None,
            offline_since: None,
            probing: false,
            agent_tx,
            agent_rx: Some(agent_rx),
            status: "".into(),
            annotating: false,
            file_tip: false,
            connections: HashMap::new(),
            listeners: HashMap::new(),
            placeholder: "Type / for commands",
            hosts: hosts::Hosts::load(),
            server_dialog: None,
            asks: VecDeque::new(),
            session_resources: load_json("resources.json"),
            session_modes: load_json("modes.json"),
            login_node_warning: None,
            login_node_ok: HashSet::new(),
            questions_tx,
            page: annotate::PageState::default(),
            page_context: String::new(),
            notebook_rename: None,
            pane_resources: None,
            pane_partition_menu: false,
            #[cfg(debug_assertions)]
            page_debug: None,
            dialog_focus: std::cell::RefCell::new(HashMap::new()),
            past_row_focus: std::cell::RefCell::new(HashMap::new()),
        };
        settings::set_webview_appearance(this.webview.read(cx).raw(), this.settings.appearance);
        // This Mac's Julia boots while the user picks a folder on the new-session screen.
        this.connect_host(&HostId::ThisMac, true, cx);
        this.scan_notebooks(cx);
        this.watch_network(cx);
        #[cfg(debug_assertions)]
        this.watch_state_requests(cx);
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
            self.save_mode(key);
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
    /// text (if any) as its first message. A chosen notebook opens in safe
    /// preview. On a server, Julia starts now if it isn't running.
    fn start_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popover(window, cx);
        if self.composer_empty(cx) {
            return;
        }
        let host = self.draft.host.clone();
        let Some(folder) = self.draft.folder.clone() else {
            self.draft.notice = Some(format!("Waiting for the connection to {}.", self.hosts.name(&host)).into());
            return cx.notify();
        };
        if host == HostId::ThisMac {
            let _ = std::fs::create_dir_all(&folder);
        }
        if let HostId::Server(id) = &host
            && !self.is_cluster(&host)
            && !self.login_node_ok.contains(id)
            && self.connection(&host).and_then(|c| c.hello.as_ref()).is_some_and(|h| h.slurm)
        {
            self.login_node_warning = Some(id.clone());
            return cx.notify();
        }
        let place = Place { host: host.clone(), path: folder.clone() };
        let server = match &host {
            HostId::ThisMac => None,
            HostId::Server(_) => Some(self.hosts.name(&host)),
        };
        let key = self.next_key;
        self.next_key += 1;
        self.recent.retain(|p| p != &place);
        self.recent.insert(0, place.clone());
        save_json("recent.json", &self.recent);
        if !self.past.contains_key(&place) {
            let _ = self.agent_tx.unbounded_send(Command::ListSessions { cwd: host.agent_cwd(&folder) });
        }
        let mut session = Session::new(key, place, server.clone());
        session.resources = self.draft.resources.clone().filter(|_| self.is_cluster(&host));
        session.start_mode = session::app_modes().get(self.draft.mode).map(|choice| session::Mode {
            // Settings' "Run notebook code without asking" is Manual's "Always this session".
            run_without_asking: choice.run_without_asking || (self.draft.mode == session::MANUAL && self.settings.run_without_asking),
            ..choice.mode()
        });
        let job = session.resources.as_ref().zip(self.draft_cluster()).map(|(resources, cluster)| cluster.job(resources));
        if let (Some(job), Some(connection)) = (job, self.connections.get_mut(&host)) {
            connection.job_request = Some(job);
        }
        let existing = match &self.draft.notebook {
            NotebookChoice::New => None,
            NotebookChoice::Existing(path) => Some(path.display().to_string()),
        };
        let mut context = Vec::new();
        if let (Some(server), HostId::Server(id)) = (&server, &host) {
            let ssh = self.hosts.server(id).map(|s| s.ssh_host.clone()).unwrap_or_default();
            context.push(format!(
                "[Endeavor] This session works on the server {server} (ssh host {ssh}), in the folder {}. Julia, Pluto, \
                 the notebook and the files are all on that server; your own file and shell tools are off because \
                 they'd see the user's Mac instead. Use the notebook tools list_folder, read_file and run_shell (the user \
                 approves each command), with the server's paths.",
                folder.display()
            ));
            if let Some(resources) = &session.resources {
                context.push(format!(
                    "[Endeavor] {server} is a Slurm cluster: Julia runs in a batch job ({}) on a compute node, and \
                     run_shell runs there too, inside that job. The notebooks stop when the job's time limit is reached.",
                    resources.summary()
                ));
            }
        }
        if let Some(path) = &existing {
            session.open_on_start(path.clone());
            context.push(format!(
                "[Endeavor] The user started this session on the Pluto notebook {path}, which is open in the \
                 notebook pane in safe preview (nothing has run). Unless they say otherwise, \"the notebook\" \
                 means this one; list_notebooks gives its id."
            ));
        }
        session.start_context = (!context.is_empty()).then(|| ContentBlock::Text(TextContent::new(context.join("\n\n"))));
        if self.holds(&session) {
            session.hold();
        }
        self.sessions.push(session);
        if let Some(path) = existing {
            self.bind_notebook(key, path, cx);
        }
        self.request_agent(key, cx);
        self.draft.notebook = NotebookChoice::New;
        self.draft.preview = None;
        self.activate(key, cx);
        self.send(key, None, false, window, cx);
    }

    /// Open the agent side of a session (new, or its history reloaded) once its
    /// host's runtime is ready, since its tools go to that runtime's bridge.
    /// Until then it waits, and the host connects and starts Julia.
    pub fn request_agent(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let host = session.place.host.clone();
        let Some(bridge) = self.bridge(&host) else { return self.ensure_runtime(&host, cx) };
        let tools = agent::Tools { bridge: bridge.clone(), server: session.server.clone() };
        let folder = session.place.path.clone();
        cx.background_executor().spawn(async move { pluto::set_session_folder(&bridge, key, &folder) }).detach();
        let cwd = host.agent_cwd(&session.place.path);
        let _ = std::fs::create_dir_all(&cwd);
        let command = match session.id.clone() {
            Some(id) => Command::LoadSession { key, id, cwd, tools },
            None => Command::NewSession { key, cwd, tools },
        };
        let _ = self.agent_tx.unbounded_send(command);
        if let Some(session) = self.session_mut(key) {
            session.agent_waiting = false;
        }
    }

    /// Where a past session works: This Mac's by the folder it ran in, a server's as recorded.
    fn past_place(&self, info: &SessionInfo) -> Option<Place> {
        match self.ours.get(&info.session_id.to_string())? {
            Place { host: HostId::ThisMac, .. } => Some(Place::local(info.cwd.clone())),
            place => Some(place.clone()),
        }
    }

    /// A past session's title: the user's name for it, else the agent's, else
    /// its notebook's name; true when it only stands in for a title.
    fn past_title(&self, info: &SessionInfo) -> (String, bool) {
        let id = info.session_id.to_string();
        if let Some(title) = self.titles.get(&id).cloned().or_else(|| info.title.as_deref().and_then(session::agent_title).map(str::to_owned)) {
            return (title, false);
        }
        let notebook = self.session_notebooks.get(&id).and_then(|p| p.path.file_name()).map(|n| n.to_string_lossy().into_owned());
        (notebook.unwrap_or_else(|| "Earlier session".into()), true)
    }

    /// Reopen a past session (or switch to it if it's already open).
    fn open_past(&mut self, info: SessionInfo, place: Place, cx: &mut Context<Self>) {
        if let Some(key) = self.sessions.iter().find(|s| s.id.as_ref() == Some(&info.session_id)).map(|s| s.key) {
            return self.activate(key, cx);
        }
        let key = self.next_key;
        self.next_key += 1;
        let named = self.titles.get(&info.session_id.to_string()).cloned();
        let resources = self.session_resources.get(&info.session_id.to_string()).cloned();
        let mode = self.session_modes.get(&info.session_id.to_string()).cloned();
        let (title, untitled) = self.past_title(&info);
        let notebook = self.session_notebooks.get(&info.session_id.to_string()).map(|p| p.path.display().to_string());
        let server = (place.host != HostId::ThisMac).then(|| self.hosts.name(&place.host));
        let mut session = Session::loading(key, info.session_id, place, server, title);
        if let Some(path) = &notebook {
            session.open_on_start(path.clone());
        }
        session.named = named.is_some();
        session.untitled = untitled;
        // A session from before modes were saved keeps the agent's own mode.
        session.run_without_asking = self.settings.run_without_asking;
        session.start_mode = mode;
        session.resources = resources;
        if self.holds(&session) {
            session.hold();
        }
        self.sessions.push(session);
        if let Some(path) = notebook {
            self.bind_notebook(key, path, cx);
        }
        self.request_agent(key, cx);
        self.activate(key, cx);
    }

    /// Stop an open session; it goes back to its folder's history.
    fn close_session(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.sessions.iter().position(|s| s.key == key) else { return };
        let session = self.sessions.remove(ix);
        if let Some(id) = session.id {
            let _ = self.agent_tx.unbounded_send(Command::CloseSession(id));
        }
        let _ = self.agent_tx.unbounded_send(Command::ListSessions { cwd: session.place.host.agent_cwd(&session.place.path) });
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
        if self.session_modes.remove(&id.to_string()).is_some() {
            save_json("modes.json", &self.session_modes);
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
        let local = match row {
            Row::Open(key) => self.sessions.iter().find(|s| s.key == *key).is_some_and(|s| s.place.host == HostId::ThisMac),
            Row::Past(_, place) => place.host == HostId::ThisMac,
        };
        RowAction::for_row(row, archived, local)
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
                Some(self.past_title(info).0)
            }
        }
    }

    fn menu_picks(&self, target: &MenuTarget) -> Vec<MenuPick> {
        match target {
            MenuTarget::Row(row) => self.row_actions(row).into_iter().map(|action| MenuPick::Row(row.clone(), action)).collect(),
            MenuTarget::Notebook(key) => {
                let session = self.sessions.iter().find(|s| s.key == *key);
                let open = session.is_some_and(|s| s.notebook.is_some() && s.stopped.is_none() && !s.missing);
                let stopped = session.is_some_and(|s| s.stopped.is_some() && !s.missing);
                let safe = session.and_then(|s| s.notebook.as_deref()).is_some_and(|id| self.page.notebook == id && self.page.safe);
                let local = session.is_some_and(|s| s.place.host == HostId::ThisMac);
                NotebookAction::for_notebook(open, stopped, safe, local).into_iter().map(|action| MenuPick::Notebook(*key, action)).collect()
            }
            MenuTarget::Share(key) => NotebookAction::for_share().into_iter().map(|action| MenuPick::Notebook(*key, action)).collect(),
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

    /// A menu or popover open that a click outside it, even on the web view,
    /// or Esc should close: the session row / notebook ⋮ and Share menus, the
    /// composer's Mode/Plus/Config menu, a sent chip's popover, the sidebar's
    /// Active/All filter, and a new-session chip's popover.
    fn dismissible_open(&self) -> bool {
        self.menu.is_some()
            || self.composer.menu.is_some()
            || self.chip_popover.is_some()
            || self.filter_menu
            || (self.draft.popover.is_some() && self.active.is_none())
    }

    /// Close whatever `dismissible_open` found, as a click outside it does.
    fn close_dismissible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        self.close_composer_menus(cx);
        self.filter_menu = false;
        if self.draft.popover.is_some() {
            self.close_popover(window, cx);
        }
        cx.notify();
    }

    fn pick(&mut self, pick: MenuPick, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        match pick {
            MenuPick::Row(row, action) => self.row_action(row, action, window, cx),
            MenuPick::Notebook(key, action) => self.notebook_action(key, action, window, cx),
        }
    }

    fn row_action(&mut self, row: Row, action: RowAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            RowAction::Rename => self.start_rename(row, window, cx),
            RowAction::Reveal => {
                let folder = match &row {
                    Row::Open(key) => self.sessions.iter().find(|s| s.key == *key).map(|s| s.place.clone()),
                    Row::Past(_, place) => Some(place.clone()),
                };
                if let Some(Place { host: HostId::ThisMac, path: folder }) = folder {
                    platform::reveal(&folder);
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
        let Some(bridge) = self.session_bridge(key) else { return };
        let Some(session) = self.session_mut(key) else { return };
        let cwd = session.place.host.agent_cwd(&session.place.path);
        let tools = agent::Tools { bridge, server: session.server.clone() };
        if let Some(source) = session.reopen_as_copy() {
            let _ = self.agent_tx.unbounded_send(Command::ForkSession { key, source, cwd, tools });
        }
        cx.notify();
    }

    /// Open a session's notebook file in the current Pluto (reusing it if it's
    /// already open; otherwise running it only if `run`) and point the session, the
    /// pane if active, and any session whose copy was stopped, at it.
    fn open_for_session(&mut self, key: u64, path: String, run: bool, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        let opened = cx.background_executor().spawn({
            let path = path.clone();
            async move {
                let listed = pluto::call_tool(&bridge, "list_notebooks", serde_json::json!({})).ok();
                let open = listed.as_ref().and_then(|l| l.as_array()?.iter().find(|nb| nb["path"] == path.as_str()).cloned());
                let nb = match open {
                    Some(nb) => nb,
                    None => pluto::call_tool(&bridge, "open_notebook", serde_json::json!({ "path": path, "run_notebook": run })).ok()?,
                };
                nb["notebook_id"].as_str().map(str::to_owned)
            }
        });
        cx.spawn(async move |this, cx| {
            let Some(id) = opened.await else {
                // A notebook file that's gone shows File not found.
                let _ = this.update(cx, |this, cx| this.check_missing(key, cx));
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let Some(host) = this.sessions.iter().find(|s| s.key == key).map(|s| s.place.host.clone()) else { return };
                let keys: Vec<u64> = this
                    .sessions
                    .iter()
                    .filter(|s| s.key == key || (s.place.host == host && s.stopped.is_some() && s.notebook_path.as_deref() == Some(path.as_str())))
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
        let place = Place { host: session.place.host.clone(), path: PathBuf::from(&path) };
        if let Some(id) = session.id.as_ref().map(ToString::to_string)
            && self.session_notebooks.get(&id) != Some(&place)
        {
            self.session_notebooks.insert(id, place);
            save_json("notebooks.json", &self.session_notebooks);
        }
        self.send_binding(key, path, cx);
    }

    fn send_binding(&self, key: u64, path: String, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        // ponytail: a failed send leaves the session unbound until its first open binds it.
        cx.background_executor().spawn(async move { pluto::set_notebook(&bridge, key, &path) }).detach();
    }

    /// Stop session `key`'s notebook at `path` (Pluto shuts it down); every
    /// session on it shows it stopped, with Start.
    fn stop_notebook(&mut self, key: u64, path: String, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        let Some(host) = self.sessions.iter().find(|s| s.key == key).map(|s| s.place.host.clone()) else { return };
        let stop = cx.background_executor().spawn({
            let path = path.clone();
            async move { pluto::stop_notebook(&bridge, &path) }
        });
        cx.spawn(async move |this, cx| {
            let result = stop.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    // Not open (already stopped elsewhere): Start reopens it in safe preview.
                    Ok(safe_preview) => {
                        let stopped = Stopped { safe_preview: safe_preview.unwrap_or(true), idle_hours: None, modified: None };
                        for session in this.sessions.iter_mut().filter(|s| s.place.host == host && s.notebook_path.as_deref() == Some(path.as_str())) {
                            session.notebook = None;
                            session.stopped = Some(stopped);
                        }
                        this.note_stopped_file(&host, path.clone(), cx);
                    }
                    Err(e) => this.status = format!("⚠ Couldn't stop {}: {e}", folder_name(Path::new(&path))).into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Show a session; the notebook pane follows it to the notebook it last viewed
    /// (without one, the pane draws a stand-in over the hidden web view).
    fn activate(&mut self, key: u64, cx: &mut Context<Self>) {
        self.active = Some(key);
        self.settings_open = false;
        self.follow_folder(cx);
        if let Some((host, notebook)) = self.active_session().and_then(|s| Some((s.place.host.clone(), s.notebook.clone()?))) {
            self.load_notebook(&host, &notebook, cx);
        }
        cx.notify();
    }

    /// Pluto's new notebooks start unsaved; point its "Save notebook" suggestion
    /// at the active session's folder (the page picks it up on its next load).
    pub fn follow_folder(&mut self, cx: &mut Context<Self>) {
        let Some((key, folder)) = self.active_session().map(|s| (s.key, s.place.path.clone())) else { return };
        let Some(bridge) = self.session_bridge(key) else { return };
        // ponytail: a failed send leaves Pluto suggesting the previous folder.
        cx.background_executor().spawn(async move { pluto::set_folder(&bridge, &folder) }).detach();
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
                    let Some(session) = self.session_mut(key) else { continue };
                    session.notebook = Some(id.clone());
                    session.stopped = None;
                    session.missing = false;
                    let host = session.place.host.clone();
                    if let Some(path) = path {
                        self.bind_notebook(key, path, cx);
                    }
                    if self.active == Some(key) {
                        self.load_notebook(&host, &id, cx);
                    }
                }
                Effect::CheckRunState => self.check_run_state(key, cx),
                Effect::SetPolicy(policy) => self.send_policy(key, policy, cx),
                Effect::SignedOut => self.signed_out(cx),
                Effect::PreviewRun { ix, tool, input } => {
                    let Some(bridge) = self.session_bridge(key) else { continue };
                    let task = cx.background_executor().spawn(async move { pluto::run_preview(&bridge, &tool, &input) });
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
        self.save_mode(key);
        cx.notify();
    }

    /// Save a session's mode when it changed, for its reopening.
    fn save_mode(&mut self, key: u64) {
        let Some((id, mode)) = self.session_mut(key).and_then(|s| Some((s.id.as_ref()?.to_string(), s.mode()?))) else { return };
        if self.session_modes.get(&id) != Some(&mode) {
            self.session_modes.insert(id, mode);
            save_json("modes.json", &self.session_modes);
        }
    }

    /// Which notebook the user is looking at, so "the notebook" is unambiguous.
    /// Also remembered as the active session's notebook.
    fn viewing_context(&mut self, cx: &mut Context<Self>) -> Option<ContentBlock> {
        // Hidden under a drawn pane, the web view can still hold another session's notebook.
        if !self.webview.read(cx).visible() {
            return None;
        }
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
        if self.composer_empty(cx) {
            return;
        }
        let context = self.viewing_context(cx);
        self.send(key, context, now, window, cx);
    }

    /// Send what's in the composer (if anything) to a session, after `context`.
    /// Files added from outside the session's folder are copied into it first
    /// (off the main thread; to a server, through its helper, with progress
    /// under the box for a big one), so a chip removed before sending leaves
    /// nothing.
    /// Meanwhile the message waits in the queue, and what's sent after it waits behind it.
    fn send(&mut self, key: u64, context: Option<ContentBlock>, now: bool, window: &mut Window, cx: &mut Context<Self>) {
        /// Smaller files are sent before progress would be worth reading.
        const SHOW_PROGRESS: u64 = 4_000_000;
        let Some(place) = self.session_mut(key).map(|s| s.place.clone()) else { return };
        let Some((text, attachments, mentioned)) = self.take_composer(window, cx) else { return };
        if !attachments.iter().any(|a| matches!(a, attach::Attachment::Upload { .. })) {
            return self.submit_message(key, context, (text, attachments, mentioned), now, cx);
        }
        let (queued, ticket) = Queued::copying(text.clone(), attachments.clone(), context.into_iter().collect());
        let Some(session) = self.session_mut(key) else { return };
        let effects = session.submit(queued, now);
        self.apply_effects(key, effects, cx);
        let (progress, mut progressed) = futures::channel::mpsc::unbounded::<attach::Progress>();
        let channel = self.connection(&place.host).and_then(|c| c.channel.clone());
        let (dest, fallback) = match (&place.host, channel) {
            (HostId::ThisMac, _) => (attach::Dest::Here(place.path), None),
            (host, _) if !self.helper_saves_files(host) => (attach::Dest::Message, Some(attach::UNWRITABLE)),
            (_, Some(channel)) => {
                let ask = Box::new(move |request| channel.files(request));
                let progress = Box::new(move |p| drop(progress.unbounded_send(p)));
                (attach::Dest::Server { folder: place.path, ask, progress }, None)
            }
            (_, None) => (attach::Dest::Message, Some("The server isn't connected, so files went in the message instead (text up to 250 KB).")),
        };
        let placing = cx.background_spawn(async move { attach::place_uploads(attachments, &dest) });
        cx.spawn(async move |this, cx| {
            let mut shown = None;
            while let Some(p) = progressed.next().await {
                if p.size < SHOW_PROGRESS {
                    continue;
                }
                let line = format!("Copying {} to the server: {} of {}", p.name, attach::size_text(p.sent), attach::size_text(p.size));
                shown = Some(line.clone());
                let _ = this.update(cx, |this, cx| {
                    this.composer.notice = Some(line);
                    cx.notify();
                });
            }
            let (attachments, refused) = placing.await;
            let _ = this.update(cx, |this, cx| {
                if shown.is_some() && this.composer.notice == shown {
                    this.composer.notice = None;
                }
                if let Some(why) = fallback {
                    this.composer.notice = Some(why.into());
                }
                if !refused.is_empty() {
                    let why = [fallback.unwrap_or_default(), &refused.join(" "), "The message went without it."].join(" ");
                    this.composer.notice = Some(why.trim_start().into());
                }
                let done = (!text.is_empty() || !attachments.is_empty()).then(|| {
                    let blocks = attach::prompt_blocks(&text, &attachments, &mentioned);
                    (attachments, blocks)
                });
                let Some(session) = this.session_mut(key) else { return };
                let effects = session.copied(ticket, done);
                this.apply_effects(key, effects, cx);
            });
        })
        .detach();
    }

    /// Submit a message as `take_composer` gives it: words, chips, mentions.
    fn submit_message(&mut self, key: u64, context: Option<ContentBlock>, message: (String, Vec<attach::Attachment>, Vec<String>), now: bool, cx: &mut Context<Self>) {
        let (text, attachments, mentioned) = message;
        let mut blocks: Vec<_> = context.into_iter().collect();
        blocks.extend(attach::prompt_blocks(&text, &attachments, &mentioned));
        let Some(session) = self.session_mut(key) else { return };
        let effects = session.submit(Queued::new(text, attachments, blocks), now);
        self.apply_effects(key, effects, cx);
    }

    pub fn send_policy(&self, key: u64, policy: &'static str, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        // ponytail: a failed send leaves the runtime's policy stale until the next change.
        cx.background_executor().spawn(async move { pluto::set_policy(&bridge, key, policy) }).detach();
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
        if self.close_composer_menus(cx) {
            return;
        }
        if self.filter_menu {
            self.filter_menu = false;
            return cx.notify();
        }
        if self.draft.popover.is_some() {
            return self.close_popover(window, cx);
        }
        if self.setup.is_none() && self.cancel_sign_in(cx) {
            return;
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
                self.agent_failed = false;
                self.finish_setup(cx);
                let mut listed = HashSet::new();
                for place in &self.recent {
                    let cwd = place.host.agent_cwd(&place.path);
                    if listed.insert(cwd.clone()) {
                        let _ = self.agent_tx.unbounded_send(Command::ListSessions { cwd });
                    }
                }
            }
            AgentEvent::Setup(p) => self.on_progress(p, cx),
            AgentEvent::SignedIn(method) => self.on_signed_in(method, cx),
            AgentEvent::Listed { cwd, sessions } => match HostId::of_agent_cwd(&cwd) {
                // One listing holds all of a server's folders.
                Some(host) => {
                    self.past.retain(|place, _| place.host != host);
                    for info in sessions {
                        if let Some(place) = self.past_place(&info).filter(|p| p.host == host) {
                            self.past.entry(place).or_default().push(info);
                        }
                    }
                }
                None => {
                    self.past.insert(Place::local(cwd), sessions);
                }
            },
            AgentEvent::Forked { key, id } => {
                if let Some(session) = self.session_mut(key) {
                    session.id = Some(id);
                }
            }
            AgentEvent::Failed(e) => {
                self.status = format!("⚠ Agent stopped: {e}").into();
                self.agent_failed = true;
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
                        let place = session.place.clone();
                        if self.ours.insert(id.to_string(), place.clone()).as_ref() != Some(&place) {
                            save_json("sessions.json", &self.ours);
                        }
                        if let Some(resources) = self.session_mut(key).and_then(|s| s.resources.clone())
                            && self.session_resources.insert(id.to_string(), resources.clone()).as_ref() != Some(&resources)
                        {
                            save_json("resources.json", &self.session_resources);
                        }
                        // Renamed before the agent assigned an id.
                        if let Some(name) = self.session_mut(key).filter(|s| s.named).map(|s| s.title.clone()) {
                            self.titles.insert(id.to_string(), name);
                            save_json("titles.json", &self.titles);
                        }
                        if !started.config.is_empty() && started.config != self.agent_options {
                            self.agent_options = started.config.clone();
                            save_json("agent-options.json", &self.agent_options);
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
            AgentEvent::Session(id, agent::SessionEvent::ApiFailed(error)) => {
                let Some(key) = self.sessions.iter().find(|s| s.id.as_ref() == Some(&id)).map(|s| s.key) else { return };
                self.api_failed(key, error, cx);
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

    /// Show notebook `id` of `host`'s Pluto in the pane.
    pub fn load_notebook(&mut self, host: &HostId, id: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self.connection(host).and_then(|c| c.runtime.as_ref()) else { return };
        // pluto_url carries Pluto's secret; keep it app-side.
        let url = Backend::Pluto.notebook_url(&runtime.pluto_url, id);
        self.webview.update(cx, |w, _| w.load_url(&url));
    }

    fn on_page_message(&mut self, body: &str, cx: &mut Context<Self>) {
        match annotate::parse(body) {
            Some(annotate::Message::Ready) => {
                // A fresh page: it gets its context again, and reports its own state.
                self.page = annotate::PageState::default();
                self.page_context.clear();
                self.push_cells(cx);
                self.apply_look(cx);
                self.sync_page_context(cx);
            }
            Some(annotate::Message::State(state)) => return self.on_page_state(state, cx),
            Some(annotate::Message::RunNotebook { notebook }) => self.run_notebook(notebook, cx),
            Some(annotate::Message::Restart { notebook }) => {
                if let Some(key) = self.active_session().filter(|s| s.notebook.as_deref() == Some(notebook.as_str())).map(|s| s.key) {
                    self.restart_notebook(key, cx);
                }
            }
            Some(annotate::Message::FixPackage { notebook, name, log }) => self.fix_package(notebook, name, log, cx),
            Some(annotate::Message::Mode(on)) => {
                self.annotating = on;
                if on {
                    self.point_tip_done();
                }
            }
            Some(annotate::Message::Ask(ask)) => {
                let Some(key) = self.active else { return };
                let mut blocks: Vec<_> = self.viewing_context(cx).into_iter().collect();
                let attachments = vec![ask.attachment];
                blocks.extend(attach::prompt_blocks(&ask.text, &attachments, &[]));
                let Some(session) = self.session_mut(key) else { return };
                let effects = session.submit(Queued::new(ask.text, attachments, blocks), ask.now);
                self.apply_effects(key, effects, cx);
            }
            Some(annotate::Message::Region(region)) => self.send_region(region, cx),
            Some(annotate::Message::Code { cell, code }) => self.on_cell_code(cell, code, cx),
            #[cfg(debug_assertions)]
            Some(annotate::Message::Debug(page)) => return self.on_page_debug(page),
            None => return,
        }
        cx.notify();
    }

    /// A box drawn with Point: take its picture, then send it with the cells
    /// under it. The page hides Point's dimming and outlines until told "shot".
    /// Without a picture it goes as the cells alone.
    fn send_region(&mut self, region: annotate::Region, cx: &mut Context<Self>) {
        let zoom = if self.settings.zoom > 0. { self.settings.zoom } else { 1. };
        let rect = region.rect.map(|n| n * zoom);
        let shot = snapshot::png(self.webview.read(cx).raw(), rect);
        cx.spawn(async move |this, cx| {
            let png = shot.await.ok().flatten();
            this.update(cx, |this, cx| {
                this.send_to_page(&serde_json::json!({ "type": "shot" }), cx);
                let Some(key) = this.active else { return };
                let annotate::Region { text, notebook, cells, now, .. } = region;
                let attachment = match png {
                    Some(png) => attach::Attachment::Region { notebook, cells, png: Arc::new(png) },
                    None if cells.is_empty() => return,
                    None => attach::Attachment::Cells { notebook, cells, ask: attach::CellAsk::About },
                };
                let mut blocks: Vec<_> = this.viewing_context(cx).into_iter().collect();
                let attachments = vec![attachment];
                blocks.extend(attach::prompt_blocks(&text, &attachments, &[]));
                let Some(session) = this.session_mut(key) else { return };
                let effects = session.submit(Queued::new(text, attachments, blocks), now);
                this.apply_effects(key, effects, cx);
                cx.notify();
            })
        })
        .detach();
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

    /// Setup is under way: the status line, and the setup screen on first launch.
    pub fn on_progress(&mut self, p: Progress, cx: &mut Context<Self>) {
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
        if self.setup.is_some() && self.agent_ready && !self.account.signed_out() {
            self.setup = None;
            Setup::finish();
            cx.notify();
        }
    }

    /// The sidebar's status line: a mark when it matters to every session, and the words.
    fn status_line(&self) -> (Option<AnyElement>, SharedString) {
        if self.offline_since.is_some() {
            let mark = new_session::glyph(new_session::Glyph::WifiOff, theme::text_muted());
            return (Some(mark.into_any_element()), "Offline · reconnects by itself".into());
        }
        if self.account.signed_out() {
            let dot = div().size(px(6.)).flex_shrink_0().rounded_full().bg(theme::accent());
            return (Some(dot.into_any_element()), "Signed out of Claude.".into());
        }
        (None, self.status.clone())
    }

    /// Retry the failed setup step: start Julia again, or the agent once Julia is up.
    pub fn retry_setup(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = &mut self.setup else { return };
        setup.clear_error();
        self.restart_agent(cx);
    }

    /// What About Endeavor shows in its update strip.
    pub fn updates(&self) -> about::Updates {
        let adapter = match agent::adapter_status() {
            Ok((version, false)) if self.agent_failed => about::Adapter::Available(version),
            Ok((version, false)) => about::Adapter::Installing(version),
            _ => about::Adapter::Current,
        };
        about::Updates { app: None, adapter }
    }

    /// About's Update: start the agent again, which installs the pinned adapter.
    pub fn update_adapter(&mut self, cx: &mut Context<Self>) {
        if !self.agent_failed {
            return;
        }
        self.agent_failed = false;
        if let Some(setup) = &mut self.setup {
            setup.clear_error();
        }
        self.restart_agent(cx);
    }

    fn restart_agent(&mut self, cx: &mut Context<Self>) {
        if self.bridge(&HostId::ThisMac).is_some() {
            // The failed agent thread dropped its command channel; start with a new one.
            let (tx, rx) = futures::channel::mpsc::unbounded();
            self.agent_tx = tx;
            self.start_agent(rx, cx);
        } else {
            self.ensure_runtime(&HostId::ThisMac, cx);
        }
        cx.notify();
    }

    pub fn start_agent(&mut self, commands: UnboundedReceiver<Command>, cx: &mut Context<Self>) {
        let mut events = agent::start(commands);
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |this, cx| this.on_event(event, cx)).is_err() {
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
    pub fn push_cells(&self, cx: &mut Context<Self>) {
        let url = self.webview.read(cx).raw().url().unwrap_or_default();
        let Some(id) = viewed_notebook_id(&url) else { return };
        let cells = self.connections.values().find_map(|c| c.cells.get(id).cloned()).unwrap_or_else(|| serde_json::json!([]));
        self.send_to_page(&serde_json::json!({ "type": "cells", "cells": cells }), cx);
    }

    /// After a session goes idle, say if it left edited cells unrun or still
    /// running. Each `/events` update then cuts the note back to what's still true.
    fn check_run_state(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(host) = self.sessions.iter().find(|s| s.key == key).map(|s| &s.place.host) else { return };
        // ponytail: warns about every open notebook on the host, not only the ones this session touched.
        let warnings = self.connection(host).map(|c| pluto::run_warnings(&c.notebooks)).unwrap_or_default();
        self.with_session(key, cx, |s| s.note_run_state(warnings));
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    /// A dialog button's Tab-stop handle, keyed by its static id and cached across
    /// renders (dialogs like `server_dialog.rs`'s are otherwise rebuilt from scratch).
    pub(crate) fn dialog_focus(&self, id: &'static str, cx: &App) -> FocusHandle {
        self.dialog_focus.borrow_mut().entry(id).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    /// A past sidebar row's Tab-stop handle, cached by session id across renders.
    fn past_row_focus(&self, id: &SessionId, cx: &App) -> FocusHandle {
        self.past_row_focus.borrow_mut().entry(id.clone()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

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
        let picks = self.menu_picks(&menu.target);
        let groups: Vec<u8> = picks.iter().map(MenuPick::group).collect();
        let notebook_menu = matches!(menu.target, MenuTarget::Notebook(_) | MenuTarget::Share(_));
        let items = picks.into_iter().enumerate().flat_map(|(i, pick)| {
            let danger = pick.danger();
            let new_group = i > 0 && groups[i - 1] != groups[i];
            let separator = new_group.then(|| div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::composer_edge()).into_any_element());
            let action = match &pick {
                MenuPick::Notebook(_, action) => Some(*action),
                MenuPick::Row(..) => None,
            };
            let section = action.and_then(|a| a.section()).map(|heading| {
                div().px(px(8.)).pt(px(4.)).pb(px(2.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(heading).into_any_element()
            });
            let checked = match action {
                Some(NotebookAction::LookEndeavor) => Some(self.settings.notebook_theme == NotebookTheme::Endeavor),
                Some(NotebookAction::LookClassic) => Some(self.settings.notebook_theme == NotebookTheme::Pluto),
                _ => None,
            };
            let color = if danger { theme::danger() } else { theme::text_muted() };
            let icon = match (checked, action.and_then(|a| a.glyph())) {
                (Some(true), _) => Some(div().w(px(12.)).text_size(theme::size_meta()).text_color(theme::accent_text()).child("✓").into_any_element()),
                (Some(false), _) => Some(div().w(px(12.)).into_any_element()),
                (None, Some(g)) => Some(glyph(g, color).into_any_element()),
                (None, None) => None,
            };
            let detail = action.and_then(|a| a.detail());
            let (label, shortcut) = (pick.label(), pick.shortcut().1);
            let item = div()
                .id(ElementId::NamedInteger("menu-item".into(), i as u64))
                .role(Role::MenuItem)
                .aria_label(label)
                .flex()
                .items_center()
                .gap(px(8.))
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
                .children(icon)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(label)
                        .children(detail.map(|d| div().text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(d))),
                )
                .child(div().text_size(theme::size_meta()).text_color(theme::text_faint()).child(shortcut))
                .into_any_element();
            separator.into_iter().chain(section).chain([item])
        });
        // The ⋮ menu starts with where the notebook is; Share ends with a note on how exports look.
        let head = match &menu.target {
            MenuTarget::Notebook(key) => self.sessions.iter().find(|s| s.key == *key).and_then(|s| {
                let path = s.notebook_path.clone()?;
                Some(
                    div()
                        .px(px(8.))
                        .pt(px(4.))
                        .pb(px(6.))
                        .mb(px(4.))
                        .border_b_1()
                        .border_color(theme::composer_edge())
                        .child(div().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_secondary()).child(new_session::tilde(Path::new(&path))))
                        .child(div().text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(self.host_label(&s.place.host)))
                        .into_any_element(),
                )
            }),
            _ => None,
        };
        let foot = matches!(menu.target, MenuTarget::Share(_)).then(|| {
            div()
                .mt(px(4.))
                .px(px(8.))
                .pt(px(6.))
                .pb(px(2.))
                .border_t_1()
                .border_color(theme::composer_edge())
                .text_size(theme::size_meta_small())
                .text_color(theme::text_faint())
                .child("Exports and recordings use Pluto's standard look.")
        });
        // Over the notebook, the web view (a native view on top) lets the menu show through.
        let hole = notebook_menu.then(|| {
            let webview = self.webview.read(cx);
            let (handle, under) = (webview.handle(), webview.bounds());
            canvas(move |bounds, _, _| overlay::set_hole(handle.raw(), overlay::Hole::Menu, Some(Bounds { origin: bounds.origin - under.origin, size: bounds.size })), |_, _, _, _| ()).absolute().size_full()
        });
        let body = div()
            .id("popup-menu")
            .role(Role::Menu)
            .track_focus(&menu.focus)
            .occlude()
            .w(px(if notebook_menu { 290. } else { 210. }))
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
            .children(head)
            .children(items)
            .children(foot);
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

    /// A folder's name, with its server's for a folder on one ("decay-fits · lab").
    fn folder_heading(&self, place: &Place) -> String {
        match place.host {
            HostId::ThisMac => folder_name(&place.path),
            _ => format!("{} · {}", folder_name(&place.path), self.hosts.name(&place.host)),
        }
    }

    /// The sidebar's folders: recent ones, then any other folder with an open session.
    fn sidebar_folders(&self) -> Vec<&Place> {
        let mut folders: Vec<&Place> = self.recent.iter().collect();
        for s in &self.sessions {
            if !folders.contains(&&s.place) {
                folders.push(&s.place);
            }
        }
        folders
    }

    /// A folder's past sessions the sidebar lists, newest first: ours, not
    /// already open, and not archived unless archived ones show.
    fn past_rows(&self, folder: &Place) -> Vec<&SessionInfo> {
        let is_open = |info: &SessionInfo| self.sessions.iter().any(|s| s.id.as_ref() == Some(&info.session_id));
        let shown = |info: &SessionInfo| self.ours.contains_key(&info.session_id.to_string()) && (self.settings.show_archived || !self.archived.contains(&info.session_id.to_string()));
        self.past.get(folder).into_iter().flatten().filter(|info| !is_open(info) && shown(info)).collect()
    }

    /// The sidebar's row above the status line while This Mac's Julia is down.
    fn restart_row(&self) -> Option<&'static str> {
        match self.status(&HostId::ThisMac) {
            Some(connection::Status::Replaced) => Some("↻ Reconnect to Julia"),
            Some(connection::Status::Died(_) | connection::Status::Failed(_)) => Some("↻ Restart Julia"),
            _ => None,
        }
    }

    /// The status at an open session's row end.
    fn row_mark(&self, s: &Session) -> Option<RowMark> {
        let archived = s.id.as_ref().is_some_and(|id| self.archived.contains(&id.to_string()));
        if s.needs_approval() {
            Some(RowMark::NeedsApproval)
        } else if s.outbox.busy {
            Some(RowMark::Working)
        } else if archived {
            Some(RowMark::Archived)
        } else {
            None
        }
    }

    fn render_session_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let groups: Vec<_> = self
            .sidebar_folders()
            .into_iter()
            .map(|folder| {
                let open: Vec<_> = self
                    .sessions
                    .iter()
                    .filter(|s| &s.place == folder)
                    .map(|s| {
                        let key = s.key;
                        let active = self.active == Some(key) && !self.settings_open;
                        let group: SharedString = format!("session-{key}").into();
                        let title_text = s.title.clone();
                        let title = self.row_label(&Row::Open(key), title_text.clone());
                        // Status at the row's end: a ring waits for you, the working orbit is working.
                        let mark = self.row_mark(s).map(|mark| match mark {
                            RowMark::NeedsApproval => div().size(px(6.)).rounded_full().border_1().border_color(theme::accent()).into_any_element(),
                            RowMark::Working => session::orbit(ElementId::NamedInteger("row-orbit".into(), key), 12., cx),
                            RowMark::Archived => glyph(Glyph::Archive, theme::text_section()).into_any_element(),
                        });
                        self.session_row(Row::Open(key), group.clone(), active, cx)
                            .aria_label(title_text)
                            .track_focus(&s.focus_handle(cx))
                            .tab_stop(true)
                            .focus_visible(|st| st.border_2().border_color(theme::focus_ring()))
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
                // The newest few unless expanded.
                let is_archived = |info: &SessionInfo| self.archived.contains(&info.session_id.to_string());
                let all = self.past_rows(folder);
                let expanded = self.expanded.contains(folder);
                let limit = if expanded { all.len() } else { PAST_SHOWN };
                let past: Vec<_> = all
                    .iter()
                    .take(limit)
                    .enumerate()
                    .map(|(i, info)| {
                        let row = Row::Past(info.session_id.clone(), folder.clone());
                        let title_text = self.row_title(&row).unwrap_or_default();
                        let title = self.row_label(&row, title_text.clone());
                        let group: SharedString = format!("past-{:?}-{}-{i}", folder.host, folder.path.display()).into();
                        let open = (*info).clone();
                        let place = folder.clone();
                        let archived = is_archived(info);
                        let focus = self.past_row_focus(&info.session_id, cx);
                        self.session_row(row.clone(), group.clone(), false, cx)
                            .aria_label(title_text)
                            .track_focus(&focus)
                            .tab_stop(true)
                            .focus_visible(|d| d.border_2().border_color(theme::focus_ring()))
                            .when(archived, |d| d.text_color(theme::text_section()))
                            .child(title)
                            .when(archived, |d| d.child(glyph(Glyph::Archive, theme::text_section())))
                            .child(self.row_more(row.clone(), group, false, cx))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.renaming.as_ref().is_some_and(|(renaming, _)| *renaming == row) {
                                    this.open_past(open.clone(), place.clone(), cx);
                                }
                            }))
                    })
                    .collect();
                let more = (all.len() > PAST_SHOWN).then(|| {
                    let label = if expanded { "Show fewer".to_string() } else { format!("Show {} more", all.len() - PAST_SHOWN) };
                    let folder = folder.clone();
                    sidebar_row(ElementId::Name(format!("more-{:?}-{}", folder.host, folder.path.display()).into()), false)
                        .aria_label(label.clone())
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
                    .child(div().mt(px(18.)).px(px(10.)).pb_1().text_size(theme::size_meta_small()).text_color(theme::text_section()).child(self.folder_heading(folder)))
                    .children(open)
                    .children(past)
                    .children(more)
            })
            .collect();

        div()
            .w(px(self.settings.layout.sidebar_width))
            .min_w(px(SIDEBAR_RANGE.0))
            .flex_shrink(1.)
            .overflow_hidden()
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
                    .aria_label("New session")
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
            .map(|d| {
                let Some(label) = self.restart_row() else { return d };
                d.child(
                    sidebar_row("restart".into(), false)
                        .aria_label(label)
                        .text_color(theme::accent_text())
                        .child(label)
                        .on_click(cx.listener(|this, _, _, cx| this.ensure_runtime(&HostId::ThisMac, cx))),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .pt(px(6.))
                    .px_1()
                    .child({
                        let (mark, status) = self.status_line();
                        div()
                            .id("status")
                            .flex_1()
                            .min_w_0()
                            .pl(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .text_size(theme::size_meta())
                            .text_color(if mark.is_some() { theme::text_new() } else { theme::text_section() })
                            .children(mark)
                            .child(div().min_w_0().truncate().child(status.clone()))
                            // Wrapped narrow enough to stay clear of the notebook, which covers anything drawn over it.
                            .tooltip(move |window, cx| {
                                let status = status.clone();
                                gpui_component::tooltip::Tooltip::element(move |_, _| div().max_w(px(260.)).child(status.clone())).build(window, cx)
                            })
                    })
                    .child(
                        div()
                            .id("settings")
                            .role(Role::Button)
                            .aria_label("Settings")
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
            .id("settings")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
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
                "New sessions in Manual start as if you'd chosen \"Always this session\". A reopened session keeps the mode it was in.".into(),
            ))
            .child(heading("Where notebooks run"))
            .child(self.render_hosts(cx))
            .child(heading("Stop idle notebooks after"))
            .child(
                div().flex().flex_wrap().gap_x_4().gap_y_2().children(IdleStop::ALL.map(|(value, label)| {
                    Radio::new(label).text_size(theme::size_body()).checked(s.idle_stop == value).label(label).on_click(cx.listener(move |this, _, _, cx| {
                        this.update_settings(cx, |s| s.idle_stop = value);
                        let hosts: Vec<HostId> = this.connections.keys().cloned().collect();
                        for host in hosts {
                            this.send_idle_limit(&host, cx);
                        }
                    }))
                })),
            )
            .child(note(
                "A notebook nobody has used for this long (no edits, runs, or Claude working in it) stops, even with Endeavor open. Start brings it back.".into(),
            ))
            .child(check_row("keep-running", s.keep_running, "Keep notebooks running after Endeavor quits").on_click(
                cx.listener(|this, _, _, cx| this.update_settings(cx, |s| s.keep_running = !s.keep_running)),
            ))
            .child(note(
                "Julia and its open notebooks carry on while Endeavor is closed, and Endeavor reconnects to them when it opens. They still stop when idle for the time above.".into(),
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
            .child(heading("Notebook look"))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .children([(NotebookTheme::Endeavor, "Endeavor"), (NotebookTheme::Pluto, "Pluto classic")].map(|(value, label)| {
                        Radio::new(label).text_size(theme::size_body()).checked(s.notebook_theme == value).label(label).on_click(cx.listener(move |this, _, _, cx| {
                            this.update_settings(cx, |s| s.notebook_theme = value);
                            this.apply_look(cx);
                        }))
                    })),
            )
            .child(note("Endeavor puts Pluto's controls in the pane's header and a drawer, matched to the app in dark mode (light mode uses Pluto's light colours). Pluto classic is Pluto's own page. The notebook's ⋮ menu switches too.".into()))
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
            .child(note("Takes effect the next time Julia starts (Restart Julia).".into()))
            .when(self.status(&HostId::ThisMac) == Some(&connection::Status::Ready), |d| {
                d.child(
                    div()
                        .id("restart-julia")
                        .self_start()
                        .px_2()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(theme::bg_raised())
                        .child("Restart Julia")
                        .on_click(cx.listener(|this, _, _, cx| this.restart_local(cx))),
                )
            })
            .child(note("Stops Julia and every open notebook, then starts them again. Notebook files are already saved.".into()))
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
            .child(
                div()
                    .id("repair-runtime")
                    .self_start()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(theme::bg_raised())
                    .child("Repair runtime")
                    .on_click(cx.listener(|this, _, _, cx| this.repair_local(cx))),
            )
            .child(note(connection::REPAIR_NOTE.into()))
    }

    /// The notebook's ⋮ button in its header, and its menu while open.
    fn notebook_more(&self, key: u64, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let menu = self.menu.as_ref().filter(|menu| menu.target == MenuTarget::Notebook(key));
        div()
            .id("notebook-more")
            .role(Role::Button)
            .aria_label("More")
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
            .when(menu.is_none(), |d| d.tooltip(notebook_pane::tooltip("More", &self.webview)))
            .child("⋮")
            .children(menu.map(|menu| self.render_menu(menu, cx)))
    }

    /// A session's chat: the transcript, then what waits above the composer, then the composer.
    /// `notebook_open`: its notebook shows in the pane (Point needs it).
    fn render_chat(&self, session: &Session, notebook_open: bool, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
            .children(session::render_activity(session, self.offline_since, cx))
            .child(
                div()
                    .px_4()
                    .pb(px(11.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(self.render_commands(session, cx))
                    .children(session::render_pinned_plan(session, cx))
                    .children(self.render_offline_line(Some(session), cx))
                    .children(self.render_sign_in_card(cx))
                    .children(session::render_approval(session, cx))
                    .child(session::render_queue(self, session, cx))
                    .child(self.render_composer(Some(session), notebook_open, window, cx)),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active.and_then(|key| self.sessions.iter().position(|s| s.key == key));
        let stand_in = active.and_then(|ix| self.notebook_page(&self.sessions[ix], cx));
        self.sync_page_context(cx);
        // The web view is a native view over the window: hidden behind the setup
        // screen, and wherever the notebook pane is drawn natively.
        let modal = self.server_dialog.is_some() || !self.asks.is_empty() || self.login_node_warning.is_some();
        let show_webview = self.setup.is_none() && active.is_some() && stand_in.is_none() && !modal;
        if self.webview.read(cx).visible() != show_webview {
            self.webview.update(cx, |w, _| if show_webview { w.show() } else { w.hide() });
        }
        let menu_over_notebook = self.menu.as_ref().is_some_and(|m| matches!(m.target, MenuTarget::Notebook(_) | MenuTarget::Share(_)));
        let point_tip = self.point_tip_shows(show_webview && active.is_some_and(|ix| self.sessions[ix].notebook_path.is_some()));
        let webview = self.webview.read(cx).raw();
        for (hole, open) in [(overlay::Hole::Menu, menu_over_notebook), (overlay::Hole::Tip, point_tip), (overlay::Hole::Tooltip, notebook_pane::tooltip_over_notebook())] {
            if !open {
                overlay::set_hole(webview, hole, None);
            }
        }
        overlay::set_dismiss_on_click(webview, self.dismissible_open());
        if let Some(setup) = &self.setup {
            let below = match self.render_sign_in_panel(cx) {
                _ if self.offline_since.is_some() => splash::Below::Card(self.render_offline_setup(setup, cx)),
                Some((panel, bar, tucked)) => splash::Below::Panel { line: "Sign in to finish setting up", bar: bar.then_some(0.78), panel, tucked },
                None => splash::Below::Progress,
            };
            let retry = cx.listener(|this, _, _, cx| this.retry_setup(cx));
            return div().size_full().bg(theme::bg_page()).text_color(theme::text_primary()).text_size(theme::size_body()).child(splash::render(setup, below, retry, cx)).into_any_element();
        }
        let working = active.is_some_and(|ix| self.sessions[ix].outbox.busy);
        let placeholder = match active {
            None => "What do you want to work on?",
            Some(_) if working => "Queue a message, or ⌘⏎ to steer",
            Some(_) if self.offline_since.is_some() => "Write a message. It sends when you're back online.",
            Some(_) => "Type / for commands",
        };
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            self.input.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        let chat = match active {
            _ if self.settings_open => self.render_settings(cx).into_any_element(),
            Some(ix) => self.render_chat(&self.sessions[ix], stand_in.is_none(), window, cx).into_any_element(),
            None => self.render_new_session(window, cx).into_any_element(),
        };
        // Chat header: the session and its folder; the notebook header: its file.
        let (title, folder) = match active {
            _ if self.settings_open => ("Settings".into(), None),
            Some(ix) => (self.sessions[ix].title.clone(), Some(self.folder_heading(&self.sessions[ix].place))),
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
            Some(ix) => d.child(self.notebook_header(ix, stand_in.is_none(), cx)),
        });
        let notebook = match (active, stand_in) {
            (None, _) => self.render_draft_pane(cx),
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
            .on_action(cx.listener(Self::add_files))
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
                    .min_w(px(CHAT_MIN))
                    .flex_shrink(1.)
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
                    .min_w(px(NOTEBOOK_MIN))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(notebook_header)
                    .children(active.and_then(|ix| self.render_pane_warning(&self.sessions[ix], cx)))
                    .child(div().flex_1().min_h_0().child(notebook)),
            )
            // Deferred so they paint, and take clicks, above everything else.
            .children(self.render_server_dialog(window, cx).map(|d| deferred(d).with_priority(3)))
            .children(self.render_askpass(cx).map(|d| deferred(d).with_priority(5)))
            .children(self.render_login_node_warning(cx).map(|d| deferred(d).with_priority(4)))
            // A click outside a menu or popover closes it, like a native menu;
            // set_dismiss_on_click makes the web view forward its own clicks
            // here too, while one is open (overlay.rs).
            .when(self.dismissible_open(), |d| {
                d.child(
                    div()
                        .id("dismiss-backdrop")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_dismissible(window, cx)))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, cx| this.close_dismissible(window, cx))),
                )
            })
            .into_any_element()
    }
}

fn main() {
    // `claude auth login` opens its page through this app (signin::Login).
    signin::browser_shim();
    // Claude Code runs the plugin's execution-gate hook as `endeavor hook-pretool`.
    if std::env::args().nth(1).as_deref() == Some("hook-pretool") {
        gate::run_pretool_hook();
    }
    // This Mac's runtime helper (runtime::connect), and ssh's askpass (remote::Askpass).
    if std::env::args().nth(1).as_deref() == Some(runtime::HELPER_FLAG) {
        endeavor_remote::run_as(&[runtime::HELPER_FLAG], std::env::args().skip(2).collect());
    }
    if std::env::var_os(wire::askpass::SOCKET_ENV).is_some() {
        endeavor_remote::run(std::env::args().skip(1).collect());
    }
    logs::start();
    gpui_platform::application().run(|cx: &mut App| {
        platform::init(cx);
        gpui_component::init(cx);
        theme::load_fonts(cx);
        // Theme::change applies these before building the component defaults from them.
        let ui = Theme::global_mut(cx);
        let mut colors = ui.dark_theme.colors.clone();
        // What agent replies' markdown is drawn with: hairlines, link colour,
        // code block and table header backgrounds.
        colors.border = Some(theme::hex(theme::border()));
        colors.link = Some(theme::hex(theme::accent_text()));
        colors.muted = Some(theme::hex(theme::bg_card()));
        colors.table_head = Some(theme::hex(theme::bg_card()));
        colors.table_head_foreground = Some(theme::hex(theme::text_muted()));
        // The library's own focus ring (Input, Textarea, and anything else built
        // from it), so a tabbed-to text box matches our own focus_visible() rings.
        colors.ring = Some(theme::hex(theme::focus_ring()));
        ui.dark_theme = std::rc::Rc::new(ThemeConfig {
            font_family: Some(theme::SANS.into()),
            mono_font_family: Some(theme::MONO.into()),
            mono_font_size: Some(f32::from(theme::size_code())),
            colors,
            ..(*ui.dark_theme).clone()
        });
        cx.set_reduce_motion(platform::reduces_motion());
        // Input consumes Escape only when it has something to dismiss; otherwise it reaches us.
        cx.bind_keys([
            KeyBinding::new("escape", Interrupt, None),
            KeyBinding::new("secondary-shift-k", ToggleAnnotation, None),
            // Registered after gpui-component's, so it beats the text box's own ⇧⇥ (outdent).
            KeyBinding::new("shift-tab", CycleMode, Some("Input")),
            KeyBinding::new("shift-tab", CycleMode, None),
            KeyBinding::new("secondary-b", ToggleSidebar, Some("Input")),
            KeyBinding::new("secondary-b", ToggleSidebar, None),
            KeyBinding::new("secondary-,", OpenSettings, None),
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-m", Minimize, None),
            KeyBinding::new("secondary-=", ZoomIn, None),
            KeyBinding::new("secondary--", ZoomOut, None),
            KeyBinding::new("secondary-0", ZoomReset, None),
        ]);
        cx.bind_keys(composer::key_bindings());
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenHelp, cx| cx.open_url(about::HELP));
        cx.on_action(|_: &ReportIssue, cx| cx.open_url(about::REPORT_ISSUE));
        cx.on_action(|_: &BringAllToFront, cx| platform::bring_all_to_front(cx));
        // Deferred: from a focused view the action runs inside the active window,
        // which can't be updated until it returns.
        cx.on_action(|_: &Minimize, cx| {
            cx.defer(|cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, _| window.minimize_window());
                }
            })
        });
        cx.on_action(|_: &ZoomWindow, cx| {
            cx.defer(|cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, _| window.zoom_window());
                }
            })
        });
        // Edit's items send the native cut:/copy:/paste:/selectAll: selectors, which
        // the notebook's web view needs for the clipboard; in our own text boxes
        // they become the input's actions.
        use gpui_component::input::{Copy, Cut, Paste, SelectAll};
        let menu = |name: &str, items| Menu { name: name.to_string().into(), items, disabled: false };
        cx.set_menus(vec![
            menu(
                "Endeavor",
                vec![
                    MenuItem::action("About Endeavor", ShowAbout),
                    MenuItem::separator(),
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::separator(),
                    MenuItem::action("Quit Endeavor", Quit),
                ],
            ),
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
            // GPUI makes the menu named "Window" the app's windows menu, so macOS adds the window list.
            menu(
                "Window",
                vec![
                    MenuItem::action("Minimize", Minimize),
                    MenuItem::action("Zoom", ZoomWindow),
                    MenuItem::separator(),
                    MenuItem::action("Bring All to Front", BringAllToFront),
                ],
            ),
            menu("Help", vec![MenuItem::action("Endeavor Help", OpenHelp), MenuItem::action("Report an Issue…", ReportIssue)]),
        ]);
        let bounds = Bounds::centered(None, size(px(1560.), px(900.)), cx);
        let main_window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(WINDOW_MIN),
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
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ShowAbout, cx| about::open_about(ws.clone(), cx));
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
        .unwrap()
        .window_id();
        // About and Licences can close without quitting.
        cx.on_window_closed(move |cx, closed| {
            if closed == main_window {
                cx.quit();
            }
        })
        .detach();
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
        let labels = |row: &Row, archived, local| RowAction::for_row(row, archived, local).into_iter().map(RowAction::label).collect::<Vec<_>>();
        let past = Row::Past("s1".to_string().into(), crate::hosts::Place::local("/tmp"));
        assert_eq!(labels(&Row::Open(1), Some(false), true), ["Rename", "Reveal folder in Finder", "Archive", "Close", "Delete…"]);
        assert_eq!(labels(&Row::Open(1), None, true), ["Rename", "Reveal folder in Finder", "Close", "Delete…"]);
        assert_eq!(labels(&past, Some(true), true), ["Rename", "Reveal folder in Finder", "Unarchive", "Delete…"]);
        assert_eq!(labels(&Row::Open(1), Some(false), false), ["Rename", "Archive", "Close", "Delete…"]);
    }

    #[test]
    fn notebook_menu_groups() {
        assert!(MenuPick::Notebook(1, NotebookAction::Stop).danger());
        assert!(!MenuPick::Notebook(1, NotebookAction::Reveal).danger());
        assert_ne!(MenuPick::Notebook(1, NotebookAction::Rename).group(), MenuPick::Notebook(1, NotebookAction::LookEndeavor).group());
        assert_eq!(MenuPick::Row(Row::Open(1), RowAction::Delete).group(), 1);
    }
}
