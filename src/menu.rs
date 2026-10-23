//! The popup ⋮ / ⋯ menu: a sidebar row's actions or a notebook's Share/⋮
//! menu, whichever `MenuTarget` it's opened for. On Linux and Windows it is
//! also the app's menu, drawn from the menu bar macOS shows.

use std::path::Path;

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::hosts::HostId;
use crate::new_session::{self, glyph};
use crate::notebook_pane::NotebookAction;
use crate::overlay;
use crate::settings::NotebookTheme;
use wire::backend::Backend;
use crate::sidebar::{Row, RowAction};
use crate::theme;
use crate::{Interrupt, Workspace};

/// What a menu is for: a sidebar row (⋮), the open session's title in the
/// chat header (⌄), an open session's notebook (⋮), or its Share button.
#[derive(Clone, PartialEq)]
pub(crate) enum MenuTarget {
    Row(Row),
    Session(u64),
    Notebook(u64),
    Share(u64),
    /// The session menu's "Allowed in this folder": the agent's own rules for its folder.
    FolderRules(u64),
    /// The app's menu, where there's no menu bar (Linux and Windows).
    App,
}

impl MenuTarget {
    /// The session a sidebar row's or the chat header's menu acts on, and
    /// whether it's the header's.
    fn session_row(&self) -> Option<(Row, bool)> {
        match self {
            MenuTarget::Row(row) => Some((row.clone(), false)),
            MenuTarget::Session(key) => Some((Row::Open(*key), true)),
            MenuTarget::Notebook(_) | MenuTarget::Share(_) | MenuTarget::FolderRules(_) | MenuTarget::App => None,
        }
    }
}

/// A menu item, with what it acts on. `in_header`: picked from the chat
/// header's session menu, so Rename opens the name box there.
#[derive(Clone, PartialEq)]
enum MenuPick {
    Row { row: Row, action: RowAction, in_header: bool },
    Notebook(u64, NotebookAction),
    /// The app menu's item, and its group (the menu bar menu it's in).
    App(usize, u8),
}

/// An item of the app's menu, read from the menu bar's menus when it opens.
pub(crate) struct AppItem {
    /// The menu bar menu it's in: items of one menu are a group, under its name.
    menu: SharedString,
    group: u8,
    label: String,
    /// Its shortcut, as the app's other menus write one.
    keys: String,
    action: Box<dyn Action>,
    /// Cut, Copy, Paste or Select All: done in the notebook when it had the keyboard.
    edit: Option<OsAction>,
}

impl Workspace {
    /// The menu bar's menus as the app's menu: all but Window, whose items the
    /// window's own title bar has.
    fn app_items(cx: &App) -> Vec<AppItem> {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut items = Vec::new();
        for (group, menu) in cx.get_menus().unwrap_or_default().into_iter().filter(|m| m.name.as_ref() != "Window").enumerate() {
            for item in menu.items {
                if let OwnedMenuItem::Action { name, action, os_action, disabled: false, .. } = item {
                    // The last binding wins; one that works everywhere (no context) is the one to show.
                    let bindings: Vec<_> = keymap.bindings_for_action(action.as_ref()).collect();
                    let binding = bindings.iter().rev().find(|b| b.predicate().is_none()).or(bindings.last());
                    let keys = binding.map(|b| b.keystrokes().iter().map(keystroke_text).collect::<Vec<_>>().join(" ")).unwrap_or_default();
                    items.push(AppItem { menu: menu.name.clone(), group: group as u8, label: name, keys, action, edit: os_action });
                }
            }
        }
        items
    }
}

