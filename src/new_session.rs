//! The new-session screen: recent sessions to pick up, chips for where the
//! session runs, its folder and its notebook, and the shared composer. The
//! notebook pane shows a native stand-in (Pluto isn't involved until the
//! session starts): an empty state for a new notebook, or a static preview of
//! the chosen one's first cells.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState, Textarea};

use crate::notebook_files::{self, Found, Preview};
use crate::session::folder_name;
use crate::turtle::{self, Pose};
use crate::{Workspace, theme, when};

#[derive(Clone, Debug, PartialEq)]
pub enum NotebookChoice {
    /// The agent creates the notebook when it needs one.
    New,
    Existing(PathBuf),
}

#[derive(Clone, Copy, PartialEq)]
pub enum Chip {
    Where,
    Folder,
    Notebook,
}

/// The new-session screen's state.
pub struct Draft {
    pub folder: PathBuf,
    pub notebook: NotebookChoice,
    pub popover: Option<Chip>,
    /// Notebooks found in `folder`, newest first.
    pub notebooks: Vec<Found>,
    /// The chosen existing notebook's first cells, once read.
    pub preview: Option<Preview>,
    /// The folder popover's search box, and its highlighted row.
    pub search: Entity<InputState>,
    pub selected: usize,
}

/// A session to pick up where you left off.
struct Resume {
    title: String,
    folder: String,
    notebook: Option<String>,
    when: String,
    open: ResumeTarget,
}

enum ResumeTarget {
    Open(u64),
    Past(agent_client_protocol::schema::v1::SessionInfo),
}

const RESUME_SHOWN: usize = 3;

/// The folder a new session starts in when there are no recent ones. It's
/// created at launch, so the folder chip and Browse… have somewhere real to point.
pub fn default_folder(recent: &[PathBuf]) -> PathBuf {
    if let Some(folder) = recent.first() {
        return folder.clone();
    }
    let folder = home().join("Documents/Endeavor");
    let _ = std::fs::create_dir_all(&folder);
    folder
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `path` with the home folder written as `~`.
fn tilde(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Where Browse… opens: ~/Documents/Endeavor, or ~/Documents without it.
fn browse_start() -> PathBuf {
    let endeavor = home().join("Documents/Endeavor");
    if endeavor.is_dir() { endeavor } else { home().join("Documents") }
}

/// NSOpenPanel opens where it was last left, kept in this user default; GPUI's
/// prompt has no starting-folder option, so set it before opening the panel.
fn set_open_panel_folder(dir: &Path) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    let Ok(path) = std::ffi::CString::new(dir.display().to_string()) else { return };
    let Ok(key) = std::ffi::CString::new("NSNavLastRootDirectory") else { return };
    unsafe {
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let value: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: path.as_ptr()];
        let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: key.as_ptr()];
        let _: () = msg_send![defaults, setObject: value, forKey: key];
    }
}

impl Draft {
    pub fn new(folder: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search folders"));
        cx.subscribe_in(&search, window, |this: &mut Workspace, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                this.draft.selected = 0;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                if let Some(folder) = this.folder_matches(cx).into_iter().nth(this.draft.selected) {
                    this.set_draft_folder(folder, window, cx);
                }
            }
            _ => {}
        })
        .detach();
        Draft { folder, notebook: NotebookChoice::New, popover: None, notebooks: Vec::new(), preview: None, search, selected: 0 }
    }
}

