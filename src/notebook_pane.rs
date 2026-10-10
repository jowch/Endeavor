//! The notebook pane around Pluto's page: its header (the notebook system's
//! logo, the file, where it runs, what state it's in, then Point, Share, Live
//! docs, Status and ⋮), the Share and ⋮ menus' actions, and the pages drawn
//! natively when there's no notebook to show (docs/ui-spec.md, "Notebook
//! (Pluto)"). In the Pluto classic look the header keeps only the logo, file,
//! host, Point and ⋮, since Pluto's own page has the rest.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::annotate::PageState;
use crate::connection::HostPane;
use crate::hosts::{HostId, Place};
use crate::new_session::{self, Glyph, glyph};
use crate::resources::Target;
use crate::session::{Effect, Session, Stopped, folder_name};
use crate::settings::NotebookTheme;
use crate::overlay;
use crate::menu::MenuTarget;
use crate::notice::{Notice, Retry, Spot};
use crate::{Workspace, platform, pluto, theme};

/// An item in the notebook's ⋮ menu or its Share menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NotebookAction {
    CopyPath,
    Reveal,
    Rename,
    MoveTo,
    LookEndeavor,
    LookClassic,
    Shortcuts,
    NewSession,
    Feedback,
    Start,
    Restart,
    Stop,
    ExportFile,
    ExportHtml,
    ExportPdf,
    Present,
    Record,
    Frontmatter,
}

impl NotebookAction {
    pub fn label(self) -> &'static str {
        match self {
            NotebookAction::CopyPath => "Copy path",
            NotebookAction::Reveal => crate::platform::REVEAL,
            NotebookAction::Rename => "Rename…",
            NotebookAction::MoveTo => "Move to…",
            NotebookAction::LookEndeavor => "Endeavor",
            NotebookAction::LookClassic => "Pluto classic",
            NotebookAction::Shortcuts => "Keyboard shortcuts",
            NotebookAction::NewSession => "Open in a new session…",
            NotebookAction::Feedback => "Send feedback to Pluto's developers…",
            NotebookAction::Start => "Start notebook",
            NotebookAction::Restart => "Restart notebook",
            NotebookAction::Stop => "Stop notebook",
            NotebookAction::ExportFile => "Notebook file…",
            NotebookAction::ExportHtml => "Static HTML…",
            NotebookAction::ExportPdf => "PDF…",
            NotebookAction::Present => "Present",
            NotebookAction::Record => "Record…",
            NotebookAction::Frontmatter => "Frontmatter…",
        }
    }

    /// (key, what the menu shows) for the item's single-key shortcut.
    pub fn shortcut(self) -> (&'static str, &'static str) {
        match self {
            NotebookAction::CopyPath => ("c", "C"),
            NotebookAction::Reveal => ("f", "F"),
            NotebookAction::Rename => ("r", "R"),
            NotebookAction::MoveTo => ("m", "M"),
            NotebookAction::LookEndeavor => ("e", "E"),
            NotebookAction::LookClassic => ("p", "P"),
            NotebookAction::Shortcuts => ("f1", "F1"),
            NotebookAction::NewSession => ("n", "N"),
            NotebookAction::Feedback => ("b", ""),
            NotebookAction::Start => ("s", "S"),
            NotebookAction::Restart => ("t", ""),
            NotebookAction::Stop => ("s", "S"),
            NotebookAction::ExportFile => ("j", ""),
            NotebookAction::ExportHtml => ("h", ""),
            NotebookAction::ExportPdf => ("d", ""),
            NotebookAction::Present => ("p", ""),
            NotebookAction::Record => ("r", ""),
            NotebookAction::Frontmatter => ("m", ""),
        }
    }

    pub(crate) fn glyph(self) -> Option<Glyph> {
        Some(match self {
            NotebookAction::CopyPath => Glyph::Copy,
            NotebookAction::Reveal => Glyph::Folder,
            NotebookAction::Rename => Glyph::Pencil,
            NotebookAction::MoveTo => Glyph::Folder,
            NotebookAction::Shortcuts => Glyph::Keyboard,
            NotebookAction::NewSession => Glyph::Cells,
            NotebookAction::Feedback => Glyph::Bubble,
            NotebookAction::Start => Glyph::Play,
            NotebookAction::Restart => Glyph::Restart,
            NotebookAction::Stop => Glyph::Stop,
            NotebookAction::ExportFile => Glyph::File,
            NotebookAction::ExportHtml => Glyph::Code,
            NotebookAction::ExportPdf => Glyph::File,
            NotebookAction::Present => Glyph::Screen,
            NotebookAction::Record => Glyph::Record,
            NotebookAction::Frontmatter => Glyph::Tag,
            NotebookAction::LookEndeavor | NotebookAction::LookClassic => return None,
        })
    }

    /// A second line under the label.
    pub fn detail(self) -> Option<&'static str> {
        match self {
            NotebookAction::NewSession => Some("The same notebook, a new conversation"),
            NotebookAction::ExportFile => Some("A copy of the .jl file"),
            NotebookAction::ExportHtml => Some("A web page with the outputs"),
            NotebookAction::ExportPdf => Some("For printing or email"),
            _ => None,
        }
    }

    /// Items in one group sit together; a separator comes between groups.
    pub fn group(self) -> u8 {
        match self {
            NotebookAction::CopyPath | NotebookAction::Reveal | NotebookAction::Rename | NotebookAction::MoveTo => 0,
            NotebookAction::LookEndeavor | NotebookAction::LookClassic => 1,
            NotebookAction::Shortcuts => 2,
            NotebookAction::NewSession | NotebookAction::Feedback => 3,
            NotebookAction::Start | NotebookAction::Restart | NotebookAction::Stop => 4,
            NotebookAction::ExportFile | NotebookAction::ExportHtml | NotebookAction::ExportPdf => 5,
            NotebookAction::Present | NotebookAction::Record | NotebookAction::Frontmatter => 6,
        }
    }

    /// A small heading over the group this item starts.
    pub fn section(self) -> Option<&'static str> {
        match self {
            NotebookAction::LookEndeavor => Some("Notebook look"),
            NotebookAction::ExportFile => Some("Export"),
            _ => None,
        }
    }

    pub fn danger(self) -> bool {
        self == NotebookAction::Stop
    }

    /// The ⋮ menu. Finder and the folder picker only reach This Mac's files;
    /// Restart and Stop need a running notebook, and Restart isn't offered in
    /// safe preview, where Run notebook is the way to start it. A stopped
    /// notebook offers Start, the pane's own Start button's only other way in.
    pub fn for_notebook(open: bool, stopped: bool, safe: bool, local: bool) -> Vec<NotebookAction> {
        use NotebookAction::*;
        let mut items = vec![CopyPath];
        if local {
            items.push(Reveal);
        }
        if open {
            items.push(Rename);
            if local {
                items.push(MoveTo);
            }
        }
        items.extend([LookEndeavor, LookClassic, Shortcuts, NewSession, Feedback]);
        if stopped {
            items.push(Start);
        }
        if open && !safe {
            items.push(Restart);
        }
        if open {
            items.push(Stop);
        }
        items
    }

    pub fn for_share() -> Vec<NotebookAction> {
        use NotebookAction::*;
        vec![ExportFile, ExportHtml, ExportPdf, Present, Record, Frontmatter]
    }
}

/// What the notebook pane shows for a session: a page drawn natively, or the
/// notebook's own page in the web view.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneShows {
    /// Its host isn't ready.
    Host(HostPane),
    /// "Can't find" its file.
    Missing,
    /// "No notebook in this session yet".
    NoNotebook,
    Stopped(Stopped),
    /// Its Julia stopped by itself: "Julia stopped unexpectedly", Restart Julia.
    Crashed,
    /// "Opening <file>…".
    Opening,
    Page,
}

/// How long the pane may say "Opening" for a notebook its runtime has open
/// before the app loads the page again. A page starts loading as soon as it's
/// asked for, so by then it isn't coming.
pub const OPENING_GRACE: Duration = Duration::from_secs(5);

/// Watches for the pane saying "Opening" for a notebook that's open in its
/// runtime: it loads the page again once, then offers Reload notebook.
#[derive(Debug, Default)]
pub struct OpeningWatch {
    /// The session whose pane is stuck, and since when (or since the reload).
    since: Option<(u64, Instant)>,
    reloaded: bool,
    /// A reload didn't help: the pane shows Reload notebook.
    pub stuck: bool,
}

