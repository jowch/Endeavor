//! The sidebar: the session list, its folders and filter menu, search, and
//! the status line at the bottom.

use agent_client_protocol::schema::v1::SessionId;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::Sizable;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::agent::Agent;
use crate::hosts::{HostId, Place};
use crate::menu::MenuTarget;
use crate::new_session::{Glyph, NotebookChoice, glyph, glyph_at, menu_row};
use crate::row_marks::{self, RowFacts, RowMark};
use crate::session::{Session, folder_name};
use crate::{Interrupt, NewSession, OpenSettings, SIDEBAR_RANGE, Workspace, column_header, connection, platform, save_json, settings_panel, sidebar_filter, sidebar_toggle, theme};
use crate::theme::FocusRing as _;

/// A 28px sidebar row (sessions, "New session"). Its transparent border is
/// where a focus ring draws; with the padding, content starts 18px in from
/// the sidebar's edge and ends 18px before its other edge.
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
        .border_2()
        .border_color(gpui::transparent_black())
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
pub(crate) enum Row {
    Open(u64),
    Past(SessionId, Place),
}

/// A session being renamed and its name box: in its sidebar row, or in the
/// chat header when the session menu's Rename opened it.
pub(crate) struct Rename {
    pub(crate) row: Row,
    pub(crate) input: Entity<InputState>,
    pub(crate) in_header: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum RowAction {
    Rename,
    Reveal,
    Archive,
    Unarchive,
    Close,
    Delete,
    /// The session menu's list of the agent's rules for the folder (not in a row's ⋮ menu).
    FolderRules,
}

impl RowAction {
    const ALL: [RowAction; 6] = [RowAction::Rename, RowAction::Reveal, RowAction::Archive, RowAction::Unarchive, RowAction::Close, RowAction::Delete];

    pub(crate) fn label(self) -> &'static str {
        match self {
            RowAction::Rename => "Rename",
            RowAction::Reveal => crate::platform::REVEAL_FOLDER,
            RowAction::Archive => "Archive",
            RowAction::Unarchive => "Unarchive",
            RowAction::Close => "Close",
            RowAction::Delete => "Delete…",
            RowAction::FolderRules => "Allowed in this folder",
        }
    }

    /// The key that picks it while the menu is open (`Keystroke::key`), and how
    /// the menu shows that key.
    pub(crate) fn shortcut(self) -> (&'static str, &'static str) {
        match self {
            RowAction::Rename => ("r", "R"),
            RowAction::Reveal => ("f", "F"),
            RowAction::Archive | RowAction::Unarchive => ("a", "A"),
            RowAction::Close => ("c", "C"),
            RowAction::Delete => ("backspace", "⌫"),
            RowAction::FolderRules => ("l", "L"),
        }
    }

    /// A row's menu; Delete comes last, after a separator. `archived` is None for
    /// a session the agent hasn't given an id yet, which can't be archived.
    /// Finder can only reveal This Mac's folders.
    pub(crate) fn for_row(row: &Row, archived: Option<bool>, local: bool) -> Vec<RowAction> {
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

/// A row of the sidebar's filter menu that opens a submenu.
#[derive(Clone, Copy, PartialEq, Debug)]
enum FilterRow {
    Status,
    Where,
    GroupBy,
    SortBy,
}

impl FilterRow {
    const ALL: [FilterRow; 4] = [FilterRow::Status, FilterRow::Where, FilterRow::GroupBy, FilterRow::SortBy];

    fn label(self) -> &'static str {
        match self {
            FilterRow::Status => "Status",
            FilterRow::Where => "Where",
            FilterRow::GroupBy => "Group by",
            FilterRow::SortBy => "Sort by",
        }
    }
}

/// The sidebar's filter menu (the sliders button), and its open submenu if any.
pub(crate) struct FilterMenu {
    /// The row whose submenu is open, if any; the main menu otherwise.
    submenu: Option<FilterRow>,
    /// The item selected by the arrow keys, in whichever level is open.
    selected: Option<usize>,
    focus: FocusHandle,
    /// Focus to give back when the menu closes.
    restore: Option<FocusHandle>,
}

/// Something the filter menu's main rows or a submenu's rows pick, shared by
/// click and keyboard handling (arrow keys, Enter/Space), like `MenuPick`.
#[derive(Clone, PartialEq)]
enum FilterPick {
    /// A main-menu row that opens a submenu.
    Open(FilterRow),
    Status(sidebar_filter::StatusFilter),
    WhereAll,
    WhereHost(HostId),
    GroupBy(sidebar_filter::GroupBy),
    SortBy(sidebar_filter::SortBy),
    ToggleEmptyFolders,
    Clear,
}

/// The column every row's leading bullet, "New session"'s + and the status
/// line's mark sit in, so the words after them all start at one x.
fn bullet_slot() -> Div {
    div().flex_shrink_0().w(px(12.)).h(px(12.)).flex().items_center().justify_center()
}

/// A 24px icon button in the sidebar's right column: drawn 6px into its
/// row's 18px end inset, so its centre is 24px from the sidebar's right
/// edge, like the ⋮, the folders' +, the filter button and the gear.
fn end_button(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).role(Role::Button).flex_shrink_0().size(px(24.)).mr(px(-6.)).flex().items_center().justify_center().rounded(px(4.))
}

/// The ⋮ button at a row's end: shown while the pointer is over the row
/// (`group`), and always on the active row or while its menu is open.
fn more_button(id: impl Into<ElementId>, group: SharedString, shown: bool) -> Stateful<Div> {
    end_button(id)
        .aria_label("Session actions")
        .relative()
        .flex()
        .font_family(theme::MONO)
        .text_size(theme::size_subhead())
        .text_color(if shown { theme::text_muted().into() } else { gpui::transparent_black() })
        .group_hover(group, |s| s.text_color(theme::text_muted()))
        .hover(|s| s.text_color(theme::text_primary()))
        .child("⋮")
}

/// The mark before the status line's words.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StatusMark {
    Offline,
    /// An orange dot.
    SignedOut,
    /// A spinner: it fixes itself.
    Waiting,
    /// A red dot: it needs the user.
    NeedsYou,
}

