//! The notebook pane around Pluto's page: its header (the notebook system's
//! logo, the file, where it runs, what state it's in, then Point, Share, Live
//! docs, Status and ⋮), the Share and ⋮ menus' actions, and the pages drawn
//! natively when there's no notebook to show (docs/design-gaps.md, "Notebook
//! pane"). In the Pluto classic look the header keeps only the logo, file,
//! host, Point and ⋮, since Pluto's own page has the rest.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::annotate::PageState;
use crate::hosts::{HostId, Place};
use crate::new_session::{self, Glyph, glyph};
use crate::resources::Target;
use crate::session::{Effect, Session, folder_name};
use crate::settings::NotebookTheme;
use crate::overlay;
use crate::{MenuTarget, Workspace, platform, pluto, theme};

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
            NotebookAction::Reveal => "Reveal in Finder",
            NotebookAction::Rename => "Rename…",
            NotebookAction::MoveTo => "Move to…",
            NotebookAction::LookEndeavor => "Endeavor",
            NotebookAction::LookClassic => "Pluto classic",
            NotebookAction::Shortcuts => "Keyboard shortcuts",
            NotebookAction::NewSession => "Open in a new session…",
            NotebookAction::Feedback => "Send feedback to Pluto's developers…",
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
            NotebookAction::Restart | NotebookAction::Stop => 4,
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
    /// safe preview, where Run notebook is the way to start it.
    pub fn for_notebook(open: bool, safe: bool, local: bool) -> Vec<NotebookAction> {
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

/// Header buttons' tooltips showing now; while one is, the web view keeps its hole.
static TOOLTIPS: AtomicUsize = AtomicUsize::new(0);

pub fn tooltip_over_notebook() -> bool {
    TOOLTIPS.load(Ordering::Relaxed) > 0
}

/// A header button's tooltip. It hangs over the notebook, whose web view (a
/// native view on top of what GPUI draws) gets a hole there, as for menus.
struct PaneTooltip {
    text: SharedString,
    webview: Entity<gpui_wry::WebView>,
    hole: Rc<Cell<Option<Bounds<Pixels>>>>,
}

pub fn tooltip(text: &'static str, webview: &Entity<gpui_wry::WebView>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let webview = webview.clone();
    move |_, cx| {
        TOOLTIPS.fetch_add(1, Ordering::Relaxed);
        cx.new(|_| PaneTooltip { text: text.into(), webview: webview.clone(), hole: Rc::default() }).into()
    }
}

impl Render for PaneTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (webview, hole) = (self.webview.clone(), self.hole.clone());
        let cut = canvas(
            move |bounds, _, cx| {
                let webview = webview.read(cx);
                let rect = Bounds { origin: bounds.origin - webview.bounds().origin, size: bounds.size };
                overlay::set_hole(webview.raw(), Some(rect));
                hole.set(Some(rect));
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

impl Drop for PaneTooltip {
    fn drop(&mut self) {
        TOOLTIPS.fetch_sub(1, Ordering::Relaxed);
        if let Some(rect) = self.hole.get() {
            overlay::close_hole_at(rect);
        }
    }
}

/// A 24px header button: an icon, maybe a label; lit while its panel or menu is open.
fn header_button(id: &'static str, icon: Glyph, label: Option<&'static str>, active: bool) -> Stateful<Div> {
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
        .children(label)
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
                d.bg(theme::accent()).text_color(theme::text_primary()).child(glyph(icon, theme::text_primary()))
            } else {
                d.border_1().border_color(theme::composer_edge()).text_color(theme::text_primary()).hover(|s| s.bg(theme::row_active())).child(glyph(icon, theme::text_muted()))
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

    /// The notebook pane's header for session `ix`. `shown`: its notebook is on screen (Point works).
    pub fn notebook_header(&self, ix: usize, shown: bool, cx: &mut Context<Self>) -> AnyElement {
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
        let host = self.host_label(&session.place.host);
        let read_only = self.read_only(session);
        let reconnecting = page.is_some_and(|p| !p.connected) && !read_only;
        let host_icon = match &session.place.host {
            HostId::ThisMac => Glyph::Laptop,
            HostId::Server(_) if self.is_cluster(&session.place.host) => Glyph::Cluster,
            HostId::Server(_) => Glyph::Server,
        };
        let host_chip = chip(Some(host_icon), if reconnecting { format!("{host} · reconnecting") } else { host }, theme::text_tag());

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

        let mut chips: Vec<AnyElement> = Vec::new();
        if read_only {
            chips.push(chip(Some(Glyph::Lock), "Read-only", theme::text_tag()).into_any_element());
        }
        if session.missing {
            chips.push(chip(Some(Glyph::Warning), "Not found", theme::danger()).into_any_element());
        } else if session.stopped.is_some() {
            chips.push(chip(None, "Stopped", theme::text_faint()).into_any_element());
        }
        if let Some(p) = page.filter(|_| endeavor) {
            if p.safe {
                chips.push(chip(Some(Glyph::Shield), "Safe preview", theme::accent_text()).into_any_element());
            }
            if p.save_failed {
                chips.push(chip(Some(Glyph::Warning), "Not saved", theme::danger()).into_any_element());
            }
            if p.package_failed.is_some() {
                chips.push(
                    chip(Some(Glyph::Warning), "Package failed", theme::danger())
                        .id("package-failed")
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| this.open_drawer(Some("status"), cx)))
                        .into_any_element(),
                );
            } else if p.restart.is_some() {
                let why = if p.restart.as_deref() == Some("required") { "Restart needed" } else { "Restart recommended" };
                chips.push(
                    chip(Some(Glyph::Restart), why, theme::accent_text())
                        .id("restart-needed")
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| this.restart_notebook(key, cx)))
                        .into_any_element(),
                );
            } else if p.dead {
                chips.push(chip(None, "Julia exited", theme::danger()).into_any_element());
            }
        }

        // What's under way, while the drawer's Status tab isn't showing it.
        let busy = page.filter(|p| endeavor && p.drawer.as_deref() != Some("status")).and_then(|p| p.busy.clone()).map(|text| {
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
        // The tip and a tooltip would fight over the web view's one hole.
        let point_tip = self.point_tip_shows(shown);
        let tip = |d: Stateful<Div>, text: &'static str| if point_tip { d } else { d.tooltip(tooltip(text, &self.webview)) };
        let point = tip(header_button("header-point", Glyph::Pointer, Some("Point"), self.annotating), "Pick cells or draw a box to ask Claude about  ⌘⇧K")
            .on_click(cx.listener(|this, _, window, cx| this.toggle_annotation(&crate::ToggleAnnotation, window, cx)))
            .when(point_tip, |d| d.child(self.render_point_tip(if endeavor { 120. } else { 32. }, cx)));
        let drawer = page.and_then(|p| p.drawer.clone());
        let tools = (endeavor && shown).then(|| {
            let share_menu = self.menu.as_ref().filter(|m| m.target == MenuTarget::Share(key));
            [
                header_button("header-share", Glyph::Share, None, open(MenuTarget::Share(key)))
                    .when(share_menu.is_none() && !point_tip, |d| d.tooltip(tooltip("Share and export", &self.webview)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_menu(MenuTarget::Share(key), None, window, cx);
                    }))
                    .children(share_menu.map(|menu| self.render_menu(menu, cx)))
                    .into_any_element(),
                tip(header_button("header-docs", Glyph::Book, None, drawer.as_deref() == Some("docs")), "Live docs")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_drawer("docs", cx)))
                    .into_any_element(),
                tip(header_button("header-status", Glyph::Pulse, None, drawer.as_deref() == Some("status")), "Status")
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
            HostId::ThisMac => "This Mac".to_string(),
            host => self.hosts.name(host),
        };
        let msg = serde_json::json!({ "type": "context", "host": host, "asking": session.asking_to_run(), "readonly": self.read_only(session) });
        let text = msg.to_string();
        if text != self.page_context {
            self.page_context = text;
            self.send_to_page(&msg, cx);
        }
    }

    /// The page reported its notebook's state. Leaving safe preview some other
    /// way (Pluto classic's own button) answers a pending "Let this notebook run?".
    pub fn on_page_state(&mut self, state: PageState, cx: &mut Context<Self>) {
        let left_safe = self.page.notebook == state.notebook && self.page.safe && !state.safe && state.connected;
        self.page = state;
        if left_safe && let Some(key) = self.session_showing(&self.page.notebook) {
            self.with_session(key, cx, |s| {
                if s.asking_to_run() {
                    s.answer_pending(agent_client_protocol::schema::v1::PermissionOptionKind::AllowOnce, false);
                }
            });
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
                answered = s.answer_pending(agent_client_protocol::schema::v1::PermissionOptionKind::AllowOnce, false);
            }
        });
        if answered {
            return;
        }
        let Some(bridge) = self.session_bridge(key) else { return };
        let task = cx.background_executor().spawn(async move { pluto::allow_execution(&bridge, &notebook) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.status = format!("⚠ Couldn't run the notebook: {e}").into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn restart_notebook(&mut self, key: u64, cx: &mut Context<Self>) {
        let (Some(bridge), Some(id)) = (self.session_bridge(key), self.sessions.iter().find(|s| s.key == key).and_then(|s| s.notebook.clone())) else { return };
        let task = cx.background_executor().spawn(async move { pluto::restart_notebook(&bridge, &id) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.status = format!("⚠ Couldn't restart the notebook: {e}").into();
                    cx.notify();
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
        let Some(session) = self.session_mut(key) else { return };
        let effects = session.submit(crate::outbox::Queued::new(text, Vec::new(), blocks), false);
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
                self.new_session_on(folder, PathBuf::from(path), cx);
            }
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
    fn export(&mut self, key: u64, kind: &'static str, extension: &'static str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let (Some(id), Some(path)) = (session.notebook.clone(), session.notebook_path.clone()) else { return };
        let Some(runtime) = self.connection(&session.place.host).and_then(|c| c.runtime.as_ref()) else { return };
        let offline = if kind == "notebookexport" { "offline_bundle=true&" } else { "" };
        let url = runtime.pluto_url.replacen("/?", &format!("/{kind}?id={id}&{offline}"), 1);
        let stem = Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("notebook").to_string();
        let dir = match &session.place.host {
            HostId::ThisMac => Path::new(&path).parent().map(Path::to_path_buf).unwrap_or_default(),
            _ => dirs_downloads(),
        };
        let picked = cx.prompt_for_new_path(&dir, Some(&format!("{stem}.{extension}")));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(target))) = picked.await else { return };
            let saved = cx.background_executor().spawn(async move { pluto::fetch(&url).and_then(|bytes| std::fs::write(&target, bytes).map_err(|e| e.to_string())) }).await;
            if let Err(e) = saved {
                let _ = this.update(cx, |this, cx| {
                    this.status = format!("⚠ Couldn't export: {e}").into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn start_notebook_rename(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
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
        let Some(path) = self.sessions.iter().find(|s| s.key == key).and_then(|s| s.notebook_path.clone()) else { return };
        let old = Path::new(&path);
        if name.is_empty() || name.contains('/') || Some(name.as_str()) == old.file_stem().and_then(|s| s.to_str()) {
            return cx.notify();
        }
        let target = old.with_file_name(format!("{name}.jl"));
        self.move_notebook(key, target.display().to_string(), cx);
    }

    /// Move to…: a folder on This Mac, from the macOS panel.
    fn move_notebook_to(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(path) = self.sessions.iter().find(|s| s.key == key).and_then(|s| s.notebook_path.clone()) else { return };
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Move here".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(folders))) = picked.await else { return };
            let Some(folder) = folders.into_iter().next() else { return };
            let Some(name) = Path::new(&path).file_name() else { return };
            let target = folder.join(name).display().to_string();
            let _ = this.update(cx, |this, cx| this.move_notebook(key, target, cx));
        })
        .detach();
    }

    /// Rename or move session `key`'s notebook file (Pluto moves it), then point
    /// every session on it at the new path and tell the agent.
    fn move_notebook(&mut self, key: u64, target: String, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        let Some((id, old, host)) = self.sessions.iter().find(|s| s.key == key).and_then(|s| Some((s.notebook.clone()?, s.notebook_path.clone()?, s.place.host.clone()))) else { return };
        let task = cx.background_executor().spawn(async move { pluto::move_notebook(&bridge, &id, &target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(new) => this.notebook_moved(&host, &old, new, cx),
                    Err(e) => this.status = format!("⚠ Couldn't rename the notebook: {e}").into(),
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
                session.start_context = Some(ContentBlock::Text(TextContent::new(text)));
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
    fn new_notebook_here(&mut self, key: u64, cx: &mut Context<Self>) {
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
                Err(e) => {
                    this.status = format!("⚠ Couldn't make a notebook: {e}").into();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Locate file…: pick where the missing notebook went (This Mac); the session follows it.
    fn locate_file(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.iter().find(|s| s.key == key) else { return };
        let Some(old) = session.notebook_path.clone() else { return };
        let host = session.place.host.clone();
        #[cfg(target_os = "macos")]
        if let Some(dir) = Path::new(&old).parent().filter(|d| d.is_dir()) {
            new_session::set_open_panel_folder(dir);
        }
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Use this notebook".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(new) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |this, cx| {
                let new = new.display().to_string();
                this.notebook_moved(&host, &old, new.clone(), cx);
                this.open_for_session(key, new, false, cx);
            });
        })
        .detach();
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
        new_session::set_open_panel_folder(&place.path);
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Open".into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(file) = paths.into_iter().next() else { return };
            let folder = file.parent().map(Path::to_path_buf).unwrap_or_default();
            let _ = this.update(cx, |this, cx| this.new_session_on(Place::local(folder), file, cx));
        })
        .detach();
    }

    /// The notebook pane drawn natively while a session has no notebook to show;
    /// None shows the web view. Our pages, not Pluto's welcome or "Can't find a
    /// file here": what happened, and at most two things to do.
    pub fn notebook_page(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(pane) = self.host_pane(&session.place.host, true, cx) {
            return Some(pane);
        }
        let key = session.key;
        let mono = |text: String| div().font_family(theme::MONO).text_size(theme::size_code()).text_color(theme::text_primary()).child(text);
        if session.missing {
            let path = session.notebook_path.clone().unwrap_or_default();
            let file = folder_name(Path::new(&path));
            let local = session.place.host == HostId::ThisMac;
            return Some(
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
                            .when(local, |d| d.child(page_button("locate-file", Glyph::Search, "Locate file…", false).on_click(cx.listener(move |this, _, _, cx| this.locate_file(key, cx)))))
                            .child(page_button("new-notebook-here", Glyph::File, "New notebook in this session", false).on_click(cx.listener(move |this, _, _, cx| this.new_notebook_here(key, cx)))),
                    )
                    .into_any_element(),
            );
        }
        let Some(path) = session.notebook_path.as_deref() else {
            if session.notebook.is_some() {
                return None;
            }
            return Some(
                new_session::turtle_pane()
                    .child(page_title("No notebook in this session yet"))
                    .child(page_text("Claude makes one when there is code to run. You can also start one yourself."))
                    .child(
                        div()
                            .mt(px(6.))
                            .flex()
                            .gap(px(12.))
                            .child(page_button("new-notebook", Glyph::File, "New notebook", false).on_click(cx.listener(move |this, _, _, cx| this.new_notebook_here(key, cx))))
                            .child(page_button("open-in-new-session", Glyph::Cells, "Open a notebook in a new session…", false).on_click(cx.listener(move |this, _, _, cx| this.open_in_new_session(key, cx)))),
                    )
                    .into_any_element(),
            );
        };
        let file = folder_name(Path::new(path));
        if let Some(stopped) = session.stopped {
            let why = match stopped.idle_hours {
                Some(hours) => format!("It stopped after {hours} hours without use. The file is saved; Start runs it again from the top."),
                None => "The file is saved; Start runs it again from the top.".to_string(),
            };
            let (why, then) = if stopped.safe_preview {
                ("The file is saved. It was in safe preview, so Start opens it that way again.".to_string(), None)
            } else {
                (why, Some("If the file changed on disk meanwhile, it opens in safe preview instead."))
            };
            return Some(
                new_session::turtle_pane()
                    .child(page_title(div().flex().gap(px(6.)).child(mono(file)).child("is stopped")))
                    .child(page_text(why))
                    .children(then.map(|t| page_text(t).text_size(theme::size_meta())))
                    .child(div().mt(px(6.)).child(page_button("start-notebook", Glyph::Play, "Start", true).on_click(cx.listener(move |this, _, _, cx| this.start_notebook(key, cx)))))
                    .into_any_element(),
            );
        }
        // A page taken down with its runtime (`close_page`) stays covered until the reopened one loads.
        let blank = self.webview.read(cx).raw().url().is_ok_and(|url| url == "about:blank");
        if session.notebook.is_some() && !blank {
            return None;
        }
        Some(new_session::turtle_pane().child(div().flex().items_baseline().text_color(theme::text_muted()).child("Opening ").child(new_session::file_name(file)).child("…")).into_any_element())
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
                        .border_color(theme::composer_edge())
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
        let start_label = if cluster || *host != HostId::ThisMac { format!("Start on {name}") } else { "Start Julia".to_string() };
        let host_for_start = host.clone();
        let fixes = self.fixes(host, reason);
        let repair = fixes.contains(&crate::connection::Fix::Repair);
        new_session::turtle_pane()
            .child(page_title(format!("Julia isn't running on {}", if *host == HostId::ThisMac { "This Mac".to_string() } else { name.clone() })))
            .child(page_text(text))
            .when(!reason.is_empty(), |d| d.child(page_text(reason.to_string()).text_size(theme::size_meta())))
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .children(resources)
                    .child(page_button("host-start", Glyph::Play, start_label, true).on_click(cx.listener(move |this, _, _, cx| {
                        this.pane_resources = None;
                        this.start_host(&host_for_start, cx);
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
            .rounded(px(8.))
            .border_1()
            .border_color(theme::composer_edge())
            .bg(theme::bg_raised())
            .text_size(theme::size_body())
            .text_color(theme::text_primary())
            .children(rows)
            .child(div().text_size(theme::size_meta()).text_color(theme::text_faint()).child("Notebooks stop when the job's time runs out."));
        div().absolute().top(px(32.)).left_0().child(deferred(anchored().anchor(Anchor::TopLeft).child(body)).with_priority(1))
    }
}

/// Where exports from a server's notebook are suggested: ~/Downloads.
fn dirs_downloads() -> PathBuf {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Downloads")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::NotebookAction::{self, *};

    #[test]
    fn the_notebook_menu_offers_what_applies() {
        assert_eq!(
            NotebookAction::for_notebook(true, false, true),
            [CopyPath, Reveal, Rename, MoveTo, LookEndeavor, LookClassic, Shortcuts, NewSession, Feedback, Restart, Stop]
        );
        assert_eq!(NotebookAction::for_notebook(true, true, true).contains(&Restart), false, "safe preview: Run notebook, not Restart");
        assert_eq!(NotebookAction::for_notebook(true, false, false), [CopyPath, Rename, LookEndeavor, LookClassic, Shortcuts, NewSession, Feedback, Restart, Stop]);
        assert_eq!(NotebookAction::for_notebook(false, false, true), [CopyPath, Reveal, LookEndeavor, LookClassic, Shortcuts, NewSession, Feedback]);
        assert!(Stop.danger() && !Restart.danger());
        let share = NotebookAction::for_share();
        assert_eq!(share.iter().map(|a| a.label()).collect::<Vec<_>>(), ["Notebook file…", "Static HTML…", "PDF…", "Present", "Record…", "Frontmatter…"]);
        for menu in [NotebookAction::for_notebook(true, false, true), share] {
            let mut keys: Vec<_> = menu.iter().map(|a| a.shortcut().0).collect();
            keys.sort();
            keys.dedup();
            assert_eq!(keys.len(), menu.len(), "one key per item in {menu:?}");
        }
    }
}