#[derive(Debug, PartialEq)]
pub enum OpeningStep {
    Wait,
    /// Load the page again.
    Reload,
    /// Still stuck after a reload: show Reload notebook.
    Stuck,
}

impl OpeningWatch {
    /// Called every second. `stuck`: the session whose pane shows "Opening"
    /// while its runtime has its notebook open; None when no pane is.
    pub fn step(&mut self, stuck: Option<u64>, now: Instant) -> OpeningStep {
        let Some(key) = stuck else {
            *self = OpeningWatch::default();
            return OpeningStep::Wait;
        };
        let since = match self.since {
            Some((k, since)) if k == key => since,
            _ => {
                *self = OpeningWatch { since: Some((key, now)), ..OpeningWatch::default() };
                return OpeningStep::Wait;
            }
        };
        if self.stuck || now.duration_since(since) < OPENING_GRACE {
            return OpeningStep::Wait;
        }
        if self.reloaded {
            self.stuck = true;
            return OpeningStep::Stuck;
        }
        self.reloaded = true;
        self.since = Some((key, now));
        OpeningStep::Reload
    }

    /// Reload notebook was clicked: it waits again before offering it again.
    pub fn retry(&mut self, now: Instant) {
        if let Some((key, _)) = self.since {
            self.since = Some((key, now));
        }
        self.stuck = false;
    }

    /// For the state dump: how long the pane has said "Opening" (since the
    /// last reload), whether it was reloaded, and whether the button shows.
    #[cfg(debug_assertions)]
    pub fn state(&self, now: Instant) -> Option<(u64, bool, bool)> {
        let (_, since) = self.since?;
        Some((now.duration_since(since).as_secs(), self.reloaded, self.stuck))
    }
}

/// `ENDEAVOR_TEST_STUCK_OPENING` (debug builds): a file path. When the file
/// appears, the page is taken down as a runtime going away does, and the pane
/// says "Opening". While the file says `keep`, loading a notebook loads a
/// blank page instead, so it stays stuck. Some(true) for `keep`.
#[cfg(debug_assertions)]
pub fn test_stuck_opening() -> Option<bool> {
    let path = std::env::var_os("ENDEAVOR_TEST_STUCK_OPENING")?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.trim() == "keep")
}

/// A stopped notebook's page: why it stopped, and what Start does if the file changed.
fn stopped_text(stopped: Stopped) -> (String, Option<&'static str>) {
    if stopped.safe_preview {
        return ("The file is saved. It was in safe preview, so Start opens it that way again.".to_string(), None);
    }
    let why = match stopped.idle_hours {
        Some(hours) => format!("It stopped after {hours} hours without use. The file is saved; Start runs it again from the top."),
        None => "The file is saved; Start runs it again from the top.".to_string(),
    };
    (why, Some("If the file changed on disk meanwhile, it opens in safe preview instead."))
}

/// The notebook header's parts that say what state the notebook is in.
pub struct HeaderInfo {
    /// Where it runs: "Local", a server's name, or a cluster's with its job.
    pub host: String,
    pub tags: Vec<HeaderTag>,
    /// Work under way, e.g. "Installing packages · 2 of 5".
    pub busy: Option<String>,
}

/// A tag after the host in the notebook header.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeaderTag {
    ReadOnly,
    NotFound,
    Stopped,
    SafePreview,
    NotSaved,
    PackageFailed,
    RestartNeeded,
    RestartRecommended,
    JuliaStopped,
}

impl HeaderTag {
    pub fn label(self) -> &'static str {
        match self {
            HeaderTag::ReadOnly => "Read-only",
            HeaderTag::NotFound => "Not found",
            HeaderTag::Stopped => "Stopped",
            HeaderTag::SafePreview => "Safe preview",
            HeaderTag::NotSaved => "Not saved",
            HeaderTag::PackageFailed => "Package failed",
            HeaderTag::RestartNeeded => "Restart needed",
            HeaderTag::RestartRecommended => "Restart recommended",
            HeaderTag::JuliaStopped => "Julia stopped",
        }
    }
}

/// The Pluto logo: three dots, stacked.
fn pluto_logo() -> impl IntoElement {
    div().flex().flex_col().gap(px(1.)).flex_shrink_0().children([rgb(0x3b972e), rgb(0x945bb0), rgb(0xc93d39)].map(|c| div().size(px(4.)).rounded_full().bg(c)))
}

/// A small tag in the header ("Local", "Safe preview", "Not saved").
fn chip(icon: Option<Glyph>, text: impl Into<SharedString>, color: Rgba) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(4.))
        .h(px(18.))
        .px(px(6.))
        .rounded(px(3.))
        .bg(theme::bg_tag())
        .text_size(theme::size_meta_small())
        .text_color(color)
        .children(icon.map(|g| glyph(g, color)))
        .child(text.into())
}

/// A header button's tooltip was laid out in the frame being drawn.
static TOOLTIP_DRAWN: AtomicBool = AtomicBool::new(false);

/// Closes the tooltips' hole in a frame that draws no tooltip. The hole can't
/// close when the tooltip's view goes: GPUI hides a tooltip while drawing a
/// frame and keeps its view until it draws the next one, which may be much later.
/// The root view draws this in every frame, after every tooltip is laid out.
pub fn tooltip_hole_keeper(webview: &Entity<gpui_wry::WebView>) -> impl IntoElement {
    let webview = webview.clone();
    canvas(
        |_, _, _| (),
        move |_, _, _, cx| {
            if !TOOLTIP_DRAWN.swap(false, Ordering::Relaxed) {
                overlay::set_hole(webview.read(cx).raw(), overlay::Hole::Tooltip, None);
            }
        },
    )
    .absolute()
    .size_0()
}

/// A header button's tooltip. It hangs over the notebook, whose web view (a
/// native view on top of what GPUI draws) gets a hole there, as for menus.
struct PaneTooltip {
    text: SharedString,
    webview: Entity<gpui_wry::WebView>,
}

pub fn tooltip(text: impl Into<SharedString>, webview: &Entity<gpui_wry::WebView>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    let webview = webview.clone();
    move |_, cx| cx.new(|_| PaneTooltip { text: text.clone(), webview: webview.clone() }).into()
}

impl Render for PaneTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let webview = self.webview.clone();
        let cut = canvas(
            move |bounds, _, cx| {
                let webview = webview.read(cx);
                let rect = Bounds { origin: bounds.origin - webview.bounds().origin, size: bounds.size };
                TOOLTIP_DRAWN.store(true, Ordering::Relaxed);
                overlay::set_hole(webview.raw(), overlay::Hole::Tooltip, Some(rect));
            },
            |_, _, _, _| (),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div().p(px(8.)).child(
            div()
                .relative()
                .px(px(8.))
                .py(px(3.))
                .rounded(px(5.))
                .border_1()
                .border_color(theme::composer_edge())
                .bg(theme::bg_raised())
                .font_family(theme::SANS)
                .text_size(theme::size_meta())
                .text_color(theme::text_primary())
                .child(self.text.clone())
                .child(cut),
        )
    }
}

/// A 24px header button: an icon, maybe a label; lit while its panel or menu is open.
/// `compact`: icon only (the label stays its accessible name), for a narrow pane.
/// The notebook pane's width from which its header buttons show their labels.
const HEADER_LABELS_MIN: f32 = 680.;