impl Workspace {
    /// Look for notebooks in the draft's folder, off the main thread.
    pub fn scan_notebooks(&mut self, cx: &mut Context<Self>) {
        let folder = self.draft.folder.clone();
        let scan = cx.background_spawn({
            let folder = folder.clone();
            async move { notebook_files::scan(&folder) }
        });
        cx.spawn(async move |this, cx| {
            let found = scan.await;
            let _ = this.update(cx, |this, cx| {
                if this.draft.folder == folder {
                    this.draft.notebooks = found;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_draft_folder(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if folder != self.draft.folder {
            self.draft.folder = folder;
            self.draft.notebooks.clear();
            self.choose_notebook(NotebookChoice::New, cx);
            self.scan_notebooks(cx);
        }
        self.close_popover(window, cx);
    }

    fn choose_notebook(&mut self, choice: NotebookChoice, cx: &mut Context<Self>) {
        self.draft.preview = None;
        self.draft.notebook = choice.clone();
        if let NotebookChoice::Existing(path) = choice {
            let read = cx.background_spawn({
                let path = path.clone();
                async move { std::fs::read_to_string(&path).map(|text| notebook_files::preview(&text)).unwrap_or_default() }
            });
            cx.spawn(async move |this, cx| {
                let preview = read.await;
                let _ = this.update(cx, |this, cx| {
                    if this.draft.notebook == NotebookChoice::Existing(path) {
                        this.draft.preview = Some(preview);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn toggle_popover(&mut self, chip: Chip, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.popover == Some(chip) {
            return self.close_popover(window, cx);
        }
        self.draft.popover = Some(chip);
        match chip {
            Chip::Folder => {
                self.draft.selected = 0;
                self.draft.search.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                    s.focus(window, cx);
                });
            }
            Chip::Notebook => self.scan_notebooks(cx),
            Chip::Where => {}
        }
        cx.notify();
    }

    /// Close the open chip menu; the composer gets the keyboard back.
    pub fn close_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.popover.take().is_some() {
            self.input.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    fn browse_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popover(window, cx);
        set_open_panel_folder(&browse_start());
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Choose folder".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = picked.await
                && let Some(path) = paths.pop()
            {
                let _ = this.update_in(cx, |this, window, cx| this.set_draft_folder(path, window, cx));
            }
        })
        .detach();
    }

    /// Recent folders (the current one first if it isn't recent) matching the search.
    fn folder_matches(&self, cx: &App) -> Vec<PathBuf> {
        let query = self.draft.search.read(cx).value().trim().to_lowercase();
        let current = (!self.recent.contains(&self.draft.folder)).then(|| self.draft.folder.clone());
        current
            .into_iter()
            .chain(self.recent.iter().cloned())
            .filter(|p| query.is_empty() || format!("{}\n{}", p.display(), tilde(p)).to_lowercase().contains(&query))
            .collect()
    }

    fn folder_popover_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.folder_matches(cx).len().max(1);
        match e.keystroke.key.as_str() {
            "down" => self.draft.selected = (self.draft.selected + 1) % n,
            "up" => self.draft.selected = (self.draft.selected + n - 1) % n,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Sessions to pick up: open ones first (newest first), then Endeavor's past
    /// sessions by last activity.
    fn resumable(&self) -> Vec<Resume> {
        let notebook_file = |id: &String| self.last_notebooks.iter().find(|(nid, _)| nid == id).map(|(_, path)| folder_name(Path::new(path)));
        let recorded = |id: &agent_client_protocol::schema::v1::SessionId| self.session_notebooks.get(&id.to_string()).map(|path| folder_name(Path::new(path)));
        let open = self.sessions.iter().rev().map(|s| Resume {
            title: s.title.clone(),
            folder: folder_name(&s.cwd),
            notebook: s.notebook.as_ref().and_then(notebook_file).or_else(|| s.id.as_ref().and_then(recorded)),
            when: "open".into(),
            open: ResumeTarget::Open(s.key),
        });
        let mut past: Vec<(SystemTime, _)> = self
            .past
            .values()
            .flatten()
            .filter(|info| self.ours.contains(&info.session_id.to_string()) && !self.archived.contains(&info.session_id.to_string()))
            .filter(|info| !self.sessions.iter().any(|s| s.id.as_ref() == Some(&info.session_id)))
            .map(|info| (info.updated_at.as_deref().and_then(when::parse_iso8601).unwrap_or(SystemTime::UNIX_EPOCH), info))
            .collect();
        past.sort_by(|a, b| b.0.cmp(&a.0));
        let past = past.into_iter().map(|(at, info)| Resume {
            title: self.titles.get(&info.session_id.to_string()).cloned().or(info.title.clone()).unwrap_or_else(|| "Earlier session".into()),
            folder: folder_name(&info.cwd),
            notebook: recorded(&info.session_id),
            when: if at == SystemTime::UNIX_EPOCH { String::new() } else { when::ago(at) },
            open: ResumeTarget::Past(info.clone()),
        });
        open.chain(past).take(RESUME_SHOWN).collect()
    }

    pub fn render_new_session(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let resume = self.resumable();
        let rows = resume.into_iter().enumerate().map(|(i, r)| {
            let meta = std::iter::once(r.folder).chain(r.notebook).collect::<Vec<_>>().join(" · ");
            div()
                .id(("resume", i))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(30.))
                .px(px(4.))
                .mx(px(-4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::row_active()))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(theme::text_row_active()).child(r.title))
                .child(div().flex_shrink_0().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(meta))
                .child(div().flex_1())
                .child(div().flex_shrink_0().text_size(theme::size_meta()).text_color(theme::text_faint()).child(r.when))
                .on_click(cx.listener(move |this, _, _, cx| match &r.open {
                    ResumeTarget::Open(key) => this.activate(*key, cx),
                    ResumeTarget::Past(info) => this.open_past(info.clone(), cx),
                }))
        });
        let rows: Vec<_> = rows.collect();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .px(px(20.))
                    .pt(px(28.))
                    .child(div().text_size(theme::size_title()).font_weight(FontWeight::SEMIBOLD).child("Start a session"))
                    .when(!rows.is_empty(), |d| {
                        d.child(div().mt(px(18.)).mb(px(4.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child("Pick up where you left off"))
                            .children(rows)
                    }),
            )
            .child(
                div()
                    .px_4()
                    .pb(px(11.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(self.render_chips(cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_h(px(76.))
                            .pl(px(10.))
                            .pr(px(8.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme::composer_edge())
                            .bg(theme::bg_card())
                            .child(div().flex_1().child(Textarea::new(&self.input).appearance(false).text_size(theme::size_body())))
                            .child(
                                div().flex().justify_end().child(
                                    div()
                                        .id("start-session")
                                        .role(Role::Button)
                                        .aria_label("Start session")
                                        .size(px(24.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .cursor_pointer()
                                        .bg(theme::accent())
                                        .text_color(theme::text_primary())
                                        .text_size(theme::size_meta())
                                        .child("↑")
                                        .on_click(cx.listener(|this, _, window, cx| this.start_session(window, cx))),
                                ),
                            ),
                    ),
            )
    }

    fn render_chips(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let notebook_label = match &self.draft.notebook {
            NotebookChoice::New => "New notebook".to_string(),
            NotebookChoice::Existing(path) => folder_name(path),
        };
        let mono = matches!(self.draft.notebook, NotebookChoice::Existing(_));
        let chips = [
            (Chip::Where, "where", Glyph::Laptop, "This Mac".to_string(), false),
            (Chip::Folder, "folder", Glyph::Folder, folder_name(&self.draft.folder), false),
            (Chip::Notebook, "notebook", Glyph::File, notebook_label, mono),
        ];
        div().flex().gap(px(6.)).children(chips.map(|(chip, id, icon, label, mono)| {
            let open = self.draft.popover == Some(chip);
            div()
                .relative()
                .child(
                    div()
                        .id(id)
                        .role(Role::Button)
                        .h(px(26.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .bg(if open { theme::bg_raised() } else { theme::bg_tag() })
                        .hover(|s| s.bg(theme::bg_raised()))
                        .text_size(theme::size_meta())
                        .text_color(theme::text_secondary())
                        .child(glyph(icon, theme::text_muted()))
                        .child(div().when(mono, |d| d.font_family(theme::MONO)).child(label))
                        .child(glyph(Glyph::Chevron, theme::text_faint()))
                        .on_click(cx.listener(move |this, _, window, cx| this.toggle_popover(chip, window, cx))),
                )
                .when(open, |d| d.child(self.render_popover(chip, cx)))
        }))
    }

    /// A chip's menu, opening upward from the chip's top-left corner.
    fn render_popover(&self, chip: Chip, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (width, body) = match chip {
            Chip::Where => (220., self.where_menu().into_any_element()),
            Chip::Folder => (360., self.folder_menu(cx).into_any_element()),
            Chip::Notebook => (320., self.notebook_menu(cx).into_any_element()),
        };
        let body = div()
            .id("chip-menu")
            .occlude()
            .w(px(width))
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
            .when(chip == Chip::Folder, |d| d.capture_key_down(cx.listener(Self::folder_popover_key)))
            .child(body);
        div().absolute().top(px(-6.)).left_0().child(deferred(anchored().anchor(Anchor::BottomLeft).child(body)).with_priority(1))
    }

    fn where_menu(&self) -> impl IntoElement {
        menu_row("where-this-mac", true, false).child(glyph(Glyph::Laptop, theme::text_muted())).child("This Mac")
    }

    fn folder_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let matches = self.folder_matches(cx);
        let rows = matches.into_iter().enumerate().map(|(i, folder)| {
            let current = folder == self.draft.folder;
            let path = tilde(&folder);
            menu_row(("folder-row", i), current, self.draft.selected == i)
                .child(glyph(Glyph::Folder, theme::text_muted()))
                .child(div().flex_shrink_0().child(folder_name(&folder)))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_right().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(path))
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.draft.selected != i {
                        this.draft.selected = i;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| this.set_draft_folder(folder.clone(), window, cx)))
        });
        let rows: Vec<_> = rows.collect();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .m(px(4.))
                    .mb(px(6.))
                    .px(px(8.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme::accent())
                    .bg(theme::bg_card())
                    .child(glyph(Glyph::Search, theme::text_faint()))
                    .child(div().flex_1().child(Input::new(&self.draft.search).appearance(false).text_size(theme::size_body()))),
            )
            .child(section_label("Recent"))
            .when(rows.is_empty(), |d| d.child(div().px(px(8.)).py(px(4.)).pl(px(28.)).text_color(theme::text_faint()).child("No matching folders")))
            .children(rows)
            .child(div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::composer_edge()))
            .child(
                menu_row("browse", false, false)
                    .child(glyph(Glyph::Folder, theme::text_muted()))
                    .child("Browse…")
                    .child(div().flex_1().text_right().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(tilde(&browse_start())))
                    .on_click(cx.listener(|this, _, window, cx| this.browse_folder(window, cx))),
            )
    }

    fn notebook_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let folder = self.draft.folder.clone();
        let rows = self.draft.notebooks.iter().enumerate().map(|(i, found)| {
            let chosen = self.draft.notebook == NotebookChoice::Existing(found.path.clone());
            let relative = found.path.strip_prefix(&folder).unwrap_or(&found.path);
            let dir = relative.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| format!("{}/", p.display()));
            let path = found.path.clone();
            menu_row(("notebook-row", i), chosen, false)
                .child(glyph(Glyph::File, theme::text_muted()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .font_family(theme::MONO)
                        .text_size(theme::size_code())
                        .children(dir.map(|d| div().text_color(theme::text_faint()).child(d)))
                        .child(folder_name(&found.path)),
                )
                .child(div().flex_shrink_0().text_size(theme::size_meta()).text_color(theme::text_faint()).child(when::ago(found.modified)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose_notebook(NotebookChoice::Existing(path.clone()), cx);
                    this.close_popover(window, cx);
                }))
        });
        let rows: Vec<_> = rows.collect();
        div()
            .flex()
            .flex_col()
            .child(
                menu_row("new-notebook", self.draft.notebook == NotebookChoice::New, false)
                    .child(glyph(Glyph::File, theme::text_muted()))
                    .child("New notebook")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.choose_notebook(NotebookChoice::New, cx);
                        this.close_popover(window, cx);
                    })),
            )
            .child(section_label(format!("In {}", folder_name(&folder))))
            .when(rows.is_empty(), |d| d.child(div().py(px(4.)).pl(px(28.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child("No Pluto notebooks in this folder")))
            .child(div().id("notebook-rows").max_h(px(300.)).overflow_y_scroll().flex().flex_col().children(rows))
    }

    /// The notebook pane's header before the session starts.
    pub fn draft_pane_header(&self) -> AnyElement {
        match &self.draft.notebook {
            NotebookChoice::New => div().text_color(theme::text_muted()).child("New notebook").into_any_element(),
            NotebookChoice::Existing(path) => div()
                .flex()
                .min_w_0()
                .items_baseline()
                .gap_2()
                .font_family(theme::MONO)
                .child(div().flex_shrink_0().text_size(theme::size_meta()).text_color(theme::text_muted()).child(folder_name(path)))
                .children(path.parent().map(|dir| {
                    div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(theme::size_meta_small()).text_color(theme::text_section()).child(tilde(dir))
                }))
                .into_any_element(),
        }
    }

    /// The notebook pane before the session starts (the web view is hidden).
    pub fn render_draft_pane(&self) -> AnyElement {
        match (&self.draft.notebook, &self.draft.preview) {
            (NotebookChoice::New, _) => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.))
                .child(canvas(|_, _, _| (), |bounds, _, window, _| turtle::paint(window, point(bounds.left() + px(TURTLE_R), bounds.bottom()), TURTLE_R, &Pose::default())).w(px(TURTLE_R * 2.4)).h(px(TURTLE_R * 1.25)))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .text_color(theme::text_muted())
                        .child("A new notebook will be created in ")
                        .child(div().font_family(theme::MONO).text_size(theme::size_code()).text_color(theme::text_secondary()).child(folder_name(&self.draft.folder)))
                        .child(" when you start."),
                )
                .into_any_element(),
            (NotebookChoice::Existing(_), None) => div().into_any_element(),
            (NotebookChoice::Existing(_), Some(preview)) => {
                let cells = preview.cells.iter().map(|cell| {
                    div()
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(6.))
                        .bg(theme::bg_card())
                        .font_family(theme::MONO)
                        .text_size(theme::size_code())
                        .text_color(theme::text_primary())
                        .child(cell.code.replace('\t', "    "))
                        .when(cell.clipped, |d| d.child(div().text_color(theme::text_faint()).child("⋯")))
                });
                let note = match preview.total {
                    0 => Some("This notebook has no cells.".to_string()),
                    n if n > preview.cells.len() => Some(format!("Showing the first {} of {n} cells.", preview.cells.len())),
                    _ => None,
                };
                div()
                    .id("draft-preview")
                    .size_full()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px_4()
                    .pb_4()
                    .child(
                        div()
                            .self_start()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .mb(px(4.))
                            .px(px(10.))
                            .py(px(3.))
                            .rounded_full()
                            .border_1()
                            .border_color(theme::composer_edge())
                            .text_size(theme::size_meta())
                            .text_color(theme::text_secondary())
                            .child(div().size(px(6.)).rounded_full().bg(theme::accent_text()))
                            .child("Safe preview · nothing runs until you allow it"),
                    )
                    .children(cells)
                    .children(note.map(|n| div().mt(px(4.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child(n)))
                    .into_any_element()
            }
        }
    }
}

const TURTLE_R: f32 = 22.;

/// A chip-menu row: a ✓ column, then the caller's icon and text.
pub(crate) fn menu_row(id: impl Into<ElementId>, checked: bool, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .px(px(8.))
        .rounded(px(5.))
        .cursor_pointer()
        .when(selected, |d| d.bg(theme::composer_edge()))
        .hover(|s| s.bg(theme::composer_edge()))
        .child(div().w(px(10.)).flex_shrink_0().text_size(theme::size_meta()).text_color(theme::accent_text()).child(if checked { "✓" } else { "" }))
}

fn section_label(text: impl Into<SharedString>) -> impl IntoElement {
    div().pl(px(26.)).pt(px(4.)).pb(px(2.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(text.into())
}

#[derive(Clone, Copy)]
pub(crate) enum Glyph {
    Laptop,
    Folder,
    File,
    Chevron,
    Search,
    Funnel,
    Archive,
}

/// A 12px line icon (the app ships no icon set).
pub(crate) fn glyph(glyph: Glyph, color: Rgba) -> impl IntoElement {
    const SIZE: f32 = 12.;
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let at = |x: f32, y: f32| point(b.left() + px(x), b.top() + px(y));
            let mut path = PathBuilder::stroke(px(1.));
            let mut polyline = |points: &[(f32, f32)]| {
                for (i, &(x, y)) in points.iter().enumerate() {
                    if i == 0 { path.move_to(at(x, y)) } else { path.line_to(at(x, y)) }
                }
            };
            match glyph {
                Glyph::Laptop => {
                    polyline(&[(2., 2.5), (10., 2.5), (10., 8.), (2., 8.), (2., 2.5)]);
                    polyline(&[(0.5, 10.), (11.5, 10.)]);
                }
                Glyph::Folder => polyline(&[(1., 3.), (1., 10.), (11., 10.), (11., 4.), (6., 4.), (5., 2.5), (1.5, 2.5), (1., 3.)]),
                Glyph::File => {
                    polyline(&[(2.5, 1.), (7., 1.), (9.5, 3.5), (9.5, 11.), (2.5, 11.), (2.5, 1.)]);
                    polyline(&[(4.5, 6.), (7.5, 6.)]);
                    polyline(&[(4.5, 8.5), (7.5, 8.5)]);
                }
                Glyph::Chevron => polyline(&[(3.5, 5.), (6., 7.5), (8.5, 5.)]),
                Glyph::Search => {
                    let circle: Vec<(f32, f32)> = (0..=24).map(|i| {
                        let a = std::f32::consts::TAU * i as f32 / 24.;
                        (5. + 3.5 * a.cos(), 5. + 3.5 * a.sin())
                    }).collect();
                    polyline(&circle);
                    polyline(&[(7.6, 7.6), (11., 11.)]);
                }
                Glyph::Funnel => polyline(&[(1.5, 2.), (10.5, 2.), (7., 6.5), (7., 10.5), (5., 9.5), (5., 6.5), (1.5, 2.)]),
                Glyph::Archive => {
                    polyline(&[(1., 2.), (11., 2.), (11., 4.5), (1., 4.5), (1., 2.)]);
                    polyline(&[(2., 4.5), (2., 10.5), (10., 10.5), (10., 4.5)]);
                    polyline(&[(4.5, 6.5), (7.5, 6.5)]);
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .flex_shrink_0()
    .size(px(SIZE))
}
