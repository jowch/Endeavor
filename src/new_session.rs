//! The new-session screen: recent sessions to pick up, chips for where the
//! session runs, its folder and its notebook, and the shared composer. The
//! notebook pane shows a native stand-in (Pluto isn't involved until the
//! session starts): an empty state for a new notebook, or a static preview of
//! the chosen one's first cells. On a server, picking it connects (Julia waits
//! for the session), and its folders and notebooks come through the helper.
//! On a cluster, a resources chip sets what the session's job asks for.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState, Textarea};
use wire::files::{self, Entry, Reply, Request};
use wire::notebooks::{Found, Preview};

use wire::slurm::{Resources, duration_text};

use crate::connection::Status;
use crate::hosts::{Cluster, HostId, Place, Server};
use crate::resources::Target;
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
    /// The in-app folder browser for a server's disk.
    Browse,
    /// A cluster job's resources.
    Resources,
}

/// The new-session screen's state.
pub struct Draft {
    /// The machine the session runs on.
    pub host: HostId,
    /// A short note above the chips (why pressing send did nothing yet).
    pub notice: Option<SharedString>,
    /// The folder on `host`; unknown until a server has said where home is.
    pub folder: Option<PathBuf>,
    pub notebook: NotebookChoice,
    pub popover: Option<Chip>,
    /// Notebooks found in `folder`, newest first.
    pub notebooks: Vec<Found>,
    /// The chosen existing notebook's first cells, once read.
    pub preview: Option<Preview>,
    /// The folder popover's search box, and its highlighted row.
    pub search: Entity<InputState>,
    pub selected: usize,
    pub browser: Option<Browser>,
    /// On a cluster: what this session's job asks for.
    pub resources: Option<Resources>,
    pub partition_menu: bool,
    /// The resources popover's "Paste an salloc line…" box, while it's open.
    pub salloc: Option<Entity<InputState>>,
    pub salloc_error: Option<String>,
}

/// The server folder browser: the folder shown, and its folders and notebooks once listed.
pub struct Browser {
    pub path: PathBuf,
    pub listing: Option<Result<Vec<Entry>, String>>,
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
    Past(agent_client_protocol::schema::v1::SessionInfo, Place),
}

const RESUME_SHOWN: usize = 3;

/// The folder a new session on This Mac starts in: the last one used there,
/// else ~/Documents/Endeavor, created at launch so the folder chip and
/// Browse… have somewhere real to point.
pub fn default_folder(recent: &[Place]) -> PathBuf {
    if let Some(place) = recent.iter().find(|p| p.host == HostId::ThisMac) {
        return place.path.clone();
    }
    let folder = home().join("Documents/Endeavor");
    let _ = std::fs::create_dir_all(&folder);
    folder
}

fn home() -> PathBuf {
    files::home()
}

/// `path` with `home` written as `~`.
fn tilde_of(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// `path` with This Mac's home folder written as `~`.
fn tilde(path: &Path) -> String {
    tilde_of(path, &home())
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
        Draft {
            host: HostId::ThisMac,
            notice: None,
            folder: Some(folder),
            notebook: NotebookChoice::New,
            popover: None,
            notebooks: Vec::new(),
            preview: None,
            search,
            selected: 0,
            browser: None,
            resources: None,
            partition_menu: false,
            salloc: None,
            salloc_error: None,
        }
    }
}

impl Workspace {
    /// Ask `host` about its files: This Mac answers here, a server through its
    /// helper (once connected). Off the main thread either way.
    fn ask_files(&self, host: &HostId, request: Request, cx: &mut Context<Self>) -> Option<Task<Result<Reply, String>>> {
        match host {
            HostId::ThisMac => Some(cx.background_spawn(async move {
                match files::answer(&request) {
                    Reply::Error { message } => Err(message),
                    reply => Ok(reply),
                }
            })),
            HostId::Server(_) => {
                let channel = self.connection(host)?.channel.clone()?;
                Some(cx.background_spawn(async move { channel.files(request) }))
            }
        }
    }

    /// The draft host's cluster settings, if it's a cluster.
    pub fn draft_cluster(&self) -> Option<&Cluster> {
        match &self.draft.host {
            HostId::Server(id) => self.hosts.server(id)?.cluster.as_ref(),
            HostId::ThisMac => None,
        }
    }