/// "Ctrl+Shift+E", the way `platform::shortcut!` writes a shortcut.
fn keystroke_text(keystroke: &KeybindingKeystroke) -> String {
    let k = keystroke.inner();
    let m = &k.modifiers;
    let mut text = String::new();
    for (on, name) in [(m.control || m.platform && !cfg!(target_os = "macos"), "Ctrl+"), (m.alt, "Alt+"), (m.shift, "Shift+"), (m.platform && cfg!(target_os = "macos"), "⌘")] {
        if on {
            text.push_str(name);
        }
    }
    let key = match k.key.as_str() {
        "escape" => "Esc".to_string(),
        key if key.chars().count() == 1 => key.to_uppercase(),
        key => {
            let mut c = key.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    };
    text + &key
}

impl MenuPick {
    fn label<'a>(&'a self, menu: &'a PopupMenu) -> &'a str {
        match self {
            MenuPick::Row { action, .. } => action.label(),
            MenuPick::Notebook(_, action) => action.label(),
            MenuPick::App(i, _) => &menu.app[*i].label,
        }
    }

    /// The key that picks it while the menu is open, and the shortcut shown.
    fn shortcut<'a>(&'a self, menu: &'a PopupMenu) -> (&'a str, &'a str) {
        match self {
            MenuPick::Row { action, .. } => action.shortcut(),
            MenuPick::Notebook(_, action) => action.shortcut(),
            MenuPick::App(i, _) => ("", &menu.app[*i].keys),
        }
    }

    /// Shown in the danger colour.
    fn danger(&self) -> bool {
        match self {
            MenuPick::Row { action, .. } => *action == RowAction::Delete,
            MenuPick::Notebook(_, action) => action.danger(),
            MenuPick::App(..) => false,
        }
    }

    /// Items in one group sit together, with a separator between groups.
    fn group(&self) -> u8 {
        match self {
            MenuPick::Row { action, .. } => (*action == RowAction::Delete) as u8,
            MenuPick::Notebook(_, action) => action.group(),
            MenuPick::App(_, group) => *group,
        }
    }

    /// A small heading over the group this item starts.
    fn section<'a>(&self, menu: &'a PopupMenu, starts_group: bool) -> Option<&'a str> {
        match self {
            MenuPick::Notebook(_, action) => action.section(),
            MenuPick::App(i, _) => starts_group.then(|| menu.app[*i].menu.as_ref()),
            MenuPick::Row { .. } => None,
        }
    }
}

/// An open ⋮ / ⋯ menu.
pub(crate) struct PopupMenu {
    pub(crate) target: MenuTarget,
    /// The folder's rules, for `FolderRules`, as read when it opened.
    rules: Vec<String>,
    /// Where a right-click opened it; None hangs it under the ⋮ button.
    at: Option<Point<Pixels>>,
    /// The item picked with the arrow keys or the pointer.
    selected: Option<usize>,
    focus: FocusHandle,
    /// Focus to give back when the menu closes.
    restore: Option<FocusHandle>,
    /// The app menu's items (`MenuTarget::App`).
    app: Vec<AppItem>,
    /// The notebook had the keyboard before the menu opened, so Edit's items act there.
    notebook_had_keyboard: bool,
}

impl Workspace {
    fn menu_picks(&self, target: &MenuTarget) -> Vec<MenuPick> {
        match target {
            MenuTarget::Row(_) | MenuTarget::Session(_) => {
                let Some((row, in_header)) = target.session_row() else { return Vec::new() };
                let mut actions = self.row_actions(&row);
                // The session menu lists the agent's folder rules, for This Mac's folders.
                if let MenuTarget::Session(key) = target
                    && self.sessions.iter().any(|s| s.key == *key && s.place.host == HostId::ThisMac && s.agent.facts().folder_rules.is_some())
                {
                    let at = actions.iter().position(|a| *a == RowAction::Rename).map_or(0, |i| i + 1);
                    actions.insert(at, RowAction::FolderRules);
                }
                actions.into_iter().map(|action| MenuPick::Row { row: row.clone(), action, in_header }).collect()
            }
            MenuTarget::FolderRules(_) => Vec::new(),
            MenuTarget::App => self.menu.as_ref().map_or(Vec::new(), |m| m.app.iter().enumerate().map(|(i, item)| MenuPick::App(i, item.group)).collect()),
            MenuTarget::Notebook(key) => {
                let session = self.sessions.iter().find(|s| s.key == *key);
                let open = session.is_some_and(|s| s.notebook.is_some() && s.stopped.is_none() && !s.missing);
                let stopped = session.is_some_and(|s| s.stopped.is_some() && !s.missing);
                let safe = session.and_then(|s| s.notebook.as_deref()).is_some_and(|id| self.page.notebook == id && self.page.safe);
                let local = session.is_some_and(|s| s.place.host == HostId::ThisMac);
                let kind = session.map_or(Backend::Pluto, |s| s.kind);
                NotebookAction::for_notebook(kind, open, stopped, safe, local).into_iter().map(|action| MenuPick::Notebook(*key, action)).collect()
            }
            MenuTarget::Share(key) => NotebookAction::for_share(self.sessions.iter().find(|s| s.key == *key).map_or(Backend::Pluto, |s| s.kind)).into_iter().map(|action| MenuPick::Notebook(*key, action)).collect(),
        }
    }

