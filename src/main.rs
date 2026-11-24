#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod click_log;
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
mod diagram;
mod details;
mod failure;
mod find_bar;
mod host_list;
mod hosts;
mod install;
mod layer;
mod logs;
mod new_session;
mod network;
mod notebook_pane;
mod notice;
mod offline;
mod older_runtime;
mod opening;
mod orbit;
mod own_julia;
mod own_r;
mod outbox;
#[cfg(target_os = "macos")]
mod overlay;
mod platform;
mod permits;
mod notify;
mod pluto;
mod queue;
mod cell_shot;
mod quotes;
mod records;
mod remote;
mod row_marks;
mod resources;
mod restart_ask;
mod runs;
mod runtime;
#[cfg(not(windows))]
mod save_image;
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
mod updater;
mod when;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::{overlay, snapshot, webcontent};
#[cfg(not(target_os = "macos"))]
use platform::dialogs;
#[cfg(windows)]
use platform::{overlay, snapshot, webcontent};

use agent::{AgentEvent, Command};
use layer::Layer;
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
use crate::theme::KeepsKeyboard as _;
use crate::theme::TabStop as _;

fn viewed_notebook_id(url: &str) -> Option<&str> {
    viewed_notebook(url).map(|(_, id)| id)
}

fn viewed_notebook(url: &str) -> Option<(Backend, &str)> {
    Backend::ALL.into_iter().find_map(|kind| Some((kind, kind.notebook_id(url).filter(|id| annotate::is_uuid(id))?)))
}

fn chosen_kind_note(kind: Backend) -> Option<&'static str> {
    match kind {
        Backend::Pluto => None,
        Backend::Ember => Some(
            "[Endeavor] The user chose an R notebook for this session. Make the notebook an R notebook \
             (a .R path, or new_notebook without a path, which makes one).",
        ),
        Backend::Marimo => Some(
            "[Endeavor] The user chose a Python notebook for this session. Make the notebook a Python (marimo) notebook \
             (a .py path, or new_notebook without a path, which makes one).",
        ),
    }
}

fn viewed_kind(url: &str) -> Option<Backend> {
    viewed_notebook(url).map(|(kind, _)| kind).or_else(|| Backend::Marimo.notebook_path(url).map(|_| Backend::Marimo))
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
        SendFeedback,
        ReportIssue
    ]
);

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

fn load_json_at<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(bytes) = std::fs::read(path) else { return T::default() };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        set_aside(path, &e);
        T::default()
    })
}

