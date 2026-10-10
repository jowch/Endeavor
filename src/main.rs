//! Endeavor: app-owned Julia running Pluto + EndeavorRuntime, the live Pluto frontend in a
//! child webview, and ACP agent sessions (a session bar, one chat pane) wired to the
//! same Pluto session over MCP.

// A GUI program on Windows: no console window of its own, and the console
// programs it starts are given none either (`client::no_window`).
#![cfg_attr(windows, windows_subsystem = "windows")]

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
mod antigravity;
mod annotate;
mod askpass_watch;
mod approval;
mod attach;
mod celldiff;
mod agent_process;
mod agent_job;
mod codex;
mod composer;
mod confirm;
mod context;
mod connection;
mod crash;
#[cfg(debug_assertions)]
mod debug_state;
mod details;
mod failure;
mod find_bar;
mod host_list;
mod hosts;
mod install;
mod logs;
mod new_session;
mod network;
mod notebook_pane;
mod notice;
mod offline;
mod older_runtime;
mod opening;
mod orbit;
mod outbox;
#[cfg(target_os = "macos")]
mod overlay;
mod platform;
mod permits;
mod notify;
mod pluto;
mod queue;
mod quotes;
mod records;
mod remote;
mod row_marks;
mod resources;
mod runs;
mod runtime;
mod server_dialog;
mod menu;
mod motion;
mod session;
mod settings;
mod sidebar;
mod sidebar_filter;
mod settings_panel;
mod signin;
mod slash;
#[cfg(target_os = "macos")]
mod snapshot;
#[cfg(target_os = "macos")]
mod webcontent;
mod splash;
mod theme;
mod tips;
mod transcript;
mod transcript_copy;
mod trouble;
mod turtle;
mod when;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::{overlay, webcontent};
#[cfg(not(target_os = "macos"))]
use platform::{dialogs, snapshot};
#[cfg(windows)]
use platform::{overlay, webcontent};

use agent::{AgentEvent, Command};
use agent_client_protocol::schema::v1::{ContentBlock, PermissionOptionKind, SessionId, SessionInfo, TextContent};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{InputEvent, InputState, TextareaState};

use gpui_component::{Root, Theme, ThemeConfig, ThemeMode};
use gpui_wry::WebView;
use hosts::{HostId, Place};
use outbox::Queued;
use new_session::{Draft, NotebookChoice};
use menu::{MenuTarget, PopupMenu};
use session::{Effect, Session, Stopped, folder_name};
use sidebar::{FilterMenu, Row};
use settings::Settings;
use splash::{Progress, Setup};
use wire::backend::Backend;
use crate::theme::FocusRing as _;

/// Notebook id from a notebook page's URL. Only the id is used: the URL may
/// also carry the runtime's token, which must never reach the agent.
fn viewed_notebook_id(url: &str) -> Option<&str> {
    Backend::Pluto.notebook_id(url).filter(|id| annotate::is_uuid(id))
}

actions!(
    endeavor,
    [
        Interrupt,
        ToggleAnnotation,
        ReplyToSelection,
        CycleMode,
        ToggleSidebar,
        NewSession,
        OpenSettings,
        FindSetting,
        FindNext,
        FindPrevious,
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
    app_file(name).map(|f| load_json_at(&f)).unwrap_or_default()
}

fn save_json(name: &str, value: &impl serde::Serialize) {
    if let Some(file) = app_file(name) {
        save_json_at(&file, value);
    }
}

/// `load_json`'s read, by path (split out so the round trip is unit-testable
/// without the real Application Support folder). A file that's there but
/// doesn't parse reads as the default, and is first set aside (see `set_aside`)
/// so the next save can't overwrite what's left of it.
fn load_json_at<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    // Bytes, not a string: a file cut inside a multi-byte character must count
    // as one that doesn't parse, not as no file.
    let Ok(bytes) = std::fs::read(path) else { return T::default() };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        set_aside(path, &e);
        T::default()
    })
}

/// `save_json`'s write, by path.
fn save_json_at(path: &Path, value: &impl serde::Serialize) {
    // ponytail: best effort. A failed save leaves the last good file in place
    // (`write_atomic`), so it costs only the change since then.
    if let Ok(json) = serde_json::to_string_pretty(value)
        && let Err(e) = write_atomic(path, json.as_bytes())
    {
        eprintln!("Couldn't save {}: {e}", path.display());
    }
}

/// Replace `path` with `bytes` so that a crash or a full disk leaves either
/// the old file or the new one, never half of one: write a file beside it,
/// flush it to disk, then rename it over the old one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    // One write at a time in this whole process (not per file, on purpose:
    // these files are small), so two saves of one file can't share its
    // `.partial`. ponytail: `sync_all` runs on the caller's thread, the UI
    // thread for most saves; fine for user actions and turn ends, not for a
    // hot path.
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let written = std::fs::File::create(&partial).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    let renamed = written.and_then(|_| rename_over(&partial, path));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    renamed
}

/// `std::fs::rename`, which replaces an existing file on every platform. On
/// Windows it fails while another program (a virus scanner, the search
/// indexer) has either file open without sharing it, most often the scanner
/// checking the `.partial` just closed, so try a few more times.
fn rename_over(from: &Path, to: &Path) -> std::io::Result<()> {
    // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
    const HELD: [i32; 3] = [5, 32, 33];
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if cfg!(windows) && e.raw_os_error().is_some_and(|code| HELD.contains(&code)) && tries < 5 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20 * tries));
            }
            done => return done,
        }
    }
}

/// Move a file that doesn't parse to `<name>.bad` (replacing an older one),
/// so a person can still look at it, and say so on stderr.
pub fn set_aside(path: &Path, why: &dyn std::fmt::Display) {
    let mut bad = path.as_os_str().to_owned();
    bad.push(".bad");
    match std::fs::rename(path, &bad) {
        Ok(()) => eprintln!("Couldn't read {} ({why}); kept it as {}.", path.display(), Path::new(&bad).display()),
        Err(e) => eprintln!("Couldn't read {} ({why}) or set it aside ({e}).", path.display()),
    }
}

/// The range a divider can drag the panes to (defaults: settings::Layout).
const SIDEBAR_RANGE: (f32, f32) = (180., 400.);
const CHAT_MIN: f32 = 320.;
const NOTEBOOK_MIN: f32 = 360.;
/// The window can't shrink below every column at its minimum, plus the two dividers.
const WINDOW_MIN: Size<Pixels> = Size { width: px(SIDEBAR_RANGE.0 + CHAT_MIN + NOTEBOOK_MIN + 2.), height: px(600.) };

/// The chat column's content (the transcript and the whole composer area,
/// including its cards and the chips row) sits in one centred column like
/// Claude desktop's: at most this wide.
const CHAT_CONTENT_MAX: f32 = 760.;
/// The column's side margin once the chat pane is wide enough to show it in
/// full (`CHAT_CONTENT_MAX` plus two of these).
const CHAT_MARGIN_MAX: f32 = 50.;
/// The chat pane's width at and above which the column sits at its max width
/// with `CHAT_MARGIN_MAX` on each side.
const CHAT_MARGIN_BREAK: f32 = CHAT_CONTENT_MAX + 2. * CHAT_MARGIN_MAX;