    /// The open menu for the state dump: what it's for, and its items with their keys.
    pub(crate) fn menu_state(&self) -> Option<serde_json::Value> {
        let menu = self.menu.as_ref()?;
        let target = match menu.target {
            MenuTarget::Row(_) => "row",
            MenuTarget::Session(_) => "session",
            MenuTarget::Notebook(_) => "notebook",
            MenuTarget::Share(_) => "share",
            MenuTarget::FolderRules(_) => "folder_rules",
            MenuTarget::App => "app",
        };
        if matches!(menu.target, MenuTarget::FolderRules(_)) {
            return Some(serde_json::json!({ "for": target, "rules": menu.rules, "words": menu.rules.iter().map(|r| crate::permits::rule_words(r)).collect::<Vec<_>>() }));
        }
        let items: Vec<_> = self.menu_picks(&menu.target).iter().map(|p| serde_json::json!({ "label": p.label(menu), "key": p.shortcut(menu).1 })).collect();
        Some(serde_json::json!({ "for": target, "items": items }))
    }

    pub(crate) fn open_menu(&mut self, target: MenuTarget, at: Option<Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        let notebook_had_keyboard = self.menu.as_ref().map_or_else(|| crate::webcontent::had_keyboard(self.webview.read(cx).raw()), |m| m.notebook_had_keyboard);
        let restore = match self.menu.take() {
            Some(menu) => menu.restore,
            None => window.focused(cx),
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let rules = match &target {
            MenuTarget::FolderRules(key) => self.sessions.iter().find(|s| s.key == *key).map(|s| crate::permits::folder_rules(Path::new(&s.place.path))).unwrap_or_default(),
            _ => Vec::new(),
        };
        let app = if target == MenuTarget::App { Self::app_items(cx) } else { Vec::new() };
        self.menu = Some(PopupMenu { target, rules, at, selected: None, focus, restore, app, notebook_had_keyboard });
        cx.notify();
    }

    pub(crate) fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(restore) = self.menu.take().and_then(|menu| menu.restore) {
            window.focus(&restore, cx);
        }
        cx.notify();
    }