fn save_json_at(path: &Path, value: &impl serde::Serialize) {
    if let Ok(json) = serde_json::to_string_pretty(value)
        && let Err(e) = write_atomic(path, json.as_bytes())
    {
        eprintln!("Couldn't save {}: {e}", path.display());
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
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

fn rename_over(from: &Path, to: &Path) -> std::io::Result<()> {
    rename_held(from, to, &[20, 40, 60, 80, 100])
}

pub fn rename_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    rename_held(from, to, &[100, 200, 400, 800, 1000, 1000, 1000, 1000, 1000, 1000, 1000])
}

fn rename_held(from: &Path, to: &Path, waits: &[u64]) -> std::io::Result<()> {
    const HELD: [i32; 3] = [5, 32, 33];
    let mut waits = waits.iter();
    loop {
        match std::fs::rename(from, to) {
            Err(e) if cfg!(windows) && e.raw_os_error().is_some_and(|code| HELD.contains(&code)) => match waits.next() {
                Some(&ms) => std::thread::sleep(Duration::from_millis(ms)),
                None => return Err(e),
            },
            done => return done,
        }
    }
}

pub fn set_aside(path: &Path, why: &dyn std::fmt::Display) {
    let mut bad = path.as_os_str().to_owned();
    bad.push(".bad");
    match std::fs::rename(path, &bad) {
        Ok(()) => eprintln!("Couldn't read {} ({why}); kept it as {}.", path.display(), Path::new(&bad).display()),
        Err(e) => eprintln!("Couldn't read {} ({why}) or set it aside ({e}).", path.display()),
    }
}

const SIDEBAR_RANGE: (f32, f32) = (180., 400.);
const CHAT_MIN: f32 = 320.;
const NOTEBOOK_MIN: f32 = 360.;
const WINDOW_MIN: Size<Pixels> = Size { width: px(SIDEBAR_RANGE.0 + CHAT_MIN + NOTEBOOK_MIN + 2.), height: px(600.) };

const CHAT_CONTENT_MAX: f32 = 760.;
const CHAT_MARGIN_MAX: f32 = 50.;
const CHAT_MARGIN_BREAK: f32 = CHAT_CONTENT_MAX + 2. * CHAT_MARGIN_MAX;

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
        .keeps_keyboard_on_click()
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

fn context_ring(fraction: f32) -> impl IntoElement {
    const SIZE: f32 = 14.;
    const WIDTH: f32 = 2.;
    let fraction = fraction.clamp(0., 1.);
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let r = px((SIZE - WIDTH) / 2.);
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
const TRAFFIC_LIGHTS: f32 = 84.;

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

fn load_recent() -> Vec<Place> {
    load_json::<Vec<Place>>("recent.json").into_iter().filter(|p| p.here().is_none_or(Path::is_dir)).collect()
}

fn load_records() -> records::Records {
    records::Records::parse(&app_file("sessions.json").and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default())
}

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

const STREAM_FRAME: Duration = Duration::from_millis(50);

pub struct Workspace {
    webview: Entity<WebView>,
    pub input: Entity<TextareaState>,
    sessions: Vec<Session>,
    active: Option<u64>,
    next_key: u64,
    draft: Draft,
    locate: Option<new_session::Browser>,
    packages_feed: notebook_pane::PackagesFeed,
    recent: Vec<Place>,
    records: records::Records,
    this_mac_was_ready: bool,
    titles: HashMap<String, String>,
    archived: HashSet<String>,
    filter_menu: Option<FilterMenu>,
    sidebar_search: Option<Entity<InputState>>,
    session_notebooks: HashMap<String, Place>,
    pending_moved: HashMap<String, String>,
    renaming: Option<sidebar::Rename>,
    menu: Option<PopupMenu>,
    expanded: HashSet<Place>,
    settings: Settings,
    resizing: Option<Divider>,
    composer: composer::Composer,
    chip_popover: Option<composer::ChipPopover>,
    reply: Option<quotes::Reply>,
    reply_drafts: quotes::Drafts,
    agent_options: HashMap<agent::Agent, Vec<agent_client_protocol::schema::v1::SessionConfigOption>>,
    settings_panel: Option<settings_panel::Panel>,
    settings_page: settings_panel::Page,
    settings_checks: settings_panel::Checks,
    own_julia: own_julia::OwnJuliaState,
    own_r: own_r::OwnRState,
    setup: Option<Setup>,
    account: signin::Account,
    sign_in_checked: Option<std::time::Instant>,
    offline_since: Option<std::time::Instant>,
    probing: bool,
    links: agent_process::Links,
    app_update: updater::Updater,
    codex_account: agent::Account,
    codex_login: Option<(u64, u32)>,
    antigravity_account: agent::Account,
    status: SharedString,
    annotating: bool,
    shots: HashMap<u32, Arc<Vec<u8>>>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    cell_shots: futures::channel::mpsc::UnboundedSender<cell_shot::Ask>,
    restart_asks: futures::channel::mpsc::UnboundedSender<restart_ask::Ask>,
    restart_replies: HashMap<u64, restart_ask::Pending>,
    second: cell_shot::Second,
    file_tip: bool,
    connections: HashMap<HostId, connection::Connection>,
    listeners: HashMap<HostId, Arc<runtime::Listener>>,
    lines: HashMap<HostId, connection::Line>,
    placeholder: SharedString,
    usage_limits: offline::UsageLimits,
    crashes: crash::Crashes,
    notice: Option<notice::Notice>,
    missing_files: Option<String>,
    hosts: hosts::Hosts,
    server_dialog: Option<server_dialog::ServerDialog>,
    asks: VecDeque<server_dialog::AskModal>,
    session_resources: HashMap<String, wire::slurm::Resources>,
    session_modes: HashMap<String, session::Mode>,
    login_node_warning: Option<String>,
    login_node_ok: HashSet<String>,
    questions_tx: UnboundedSender<remote::Question>,
    page: annotate::PageState,
    drawer_ask: notebook_pane::DrawerAsk,
    page_context: String,
    window_active: bool,
    last_ask_notice: Option<std::time::Instant>,
    notebook_rename: Option<(u64, Entity<InputState>)>,
    pane_resources: Option<u64>,
    pane_partition_menu: bool,
    #[cfg(debug_assertions)]
    page_debug: Option<futures::channel::oneshot::Sender<serde_json::Value>>,
    dialog_focus: std::cell::RefCell<HashMap<String, FocusHandle>>,
    past_row_focus: std::cell::RefCell<HashMap<SessionId, FocusHandle>>,
    keyboard_home: FocusHandle,
    sidebar_focus: FocusHandle,
    zoom_chat: bool,
    zoom_chat_before_click: bool,
    confirm: Option<confirm::Confirm>,
    find: Option<find_bar::FindBar>,
    queue_edit: Option<queue::QueueEdit>,
    opening: notebook_pane::OpeningWatch,
    #[cfg(debug_assertions)]
    test_blanked: bool,
    drawn: Option<Option<u64>>,
    redraw_due: bool,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (page_tx, mut page_rx) = futures::channel::mpsc::unbounded::<String>();
        let (ended_tx, mut ended_rx) = futures::channel::mpsc::unbounded::<()>();
        let (new_window_tx, mut new_window_rx) = futures::channel::mpsc::unbounded::<String>();
        #[cfg(not(windows))]
        let (image_asks, mut image_ask_rx) = futures::channel::mpsc::unbounded::<save_image::Ask>();
        let webview = cx.new(|cx| {
            let handle = platform::webview_parent(window);
            let builder = wry::WebViewBuilder::new()
                .with_url("about:blank")
                .with_devtools(true)
                .with_initialization_script(&annotate::script())
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
                .with_new_window_req_handler(move |url, _| {
                    if let Some(url) = outside_link(&url) {
                        let _ = new_window_tx.unbounded_send(url);
                    }
                    wry::NewWindowResponse::Deny
                });
            #[cfg(target_os = "linux")]
            let builder = builder.with_download_started_handler(save_image::started(image_asks.clone()));
            #[cfg(windows)]
            let builder = wry::WebViewBuilderExtWindows::with_browser_accelerator_keys(builder, false);
            #[cfg(not(windows))]
            let webview = builder.build_as_child(&handle).expect("child webview");
            #[cfg(windows)]
            let webview = builder.build_as_child(&handle).unwrap_or_else(|e| platform::no_web_view(&e));
            #[cfg(target_os = "macos")]
            save_image::attach(&webview, image_asks.clone());
            #[cfg(target_os = "macos")]
            {
                webkeys::fix_key_handling();
                webkeys::allow_pinch_zoom(&webview);
                webkeys::no_new_windows();
            }
            #[cfg(target_os = "linux")]
            linux::attach(&webview, window, cx);
            #[cfg(windows)]
            webcontent::attach(&webview, window, cx);
            dialogs::show_page_dialogs(&webview);
            webcontent::on_process_ended(&webview, move || drop(ended_tx.unbounded_send(())));
            WebView::new(webview, window, cx)
        });

        let (cell_shots, mut cell_shot_rx) = futures::channel::mpsc::unbounded::<cell_shot::Ask>();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(ask) = cell_shot_rx.next().await {
                let Ok(job) = this.update_in(cx, |this, window, cx| this.shoot_cell(ask, window, cx)) else { break };
                let outcome = job.await;
                if this.update(cx, |this, cx| this.cell_shot_done(outcome, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        #[cfg(not(windows))]
        cx.spawn(async move |this, cx| {
            while let Some(ask) = image_ask_rx.next().await {
                if this.update(cx, |this, cx| this.save_image(ask, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let (restart_asks, mut restart_ask_rx) = futures::channel::mpsc::unbounded::<restart_ask::Ask>();
        cx.spawn(async move |this, cx| {
            while let Some(ask) = restart_ask_rx.next().await {
                if this.update(cx, |this, cx| this.restart_asked(ask, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |_, cx| {
            while let Some(url) = new_window_rx.next().await {
                cx.update(|cx| platform::open_url(&url, cx));
            }
        })
        .detach();

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

        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Type / for commands")
                .submit_on_enter(true)
                .auto_grow(1, 10)
                .scroll_bounce(false)
        });
        composer::subscribe(&input, window, cx);
        cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary, shift: false } = event {
                if input.read(cx).value().trim().is_empty() && this.sign_in_again(cx) {
                    return;
                }
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

        #[cfg(target_os = "linux")]
        {
            let mut clicks = notify::clicks();
            cx.spawn_in(window, async move |this, cx| {
                while clicks.next().await.is_some() {
                    let opened = this.update_in(cx, |this, window, cx| {
                        window.activate_window();
                        this.open_clicked(cx);
                    });
                    if opened.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        #[cfg(windows)]
        notify::set_window(window);

        let (questions_tx, mut questions) = futures::channel::mpsc::unbounded::<remote::Question>();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(question) = questions.next().await {
                if this.update_in(cx, |this, window, cx| this.on_question(question, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.on_app_quit(|this, _| {
            eprintln!("Endeavor is quitting");
            this.quit_runtimes();
            if let Err(e) = this.app_update.install(false) {
                eprintln!("Couldn't install the update: {e}");
            }
            async {}
        })
        .detach();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(20)).await;
            loop {
                let Ok(()) = this.update(cx, |this, cx| {
                    if this.app_update.due(Instant::now()) {
                        this.check_for_update(cx);
                    }
                }) else {
                    break;
                };
                cx.background_executor().timer(Duration::from_secs(600)).await;
            }
        })
        .detach();

        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok(busy) = this.update(cx, |this, cx| {
                this.check_opening(cx);
                this.check_usage_limit(cx);
                this.feed_packages(Instant::now(), cx);
                !this.usage_limits.is_empty()
                    || this.app_update.percent().is_some()
                    || this.working()
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
                this.open_clicked(cx);
            }
        })
        .detach();

        let recent = load_recent();
        let settings = Settings::load();
        theme::set_chat_scale(settings.chat_zoom as f32);
        let mut draft = Draft::new(new_session::default_folder(&recent), new_session::new_kind(&settings, &HostId::ThisMac), window, cx);
        draft.agent = settings.agent;
        draft.mode = settings.start_mode();
        let mut this = Self {
            webview,
            input,
            sessions: Vec::new(),
            active: None,
            next_key: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_millis() as u64),
            draft,
            locate: None,
            packages_feed: Default::default(),
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
            own_julia: own_julia::OwnJuliaState::default(),
            own_r: own_r::OwnRState::default(),
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
            app_update: updater::Updater::new(),
            codex_account: agent::Account::Unknown,
            codex_login: None,
            antigravity_account: agent::Account::Unknown,
            status: "".into(),
            annotating: false,
            shots: HashMap::new(),
            cell_shots,
            restart_asks,
            restart_replies: HashMap::new(),
            second: cell_shot::Second::default(),
            file_tip: false,
            connections: HashMap::new(),
            listeners: HashMap::new(),
            lines: HashMap::new(),
            placeholder: "Type / for commands".into(),
            drawn: None,
            redraw_due: false,
            usage_limits: offline::UsageLimits::default(),
            crashes: crash::Crashes::default(),
            notice: None,
            missing_files: install::missing_files(),
            hosts: hosts::Hosts::load(),
            server_dialog: None,
            asks: VecDeque::new(),
            session_resources: load_json("resources.json"),
            session_modes: load_json("modes.json"),
            login_node_warning: None,
            login_node_ok: HashSet::new(),
            questions_tx,
            page: annotate::PageState::default(),
            drawer_ask: Default::default(),
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
            zoom_chat: false,
            zoom_chat_before_click: false,
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
        cx.observe_window_appearance(window, |this, _, cx| {
            if this.settings.appearance == settings::Appearance::System {
                this.apply_look(cx);
            }
        })
        .detach();
        if this.missing_files.is_some() {
            #[cfg(debug_assertions)]
            this.watch_state_requests(cx);
            return this;
        }
        this.connect_host(&HostId::ThisMac, true, cx);
        if this.setup.is_none() {
            this.ensure_agent(this.draft_agent(), cx);
            if this.records.any_of(agent::Agent::Claude) {
                this.ensure_agent(agent::Agent::Claude, cx);
            }
        }
        this.scan_notebooks(cx);
        this.watch_network(cx);
        #[cfg(debug_assertions)]
        this.watch_state_requests(cx);
        #[cfg(debug_assertions)]
        this.watch_test_actions(window, cx);
        this
    }

    pub fn session_mut(&mut self, key: u64) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.key == key)
    }

    pub fn with_session(&mut self, key: u64, cx: &mut Context<Self>, f: impl FnOnce(&mut Session)) {
        if let Some(session) = self.session_mut(key) {
            f(session);
            self.apply_effects(key, Vec::new(), cx);
        }
    }

    fn active_session(&self) -> Option<&Session> {
        self.active.and_then(|key| self.sessions.iter().find(|s| s.key == key))
    }

    fn remember_mode(&mut self, key: u64) {
        let Some(mode) = self.sessions.iter().find(|s| s.key == key).and_then(Session::mode) else { return };
        if self.settings.remember_mode(&mode) {
            self.settings.save();
            if let Some(i) = session::app_modes().iter().position(|c| c.mode() == mode) {
                self.draft.mode = i;
                self.draft.mode_picked = false;
            }
        }
    }

    fn update_settings(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings);
        self.settings.save();
        cx.notify();
    }

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
        if let HostId::Server(id) = &host
            && let Some(server) = self.hosts.servers.iter_mut().find(|s| s.id == *id && s.fresh)
        {
            server.fresh = false;
            if let Err(e) = self.hosts.save() {
                eprintln!("{e}");
            }
        }
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
        session.kind = self.draft.notebook.kind();
        session.resources = self.draft.resources.as_ref().filter(|_| self.is_cluster(&host)).map(|r| self.draft_cluster().map_or_else(|| r.clone(), |c| c.job(r).resources));
        let start = self.draft.start_mode();
        session.start_mode = session::app_modes().get(start).map(|choice| session::Mode {
            run_without_asking: choice.run_without_asking || (start == session::MANUAL && self.settings.run_without_asking),
            ..choice.mode()
        });
        let job = session.resources.as_ref().zip(self.draft_cluster()).map(|(resources, cluster)| cluster.job(resources));
        if let (Some(job), Some(connection)) = (job, self.connections.get_mut(&host)) {
            connection.job_request = Some(job);
        }
        let existing = match &self.draft.notebook {
            NotebookChoice::New(_) => None,
            NotebookChoice::Existing(path) => Some(path.clone()),
        };
        let mut context: Vec<String> = agent.facts().session_intro.map(str::to_owned).into_iter().collect();
        if let (Some(server), HostId::Server(id)) = (&server, &host) {
            let ssh = self.hosts.server(id).map(|s| s.ssh_host.clone()).unwrap_or_default();
            context.push(format!(
                "[Endeavor] This session works on the server {server} (ssh host {ssh}), in the folder {}. The notebooks, \
                 what runs them and the files are all on that server; your own file and shell tools are off because \
                 they'd see the user's computer instead. Use the notebook tools list_folder, read_file and run_shell (the user \
                 approves each command), with the server's paths.",
                folder
            ));
            if let Some(resources) = &session.resources {
                context.push(format!(
                    "[Endeavor] {server} is a Slurm cluster: the notebooks run in a batch job ({}) on a compute node, and \
                     run_shell runs there too, inside that job. The notebooks stop when the job's time limit is reached.",
                    resources.summary()
                ));
            }
        }
        if let Some(chosen) = existing.is_none().then(|| chosen_kind_note(session.kind)).flatten() {
            context.push(chosen.into());
        }
        if let Some(path) = &existing {
            context.push(format!(
                "[Endeavor] The user started this session on the notebook {path}, which is open in the \
                 notebook pane in safe preview (nothing has run). Unless they say otherwise, \"the notebook\" \
                 means this one; list_notebooks gives its id."
            ));
        }
        session.start_context = (!context.is_empty()).then(|| ContentBlock::Text(TextContent::new(context.join("\n\n"))));
        if self.holds(&session) {
            session.hold();
        }
        self.sessions.push(session);
        self.send_session_folder(key, cx);
        if let Some(path) = existing {
            self.bind_notebook(key, path.clone(), cx);
            if self.bridge(&host).is_some() {
                self.open_for_session(key, path, false, cx);
            }
        }
        self.request_agent(key, cx);
        self.draft.notebook = self.new_notebook_choice();
        self.draft.preview = None;
        self.draft.mode = self.settings.start_mode();
        self.draft.mode_picked = false;
        self.activate(key, cx);
        self.send(key, None, false, window, cx);
    }

    pub fn request_agent(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(agent) = self.sessions.iter().find(|s| s.key == key).map(|s| s.agent) else { return };
        if self.ensure_agent(agent, cx) || !self.links.get(agent).ready || !self.links.get(agent).process.up() || self.signed_out_of_agent(agent) {
            return;
        }
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let host = session.place.host.clone();
        let Some(bridge) = self.bridge(&host) else { return self.ensure_runtime(&host, cx) };
        let tools = agent::Tools { bridge: bridge.clone(), server: session.server.clone() };
        let (policy, edits) = (session.policy(), session.edits_ask());
        self.send_session_folder(key, cx);
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

    fn past_title(&self, id: &SessionId) -> (String, bool) {
        let id = id.to_string();
        if let Some(title) = self.titles.get(&id).cloned().or_else(|| self.records.get(&id)?.title.clone()) {
            return (title, false);
        }
        let notebook = self.session_notebooks.get(&id).map(Place::name);
        (notebook.unwrap_or_else(|| "Earlier session".into()), true)
    }

    fn open_past(&mut self, id: SessionId, place: Place, cx: &mut Context<Self>) {
        if let Some(key) = self.sessions.iter().find(|s| s.id.as_ref() == Some(&id)).map(|s| s.key) {
            return self.activate(key, cx);
        }
        let key = self.next_key;
        self.next_key += 1;
        let named = self.titles.get(&id.to_string()).cloned();
        let resources = self.session_resources.get(&id.to_string()).cloned();
        let mode = self.session_modes.get(&id.to_string()).cloned();
        let pending = self.pending_moved.get(&id.to_string()).cloned();
        let (title, untitled) = self.past_title(&id);
        let notebook = self.session_notebooks.get(&id.to_string()).map(|p| p.path.clone());
        let server = (place.host != HostId::ThisMac).then(|| self.hosts.name(&place.host));
        let row_focus = self.past_row_focus.get_mut().remove(&id);
        let copy = transcript_copy::load(&id.to_string());
        let agent = self.records.get(&id.to_string()).map_or(agent::Agent::Claude, |r| r.agent);
        let kind = notebook.as_deref().map(new_session::kind_of_path).or_else(|| self.records.get(&id.to_string())?.kind).unwrap_or(Backend::Pluto);
        let mut session = Session::loading(key, id, place, server, title);
        session.agent = agent;
        session.kind = kind;
        if let Some(copy) = copy {
            session.show_copy(copy);
        }
        session.named = named.is_some();
        session.untitled = untitled;
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
        self.send_session_folder(key, cx);
        if let Some(path) = notebook {
            self.bind_notebook(key, path.clone(), cx);
            if self.bridge(&host).is_some() {
                self.open_for_session(key, path, false, cx);
            }
        }
        self.request_agent(key, cx);
        self.activate(key, cx);
    }

    fn keep_transcript(&self, key: u64) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        if let Some(id) = session.id.as_ref().filter(|_| !session.opening() && session.failed.is_none()) {
            transcript_copy::save(&id.to_string(), &session.entries, &session.restored_cards(), &session.restored);
        }
    }

    fn close_session(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.sessions.iter().position(|s| s.key == key) else { return };
        self.keep_transcript(key);
        let session = self.sessions.remove(ix);
        self.restart_session_closed(key);
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
        self.links.send(agent, Command::DeleteSession(id.clone()));
        if self.renaming.as_ref().is_some_and(|r| matches!(&r.row, Row::Past(p, _) if *p == id)) {
            self.renaming = None;
        }
        self.forget_session(&id.to_string());
        transcript_copy::delete(&id.to_string());
        cx.notify();
    }

    fn forget_session(&mut self, id: &str) {
        if self.records.remove(id) {
            self.save_records();
        }
        if self.titles.remove(id).is_some() {
            save_json("titles.json", &self.titles);
        }
        if self.session_modes.remove(id).is_some() {
            save_json("modes.json", &self.session_modes);
        }
        if self.archived.remove(id) {
            save_json("archived.json", &self.archived);
        }
        if self.session_notebooks.remove(id).is_some() {
            save_json("notebooks.json", &self.session_notebooks);
        }
        if self.pending_moved.remove(id).is_some() {
            save_json("pending-context.json", &self.pending_moved);
        }
        if self.session_resources.remove(id).is_some() {
            save_json("resources.json", &self.session_resources);
        }
    }

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

    pub(crate) fn close_dismissible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm.is_some() {
            return self.close_confirm(window, cx);
        }
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

    fn open_before_sending(&mut self, key: u64, cx: &mut Context<Self>) {
        match self.sessions.iter().find(|s| s.key == key).and_then(|s| s.failed.as_ref()).map(|f| f.kind) {
            Some(session::OpenFailure::InCli) => self.open_copy(key, cx),
            Some(session::OpenFailure::Other(_)) => self.retry_open(key, cx),
            None => {}
        }
    }

    fn retry_open(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(key) else { return };
        session.retry_open();
        self.request_agent(key, cx);
        cx.notify();
    }

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

    fn open_for_session(&mut self, key: u64, path: String, run: bool, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        if let Some(session) = self.session_mut(key) {
            session.open_failed = None;
        }
        let Some(host) = self.sessions.iter().find(|s| s.key == key).map(|s| s.place.host.clone()) else { return };
        let kind = new_session::kind_of_path(&path);
        let steps = self.julia_steps(&host, kind, cx);
        let opened = cx.background_executor().spawn({
            let path = path.clone();
            async move {
                let listed = pluto::call_tool(&bridge, "list_notebooks", serde_json::json!({})).ok();
                let open = listed.as_ref().and_then(|l| l.as_array()?.iter().find(|nb| nb["path"] == path.as_str()).cloned());
                let nb = match open {
                    Some(nb) => nb,
                    None => pluto::until_julia(|| pluto::open_notebook(&bridge, &path, run), &|step| steps.step(step))?,
                };
                nb["notebook_id"].as_str().map(str::to_owned).ok_or_else(|| format!("no notebook id in {nb}"))
            }
        });
        cx.spawn(async move |this, cx| {
            let opened = opened.await;
            let _ = this.update(cx, |this, cx| this.julia_answer(&host, kind, opened.as_ref().err().map(String::as_str), cx));
            let id = match opened {
                Ok(id) => id,
                Err(e) => {
                    let unexplained = connection::pane_failure(kind, &e).is_none().then(|| (path.clone(), e));
                    let _ = this.update(cx, |this, cx| this.check_missing(key, unexplained, cx));
                    return;
                }
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

    fn bind_notebook(&mut self, key: u64, path: String, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(key) else { return };
        session.notebook_path = Some(path.clone());
        session.kind = new_session::kind_of_path(&path);
        let kind = session.kind;
        let place = Place { host: session.place.host.clone(), path: path.clone() };
        let id = session.id.as_ref().map(ToString::to_string);
        if let Some(id) = &id
            && self.records.set_kind(id, kind)
        {
            self.save_records();
        }
        if let Some(id) = id
            && self.session_notebooks.get(&id) != Some(&place)
        {
            self.session_notebooks.insert(id, place);
            save_json("notebooks.json", &self.session_notebooks);
        }
        self.send_binding(key, path, cx);
    }

    fn send_session_folder(&self, key: u64, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        let Some((host, folder, kind)) = self.sessions.iter().find(|s| s.key == key).map(|s| (s.place.host.clone(), s.place.path.clone(), s.kind)) else { return };
        self.note_session_kind(&host, key, kind);
        cx.background_executor().spawn(async move { pluto::set_session_folder(&bridge, key, &folder, kind) }).detach();
    }

    fn send_binding(&self, key: u64, path: String, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        cx.background_executor().spawn(async move { pluto::set_notebook(&bridge, key, &path) }).detach();
    }

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

    fn notebook_closed(&mut self, key: u64, id: &str, path: Option<&str>, cx: &mut Context<Self>) {
        let Some(host) = self.sessions.iter().find(|s| s.key == key).map(|s| s.place.host.clone()) else { return };
        let stopped = Stopped { safe_preview: true, idle_hours: None, modified: None };
        let mut paths = Vec::new();
        let on_it = |s: &crate::session::Session| {
            s.notebook.as_deref() == Some(id) || path.zip(s.notebook_path.as_deref()).is_some_and(|(closed, own)| crate::connection::same_file(&host, own, closed))
        };
        for session in self.sessions.iter_mut().filter(|s| s.place.host == host && on_it(s)) {
            session.notebook = None;
            session.stopped = Some(stopped);
            if let Some(path) = session.notebook_path.clone().filter(|p| !paths.contains(p)) {
                paths.push(path);
            }
        }
        for path in paths {
            self.note_stopped_file(&host, path, cx);
        }
        cx.notify();
    }

    fn open_clicked(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = notify::take_clicked().filter(|key| self.sessions.iter().any(|s| s.key == *key)) {
            self.activate(key, cx);
        }
    }

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
        if self.notice.as_ref().is_some_and(|n| matches!(&n.retry, Some(notice::Retry::OpenNotebook(k, _)) if *k != key)) {
            self.notice = None;
        }
        self.show_open_failed(key, cx);
        cx.notify();
    }

    pub fn follow_folder(&mut self, cx: &mut Context<Self>) {
        let Some((key, folder)) = self.active_session().map(|s| (s.key, s.place.path.clone())) else { return };
        let Some(bridge) = self.session_bridge(key) else { return };
        cx.background_executor().spawn(async move { pluto::set_folder(&bridge, &folder) }).detach();
    }

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
                    let stop = matches!(turn, agent::Turn::Cancel);
                    if let Some(id) = self.session_mut(key).and_then(|s| s.id.clone()) {
                        self.links.send(agent, Command::Turn(id, turn));
                    }
                    if stop {
                        self.stop_session_runs(key, cx);
                    }
                }
                Effect::ShowNotebook { id, path } => {
                    if self.notice.as_ref().is_some_and(|n| matches!(&n.retry, Some(notice::Retry::OpenNotebook(k, _)) if *k == key)) {
                        self.notice = None;
                    }
                    let Some(session) = self.session_mut(key) else { continue };
                    session.notebook = Some(id.clone());
                    session.stopped = None;
                    session.missing = false;
                    session.open_failed = None;
                    let host = session.place.host.clone();
                    if let Some(path) = path {
                        self.bind_notebook(key, path, cx);
                    }
                    if self.active == Some(key) {
                        self.load_notebook(&host, &id, cx);
                    }
                }
                Effect::NotebookClosed { id, path } => self.notebook_closed(key, &id, path.as_deref(), cx),
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
                Effect::AnswerRestart { id, restart } => self.restart_answered(key, id, restart, cx),
                Effect::AnswerRun { ask, allow, user_ran, packages } => {
                    let Some(bridge) = self.session_bridge(key) else { continue };
                    cx.background_executor()
                        .spawn(async move {
                            if let Err(e) = pluto::answer_run(&bridge, ask, allow, &user_ran, packages.as_deref()) {
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

    fn save_mode(&mut self, key: u64) {
        let Some((id, mode)) = self.session_mut(key).and_then(|s| Some((s.id.as_ref()?.to_string(), s.mode()?))) else { return };
        if self.session_modes.get(&id) != Some(&mode) {
            self.session_modes.insert(id, mode);
            save_json("modes.json", &self.session_modes);
        }
    }

    fn viewing_context(&mut self, cx: &mut Context<Self>) -> Option<ContentBlock> {
        if !self.webview.read(cx).visible() {
            return None;
        }
        let url = crate::webcontent::url(self.webview.read(cx).raw());
        let id = self.shown_notebook_id(&url)?;
        let (mut edits, mut restored) = (Vec::new(), Vec::new());
        if let Some(session) = self.active.and_then(|key| self.session_mut(key)) {
            session.notebook = Some(id.clone());
            edits = std::mem::take(&mut session.user_edits);
            restored = std::mem::take(&mut session.restored);
        }
        if let Some(key) = self.active.filter(|_| !restored.is_empty()) {
            self.keep_transcript(key);
        }
        let mut text = format!(
            "[Endeavor] The user is viewing notebook {id} in the notebook pane. \
             Unless they say otherwise, \"the notebook\" means this one."
        );
        if !edits.is_empty() {
            let cells: Vec<String> = edits.iter().map(|(cell, name)| name.as_ref().map_or(cell.clone(), |n| format!("`{n}` ({cell})"))).collect();
            text += &format!(" Since you last heard, the user edited these cells: {}. Re-read them before relying on their code.", cells.join(", "));
        }
        for restored in restored {
            text += &format!(" With Restore, the user {restored}. Re-read the notebook before relying on that turn's changes.");
        }
        Some(ContentBlock::Text(TextContent::new(text)))
    }

    fn submit(&mut self, now: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active else { return };
        if self.composer_empty(cx) || self.run_own_command(window, cx) {
            return;
        }
        let context = if slash::is_command(self.input.read(cx).value().trim()) { None } else { self.viewing_context(cx) };
        self.send(key, context, now, window, cx);
    }

    fn send(&mut self, key: u64, context: Option<ContentBlock>, now: bool, window: &mut Window, cx: &mut Context<Self>) {
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
                let kind = this.session_kind(key);
                let done = (!text.is_empty() || !attachments.is_empty()).then(|| {
                    let blocks = attach::prompt_blocks(&text, &attachments, &mentioned, kind);
                    (attachments, blocks)
                });
                let Some(session) = this.session_mut(key) else { return };
                let effects = session.copied(ticket, done);
                this.apply_effects(key, effects, cx);
            });
        })
        .detach();
    }

    fn submit_message(&mut self, key: u64, context: Option<ContentBlock>, message: (String, Vec<attach::Attachment>, Vec<String>), now: bool, edit: bool, cx: &mut Context<Self>) {
        let (text, attachments, mentioned) = message;
        let mut blocks: Vec<_> = context.into_iter().collect();
        blocks.extend(attach::prompt_blocks(&text, &attachments, &mentioned, self.session_kind(key)));
        let effects = self.submit_for(key, Queued::new(text, attachments, blocks).as_edit(edit), now);
        self.apply_effects(key, effects, cx);
    }

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

    fn stop_session_runs(&self, key: u64, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = pluto::stop_session(&bridge, key) {
                    eprintln!("Stop: the runtime couldn't stop session {key}'s runs: {error}");
                }
            })
            .detach();
    }

    pub fn send_policy(&self, key: u64, policy: &'static str, edits: bool, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        cx.background_executor().spawn(async move { pluto::set_policy(&bridge, key, policy, edits) }).detach();
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_last(window, cx);
    }

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

    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.find_in_page(false, cx);
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.find_in_page(true, cx);
    }

    pub(crate) fn zoom(&mut self, step: f64, reset: bool, menu: Option<menu::Before>, cx: &mut Context<Self>) {
        if self.zooms_chat(menu, cx) {
            let current = if self.settings.chat_zoom > 0. { self.settings.chat_zoom } else { 1. };
            self.settings.chat_zoom = if reset { 1. } else { theme::chat_scale_step(current as f32, step > 1.) as f64 };
            self.settings.save();
            theme::set_chat_scale(self.settings.chat_zoom as f32);
            for session in &self.sessions {
                session.list.remeasure();
            }
            return cx.notify();
        }
        if self.missing_files.is_some() {
            return;
        }
        let current = if self.settings.zoom > 0. { self.settings.zoom } else { 1. };
        self.settings.zoom = if reset { 1. } else { (current * step).clamp(0.5, 3.) };
        self.settings.save();
        let _ = self.webview.read(cx).raw().zoom(self.settings.zoom);
    }

    pub(crate) fn zooms_chat(&self, menu: Option<menu::Before>, cx: &App) -> bool {
        if self.settings_panel.is_some() {
            return false;
        }
        let webview = self.webview.read(cx);
        if let Some(before) = menu.or(self.menu.as_ref().map(|m| m.before)) {
            return before.chat && !(webview.visible() && before.notebook);
        }
        self.zoom_chat && !(webview.visible() && webcontent::has_keyboard(webview.raw()))
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.settings.layout.sidebar_open = !self.settings.layout.sidebar_open;
        self.settings.save();
        cx.notify();
    }

    fn new_session(&mut self, _: &NewSession, _: &mut Window, cx: &mut Context<Self>) {
        self.active = None;
        self.scan_notebooks(cx);
        cx.notify();
    }

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

    fn drag_divider(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = self.resizing else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.resizing = None;
            return;
        }
        let x = e.position.x.as_f32();
        match which {
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
            self.remember_mode(key);
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
        let mut streamed = false;
        match event {
            AgentEvent::Ready => {
                if self.agent_in_use(agent) {
                    self.status = self.connected_status(agent).into();
                }
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
            AgentEvent::Setup(_) if self.setup.as_ref().is_some_and(|s| s.agent.is_some_and(|a| a != agent)) => {}
            AgentEvent::Setup(_) if self.setup.is_none() && !self.agent_in_use(agent) => {}
            AgentEvent::Setup(p) => self.on_progress(p, cx),
            AgentEvent::SignedIn(method) => self.on_signed_in(method, cx),
            AgentEvent::CodexSignedIn(signed_in) => self.on_codex_signed_in(signed_in, cx),
            AgentEvent::AntigravitySignedIn(signed_in) => self.on_antigravity_signed_in(signed_in, cx),
            AgentEvent::SignInEnded(result) => self.on_antigravity_sign_in_ended(result, cx),
            AgentEvent::Listed { cwd, sessions: Ok(sessions), complete } => self.on_listed(agent, &cwd, sessions, complete),
            AgentEvent::Listed { cwd, sessions: Err(e), .. } => eprintln!("Couldn't list the sessions in {}: {e}", cwd.display()),
            AgentEvent::Forked { key, id } => {
                if let Some(session) = self.session_mut(key) {
                    session.id = Some(id);
                }
            }
            AgentEvent::Failed(e) if self.setup.as_ref().is_some_and(|s| s.agent == Some(agent)) => {
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
                        let (place, kind) = (session.place.clone(), session.kind);
                        if self.records.started(&id.to_string(), agent, &place, unix_now()) | self.records.set_kind(&id.to_string(), kind) {
                            self.save_records();
                        }
                        if let Some(resources) = self.session_mut(key).and_then(|s| s.resources.clone())
                            && self.session_resources.insert(id.to_string(), resources.clone()).as_ref() != Some(&resources)
                        {
                            save_json("resources.json", &self.session_resources);
                        }
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
                        let bound = session.notebook_path.clone();
                        let queued = session.started(started);
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
                        if let Some(path) = bound {
                            self.bind_notebook(key, path, cx);
                        }
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
                streamed = matches!(event, agent::SessionEvent::Update(_));
                let effects = session.apply(event);
                let title = (!session.untitled && !session.named).then(|| session.title.clone());
                if self.records.touch(&id.to_string(), title.as_deref(), ended.then(unix_now)) {
                    self.save_records();
                }
                self.apply_effects(key, effects, cx);
            }
        }
        if let Some(s) = self.active.and_then(|key| self.session_mut(key)) {
            s.unseen = false;
        }
        if streamed {
            self.redraw_soon(cx);
        } else {
            cx.notify();
        }
    }

    fn redraw_soon(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.redraw_due, true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STREAM_FRAME).await;
            this.update(cx, |this, cx| {
                this.redraw_due = false;
                cx.notify();
            })
        })
        .detach();
    }

    fn known_ids(&self) -> HashSet<String> {
        (self.titles.keys())
            .chain(self.session_notebooks.keys())
            .chain(self.session_modes.keys())
            .chain(self.session_resources.keys())
            .chain(self.pending_moved.keys())
            .chain(self.archived.iter())
            .cloned()
            .collect()
    }

    fn save_records(&self) {
        save_json("sessions.json", self.records.saved());
    }

    fn on_listed(&mut self, agent: records::Agent, cwd: &Path, sessions: Vec<SessionInfo>, complete: bool) {
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
        if self.records.merge(agent, scope, listed, complete, &open, &self.known_ids()) {
            self.save_records();
        }
    }

    fn session_kind(&self, key: u64) -> Backend {
        self.sessions.iter().find(|s| s.key == key).map_or(Backend::Pluto, |s| s.kind)
    }

    fn notebook_path(&self, host: &HostId, id: &str) -> Option<String> {
        let session = self.sessions.iter().filter(|s| &s.place.host == host && s.notebook.as_deref() == Some(id)).find_map(|s| s.notebook_path.clone());
        let listed = || self.connection(host)?.last_notebooks.iter().find(|(nb, _)| nb == id).map(|(_, path)| path.clone());
        session.or_else(listed)
    }

    fn notebook_id_at(&self, path: &str) -> Option<String> {
        let session = self.sessions.iter().filter(|s| s.notebook_path.as_deref() == Some(path)).find_map(|s| s.notebook.clone());
        let listed = || self.connections.values().find_map(|c| c.last_notebooks.iter().find(|(_, at)| at == path).map(|(nb, _)| nb.clone()));
        session.or_else(listed)
    }

    fn shown_notebook_id(&self, url: &str) -> Option<String> {
        match viewed_notebook_id(url) {
            Some(id) => Some(id.to_owned()),
            None => self.notebook_id_at(&Backend::Marimo.notebook_path(url)?),
        }
    }

    pub fn notebook_kind(&self, host: &HostId, id: &str) -> Backend {
        self.notebook_path(host, id).map_or(Backend::Pluto, |path| new_session::kind_of_path(&path))
    }

    pub fn load_notebook(&mut self, host: &HostId, id: &str, cx: &mut Context<Self>) {
        let path = self.notebook_path(host, id).unwrap_or_default();
        let kind = new_session::kind_of_path(&path);
        let Some(runtime) = self.connection(host).and_then(|c| c.runtime.as_ref()) else { return };
        let url = kind.notebook_url(&runtime.page_url, id, &path);
        #[cfg(debug_assertions)]
        let url = if notebook_pane::test_stuck_opening().is_some_and(|keep| keep) { "about:blank".to_owned() } else { url };
        self.drawer_ask.page_loading();
        self.webview.update(cx, |w, _| w.load_url(&url));
    }

    fn on_page_message(&mut self, body: &str, window: &mut Window, cx: &mut Context<Self>) {
        match annotate::parse(body) {
            Some(annotate::Message::Ready) => {
                self.page = annotate::PageState::default();
                self.annotating = false;
                self.page_context.clear();
                self.packages_feed.page_reloaded();
                self.push_cells(cx);
                self.apply_look(cx);
                self.sync_page_context(cx);
                if let Some(tab) = self.drawer_ask.page_ready() {
                    self.open_drawer(Some(&tab), cx);
                }
            }
            Some(annotate::Message::OutOfStep) => self.on_out_of_step(cx),
            Some(annotate::Message::State(state)) => return self.on_page_state(state, cx),
            Some(annotate::Message::RunNotebook { notebook }) => self.run_notebook(notebook, cx),
            Some(annotate::Message::RunAnyway { notebook, cells }) => self.run_anyway(notebook, cells, cx),
            Some(annotate::Message::AnswerCard { notebook, card }) => self.answer_card(notebook, card, cx),
            Some(annotate::Message::Docs(open)) => {
                self.page.drawer = open.then(|| "docs".to_owned());
                cx.notify();
            }
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
                blocks.extend(attach::prompt_blocks(&ask.text, &attachments, &[], self.session_kind(key)));
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

    fn shoot(&mut self, id: u32, rect: [f64; 4], cx: &mut Context<Self>) {
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

    pub fn use_quotes(&mut self, quotes: Vec<attach::Quote>, add: bool, cx: &mut Context<Self>) {
        let quotes: Vec<_> = quotes.into_iter().map(attach::Attachment::Quote).collect();
        if add {
            self.composer.attachments.extend(quotes);
            return cx.notify();
        }
        let Some(key) = self.active else { return };
        let mut staged = attach::take_staged_quotes(&mut self.composer.attachments);
        staged.extend(quotes);
        let quotes = staged;
        self.open_before_sending(key, cx);
        let mut blocks: Vec<_> = self.viewing_context(cx).into_iter().collect();
        blocks.extend(attach::prompt_blocks("", &quotes, &[], self.session_kind(key)));
        let effects = self.submit_for(key, Queued::new(String::new(), quotes, blocks), false);
        self.apply_effects(key, effects, cx);
        cx.notify();
    }

    fn toggle_annotation(&mut self, _: &ToggleAnnotation, _: &mut Window, cx: &mut Context<Self>) {
        let enable = !self.annotating;
        if enable {
            let _ = self.webview.read(cx).raw().focus();
        }
        self.send_to_page(&serde_json::json!({ "type": "annotate", "on": enable }), cx);
    }

    fn send_to_page(&self, msg: &serde_json::Value, cx: &mut Context<Self>) {
        let _ = self.webview.read(cx).raw().evaluate_script(&format!("window.__endeavor && window.__endeavor.receive({msg})"));
    }

    pub fn on_progress(&mut self, p: Progress, cx: &mut Context<Self>) {
        if !p.log {
            self.status = p.detail.clone().into();
        }
        if let Some(setup) = &mut self.setup {
            setup.apply(p);
        }
        cx.notify();
    }

    fn finish_setup(&mut self, cx: &mut Context<Self>) {
        let Some(agent) = self.setup.as_ref().and_then(|s| s.agent) else { return };
        if self.links.get(agent).ready && (agent != agent::Agent::Claude || !self.account.signed_out()) {
            self.setup = None;
            Setup::finish();
            cx.notify();
        }
    }

    pub fn retry_setup(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = &mut self.setup else { return };
        setup.clear_error();
        match setup.agent {
            Some(agent) => self.restart_agent(agent, cx),
            None => self.ensure_runtime(&HostId::ThisMac, cx),
        }
    }

    pub fn updates(&self) -> about::Updates {
        let adapter = match agent::adapter_status(agent::Agent::Claude) {
            Ok((version, false)) if self.links.get(agent::Agent::Claude).failed => about::Adapter::Available(version),
            Ok((_, false)) if self.links.get(agent::Agent::Claude).rx.is_some() && self.setup.as_ref().and_then(|s| s.agent) != Some(agent::Agent::Claude) => about::Adapter::Later,
            Ok((version, false)) => about::Adapter::Installing(version),
            _ => about::Adapter::Current,
        };
        #[cfg(debug_assertions)]
        if std::env::var_os("ENDEAVOR_TEST_ADAPTER_UPDATE").is_some_and(|f| Path::new(&f).exists()) {
            return about::Updates { app: self.app_update_state(), adapter: about::Adapter::Available("0.4".into()) };
        }
        about::Updates { app: self.app_update_state(), adapter }
    }

    fn app_update_state(&self) -> about::AppUpdate {
        match &self.app_update.state {
            updater::State::Off(_) | updater::State::Installing => about::AppUpdate::Off,
            updater::State::Waiting => about::AppUpdate::Unchecked,
            updater::State::Checking => about::AppUpdate::Checking(self.app_update.percent()),
            updater::State::UpToDate => about::AppUpdate::UpToDate,
            updater::State::Ready { build, .. } if self.working() => about::AppUpdate::ReadyAfterTurn(build.clone()),
            updater::State::Ready { build, .. } => about::AppUpdate::Ready(build.clone()),
            updater::State::Manual { build } => about::AppUpdate::Manual { build: build.clone(), protected: self.app_update.place.as_ref().is_ok_and(|p| p.protected) },
            updater::State::Failed(e) => about::AppUpdate::Failed(e.clone()),
        }
    }

    pub fn check_for_update(&mut self, cx: &mut Context<Self>) {
        let Some(place) = self.app_update.start(Instant::now()) else { return };
        let progress = self.app_update.progress.clone();
        let checked = cx.background_executor().spawn(async move {
            updater::check(&place, updater::COMMIT, updater::COMMITTED, &|f| progress.store((f.clamp(0., 1.) * 1000.) as u32, std::sync::atomic::Ordering::Relaxed))
        });
        cx.spawn(async move |this, cx| {
            let result = checked.await;
            if let Err(e) = &result {
                eprintln!("Update check: {e}");
            }
            let _ = this.update(cx, |this, cx| {
                this.app_update.finish(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn working(&self) -> bool {
        self.sessions.iter().any(|s| s.busy_since.is_some())
    }

    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        if self.working() {
            return;
        }
        match self.app_update.install(true) {
            Ok(true) => cx.quit(),
            Ok(false) => {}
            Err(e) => {
                self.app_update.finish(Err(e));
                cx.notify();
            }
        }
    }

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

    pub(crate) fn restart_agent(&mut self, agent: agent::Agent, cx: &mut Context<Self>) {
        let link = self.links.get_mut(agent);
        let commands = link.renew();
        link.ready = false;
        if self.setup.as_ref().and_then(|s| s.agent) != Some(agent) || self.bridge(&HostId::ThisMac).is_some() {
            self.start_agent(agent, commands, cx);
        } else {
            self.links.get_mut(agent).rx = Some(commands);
            self.ensure_runtime(&HostId::ThisMac, cx);
        }
        cx.notify();
    }

    fn agent_in_use(&self, agent: agent::Agent) -> bool {
        agent == self.draft_agent() || self.active_session().is_some_and(|s| s.agent == agent)
    }

    pub fn ensure_agent(&mut self, agent: agent::Agent, cx: &mut Context<Self>) -> bool {
        let Some(commands) = self.links.get_mut(agent).rx.take() else { return false };
        self.start_agent(agent, commands, cx);
        true
    }

    pub fn start_agent(&mut self, agent: agent::Agent, commands: UnboundedReceiver<Command>, cx: &mut Context<Self>) {
        if self.missing_files.is_some() {
            self.links.get_mut(agent).rx = Some(commands);
            return;
        }
        let generation = self.links.get(agent).generation;
        let mut events = agent::start(agent, commands);
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let current = |this: &Self| this.links.get(agent).generation == generation;
                if this.update(cx, |this, cx| if current(this) { this.on_event(agent, event, cx) }).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_look(&self, cx: &mut Context<Self>) {
        let light = apply_appearance(self.settings.appearance, cx);
        settings::set_webview_appearance(self.webview.read(cx).raw(), light);
        if self.settings.zoom > 0. {
            let _ = self.webview.read(cx).raw().zoom(self.settings.zoom);
        }
        self.send_to_page(&serde_json::json!({ "type": "theme", "name": self.settings.notebook_theme.name() }), cx);
    }

    pub fn push_cells(&self, cx: &mut Context<Self>) {
        let url = crate::webcontent::url(self.webview.read(cx).raw());
        let Some(id) = self.shown_notebook_id(&url) else { return };
        let cells = self.connections.values().find_map(|c| c.cells.get(&id).cloned()).unwrap_or_else(|| serde_json::json!([]));
        self.send_to_page(&serde_json::json!({ "type": "cells", "cells": cells }), cx);
    }

    fn check_run_state(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(host) = self.sessions.iter().find(|s| s.key == key).map(|s| &s.place.host) else { return };
        let warnings = self.connection(host).map(|c| pluto::run_warnings(&c.notebooks)).unwrap_or_default();
        self.with_session(key, cx, |s| s.note_run_state(warnings));
    }

    pub(crate) fn dialog_focus(&self, id: impl Into<String>, cx: &App) -> FocusHandle {
        self.dialog_focus.borrow_mut().entry(id.into()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone()
    }

    pub(crate) fn app_menu_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let menu = self.menu.as_ref().filter(|menu| menu.target == MenuTarget::App);
        let line = || div().w(px(13.)).h(px(1.5)).rounded(px(1.)).bg(if menu.is_some() { theme::text_primary() } else { theme::text_muted() });
        div()
            .id("app-menu")
            .role(Role::Button)
            .aria_label("Menu")
            .relative()
            .flex_shrink_0()
            .size(px(24.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3.))
            .rounded(px(4.))
            .cursor_pointer()
            .tab_stop_on(theme::bg_sidebar())
            .when(menu.is_some(), |d| d.bg(theme::row_active()))
            .hover(|s| s.bg(theme::row_active()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if event.is_keyboard() {
                    this.zoom_chat_before_click = this.zoom_chat;
                }
                if this.menu.as_ref().is_some_and(|m| m.target == MenuTarget::App) {
                    this.close_menu(window, cx);
                } else {
                    this.open_menu(MenuTarget::App, None, window, cx);
                }
            }))
            .when(menu.is_none(), |d| d.tooltip(notebook_pane::tooltip("Menu", &self.webview)))
            .child(line())
            .child(line())
            .child(line())
            .children(menu.map(|menu| self.render_menu(menu, cx)))
    }

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
            .tab_stop_on(theme::bg_page())
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
        let key = session.key;
        let card = |name: &'static str, card: Option<_>| card.map(|card| motion::arriving(div().child(card), ElementId::NamedInteger(name.into(), key), false));
        div()
            .relative()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .line_height(theme::chat_px(19.5))
            .children(summary)
            .map(|d| match &session.failed {
                Some(failure) => d.child(div().flex_1().min_h_0().child(self.render_open_failure(session, failure, cx))),
                None if session.opening() && !session.showing_copy() => d.child(div().flex_1().min_h_0()),
                None => d.child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
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
        if self.queue_edit.as_ref().is_some_and(|e| Some(e.key) != self.active) {
            self.cancel_queue_edit(window, cx);
        }
        let active = self.active.and_then(|key| self.sessions.iter().position(|s| s.key == key));
        let stand_in = active.and_then(|ix| self.notebook_page(&self.sessions[ix], cx));
        self.sync_page_context(cx);
        let modal = self.server_dialog.is_some() || !self.asks.is_empty() || self.login_node_warning.is_some();
        let show_webview = self.setup.is_none() && active.is_some() && stand_in.is_none() && !modal;
        if self.webview.read(cx).visible() != show_webview {
            self.webview.update(cx, |w, _| if show_webview { w.show() } else { w.hide() });
        }
        overlay::set_dimmed(self.webview.read(cx).raw(), self.settings_panel.is_some() || self.confirm.is_some());
        self.second.follow_pane();
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
        if let Some(missing) = &self.missing_files {
            return div()
                .size_full()
                .bg(theme::bg_page())
                .text_color(theme::text_primary())
                .text_size(theme::size_body())
                .flex()
                .items_center()
                .justify_center()
                .child(div().track_focus(&self.keyboard_home))
                .child(div().max_w(px(460.)).px_6().flex().flex_col().gap_3().children(missing.split("\n\n").enumerate().map(|(i, p)| div().when(i > 0, |d| d.text_color(theme::text_muted())).child(p.to_owned()))))
                .into_any_element();
        }
        if let Some(setup) = &self.setup {
            let below = match self.render_sign_in_panel(cx) {
                _ if self.offline_since.is_some() => splash::Below::Card(self.render_offline_setup(setup, cx)),
                Some((line, panel, bar, tucked)) => splash::Below::Panel { line, bar: bar.then_some(0.78), panel, tucked },
                None => splash::Below::Progress,
            };
            let retry = cx.listener(|this, _, _, cx| this.retry_setup(cx));
            let change = (setup.agent.is_some() && setup.step() >= splash::Step::Agent).then(|| cx.listener(|this, _, _, cx| this.change_assistant(cx)));
            let settings = self.render_settings_panel(window, cx).map(|d| layer::place(Layer::Settings, d));
            let server_dialog = self.render_server_dialog(window, cx).map(|d| layer::place(Layer::ServerDialog, d));
            let askpass = self.render_askpass(cx).map(|d| layer::place(Layer::Askpass, d));
            let login_node = self.render_login_node_warning(cx).map(|d| layer::place(Layer::LoginNodeWarning, d));
            let confirm = self.render_confirm(cx).map(|d| layer::place(Layer::Confirm, d));
            return div()
                .size_full()
                .bg(theme::bg_page())
                .text_color(theme::text_primary())
                .text_size(theme::size_body())
                .on_action(cx.listener(Self::interrupt))
                .on_action(cx.listener(Self::find_setting))
                .child(div().track_focus(&self.keyboard_home))
                .child(splash::render(setup, below, retry, change, &|id| self.dialog_focus(id, cx), cx))
                .children(settings)
                .children(server_dialog)
                .children(askpass)
                .children(login_node)
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
        let (title, folder) = match active {
            Some(ix) => (self.session_title(self.sessions[ix].key, self.sessions[ix].title.clone(), cx), Some(self.folder_heading(&self.sessions[ix].place))),
            None => (div().overflow_hidden().whitespace_nowrap().child("New session").into_any_element(), None),
        };
        let chat_header = column_header("chat-header")
            .when(!self.settings.layout.sidebar_open, |d| {
                d.when(cfg!(target_os = "macos"), |d| d.pl(px(TRAFFIC_LIGHTS)))
                    .when(!cfg!(target_os = "macos"), |d| d.child(self.app_menu_button(cx)))
                    .child(sidebar_toggle(self, cx))
            })
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
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                this.zoom_chat_before_click = this.zoom_chat;
                this.zoom_chat = false;
                click_log::note(|| format!("gpui down {:?} at {:.0},{:.0}", event.button, f32::from(event.position.x), f32::from(event.position.y)));
            }))
            .capture_key_down(cx.listener(|this, _: &KeyDownEvent, window, cx| {
                let composer = this.input.read(cx).focus_handle(cx).is_focused(window);
                click_log::note(|| format!("gpui key, composer focused {composer}, anything focused {}", window.focused(cx).is_some()));
            }))
            .capture_any_mouse_up(|event: &MouseUpEvent, _, _| click_log::note(|| format!("gpui up {:?} at {:.0},{:.0}", event.button, f32::from(event.position.x), f32::from(event.position.y))))
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
                    .tab_group()
                    .tab_index(1)
                    .capture_any_mouse_down(cx.listener(|this, _, _, _| this.zoom_chat = true))
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
            .children(self.render_settings_panel(window, cx).map(|d| layer::place(Layer::Settings, d)))
            .children(self.render_server_dialog(window, cx).map(|d| layer::place(Layer::ServerDialog, d)))
            .children(self.render_askpass(cx).map(|d| layer::place(Layer::Askpass, d)))
            .children(self.render_login_node_warning(cx).map(|d| layer::place(Layer::LoginNodeWarning, d)))
            .children(self.render_confirm(cx).map(|d| layer::place(Layer::Confirm, d)))
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
    #[cfg(target_os = "linux")]
    if let Some(display) = std::env::var_os("WAYLAND_DISPLAY") {
        // SAFETY: first thing in main, before any other thread exists.
        unsafe {
            std::env::set_var(platform::WAYLAND_DISPLAY_KEPT, display);
            std::env::remove_var("WAYLAND_DISPLAY");
        }
    } else if let Some(display) = std::env::var_os(platform::WAYLAND_DISPLAY_KEPT).filter(|_| std::env::args().nth(1).as_deref() == Some(runtime::HELPER_FLAG)) {
        // SAFETY: first thing in main, before any other thread exists.
        unsafe { std::env::set_var("WAYLAND_DISPLAY", display) };
    }
    signin::browser_shim();
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some(agent_job::FLAG) {
        agent_job::run(std::env::args().skip(2).collect());
    }
    if std::env::args().nth(1).as_deref() == Some(runtime::HELPER_FLAG) {
        endeavor_mcp::run_as(&[runtime::HELPER_FLAG], std::env::args().skip(2).collect());
    }
    if std::env::args().nth(1).as_deref() == Some(runtime::STOP_FLAG) {
        runtime::stop_recorded();
        std::process::exit(0);
    }
    if [wire::askpass::ADDRESS_ENV, wire::askpass::SOCKET_ENV].iter().any(|v| std::env::var_os(v).is_some()) {
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
        cx.bind_keys([
            KeyBinding::new("escape", Interrupt, None),
            KeyBinding::new(crate::platform::keys!("shift-e"), ToggleAnnotation, None),
            KeyBinding::new(crate::platform::keys!("e"), ReplyToSelection, Some("Input")),
            KeyBinding::new(crate::platform::keys!("e"), ReplyToSelection, None),
            KeyBinding::new(crate::platform::keys!("j"), ReplyToSelection, Some("Input")),
            KeyBinding::new(crate::platform::keys!("j"), ReplyToSelection, None),
            KeyBinding::new("shift-tab", CycleMode, Some("Composer > Input")),
            KeyBinding::new("shift-tab", CycleMode, Some("ModeMenu > Input")),
            KeyBinding::new("shift-tab", CycleMode, Some("MentionList > Input")),
            KeyBinding::new("shift-tab", CycleMode, Some("SlashList > Input")),
            KeyBinding::new(crate::platform::keys!("b"), ToggleSidebar, Some("Input")),
            KeyBinding::new(crate::platform::keys!("b"), ToggleSidebar, None),
            KeyBinding::new(crate::platform::keys!("n"), NewSession, Some("Input")),
            KeyBinding::new(crate::platform::keys!("n"), NewSession, None),
            KeyBinding::new(crate::platform::keys!(","), OpenSettings, None),
            KeyBinding::new(crate::platform::keys!("f"), FindSetting, None),
            KeyBinding::new(crate::platform::keys!("g"), FindNext, None),
            KeyBinding::new(crate::platform::keys!("shift-g"), FindPrevious, None),
            KeyBinding::new(crate::platform::keys!("q"), Quit, None),
            KeyBinding::new(crate::platform::keys!("m"), Minimize, None),
            KeyBinding::new(crate::platform::keys!("="), ZoomIn, None),
            KeyBinding::new(crate::platform::keys!("-"), ZoomOut, None),
            KeyBinding::new(crate::platform::keys!("0"), ZoomReset, None),
        ]);
        cx.bind_keys(composer::key_bindings());
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenHelp, cx| crate::platform::open_url(about::HELP, cx));
        cx.on_action(|_: &SendFeedback, cx| crate::platform::open_url(&about::feedback_url(crate::settings::Settings::load().agent), cx));
        cx.on_action(|_: &ReportIssue, cx| crate::platform::open_url(&about::github_feedback_url(crate::settings::Settings::load().agent), cx));
        cx.on_action(|_: &BringAllToFront, cx| platform::bring_all_to_front(cx));
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
                    MenuItem::action("Point", ToggleAnnotation),
                    MenuItem::separator(),
                    MenuItem::action("Zoom In", ZoomIn),
                    MenuItem::action("Zoom Out", ZoomOut),
                    MenuItem::action("Actual Size", ZoomReset),
                ],
            ),
            menu(
                "Window",
                vec![
                    MenuItem::action("Minimize", Minimize),
                    MenuItem::action("Zoom", ZoomWindow),
                    MenuItem::separator(),
                    MenuItem::action("Bring All to Front", BringAllToFront),
                ],
            ),
            menu("Help", vec![MenuItem::action("Endeavor Help", OpenHelp), MenuItem::action("Send Feedback…", SendFeedback), MenuItem::action("Report an Issue on GitHub…", ReportIssue)]),
        ]);
        let bounds = Bounds::centered(None, size(px(1560.), px(900.)), cx);
        let main_window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(WINDOW_MIN),
                titlebar: Some(TitlebarOptions {
                    title: Some("Endeavor".into()),
                    appears_transparent: !cfg!(windows),
                    traffic_light_position: Some(point(px(16.), px(16.))),
                }),
                app_id: cfg!(target_os = "linux").then(|| "endeavor".into()),
                ..Default::default()
            },
            |window, cx| {
                #[cfg(debug_assertions)]
                if let Some(preview) = splash::preview::open(cx) {
                    return cx.new(|cx| Root::new(preview, window, cx));
                }
                let workspace = cx.new(|cx| Workspace::new(window, cx));
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    false
                });
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ShowAbout, cx| about::open_about(ws.clone(), cx));
                let (ws, handle) = (workspace.downgrade(), window.window_handle());
                cx.on_action(move |_: &OpenSettings, cx| {
                    let ws = ws.clone();
                    cx.defer(move |cx| drop(handle.update(cx, |_, window, cx| ws.update(cx, |this, cx| this.open_settings_last(window, cx)))));
                });
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ZoomIn, cx| drop(ws.update(cx, |this, cx| this.zoom(1.1, false, None, cx))));
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ZoomOut, cx| drop(ws.update(cx, |this, cx| this.zoom(1. / 1.1, false, None, cx))));
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ZoomReset, cx| drop(ws.update(cx, |this, cx| this.zoom(1., true, None, cx))));
                let ws = workspace.downgrade();
                cx.on_action(move |_: &ToggleSidebar, cx| {
                    ws.update(cx, |this, cx| {
                        if this.missing_files.is_some() {
                            return;
                        }
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
        cx.on_window_closed(move |cx, closed| {
            if closed == main_window {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

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
    let ui = Theme::global_mut(cx);
    let base = if light { &ui.light_theme } else { &ui.dark_theme };
    let mut colors = base.colors.clone();
    colors.border = Some(theme::hex(theme::border()));
    colors.link = Some(theme::hex(theme::accent_text()));
    colors.muted = Some(theme::hex(theme::bg_card()));
    colors.table_head = Some(theme::hex(theme::bg_card()));
    colors.table_head_foreground = Some(theme::hex(theme::text_muted()));
    colors.ring = Some(theme::hex(theme::focus_ring()));
    colors.foreground = Some(theme::hex(theme::text_primary()));
    colors.popover_foreground = Some(theme::hex(theme::text_primary()));
    if light {
        colors.popover = Some(theme::hex(theme::popover_bg()));
    }
    let mut config: ThemeConfig = (**base).clone();
    config.font_family = Some(theme::SANS.into());
    config.mono_font_family = Some(theme::MONO.into());
    config.mono_font_size = Some(f32::from(theme::size_code()));
    config.colors = colors;
    let config = std::rc::Rc::new(config);
    if light {
        ui.light_theme = config;
    } else {
        ui.dark_theme = config;
    }
    Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
    cx.refresh_windows();
    light
}

fn outside_link(url: &str) -> Option<String> {
    use url::Host;
    let url = url::Url::parse(url).ok()?;
    match url.scheme() {
        "mailto" => {}
        "http" | "https" => {
            let local = match url.host()? {
                Host::Ipv4(ip) => ip.is_loopback() || ip.is_unspecified(),
                Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback() || v4.is_unspecified()),
                Host::Domain(name) => {
                    let name = name.trim_end_matches('.').to_ascii_lowercase();
                    name == "localhost" || name.ends_with(".localhost")
                }
            };
            if local {
                return None;
            }
        }
        _ => return None,
    }
    Some(url.into())
}

#[cfg(test)]
mod tests {

    #[test]
    fn only_web_links_leave_for_the_browser() {
        let out = |url: &str| super::outside_link(url).is_some();
        assert!(out("https://julialang.org/learning/"));
        assert!(out("http://example.com"));
        assert!(out("mailto:someone@example.com"));
        assert_eq!(super::outside_link("HTTPS://Example.COM/a b").as_deref(), Some("https://example.com/a%20b"), "the browser gets the parsed form");
        for local in [
            "http://127.0.0.1:1234/edit?id=x",
            "http://localhost:1234/",
            "http://LOCALHOST:1234/",
            "http://localhost.:1234/",
            "http://a.localhost:1234/",
            "http://[::1]:1234/",
            "http://0.0.0.0:1234/",
            "http://[::]:1234/",
            "http://[::ffff:127.0.0.1]:1234/",
            "http://[::ffff:7f00:1]:1234/",
            "http://127.0.0.2:1234/",
            "http://127.1:1234/",
            "http://2130706433:1234/",
            "http://127.0.0.1\\@evil.com/",
            "http://evil.com@127.0.0.1:1234/",
        ] {
            assert!(!out(local), "{local} is the notebook's own computer");
        }
        assert!(!out("data:image/png;base64,iVBOR"));
        assert!(!out("blob:http://127.0.0.1:1234/3f2a"));
        assert!(!out("file:///etc/passwd"));
        assert!(!out("javascript:alert(1)"));
        assert!(!out("not a url"));
    }
    use super::{load_json_at, save_json_at, viewed_notebook, viewed_notebook_id, write_atomic};
    use wire::backend::Backend;
    use std::collections::HashMap;

    #[test]
    fn a_folder_is_renamed_and_a_missing_one_fails_at_once() {
        let dir = std::env::temp_dir().join(format!("endeavor-test-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (from, to) = (dir.join("adapter.installing"), dir.join("adapter"));
        std::fs::create_dir_all(from.join("node_modules/x")).unwrap();
        std::fs::write(from.join("node_modules/x/index.js"), "1").unwrap();
        super::rename_folder(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(to.join("node_modules/x/index.js")).unwrap(), "1");
        let started = std::time::Instant::now();
        assert!(super::rename_folder(&from, &dir.join("other")).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_held_folder_is_renamed_once_it_is_let_go() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("endeavor-test-held-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (from, to) = (dir.join("adapter.installing"), dir.join("adapter"));
        std::fs::create_dir_all(from.join("node_modules")).unwrap();
        std::fs::write(from.join("node_modules/index.js"), "1").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(from.join("node_modules/index.js")).unwrap();
        assert!(std::fs::rename(&from, &to).is_err(), "the open file should stop a plain rename");
        let letting_go = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(held);
        });
        let started = std::time::Instant::now();
        super::rename_folder(&from, &to).unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_millis(250), "renamed before the file was let go");
        letting_go.join().unwrap();
        assert_eq!(std::fs::read_to_string(to.join("node_modules/index.js")).unwrap(), "1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notebook_id_from_pluto_url() {
        let id = "6a1b2c3d-0000-4000-8000-1234567890ab";
        let url = format!("http://127.0.0.1:1234/edit?token=t0k3n&id={id}");
        assert_eq!(viewed_notebook_id(&url), Some(id));
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/?token=t0k3n"), None);
        assert_eq!(viewed_notebook_id("http://127.0.0.1:1234/edit?id=../../secret"), None);
    }

    #[test]
    fn an_ember_page_shows_an_r_notebook() {
        let id = "6a1b2c3d-0000-4000-8000-1234567890ab";
        assert_eq!(viewed_notebook(&format!("http://127.0.0.1:1234/ember/edit?id={id}")), Some((Backend::Ember, id)));
        assert_eq!(viewed_notebook(&format!("http://127.0.0.1:1234/edit?id={id}")), Some((Backend::Pluto, id)));
        assert_eq!(viewed_notebook("http://127.0.0.1:1234/ember/edit?id=../x"), None);
    }

    #[test]
    fn a_marimo_page_shows_a_python_notebook_by_its_path() {
        assert_eq!(super::viewed_kind("http://127.0.0.1:1234/marimo/?file=/w/fit.py&token=t"), Some(Backend::Marimo));
        assert_eq!(viewed_notebook("http://127.0.0.1:1234/marimo/?file=/w/fit.py"), None, "its id comes from the runtime's list, by path");
        assert_eq!(super::viewed_kind("http://127.0.0.1:1234/edit?id=6a1b2c3d-0000-4000-8000-1234567890ab"), Some(Backend::Pluto));
    }

    #[test]
    fn a_new_r_or_python_session_tells_the_agent_which_notebook_to_make() {
        assert_eq!(super::chosen_kind_note(Backend::Pluto), None);
        assert!(super::chosen_kind_note(Backend::Ember).is_some_and(|note| note.contains("R notebook") && note.contains(".R path")));
        assert!(super::chosen_kind_note(Backend::Marimo).is_some_and(|note| note.contains("Python (marimo) notebook") && note.contains(".py path")));
    }

    #[test]
    fn pending_moved_note_survives_a_restart_and_clears_once_sent() {
        let dir = std::env::temp_dir().join(format!("endeavor-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        let file = dir.join("pending-context.json");

        let mut pending: HashMap<String, String> = HashMap::new();
        pending.insert("session-1".into(), "[Endeavor] The session's notebook file moved from a.jl to b.jl.".into());
        save_json_at(&file, &pending);

        let restored: HashMap<String, String> = load_json_at(&file);
        assert_eq!(restored.get("session-1").map(String::as_str), Some("[Endeavor] The session's notebook file moved from a.jl to b.jl."));

        let mut after_send = restored;
        after_send.remove("session-1");
        save_json_at(&file, &after_send);
        let reloaded: HashMap<String, String> = load_json_at(&file);
        assert!(reloaded.is_empty(), "sent and saved: nothing left to resend after another restart");

        let _ = std::fs::remove_dir_all(&dir);
    }

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