/// The chat column's side margin for a pane this wide. At `CHAT_MARGIN_BREAK`
/// (860 px) and above, the column centres at `CHAT_CONTENT_MAX` with
/// `CHAT_MARGIN_MAX` (50 px) on each side. Below that, the margin holds at
/// 50 px -- the column fills the pane, less the margins -- until the pane
/// gets close to its minimum (`CHAT_MIN`, 320 px), where a flat 50 px would
/// leave only 220 px for the content. There the margin shrinks in proportion
/// to the pane's width (the same 50:860 ratio as the wide case), down to a
/// floor of 16 px -- today's `px_4` padding -- so the column never loses more
/// width to its margins than it did before this column existed.
fn chat_margin(chat_width: f32) -> f32 {
    if chat_width >= CHAT_MARGIN_BREAK {
        (chat_width - CHAT_CONTENT_MAX) / 2.
    } else {
        (chat_width * CHAT_MARGIN_MAX / CHAT_MARGIN_BREAK).clamp(16., CHAT_MARGIN_MAX)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Divider {
    Sidebar,
    Chat,
}

/// The ⌘B button: a small drawn sidebar glyph. It sits in a header, so it stops
/// the mouse-down that would otherwise start a window move.
fn sidebar_toggle(this: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("sidebar-toggle")
        .role(Role::Button)
        .aria_label("Toggle sidebar")
        .p(px(4.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::row_active()))
        .border_2()
        .border_color(gpui::transparent_black())
        .track_focus(&this.dialog_focus("sidebar-toggle", cx))
        .tab_stop(true)
        .focus_ring_on(if this.settings.layout.sidebar_open { theme::bg_sidebar() } else { theme::bg_page() })
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
/// Room the traffic lights take at the start of a header. On Linux the window
/// manager's title bar has the window's buttons, and on Windows its own title bar.
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

/// Recently used working folders on every host, most recent first, kept across
/// launches. Only This Mac's can be checked for still being there.
fn load_recent() -> Vec<Place> {
    load_json::<Vec<Place>>("recent.json").into_iter().filter(|p| p.here().is_none_or(Path::is_dir)).collect()
}

fn load_records() -> records::Records {
    records::Records::parse(&app_file("sessions.json").and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default())
}

/// Where each agent's last config options are kept.
fn agent_options_file(agent: agent::Agent) -> &'static str {
    match agent {
        agent::Agent::Claude => "agent-options.json",
        agent::Agent::Codex => "codex-options.json",
        agent::Agent::Antigravity => "antigravity-options.json",
    }
}

fn load_agent_options() -> HashMap<agent::Agent, Vec<agent_client_protocol::schema::v1::SessionConfigOption>> {
    agent::Agent::ALL.into_iter().map(|agent| (agent, load_json(agent_options_file(agent)))).collect()
}

fn save_agent_options(options: &HashMap<agent::Agent, Vec<agent_client_protocol::schema::v1::SessionConfigOption>>) {
    for (agent, options) in options {
        save_json(agent_options_file(*agent), options);
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
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
    /// The folder browser picking a session's notebook on a server (Locate file…, Open the notebook only).
    locate: Option<new_session::Browser>,
    /// Working folders on every host, most recent first (persisted).
    recent: Vec<Place>,
    /// Every session's agent, place, title and last activity (persisted): the
    /// sidebar's past sessions, kept up to date by the agent's listings.
    records: records::Records,
    /// This Mac's Julia has been ready since launch.
    this_mac_was_ready: bool,
    /// Session names the user gave, by session id (persisted).
    titles: HashMap<String, String>,
    /// Ids of archived sessions (persisted): hidden from the sidebar's Active view.
    archived: HashSet<String>,
    /// The sidebar's filter menu (Status, Where, Group by, Sort by…), while open.
    filter_menu: Option<FilterMenu>,
    /// The sidebar's inline search field, in place of the "Sessions" heading, while open.
    sidebar_search: Option<Entity<InputState>>,
    /// Each session's notebook file, by session id (persisted), for reopening it
    /// with the session: one the app opened isn't in the agent's history.
    session_notebooks: HashMap<String, Place>,
    /// A "notebook moved" note waiting to reach the agent, by session id
    /// (persisted): set when the user locates or moves a session's notebook
    /// file, and cleared once it's actually sent, so it survives a quit in between.
    pending_moved: HashMap<String, String>,
    renaming: Option<sidebar::Rename>,
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
    /// Reply on text selected in one of Claude's replies.
    reply: Option<quotes::Reply>,
    /// Words left in Reply's prompt by Esc or a click elsewhere, per selection.
    reply_drafts: quotes::Drafts,
    /// Each agent's config options (model, effort) as its last session offered
    /// them (persisted), so the new-session screen can offer them too.
    agent_options: HashMap<agent::Agent, Vec<agent_client_protocol::schema::v1::SessionConfigOption>>,
    /// The Settings panel, while open.
    settings_panel: Option<settings_panel::Panel>,
    /// Where Settings was last left, to open there again.
    settings_page: settings_panel::Page,
    /// What Settings found out: the account, the chosen Julia.
    settings_checks: settings_panel::Checks,
    /// First launch: the setup screen covers the window until setup finishes.
    setup: Option<Setup>,
    /// Claude Code's sign-in: checked when the agent starts and when the window
    /// comes back to the front, and lost when a turn fails for want of it.
    account: signin::Account,
    /// When the sign-in was last checked on coming to the front.
    sign_in_checked: Option<std::time::Instant>,
    /// Since when the network has been unreachable (None: online).
    offline_since: Option<std::time::Instant>,
    /// Try now is looking at the network.
    probing: bool,
    /// Each agent's connection and process.
    links: agent_process::Links,
    /// Codex's sign-in, once Codex has been started.
    codex_account: agent::Account,
    /// Antigravity's sign-in, kept the same way as Codex's.
    antigravity_account: agent::Account,
    /// App-level status (Julia, agent connection), shown under the session bar.
    status: SharedString,
    annotating: bool,
    /// Pictures the page asked for (`Shoot`), by id, until its quote takes them.
    shots: HashMap<u32, Arc<Vec<u8>>>,
    /// "Two ways to add your file" shows over the composer (a "Uses your file"
    /// example was clicked and the tip isn't done yet).
    file_tip: bool,
    /// Each host's connection and runtime.
    connections: HashMap<HostId, connection::Connection>,
    /// Each host's loopback ports for the webview and the agent, relayed to its
    /// runtime of the moment; kept for the whole launch.
    listeners: HashMap<HostId, Arc<runtime::Listener>>,
    /// Each server's line to its helper and runtime (`client::Session`), while connected or connecting.
    lines: HashMap<HostId, connection::Line>,
    /// The composer's placeholder as last set (it changes while Claude works).
    placeholder: SharedString,
    /// Each agent's usage limit, while reached: its sessions' messages wait until it resets.
    usage_limits: offline::UsageLimits,
    /// Notebooks whose Julia stopped by itself, and the runs after Restart Julia.
    crashes: crash::Crashes,
    /// A one-off failure's notice, under the control that was used.
    notice: Option<notice::Notice>,
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
    /// Endeavor's window is the active one (prompts arriving otherwise notify).
    window_active: bool,
    /// When the last "Claude is waiting for you" notification went out.
    last_ask_notice: Option<std::time::Instant>,
    /// The notebook being renamed in its header, and its name box.
    notebook_rename: Option<(u64, Entity<InputState>)>,
    /// The session whose job resources are open from the Julia-not-running page's gear.
    pane_resources: Option<u64>,
    pane_partition_menu: bool,
    /// A state dump waiting for the page's part (debug_state.rs).
    #[cfg(debug_assertions)]
    page_debug: Option<futures::channel::oneshot::Sender<serde_json::Value>>,
    /// Tab-stop handles for controls that are rebuilt fresh each render (dialog
    /// buttons in server_dialog.rs, new-session chips, the sidebar's static
    /// rows, resume rows…), keyed by an id unique to the control.
    dialog_focus: std::cell::RefCell<HashMap<String, FocusHandle>>,
    /// Past (not open) sidebar rows' Tab-stop handles, keyed by session id: unlike
    /// an open session, a past row has no `Session` to hold its own handle.
    past_row_focus: std::cell::RefCell<HashMap<SessionId, FocusHandle>>,
    /// Has the keyboard when no control does: at launch, and after the focused
    /// control leaves the screen. Keys dispatch from the focused element; with
    /// none they start outside Root, whose Tab and ⇧⇥ bindings then never match.
    /// It's tracked on an empty child because a tracked element takes focus when
    /// clicked, which would pull the keyboard out of the composer.
    keyboard_home: FocusHandle,
    /// Tracked on the sidebar's outer element, so ⌘F can tell whether it
    /// should open the sidebar search (focus is in the sidebar) or, when
    /// Settings is open instead, its own search.
    sidebar_focus: FocusHandle,
    /// The open confirm dialog (Stop a host, Cancel a job, Repair Julia, Sign
    /// out, Delete session), if any.
    confirm: Option<confirm::Confirm>,
    /// Find in the notebook, while its bar is open.
    find: Option<find_bar::FindBar>,
    /// A queued message being edited in the composer.
    queue_edit: Option<queue::QueueEdit>,
    /// Watches for the pane saying "Opening" for a notebook that's open (notebook_pane.rs).
    opening: notebook_pane::OpeningWatch,
    /// ENDEAVOR_TEST_STUCK_OPENING's file was there at the last check.
    #[cfg(debug_assertions)]
    test_blanked: bool,
    /// The session the last frame showed (None: the new-session screen); None
    /// before the first frame. Showing another one draws it without motion.
    drawn: Option<Option<u64>>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (page_tx, mut page_rx) = futures::channel::mpsc::unbounded::<String>();
        let (ended_tx, mut ended_rx) = futures::channel::mpsc::unbounded::<()>();
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
            #[cfg(target_os = "macos")]
            {
                webkeys::fix_key_handling();
                webkeys::allow_pinch_zoom(&webview);
            }
            #[cfg(target_os = "linux")]
            linux::attach(&webview, window, cx);
            dialogs::show_page_dialogs(&webview);
            webcontent::on_process_ended(&webview, move || drop(ended_tx.unbounded_send(())));
            WebView::new(webview, window, cx)
        });

        cx.spawn(async move |this, cx| {
            while ended_rx.next().await.is_some() {
                if this.update(cx, |this, cx| this.on_web_content_ended(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        cx.spawn_in(window, async move |this, cx| {
            while let Some(body) = page_rx.next().await {
                if this.update_in(cx, |this, window, cx| this.on_page_message(&body, window, cx)).is_err() {
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
            if let InputEvent::PressEnter { secondary, shift: false } = event {
                // An empty box answers the sign-in card: ⏎ signs in again.
                if input.read(cx).value().trim().is_empty() && this.sign_in_again(cx) {
                    return;
                }
                // An empty box answers a pending approval: ⏎ allow, ⌘⏎ always this session where the card offers it.
                // With words in it, ⏎ queues them as it does while Claude works.
                if input.read(cx).value().trim().is_empty()
                    && let Some(key) = this.active
                    && this.session_mut(key).is_some_and(|s| s.pending_permission().is_some())
                {
                    this.with_session(key, cx, |s| {
                        let scope = if *secondary { crate::approval::command_enter_scope(s) } else { session::Scope::Once };
                        s.answer_pending(PermissionOptionKind::AllowOnce, scope);
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
            let Ok(busy) = this.update(cx, |this, cx| {
                this.check_opening(cx);
                this.check_usage_limit(cx);
                !this.usage_limits.is_empty()
                    || this.sessions.iter().any(|s| s.busy_since.is_some())
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
            this.window_active = window.is_window_active();
            if window.is_window_active() {
                this.recheck_sign_in(cx);
                // Back from a "Claude is waiting for you" notification: its session.
                if let Some(key) = notify::take_clicked().filter(|key| this.sessions.iter().any(|s| s.key == *key)) {
                    this.activate(key, cx);
                }
            }
        })
        .detach();

        let recent = load_recent();
        let mut draft = Draft::new(new_session::default_folder(&recent), window, cx);
        let settings = Settings::load();
        draft.agent = settings.agent;
        let mut this = Self {
            webview,
            input,
            sessions: Vec::new(),
            active: None,
            // Keys name sessions to the runtime, which can outlive a launch (Keep notebooks
            // running): starting from the clock keeps them from matching an earlier launch's.
            next_key: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_millis() as u64),
            draft,
            locate: None,
            recent,
            records: load_records(),
            this_mac_was_ready: false,
            titles: load_json("titles.json"),
            archived: load_json("archived.json"),
            filter_menu: None,
            sidebar_search: None,
            session_notebooks: load_json("notebooks.json"),
            pending_moved: load_json("pending-context.json"),
            renaming: None,
            menu: None,
            expanded: HashSet::new(),
            settings,
            settings_panel: None,
            settings_page: settings_panel::Page::Section(settings_panel::Section::Assistants),
            settings_checks: settings_panel::Checks::default(),
            resizing: None,
            composer: composer::Composer::new(cx),
            chip_popover: None,
            reply: None,
            reply_drafts: Default::default(),
            agent_options: load_agent_options(),
            setup: Setup::needed().then(Setup::default),
            account: signin::Account::Unknown,
            sign_in_checked: None,
            offline_since: None,
            probing: false,
            links: agent_process::Links::default(),
            codex_account: agent::Account::Unknown,
            antigravity_account: agent::Account::Unknown,
            status: "".into(),
            annotating: false,
            shots: HashMap::new(),
            file_tip: false,
            connections: HashMap::new(),
            listeners: HashMap::new(),
            lines: HashMap::new(),
            placeholder: "Type / for commands".into(),
            drawn: None,
            usage_limits: offline::UsageLimits::default(),
            crashes: crash::Crashes::default(),
            notice: None,
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
            window_active: true,
            last_ask_notice: None,
            notebook_rename: None,
            pane_resources: None,
            pane_partition_menu: false,
            #[cfg(debug_assertions)]
            page_debug: None,
            dialog_focus: std::cell::RefCell::new(HashMap::new()),
            past_row_focus: std::cell::RefCell::new(HashMap::new()),
            keyboard_home: cx.focus_handle(),
            sidebar_focus: cx.focus_handle(),
            confirm: None,
            find: None,
            queue_edit: None,
            opening: notebook_pane::OpeningWatch::default(),
            #[cfg(debug_assertions)]
            test_blanked: false,
        };
        window.focus(&this.keyboard_home, cx);
        cx.on_focus_lost(window, |this, window, cx| window.focus(&this.keyboard_home, cx)).detach();
        settings::set_webview_appearance(this.webview.read(cx).raw(), theme::is_light());
        // Under Match macOS, follow the Mac's own switch while running.
        cx.observe_window_appearance(window, |this, _, cx| {
            if this.settings.appearance == settings::Appearance::System {
                this.apply_look(cx);
            }
        })
        .detach();
        // This Mac's Julia boots while the user picks a folder on the new-session screen.
        this.connect_host(&HostId::ThisMac, true, cx);
        // Claude starts alongside it, except on first launch, whose setup screen goes step by step.
        if this.setup.is_none() {
            this.ensure_agent(agent::Agent::Claude, cx);
            // The new-session screen shows the last agent picked: its sign-in and options.
            this.ensure_agent(this.draft_agent(), cx);
        }
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
            self.apply_effects(key, Vec::new(), cx);
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

    /// Start a session with the new-session screen's choices, and the input's
    /// text (if any) as its first message. A chosen notebook opens in safe
    /// preview. On a server, Julia starts now if it isn't running.
    fn start_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popover(window, cx);
        if self.composer_empty(cx) || self.run_own_command(window, cx) {
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
        let agent = self.draft_agent();
        if !self.records.was_listed(agent, &place) {
            self.links.send(agent, Command::ListSessions { cwd: host.agent_cwd(&folder) });
        }
        let mut session = Session::new(key, place, server.clone());
        session.agent = agent;
        // As the job will ask for them (fitted to the partition), so the session's chip says what was submitted.
        session.resources = self.draft.resources.as_ref().filter(|_| self.is_cluster(&host)).map(|r| self.draft_cluster().map_or_else(|| r.clone(), |c| c.job(r).resources));
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
            NotebookChoice::Existing(path) => Some(path.clone()),
        };
        let mut context: Vec<String> = agent.facts().session_intro.map(str::to_owned).into_iter().collect();
        if let (Some(server), HostId::Server(id)) = (&server, &host) {
            let ssh = self.hosts.server(id).map(|s| s.ssh_host.clone()).unwrap_or_default();
            context.push(format!(
                "[Endeavor] This session works on the server {server} (ssh host {ssh}), in the folder {}. Julia, Pluto, \
                 the notebook and the files are all on that server; your own file and shell tools are off because \
                 they'd see the user's computer instead. Use the notebook tools list_folder, read_file and run_shell (the user \
                 approves each command), with the server's paths.",
                folder
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
        let Some(agent) = self.sessions.iter().find(|s| s.key == key).map(|s| s.agent) else { return };
        // The agent isn't running yet, or is restarting or down: the session
        // opens once it is (`agent_ready`).
        if self.ensure_agent(agent, cx) || !self.links.get(agent).ready || !self.links.get(agent).process.up() || self.signed_out_of_agent(agent) {
            return;
        }
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let host = session.place.host.clone();
        let Some(bridge) = self.bridge(&host) else { return self.ensure_runtime(&host, cx) };
        let tools = agent::Tools { bridge: bridge.clone(), server: session.server.clone() };
        let folder = session.place.path.clone();
        let (policy, edits) = (session.policy(), session.edits_ask());
        cx.background_executor().spawn(async move { pluto::set_session_folder(&bridge, key, &folder) }).detach();
        self.send_policy(key, policy, edits, cx);
        let older = self.connections.get(&host).and_then(|c| c.older);
        let cwd = host.agent_cwd(&session.place.path);
        let _ = std::fs::create_dir_all(&cwd);
        let command = match session.id.clone() {
            Some(id) => Command::LoadSession { key, id, cwd, tools },
            None => Command::NewSession { key, cwd, tools },
        };
        self.links.send(agent, command);
        if let Some(session) = self.session_mut(key) {
            session.agent_waiting = false;
            if let Some(older) = older {
                session.runtime_build(older);
            }
        }
    }

    /// A past session's title: the user's name for it, else the recorded one,
    /// else its notebook's name; true when it only stands in for a title.
    fn past_title(&self, id: &SessionId) -> (String, bool) {
        let id = id.to_string();
        if let Some(title) = self.titles.get(&id).cloned().or_else(|| self.records.get(&id)?.title.clone()) {
            return (title, false);
        }
        let notebook = self.session_notebooks.get(&id).map(Place::name);
        (notebook.unwrap_or_else(|| "Earlier session".into()), true)
    }

    /// Reopen a past session (or switch to it if it's already open).
    fn open_past(&mut self, id: SessionId, place: Place, cx: &mut Context<Self>) {
        if let Some(key) = self.sessions.iter().find(|s| s.id.as_ref() == Some(&id)).map(|s| s.key) {
            return self.activate(key, cx);
        }
        let key = self.next_key;
        self.next_key += 1;
        let named = self.titles.get(&id.to_string()).cloned();
        let resources = self.session_resources.get(&id.to_string()).cloned();
        let mode = self.session_modes.get(&id.to_string()).cloned();
        // A "notebook moved" note that never reached the agent before the app quit.
        let pending = self.pending_moved.get(&id.to_string()).cloned();
        let (title, untitled) = self.past_title(&id);
        let notebook = self.session_notebooks.get(&id.to_string()).map(|p| p.path.clone());
        let server = (place.host != HostId::ThisMac).then(|| self.hosts.name(&place.host));
        // The row keeps its handle as it turns from past to open, so keyboard focus stays on it.
        let row_focus = self.past_row_focus.get_mut().remove(&id);
        let copy = transcript_copy::load(&id.to_string());
        let agent = self.records.get(&id.to_string()).map_or(agent::Agent::Claude, |r| r.agent);
        let mut session = Session::loading(key, id, place, server, title);
        session.agent = agent;
        if let Some(copy) = copy {
            session.show_copy(copy);
        }
        session.named = named.is_some();
        session.untitled = untitled;
        // A session from before modes were saved keeps the agent's own mode.
        session.run_without_asking = self.settings.run_without_asking;
        session.start_mode = mode;
        session.resources = resources;
        session.start_context = pending.map(|text| ContentBlock::Text(TextContent::new(text)));
        if self.holds(&session) {
            session.hold();
        }
        *session.focus.get_mut() = row_focus;
        let host = session.place.host.clone();
        self.sessions.push(session);
        // The recorded notebook opens as soon as its Julia is ready, without
        // waiting for the history: now if it's up, else with the runtime's
        // reopen (`reopen_notebooks`). Without one, the history's last
        // notebook opens once it has loaded (`Effect::ReopenNotebook`).
        if let Some(path) = notebook {
            self.bind_notebook(key, path.clone(), cx);
            if self.bridge(&host).is_some() {
                self.open_for_session(key, path, false, cx);
            }
        }
        self.request_agent(key, cx);
        self.activate(key, cx);
    }

    /// Save Endeavor's copy of a session's transcript (a turn ended, or it's
    /// closing). Not while its history loads: then it shows the copy itself.
    fn keep_transcript(&self, key: u64) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        if let Some(id) = session.id.as_ref().filter(|_| !session.opening() && session.failed.is_none()) {
            transcript_copy::save(&id.to_string(), &session.entries);
        }
    }

    /// Stop an open session; it goes back to its folder's history.
    fn close_session(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.sessions.iter().position(|s| s.key == key) else { return };
        self.keep_transcript(key);
        let session = self.sessions.remove(ix);
        if let Some(id) = session.id {
            self.links.send(session.agent, Command::CloseSession(id));
        }
        self.links.send(session.agent, Command::ListSessions { cwd: session.place.host.agent_cwd(&session.place.path) });
        if self.renaming.as_ref().is_some_and(|r| r.row == Row::Open(key)) {
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
        let agent = self.records.get(&id.to_string()).map_or(agent::Agent::Claude, |r| r.agent);
        // Deleting reaches an agent that isn't running once it starts.
        self.links.send(agent, Command::DeleteSession(id.clone()));
        if self.renaming.as_ref().is_some_and(|r| matches!(&r.row, Row::Past(p, _) if *p == id)) {
            self.renaming = None;
        }
        if self.records.remove(&id.to_string()) {
            self.save_records();
        }
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
        if self.pending_moved.remove(&id.to_string()).is_some() {
            save_json("pending-context.json", &self.pending_moved);
        }
        transcript_copy::delete(&id.to_string());
        cx.notify();
    }

    /// A menu or popover open that a click outside it, even on the web view,
    /// or Esc should close: the session row / notebook ⋮ and Share menus, the
    /// composer's Mode/Plus/Config menu, a sent chip's popover, the sidebar's
    /// Active/All filter, a new-session chip's popover, and the notebook picker.
    fn dismissible_open(&self) -> bool {
        self.confirm.is_some()
            || self.locate.is_some()
            || self.menu.is_some()
            || self.composer.menu.is_some()
            || self.chip_popover.is_some()
            || self.filter_menu.is_some()
            || (self.draft.popover.is_some() && self.active.is_none())
            || self.settings_panel.is_some()
    }

    /// Close whatever `dismissible_open` found, as a click outside it does.
    pub(crate) fn close_dismissible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The confirm dialog is the topmost thing that can be open, even over
        // Settings (Stop, Repair and Sign out all open it from a Settings page).
        if self.confirm.is_some() {
            return self.close_confirm(window, cx);
        }
        // Settings closes on a click outside it only when nothing inside it was open.
        if let Some(panel) = &mut self.settings_panel {
            if panel.idle_menu {
                panel.idle_menu = false;
                return cx.notify();
            }
            return self.close_settings(window, cx);
        }
        self.close_menu(window, cx);
        self.close_composer_menus(cx);
        self.close_filter_menu(window, cx);
        self.locate = None;
        if self.draft.popover.is_some() {
            self.close_popover(window, cx);
        }
        cx.notify();
    }

    /// A message for a session that couldn't open: open it first (a copy, for
    /// one open in Claude Code's CLI), so the message goes once it is.
    fn open_before_sending(&mut self, key: u64, cx: &mut Context<Self>) {
        match self.sessions.iter().find(|s| s.key == key).and_then(|s| s.failed.as_ref()).map(|f| f.kind) {
            Some(session::OpenFailure::InCli) => self.open_copy(key, cx),
            Some(session::OpenFailure::Other(_)) => self.retry_open(key, cx),
            None => {}
        }
    }

    /// Try again on a session that couldn't open.
    fn retry_open(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(key) else { return };
        session.retry_open();
        self.request_agent(key, cx);
        cx.notify();
    }

    /// Continue a session that couldn't be reopened (e.g. live in the CLI) as a copy.
    fn open_copy(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        let Some(session) = self.session_mut(key) else { return };
        let cwd = session.place.host.agent_cwd(&session.place.path);
        let tools = agent::Tools { bridge, server: session.server.clone() };
        let agent = session.agent;
        if let Some(source) = session.reopen_as_copy() {
            self.links.send(agent, Command::ForkSession { key, source, cwd, tools });
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
        let place = Place { host: session.place.host.clone(), path: path.clone() };
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
    pub(crate) fn stop_notebook(&mut self, key: u64, path: String, cx: &mut Context<Self>) {
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
                    Err(e) => {
                        let title = format!("Couldn't stop {}", folder_name(Path::new(&path)));
                        this.show_notice(notice::Notice::new(notice::Spot::NotebookRight, title, &e, Some(notice::Retry::StopNotebook(key, path.clone()))), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Show a session; the notebook pane follows it to the notebook it last viewed
    /// (without one, the pane draws a stand-in over the hidden web view).
    fn activate(&mut self, key: u64, cx: &mut Context<Self>) {
        if self.active != Some(key) && self.find.take().is_some() {
            webcontent::clear_find(self.webview.read(cx).raw());
        }
        self.active = Some(key);
        if let Some(s) = self.session_mut(key) {
            s.unseen = false;
        }
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

    /// A prompt waits in session `key`: with Endeavor in the background, one
    /// notification, then none from any session for `permits::NOTIFY_QUIET`.
    fn prompt_arrived(&mut self, key: u64, ix: usize) {
        let now = std::time::Instant::now();
        if self.window_active || !permits::may_notify(self.last_ask_notice, now) {
            return;
        }
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let Some(question) = approval::heading_at(session, ix) else { return };
        self.last_ask_notice = Some(now);
        notify::waiting(session.agent.name(), &question, &session.title, key);
    }

    fn apply_effects(&mut self, key: u64, mut effects: Vec<Effect>, cx: &mut Context<Self>) {
        effects.extend(self.session_mut(key).map(Session::take_later).unwrap_or_default());
        let agent = self.session_mut(key).map_or(agent::Agent::Claude, |s| s.agent);
        for effect in effects {
            match effect {
                Effect::Send(turn) => {
                    if let Some(id) = self.session_mut(key).and_then(|s| s.id.clone()) {
                        self.links.send(agent, Command::Turn(id, turn));
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
                Effect::CheckRunState => {
                    self.keep_transcript(key);
                    self.check_run_state(key, cx)
                }
                Effect::SetPolicy(policy, edits) => self.send_policy(key, policy, edits, cx),
                Effect::SignedOut => self.signed_out_of(agent, cx),
                Effect::Asked(ix) => self.prompt_arrived(key, ix),
                Effect::UsageLimit(reset) => self.hit_usage_limit(agent, reset, cx),
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
                Effect::FetchResult { call, tool, input } => {
                    let Some(bridge) = self.session_bridge(key) else { continue };
                    let call_id = call.to_string();
                    let task = cx.background_executor().spawn(async move { pluto::tool_result(&bridge, key, &call_id, &tool, &input) });
                    cx.spawn(async move |this, cx| match task.await {
                        Ok(Some(result)) => {
                            let _ = this.update(cx, |this, cx| {
                                let effects = this.session_mut(key).map(|s| s.result_fetched(&call, result)).unwrap_or_default();
                                this.apply_effects(key, effects, cx);
                            });
                        }
                        Ok(None) => {}
                        Err(e) => eprintln!("tool result: {e}"),
                    })
                    .detach();
                }
                Effect::AnswerRun { ask, allow, user_ran } => {
                    let Some(bridge) = self.session_bridge(key) else { continue };
                    cx.background_executor()
                        .spawn(async move {
                            // The ask may have gone meanwhile (the agent gave up on the call).
                            if let Err(e) = pluto::answer_run(&bridge, ask, allow, &user_ran) {
                                eprintln!("answer run {ask}: {e}");
                            }
                        })
                        .detach();
                }
                Effect::SetConfig(id_, value) => {
                    if let Some(id) = self.session_mut(key).and_then(|s| s.id.clone()) {
                        self.links.send(agent, Command::SetConfig(id, id_, value));
                    }
                }
                Effect::SetMode(mode) => {
                    if let Some(id) = self.session_mut(key).and_then(|s| s.id.clone()) {
                        self.links.send(agent, Command::SetMode(id, mode));
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
        let url = crate::webcontent::url(self.webview.read(cx).raw());
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
        if self.composer_empty(cx) || self.run_own_command(window, cx) {
            return;
        }
        // Claude Code reads a command only at the start of the message.
        let context = if slash::is_command(self.input.read(cx).value().trim()) { None } else { self.viewing_context(cx) };
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
        self.open_before_sending(key, cx);
        let Some(place) = self.session_mut(key).map(|s| s.place.clone()) else { return };
        let Some((text, attachments, mentioned)) = self.take_composer(window, cx) else { return };
        let edit = self.editing_queued(key);
        if edit {
            self.queue_edit_done(window, cx);
        }
        if !attachments.iter().any(|a| matches!(a, attach::Attachment::Upload { .. })) {
            return self.submit_message(key, context, (text, attachments, mentioned), now, edit, cx);
        }
        let (queued, ticket) = Queued::copying(text.clone(), attachments.clone(), context.into_iter().collect());
        let effects = self.submit_for(key, queued.as_edit(edit), now);
        self.apply_effects(key, effects, cx);
        let (progress, mut progressed) = futures::channel::mpsc::unbounded::<attach::Progress>();
        let helper = self.helper(&place.host);
        let (dest, fallback) = match (&place.host, helper) {
            (HostId::ThisMac, _) => (attach::Dest::Here(place.path.into()), None),
            (host, _) if !self.helper_saves_files(host) => (attach::Dest::Message, Some(attach::UNWRITABLE)),
            (_, Some(helper)) => {
                let ask = Box::new(move |request| helper.files(request));
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
    fn submit_message(&mut self, key: u64, context: Option<ContentBlock>, message: (String, Vec<attach::Attachment>, Vec<String>), now: bool, edit: bool, cx: &mut Context<Self>) {
        let (text, attachments, mentioned) = message;
        let mut blocks: Vec<_> = context.into_iter().collect();
        blocks.extend(attach::prompt_blocks(&text, &attachments, &mentioned));
        let effects = self.submit_for(key, Queued::new(text, attachments, blocks).as_edit(edit), now);
        self.apply_effects(key, effects, cx);
    }

    /// `session.submit`, plus: once its message carries away a pending
    /// "notebook moved" note (`notebook_moved`'s `start_context`), the
    /// persisted copy goes too, so a later restart doesn't resend it.
    fn submit_for(&mut self, key: u64, queued: Queued, now: bool) -> Vec<Effect> {
        let Some(session) = self.session_mut(key) else { return Vec::new() };
        let had_context = session.start_context.is_some();
        let effects = session.submit(queued, now);
        let sent = had_context && session.start_context.is_none();
        let id = session.id.clone();
        if let Some(id) = id.filter(|_| sent) {
            if self.pending_moved.remove(&id.to_string()).is_some() {
                save_json("pending-context.json", &self.pending_moved);
            }
        }
        effects
    }

    pub fn send_policy(&self, key: u64, policy: &'static str, edits: bool, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        // ponytail: a failed send leaves the runtime's policy stale until the next change.
        cx.background_executor().spawn(async move { pluto::set_policy(&bridge, key, policy, edits) }).detach();
    }

    /// ⇧⇥: the active session's next mode (e.g. default → plan → auto).
    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_last(window, cx);
    }

    /// ⌘F: Settings' own search while it's open; else find in the notebook,
    /// while it has the keyboard (a click in the page gives it the keyboard)
    /// or its find bar is open; else the sidebar search, while the sidebar has
    /// focus. The notebook comes before the sidebar: while the web view has the
    /// keyboard, GPUI's focus stays wherever it was last.
    fn find_setting(&mut self, _: &FindSetting, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_panel.is_some() {
            return self.focus_settings_search(window, cx);
        }
        let webview = self.webview.read(cx);
        if webview.visible() && (self.find.is_some() || webcontent::has_keyboard(webview.raw())) {
            return self.open_find(window, cx);
        }
        if self.sidebar_focus.contains_focused(window, cx) {
            self.open_sidebar_search(window, cx);
        }
    }

    /// ⌘G and ⇧⌘G: the next and previous match, while the find bar is open.
    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.find_in_page(false, cx);
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.find_in_page(true, cx);
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

    /// ⌘N, and a click on the sidebar's "New session" row.
    fn new_session(&mut self, _: &NewSession, _: &mut Window, cx: &mut Context<Self>) {
        self.active = None;
        self.scan_notebooks(cx);
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
        if self.confirm.is_some() {
            self.close_confirm(window, cx);
            return;
        }
        if self.settings_escape(window, cx) {
            return;
        }
        if self.close_reply(cx) {
            return;
        }
        if self.find.as_ref().is_some_and(|f| f.input.read(cx).focus_handle(cx).is_focused(window)) {
            return self.close_find(cx);
        }
        if self.close_composer_menus(cx) {
            return;
        }
        if self.cancel_queue_edit(window, cx) || self.close_context_popover(cx) {
            return;
        }
        // The filter menu closes itself (a submenu first) through its own
        // `on_action`, like the row ⋮ menu; only the sidebar search needs a
        // fallback here, the way the folder popover's search box does.
        if self.sidebar_search.is_some() {
            self.close_sidebar_search(window, cx);
            return;
        }
        if self.draft.popover.is_some() {
            return self.close_popover(window, cx);
        }
        if self.setup.is_none() && self.cancel_sign_in(cx) {
            return;
        }
        let Some(key) = self.active else { return };
        // Esc closes the card's ⌄ menu, then denies a pending approval, before it stops the turn.
        if let Some(s) = self.session_mut(key)
            && s.asks.always_menu
        {
            s.asks.always_menu = false;
            cx.notify();
            return;
        }
        if let Some(s) = self.session_mut(key)
            && s.answer_pending(PermissionOptionKind::RejectOnce, session::Scope::Once)
        {
            self.apply_effects(key, Vec::new(), cx);
            return;
        }
        if let Some(effect) = self.active_session().and_then(Session::interrupt) {
            self.apply_effects(key, vec![effect], cx);
        }
    }

    fn on_event(&mut self, agent: agent::Agent, event: AgentEvent, cx: &mut Context<Self>) {
        match event {
            AgentEvent::Ready => {
                self.status = format!("{} connected.", agent.name()).into();
                let link = self.links.get_mut(agent);
                link.ready = true;
                link.failed = false;
                self.agent_ready(agent, cx);
                self.finish_setup(cx);
                let mut listed = HashSet::new();
                for place in self.recent.iter().filter(|p| agent.facts().on_servers || p.host == HostId::ThisMac) {
                    let cwd = place.host.agent_cwd(&place.path);
                    if listed.insert(cwd.clone()) {
                        self.links.send(agent, Command::ListSessions { cwd });
                    }
                }
            }
            AgentEvent::Setup(p) => self.on_progress(p, cx),
            AgentEvent::SignedIn(method) => self.on_signed_in(method, cx),
            AgentEvent::CodexSignedIn(signed_in) => self.on_codex_signed_in(signed_in, cx),
            AgentEvent::AntigravitySignedIn(signed_in) => self.on_antigravity_signed_in(signed_in, cx),
            AgentEvent::SignInEnded(result) => self.on_antigravity_sign_in_ended(result, cx),
            AgentEvent::Listed { cwd, sessions: Ok(sessions) } => self.on_listed(agent, &cwd, sessions),
            // The sidebar keeps what the record has.
            AgentEvent::Listed { cwd, sessions: Err(e) } => eprintln!("Couldn't list the sessions in {}: {e}", cwd.display()),
            AgentEvent::Forked { key, id } => {
                if let Some(session) = self.session_mut(key) {
                    session.id = Some(id);
                }
            }
            // First launch: the setup screen says why, with Retry.
            AgentEvent::Failed(e) if self.setup.is_some() && agent == agent::Agent::Claude => {
                self.links.get_mut(agent).failed = true;
                if let Some(setup) = &mut self.setup {
                    setup.fail(e);
                }
            }
            AgentEvent::Failed(e) => self.agent_stopped(agent, e, cx),
            AgentEvent::Started { key, result } => {
                let Some(session) = self.session_mut(key) else { return };
                match result {
                    Ok(started) => {
                        let id = started.id.clone();
                        let place = session.place.clone();
                        if self.records.started(&id.to_string(), agent, &place, unix_now()) {
                            self.save_records();
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
                        if !started.config.is_empty() && self.agent_options.get(&agent) != Some(&started.config) {
                            self.agent_options.insert(agent, started.config.clone());
                            save_agent_options(&self.agent_options);
                        }
                        let picked = self.settings.config_picks(agent).clone();
                        let Some(session) = self.session_mut(key) else { return };
                        let queued = session.started(started);
                        // The user's last picks, ahead of a queued first message; the model
                        // first, since effort's choices depend on it.
                        let mut effects = Vec::new();
                        for &(id, _) in agent.facts().config {
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
            AgentEvent::Session(id, agent::SessionEvent::TurnFailed { kind, error }) => {
                let Some(key) = self.sessions.iter().find(|s| s.id.as_ref() == Some(&id)).map(|s| s.key) else { return };
                self.turn_failed(key, kind, error, cx);
            }
            AgentEvent::Session(id, event) => {
                let Some(session) = self.sessions.iter_mut().find(|s| s.id.as_ref() == Some(&id)) else { return };
                let key = session.key;
                let ended = matches!(event, agent::SessionEvent::TurnEnded(_));
                let effects = session.apply(event);
                let title = (!session.untitled && !session.named).then(|| session.title.clone());
                if self.records.touch(&id.to_string(), title.as_deref(), ended.then(unix_now)) {
                    self.save_records();
                }
                self.apply_effects(key, effects, cx);
            }
        }
        // A reply in the open session is seen as it arrives.
        if let Some(s) = self.active.and_then(|key| self.session_mut(key)) {
            s.unseen = false;
        }
        cx.notify();
    }

    fn save_records(&self) {
        save_json("sessions.json", self.records.saved());
    }

    /// An agent's listing of a folder, or of a server's whole agent folder.
    fn on_listed(&mut self, agent: records::Agent, cwd: &Path, sessions: Vec<SessionInfo>) {
        let scope = match HostId::of_agent_cwd(cwd) {
            Some(host) => records::Scope::Host(host),
            None => records::Scope::Folder(Place::local(cwd)),
        };
        let listed = sessions
            .into_iter()
            .map(|info| records::Listed {
                id: info.session_id.to_string(),
                title: info.title.as_deref().and_then(session::agent_title).filter(|t| !session::placeholder_title(t, &info.session_id.to_string())).map(str::to_owned),
                updated: info.updated_at.as_deref().and_then(when::parse_iso8601).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()),
            })
            .collect();
        let open: HashSet<String> = self.sessions.iter().filter_map(|s| Some(s.id.as_ref()?.to_string())).collect();
        let before: Vec<String> = self.records.saved().keys().cloned().collect();
        if self.records.merge(agent, scope, listed, &open) {
            self.save_records();
            // Sessions their agent no longer has: their copies of the transcript go too.
            before.iter().filter(|id| self.records.get(id).is_none()).for_each(|id| transcript_copy::delete(id));
        }
    }

    // -----------------------------------------------------------------------
    // Notebook pane and annotation mode
    // -----------------------------------------------------------------------

    /// Show notebook `id` of `host`'s Pluto in the pane.
    pub fn load_notebook(&mut self, host: &HostId, id: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self.connection(host).and_then(|c| c.runtime.as_ref()) else { return };
        // page_url carries the runtime's token; keep it app-side.
        let url = Backend::Pluto.notebook_url(&runtime.page_url, id);
        #[cfg(debug_assertions)]
        let url = if notebook_pane::test_stuck_opening().is_some_and(|keep| keep) { "about:blank".to_owned() } else { url };
        self.webview.update(cx, |w, _| w.load_url(&url));
    }

    fn on_page_message(&mut self, body: &str, window: &mut Window, cx: &mut Context<Self>) {
        match annotate::parse(body) {
            Some(annotate::Message::Ready) => {
                // A fresh page: it gets its context again, and reports its own state.
                self.page = annotate::PageState::default();
                self.annotating = false;
                self.page_context.clear();
                self.push_cells(cx);
                self.apply_look(cx);
                self.sync_page_context(cx);
            }
            Some(annotate::Message::State(state)) => return self.on_page_state(state, cx),
            Some(annotate::Message::RunNotebook { notebook }) => self.run_notebook(notebook, cx),
            Some(annotate::Message::RunAnyway { notebook, cells }) => self.run_anyway(notebook, cells, cx),
            Some(annotate::Message::AnswerCard { notebook, card }) => self.answer_card(notebook, card, cx),
            Some(annotate::Message::AskedVisible(visible)) => {
                if let Some(key) = self.active {
                    self.with_session(key, cx, |s| s.asks.cells_visible = Some(visible));
                }
            }
            Some(annotate::Message::Restart { notebook }) => {
                if let Some(key) = self.active_session().filter(|s| s.notebook.as_deref() == Some(notebook.as_str())).map(|s| s.key) {
                    self.restart_notebook(key, cx);
                }
            }
            Some(annotate::Message::FixPackage { notebook, name, log }) => self.fix_package(notebook, name, log, cx),
            Some(annotate::Message::ShowErrorAsk { cell }) => {
                if let Some(key) = self.active {
                    self.with_session(key, cx, |s| s.show_error_ask(&cell));
                }
            }
            Some(annotate::Message::CancelErrorAsk { cell }) => {
                if let Some(key) = self.active {
                    self.with_session(key, cx, |s| s.cancel_error_ask(&cell));
                }
            }
            Some(annotate::Message::Mode(on)) => {
                self.annotating = on;
                if on {
                    self.point_tip_done();
                }
            }
            Some(annotate::Message::Ask(ask)) if ask.add => {
                // A new cell asked for with ⌘↩: its chip and the words join the composer's message.
                self.composer.attachments.push(ask.attachment);
                let typed = self.input.read(cx).value().trim_end().to_string();
                let text = if typed.is_empty() { ask.text } else { format!("{typed}\n{}", ask.text) };
                self.input.update(cx, |s, cx| s.set_value(text, window, cx));
            }
            Some(annotate::Message::Ask(ask)) => {
                let Some(key) = self.active else { return };
                self.open_before_sending(key, cx);
                let mut blocks: Vec<_> = self.viewing_context(cx).into_iter().collect();
                let attachments = vec![ask.attachment];
                blocks.extend(attach::prompt_blocks(&ask.text, &attachments, &[]));
                let effects = self.submit_for(key, Queued::new(ask.text, attachments, blocks), false);
                self.apply_effects(key, effects, cx);
            }
            Some(annotate::Message::Shoot { id, rect }) => self.shoot(id, rect, cx),
            Some(annotate::Message::Quote(picks)) => self.take_picks(picks, cx),
            Some(annotate::Message::Code { cell, code }) => self.on_cell_code(cell, code, cx),
            #[cfg(debug_assertions)]
            Some(annotate::Message::Debug(page)) => return self.on_page_debug(page),
            None => return,
        }
        cx.notify();
    }

    /// Take a picture of part of the page for a quote to come. The page hides
    /// Point's dimming and outlines until told "shot".
    fn shoot(&mut self, id: u32, rect: [f64; 4], cx: &mut Context<Self>) {
        /// Pictures a page asked for and never used don't pile up.
        const KEPT: usize = 16;
        let zoom = if self.settings.zoom > 0. { self.settings.zoom } else { 1. };
        let shot = snapshot::png(self.webview.read(cx).raw(), rect.map(|n| n * zoom));
        cx.spawn(async move |this, cx| {
            let png = shot.await.ok().flatten();
            this.update(cx, |this, cx| {
                if this.shots.len() >= KEPT {
                    this.shots.clear();
                }
                if let Some(png) = png {
                    this.shots.insert(id, Arc::new(png));
                }
                this.send_to_page(&serde_json::json!({ "type": "shot", "id": id }), cx);
            })
        })
        .detach();
    }

    /// Quotes picked in the page, each with its picture; the comment goes on
    /// the last. A picture that couldn't be taken leaves the quote without one.
    fn take_picks(&mut self, picks: annotate::Picks, cx: &mut Context<Self>) {
        let annotate::Picks { quotes, comment, add } = picks;
        let last = quotes.len().saturating_sub(1);
        let quotes = quotes
            .into_iter()
            .enumerate()
            .map(|(i, (mut from, shot))| {
                if let (attach::Quoted::Cell { part: attach::Part::Figure(png), .. } | attach::Quoted::Box { png, .. }, Some(shot)) = (&mut from, shot)
                    && let Some(picture) = self.shots.remove(&shot)
                {
                    *png = picture;
                }
                attach::Quote { from, comment: if i == last { comment.clone() } else { String::new() } }
            })
            .collect();
        self.use_quotes(quotes, add, cx);
    }

    /// Quotes from Reply or Point: sent now as their own message (queued while
    /// Claude works), or added to the composer's message as cards.
    pub fn use_quotes(&mut self, quotes: Vec<attach::Quote>, add: bool, cx: &mut Context<Self>) {
        let quotes: Vec<_> = quotes.into_iter().map(attach::Attachment::Quote).collect();
        if add {
            self.composer.attachments.extend(quotes);
            return cx.notify();
        }
        let Some(key) = self.active else { return };
        self.open_before_sending(key, cx);
        let mut blocks: Vec<_> = self.viewing_context(cx).into_iter().collect();
        blocks.extend(attach::prompt_blocks("", &quotes, &[]));
        let effects = self.submit_for(key, Queued::new(String::new(), quotes, blocks), false);
        self.apply_effects(key, effects, cx);
        cx.notify();
    }

    /// ⌘⇧E, wherever the keyboard is, and the Point buttons.
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
        if self.setup.is_some() && self.links.get(agent::Agent::Claude).ready && !self.account.signed_out() {
            self.setup = None;
            Setup::finish();
            cx.notify();
        }
    }

    /// Retry the failed setup step: start Julia again, or the agent once Julia is up.
    pub fn retry_setup(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = &mut self.setup else { return };
        setup.clear_error();
        self.restart_agent(agent::Agent::Claude, cx);
    }

    /// What About Endeavor shows in its update strip.
    pub fn updates(&self) -> about::Updates {
        let adapter = match agent::adapter_status(agent::Agent::Claude) {
            Ok((version, false)) if self.links.get(agent::Agent::Claude).failed => about::Adapter::Available(version),
            Ok((version, false)) => about::Adapter::Installing(version),
            _ => about::Adapter::Current,
        };
        // Debug builds: while the file `ENDEAVOR_TEST_ADAPTER_UPDATE` names exists,
        // an adapter update shows as available (About, Settings, the gear's dot).
        #[cfg(debug_assertions)]
        if std::env::var_os("ENDEAVOR_TEST_ADAPTER_UPDATE").is_some_and(|f| Path::new(&f).exists()) {
            return about::Updates { app: None, adapter: about::Adapter::Available("0.4".into()) };
        }
        about::Updates { app: None, adapter }
    }

    /// About's Update: start Claude again, which installs the pinned adapter.
    pub fn update_adapter(&mut self, cx: &mut Context<Self>) {
        let link = self.links.get_mut(agent::Agent::Claude);
        if !link.failed {
            return;
        }
        link.failed = false;
        if let Some(setup) = &mut self.setup {
            setup.clear_error();
        }
        self.restart_agent(agent::Agent::Claude, cx);
    }

    fn restart_agent(&mut self, agent: agent::Agent, cx: &mut Context<Self>) {
        // The failed agent thread dropped its command channel; start with a new one.
        let link = self.links.get_mut(agent);
        let commands = link.renew();
        link.ready = false;
        if agent != agent::Agent::Claude || self.setup.is_none() || self.bridge(&HostId::ThisMac).is_some() {
            self.start_agent(agent, commands, cx);
        } else {
            // First launch: started once This Mac's Julia is up (`on_ready`).
            self.links.get_mut(agent).rx = Some(commands);
            self.ensure_runtime(&HostId::ThisMac, cx);
        }
        cx.notify();
    }

    /// Start an agent that isn't running yet. True if it starts now: its
    /// sessions open once it's connected (`agent_ready`).
    pub fn ensure_agent(&mut self, agent: agent::Agent, cx: &mut Context<Self>) -> bool {
        let Some(commands) = self.links.get_mut(agent).rx.take() else { return false };
        self.start_agent(agent, commands, cx);
        true
    }

    pub fn start_agent(&mut self, agent: agent::Agent, commands: UnboundedReceiver<Command>, cx: &mut Context<Self>) {
        let mut events = agent::start(agent, commands);
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |this, cx| this.on_event(agent, event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The notebook's appearance and theme, from Settings.
    fn apply_look(&self, cx: &mut Context<Self>) {
        let light = apply_appearance(self.settings.appearance, cx);
        settings::set_webview_appearance(self.webview.read(cx).raw(), light);
        if self.settings.zoom > 0. {
            let _ = self.webview.read(cx).raw().zoom(self.settings.zoom);
        }
        self.send_to_page(&serde_json::json!({ "type": "theme", "name": self.settings.notebook_theme.name() }), cx);
    }

    /// Mark the shown notebook's cells in the page (unrun, author).
    pub fn push_cells(&self, cx: &mut Context<Self>) {
        let url = crate::webcontent::url(self.webview.read(cx).raw());
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

    /// A rebuilt-every-render control's Tab-stop handle, keyed by an id unique
    /// to it and cached across renders.
    pub(crate) fn dialog_focus(&self, id: impl Into<String>, cx: &App) -> FocusHandle {
        self.dialog_focus.borrow_mut().entry(id.into()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
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

    /// In place of the transcript of a session that couldn't open: why, and the ways on.
    fn render_open_failure(&self, session: &Session, failure: &session::Failure, cx: &mut Context<Self>) -> AnyElement {
        let key = session.key;
        let try_again = |look| {
            failure::action("open-try-again", Some(new_session::Glyph::Restart), "Try again", look).on_click(cx.listener(move |this, _, _, cx| this.retry_open(key, cx)))
        };
        let (buttons, details) = match failure.kind {
            session::OpenFailure::InCli => (
                vec![
                    failure::action("open-copy", Some(new_session::Glyph::Copy), "Open a copy", signin::Look::Primary)
                        .on_click(cx.listener(move |this, _, _, cx| this.open_copy(key, cx)))
                        .into_any_element(),
                    try_again(signin::Look::Secondary).into_any_element(),
                ],
                None,
            ),
            session::OpenFailure::Other(_) => {
                let entity = cx.entity().downgrade();
                let details = failure::details("open-details", failure.raw.clone(), failure.details_open, move |_, cx| {
                    let _ = entity.update(cx, |this, cx| this.with_session(key, cx, Session::toggle_failure_details));
                });
                let mut buttons = vec![try_again(signin::Look::Primary).into_any_element()];
                // Only its history knows its notebook: pick it, and it opens beside this page.
                if session.notebook_beside() == session::Beside::Unknown {
                    buttons.push(
                        div()
                            .relative()
                            .child(
                                failure::action("open-notebook-only", Some(new_session::Glyph::File), "Open the notebook only", signin::Look::Secondary)
                                    .on_click(cx.listener(move |this, _, _, cx| this.pick_notebook(key, cx))),
                            )
                            .children(self.render_notebook_picker(key, cx))
                            .into_any_element(),
                    );
                }
                (buttons, Some(details))
            }
        };
        let body = vec![div().child(failure.body(session.notebook_beside(), session.agent)).into_any_element()];
        failure::page(new_session::Glyph::Bubble, failure.title(session.id.is_none()), body, buttons, details).into_any_element()
    }

    /// A session's chat: the transcript, then what waits above the composer, then the composer.
    /// `notebook_open`: its notebook shows in the pane (Point needs it).
    fn render_chat(&self, session: &Session, notebook_open: bool, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let margin = px(chat_margin(self.settings.layout.chat_width));
        let scroll_top = session.list.logical_scroll_top();
        let at_top = scroll_top.item_ix == 0 && scroll_top.offset_in_item <= px(0.);
        let at_end = session.list.is_scrolled_to_end().unwrap_or(true) || session.following();
        let top_fade = (!at_top).then(|| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(20.))
                .bg(linear_gradient(180., linear_color_stop(theme::bg_page(), 0.), linear_color_stop(theme::bg_page().opacity(0.), 1.)))
        });
        let bottom_fade = (!at_end).then(|| {
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(px(24.))
                .bg(linear_gradient(180., linear_color_stop(theme::bg_page().opacity(0.), 0.), linear_color_stop(theme::bg_page(), 1.)))
        });
        let summary = self.render_opening_summary(session);
        // What shows above the composer rises from it as it appears.
        let key = session.key;
        let card = |name: &'static str, card: Option<_>| card.map(|card| motion::arriving(div().child(card), ElementId::NamedInteger(name.into(), key), false));
        div()
            .relative()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .children(summary)
            .map(|d| match &session.failed {
                Some(failure) => d.child(div().flex_1().min_h_0().child(self.render_open_failure(session, failure, cx))),
                // The history shows whole once it has loaded, not growing as it arrives.
                None if session.opening() && !session.showing_copy() => d.child(div().flex_1().min_h_0()),
                None => d.child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        // After the text view has settled the selection the pointer made.
                        .capture_any_mouse_up(cx.listener(|_, event: &MouseUpEvent, window, cx| {
                            let at = event.position;
                            cx.defer_in(window, move |this, _, cx| this.check_reply_selection(at, cx));
                        }))
                        .child(transcript::render_transcript(session, margin, window, cx))
                        .children(top_fade)
                        .children(bottom_fade)
                        .children(transcript::render_jump(session, cx))
                        .children(self.render_reply(window, cx)),
                ),
            })
            .children(transcript::render_activity(session, self.offline_since, margin, cx))
            .child(
                div()
                    .px(margin)
                    .pb(px(11.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(card("pinned-plan", approval::render_pinned_plan(session, cx)))
                    .children(card("offline-line", self.render_offline_line(Some(session), cx)))
                    .children(card("usage-line", self.render_usage_line(session, cx)))
                    .children(card("runtime-wait", self.render_runtime_wait(session, cx)))
                    .children(card("agent-trouble", self.render_agent_trouble(session.agent, cx)))
                    .children(card("sign-in-card", self.render_agent_sign_in(session.agent, cx)))
                    .children(card("approval-card", approval::render_approval(session, notebook_open && session.notebook.as_deref() == Some(self.page.notebook.as_str()), window, cx)))
                    .child(self.render_queue(session, cx))
                    .child(self.render_composer(Some(session), notebook_open, window, cx)),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.drawn != Some(self.active) {
            self.drawn = Some(self.active);
            motion::hush(window);
        }
        // Another session was opened while one's queued message was in the box.
        if self.queue_edit.as_ref().is_some_and(|e| Some(e.key) != self.active) {
            self.cancel_queue_edit(window, cx);
        }
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
        overlay::set_dimmed(self.webview.read(cx).raw(), self.settings_panel.is_some() || self.confirm.is_some());
        let menu_over_notebook = self.menu.as_ref().is_some_and(|m| matches!(m.target, MenuTarget::Notebook(_) | MenuTarget::Share(_)));
        let point_tip = self.point_tip_shows(show_webview && active.is_some_and(|ix| self.sessions[ix].notebook_path.is_some()));
        let webview = self.webview.read(cx).raw();
        let settings_over_notebook = self.settings_panel.is_some() && show_webview;
        let confirm_over_notebook = self.confirm.is_some() && show_webview;
        for (hole, open) in [
            (overlay::Hole::Menu, menu_over_notebook),
            (overlay::Hole::Tip, point_tip),
            (overlay::Hole::Settings, settings_over_notebook),
            (overlay::Hole::Confirm, confirm_over_notebook),
            (overlay::Hole::Notice, show_webview && self.notice.as_ref().is_some_and(|n| n.spot != notice::Spot::Settings)),
        ] {
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
            let settings = self.render_settings_panel(window, cx).map(|d| deferred(d).with_priority(2));
            let confirm = self.render_confirm(cx).map(|d| deferred(d).with_priority(10));
            return div()
                .size_full()
                .bg(theme::bg_page())
                .text_color(theme::text_primary())
                .text_size(theme::size_body())
                .on_action(cx.listener(Self::interrupt))
                .on_action(cx.listener(Self::find_setting))
                .child(div().track_focus(&self.keyboard_home))
                .child(splash::render(setup, below, retry, cx))
                .children(settings)
                .children(confirm)
                .into_any_element();
        }
        let working = active.is_some_and(|ix| self.sessions[ix].outbox.busy && !self.sessions[ix].agent_waiting);
        let opening_wait = active.and_then(|ix| self.session_wait(&self.sessions[ix])).filter(|w| !matches!(w, opening::Waiting::Agent { .. } | opening::Waiting::SignIn(_)));
        let placeholder: SharedString = match active {
            None => "What do you want to work on?".into(),
            Some(_) if opening_wait.is_some() => opening_wait.as_ref().map(opening::Waiting::placeholder).unwrap_or_default().into(),
            Some(ix) if self.sessions[ix].pending_permission().is_some() => "Queue a message".into(),
            Some(ix) if working && self.links.get(self.sessions[ix].agent).process.up() => concat!("Queue a message, or ", crate::platform::shortcut!("⏎"), " to steer").into(),
            Some(_) if self.offline_since.is_some() => "Write a message. It sends when you're back online.".into(),
            Some(ix) => self.waiting_placeholder(self.sessions[ix].agent).unwrap_or_else(|| "Type / for commands".into()),
        };
        if self.placeholder != placeholder {
            self.placeholder = placeholder.clone();
            self.input.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        let chat = match active {
            Some(ix) => self.render_chat(&self.sessions[ix], stand_in.is_none(), window, cx).into_any_element(),
            None => self.render_new_session(window, cx).into_any_element(),
        };
        // Chat header: the session and its folder; the notebook header: its file.
        let (title, folder) = match active {
            Some(ix) => (self.session_title(self.sessions[ix].key, self.sessions[ix].title.clone(), cx), Some(self.folder_heading(&self.sessions[ix].place))),
            None => (div().overflow_hidden().whitespace_nowrap().child("New session").into_any_element(), None),
        };
        let chat_header = column_header("chat-header")
            .when(!self.settings.layout.sidebar_open, |d| d.when(cfg!(target_os = "macos"), |d| d.pl(px(TRAFFIC_LIGHTS))).child(sidebar_toggle(self, cx)))
            .child(title)
            .children(folder.map(|f| {
                div().px(px(6.)).rounded(px(3.)).bg(theme::bg_tag()).text_color(theme::text_tag()).font_family(theme::MONO).text_size(theme::size_meta_small()).child(f)
            }))
            .when(active.is_some_and(|ix| self.sessions[ix].showing_copy()), |d| {
                let agent_name = active.map_or("Claude", |ix| self.sessions[ix].agent.name());
                d.child(
                    div()
                        .id("read-only")
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(theme::size_meta_small())
                        .text_color(theme::text_faint())
                        .child(new_session::glyph(new_session::Glyph::Lock, theme::text_faint()))
                        .child("Read-only")
                        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(format!("Endeavor's own copy, shown while {agent_name} loads this session")).build(window, cx)),
                )
            });
        let notebook_header = column_header("notebook-header").map(|d| match active {
            None => d.child(self.draft_pane_header()),
            Some(ix) => {
                let layout = &self.settings.layout;
                let sidebar = if layout.sidebar_open { layout.sidebar_width + 1. } else { 0. };
                let width = window.viewport_size().width.as_f32() - sidebar - layout.chat_width - 1.;
                d.child(self.notebook_header(ix, stand_in.is_none(), width, cx))
            }
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
            .on_action(cx.listener(Self::reply_to_selection))
            .on_action(cx.listener(Self::cycle_mode))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::new_session))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::find_setting))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
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
            .child(div().track_focus(&self.keyboard_home))
            .child(platform::web_view_hooks())
            .child(notebook_pane::tooltip_hole_keeper(&self.webview))
            .when(self.settings.layout.sidebar_open, |d| d.child(self.render_session_bar(window, cx)).child(self.divider(Divider::Sidebar, theme::sidebar_edge(), cx)))
            .child(
                div()
                    .w(px(self.settings.layout.chat_width))
                    .min_w(px(CHAT_MIN))
                    .flex_shrink(1.)
                    .h_full()
                    .flex()
                    .flex_col()
                    // Its own tab group, right after the sidebar's, so Tab
                    // reaches the composer after the sidebar and not before it
                    // (paint order alone doesn't put them in this order).
                    .tab_group()
                    .tab_index(1)
                    .child(chat_header)
                    .child(chat),
            )
            .child(self.divider(Divider::Chat, theme::divider(), cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(NOTEBOOK_MIN))
                    .h_full()
                    .flex()
                    .flex_col()
                    .tab_group()
                    .tab_index(2)
                    .child(notebook_header)
                    .children(self.render_find_bar(window, cx).filter(|_| show_webview))
                    .children(active.and_then(|ix| self.render_pane_warning(&self.sessions[ix], cx)))
                    .child(div().flex_1().min_h_0().child(notebook))
                    .children(self.render_notice(false, cx)),
            )
            .children(self.render_notice(true, cx))
            // Deferred so they paint, and take clicks, above everything else.
            .children(self.render_settings_panel(window, cx).map(|d| deferred(d).with_priority(2)))
            .children(self.render_server_dialog(window, cx).map(|d| deferred(d).with_priority(3)))
            .children(self.render_askpass(cx).map(|d| deferred(d).with_priority(5)))
            .children(self.render_login_node_warning(cx).map(|d| deferred(d).with_priority(4)))
            // Topmost: it can open from within Settings (Stop, Repair, Sign out).
            .children(self.render_confirm(cx).map(|d| deferred(d).with_priority(10)))
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
    // An agent's adapter, in a job that ends its whole tree (agent_job).
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some(agent_job::FLAG) {
        agent_job::run(std::env::args().skip(2).collect());
    }
    // This Mac's runtime helper (runtime::connect), and ssh's askpass (remote::asker).
    if std::env::args().nth(1).as_deref() == Some(runtime::HELPER_FLAG) {
        endeavor_mcp::run_as(&[runtime::HELPER_FLAG], std::env::args().skip(2).collect());
    }
    // The Windows installer stopping a runtime kept running (runtime::stop_recorded).
    if std::env::args().nth(1).as_deref() == Some(runtime::STOP_FLAG) {
        runtime::stop_recorded();
        std::process::exit(0);
    }
    if [wire::askpass::ADDRESS_ENV, wire::askpass::SOCKET_ENV].iter().any(|v| std::env::var_os(v).is_some()) {
        // Ends the askpass if the server gives up on the sign-in meanwhile.
        askpass_watch::start();
        endeavor_mcp::run(std::env::args().skip(1).collect());
    }
    logs::start();
    platform::application().run(|cx: &mut App| {
        platform::init(cx);
        gpui_component::init(cx);
        theme::load_fonts(cx);
        apply_appearance(Settings::load().appearance, cx);
        motion::set_reduced(platform::reduces_motion(), cx);
        let mut reduce_motion = platform::watch_reduce_motion();
        cx.spawn(async move |cx| {
            use futures::StreamExt;
            while let Some(reduce) = reduce_motion.next().await {
                cx.update(|cx| motion::set_reduced(reduce, cx));
            }
        })
        .detach();
        // Input consumes Escape only when it has something to dismiss; otherwise it reaches us.
        cx.bind_keys([
            KeyBinding::new("escape", Interrupt, None),
            KeyBinding::new("secondary-shift-e", ToggleAnnotation, None),
            // ⌘E on text selected in a reply; ⌘J is the older key for it.
            KeyBinding::new("secondary-e", ReplyToSelection, Some("Input")),
            KeyBinding::new("secondary-e", ReplyToSelection, None),
            KeyBinding::new("secondary-j", ReplyToSelection, Some("Input")),
            KeyBinding::new("secondary-j", ReplyToSelection, None),
            // Registered after gpui-component's, so it beats the text box's own ⇧⇥ (outdent).
            // Only in the composer's box: everywhere else ⇧⇥ moves focus back.
            KeyBinding::new("shift-tab", CycleMode, Some("Composer > Input")),
            KeyBinding::new("shift-tab", CycleMode, Some("ModeMenu > Input")),
            KeyBinding::new("shift-tab", CycleMode, Some("MentionList > Input")),
            KeyBinding::new("shift-tab", CycleMode, Some("SlashList > Input")),
            KeyBinding::new("secondary-b", ToggleSidebar, Some("Input")),
            KeyBinding::new("secondary-b", ToggleSidebar, None),
            KeyBinding::new("secondary-n", NewSession, Some("Input")),
            KeyBinding::new("secondary-n", NewSession, None),
            KeyBinding::new("secondary-,", OpenSettings, None),
            KeyBinding::new("secondary-f", FindSetting, None),
            KeyBinding::new("secondary-g", FindNext, None),
            KeyBinding::new("secondary-shift-g", FindPrevious, None),
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
                    // Menu items, so their keys reach the app while the notebook has the
                    // keyboard: the web view offers keys to the menu first (webkeys.rs).
                    MenuItem::separator(),
                    MenuItem::action("Find…", FindSetting),
                    MenuItem::action("Find Next", FindNext),
                    MenuItem::action("Find Previous", FindPrevious),
                ],
            ),
            menu(
                "View",
                vec![
                    MenuItem::action("Toggle Sidebar", ToggleSidebar),
                    // A menu item, so ⌘⇧E reaches the app while the notebook has the keyboard.
                    MenuItem::action("Point", ToggleAnnotation),
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
                // lights sit in the sidebar's. Windows keeps its own title bar,
                // which has the window's buttons (the app draws none).
                titlebar: Some(TitlebarOptions {
                    title: Some("Endeavor".into()),
                    appears_transparent: !cfg!(windows),
                    traffic_light_position: Some(point(px(16.), px(16.))),
                }),
                ..Default::default()
            },
            |window, cx| {
                #[cfg(debug_assertions)]
                if let Some(preview) = splash::preview::open(cx) {
                    return cx.new(|cx| Root::new(preview, window, cx));
                }
                let workspace = cx.new(|cx| Workspace::new(window, cx));
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ShowAbout, cx| about::open_about(ws.clone(), cx));
                // Menu items and shortcuts pressed while the notebook has the keyboard reach
                // no focused GPUI element; these app-wide handlers forward them.
                let (ws, handle) = (workspace.downgrade(), window.window_handle());
                cx.on_action(move |_: &OpenSettings, cx| {
                    let ws = ws.clone();
                    cx.defer(move |cx| drop(handle.update(cx, |_, window, cx| ws.update(cx, |this, cx| this.open_settings_last(window, cx)))));
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

/// Resolve Settings → Appearance to light or dark and apply it everywhere
/// native: the windows' own chrome, every token in `theme`, and the component
/// library's theme. Returns whether it's light; the caller sets the notebook's.
fn apply_appearance(appearance: settings::Appearance, cx: &mut App) -> bool {
    use settings::Appearance;
    cx.set_window_appearance(match appearance {
        Appearance::Dark => Some(WindowAppearance::Dark),
        Appearance::Light => Some(WindowAppearance::Light),
        Appearance::System => None,
    });
    let light = match appearance {
        Appearance::Dark => false,
        Appearance::Light => true,
        Appearance::System => matches!(cx.window_appearance(), WindowAppearance::Light | WindowAppearance::VibrantLight),
    };
    theme::set_light(light);
    // Theme::change applies these before building the component defaults from them.
    let ui = Theme::global_mut(cx);
    let base = if light { &ui.light_theme } else { &ui.dark_theme };
    let mut colors = base.colors.clone();
    // What agent replies' markdown is drawn with: hairlines, link colour,
    // code block and table header backgrounds.
    colors.border = Some(theme::hex(theme::border()));
    colors.link = Some(theme::hex(theme::accent_text()));
    colors.muted = Some(theme::hex(theme::bg_card()));
    colors.table_head = Some(theme::hex(theme::bg_card()));
    colors.table_head_foreground = Some(theme::hex(theme::text_muted()));
    // The library's own focus ring (Input, Textarea, and anything else built
    // from it), so a tabbed-to text box matches our own focus rings.
    colors.ring = Some(theme::hex(theme::focus_ring()));
    colors.foreground = Some(theme::hex(theme::text_primary()));
    colors.popover_foreground = Some(theme::hex(theme::text_primary()));
    if light {
        colors.popover = Some(theme::hex(theme::popover_bg()));
    }
    let config = std::rc::Rc::new(ThemeConfig {
        font_family: Some(theme::SANS.into()),
        mono_font_family: Some(theme::MONO.into()),
        mono_font_size: Some(f32::from(theme::size_code())),
        colors,
        ..(**base).clone()
    });
    if light {
        ui.light_theme = config;
    } else {
        ui.dark_theme = config;
    }
    Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
    cx.refresh_windows();
    light
}

#[cfg(test)]
mod tests {
    use super::{load_json_at, save_json_at, viewed_notebook_id, write_atomic};
    use std::collections::HashMap;

    #[test]
    fn notebook_id_from_pluto_url() {
        let id = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let url = format!("http://127.0.0.1:1234/edit?token=t0k3n&id={id}");
        assert_eq!(viewed_notebook_id(&url), Some(id));
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/?token=t0k3n"), None);
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/edit?id=../../secret"), None);
    }

    /// `pending_moved` (the "notebook moved" note waiting to reach the agent,
    /// by session id): a move saves it, a restart's fresh load still has it,
    /// and once it's sent, clearing and saving again leaves it gone for good.
    #[test]
    fn pending_moved_note_survives_a_restart_and_clears_once_sent() {
        let dir = std::env::temp_dir().join(format!("endeavor-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        let file = dir.join("pending-context.json");

        let mut pending: HashMap<String, String> = HashMap::new();
        pending.insert("session-1".into(), "[Endeavor] The session's notebook file moved from a.jl to b.jl.".into());
        save_json_at(&file, &pending);

        // The app quit before the note went out; reopening the session loads it fresh.
        let restored: HashMap<String, String> = load_json_at(&file);
        assert_eq!(restored.get("session-1").map(String::as_str), Some("[Endeavor] The session's notebook file moved from a.jl to b.jl."));

        // A later message carries it to the agent: it's cleared and the clear is saved.
        let mut after_send = restored;
        after_send.remove("session-1");
        save_json_at(&file, &after_send);
        let reloaded: HashMap<String, String> = load_json_at(&file);
        assert!(reloaded.is_empty(), "sent and saved: nothing left to resend after another restart");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A save replaces the file whole and leaves nothing beside it.
    #[test]
    fn a_save_replaces_the_file_and_leaves_no_partial() {
        let dir = std::env::temp_dir().join(format!("endeavor-test-atomic-{}-{:?}", std::process::id(), std::thread::current().id()));
        let file = dir.join("titles.json");
        write_atomic(&file, b"{\"a\": \"first, and longer than the second\"}").unwrap();
        write_atomic(&file, b"{\"a\": \"second\"}").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"a\": \"second\"}");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["titles.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file torn by an old crash reads as empty, but is kept as `.bad`, so
    /// the next save doesn't overwrite what's left of it.
    #[test]
    fn a_file_that_doesnt_parse_is_set_aside_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("endeavor-test-torn-{}-{:?}", std::process::id(), std::thread::current().id()));
        let file = dir.join("sessions.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, "{\"session-1\": \"a.j").unwrap();

        let loaded: HashMap<String, String> = load_json_at(&file);
        assert!(loaded.is_empty());
        assert!(!file.exists());
        assert_eq!(std::fs::read_to_string(dir.join("sessions.json.bad")).unwrap(), "{\"session-1\": \"a.j");

        save_json_at(&file, &HashMap::from([("session-2".to_string(), "b.jl".to_string())]));
        assert_eq!(std::fs::read_to_string(dir.join("sessions.json.bad")).unwrap(), "{\"session-1\": \"a.j");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A cut inside a multi-byte character (an é in a title) leaves bytes that
    /// aren't UTF-8; that file is set aside too, not read as missing.
    #[test]
    fn a_file_cut_mid_character_is_set_aside_too() {
        let dir = std::env::temp_dir().join(format!("endeavor-test-torn-utf8-{}-{:?}", std::process::id(), std::thread::current().id()));
        let file = dir.join("titles.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, b"{\"session-1\": \"Caf\xC3").unwrap();

        let loaded: HashMap<String, String> = load_json_at(&file);
        assert!(loaded.is_empty());
        assert_eq!(std::fs::read(dir.join("titles.json.bad")).unwrap(), b"{\"session-1\": \"Caf\xC3");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