    fn pick(&mut self, pick: MenuPick, window: &mut Window, cx: &mut Context<Self>) {
        let app_item = match pick {
            MenuPick::App(i, _) => self.menu.as_ref().and_then(|m| Some((m.app.get(i)?.action.boxed_clone(), m.app[i].edit, m.notebook_had_keyboard))),
            _ => None,
        };
        self.close_menu(window, cx);
        match pick {
            MenuPick::App(..) => {
                let Some((action, edit, notebook_had_keyboard)) = app_item else { return };
                // The web view takes the clipboard commands itself; GPUI's own text boxes take the actions.
                let webview = self.webview.read(cx).raw();
                match edit.filter(|_| notebook_had_keyboard) {
                    Some(edit) => {
                        crate::webcontent::give_keyboard(webview);
                        // ponytail: a fixed wait for the page to take its focus back; a paste sent at once finds no editor.
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                            let _ = this.update(cx, |this, cx| crate::webcontent::edit(this.webview.read(cx).raw(), edit));
                        })
                        .detach();
                    }
                    None => window.dispatch_action(action, cx),
                }
            }
            MenuPick::Row { row, action: RowAction::Rename, in_header: true } => self.start_rename(row, true, window, cx),
            MenuPick::Row { row, action, .. } => self.row_action(row, action, window, cx),
            MenuPick::Notebook(key, action) => self.notebook_action(key, action, window, cx),
        }
    }

    /// Remove one of the folder's rules from the agent's settings file, and
    /// show the list as the file now reads.
    fn remove_folder_rule(&mut self, key: u64, rule: String, cx: &mut Context<Self>) {
        let Some(folder) = self.sessions.iter().find(|s| s.key == key).map(|s| s.place.path.clone()) else { return };
        if let Err(e) = crate::permits::remove_folder_rule(Path::new(&folder), &rule) {
            eprintln!("remove folder rule {rule}: {e}");
        }
        if let Some(menu) = self.menu.as_mut().filter(|m| m.target == MenuTarget::FolderRules(key)) {
            menu.rules = crate::permits::folder_rules(Path::new(&folder));
        }
        cx.notify();
    }

    /// "Allowed in this folder": the agent's rules for the session's folder in
    /// plain words, each with Remove. Endeavor keeps no copy; it reads the file.
    fn folder_rules_body(&self, key: u64, menu: &PopupMenu, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let heading = div().px(px(8.)).pt(px(4.)).pb(px(6.)).text_size(theme::size_meta()).font_weight(FontWeight::MEDIUM).child("Allowed in this folder");
        let rows: Vec<AnyElement> = menu
            .rules
            .iter()
            .enumerate()
            .map(|(i, rule)| {
                let words = crate::permits::rule_words(rule);
                let text = div().flex_1().min_w_0().flex().flex_wrap().children(words.split('`').enumerate().map(|(j, part)| {
                    let d = div().child(part.replace(' ', "\u{a0}"));
                    if j % 2 == 1 { d.font_family(theme::MONO).text_size(theme::size_meta_small()) } else { d }
                }));
                let rule = rule.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(8.))
                    .py(px(5.))
                    .text_size(theme::size_meta())
                    .child(text)
                    .child(
                        div()
                            .id(ElementId::NamedInteger("folder-rule-remove".into(), i as u64))
                            .role(Role::Button)
                            .aria_label(format!("Remove {words}"))
                            .flex_none()
                            .cursor_pointer()
                            .text_color(theme::text_faint())
                            .hover(|s| s.text_color(theme::danger()))
                            .child("Remove")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove_folder_rule(key, rule.clone(), cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty().then(|| {
            div().px(px(8.)).pb(px(6.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child("Nothing yet. Choosing “In this folder” on a prompt adds a rule here.").into_any_element()
        });
        [heading.into_any_element()].into_iter().chain(rows).chain(empty).collect()
    }

    /// A menu, under its ⋮ / ⋯ button or at the pointer that right-clicked.
    pub(crate) fn render_menu(&self, menu: &PopupMenu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rules_body = match menu.target {
            MenuTarget::FolderRules(key) => Some(self.folder_rules_body(key, menu, cx)),
            _ => None,
        };
        let picks = self.menu_picks(&menu.target);
        let groups: Vec<u8> = picks.iter().map(MenuPick::group).collect();
        let notebook_menu = matches!(menu.target, MenuTarget::Notebook(_) | MenuTarget::Share(_));
        // The app menu can reach over the notebook when the sidebar is narrow.
        let over_notebook = notebook_menu || menu.target == MenuTarget::App;
        let items = picks.into_iter().enumerate().flat_map(|(i, pick)| {
            let danger = pick.danger();
            let new_group = i > 0 && groups[i - 1] != groups[i];
            let separator = new_group.then(|| div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::popover_edge()).into_any_element());
            let action = match &pick {
                MenuPick::Notebook(_, action) => Some(*action),
                MenuPick::Row { .. } | MenuPick::App(..) => None,
            };
            let section = pick.section(menu, i == 0 || new_group).map(|heading| {
                div().px(px(8.)).pt(px(4.)).pb(px(2.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(SharedString::from(heading.to_string())).into_any_element()
            });
            let checked = match action {
                Some(NotebookAction::LookEndeavor) => Some(self.settings.notebook_theme == NotebookTheme::Endeavor),
                Some(NotebookAction::LookClassic | NotebookAction::LookEmber) => Some(self.settings.notebook_theme == NotebookTheme::Pluto),
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
            let (label, shortcut) = (SharedString::from(pick.label(menu).to_string()), SharedString::from(pick.shortcut(menu).1.to_string()));
            let item = div()
                .id(ElementId::NamedInteger("menu-item".into(), i as u64))
                .role(Role::MenuItem)
                .aria_label(label.clone())
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(5.))
                .cursor_pointer()
                .when(danger, |d| d.text_color(theme::danger()))
                .when(menu.selected == Some(i), |d| d.bg(theme::menu_hover()))
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
                        .border_color(theme::popover_edge())
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
                .border_color(theme::popover_edge())
                .text_size(theme::size_meta_small())
                .text_color(theme::text_faint())
                .child("Exports and recordings use the notebook's standard look.")
        });
        // Over the notebook, the web view (a native view on top) lets the menu show through.
        let hole = over_notebook.then(|| {
            let webview = self.webview.read(cx);
            let (handle, under) = (webview.handle(), webview.bounds());
            canvas(move |bounds, _, _| overlay::set_hole(handle.raw(), overlay::Hole::Menu, Some(Bounds { origin: bounds.origin - under.origin, size: bounds.size })), |_, _, _, _| ()).absolute().size_full()
        });
        let body = div()
            .id("popup-menu")
            .role(Role::Menu)
            .track_focus(&menu.focus)
            .occlude()
            .w(px(if notebook_menu || rules_body.is_some() { 290. } else if menu.target == MenuTarget::App { 240. } else { 210. }))
            .p(px(4.))
            .flex()
            .flex_col()
            .map(theme::popover)
            .font_family(theme::SANS)
            .text_size(theme::size_body())
            .text_color(theme::text_primary())
            .on_action(cx.listener(|this, _: &Interrupt, window, cx| this.close_menu(window, cx)))
            .on_key_down(cx.listener(Self::menu_key))
            .children(hole)
            .children(head)
            .children(items)
            .children(rules_body.into_iter().flatten())
            .children(foot);
        // The session menu hangs from the title's left edge; an un-positioned
        // menu otherwise hangs from its button's right edge.
        let from_left = matches!(menu.target, MenuTarget::Session(_) | MenuTarget::FolderRules(_) | MenuTarget::App);
        let placed = match menu.at {
            Some(at) => anchored().position(at),
            None if from_left => anchored().anchor(Anchor::TopLeft),
            None => anchored().anchor(Anchor::TopRight),
        };
        let wrapper = div().absolute().top(px(28.));
        let wrapper = if from_left { wrapper.left_0() } else { wrapper.right_0() };
        // Over the notebook it shows at once: its hole in the web view can't fade with it.
        let body = if over_notebook { body.into_any_element() } else { crate::motion::arriving(body, "popup-menu-in", true).into_any_element() };
        wrapper.child(deferred(placed.child(body)).with_priority(1))
    }

    fn menu_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.menu.as_ref().map(|menu| menu.target.clone()) else { return };
        let picks = self.menu_picks(&target);
        if !e.keystroke.modifiers.modified()
            && let Some(pick) = self.menu.as_ref().and_then(|menu| picks.iter().find(|p| p.shortcut(menu).0 == e.keystroke.key))
        {
            cx.stop_propagation();
            return self.pick(pick.clone(), window, cx);
        }
        let Some(menu) = self.menu.as_mut() else { return };
        let n = picks.len();
        if n == 0 {
            if e.keystroke.key == "escape" {
                cx.stop_propagation();
                self.close_menu(window, cx);
            }
            return;
        }
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
}

#[cfg(test)]
mod tests {
    use super::{MenuPick, MenuTarget, NotebookAction};
    use crate::sidebar::{Row, RowAction};

    #[test]
    fn notebook_menu_groups() {
        assert!(MenuPick::Notebook(1, NotebookAction::Stop).danger());
        assert!(!MenuPick::Notebook(1, NotebookAction::Reveal).danger());
        assert_ne!(MenuPick::Notebook(1, NotebookAction::Rename).group(), MenuPick::Notebook(1, NotebookAction::LookEndeavor).group());
        assert_eq!(MenuPick::Row { row: Row::Open(1), action: RowAction::Delete, in_header: false }.group(), 1);
    }

    #[test]
    fn app_menu_shortcuts_read_like_the_apps_others() {
        let text = |s: &str| super::keystroke_text(&gpui::KeybindingKeystroke::from_keystroke(gpui::Keystroke::parse(s).unwrap()));
        if cfg!(target_os = "macos") {
            assert_eq!(text("secondary-shift-e"), "Shift+⌘E");
        } else {
            assert_eq!(text("secondary-shift-e"), "Ctrl+Shift+E");
            assert_eq!(text("ctrl-shift-g"), "Ctrl+Shift+G");
            assert_eq!(text("ctrl-,"), "Ctrl+,");
        }
        assert_eq!(text("f1"), "F1");
    }

    #[test]
    fn the_session_menu_acts_on_the_open_sessions_row() {
        assert!(MenuTarget::Session(3).session_row() == Some((Row::Open(3), true)));
        assert!(MenuTarget::Row(Row::Open(3)).session_row() == Some((Row::Open(3), false)));
        assert!(MenuTarget::Notebook(3).session_row().is_none());
    }
}
