use std::path::Path;

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::hosts::HostId;
use crate::new_session::{self, glyph};
use crate::notebook_pane::NotebookAction;
use crate::layer::{self, Hole, Layer};
use crate::settings::NotebookTheme;
use wire::backend::Backend;
use crate::sidebar::{Row, RowAction};
use crate::theme;
use crate::theme::TabStop as _;
use crate::{Interrupt, Workspace};

#[derive(Clone, PartialEq)]
pub(crate) enum MenuTarget {
    Row(Row),
    Session(u64),
    Notebook(u64),
    Share(u64),
    FolderRules(u64),
    App,
}

impl MenuTarget {
    fn session_row(&self) -> Option<(Row, bool)> {
        match self {
            MenuTarget::Row(row) => Some((row.clone(), false)),
            MenuTarget::Session(key) => Some((Row::Open(*key), true)),
            MenuTarget::Notebook(_) | MenuTarget::Share(_) | MenuTarget::FolderRules(_) | MenuTarget::App => None,
        }
    }
}

#[derive(Clone, PartialEq)]
enum MenuPick {
    Row { row: Row, action: RowAction, in_header: bool },
    Notebook(u64, NotebookAction),
    App(usize, u8),
}

pub(crate) struct AppItem {
    menu: SharedString,
    group: u8,
    label: String,
    keys: String,
    action: Box<dyn Action>,
    edit: Option<OsAction>,
}

impl Workspace {
    fn app_items(cx: &App) -> Vec<AppItem> {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut items = Vec::new();
        for (group, menu) in cx.get_menus().unwrap_or_default().into_iter().filter(|m| m.name.as_ref() != "Window").enumerate() {
            for item in menu.items {
                if let OwnedMenuItem::Action { name, action, os_action, disabled: false, .. } = item {
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

    fn shortcut<'a>(&'a self, menu: &'a PopupMenu) -> (&'a str, &'a str) {
        match self {
            MenuPick::Row { action, .. } => action.shortcut(),
            MenuPick::Notebook(_, action) => action.shortcut(),
            MenuPick::App(i, _) => ("", &menu.app[*i].keys),
        }
    }

    fn danger(&self) -> bool {
        match self {
            MenuPick::Row { action, .. } => *action == RowAction::Delete,
            MenuPick::Notebook(_, action) => action.danger(),
            MenuPick::App(..) => false,
        }
    }

    fn group(&self) -> u8 {
        match self {
            MenuPick::Row { action, .. } => (*action == RowAction::Delete) as u8,
            MenuPick::Notebook(_, action) => action.group(),
            MenuPick::App(_, group) => *group,
        }
    }

    fn section<'a>(&self, menu: &'a PopupMenu, starts_group: bool) -> Option<&'a str> {
        match self {
            MenuPick::Notebook(_, action) => action.section(),
            MenuPick::App(i, _) => starts_group.then(|| menu.app[*i].menu.as_ref()),
            MenuPick::Row { .. } => None,
        }
    }
}

pub(crate) struct PopupMenu {
    pub(crate) target: MenuTarget,
    rules: Vec<String>,
    at: Option<Point<Pixels>>,
    selected: Option<usize>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    app: Vec<AppItem>,
    notebook_had_keyboard: bool,
    pub(crate) before: Before,
}

impl Workspace {
    fn menu_picks(&self, target: &MenuTarget) -> Vec<MenuPick> {
        match target {
            MenuTarget::Row(_) | MenuTarget::Session(_) => {
                let Some((row, in_header)) = target.session_row() else { return Vec::new() };
                let mut actions = self.row_actions(&row);
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
            MenuTarget::Share(key) => {
                let Some(session) = self.sessions.iter().find(|s| s.key == *key) else { return Vec::new() };
                let exports_python = self.connection(&session.place.host).and_then(|c| c.runtime.as_ref()).is_some_and(|r| r.version.exports_python());
                NotebookAction::for_share(session.kind, exports_python).into_iter().map(|action| MenuPick::Notebook(*key, action)).collect()
            }
        }
    }

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
        let chat = self.menu.as_ref().map_or(self.zoom_chat_before_click, |m| m.before.chat);
        let before = Before { notebook: notebook_had_keyboard, chat };
        self.menu = Some(PopupMenu { target, rules, at, selected: None, focus, restore, app, notebook_had_keyboard, before });
        cx.notify();
    }

    pub(crate) fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismiss_menu(window, cx) {
            crate::webcontent::give_keyboard(self.webview.read(cx).raw());
        }
    }