impl StatusMark {
    pub fn label(self) -> &'static str {
        match self {
            StatusMark::Offline => "offline",
            StatusMark::SignedOut => "signed_out",
            StatusMark::Waiting => "spinner",
            StatusMark::NeedsYou => "red_dot",
        }
    }

    fn render(self, cx: &App) -> AnyElement {
        let dot = |color: Rgba| div().size(px(6.)).flex_shrink_0().rounded_full().bg(color).into_any_element();
        match self {
            StatusMark::Offline => glyph(Glyph::WifiOff, theme::text_muted()).into_any_element(),
            StatusMark::SignedOut => dot(theme::accent()),
            StatusMark::Waiting => crate::orbit::orbit_with("status-waiting".into(), 12., theme::orbit_sphere(), cx),
            StatusMark::NeedsYou => dot(theme::danger()),
        }
    }
}

/// The marks drawn as icons, 9px: as small as they stay legible, near the dots' 6px.
const MARK_GLYPH: f32 = 0.75;

/// A row's leading bullet: its mark in `bullet_slot`, with the mark's words
/// as the tooltip, or a faint ring when it has none. The waiting count is in
/// the words, so the slot keeps its width.
fn bullet(id: impl Into<ElementId>, mark: Option<RowMark>) -> Stateful<Div> {
    let id = id.into();
    let slot = bullet_slot().id(id.clone());
    let dot = |d: f32| div().size(px(d)).rounded_full();
    let Some(mark) = mark else { return slot.child(dot(6.).border_1().border_color(theme::text_faint())) };
    let words: SharedString = mark.words().into();
    let icon = match &mark {
        RowMark::NeedsYou => dot(6.).bg(theme::accent()).into_any_element(),
        RowMark::Error => glyph_at(Glyph::Warning, theme::danger(), MARK_GLYPH).into_any_element(),
        RowMark::Working { .. } => {
            let fill = dot(6.).bg(theme::text_muted());
            let breath = theme::motion_pulse();
            if breath.is_zero() {
                fill.into_any_element()
            } else {
                fill.with_animation(id, Animation::new(breath).repeat(), |d, t| d.opacity(0.65 - 0.35 * (std::f32::consts::TAU * t).cos())).into_any_element()
            }
        }
        RowMark::NewReply => dot(6.).border_1().border_color(theme::accent()).into_any_element(),
        RowMark::ServerDown { .. } => glyph_at(Glyph::WifiOff, theme::text_muted(), MARK_GLYPH).into_any_element(),
        RowMark::Waiting { .. } => glyph_at(Glyph::Clock, theme::text_muted(), MARK_GLYPH).into_any_element(),
    };
    slot.child(icon).tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(words.clone()).build(window, cx))
}

/// `text` with the first case-insensitive match of `query` picked out in
/// `theme::accent_text()`, for the sidebar search's highlighting. With no
/// match (or an empty query), the plain, truncating text.
fn highlighted_span(text: &str, query: &str) -> AnyElement {
    let q = query.trim();
    let start = (!q.is_empty()).then(|| text.to_lowercase().find(&q.to_lowercase())).flatten();
    let Some(start) = start else {
        return div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(text.to_string()).into_any_element();
    };
    let end = start + q.len();
    div()
        .flex()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text[..start].to_string())
        .child(div().flex_shrink_0().text_color(theme::accent_text()).child(text[start..end].to_string()))
        .child(div().min_w_0().overflow_hidden().text_ellipsis().child(text[end..].to_string()))
        .into_any_element()
}

impl Workspace {
    pub(crate) fn row_session_id(&self, row: &Row) -> Option<SessionId> {
        match row {
            Row::Open(key) => self.sessions.iter().find(|s| s.key == *key).and_then(|s| s.id.clone()),
            Row::Past(id, _) => Some(id.clone()),
        }
    }

    pub(crate) fn row_actions(&self, row: &Row) -> Vec<RowAction> {
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

    /// Open the sidebar's filter menu (the sliders button).
    fn open_filter_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restore = window.focused(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.filter_menu = Some(FilterMenu { submenu: None, selected: None, focus, restore });
        cx.notify();
    }

    pub(crate) fn close_filter_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(restore) = self.filter_menu.take().and_then(|m| m.restore) {
            window.focus(&restore, cx);
        }
        cx.notify();
    }

