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
use crate::sidebar::{Row, RowAction};
use crate::theme;
use crate::{Interrupt, Workspace};

/// What a menu is for: a sidebar row (⋮), an open session's notebook (⋮), or
/// its Share button.
#[derive(Clone, PartialEq)]
pub(crate) enum MenuTarget {
    Row(Row),
    Notebook(u64),
    Share(u64),
}

/// A menu item, with what it acts on.
#[derive(Clone, PartialEq)]
enum MenuPick {
    Row(Row, RowAction),
    Notebook(u64, NotebookAction),
}

impl MenuPick {
    fn label(&self) -> &'static str {
        match self {
            MenuPick::Row(_, action) => action.label(),
            MenuPick::Notebook(_, action) => action.label(),
        }
    }

    fn shortcut(&self) -> (&'static str, &'static str) {
        match self {
            MenuPick::Row(_, action) => action.shortcut(),
            MenuPick::Notebook(_, action) => action.shortcut(),
        }
    }

    /// Shown in the danger colour.
    fn danger(&self) -> bool {
        match self {
            MenuPick::Row(_, action) => *action == RowAction::Delete,
            MenuPick::Notebook(_, action) => action.danger(),
        }
    }

    /// Items in one group sit together, with a separator between groups.
    fn group(&self) -> u8 {
        match self {
            MenuPick::Row(_, action) => (*action == RowAction::Delete) as u8,
            MenuPick::Notebook(_, action) => action.group(),
        }
    }
}

/// An open ⋮ / ⋯ menu.
pub(crate) struct PopupMenu {
    pub(crate) target: MenuTarget,
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
            MenuTarget::Row(row) => self.row_actions(row).into_iter().map(|action| MenuPick::Row(row.clone(), action)).collect(),
            MenuTarget::Notebook(key) => {
                let session = self.sessions.iter().find(|s| s.key == *key);
                let open = session.is_some_and(|s| s.notebook.is_some() && s.stopped.is_none() && !s.missing);
                let stopped = session.is_some_and(|s| s.stopped.is_some() && !s.missing);
                let safe = session.and_then(|s| s.notebook.as_deref()).is_some_and(|id| self.page.notebook == id && self.page.safe);
                let local = session.is_some_and(|s| s.place.host == HostId::ThisMac);
                NotebookAction::for_notebook(open, stopped, safe, local).into_iter().map(|action| MenuPick::Notebook(*key, action)).collect()
            }
            MenuTarget::Share(key) => NotebookAction::for_share().into_iter().map(|action| MenuPick::Notebook(*key, action)).collect(),
        }
    }

    pub(crate) fn open_menu(&mut self, target: MenuTarget, at: Option<Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        let restore = match self.menu.take() {
            Some(menu) => menu.restore,
            None => window.focused(cx),
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.menu = Some(PopupMenu { target, at, selected: None, focus, restore });
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
            MenuPick::Row(row, action) => self.row_action(row, action, window, cx),
            MenuPick::Notebook(key, action) => self.notebook_action(key, action, window, cx),
        }
    }

    /// A menu, under its ⋮ / ⋯ button or at the pointer that right-clicked.
    pub(crate) fn render_menu(&self, menu: &PopupMenu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let picks = self.menu_picks(&menu.target);
        let groups: Vec<u8> = picks.iter().map(MenuPick::group).collect();
        let notebook_menu = matches!(menu.target, MenuTarget::Notebook(_) | MenuTarget::Share(_));
        let items = picks.into_iter().enumerate().flat_map(|(i, pick)| {
            let danger = pick.danger();
            let new_group = i > 0 && groups[i - 1] != groups[i];
            let separator = new_group.then(|| div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(theme::popover_edge()).into_any_element());
            let action = match &pick {
                MenuPick::Notebook(_, action) => Some(*action),
                MenuPick::Row(..) => None,
            };
            let section = action.and_then(|a| a.section()).map(|heading| {
                div().px(px(8.)).pt(px(4.)).pb(px(2.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(heading).into_any_element()
            });
            let checked = match action {
                Some(NotebookAction::LookEndeavor) => Some(self.settings.notebook_theme == NotebookTheme::Endeavor),
                Some(NotebookAction::LookClassic) => Some(self.settings.notebook_theme == NotebookTheme::Pluto),
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
            .w(px(if notebook_menu { 290. } else { 210. }))
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
            .children(foot);
        let placed = match menu.at {
            Some(at) => anchored().position(at),
            None => anchored().anchor(Anchor::TopRight),
        };
        // The wrapper puts an un-positioned menu under the button's right edge.
        div().absolute().top(px(28.)).right_0().child(deferred(placed.child(body)).with_priority(1))
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
    use super::{MenuPick, NotebookAction};
    use crate::sidebar::{Row, RowAction};

    #[test]
    fn notebook_menu_groups() {
        assert!(MenuPick::Notebook(1, NotebookAction::Stop).danger());
        assert!(!MenuPick::Notebook(1, NotebookAction::Reveal).danger());
        assert_ne!(MenuPick::Notebook(1, NotebookAction::Rename).group(), MenuPick::Notebook(1, NotebookAction::LookEndeavor).group());
        assert_eq!(MenuPick::Row(Row::Open(1), RowAction::Delete).group(), 1);
    }
}
