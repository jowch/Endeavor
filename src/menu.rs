//! The popup ⋮ / ⋯ menu: a sidebar row's actions or a notebook's Share/⋮
//! menu, whichever `MenuTarget` it's opened for.

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
}

impl MenuTarget {
    /// The session a sidebar row's or the chat header's menu acts on, and
    /// whether it's the header's.
    fn session_row(&self) -> Option<(Row, bool)> {
        match self {
            MenuTarget::Row(row) => Some((row.clone(), false)),
            MenuTarget::Session(key) => Some((Row::Open(*key), true)),
            MenuTarget::Notebook(_) | MenuTarget::Share(_) | MenuTarget::FolderRules(_) => None,
        }
    }
}

/// A menu item, with what it acts on. `in_header`: picked from the chat
/// header's session menu, so Rename opens the name box there.
#[derive(Clone, PartialEq)]
enum MenuPick {
    Row { row: Row, action: RowAction, in_header: bool },
    Notebook(u64, NotebookAction),
}

impl MenuPick {
    fn label(&self) -> &'static str {
        match self {
            MenuPick::Row { action, .. } => action.label(),
            MenuPick::Notebook(_, action) => action.label(),
        }
    }

    fn shortcut(&self) -> (&'static str, &'static str) {
        match self {
            MenuPick::Row { action, .. } => action.shortcut(),
            MenuPick::Notebook(_, action) => action.shortcut(),
        }
    }

    /// Shown in the danger colour.
    fn danger(&self) -> bool {
        match self {
            MenuPick::Row { action, .. } => *action == RowAction::Delete,
            MenuPick::Notebook(_, action) => action.danger(),
        }
    }

    /// Items in one group sit together, with a separator between groups.
    fn group(&self) -> u8 {
        match self {
            MenuPick::Row { action, .. } => (*action == RowAction::Delete) as u8,
            MenuPick::Notebook(_, action) => action.group(),
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
        };
        if matches!(menu.target, MenuTarget::FolderRules(_)) {
            return Some(serde_json::json!({ "for": target, "rules": menu.rules, "words": menu.rules.iter().map(|r| crate::permits::rule_words(r)).collect::<Vec<_>>() }));
        }
        let items: Vec<_> = self.menu_picks(&menu.target).iter().map(|p| serde_json::json!({ "label": p.label(), "key": p.shortcut().1 })).collect();
        Some(serde_json::json!({ "for": target, "items": items }))
    }

    pub(crate) fn open_menu(&mut self, target: MenuTarget, at: Option<Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
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
        self.menu = Some(PopupMenu { target, rules, at, selected: None, focus, restore });
        cx.notify();
    }

    pub(crate) fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(restore) = self.menu.take().and_then(|menu| menu.restore) {
            window.focus(&restore, cx);
        }
        cx.notify();
    }

    fn pick(&mut self, pick: MenuPick, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(window, cx);
        match pick {
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
        let items = picks.into_iter().enumerate().flat_map(|(i, pick)| {
            let danger = pick.danger();
            let new_group = i > 0 && groups[i - 1] != groups[i];
            let separator = new_group.then(|| div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::popover_edge()).into_any_element());
            let action = match &pick {
                MenuPick::Notebook(_, action) => Some(*action),
                MenuPick::Row { .. } => None,
            };
            let section = action.and_then(|a| a.section()).map(|heading| {
                div().px(px(8.)).pt(px(4.)).pb(px(2.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(heading).into_any_element()
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
            .w(px(if notebook_menu || rules_body.is_some() { 290. } else { 210. }))
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
        let from_left = matches!(menu.target, MenuTarget::Session(_) | MenuTarget::FolderRules(_));
        let placed = match menu.at {
            Some(at) => anchored().position(at),
            None if from_left => anchored().anchor(Anchor::TopLeft),
            None => anchored().anchor(Anchor::TopRight),
        };
        let wrapper = div().absolute().top(px(28.));
        let wrapper = if from_left { wrapper.left_0() } else { wrapper.right_0() };
        // Over the notebook it shows at once: its hole in the web view can't fade with it.
        let body = if notebook_menu { body.into_any_element() } else { crate::motion::arriving(body, "popup-menu-in", true).into_any_element() };
        wrapper.child(deferred(placed.child(body)).with_priority(1))
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
    fn the_session_menu_acts_on_the_open_sessions_row() {
        assert!(MenuTarget::Session(3).session_row() == Some((Row::Open(3), true)));
        assert!(MenuTarget::Row(Row::Open(3)).session_row() == Some((Row::Open(3), false)));
        assert!(MenuTarget::Notebook(3).session_row().is_none());
    }
}