fn header_button(id: &'static str, icon: Glyph, label: Option<&'static str>, active: bool, compact: bool) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.unwrap_or(id))
        .relative()
        .flex_shrink_0()
        .h(px(24.))
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(6.))
        .rounded(px(4.))
        .cursor_pointer()
        .text_size(theme::size_meta())
        .text_color(if active { theme::text_primary() } else { theme::text_muted() })
        .when(active, |d| d.bg(theme::row_active()))
        .hover(|s| s.bg(theme::row_active()).text_color(theme::text_primary()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(glyph(icon, if active { theme::text_primary() } else { theme::text_muted() }))
        .children(label.filter(|_| !compact))
}

/// A button on the empty pages: filled orange for the one thing to do, else outlined.
fn page_button(id: &'static str, icon: Glyph, label: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .px(px(10.))
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(theme::size_body())
        .map(|d| {
            if primary {
                d.bg(theme::accent()).text_color(gpui::white()).child(glyph(icon, gpui::white().into()))
            } else {
                d.border_1().border_color(theme::control_edge()).text_color(theme::text_primary()).hover(|s| s.bg(theme::row_active())).child(glyph(icon, theme::text_muted()))
            }
        })
        .child(label.into())
}

fn page_title(text: impl IntoElement) -> Div {
    div().text_size(theme::size_subhead()).text_color(theme::text_primary()).child(text)
}

fn page_text(text: impl Into<SharedString>) -> Div {
    div().max_w(px(420.)).text_center().text_color(theme::text_muted()).child(text.into())
}

impl Workspace {
    /// Where a session's notebook runs, for its header: "Local", the server's
    /// name, or a cluster's with its job ("hoffman2 · job 16").
    pub fn host_label(&self, host: &HostId) -> String {
        match host {
            HostId::ThisMac => "Local".into(),
            HostId::Server(_) => {
                let name = self.hosts.name(host);
                let job = self.connection(host).and_then(|c| c.runtime.as_ref()?.job.as_ref().map(|j| j.id.clone()));
                match job {
                    Some(job) => format!("{name} · job {job}"),
                    None => name,
                }
            }
        }
    }

    /// The page's state for this session's notebook, if it's the one shown.
    fn page_for(&self, session: &Session) -> Option<&PageState> {
        let id = session.notebook.as_deref()?;
        (self.page.notebook == id).then_some(&self.page)
    }

    /// The notebook header's host, tags and work under way for a session's
    /// notebook. `shown`: its notebook is on screen.
    pub fn header_info(&self, session: &Session, shown: bool) -> HeaderInfo {
        let endeavor = self.settings.notebook_theme == NotebookTheme::Endeavor;
        let page = self.page_for(session).filter(|_| shown);
        let host = self.host_label(&session.place.host);
        let read_only = self.read_only(session);
        let reconnecting = page.is_some_and(|p| !p.connected) && !read_only;
        let mut tags = Vec::new();
        if read_only {
            tags.push(HeaderTag::ReadOnly);
        }
        let crashed = self.notebook_at(session.key).is_some_and(|at| self.crashes.stopped_at(&at).is_some())
            || self.connection(&session.place.host).is_some_and(|c| c.crashed && matches!(c.status, crate::connection::Status::Died(_)));
        if session.missing {
            tags.push(HeaderTag::NotFound);
        } else if session.stopped.is_some() {
            tags.push(HeaderTag::Stopped);
        } else if crashed {
            tags.push(HeaderTag::JuliaStopped);
        }
        let page = page.filter(|_| endeavor);
        if let Some(p) = page {
            if p.safe {
                tags.push(HeaderTag::SafePreview);
            }
            if p.save_failed {
                tags.push(HeaderTag::NotSaved);
            }
            if p.package_failed.is_some() {
                tags.push(HeaderTag::PackageFailed);
            } else if let Some(restart) = &p.restart {
                tags.push(if restart == "required" { HeaderTag::RestartNeeded } else { HeaderTag::RestartRecommended });
            } else if p.dead && !crashed {
                tags.push(HeaderTag::JuliaStopped);
            }
        }
        HeaderInfo {
            host: if reconnecting { format!("{host} · reconnecting") } else { host },
            tags,
            // What's under way, while the drawer's Status tab isn't showing it.
            busy: page.filter(|p| p.drawer.as_deref() != Some("status")).and_then(|p| p.busy.clone()),
        }
    }

    /// The notebook pane's header for session `ix`. `shown`: its notebook is on screen (Point works).
    /// `width`: the notebook pane's; below `HEADER_LABELS_MIN` its buttons show icons only.
    pub fn notebook_header(&self, ix: usize, shown: bool, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let compact = width < HEADER_LABELS_MIN;
        let session = &self.sessions[ix];
        let key = session.key;
        if session.notebook_path.is_none() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .text_size(theme::size_meta())
                .text_color(theme::text_muted())
                .child("No notebook")
                .child(div().flex_1())
                .children(self.job_ends(&session.place.host))
                .into_any_element();
        }
        let endeavor = self.settings.notebook_theme == NotebookTheme::Endeavor;
        let page = self.page_for(session).filter(|_| shown);
        let header = self.header_info(session, shown);
        let host_icon = match &session.place.host {
            HostId::ThisMac => Glyph::Laptop,
            HostId::Server(_) if self.is_cluster(&session.place.host) => Glyph::Cluster,
            HostId::Server(_) => Glyph::Server,
        };
        let host_chip = chip(Some(host_icon), header.host, theme::text_tag());

        let name = match &self.notebook_rename {
            Some((renaming, input)) if *renaming == key => div()
                .flex()
                .items_center()
                .gap(px(2.))
                .font_family(theme::MONO)
                .text_size(theme::size_code())
                .child(
                    div()
                        .w(px(180.))
                        .h(px(22.))
                        .px(px(4.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme::accent())
                        .capture_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                            if e.keystroke.key == "escape" {
                                cx.stop_propagation();
                                this.notebook_rename = None;
                                cx.notify();
                            }
                        }))
                        .child(Input::new(input).appearance(false).text_size(theme::size_code())),
                )
                .child(div().text_color(theme::text_faint()).child(".jl"))
                .into_any_element(),
            _ => div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(theme::MONO)
                .text_size(theme::size_code())
                .text_color(if session.missing { theme::text_faint() } else { theme::text_primary() })
                .child(session.notebook_path.as_deref().map(|p| folder_name(Path::new(p))).unwrap_or_default())
                .into_any_element(),
        };

        let chips = header.tags.into_iter().map(|tag| {
            let label = tag.label();
            match tag {
                HeaderTag::ReadOnly => chip(Some(Glyph::Lock), label, theme::text_tag()).into_any_element(),
                HeaderTag::NotFound | HeaderTag::NotSaved => chip(Some(Glyph::Warning), label, theme::danger()).into_any_element(),
                HeaderTag::Stopped => chip(None, label, theme::text_faint()).into_any_element(),
                HeaderTag::SafePreview => chip(Some(Glyph::Shield), label, theme::accent_text()).into_any_element(),
                HeaderTag::PackageFailed => chip(Some(Glyph::Warning), label, theme::danger())
                    .id("package-failed")
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| this.open_drawer(Some("status"), cx)))
                    .into_any_element(),
                HeaderTag::RestartNeeded | HeaderTag::RestartRecommended => chip(Some(Glyph::Restart), label, theme::accent_text())
                    .id("restart-needed")
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| this.restart_notebook(key, cx)))
                    .into_any_element(),
                HeaderTag::JuliaStopped => chip(Some(Glyph::Warning), label, theme::danger()).into_any_element(),
            }
        });
        let chips: Vec<AnyElement> = chips.collect();

        let busy = header.busy.map(|text| {
            div()
                .id("header-busy")
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .mr(px(4.))
                .cursor_pointer()
                .text_size(theme::size_meta())
                .text_color(theme::text_muted())
                .hover(|s| s.text_color(theme::text_primary()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| this.open_drawer(Some("status"), cx)))
                .child(div().size(px(6.)).rounded_full().bg(theme::accent()))
                .child(text)
        });

        let open = |target: MenuTarget| self.menu.as_ref().is_some_and(|m| m.target == target);
        let point_tip = self.point_tip_shows(shown);
        let tip = |d: Stateful<Div>, text: SharedString| d.tooltip(tooltip(text, &self.webview));
        let point_text = format!("Pick cells or draw a box to ask {} about  {}", session.agent.name(), crate::platform::shortcut!(shift "E"));
        let point = tip(header_button("header-point", Glyph::Pointer, Some("Point"), self.annotating, compact), point_text.into())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_annotation(&crate::ToggleAnnotation, window, cx)))
            .when(point_tip, |d| d.child(self.render_point_tip(if endeavor { 120. } else { 32. }, session.agent, cx)));
        let drawer = page.and_then(|p| p.drawer.clone());
        let tools = (endeavor && shown).then(|| {
            let share_menu = self.menu.as_ref().filter(|m| m.target == MenuTarget::Share(key));
            [
                header_button("header-share", Glyph::Share, Some("Share and export"), open(MenuTarget::Share(key)), compact)
                    .when(share_menu.is_none(), |d| d.tooltip(tooltip("Share and export", &self.webview)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_menu(MenuTarget::Share(key), None, window, cx);
                    }))
                    .children(share_menu.map(|menu| self.render_menu(menu, cx)))
                    .into_any_element(),
                tip(header_button("header-docs", Glyph::Book, Some("Live docs"), drawer.as_deref() == Some("docs"), compact), "Live docs".into())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_drawer("docs", cx)))
                    .into_any_element(),
                tip(header_button("header-status", Glyph::Pulse, Some("Status"), drawer.as_deref() == Some("status"), compact), "Status".into())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_drawer("status", cx)))
                    .into_any_element(),
            ]
        });

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(pluto_logo())
            .child(name)
            .child(host_chip)
            .children(chips)
            .child(div().flex_1())
            .children(self.job_ends(&session.place.host))
            .children(busy)
            .when(shown, |d| d.child(point))
            .children(tools.into_iter().flatten())
            .when(session.notebook_path.is_some(), |d| d.child(self.notebook_more(key, cx)))
            .into_any_element()
    }

    fn toggle_drawer(&mut self, tab: &str, cx: &mut Context<Self>) {
        let next = (self.page.drawer.as_deref() != Some(tab)).then_some(tab);
        self.open_drawer(next, cx);
    }

    pub fn open_drawer(&mut self, tab: Option<&str>, cx: &mut Context<Self>) {
        self.page.drawer = tab.map(str::to_owned);
        self.send_to_page(&serde_json::json!({ "type": "drawer", "tab": tab }), cx);
        cx.notify();
    }

    /// Tell the page where its notebook runs and whether the agent is asking to
    /// run it, when that changes (the callout and Status say so).
    pub fn sync_page_context(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else { return };
        let host = match &session.place.host {
            HostId::ThisMac => crate::platform::this_computer!().to_string(),
            host => self.hosts.name(host),
        };
        let crash = self.notebook_at(session.key).and_then(|at| self.crashes.callout(&at)).map(|(title, body)| serde_json::json!({ "title": title, "body": body }));
        // The cells the card asks to run, those that re-run after them, and
        // those it needs that never ran (they run first), for their lines in the page.
        let card = crate::approval::approval_view(session);
        // The waiting card, by its entry: ⏎ in the page answers it, and only it.
        let card_ix = card.as_ref().and(session.pending_permission());
        let (ask_cells, rerun_cells, needed_ids) = card.map(|c| (c.cells, c.rerun, c.needed)).unwrap_or_default();
        // Claude is working: the page's prompts queue what they send.
        let working = session.outbox.busy && !session.agent_waiting;
        // Error boxes whose Fix or Explain Claude is answering, or that wait in the queue.
        let error_asks: Vec<_> = session.error_asks().into_iter().map(|(cell, kind, queued)| serde_json::json!({ "cell": cell, "kind": kind, "queued": queued })).collect();
        // Every waiting run card's cells: a run of the user's that reaches one asks first.
        let waiting: Vec<serde_json::Value> = crate::approval::waiting_run_cells(session).into_iter().map(|(id, name)| serde_json::json!({ "id": id, "name": name })).collect();
        let msg = serde_json::json!({ "type": "context", "host": host, "agent": session.agent.name(), "asking": session.asking_to_run(), "readonly": self.read_only(session), "crash": crash, "ask_cells": ask_cells, "rerun_cells": rerun_cells, "needed_ids": needed_ids, "working": working, "error_asks": error_asks, "waiting_runs": waiting, "card": card_ix });
        let text = msg.to_string();
        if text != self.page_context {
            self.page_context = text;
            self.send_to_page(&msg, cx);
        }
    }

    /// The page reported its notebook's state. Leaving safe preview some other
    /// way (Pluto classic's own button) answers a pending "Let this notebook run?".
    pub fn on_page_state(&mut self, state: PageState, cx: &mut Context<Self>) {
        let same = self.page.notebook == state.notebook;
        let left_safe = same && self.page.safe && !state.safe && state.connected;
        let was_dead = same && self.page.dead;
        self.page = state;
        if let Some(key) = self.session_showing(&self.page.notebook) {
            if left_safe {
                self.with_session(key, cx, |s| {
                    if s.asking_to_run() {
                        s.answer_pending(agent_client_protocol::schema::v1::PermissionOptionKind::AllowOnce, crate::session::Scope::Once);
                    }
                });
                // Let run after a second stop: the callout has said its piece.
                if let Some(at) = self.notebook_at(key) {
                    self.crashes.forget_again(&at);
                }
            }
            self.page_process(key, self.page.dead, was_dead, cx);
        }
        cx.notify();
    }

    /// The active session, if it shows notebook `id`.
    fn session_showing(&self, id: &str) -> Option<u64> {
        self.active_session().filter(|s| s.notebook.as_deref() == Some(id)).map(|s| s.key)
    }

    /// Run notebook (the callout): answers the agent's pending card if there is
    /// one, so its call does the running; otherwise the notebook leaves safe preview.
    pub fn run_notebook(&mut self, notebook: String, cx: &mut Context<Self>) {
        let Some(key) = self.session_showing(&notebook) else { return };
        let mut answered = false;
        self.with_session(key, cx, |s| {
            if s.asking_to_run() {
                answered = s.answer_pending(agent_client_protocol::schema::v1::PermissionOptionKind::AllowOnce, crate::session::Scope::Once);
            }
        });
        if answered {
            return;
        }
        let Some(bridge) = self.session_bridge(key) else { return };
        let task = cx.background_executor().spawn({
            let notebook = notebook.clone();
            async move { pluto::allow_execution(&bridge, &notebook) }
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.show_notice(Notice::new(Spot::Pane, "Couldn't run the notebook", &e, Some(Retry::RunNotebook(notebook))), cx);
                });
            }
        })
        .detach();
    }

    /// Run anyway, in the page: the user's run reaches `cells` (each with its
    /// `last_run` from before), which cards ask to run. Those cards are
    /// answered as their Run button does, telling the runtime which cells the
    /// user ran, so the approved calls don't run them again. An older
    /// runtime's agent cards wait until the runtime has heard it on its own.
    /// ⏎ in the page with nothing focused: the filled answer to the card the
    /// page was told about, if it is still the one waiting.
    pub fn answer_card(&mut self, notebook: String, card: usize, cx: &mut Context<Self>) {
        let Some(key) = self.session_showing(&notebook) else { return };
        self.with_session(key, cx, |s| {
            if s.pending_permission() == Some(card) {
                s.answer_pending(agent_client_protocol::schema::v1::PermissionOptionKind::AllowOnce, crate::session::Scope::Once);
            }
        });
    }

    pub fn run_anyway(&mut self, notebook: String, cells: Vec<(String, f64)>, cx: &mut Context<Self>) {
        let Some(key) = self.session_showing(&notebook) else { return };
        let ids: Vec<String> = cells.iter().map(|(id, _)| id.clone()).collect();
        let mut agent = false;
        self.with_session(key, cx, |s| {
            s.allow_runs_of(&cells, false);
            agent = s.agent_asks_to_run(&ids);
        });
        if !agent {
            return;
        }
        let Some(bridge) = self.session_bridge(key) else {
            self.with_session(key, cx, |s| {
                s.allow_runs_of(&cells, true);
            });
            return;
        };
        let told = cells.clone();
        let task = cx.background_executor().spawn(async move { pluto::run_anyway(&bridge, &notebook, &told) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                eprintln!("Run anyway: the runtime didn't hear which cells the user ran: {e}");
            }
            let _ = this.update(cx, |this, cx| {
                this.with_session(key, cx, |s| {
                    s.allow_runs_of(&cells, true);
                });
            });
        })
        .detach();
    }

    pub fn restart_notebook(&mut self, key: u64, cx: &mut Context<Self>) {
        let (Some(bridge), Some(id)) = (self.session_bridge(key), self.sessions.iter().find(|s| s.key == key).and_then(|s| s.notebook.clone())) else { return };
        let task = cx.background_executor().spawn(async move { pluto::restart_notebook(&bridge, &id) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.show_notice(Notice::new(Spot::NotebookRight, "Couldn't restart the notebook", &e, Some(Retry::RestartNotebook(key))), cx);
                });
            }
        })
        .detach();
    }

    /// Status's Fix with Claude for a package that failed: a message with its log.
    pub fn fix_package(&mut self, notebook: String, name: String, log: String, cx: &mut Context<Self>) {
        let Some(key) = self.session_showing(&notebook) else { return };
        let text = format!("The package {name} failed to install or precompile, so the cells that use it can't run. Find out why and fix it.");
        // "[Endeavor]" marks it as context, so a reopened session's transcript shows only the words.
        let context = format!("[Endeavor] Pkg's log for {name} (its end), from the notebook's Status:\n```\n{log}\n```");
        use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
        let blocks = vec![ContentBlock::Text(TextContent::new(context)), ContentBlock::Text(TextContent::new(text.clone()))];
        let effects = self.submit_for(key, crate::outbox::Queued::new(text, Vec::new(), blocks), false);
        self.apply_effects(key, effects, cx);
    }

    /// A ⋮ or Share menu item.
    pub fn notebook_action(&mut self, key: u64, action: NotebookAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let Some(path) = session.notebook_path.clone() else { return };
        let page = |name: &str| serde_json::json!({ "type": "action", "name": name });
        match action {
            NotebookAction::CopyPath => cx.write_to_clipboard(ClipboardItem::new_string(path)),
            NotebookAction::Reveal => platform::reveal(Path::new(&path)),
            NotebookAction::Rename => self.start_notebook_rename(key, window, cx),
            NotebookAction::MoveTo => self.move_notebook_to(key, cx),
            NotebookAction::LookEndeavor | NotebookAction::LookClassic => {
                let look = if action == NotebookAction::LookEndeavor { NotebookTheme::Endeavor } else { NotebookTheme::Pluto };
                self.update_settings(cx, |s| s.notebook_theme = look);
                self.apply_look(cx);
            }
            NotebookAction::Shortcuts => self.send_to_page(&page("shortcuts"), cx),
            NotebookAction::Feedback => self.send_to_page(&page("feedback"), cx),
            NotebookAction::Present => self.send_to_page(&page("present"), cx),
            NotebookAction::Record => self.send_to_page(&page("record"), cx),
            NotebookAction::Frontmatter => self.send_to_page(&page("frontmatter"), cx),
            NotebookAction::NewSession => {
                let folder = session.place.clone();
                self.new_session_on(folder, path, cx);
            }
            NotebookAction::Start => self.start_notebook(key, cx),
            NotebookAction::Restart => self.restart_notebook(key, cx),
            NotebookAction::Stop => self.stop_notebook(key, path, cx),
            NotebookAction::ExportFile => self.export(key, "notebookfile", "jl", cx),
            NotebookAction::ExportHtml => self.export(key, "notebookexport", "html", cx),
            NotebookAction::ExportPdf => {
                // WebKit's print panel, whose PDF menu saves the file.
                let _ = self.webview.read(cx).raw().print();
            }
        }
        if matches!(action, NotebookAction::Shortcuts | NotebookAction::Feedback | NotebookAction::Frontmatter) {
            let _ = self.webview.read(cx).raw().focus();
        }
    }

    /// Save one of Pluto's exports (`kind`: its URL path) where the user picks.
    pub(crate) fn export(&mut self, key: u64, kind: &'static str, extension: &'static str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let (Some(id), Some(path)) = (session.notebook.clone(), session.notebook_path.clone()) else { return };
        let Some(runtime) = self.connection(&session.place.host).and_then(|c| c.runtime.as_ref()) else { return };
        let offline = if kind == "notebookexport" { "&offline_bundle=true" } else { "" };
        let (bridge, export) = (runtime.bridge.clone(), format!("/{kind}?id={id}{offline}"));
        let stem = Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("notebook").to_string();
        let dir = match &session.place.host {
            HostId::ThisMac => Path::new(&path).parent().map(Path::to_path_buf).unwrap_or_default(),
            _ => dirs_downloads(),
        };
        let picked = cx.prompt_for_new_path(&dir, Some(&format!("{stem}.{extension}")));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(target))) = picked.await else { return };
            let file = target.file_name().map_or_else(|| format!("{stem}.{extension}"), |f| f.to_string_lossy().into_owned());
            let folder = target.parent().map(Path::to_path_buf).unwrap_or_default();
            let saved = cx.background_executor().spawn(async move { pluto::fetch(&bridge, &export).and_then(|bytes| std::fs::write(&target, bytes).map_err(|e| e.to_string())) }).await;
            if let Err(e) = saved {
                let _ = this.update(cx, |this, cx| {
                    let mut notice = Notice::new(Spot::NotebookRight, format!("Couldn't export {file}"), &e, Some(Retry::Export { key, kind, extension }));
                    notice.reason = crate::notice::export_reason(&e, &folder);
                    this.show_notice(notice, cx);
                });
            }
        })
        .detach();
    }

    pub(crate) fn start_notebook_rename(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.sessions.iter().find(|s| s.key == key).and_then(|s| s.notebook_path.clone()) else { return };
        let stem = Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(stem));
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::PressEnter { .. } => this.finish_notebook_rename(cx),
            InputEvent::Blur => {
                this.notebook_rename = None;
                cx.notify();
            }
            _ => {}
        })
        .detach();
        self.notebook_rename = Some((key, input));
        cx.notify();
    }

    fn finish_notebook_rename(&mut self, cx: &mut Context<Self>) {
        let Some((key, input)) = self.notebook_rename.take() else { return };
        let name = input.read(cx).value().trim().trim_end_matches(".jl").to_string();
        let Some((host, path)) = self.sessions.iter().find(|s| s.key == key).and_then(|s| Some((s.place.host.clone(), s.notebook_path.clone()?))) else { return };
        let old = host.file_name(&path).unwrap_or_default();
        let separator = host == HostId::ThisMac && name.contains(std::path::MAIN_SEPARATOR);
        if name.is_empty() || name.contains('/') || separator || name == old.trim_end_matches(".jl") {
            return cx.notify();
        }
        // In the notebook's own folder, by its host's rules: a server's `/`, whatever this computer uses.
        let target = host.join(&host.parent(&path).unwrap_or_default(), &format!("{name}.jl"));
        self.move_notebook(key, target, true, cx);
    }

    /// Move to…: a folder on This Mac, from the macOS panel.
    pub(crate) fn move_notebook_to(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(path) = self.sessions.iter().find(|s| s.key == key).and_then(|s| s.notebook_path.clone()) else { return };
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Move here".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(folders))) = picked.await else { return };
            let Some(folder) = folders.into_iter().next() else { return };
            let Some(name) = Path::new(&path).file_name() else { return };
            let target = folder.join(name).display().to_string();
            let _ = this.update(cx, |this, cx| this.move_notebook(key, target, false, cx));
        })
        .detach();
    }

    /// Rename (`renaming`) or move session `key`'s notebook file (Pluto moves
    /// it), then point every session on it at the new path and tell the agent.
    fn move_notebook(&mut self, key: u64, target: String, renaming: bool, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        let Some((id, old, host)) = self.sessions.iter().find(|s| s.key == key).and_then(|s| Some((s.notebook.clone()?, s.notebook_path.clone()?, s.place.host.clone()))) else { return };
        let task = cx.background_executor().spawn(async move { pluto::move_notebook(&bridge, &id, &target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(new) => this.notebook_moved(&host, &old, new, cx),
                    Err(e) if renaming => this.show_notice(Notice::new(Spot::NotebookLeft, "Couldn't rename the notebook", &e, Some(Retry::Rename(key))), cx),
                    Err(e) => this.show_notice(Notice::new(Spot::NotebookRight, "Couldn't move the notebook", &e, Some(Retry::MoveTo(key))), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The notebook at `old` is now at `new` (Endeavor moved it, or the user located it).
    fn notebook_moved(&mut self, host: &HostId, old: &str, new: String, cx: &mut Context<Self>) {
        let keys: Vec<u64> = self.sessions.iter().filter(|s| s.place.host == *host && s.notebook_path.as_deref() == Some(old)).map(|s| s.key).collect();
        for key in keys {
            self.bind_notebook(key, new.clone(), cx);
            let mut persist = None;
            if let Some(session) = self.session_mut(key) {
                use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
                session.missing = false;
                let (old_dir, new_dir) = (Path::new(old).parent(), Path::new(&new).parent());
                let shown = if old_dir == new_dir { folder_name(Path::new(&new)) } else { new_session::tilde(Path::new(&new)) };
                session.note(format!("The notebook is now {shown}"));
                // Goes to the agent with the next message, after anything already waiting.
                let moved = format!("[Endeavor] The session's notebook file moved from {old} to {new}.");
                let text = match session.start_context.take() {
                    Some(ContentBlock::Text(t)) => format!("{}\n\n{moved}", t.text),
                    _ => moved,
                };
                session.start_context = Some(ContentBlock::Text(TextContent::new(text.clone())));
                // Kept on disk too, so a quit before the note is sent doesn't lose it.
                if let Some(id) = session.id.clone() {
                    persist = Some((id.to_string(), text));
                }
            }
            if let Some((id, text)) = persist {
                self.pending_moved.insert(id, text);
                crate::save_json("pending-context.json", &self.pending_moved);
            }
        }
    }

    /// Record the file's modification time for a notebook that just stopped, so
    /// Start can tell whether it changed meanwhile.
    pub fn note_stopped_file(&mut self, host: &HostId, path: String, cx: &mut Context<Self>) {
        let Some(bridge) = self.bridge(host) else { return };
        let host = host.clone();
        let task = cx.background_executor().spawn({
            let path = path.clone();
            async move { pluto::file_info(&bridge, &path) }
        });
        cx.spawn(async move |this, cx| {
            let Ok(modified) = task.await else { return };
            let _ = this.update(cx, |this, _| {
                for session in this.sessions.iter_mut().filter(|s| s.place.host == host && s.notebook_path.as_deref() == Some(path.as_str())) {
                    if let Some(stopped) = &mut session.stopped {
                        stopped.modified = modified;
                    }
                }
            });
        })
        .detach();
    }

    /// Start a stopped notebook: running if it was, unless its file changed
    /// meanwhile (then safe preview). A file that's gone shows File not found.
    pub fn start_notebook(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some((path, stopped)) = self.session_mut(key).and_then(|s| s.notebook_path.clone().zip(s.stopped)) else { return };
        let Some(bridge) = self.session_bridge(key) else { return };
        let task = cx.background_executor().spawn({
            let path = path.clone();
            async move { pluto::file_info(&bridge, &path) }
        });
        cx.spawn(async move |this, cx| {
            let info = task.await;
            let _ = this.update(cx, |this, cx| match info {
                Ok(None) => this.with_session(key, cx, |s| s.missing = true),
                Ok(Some(modified)) => {
                    let unchanged = stopped.modified.is_none_or(|m| m == modified);
                    this.open_for_session(key, path, !stopped.safe_preview && unchanged, cx);
                }
                Err(_) => this.open_for_session(key, path, false, cx),
            });
        })
        .detach();
    }

    /// Opening a session's notebook failed: if its file is gone, say so.
    pub fn check_missing(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(path) = self.sessions.iter().find(|s| s.key == key).and_then(|s| s.notebook_path.clone()) else { return };
        let Some(bridge) = self.session_bridge(key) else { return };
        let task = cx.background_executor().spawn(async move { pluto::file_info(&bridge, &path) });
        cx.spawn(async move |this, cx| {
            if let Ok(None) = task.await {
                let _ = this.update(cx, |this, cx| this.with_session(key, cx, |s| s.missing = true));
            }
        })
        .detach();
    }

    /// New notebook, from the empty pages: made in the session's folder and shown.
    pub(crate) fn new_notebook_here(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        let task = cx.background_executor().spawn(async move { pluto::new_notebook(&bridge, key) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((id, path)) => {
                    if let Some(session) = this.session_mut(key) {
                        session.missing = false;
                        session.stopped = None;
                        session.note(format!("You made a new notebook, {}", folder_name(Path::new(&path))));
                    }
                    this.apply_effects(key, vec![Effect::ShowNotebook { id, path: Some(path) }], cx);
                }
                Err(e) => this.show_notice(Notice::new(Spot::Pane, "Couldn't make a notebook", &e, Some(Retry::NewNotebook(key))), cx),
            });
        })
        .detach();
    }

    /// Pick session `key`'s notebook file: where its missing notebook went
    /// (Locate file…), or the notebook of a session whose history couldn't
    /// load (Open the notebook only). This Mac uses the file panel; a server,
    /// the in-app folder browser, from the session's folder.
    pub(crate) fn pick_notebook(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let host = session.place.host.clone();
        let folder = session.place.path.clone();
        if host != HostId::ThisMac {
            self.locate = Some(new_session::Browser { host, pick_for: Some(key), path: folder.clone(), listing: None });
            return self.browse_to(folder, cx);
        }
        #[cfg(target_os = "macos")]
        {
            let old_dir = session.notebook_path.as_deref().and_then(|p| Path::new(p).parent()).filter(|d| d.is_dir()).map(Path::to_path_buf);
            new_session::set_open_panel_folder(&old_dir.unwrap_or_else(|| PathBuf::from(&folder)));
        }
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Use this notebook".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(new) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |this, cx| this.notebook_located(key, new.display().to_string(), cx));
        })
        .detach();
    }

    /// The user picked session `key`'s notebook file at `new`: a missing
    /// notebook's sessions follow it there (and Claude is told), a session
    /// without one takes it, and it opens in safe preview.
    pub(crate) fn notebook_located(&mut self, key: u64, new: String, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let host = session.place.host.clone();
        match session.notebook_path.clone() {
            Some(old) => self.notebook_moved(&host, &old, new.clone(), cx),
            None => self.bind_notebook(key, new.clone(), cx),
        }
        self.open_for_session(key, new, false, cx);
    }

    /// The server folder browser picking session `key`'s notebook, as a popover under the button that opened it.
    pub(crate) fn render_notebook_picker(&self, key: u64, cx: &mut Context<Self>) -> Option<AnyElement> {
        let browser = self.locate.as_ref().filter(|b| b.pick_for == Some(key))?;
        let body = div()
            .id("notebook-picker")
            .occlude()
            .w(px(380.))
            .p(px(4.))
            .flex()
            .flex_col()
            .map(theme::popover)
            .font_family(theme::SANS)
            .text_size(theme::size_body())
            .text_color(theme::text_primary())
            .text_left()
            .child(self.browser_menu(browser, cx));
        Some(div().absolute().top(px(34.)).left_0().child(deferred(anchored().child(body)).with_priority(1)).into_any_element())
    }

    /// Open a notebook in a new session…: This Mac's file panel, else the
    /// new-session screen on the session's server and folder.
    fn open_in_new_session(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(place) = self.sessions.iter().find(|s| s.key == key).map(|s| s.place.clone()) else { return };
        if place.host != HostId::ThisMac {
            self.active = None;
            self.draft.host = place.host.clone();
            self.draft.folder = Some(place.path);
            self.scan_notebooks(cx);
            return cx.notify();
        }
        #[cfg(target_os = "macos")]
        new_session::set_open_panel_folder(Path::new(&place.path));
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Open".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(file) = paths.into_iter().next() else { return };
            let folder = file.parent().map(Path::to_path_buf).unwrap_or_default();
            let _ = this.update(cx, |this, cx| this.new_session_on(Place::local(folder), crate::hosts::text(&file), cx));
        })
        .detach();
    }

    /// The notebook pane drawn natively while a session has no notebook to show;
    /// None shows the web view. Our pages, not Pluto's welcome or "Can't find a
    /// file here": what happened, and at most two things to do.
    pub fn notebook_page(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        let key = session.key;
        let mono = |text: String| div().font_family(theme::MONO).text_size(theme::size_code()).text_color(theme::text_primary()).child(text);
        let path = session.notebook_path.clone().unwrap_or_default();
        let file = folder_name(Path::new(&path));
        Some(match self.pane_shows(session, cx) {
            PaneShows::Host(_) => return self.host_pane(&session.place.host, true, cx),
            PaneShows::Page => return None,
            PaneShows::Crashed => self.render_crash_page(key, cx),
            PaneShows::Missing => {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .child(new_session::glyph_at(Glyph::FileX, theme::text_muted(), 2.))
                    .child(page_title(div().flex().gap(px(6.)).child("Can't find").child(mono(file))))
                    .child(div().max_w(px(460.)).px_4().text_center().font_family(theme::MONO).text_size(theme::size_meta()).text_color(theme::text_muted()).child(new_session::tilde(Path::new(&path))))
                    .child(page_text("Nothing is there anymore. It may have been moved, renamed or deleted outside Endeavor."))
                    .child(
                        div()
                            .mt(px(6.))
                            .flex()
                            .gap(px(12.))
                            .child(
                                div()
                                    .relative()
                                    .child(page_button("locate-file", Glyph::Search, "Locate file…", false).on_click(cx.listener(move |this, _, _, cx| this.pick_notebook(key, cx))))
                                    .children(self.render_notebook_picker(key, cx)),
                            )
                            .child(page_button("new-notebook-here", Glyph::File, "New notebook in this session", false).on_click(cx.listener(move |this, _, _, cx| this.new_notebook_here(key, cx)))),
                    )
                    .into_any_element()
            }
            PaneShows::NoNotebook => new_session::turtle_pane()
                .child(page_title("No notebook in this session yet"))
                .child(page_text(format!("{} makes one when there is code to run. You can also start one yourself.", session.agent.name())))
                .child(
                    div()
                        .mt(px(6.))
                        .flex()
                        .gap(px(12.))
                        .child(page_button("new-notebook", Glyph::File, "New notebook", false).on_click(cx.listener(move |this, _, _, cx| this.new_notebook_here(key, cx))))
                        .child(page_button("open-in-new-session", Glyph::Cells, "Open a notebook in a new session…", false).on_click(cx.listener(move |this, _, _, cx| this.open_in_new_session(key, cx)))),
                )
                .into_any_element(),
            PaneShows::Stopped(stopped) => {
                let (why, then) = stopped_text(stopped);
                new_session::turtle_pane()
                    .child(page_title(div().flex().gap(px(6.)).child(mono(file)).child("is stopped")))
                    .child(page_text(why))
                    .children(then.map(|t| page_text(t).text_size(theme::size_meta())))
                    .child(div().mt(px(6.)).child(page_button("start-notebook", Glyph::Play, "Start", true).on_click(cx.listener(move |this, _, _, cx| this.start_notebook(key, cx)))))
                    .into_any_element()
            }
            PaneShows::Opening => new_session::turtle_pane()
                .child(div().flex().items_baseline().text_color(theme::text_muted()).child("Opening ").child(new_session::file_name(file)).child("…"))
                .when(self.opening.stuck, |d| {
                    d.child(div().mt(px(6.)).child(page_button("reload-notebook", Glyph::Restart, "Reload notebook", false).on_click(cx.listener(move |this, _, _, cx| {
                        eprintln!("Reload notebook clicked for session {key}");
                        this.opening.retry(Instant::now());
                        this.reload_notebook(key, cx);
                        cx.notify();
                    }))))
                })
                .into_any_element(),
        })
    }

    /// What the notebook pane shows for a session.
    pub fn pane_shows(&self, session: &Session, cx: &App) -> PaneShows {
        if let Some(host) = self.host_pane_state(&session.place.host, cx) {
            return PaneShows::Host(host);
        }
        if session.missing {
            return PaneShows::Missing;
        }
        if session.notebook_path.is_none() {
            return if session.notebook.is_some() { PaneShows::Page } else { PaneShows::NoNotebook };
        }
        if let Some(stopped) = session.stopped {
            return PaneShows::Stopped(stopped);
        }
        if self.notebook_at(session.key).is_some_and(|at| self.crashes.stopped_at(&at).is_some()) {
            return PaneShows::Crashed;
        }
        // A page taken down with its runtime (`close_page`), or whose web content process
        // ended (no address), stays covered until the reopened one loads.
        let blank = matches!(crate::webcontent::url(self.webview.read(cx).raw()).as_str(), "about:blank" | "");
        if session.notebook.is_some() && !blank { PaneShows::Page } else { PaneShows::Opening }
    }

    /// Every second: if the active session's pane has said "Opening" for a
    /// while though its runtime has the notebook open, load the page again,
    /// once; after that, offer Reload notebook.
    pub fn check_opening(&mut self, cx: &mut Context<Self>) {
        #[cfg(debug_assertions)]
        self.test_blank_page(cx);
        let stuck = self.active_session().filter(|s| self.pane_shows(s, cx) == PaneShows::Opening && self.runtime_has_open(s)).map(|s| (s.key, s.notebook_path.clone().unwrap_or_default()));
        let file = stuck.as_ref().map(|(_, path)| folder_name(Path::new(path))).unwrap_or_default();
        match self.opening.step(stuck.as_ref().map(|(key, _)| *key), Instant::now()) {
            OpeningStep::Wait => {}
            OpeningStep::Reload => {
                eprintln!("The pane said \"Opening {file}\" for {} s while Julia has it open; loading it again", OPENING_GRACE.as_secs());
                if let Some((key, _)) = stuck {
                    self.reload_notebook(key, cx);
                }
            }
            OpeningStep::Stuck => {
                eprintln!("Still \"Opening {file}\" after loading it again; showing Reload notebook");
                cx.notify();
            }
        }
    }

    /// The session's host's runtime lists its notebook file as open.
    fn runtime_has_open(&self, session: &Session) -> bool {
        let Some(path) = session.notebook_path.as_deref() else { return false };
        let Some(list) = self.connection(&session.place.host).and_then(|c| c.notebooks.as_array()) else { return false };
        list.iter().any(|nb| nb["path"] == path)
    }

    /// Load session `key`'s notebook page again, the way opening it does: by
    /// the id its runtime has for the file now.
    pub fn reload_notebook(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        match (session.notebook_path.clone(), session.notebook.clone()) {
            (Some(path), _) => self.open_for_session(key, path, false, cx),
            (None, Some(id)) => {
                let host = session.place.host.clone();
                self.load_notebook(&host, &id, cx);
            }
            (None, None) => {}
        }
    }

    /// WebKit's web content process for the page ended (it crashed, or the
    /// system ended it), leaving the web view blank: load the page again.
    pub fn on_web_content_ended(&mut self, cx: &mut Context<Self>) {
        let showing = |s: &&Session| matches!(self.pane_shows(s, cx), PaneShows::Page | PaneShows::Opening);
        let Some((key, id, host)) = self.active_session().filter(showing).and_then(|s| Some((s.key, s.notebook.clone()?, s.place.host.clone()))) else {
            eprintln!("The notebook's web content process ended; no notebook page was showing");
            return;
        };
        eprintln!("The notebook's web content process ended; loading notebook {id} of session {key} again");
        self.load_notebook(&host, &id, cx);
        cx.notify();
    }

    /// ENDEAVOR_TEST_STUCK_OPENING: take the page down once when the file appears.
    #[cfg(debug_assertions)]
    fn test_blank_page(&mut self, cx: &mut Context<Self>) {
        let wanted = test_stuck_opening().is_some();
        if wanted && !self.test_blanked {
            eprintln!("ENDEAVOR_TEST_STUCK_OPENING: taking the page down");
            self.webview.update(cx, |w, _| w.load_url("about:blank"));
            cx.notify();
        }
        self.test_blanked = wanted;
    }

    /// All of the host's Julia stopped by itself: the crash page for the
    /// session shown, as for a notebook's own Julia.
    pub fn julia_crashed_page(&self, host: &HostId, reason: &str, cx: &mut Context<Self>) -> AnyElement {
        match self.active_session().filter(|s| s.place.host == *host && s.notebook_path.is_some()) {
            Some(session) => self.render_crash_page(session.key, cx),
            None => self.julia_stopped_page(host, reason, cx),
        }
    }

    /// Julia isn't running on the session's host: Start, and on a cluster the
    /// job's resources behind a gear (the new-session screen's chip).
    pub fn julia_stopped_page(&self, host: &HostId, reason: &str, cx: &mut Context<Self>) -> AnyElement {
        let name = self.hosts.name(host);
        let cluster = self.is_cluster(host);
        let session = self.active_session().filter(|s| s.place.host == *host);
        let text = if cluster {
            "This session's notebook runs there. Starting asks Slurm for a node, which can take a few minutes.".to_string()
        } else if *host == HostId::ThisMac {
            "This session's notebook runs here, in Endeavor's Julia.".to_string()
        } else {
            "This session's notebook runs there.".to_string()
        };
        let resources = session.filter(|_| cluster).map(|s| {
            let key = s.key;
            let summary = s
                .resources
                .clone()
                .or_else(|| match host {
                    HostId::Server(id) => self.hosts.server(id).and_then(|s| s.cluster.as_ref()).map(|c| c.resources.clone()),
                    HostId::ThisMac => None,
                })
                .map(|r| r.summary())
                .unwrap_or_default();
            let open = self.pane_resources == Some(key);
            div()
                .relative()
                .child(
                    div()
                        .id("pane-resources")
                        .role(Role::Button)
                        .aria_label("Job resources")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(26.))
                        .px(px(10.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(theme::control_edge())
                        .cursor_pointer()
                        .text_size(theme::size_meta())
                        .text_color(theme::text_secondary())
                        .when(open, |d| d.bg(theme::row_active()))
                        .hover(|s| s.bg(theme::row_active()))
                        .child(glyph(Glyph::Gear, theme::text_muted()))
                        .child(summary)
                        .child(glyph(Glyph::Chevron, theme::text_muted()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pane_resources = if this.pane_resources == Some(key) { None } else { Some(key) };
                            this.pane_partition_menu = false;
                            cx.notify();
                        })),
                )
                .when(open, |d| d.child(self.pane_resources_popover(key, host, cx)))
        });
        let older = reason == endeavor_mcp::OLDER_RUNTIME;
        let on = if cluster || *host != HostId::ThisMac { format!(" on {name}") } else { String::new() };
        let start_label = match (older, on.is_empty()) {
            (true, _) => format!("Restart Julia{on}"),
            (false, false) => format!("Start{on}"),
            (false, true) => "Start Julia".to_string(),
        };
        let host_for_start = host.clone();
        let fixes = self.fixes(host, reason);
        let repair = fixes.contains(&crate::connection::Fix::Repair);
        new_session::turtle_pane()
            .child(page_title(format!("Julia isn't running on {}", if *host == HostId::ThisMac { crate::platform::this_computer!().to_string() } else { name.clone() })))
            .child(page_text(text))
            .when(!reason.is_empty(), |d| d.child(page_text(reason.to_string()).text_size(theme::size_meta())))
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .children(resources)
                    .child(page_button("host-start", Glyph::Play, start_label, true).on_click(cx.listener(move |this, _, window, cx| {
                        this.pane_resources = None;
                        if !older {
                            return this.start_host(&host_for_start, cx);
                        }
                        let host = host_for_start.clone();
                        let body = "An older version of Endeavor started it. Notebooks open there stop; their files are saved.";
                        this.open_confirm(format!("Restart Julia{on}?"), body, "Restart", window, cx, move |this, _, cx| this.restart_host(&host, cx));
                    })))
                    .children(fixes.into_iter().map(|fix| self.fix_button(fix, false, cx))),
            )
            .when(repair, |d| d.child(page_text(crate::connection::REPAIR_NOTE.to_string()).text_size(theme::size_meta())))
            .into_any_element()
    }

    /// The gear's popover: the session's job resources, as on the new-session screen.
    fn pane_resources_popover(&self, key: u64, host: &HostId, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let HostId::Server(id) = host else { return div() };
        let Some(cluster) = self.hosts.server(id).and_then(|s| s.cluster.as_ref()) else { return div() };
        let r = self.sessions.iter().find(|s| s.key == key).and_then(|s| s.resources.clone()).unwrap_or_else(|| cluster.resources.clone());
        let rows = self.resource_rows(Target::Session(key), &r, &cluster.partitions, self.pane_partition_menu, true, cx);
        let body = div()
            .id("pane-resources-menu")
            .occlude()
            .w(px(300.))
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .map(theme::popover)
            .text_size(theme::size_body())
            .text_color(theme::text_primary())
            .children(rows)
            .child(div().text_size(theme::size_meta()).text_color(theme::text_faint()).child("Notebooks stop when the job's time runs out."));
        div().absolute().top(px(32.)).left_0().child(deferred(anchored().anchor(Anchor::TopLeft).child(body)).with_priority(1))
    }
}