    fn set_status_filter(&mut self, status: sidebar_filter::StatusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| s.sidebar_filters.status = status);
        self.close_filter_menu(window, cx);
    }

    /// Where checkboxes: several can be ticked, so the menu stays open.
    fn toggle_where_host(&mut self, host: HostId, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| {
            if !s.sidebar_filters.where_hosts.remove(&host) {
                s.sidebar_filters.where_hosts.insert(host);
            }
        });
    }

    fn set_where_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| s.sidebar_filters.where_hosts.clear());
        self.close_filter_menu(window, cx);
    }

    fn set_group_by(&mut self, group_by: sidebar_filter::GroupBy, window: &mut Window, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| s.sidebar_filters.group_by = group_by);
        self.close_filter_menu(window, cx);
    }

    fn set_sort_by(&mut self, sort_by: sidebar_filter::SortBy, window: &mut Window, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| s.sidebar_filters.sort_by = sort_by);
        self.close_filter_menu(window, cx);
    }

    fn toggle_show_empty_folders(&mut self, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| s.sidebar_filters.show_empty_folders = !s.sidebar_filters.show_empty_folders);
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| s.sidebar_filters = sidebar_filter::SidebarFilters::default());
        self.close_filter_menu(window, cx);
    }

    /// A folder's sidebar heading collapses or expands.
    fn toggle_folder_collapsed(&mut self, folder: Place, cx: &mut Context<Self>) {
        self.update_settings(cx, |s| {
            if !s.collapsed_folders.remove(&folder) {
                s.collapsed_folders.insert(folder);
            }
        });
    }

    /// Open the sidebar's inline search field, in place of the "Sessions" heading.
    pub(crate) fn open_sidebar_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_search.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions"));
        cx.subscribe_in(&input, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        input.update(cx, |s, cx| s.focus(window, cx));
        self.sidebar_search = Some(input);
        cx.notify();
    }

    pub(crate) fn close_sidebar_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_search.take().is_some() {
            window.focus(&self.keyboard_home, cx);
            cx.notify();
        }
    }

    fn toggle_sidebar_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_search.is_some() {
            self.close_sidebar_search(window, cx);
        } else {
            self.open_sidebar_search(window, cx);
        }
    }

    /// A row's notebook file name, for the search's second field (matches on
    /// the notebook a session started with or later opened, whether it's
    /// open now or a past session).
    fn row_notebook_name(&self, row: &Row) -> Option<String> {
        let id = self.row_session_id(row)?;
        self.session_notebooks.get(&id.to_string()).and_then(|p| p.path.file_name()).map(|n| n.to_string_lossy().into_owned())
    }

    /// Whether the sidebar has any session at all, open or past, before any
    /// filter or search: it hides the "Sessions" heading when there's none.
    fn any_sessions_at_all(&self) -> bool {
        !self.sessions.is_empty() || self.records.any_placed()
    }

    /// The +'s "New session in <folder>": opens the new-session screen with
    /// that folder, and its host, already chosen.
    fn start_session_in(&mut self, folder: Place, window: &mut Window, cx: &mut Context<Self>) {
        self.active = None;
        if folder.host != self.draft.host {
            self.set_draft_host(folder.host.clone(), window, cx);
        }
        if self.draft.folder.as_ref() != Some(&folder.path) {
            self.draft.folder = Some(folder.path);
            self.draft.notebooks.clear();
            self.choose_notebook(NotebookChoice::New, cx);
            self.scan_notebooks(cx);
        }
        cx.notify();
    }

    /// A row's name as the sidebar shows it.
    pub(crate) fn row_title(&self, row: &Row) -> Option<String> {
        match row {
            Row::Open(key) => self.sessions.iter().find(|s| s.key == *key).map(|s| s.title.clone()),
            Row::Past(id, _) => Some(self.past_title(id).0),
        }
    }

    pub(crate) fn row_action(&mut self, row: Row, action: RowAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            RowAction::Rename => self.start_rename(row, false, window, cx),
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
            RowAction::FolderRules => {
                if let Row::Open(key) = row {
                    self.open_menu(MenuTarget::FolderRules(key), None, window, cx);
                }
            }
            RowAction::Delete => {
                let Some(title) = self.row_title(&row) else { return };
                let agent = match &row {
                    Row::Open(key) => self.sessions.iter().find(|s| s.key == *key).map(|s| s.agent),
                    Row::Past(id, _) => self.records.get(&id.to_string()).map(|r| r.agent),
                }
                .unwrap_or_default();
                let history = match agent {
                    Agent::Claude => "Claude Code",
                    Agent::Codex => "Codex",
                };
                self.open_confirm(
                    format!("Delete “{title}”?"),
                    format!("This permanently deletes the conversation, including its {history} history. Notebooks and other files it made stay on disk."),
                    "Delete",
                    window,
                    cx,
                    move |this, _, cx| this.delete_session(row.clone(), cx),
                );
            }
        }
    }

    /// Open a session's name box: in its sidebar row, or `in_header`, in the
    /// chat header's title (the session menu's Rename).
    pub(crate) fn start_rename(&mut self, row: Row, in_header: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        self.renaming = Some(Rename { row, input, in_header });
        cx.notify();
    }

    /// Keep the typed name (an empty one leaves the title as it was).
    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(Rename { row, input, .. }) = self.renaming.take() else { return };
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

    /// The sidebar's status line: app-wide states only, with a mark when it
    /// matters to every session (red only when something needs the user).
    pub(crate) fn status_line(&self) -> (Option<StatusMark>, SharedString) {
        if self.offline_since.is_some() {
            return (Some(StatusMark::Offline), "Offline · reconnects by itself".into());
        }
        if self.account.signed_out() {
            return (Some(StatusMark::SignedOut), "Signed out of Claude.".into());
        }
        for agent in crate::agent::Agent::ALL {
            match self.links.get(agent).process.state {
                crate::agent_process::State::Restarting => return (Some(StatusMark::Waiting), format!("Restarting {}…", agent.name()).into()),
                crate::agent_process::State::Down => return (Some(StatusMark::NeedsYou), crate::agent_process::down_title(agent).into()),
                crate::agent_process::State::Up => {}
            }
        }
        if let Some(host) = self.crashed_host() {
            return (Some(StatusMark::NeedsYou), format!("Julia on {host} stopped").into());
        }
        (None, self.status.clone())
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
            // The dark-contrast raise (theme.rs): sidebar session rows read
            // lighter than folder headings (`text_section`), which keep `text_muted`.
            .when(!active, |d| d.text_color(theme::text_row()))
            .group(group)
            .when(menu_open, |d| d.bg(theme::row_active()))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| this.open_menu(MenuTarget::Row(row.clone()), Some(e.position), window, cx)),
            )
    }

    /// A row's title, or its name box while it's being renamed. Its second
    /// line is `folder_line` when given (Group by: None, so each flattened
    /// row still names its folder); otherwise, while searching, a match on
    /// the row's notebook name, highlighted there. The title itself is
    /// highlighted while searching either way.
    fn row_lines(&self, row: &Row, title: &str, query: &str, folder_line: Option<&str>) -> Div {
        if let Some(Rename { row: renaming, input, in_header: false }) = &self.renaming
            && renaming == row
        {
            return div().flex_1().child(Input::new(input).xsmall().text_size(theme::size_body()));
        }
        let searching = !query.trim().is_empty();
        let second: Option<AnyElement> = match folder_line {
            Some(f) => Some(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(f.to_string()).into_any_element()),
            None => searching
                .then(|| self.row_notebook_name(row).filter(|n| sidebar_filter::row_matches_search(query, "", Some(n))))
                .flatten()
                .map(|n| highlighted_span(&n, query)),
        };
        if !searching && second.is_none() {
            return div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title.to_string());
        }
        let title_el = if searching { highlighted_span(title, query) } else { div().overflow_hidden().whitespace_nowrap().child(title.to_string()).into_any_element() };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(title_el)
            .children(second.map(|el| div().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(el)))
    }

    /// The chat header's session title and ⌄, which opens the session menu (a
    /// sidebar row's ⋮ menu, for the open session); while renamed from that
    /// menu, its name box instead.
    pub(crate) fn session_title(&self, key: u64, title: String, cx: &mut Context<Self>) -> AnyElement {
        if let Some(Rename { row: Row::Open(renaming), input, in_header: true }) = &self.renaming
            && *renaming == key
        {
            return div()
                .w(px(260.))
                .flex_shrink(1.)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Input::new(input).xsmall().text_size(theme::size_body()))
                .into_any_element();
        }
        let target = MenuTarget::Session(key);
        let menu = self.menu.as_ref().filter(|menu| menu.target == target || menu.target == MenuTarget::FolderRules(key));
        div()
            .id("session-title")
            .role(Role::Button)
            .aria_label(format!("{title}, session menu"))
            .relative()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.))
            .h(px(26.))
            .px(px(6.))
            .ml(px(-6.))
            .rounded(px(4.))
            .cursor_pointer()
            .border_2()
            .border_color(gpui::transparent_black())
            .track_focus(&self.dialog_focus("session-title", cx))
            .tab_stop(true)
            .focus_ring_on(theme::bg_page())
            .when(menu.is_some(), |d| d.bg(theme::row_active()))
            .hover(|s| s.bg(theme::row_active()))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title))
            .child(div().flex_shrink_0().child(glyph(Glyph::Chevron, theme::text_muted())))
            .children(menu.map(|menu| self.render_menu(menu, cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.open_menu(target.clone(), None, window, cx)))
            .into_any_element()
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

    /// A folder's name, with its server's for a folder on one ("decay-fits · lab").
    pub(crate) fn folder_heading(&self, place: &Place) -> String {
        match place.host {
            HostId::ThisMac => folder_name(&place.path),
            _ => format!("{} · {}", folder_name(&place.path), self.hosts.name(&place.host)),
        }
    }

    /// The sidebar's folders: recent ones, then any other folder with an open
    /// session, filtered by Where and ordered for Group by ("Folder" and
    /// "None" keep this order; "Where it runs" clusters folders on one host).
    pub(crate) fn sidebar_folders(&self) -> Vec<Place> {
        let mut folders: Vec<Place> = self.recent.clone();
        for s in &self.sessions {
            if !folders.contains(&s.place) {
                folders.push(s.place.clone());
            }
        }
        let hosts = &self.settings.sidebar_filters.where_hosts;
        folders.retain(|f| sidebar_filter::where_matches(hosts, &f.host));
        sidebar_filter::order_folders(folders, self.settings.sidebar_filters.group_by)
    }

    /// A folder's past sessions the sidebar lists, newest first: recorded, not
    /// already open, and matching the Status filter.
    fn past_rows(&self, folder: &Place) -> Vec<SessionId> {
        let is_open = |id: &str| self.sessions.iter().any(|s| s.id.as_ref().is_some_and(|s| s.to_string() == id));
        let status = self.settings.sidebar_filters.status;
        self.records
            .in_folder(folder)
            .into_iter()
            .filter(|(id, _)| !is_open(id) && sidebar_filter::status_matches(status, self.archived.contains(*id)))
            .map(|(id, _)| SessionId::new(id.to_owned()))
            .collect()
    }

    /// The sidebar's row above the status line while This Mac's Julia is down.
    pub(crate) fn restart_row(&self) -> Option<&'static str> {
        match self.status(&HostId::ThisMac) {
            Some(connection::Status::Replaced) => Some("↻ Reconnect to Julia"),
            Some(connection::Status::Died(_) | connection::Status::Failed(_)) => Some("↻ Restart Julia"),
            _ => None,
        }
    }

    /// An open session's row mark (see `row_marks`). Working is what the
    /// composer's orbit shows, not its grey "Waiting for the connection".
    pub(crate) fn row_mark(&self, s: &Session) -> Option<RowMark> {
        let host_down = s.place.host != HostId::ThisMac && self.connections.get(&s.place.host).is_some_and(|c| c.lost.is_some());
        let held = s.outbox.held && !s.outbox.items.is_empty() && !self.links.get(s.agent).process.up();
        row_marks::row_mark(&RowFacts {
            needs_you: s.needs_approval(),
            error: s.errored || s.failed.is_some(),
            working: crate::transcript::activity(s, self.offline_since).is_some_and(|a| !a.waiting),
            agent: s.agent.name(),
            new_reply: s.unseen && self.active != Some(s.key),
            server_down: host_down.then(|| self.hosts.name(&s.place.host).to_string()),
            waiting: held.then(|| (s.outbox.items.len(), format!("{} is back", s.agent.name()))),
            mac_offline: self.offline_since.is_some(),
        })
    }

    /// A collapsed folder's mark: the strongest of its open sessions'.
    pub(crate) fn folder_mark(&self, folder: &Place) -> Option<RowMark> {
        row_marks::strongest(self.sessions.iter().filter(|s| &s.place == folder).filter_map(|s| self.row_mark(s)))
    }

    /// A folder's rows to show (Status- and search-filtered, sorted by Sort
    /// by, past sessions cut to `PAST_SHOWN` unless expanded, searching or
    /// `flat`), its "Show N more"/"Show fewer" label (never, when `flat`:
    /// Group by None has no folder heading for it to expand), and its total
    /// row count before the `PAST_SHOWN` cut (0 means genuinely empty, for
    /// Show empty folders). `flat` is Group by None's flattened list, which
    /// has no collapsed folders either.
    pub(crate) fn folder_rows(&self, folder: &Place, query: &str, collapsed: bool, flat: bool) -> (Vec<Row>, Option<(String, bool)>, usize) {
        let searching = !query.trim().is_empty();
        let mut open: Vec<(Row, String)> =
            self.sessions.iter().filter(|s| &s.place == folder).map(|s| (Row::Open(s.key), s.title.clone())).collect();
        let mut past: Vec<(Row, String)> =
            self.past_rows(folder).into_iter().map(|id| (Row::Past(id.clone(), folder.clone()), self.past_title(&id).0)).collect();
        if searching {
            let matches = |row: &Row, title: &str| sidebar_filter::row_matches_search(query, title, self.row_notebook_name(row).as_deref());
            open.retain(|(row, title)| matches(row, title));
            past.retain(|(row, title)| matches(row, title));
        }
        let sort = self.settings.sidebar_filters.sort_by;
        sidebar_filter::sort_titles(&mut open, sort, |(_, t)| t.as_str());
        sidebar_filter::sort_titles(&mut past, sort, |(_, t)| t.as_str());
        let total = open.len() + past.len();
        if collapsed && !searching {
            return (Vec::new(), None, total);
        }
        let expanded = searching || flat || self.expanded.contains(folder);
        let more = (!searching && !flat && past.len() > PAST_SHOWN).then(|| {
            if expanded { ("Show fewer".to_string(), true) } else { (format!("Show {} more", past.len() - PAST_SHOWN), false) }
        });
        if !expanded {
            past.truncate(PAST_SHOWN);
        }
        let rows = open.into_iter().chain(past).map(|(row, _)| row).collect();
        (rows, more, total)
    }

    /// One sidebar row (open or past), with its title/notebook lines, end
    /// mark, focus handle and click behavior.
    fn render_sidebar_row(&self, row: Row, query: &str, flat: bool, cx: &mut Context<Self>) -> AnyElement {
        match row.clone() {
            Row::Open(key) => {
                let Some(s) = self.sessions.iter().find(|s| s.key == key) else { return div().into_any_element() };
                let active = self.active == Some(key);
                let group: SharedString = format!("session-{key}").into();
                let title_text = s.title.clone();
                let folder_line = flat.then(|| self.folder_heading(&s.place));
                let title = self.row_lines(&row, &title_text, query, folder_line.as_deref());
                let row_mark = self.row_mark(s);
                let label = row_mark.as_ref().map_or_else(|| title_text.clone(), |m| m.label(&title_text));
                self.session_row(row.clone(), group.clone(), active, cx)
                    .aria_label(label)
                    .track_focus(&s.focus_handle(cx))
                    .tab_stop(true)
                    .focus_visible(|st| st.border_color(theme::focus_ring()))
                    .child(bullet(ElementId::NamedInteger("row-bullet".into(), key), row_mark))
                    .child(title)
                    .child(self.row_more(row.clone(), group, active, cx))
                    // Double-click renames.
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if e.click_count() >= 2 {
                            this.start_rename(Row::Open(key), false, window, cx);
                        } else {
                            this.activate(key, cx);
                        }
                    }))
                    .into_any_element()
            }
            Row::Past(id, place) => {
                let title_text = self.row_title(&row).unwrap_or_default();
                let folder_line = flat.then(|| self.folder_heading(&place));
                let title = self.row_lines(&row, &title_text, query, folder_line.as_deref());
                let group: SharedString = format!("past-{:?}-{}-{id}", place.host, place.path.display()).into();
                let archived = self.archived.contains(&id.to_string());
                let focus = self.past_row_focus(&id, cx);
                let label = if archived { format!("{title_text}, archived") } else { title_text };
                self.session_row(row.clone(), group.clone(), false, cx)
                    .aria_label(label)
                    .track_focus(&focus)
                    .tab_stop(true)
                    .focus_visible(|d| d.border_color(theme::focus_ring()))
                    .when(archived, |d| d.text_color(theme::text_section()))
                    .child(if archived { bullet_slot().child(glyph_at(Glyph::Archive, theme::text_section(), MARK_GLYPH)).into_any_element() } else { bullet("row-bullet", None).into_any_element() })
                    .child(title)
                    .child(self.row_more(row.clone(), group, false, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.renaming.as_ref().is_some_and(|r| r.row == row) {
                            this.open_past(id.clone(), place.clone(), cx);
                        }
                    }))
                    .into_any_element()
            }
        }
    }

    /// A folder's heading: its name (with a collapse chevron via "name ›"
    /// when collapsed) starting at the rows' bullet column, and a "New
    /// session in <folder>" + always visible in the rows' ⋮ column. A
    /// collapsed folder shows its rows' strongest mark after the "›".
    /// Clicking the name collapses or expands; clicking + starts a session
    /// there. Each is its own Tab stop with the sidebar's focus ring. The
    /// first folder sits right under "Sessions"; the others are 16px below
    /// the folder before, so a heading is nearer its own rows.
    fn render_folder_heading(&self, folder: &Place, first: bool, collapsed: bool, mark: Option<RowMark>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let name = self.folder_heading(folder);
        let key = format!("{:?}-{}", folder.host, folder.path.display());
        let label = if collapsed { format!("{name} ›") } else { name.clone() };
        let toggle_folder = folder.clone();
        let heading = div()
            .id(ElementId::Name(format!("folder-toggle-{key}").into()))
            .role(Role::Button)
            .aria_label(name.clone())
            .aria_expanded(!collapsed)
            .flex_1()
            .min_w_0()
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(theme::size_meta_small())
            .text_color(theme::text_section())
            .border_2()
            .border_color(gpui::transparent_black())
            .track_focus(&self.dialog_focus(format!("folder-toggle-{key}"), cx))
            .tab_stop(true)
            .focus_visible(|s| s.border_color(theme::focus_ring()))
            .hover(|s| s.bg(theme::row_active()))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(label))
            .children(mark.filter(|_| collapsed).map(|m| bullet(ElementId::Name(format!("folder-mark-{key}").into()), Some(m)).ml(px(2.))))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_folder_collapsed(toggle_folder.clone(), cx)));
        let plus_folder = folder.clone();
        let plus_label: SharedString = format!("New session in {name}").into();
        let tooltip_label = plus_label.clone();
        let plus = end_button(ElementId::Name(format!("folder-plus-{key}").into()))
            .aria_label(plus_label)
            .border_2()
            .border_color(gpui::transparent_black())
            .track_focus(&self.dialog_focus(format!("folder-plus-{key}"), cx))
            .tab_stop(true)
            .focus_visible(|s| s.border_color(theme::focus_ring()))
            .cursor_pointer()
            .hover(|s| s.bg(theme::row_active()))
            .child(glyph(Glyph::Plus, theme::text_secondary()))
            .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tooltip_label.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, window, cx| this.start_session_in(plus_folder.clone(), window, cx)));
        div().when(!first, |d| d.mt(px(16.))).pr(px(12.)).flex().items_center().child(heading).child(plus)
    }

    pub(crate) fn render_session_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let update = settings_panel::update_available(&self.updates());
        let sessions_scroll = window.use_keyed_state("sidebar-sessions-scroll", cx, |_, _| ScrollHandle::new()).read(cx).clone();
        let scrolled_from_top = sessions_scroll.offset().y < px(0.);
        let query = self.sidebar_search.as_ref().map(|s| s.read(cx).value().to_string()).unwrap_or_default();
        let searching = !query.trim().is_empty();
        let show_empty = self.settings.sidebar_filters.show_empty_folders;
        let group_by_none = self.settings.sidebar_filters.group_by == sidebar_filter::GroupBy::None;
        let mut any_matched = false;
        // Group by None: one flat list, no folder headings (so no + either --
        // "New session" covers it), sorted by Sort by across every folder.
        let groups: Vec<AnyElement> = if group_by_none {
            let mut rows: Vec<(Row, String)> = self
                .sidebar_folders()
                .into_iter()
                .flat_map(|folder| self.folder_rows(&folder, &query, false, true).0)
                .map(|row| {
                    let title = self.row_title(&row).unwrap_or_default();
                    (row, title)
                })
                .collect();
            sidebar_filter::sort_titles(&mut rows, self.settings.sidebar_filters.sort_by, |(_, t)| t.as_str());
            any_matched = !rows.is_empty();
            rows.into_iter().map(|(row, _)| self.render_sidebar_row(row, &query, true, cx)).collect()
        } else {
            self.sidebar_folders()
                .into_iter()
                .filter_map(|folder| {
                    let collapsed = !searching && self.settings.collapsed_folders.contains(&folder);
                    let (rows, more, total) = self.folder_rows(&folder, &query, collapsed, false);
                    if total == 0 && (searching || !show_empty) {
                        return None;
                    }
                    let first = !any_matched;
                    any_matched = true;
                    let mark = self.folder_mark(&folder);
                    let body: Vec<AnyElement> = rows.into_iter().map(|row| self.render_sidebar_row(row, &query, false, cx)).collect();
                    let more_row = more.map(|(label, fewer)| {
                        let folder = folder.clone();
                        sidebar_row(ElementId::Name(format!("more-{:?}-{}", folder.host, folder.path.display()).into()), false)
                            .aria_label(label.clone())
                            .text_color(theme::text_faint())
                            .child(bullet_slot())
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if fewer {
                                    this.expanded.remove(&folder);
                                } else {
                                    this.expanded.insert(folder.clone());
                                }
                                cx.notify();
                            }))
                    });
                    Some(
                        div()
                            .flex()
                            .flex_col()
                            .child(self.render_folder_heading(&folder, first, collapsed, mark, cx))
                            .children(body)
                            .children(more_row)
                            .into_any_element(),
                    )
                })
                .collect()
        };
        let no_matches = searching && !any_matched;

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
            // A tab group of its own, so Tab reaches every sidebar control
            // before the main column's, whatever order they paint in.
            .tab_group()
            .tab_index(0)
            .track_focus(&self.sidebar_focus)
            // The traffic lights sit in this header (see TitlebarOptions in main()).
            .child(column_header("sidebar-header").mx(px(-6.)).justify_end().px(px(10.)).child(sidebar_toggle(self, cx)))
            .child(
                sidebar_row("new-session".into(), false)
                    .aria_label("New session")
                    .text_color(theme::text_new())
                    .track_focus(&self.dialog_focus("new-session-row", cx))
                    .tab_stop(true)
                    .focus_visible(|s| s.border_color(theme::focus_ring()))
                    .child(bullet_slot().child(glyph(Glyph::Plus, theme::text_faint())))
                    .child(div().flex_1().child("New session"))
                    // Centred on the right column while it fits its 24px; Ctrl+N grows leftwards.
                    .child(
                        div()
                            .flex_shrink_0()
                            .min_w(px(24.))
                            .mr(px(-6.))
                            .flex()
                            .justify_center()
                            .text_size(theme::size_meta())
                            .text_color(theme::text_faint())
                            .child(crate::platform::shortcut!("N")),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.new_session(&NewSession, window, cx))),
            )
            .when(self.any_sessions_at_all(), |d| {
                d.child(div().mt(px(16.)).child(match &self.sidebar_search {
                    Some(input) => self.render_sidebar_search(input, cx).into_any_element(),
                    None => self.render_sessions_heading(cx).into_any_element(),
                }))
            })
            .child(
                // The fade is a sibling of the scrolling list, not a child of
                // it: a child would scroll away with the rest of the content
                // instead of staying put over the top of the list.
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("sessions")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&sessions_scroll)
                            .flex()
                            .flex_col()
                            .map(|d| {
                                if no_matches {
                                    d.child(
                                        div()
                                            .px(px(10.))
                                            .pt(px(8.))
                                            .text_size(theme::size_meta())
                                            .text_color(theme::text_faint())
                                            .child(format!("No sessions match “{}”.", query.trim())),
                                    )
                                } else {
                                    d.children(groups)
                                }
                            }),
                    )
                    .when(scrolled_from_top, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(px(20.))
                                .bg(linear_gradient(180., linear_color_stop(theme::bg_sidebar(), 0.), linear_color_stop(theme::bg_sidebar().opacity(0.), 1.))),
                        )
                    }),
            )
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
                        let mark = mark.map(|m| bullet_slot().child(m.render(cx)));
                        div()
                            .id("status")
                            .flex_1()
                            .min_w_0()
                            .pl(px(8.))
                            .flex()
                            .items_center()
                            .gap_2()
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
                            .relative()
                            .border_2()
                            .border_color(gpui::transparent_black())
                            .track_focus(&self.dialog_focus("settings-gear", cx))
                            .tab_stop(true)
                            .focus_ring_on(theme::bg_sidebar())
                            .text_color(if self.settings_panel.is_some() { theme::text_primary() } else { theme::text_faint() })
                            .when(self.settings_panel.is_some(), |d| d.bg(theme::row_active()))
                            .hover(|s| s.text_color(theme::text_primary()))
                            .child("⚙")
                            .when(update, |d| {
                                d.aria_label("Settings, an update is available")
                                    .child(div().absolute().top(px(5.)).right(px(5.)).size(px(6.)).rounded_full().bg(theme::accent()))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if update {
                                    this.open_settings_at(settings_panel::Page::Section(settings_panel::Section::About), window, cx);
                                } else {
                                    this.open_settings(&OpenSettings, window, cx);
                                }
                            })),
                    ),
            )
    }

    /// The "Sessions" heading row: a search button and the filter button on the right.
    fn render_sessions_heading(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .h(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .px(px(12.))
            .text_size(theme::size_meta_small())
            .text_color(theme::text_section())
            .child(div().flex_1().child("Sessions"))
            .child(
                end_button("sidebar-search-button")
                    .aria_label("Search sessions")
                    .mr_0()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::bg_raised()))
                    .child(glyph(Glyph::Search, theme::text_secondary()))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar_search(window, cx))),
            )
            .child(self.render_filter_menu_button(cx))
    }

    /// The inline search field that replaces the "Sessions" heading while open.
    fn render_sidebar_search(&self, input: &Entity<InputState>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let empty = input.read(cx).value().trim().is_empty();
        let input = input.clone();
        div()
            .h(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(12.))
            .child(glyph(Glyph::Search, theme::text_secondary()))
            .child(div().flex_1().min_w_0().child(Input::new(&input).appearance(false).text_size(theme::size_body())))
            .child(div().text_size(theme::size_meta_small()).text_color(theme::text_faint()).child("esc"))
            .child(
                end_button("sidebar-search-close")
                    .aria_label(if empty { "Close search" } else { "Clear search" })
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::row_active()))
                    .child(glyph(Glyph::Close, theme::text_faint()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if empty {
                            this.close_sidebar_search(window, cx);
                        } else {
                            input.update(cx, |s, cx| s.set_value("", window, cx));
                            cx.notify();
                        }
                    })),
            )
    }

    /// Everything a filter-menu row or submenu item picks, for the row it's
    /// at (`None` for the main menu): the main menu's rows in order, then
    /// (only when open) that row's submenu's own rows, mirroring `menu_picks`.
    fn filter_picks(&self, submenu: Option<FilterRow>) -> Vec<FilterPick> {
        match submenu {
            None => {
                let mut picks: Vec<FilterPick> = FilterRow::ALL.into_iter().map(FilterPick::Open).collect();
                picks.push(FilterPick::ToggleEmptyFolders);
                if !self.settings.sidebar_filters.is_default() {
                    picks.push(FilterPick::Clear);
                }
                picks
            }
            Some(FilterRow::Status) => sidebar_filter::StatusFilter::ALL.into_iter().map(FilterPick::Status).collect(),
            Some(FilterRow::Where) => {
                let mut picks = vec![FilterPick::WhereAll, FilterPick::WhereHost(HostId::ThisMac)];
                picks.extend(self.hosts.servers.iter().map(|s| FilterPick::WhereHost(HostId::Server(s.id.clone()))));
                picks
            }
            Some(FilterRow::GroupBy) => sidebar_filter::GroupBy::ALL.into_iter().map(FilterPick::GroupBy).collect(),
            Some(FilterRow::SortBy) => sidebar_filter::SortBy::ALL.into_iter().map(FilterPick::SortBy).collect(),
        }
    }

    fn apply_filter_pick(&mut self, pick: FilterPick, window: &mut Window, cx: &mut Context<Self>) {
        match pick {
            FilterPick::Open(row) => {
                if let Some(menu) = self.filter_menu.as_mut() {
                    menu.submenu = Some(row);
                    menu.selected = None;
                }
                cx.notify();
            }
            FilterPick::Status(status) => self.set_status_filter(status, window, cx),
            FilterPick::WhereAll => self.set_where_all(window, cx),
            // Several hosts can be ticked, so this doesn't close the menu.
            FilterPick::WhereHost(host) => self.toggle_where_host(host, cx),
            FilterPick::GroupBy(g) => self.set_group_by(g, window, cx),
            FilterPick::SortBy(s) => self.set_sort_by(s, window, cx),
            FilterPick::ToggleEmptyFolders => self.toggle_show_empty_folders(cx),
            FilterPick::Clear => self.clear_filters(window, cx),
        }
    }

    /// Up/Down move the selection in whichever level is open; Enter/Space
    /// activates it; Left backs out of a submenu; Right opens one. Esc is
    /// handled by the menu body's own `on_action`, like the row ⋮ menu's.
    fn filter_menu_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(submenu) = self.filter_menu.as_ref().map(|m| m.submenu) else { return };
        let picks = self.filter_picks(submenu);
        let n = picks.len();
        if n == 0 {
            return;
        }
        let key = e.keystroke.key.as_str();
        let activated = match key {
            "down" => {
                if let Some(menu) = self.filter_menu.as_mut() {
                    menu.selected = Some(menu.selected.map_or(0, |i| (i + 1) % n));
                }
                None
            }
            "up" => {
                if let Some(menu) = self.filter_menu.as_mut() {
                    menu.selected = Some(menu.selected.map_or(n - 1, |i| (i + n - 1) % n));
                }
                None
            }
            "left" if submenu.is_some() => {
                if let Some(menu) = self.filter_menu.as_mut() {
                    menu.submenu = None;
                    menu.selected = None;
                }
                None
            }
            "right" | "enter" | "space" => self.filter_menu.as_ref().and_then(|m| m.selected).and_then(|i| picks.get(i).cloned()),
            _ => return,
        };
        if let Some(pick) = activated {
            if key == "right" && !matches!(pick, FilterPick::Open(_)) {
                return;
            }
            return self.apply_filter_pick(pick, window, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The sliders button, and the filter menu (Status, Where, Group by, Sort
    /// by, Show empty folders, Clear filters) while open.
    fn render_filter_menu_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let changed = !self.settings.sidebar_filters.is_default();
        let color = if changed { theme::accent_text() } else { theme::text_secondary() };
        let menu = self.filter_menu.as_ref().map(|menu| self.render_filter_menu(menu, cx));
        end_button("sidebar-filter")
            .aria_label("Filter sessions")
            .relative()
            .when(self.filter_menu.is_some(), |d| d.bg(theme::bg_raised()))
            .hover(|s| s.bg(theme::bg_raised()))
            .child(glyph(Glyph::Sliders, color))
            .children(menu)
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                if this.filter_menu.is_some() {
                    this.close_filter_menu(window, cx);
                } else {
                    this.open_filter_menu(window, cx);
                }
            }))
    }

    /// The filter menu's main panel: each row shows its current value and ›
    /// and opens a submenu, then Show empty folders (a toggle) and, only when
    /// something differs from the defaults, Clear filters.
    fn render_filter_menu(&self, menu: &FilterMenu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let filters = self.settings.sidebar_filters.clone();
        let where_label = sidebar_filter::where_label(&filters.where_hosts, |h| self.hosts.name(h));
        let separator = || div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::popover_edge());
        let row = |i: usize, label: &'static str, value: String, target: FilterRow, cx: &mut Context<Self>| {
            let selected = menu.submenu.is_none() && menu.selected == Some(i);
            div()
                .id(ElementId::NamedInteger("filter-row".into(), i as u64))
                .role(Role::MenuItem)
                .aria_label(format!("{label}: {value}"))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(28.))
                .px(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(selected, |d| d.bg(theme::menu_hover()))
                .hover(|s| s.bg(theme::menu_hover()))
                .child(div().flex_1().child(label))
                .child(div().text_color(theme::text_faint()).child(value))
                .child(div().text_color(theme::text_faint()).child("›"))
                .on_click(cx.listener(move |this, _, window, cx| this.apply_filter_pick(FilterPick::Open(target), window, cx)))
        };
        let empty_toggle = div()
            .id("filter-empty-folders")
            .role(Role::Switch)
            .aria_label("Show empty folders")
            .aria_toggled(if filters.show_empty_folders { accesskit::Toggled::True } else { accesskit::Toggled::False })
            .relative()
            .w(px(30.))
            .h(px(18.))
            .flex_shrink_0()
            .rounded(px(9.))
            .cursor_pointer()
            .bg(if filters.show_empty_folders { theme::accent() } else { theme::composer_edge() })
            .child(div().absolute().top(px(2.)).left(px(if filters.show_empty_folders { 14. } else { 2. })).size(px(14.)).rounded_full().bg(gpui::white()))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_show_empty_folders(cx);
            }));
        let body = div()
            .id("filter-menu")
            .role(Role::Menu)
            .track_focus(&menu.focus)
            .occlude()
            .relative()
            .w(px(220.))
            .p(px(4.))
            .flex()
            .flex_col()
            .map(theme::popover)
            .font_family(theme::SANS)
            .text_size(theme::size_body())
            .text_color(theme::text_primary())
            .on_action(cx.listener(|this, _: &Interrupt, window, cx| {
                if let Some(menu) = this.filter_menu.as_mut()
                    && menu.submenu.take().is_some()
                {
                    menu.selected = None;
                    return cx.notify();
                }
                this.close_filter_menu(window, cx);
            }))
            .on_key_down(cx.listener(Self::filter_menu_key))
            .child(row(0, "Status", filters.status.label().to_string(), FilterRow::Status, cx))
            .child(row(1, "Where", where_label, FilterRow::Where, cx))
            .child(separator())
            .child(row(2, "Group by", filters.group_by.label().to_string(), FilterRow::GroupBy, cx))
            .child(row(3, "Sort by", filters.sort_by.label().to_string(), FilterRow::SortBy, cx))
            .child(separator())
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(28.))
                    .px(px(8.))
                    .child(div().flex_1().child("Show empty folders"))
                    .child(empty_toggle),
            )
            .when(!filters.is_default(), |d| {
                d.child(separator()).child(
                    div()
                        .id("filter-clear")
                        .role(Role::MenuItem)
                        .aria_label("Clear filters")
                        .h(px(28.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::menu_hover()))
                        .child("Clear filters")
                        .on_click(cx.listener(|this, _, window, cx| this.clear_filters(window, cx))),
                )
            })
            .children(menu.submenu.map(|submenu| self.render_filter_submenu(submenu, menu, cx)));
        div().absolute().top(px(26.)).right_0().child(deferred(anchored().anchor(Anchor::TopRight).child(crate::motion::arriving(body, "filter-menu-in", true))).with_priority(1))
    }

    /// A filter row's flyout submenu: Status, Group by and Sort by are
    /// single-choice (✓ radio); Where is several-choice (✓ checkbox, stays open).
    fn render_filter_submenu(&self, submenu: FilterRow, menu: &FilterMenu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let picks = self.filter_picks(Some(submenu));
        let filters = &self.settings.sidebar_filters;
        let checked = |pick: &FilterPick| match pick {
            FilterPick::Status(s) => *s == filters.status,
            FilterPick::WhereAll => filters.where_hosts.is_empty(),
            FilterPick::WhereHost(h) => filters.where_hosts.contains(h),
            FilterPick::GroupBy(g) => *g == filters.group_by,
            FilterPick::SortBy(s) => *s == filters.sort_by,
            _ => false,
        };
        let label = |pick: &FilterPick| -> String {
            match pick {
                FilterPick::Status(s) => s.label().to_string(),
                FilterPick::WhereAll => "All places".to_string(),
                FilterPick::WhereHost(h) => self.hosts.name(h),
                FilterPick::GroupBy(g) => g.label().to_string(),
                FilterPick::SortBy(s) => s.label().to_string(),
                _ => String::new(),
            }
        };
        let items = picks.into_iter().enumerate().map(|(i, pick)| {
            let selected = menu.selected == Some(i);
            let is_checked = checked(&pick);
            let text = label(&pick);
            menu_row(ElementId::NamedInteger("filter-submenu-item".into(), i as u64), is_checked, selected)
                .role(if submenu == FilterRow::Where { Role::MenuItemCheckBox } else { Role::MenuItemRadio })
                .aria_label(text.clone())
                .child(text)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.apply_filter_pick(pick.clone(), window, cx);
                }))
        });
        let top = match submenu {
            FilterRow::Status => 4.,
            FilterRow::Where => 32.,
            FilterRow::GroupBy => 69.,
            FilterRow::SortBy => 97.,
        };
        div()
            .id("filter-submenu")
            .role(Role::Menu)
            .aria_label(submenu.label())
            .occlude()
            .absolute()
            .top(px(top))
            .left(px(224.))
            .w(px(180.))
            .p(px(4.))
            .flex()
            .flex_col()
            .map(theme::popover)
            .font_family(theme::SANS)
            .text_size(theme::size_body())
            .text_color(theme::text_primary())
            .children(items)
    }
}

#[cfg(test)]
mod tests {
    use super::{Row, RowAction};

    #[test]
    fn row_menu_items() {
        let labels = |row: &Row, archived, local| RowAction::for_row(row, archived, local).into_iter().map(RowAction::label).collect::<Vec<_>>();
        let past = Row::Past("s1".to_string().into(), crate::hosts::Place::local("/tmp"));
        assert_eq!(labels(&Row::Open(1), Some(false), true), ["Rename", crate::platform::REVEAL_FOLDER, "Archive", "Close", "Delete…"]);
        assert_eq!(labels(&Row::Open(1), None, true), ["Rename", crate::platform::REVEAL_FOLDER, "Close", "Delete…"]);
        assert_eq!(labels(&past, Some(true), true), ["Rename", crate::platform::REVEAL_FOLDER, "Unarchive", "Delete…"]);
        assert_eq!(labels(&Row::Open(1), Some(false), false), ["Rename", "Archive", "Close", "Delete…"]);
    }
}