    fn dismiss_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let menu = self.menu.take();
        if let Some(restore) = menu.as_ref().and_then(|menu| menu.restore.clone()) {
            window.focus(&restore, cx);
        }
        cx.notify();
        menu.is_some_and(|menu| menu.notebook_had_keyboard)
    }

    fn pick(&mut self, pick: MenuPick, window: &mut Window, cx: &mut Context<Self>) {
        let app_item = match pick {
            MenuPick::App(i, _) => self.menu.as_ref().and_then(|m| Some((m.app.get(i)?.action.boxed_clone(), m.app[i].edit, m.notebook_had_keyboard, m.before))),
            _ => None,
        };
        self.dismiss_menu(window, cx);
        match pick {
            MenuPick::App(..) => {
                let Some((action, edit, notebook_had_keyboard, before)) = app_item else { return };
                let webview = self.webview.read(cx).raw();
                match edit.filter(|_| notebook_had_keyboard) {
                    Some(edit) => {
                        crate::webcontent::give_keyboard(webview);
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                            let _ = this.update(cx, |this, cx| crate::webcontent::edit(this.webview.read(cx).raw(), edit));
                        })
                        .detach();
                    }
                    None => {
                        if notebook_had_keyboard && !takes_keyboard(action.as_ref()) {
                            crate::webcontent::give_keyboard(webview);
                        }
                        match zoom_step(action.as_ref()) {
                            Some((step, reset)) => self.zoom(step, reset, Some(before), cx),
                            None => window.dispatch_action(action, cx),
                        }
                    }
                }
            }
            MenuPick::Row { row, action: RowAction::Rename, in_header: true } => self.start_rename(row, true, window, cx),
            MenuPick::Row { row, action, .. } => self.row_action(row, action, window, cx),
            MenuPick::Notebook(key, action) => self.notebook_action(key, action, window, cx),
        }
    }

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

    fn folder_rules_body(&self, key: u64, menu: &PopupMenu, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let heading = div().px(px(8.)).pt(px(4.)).pb(px(6.)).text_size(theme::size_meta()).font_weight(FontWeight::MEDIUM).child(theme::words("allowed-in-this-folder", "Allowed in this folder"));
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
                            .tab_stop_on(theme::popover_bg())
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
            div().px(px(8.)).pb(px(6.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(theme::words("nothing-yet-choosing-in-this", "Nothing yet. Choosing “In this folder” on a prompt adds a rule here.")).into_any_element()
        });
        [heading.into_any_element()].into_iter().chain(rows).chain(empty).collect()
    }

    pub(crate) fn render_menu(&self, menu: &PopupMenu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rules_body = match menu.target {
            MenuTarget::FolderRules(key) => Some(self.folder_rules_body(key, menu, cx)),
            _ => None,
        };
        let picks = self.menu_picks(&menu.target);
        let groups: Vec<u8> = picks.iter().map(MenuPick::group).collect();
        let notebook_menu = matches!(menu.target, MenuTarget::Notebook(_) | MenuTarget::Share(_));
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
                .tab_stop_inside()
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
            .children(head)
            .children(items)
            .children(rules_body.into_iter().flatten())
            .children(foot);
        let from_left = matches!(menu.target, MenuTarget::Session(_) | MenuTarget::FolderRules(_) | MenuTarget::App);
        let placed = match menu.at {
            Some(at) => anchored().position(at),
            None if from_left => anchored().anchor(Anchor::TopLeft),
            None => anchored().anchor(Anchor::TopRight),
        };
        let wrapper = div().absolute().top(px(28.));
        let wrapper = if from_left { wrapper.left_0() } else { wrapper.right_0() };
        let body = layer::cut(body, Hole::Menu, over_notebook.then(|| self.webview.read(cx)));
        let body = if over_notebook { body.into_any_element() } else { crate::motion::arriving(body, "popup-menu-in", true).into_any_element() };
        wrapper.child(layer::place(Layer::Popover, placed.child(body)))
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct Before {
    pub(crate) notebook: bool,
    pub(crate) chat: bool,
}

fn zoom_step(action: &dyn gpui::Action) -> Option<(f64, bool)> {
    let any = action.as_any();
    if any.is::<crate::ZoomIn>() {
        Some((1.1, false))
    } else if any.is::<crate::ZoomOut>() {
        Some((1. / 1.1, false))
    } else if any.is::<crate::ZoomReset>() {
        Some((1., true))
    } else {
        None
    }
}

fn takes_keyboard(action: &dyn gpui::Action) -> bool {
    use crate::{FindSetting, NewSession, OpenSettings, Quit, ShowAbout, ToggleAnnotation};
    let any = action.as_any();
    any.is::<OpenSettings>() || any.is::<ShowAbout>() || any.is::<FindSetting>() || any.is::<ToggleAnnotation>() || any.is::<NewSession>() || any.is::<Quit>()
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
        let text = |s: &'static str| super::keystroke_text(&gpui::KeybindingKeystroke::from_keystroke(gpui::Keystroke::parse(s).unwrap()));
        if cfg!(target_os = "macos") {
            assert_eq!(text(crate::platform::keys!("shift-e")), "Shift+⌘E");
        } else {
            assert_eq!(text(crate::platform::keys!("shift-e")), "Ctrl+Shift+E");
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