/// Where exports from a server's notebook are suggested: ~/Downloads.
fn dirs_downloads() -> PathBuf {
    wire::files::home().join("Downloads")
}

#[cfg(test)]
mod tests {
    use super::NotebookAction::{self, *};
    use super::{OPENING_GRACE, OpeningStep, OpeningWatch};
    use std::time::{Duration, Instant};

    #[test]
    fn a_stuck_opening_reloads_once_then_offers_the_button() {
        let start = Instant::now();
        let at = |secs: u64| start + Duration::from_secs(secs);
        let grace = OPENING_GRACE.as_secs();
        let mut watch = OpeningWatch::default();
        assert_eq!(watch.step(Some(1), at(0)), OpeningStep::Wait);
        assert_eq!(watch.step(Some(1), at(grace - 1)), OpeningStep::Wait);
        assert_eq!(watch.step(Some(1), at(grace)), OpeningStep::Reload);
        assert_eq!(watch.step(Some(1), at(grace + 1)), OpeningStep::Wait, "the reload gets its own grace");
        assert_eq!(watch.step(Some(1), at(2 * grace)), OpeningStep::Stuck);
        assert!(watch.stuck);
        assert_eq!(watch.step(Some(1), at(2 * grace + 10)), OpeningStep::Wait, "the button stays; no more reloads");
        watch.retry(at(3 * grace));
        assert!(!watch.stuck);
        assert_eq!(watch.step(Some(1), at(4 * grace)), OpeningStep::Stuck, "a click that didn't help shows it again");
    }