    /// Paste an salloc line: open its box, or apply what's in it.
    fn paste_salloc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.draft.salloc.clone() else {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("salloc -p gpu -c 8 --mem=32G -t 8:00:00"));
            cx.subscribe_in(&input, window, |this: &mut Workspace, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.paste_salloc(window, cx);
                }
            })
            .detach();
            input.update(cx, |s, cx| s.focus(window, cx));
            self.draft.salloc = Some(input);
            self.draft.salloc_error = None;
            return cx.notify();
        };
        let line = input.read(cx).value().to_string();
        let base = self.draft.resources.clone().unwrap_or_default();
        match wire::slurm::parse_salloc(&line, &base) {
            Ok((mut resources, account)) => {
                if let Some(account) = account {
                    resources.extra.insert(0, format!("--account={account}"));
                }
                let partitions = self.draft_cluster().map(|c| c.partitions.clone()).unwrap_or_default();
                let known = |name: &String| partitions.is_empty() || partitions.iter().any(|p| &p.name == name);
                if let Some(name) = resources.partition.as_ref().filter(|n| !known(n)) {
                    self.draft.salloc_error = Some(format!("This cluster has no partition \"{name}\"."));
                    return cx.notify();
                }
                let partition = partitions.iter().find(|p| Some(&p.name) == resources.partition.as_ref()).or_else(|| partitions.iter().find(|p| p.default));
                resources.clip(partition);
                self.draft.resources = Some(resources);
                self.draft.salloc = None;
                self.draft.salloc_error = None;
            }
            Err(e) => self.draft.salloc_error = Some(e),
        }
        cx.notify();
    }

    /// Ask a connected cluster for its partitions, when the last Test
    /// connection didn't list them (the resources chip needs them).
    pub fn fetch_partitions(&mut self, host: &HostId, cx: &mut Context<Self>) {
        let HostId::Server(id) = host else { return };
        if self.hosts.server(id).and_then(|s| s.cluster.as_ref()).is_none_or(|c| !c.partitions.is_empty()) {
            return;
        }
        let Some(ask) = self.ask_files(host, Request::Slurm, cx) else { return };
        let id = id.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Reply::Slurm { scheduler }) = ask.await else { return };
            let _ = this.update(cx, |this, cx| {
                let Some(server) = this.hosts.servers.iter_mut().find(|s| s.id == id) else { return };
                let Some(cluster) = server.cluster.as_mut() else { return };
                cluster.partitions = scheduler.partitions;
                cluster.scratch = scheduler.scratch;
                let _ = this.hosts.save();
                cx.notify();
            });
        })
        .detach();
    }

    /// The draft host's home folder: This Mac's, or what a connected server said.
    fn draft_home(&self) -> Option<PathBuf> {
        match &self.draft.host {
            HostId::ThisMac => Some(home()),
            host => self.connection(host)?.hello.as_ref().map(|h| h.home.clone()),
        }
    }

    fn draft_tilde(&self, path: &Path) -> String {
        match self.draft_home() {
            Some(home) => tilde_of(path, &home),
            None => path.display().to_string(),
        }
    }

    /// Look for notebooks in the draft's folder.
    pub fn scan_notebooks(&mut self, cx: &mut Context<Self>) {
        let (host, Some(folder)) = (self.draft.host.clone(), self.draft.folder.clone()) else { return };
        let Some(scan) = self.ask_files(&host, Request::Notebooks { path: folder.display().to_string() }, cx) else { return };
        cx.spawn(async move |this, cx| {
            let found = scan.await;
            let _ = this.update(cx, |this, cx| {
                if this.draft.host == host && this.draft.folder.as_ref() == Some(&folder) {
                    this.draft.notebooks = match found {
                        Ok(Reply::Notebooks { found }) => found,
                        _ => Vec::new(),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_draft_folder(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if Some(&folder) != self.draft.folder.as_ref() {
            self.draft.folder = Some(folder);
            self.draft.notebooks.clear();
            self.choose_notebook(NotebookChoice::New, cx);
            self.scan_notebooks(cx);
        }
        self.close_popover(window, cx);
    }

    pub fn choose_notebook(&mut self, choice: NotebookChoice, cx: &mut Context<Self>) {
        self.draft.preview = None;
        self.draft.notebook = choice.clone();
        if let NotebookChoice::Existing(path) = choice
            && let Some(read) = self.ask_files(&self.draft.host.clone(), Request::Preview { path: path.display().to_string() }, cx)
        {
            cx.spawn(async move |this, cx| {
                let preview = match read.await {
                    Ok(Reply::Preview { preview }) => preview,
                    _ => Preview::default(),
                };
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
            Chip::Resources => {
                self.draft.partition_menu = false;
                self.draft.salloc = None;
                self.draft.salloc_error = None;
            }
            Chip::Where | Chip::Browse => {}
        }
        cx.notify();
    }

    /// Close the open chip menu; the composer gets the keyboard back.
    pub fn close_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.popover.take().is_some() {
            self.input.update(cx, |s, cx| s.focus(window, cx));
        }
        self.draft.browser = None;
        cx.notify();
    }

    /// Browse…: the macOS panel for This Mac; for a server, the in-app browser
    /// starting at the chosen folder (or home).
    fn browse_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.host != HostId::ThisMac {
            let Some(start) = self.draft.folder.clone().or_else(|| self.draft_home()) else { return };
            self.draft.popover = Some(Chip::Browse);
            return self.browse_to(start, cx);
        }
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

    /// Show `path` in the server folder browser.
    fn browse_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let host = self.draft.host.clone();
        self.draft.browser = Some(Browser { path: path.clone(), listing: None });
        let Some(list) = self.ask_files(&host, Request::List { path: path.display().to_string() }, cx) else { return };
        cx.spawn(async move |this, cx| {
            let listed = list.await;
            let _ = this.update(cx, |this, cx| {
                let Some(browser) = this.draft.browser.as_mut().filter(|b| b.path == path) else { return };
                match listed {
                    Ok(Reply::List { path, entries }) => {
                        browser.path = path;
                        browser.listing = Some(Ok(entries));
                    }
                    Ok(_) => browser.listing = Some(Err("The server gave an unexpected answer.".into())),
                    Err(e) => browser.listing = Some(Err(e)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Recent folders on the draft's host (the current one first if it isn't
    /// recent) matching the search.
    fn folder_matches(&self, cx: &App) -> Vec<PathBuf> {
        let query = self.draft.search.read(cx).value().trim().to_lowercase();
        let host = &self.draft.host;
        let recent: Vec<PathBuf> = self.recent.iter().filter(|p| p.host == *host).map(|p| p.path.clone()).collect();
        let current = self.draft.folder.clone().filter(|f| !recent.contains(f));
        current
            .into_iter()
            .chain(recent)
            .filter(|p| query.is_empty() || format!("{}\n{}", p.display(), self.draft_tilde(p)).to_lowercase().contains(&query))
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
        let recorded = |id: &agent_client_protocol::schema::v1::SessionId| self.session_notebooks.get(&id.to_string()).map(|place| folder_name(&place.path));
        let open = self.sessions.iter().rev().map(|s| Resume {
            title: s.title.clone(),
            folder: self.folder_heading(&s.place),
            notebook: s.notebook_path.as_deref().map(|p| folder_name(Path::new(p))).or_else(|| s.id.as_ref().and_then(recorded)),
            when: "open".into(),
            open: ResumeTarget::Open(s.key),
        });
        let mut past: Vec<(SystemTime, _, Place)> = self
            .past
            .iter()
            .flat_map(|(place, infos)| infos.iter().map(move |info| (place, info)))
            .filter(|(_, info)| self.ours.contains_key(&info.session_id.to_string()) && !self.archived.contains(&info.session_id.to_string()))
            .filter(|(_, info)| !self.sessions.iter().any(|s| s.id.as_ref() == Some(&info.session_id)))
            .map(|(place, info)| (info.updated_at.as_deref().and_then(when::parse_iso8601).unwrap_or(SystemTime::UNIX_EPOCH), info, place.clone()))
            .collect();
        past.sort_by(|a, b| b.0.cmp(&a.0));
        let past = past.into_iter().map(|(at, info, place)| Resume {
            title: self.titles.get(&info.session_id.to_string()).cloned().or(info.title.clone()).unwrap_or_else(|| "Earlier session".into()),
            folder: self.folder_heading(&place),
            notebook: recorded(&info.session_id),
            when: if at == SystemTime::UNIX_EPOCH { String::new() } else { when::ago(at) },
            open: ResumeTarget::Past(info.clone(), place),
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
                    ResumeTarget::Past(info, place) => this.open_past(info.clone(), place.clone(), cx),
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

    /// The draft host's connection trouble, in plain words, with Retry.
    fn connection_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let host = self.draft.host.clone();
        let name = self.hosts.name(&host);
        let text = match self.status(&host)? {
            Status::Failed(reason) => format!("Couldn't connect to {name}. {reason}"),
            Status::Replaced => format!("Another connection took over {name}."),
            _ => return None,
        };
        Some(
            div()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .text_size(theme::size_meta())
                .child(div().flex_1().text_color(theme::danger()).child(text))
                .child(
                    div()
                        .id("retry-connect")
                        .role(Role::Button)
                        .flex_shrink_0()
                        .px(px(8.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .bg(theme::bg_raised())
                        .text_color(theme::text_primary())
                        .child("Retry")
                        .on_click(cx.listener(move |this, _, _, cx| this.connect_host(&host, false, cx))),
                )
                .into_any_element(),
        )
    }

    fn render_chips(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let connecting = self.draft.host != HostId::ThisMac && self.draft.folder.is_none();
        let notebook_label = match &self.draft.notebook {
            NotebookChoice::New => "New notebook".to_string(),
            NotebookChoice::Existing(path) => folder_name(path),
        };
        let mono = matches!(self.draft.notebook, NotebookChoice::Existing(_));
        let (where_icon, where_label) = match &self.draft.host {
            HostId::Server(_) if self.draft_cluster().is_some() => (Glyph::Cluster, self.hosts.name(&self.draft.host)),
            HostId::Server(_) => (Glyph::Server, self.hosts.name(&self.draft.host)),
            HostId::ThisMac => (Glyph::Laptop, "This Mac".to_string()),
        };
        let resources = self.draft.resources.as_ref().filter(|_| self.draft_cluster().is_some()).map(|r| (Chip::Resources, "resources", Glyph::Chip, r.summary(), false));
        let folder_label = match (&self.draft.folder, self.status(&self.draft.host)) {
            (Some(folder), _) => folder_name(folder),
            (None, Some(Status::Failed(_) | Status::Replaced)) => "Not connected".into(),
            (None, _) => format!("Connecting to {where_label}…"),
        };
        let chips = std::iter::once((Chip::Where, "where", where_icon, where_label, false))
            .chain(resources)
            .chain([(Chip::Folder, "folder", Glyph::Folder, folder_label, false), (Chip::Notebook, "notebook", Glyph::File, notebook_label, mono)]);
        let chips = div().flex().flex_wrap().gap(px(6.)).children(chips.map(|(chip, id, icon, label, mono)| {
            let open = self.draft.popover == Some(chip) || (chip == Chip::Folder && self.draft.popover == Some(Chip::Browse));
            let waiting = connecting && !matches!(chip, Chip::Where | Chip::Resources);
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
                        .bg(if open { theme::bg_raised() } else { theme::bg_tag() })
                        .text_size(theme::size_meta())
                        .text_color(if waiting { theme::text_faint() } else { theme::text_secondary() })
                        .child(glyph(icon, theme::text_muted()))
                        .child(div().when(mono, |d| d.font_family(theme::MONO)).child(label))
                        .when(!waiting, |d| {
                            d.cursor_pointer()
                                .hover(|s| s.bg(theme::bg_raised()))
                                .child(glyph(Glyph::Chevron, theme::text_faint()))
                                .on_click(cx.listener(move |this, _, window, cx| this.toggle_popover(chip, window, cx)))
                        }),
                )
                .when(open, |d| d.child(self.render_popover(self.draft.popover.unwrap_or(chip), cx)))
        }));
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(self.draft.notice.clone().map(|n| div().text_size(theme::size_meta()).text_color(theme::accent_text()).child(n)))
            .children(self.connection_notice(cx))
            .child(chips)
    }

    /// A chip's menu, opening upward from the chip's top-left corner.
    fn render_popover(&self, chip: Chip, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (width, body) = match chip {
            Chip::Where => (240., self.where_menu(cx).into_any_element()),
            Chip::Folder => (360., self.folder_menu(cx).into_any_element()),
            Chip::Notebook => (320., self.notebook_menu(cx).into_any_element()),
            Chip::Browse => (380., self.browser_menu(cx).into_any_element()),
            Chip::Resources => (300., self.resources_menu(cx).into_any_element()),
        };
        let body = div()
            .id("chip-menu")
            .occlude()
            .w(px(width))
            .p(px(if chip == Chip::Resources { 12. } else { 4. }))
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

    /// Pick where the session runs. A server connects now, so its folders can be
    /// browsed; its folder is the last one used there, else its home.
    pub fn set_draft_host(&mut self, host: HostId, window: &mut Window, cx: &mut Context<Self>) {
        if host != self.draft.host {
            self.draft.folder = match &host {
                HostId::ThisMac => Some(default_folder(&self.recent)),
                _ => self.recent.iter().find(|p| p.host == host).map(|p| p.path.clone()),
            };
            self.draft.host = host.clone();
            self.draft.resources = self.draft_cluster().map(|c| c.resources.clone());
            self.draft.notebooks.clear();
            self.choose_notebook(NotebookChoice::New, cx);
            if host != HostId::ThisMac {
                self.connect_host(&host, false, cx);
            }
            if self.draft.folder.is_none() {
                self.draft.folder = self.draft_home();
            }
            self.scan_notebooks(cx);
        }
        self.draft.notice = None;
        self.close_popover(window, cx);
    }

    fn where_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this_mac = host_row("where-this-mac", self.draft.host == HostId::ThisMac, Glyph::Laptop, "This Mac".into(), None, "host-gear-this-mac", cx)
            .on_click(cx.listener(|this, _, window, cx| this.set_draft_host(HostId::ThisMac, window, cx)));
        let this_mac = this_mac.child(gear_button("gear-this-mac", "host-gear-this-mac").on_click(cx.listener(|this, _, window, cx| {
            cx.stop_propagation();
            this.close_popover(window, cx);
            this.settings_open = true;
        })));
        let rows = |clusters: bool, cx: &mut Context<Self>| -> Vec<Stateful<Div>> {
            self.hosts
                .servers
                .iter()
                .enumerate()
                .filter(|(_, server)| server.cluster.is_some() == clusters)
                .map(|(i, server)| {
                    let group: SharedString = format!("host-gear-{i}").into();
                    let host = HostId::Server(server.id.clone());
                    let state = match self.status(&host) {
                        Some(Status::Connecting) => Some("Connecting…"),
                        Some(Status::Browsing | Status::Starting | Status::Ready | Status::Died(_)) if !clusters => Some("Connected"),
                        Some(Status::Failed(_) | Status::Replaced) => Some("Not connected"),
                        _ if clusters => Some("Slurm"),
                        _ => None,
                    };
                    let (pick, edit_id) = (host.clone(), server.id.clone());
                    let icon = if clusters { Glyph::Cluster } else { Glyph::Server };
                    host_row(("where-server", i), self.draft.host == host, icon, server.name.clone(), state, group.clone(), cx)
                        .on_click(cx.listener(move |this, _, window, cx| this.set_draft_host(pick.clone(), window, cx)))
                        .child(gear_button(("gear-server", i), group).on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_popover(window, cx);
                            this.open_server_dialog(Some(edit_id.clone()), window, cx);
                        })))
                })
                .collect()
        };
        let servers = rows(false, cx);
        let clusters = rows(true, cx);
        let add = |id: &'static str, text: &'static str, cluster: bool, cx: &mut Context<Self>| {
            menu_row(id, false, false).child(glyph(Glyph::Plus, theme::text_muted())).child(div().text_color(theme::text_muted()).child(text)).on_click(cx.listener(move |this, _, window, cx| {
                this.close_popover(window, cx);
                let template = Server { cluster: cluster.then(Cluster::default), ..Default::default() };
                this.open_new_host(template, window, cx);
            }))
        };
        div()
            .flex()
            .flex_col()
            .child(this_mac)
            .child(section_label("Servers"))
            .children(servers)
            .child(add("add-server", "Add server…", false, cx))
            .child(section_label("Clusters"))
            .children(clusters)
            .child(add("add-cluster", "Add cluster…", true, cx))
    }

    /// The resources chip's popover: presets, partition, CPUs, memory, time
    /// limit, and pasting an salloc line.
    fn resources_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (Some(r), Some(cluster)) = (self.draft.resources.as_ref(), self.draft_cluster()) else { return div() };
        let partition = cluster.partition(r.partition.as_deref());
        let limit = partition.and_then(|p| p.max_minutes).map(|m| format!(" This partition allows up to {}.", duration_text(m))).unwrap_or_default();
        let running = self.connection(&self.draft.host).filter(|c| c.status == Status::Ready).and_then(|c| c.runtime.as_ref()?.job.clone());
        let note = match running {
            Some(job) => format!(
                "Julia already runs in job {} on {}{}; a new session joins it.",
                job.id,
                job.node,
                job.ends_at.map(|at| format!(" until {}", crate::when::clock(at))).unwrap_or_default()
            ),
            None => format!("Notebooks stop when the job's time runs out.{limit}"),
        };
        let extra = (!r.extra.is_empty()).then(|| {
            div().pt(px(2.)).font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(format!("Also: {}", r.extra.join(" ")))
        });
        let salloc = match &self.draft.salloc {
            None => div()
                .id("paste-salloc")
                .cursor_pointer()
                .text_size(theme::size_meta())
                .text_color(theme::accent_text())
                .child("Paste an salloc line…")
                .on_click(cx.listener(|this, _, window, cx| this.paste_salloc(window, cx)))
                .into_any_element(),
            Some(input) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .h(px(28.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme::accent())
                        .bg(theme::bg_card())
                        .font_family(theme::MONO)
                        .child(div().flex_1().child(Input::new(input).appearance(false).text_size(theme::size_code()))),
                )
                .children(self.draft.salloc_error.clone().map(|e| div().text_size(theme::size_meta()).text_color(theme::danger()).child(e)))
                .child(
                    div().flex().justify_end().child(
                        div()
                            .id("apply-salloc")
                            .role(Role::Button)
                            .px(px(10.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .bg(theme::accent())
                            .text_color(gpui::white())
                            .text_size(theme::size_meta())
                            .child("Use these")
                            .on_click(cx.listener(|this, _, window, cx| this.paste_salloc(window, cx))),
                    ),
                )
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .children(self.resource_rows(Target::Draft, r, &cluster.partitions, self.draft.partition_menu, true, cx))
            .children(extra)
            .child(div().pt(px(6.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child(note))
            .child(div().h(px(1.)).my(px(8.)).bg(theme::composer_edge()))
            .child(salloc)
    }

    fn folder_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let matches = self.folder_matches(cx);
        let rows = matches.into_iter().enumerate().map(|(i, folder)| {
            let current = Some(&folder) == self.draft.folder.as_ref();
            let path = self.draft_tilde(&folder);
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
        let browse_hint = match &self.draft.host {
            HostId::ThisMac => tilde(&browse_start()),
            _ => self.draft.folder.as_deref().map(|f| self.draft_tilde(f)).unwrap_or_default(),
        };
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
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_right().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(browse_hint))
                    .on_click(cx.listener(|this, _, window, cx| this.browse_folder(window, cx))),
            )
    }

    /// A server's folder browser: where it is (each part a way back up), its
    /// folders to open, its notebooks for orientation, and "Choose this folder".
    fn browser_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(browser) = &self.draft.browser else { return div() };
        let path = browser.path.clone();
        let mut crumbs: Vec<(String, PathBuf)> = path.ancestors().map(|p| (folder_name(p), p.to_path_buf())).collect();
        crumbs.reverse();
        let crumbs = crumbs.into_iter().enumerate().flat_map(|(i, (name, at))| {
            let sep = (i > 1).then(|| div().text_color(theme::text_faint()).child("/").into_any_element());
            let name = if i == 0 { "/".to_owned() } else { name };
            let crumb = div()
                .id(("crumb", i))
                .px(px(3.))
                .rounded(px(3.))
                .cursor_pointer()
                .text_color(theme::text_secondary())
                .hover(|s| s.bg(theme::composer_edge()))
                .child(name)
                .on_click(cx.listener(move |this, _, _, cx| this.browse_to(at.clone(), cx)))
                .into_any_element();
            sep.into_iter().chain([crumb])
        });
        let up = path.parent().map(Path::to_path_buf).map(|parent| {
            menu_row("browse-up", false, false)
                .child(div().w(px(12.)).text_color(theme::text_muted()).child("↑"))
                .child(div().text_color(theme::text_muted()).child("Up"))
                .on_click(cx.listener(move |this, _, _, cx| this.browse_to(parent.clone(), cx)))
        });
        let rows: Vec<AnyElement> = match &browser.listing {
            None => vec![div().py(px(4.)).pl(px(28.)).text_color(theme::text_faint()).child("Loading…").into_any_element()],
            Some(Err(e)) => vec![div().py(px(4.)).px(px(8.)).text_size(theme::size_meta()).text_color(theme::danger()).child(e.clone()).into_any_element()],
            Some(Ok(entries)) if entries.is_empty() => vec![div().py(px(4.)).pl(px(28.)).text_color(theme::text_faint()).child("No folders here").into_any_element()],
            Some(Ok(entries)) => entries
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let name = div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis();
                    if entry.dir {
                        let into = path.join(&entry.name);
                        menu_row(("browse-row", i), false, false)
                            .child(glyph(Glyph::Folder, theme::text_muted()))
                            .child(name.child(entry.name.clone()))
                            .on_click(cx.listener(move |this, _, _, cx| this.browse_to(into.clone(), cx)))
                            .into_any_element()
                    } else {
                        // Notebooks show where they are; only folders open.
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(28.))
                            .px(px(8.))
                            .child(div().w(px(10.)))
                            .child(glyph(Glyph::File, theme::text_faint()))
                            .child(name.font_family(theme::MONO).text_size(theme::size_code()).text_color(theme::text_faint()).child(entry.name.clone()))
                            .into_any_element()
                    }
                })
                .collect(),
        };
        let choose = path.clone();
        div()
            .flex()
            .flex_col()
            .child(div().flex().flex_wrap().items_center().px(px(6.)).py(px(4.)).font_family(theme::MONO).text_size(theme::size_meta_small()).children(crumbs))
            .children(up)
            .child(div().id("browse-rows").max_h(px(300.)).overflow_y_scroll().flex().flex_col().children(rows))
            .child(div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::composer_edge()))
            .child(
                div().flex().justify_end().p(px(4.)).child(
                    div()
                        .id("choose-folder")
                        .role(Role::Button)
                        .px(px(10.))
                        .h(px(26.))
                        .flex()
                        .items_center()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .bg(theme::accent())
                        .text_color(gpui::white())
                        .child("Choose this folder")
                        .on_click(cx.listener(move |this, _, window, cx| this.set_draft_folder(choose.clone(), window, cx))),
                ),
            )
    }

    fn notebook_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let folder = self.draft.folder.clone().unwrap_or_default();
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

    /// The new-session screen, set to start in `folder` on `notebook` (its preview shows).
    pub fn new_session_on(&mut self, folder: Place, notebook: PathBuf, cx: &mut Context<Self>) {
        self.active = None;
        self.settings_open = false;
        if folder.host != self.draft.host || Some(&folder.path) != self.draft.folder.as_ref() {
            self.draft.host = folder.host;
            self.draft.folder = Some(folder.path);
            self.draft.notebooks.clear();
        }
        self.scan_notebooks(cx);
        self.choose_notebook(NotebookChoice::Existing(notebook), cx);
    }

    /// The notebook pane's header before the session starts.
    pub fn draft_pane_header(&self) -> AnyElement {
        match &self.draft.notebook {
            NotebookChoice::New => div().text_color(theme::text_muted()).child("New notebook").into_any_element(),
            NotebookChoice::Existing(path) => notebook_title(path).into_any_element(),
        }
    }

    /// The notebook pane before the session starts (the web view is hidden).
    pub fn render_draft_pane(&self) -> AnyElement {
        match (&self.draft.notebook, &self.draft.preview) {
            (NotebookChoice::New, _) => {
                let line = div().flex().items_baseline().text_color(theme::text_muted());
                let line = match &self.draft.folder {
                    Some(folder) => line.child("A new notebook will be created in ").child(file_name(folder_name(folder))).child(" when you start."),
                    None => line.child(format!("Connecting to {}…", self.hosts.name(&self.draft.host))),
                };
                turtle_pane().child(line).into_any_element()
            }
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

/// The notebook pane's native stand-in: the resting turtle, above the caller's lines.
pub fn turtle_pane() -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(14.))
        .child(canvas(|_, _, _| (), |bounds, _, window, _| turtle::paint(window, point(bounds.left() + px(TURTLE_R), bounds.bottom()), TURTLE_R, &Pose::default())).w(px(TURTLE_R * 2.4)).h(px(TURTLE_R * 1.25)))
}

/// A file or folder name inside a line of text.
pub fn file_name(name: String) -> Div {
    div().font_family(theme::MONO).text_size(theme::size_code()).text_color(theme::text_secondary()).child(name)
}

/// A notebook pane header: the notebook's file name, then its folder, faint.
pub fn notebook_title(path: &Path) -> Div {
    div()
        .flex()
        .min_w_0()
        .items_baseline()
        .gap_2()
        .font_family(theme::MONO)
        .child(div().flex_shrink_0().text_size(theme::size_meta()).text_color(theme::text_muted()).child(folder_name(path)))
        .children(path.parent().map(|dir| {
            div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(theme::size_meta_small()).text_color(theme::text_section()).child(tilde(dir))
        }))
}

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

/// A Where menu row: ✓, the machine's icon and name, its connection state if
/// any, and room for its gear.
fn host_row(id: impl Into<ElementId>, checked: bool, icon: Glyph, name: String, state: Option<&'static str>, group: impl Into<SharedString>, _: &mut Context<Workspace>) -> Stateful<Div> {
    menu_row(id, checked, false)
        .group(group)
        .child(glyph(icon, theme::text_muted()))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
        .children(state.map(|s| div().flex_shrink_0().text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(s)))
}

/// A host's settings button, shown while the pointer is over its row.
fn gear_button(id: impl Into<ElementId>, group: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label("Settings")
        .flex_shrink_0()
        .size(px(20.))
        .mr(px(-4.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .invisible()
        .group_hover(group, |s| s.visible())
        .hover(|s| s.bg(theme::bg_raised()))
        .child(glyph(Glyph::Gear, theme::text_muted()))
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
    Server,
    Plus,
    Gear,
    /// A cluster: four nodes.
    Cluster,
    /// A cluster job's resources: a chip with pins.
    Chip,
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
                Glyph::Server => {
                    polyline(&[(1.5, 1.5), (10.5, 1.5), (10.5, 5.), (1.5, 5.), (1.5, 1.5)]);
                    polyline(&[(1.5, 7.), (10.5, 7.), (10.5, 10.5), (1.5, 10.5), (1.5, 7.)]);
                    polyline(&[(3., 3.25), (4., 3.25)]);
                    polyline(&[(3., 8.75), (4., 8.75)]);
                }
                Glyph::Plus => {
                    polyline(&[(6., 1.5), (6., 10.5)]);
                    polyline(&[(1.5, 6.), (10.5, 6.)]);
                }
                Glyph::Cluster => {
                    for (x, y) in [(1.5, 1.5), (7., 1.5), (1.5, 7.), (7., 7.)] {
                        polyline(&[(x, y), (x + 3.5, y), (x + 3.5, y + 3.5), (x, y + 3.5), (x, y)]);
                    }
                }
                Glyph::Chip => {
                    polyline(&[(3., 3.), (9., 3.), (9., 9.), (3., 9.), (3., 3.)]);
                    for p in [4.5, 7.5] {
                        polyline(&[(p, 1.), (p, 3.)]);
                        polyline(&[(p, 9.), (p, 11.)]);
                        polyline(&[(1., p), (3., p)]);
                        polyline(&[(9., p), (11., p)]);
                    }
                }
                Glyph::Gear => {
                    let circle = |r: f32| -> Vec<(f32, f32)> {
                        (0..=24).map(|i| {
                            let a = std::f32::consts::TAU * i as f32 / 24.;
                            (6. + r * a.cos(), 6. + r * a.sin())
                        }).collect()
                    };
                    polyline(&circle(3.));
                    polyline(&circle(1.2));
                    for i in 0..8 {
                        let a = std::f32::consts::TAU * i as f32 / 8.;
                        polyline(&[(6. + 3. * a.cos(), 6. + 3. * a.sin()), (6. + 5. * a.cos(), 6. + 5. * a.sin())]);
                    }
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