    #[test]
    fn the_opening_watch_starts_over_when_the_page_comes_or_the_session_changes() {
        let start = Instant::now();
        let at = |secs: u64| start + Duration::from_secs(secs);
        let grace = OPENING_GRACE.as_secs();
        let mut watch = OpeningWatch::default();
        watch.step(Some(1), at(0));
        assert_eq!(watch.step(Some(1), at(grace)), OpeningStep::Reload);
        assert_eq!(watch.step(None, at(grace + 1)), OpeningStep::Wait, "the page came");
        assert_eq!(watch.step(Some(1), at(grace + 2)), OpeningStep::Wait);
        assert_eq!(watch.step(Some(1), at(2 * grace + 2)), OpeningStep::Reload, "stuck again later: a fresh reload");
        assert_eq!(watch.step(Some(2), at(2 * grace + 3)), OpeningStep::Wait, "another session starts its own wait");
        assert_eq!(watch.step(Some(2), at(3 * grace + 3)), OpeningStep::Reload);
        assert_eq!(watch.state(at(3 * grace + 4)), Some((1, true, false)));
    }

    #[test]
    fn the_notebook_menu_offers_what_applies() {
        assert_eq!(
            NotebookAction::for_notebook(true, false, false, true),
            [CopyPath, Reveal, Rename, MoveTo, LookEndeavor, LookClassic, Shortcuts, NewSession, Feedback, Restart, Stop]
        );
        assert_eq!(NotebookAction::for_notebook(true, false, true, true).contains(&Restart), false, "safe preview: Run notebook, not Restart");
        assert_eq!(NotebookAction::for_notebook(true, false, false, false), [CopyPath, Rename, LookEndeavor, LookClassic, Shortcuts, NewSession, Feedback, Restart, Stop]);
        assert_eq!(NotebookAction::for_notebook(false, false, false, true), [CopyPath, Reveal, LookEndeavor, LookClassic, Shortcuts, NewSession, Feedback]);
        assert_eq!(
            NotebookAction::for_notebook(false, true, false, true),
            [CopyPath, Reveal, LookEndeavor, LookClassic, Shortcuts, NewSession, Feedback, Start],
            "a stopped notebook offers Start instead of Restart and Stop"
        );
        assert!(Stop.danger() && !Restart.danger() && !Start.danger());
        let share = NotebookAction::for_share();
        assert_eq!(share.iter().map(|a| a.label()).collect::<Vec<_>>(), ["Notebook file…", "Static HTML…", "PDF…", "Present", "Record…", "Frontmatter…"]);
        for menu in [NotebookAction::for_notebook(true, false, false, true), NotebookAction::for_notebook(false, true, false, true), share] {
            let mut keys: Vec<_> = menu.iter().map(|a| a.shortcut().0).collect();
            keys.sort();
            keys.dedup();
            assert_eq!(keys.len(), menu.len(), "one key per item in {menu:?}");
        }
    }
}
