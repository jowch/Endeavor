//! The composer, one for the chat and the new-session screen: chips for what
//! goes along (inside the box, above the text), the text with its @ mentions,
//! the send button, and the row under the box: "+", Point and the mode on the
//! left; model, effort and the context ring on the right. The new-session
//! screen adds only its chips above the box. Controls that need an open
//! notebook stay in place, greyed out, until there is one.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{SessionConfigSelectOption, SessionConfigValueId};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{InputEvent, Textarea};
use wire::files::{Reply, Request};

use crate::attach::{self, Attachment, Icon};
use crate::hosts::Place;
use crate::new_session::{Glyph, glyph};
use crate::session::{self, Entry, ModeChoice, Session};
use crate::{Workspace, context_ring, theme, tool_button};

actions!(composer, [AddFiles, ListUp, ListDown, ListPick, PickMode1, PickMode2, PickMode3, PickMode4]);

/// Keys for the composer's lists. Registered after gpui-component's, so they
/// beat the text box's own up, down, enter and tab while a list is open.
pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![KeyBinding::new("secondary-u", AddFiles, Some("Input")), KeyBinding::new("secondary-u", AddFiles, None)];
    for context in ["MentionList > Input", "ModeMenu > Input", "ModeMenu"] {
        bindings.push(KeyBinding::new("up", ListUp, Some(context)));
        bindings.push(KeyBinding::new("down", ListDown, Some(context)));
        bindings.push(KeyBinding::new("enter", ListPick, Some(context)));
    }
    bindings.push(KeyBinding::new("tab", ListPick, Some("MentionList > Input")));
    for context in ["ModeMenu > Input", "ModeMenu"] {
        bindings.push(KeyBinding::new("1", PickMode1, Some(context)));
        bindings.push(KeyBinding::new("2", PickMode2, Some(context)));
        bindings.push(KeyBinding::new("3", PickMode3, Some(context)));
        bindings.push(KeyBinding::new("4", PickMode4, Some(context)));
    }
    bindings
}

#[derive(Clone, Copy, PartialEq)]
pub enum Menu {
    Plus,
    Mode,
    /// A config option's picker ("model", "effort").
    Config(&'static str),
}

/// A folder's files for the @ list.
pub enum Files {
    Loading,
    Ready(Vec<String>),
    Failed(String),
}

/// How many rows the @ list shows.
const MENTIONS_SHOWN: usize = 8;

#[derive(Default)]
pub struct Composer {
    /// Chips in the box, sent with the next message.
    pub attachments: Vec<Attachment>,
    /// Paths picked from the @ list; their tokens in the text stay whole.
    pub mentions: Vec<String>,
    /// The text as last seen, to notice an edit that cut a mention.
    last_text: String,
    pub menu: Option<Menu>,
    /// The "@query" being typed (its byte range), which opens the @ list.
    typing: Option<(Range<usize>, String)>,
    /// The highlighted row of the open list (the @ list or the mode menu).
    selected: usize,
    /// Each folder's files, listed on the first @ there.
    files: HashMap<Place, Files>,
    /// Why the last files couldn't be attached.
    pub notice: Option<String>,
    /// The chip under the pointer, whose preview shows.
    hovered: Option<usize>,
    /// The box's height in lines as last set: (fewest, most).
    rows: std::cell::Cell<(usize, usize)>,
}

/// A sent chip's popover: which chip, and the cell's code now once the page
/// says (None inside: the cell is gone).
pub struct ChipPopover {
    pub key: u64,
    pub entry: usize,
    pub chip: usize,
    pub now: Option<Option<String>>,
}

fn icon_glyph(icon: Icon) -> Glyph {
    match icon {
        Icon::Cell => Glyph::Code,
        Icon::Cells => Glyph::Cells,
        Icon::Selection => Glyph::Lines,
        Icon::Error => Glyph::Warning,
        Icon::Region => Glyph::Region,
        Icon::Image => Glyph::Picture,
        Icon::File => Glyph::File,
    }
}

/// A 22px chip: icon, then the label (names in mono).
pub fn chip(id: ElementId, attachment: &Attachment) -> Stateful<Div> {
    let label = attachment.label();
    let error = attachment.icon() == Icon::Error;
    div()
        .id(id)
        .flex_shrink_0()
        .h(px(22.))
        .max_w(px(240.))
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(6.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme::composer_edge())
        .bg(theme::bg_tag())
        .text_size(theme::size_meta())
        .text_color(theme::text_secondary())
        .child(glyph(icon_glyph(attachment.icon()), if error { theme::danger() } else { theme::text_muted() }))
        .when(!label.plain.is_empty(), |d| d.child(div().flex_shrink_0().whitespace_nowrap().child(label.plain)))
        .when(!label.mono.is_empty(), |d| {
            d.child(div().min_w_0().truncate().font_family(theme::MONO).text_size(theme::size_meta_small()).child(label.mono))
        })
}

fn image_format(mime: &str) -> ImageFormat {
    match mime {
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::Webp,
        _ => ImageFormat::Png,
    }
}

fn mime_of(format: ImageFormat) -> Option<&'static str> {
    Some(match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::Webp => "image/webp",
        _ => return None,
    })
}

/// Code or text in a small mono panel, cut to its first lines.
fn text_panel(text: &str, lines: usize, color: Rgba) -> Div {
    let mut shown: Vec<&str> = text.lines().take(lines).collect();
    if text.lines().count() > lines {
        shown.push("…");
    }
    div()
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .bg(theme::bg_page())
        .font_family(theme::MONO)
        .text_size(theme::size_meta_small())
        .line_height(px(16.))
        .text_color(color)
        .overflow_hidden()
        .child(shown.join("\n").replace('\t', "    "))
}

/// What a chip carries, for its hover preview and its popover.
fn preview_body(attachment: &Attachment) -> Div {
    match attachment {
        Attachment::Cells { cells, .. } => div().flex().flex_col().gap(px(6.)).children(cells.iter().take(3).map(|c| text_panel(&c.code, 6, theme::text_secondary()))),
        Attachment::Selection { text, .. } => text_panel(text, 8, theme::text_secondary()),
        Attachment::Error { text, .. } => text_panel(text, 8, theme::danger()),
        Attachment::Region { png, .. } => thumbnail("image/png", png),
        Attachment::Image { mime, bytes, .. } => thumbnail(mime, bytes),
        Attachment::Text { text, .. } => text_panel(text, 8, theme::text_secondary()),
    }
}

fn thumbnail(mime: &str, bytes: &[u8]) -> Div {
    div().child(img(Arc::new(Image::from_bytes(image_format(mime), bytes.to_vec()))).max_w(px(320.)).max_h(px(200.)).rounded(px(4.)).object_fit(ObjectFit::Contain))
}

/// The preview's heading: the chip's name, and what it is.
fn preview_heading(attachment: &Attachment) -> String {
    match attachment {
        Attachment::Cells { cells, .. } if cells.len() == 1 => format!("{} · cell", cells[0].name()),
        Attachment::Cells { cells, .. } => format!("{} cells", cells.len()),
        Attachment::Selection { cell, .. } => format!("Selected in {}", cell.name()),
        Attachment::Error { cell, .. } => format!("Error in {}", cell.name()),
        Attachment::Region { cells, .. } => match cells.len() {
            1 => format!("Region over {}", cells[0].name()),
            n => format!("Region over {n} cells"),
        },
        Attachment::Image { name, bytes, .. } => format!("{name} · {}", attach::size_text(bytes.len() as u64)),
        Attachment::Text { name, text } => format!("{name} · {}", attach::size_text(text.len() as u64)),
    }
}

fn popup() -> Div {
    div()
        .p(px(4.))
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(theme::composer_edge())
        .bg(theme::bg_raised())
        .text_size(theme::size_body())
        .text_color(theme::text_primary())
}

fn popup_row(id: ElementId, selected: bool) -> Stateful<Div> {
    inert_row(id).cursor_pointer().when(selected, |d| d.bg(theme::composer_edge())).hover(|s| s.bg(theme::composer_edge()))
}

/// A menu row that can't be picked (greyed by the caller).
fn inert_row(id: ElementId) -> Stateful<Div> {
    div().id(id).flex().items_center().gap(px(8.)).px(px(8.)).py(px(5.)).rounded(px(5.))
}

impl Workspace {
    /// The folder the composer's @ list lists: the session's, or the one
    /// chosen on the new-session screen.
    fn composer_place(&self) -> Option<Place> {
        match self.active_session() {
            Some(session) => Some(session.place.clone()),
            None => self.draft.folder.clone().map(|path| Place { host: self.draft.host.clone(), path }),
        }
    }

    pub(crate) fn composer_empty(&self, cx: &App) -> bool {
        self.input.read(cx).value().trim().is_empty() && self.composer.attachments.is_empty()
    }

    /// Take what's in the composer to send: the words, the chips, and the
    /// mentions still in the words. None when there's nothing.
    pub fn take_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<(String, Vec<Attachment>, Vec<String>)> {
        if self.composer_empty(cx) {
            return None;
        }
        let text = self.input.read(cx).value().trim().to_string();
        let mentioned = attach::mentioned(&text, &self.composer.mentions);
        let attachments = std::mem::take(&mut self.composer.attachments);
        self.composer.notice = None;
        self.composer.hovered = None;
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        Some((text, attachments, mentioned))
    }

    /// Put a message back in the composer (a queued one being edited).
    pub fn restore_composer(&mut self, text: String, attachments: Vec<Attachment>, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.attachments = attachments;
        self.input.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
        window.dispatch_action(Box::new(gpui_component::input::MoveToEnd), cx);
        cx.notify();
    }

    /// The text changed: keep mentions whole, and follow an "@query" being typed.
    pub fn on_composer_input(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        let old = std::mem::replace(&mut self.composer.last_text, text.clone());
        if let Some((whole, at)) = attach::keep_tokens_whole(&old, &text, &self.composer.mentions) {
            self.composer.last_text = whole.clone();
            self.input.update(cx, |s, cx| {
                s.set_value(whole, window, cx);
                s.set_selected_range(at..at, cx);
            });
        }
        let text = self.composer.last_text.clone();
        let cursor = self.input.read(cx).cursor();
        let typing = attach::typing_mention(&text, cursor).map(|(range, query)| (range, query.to_string()));
        if typing.as_ref().map(|t| &t.1) != self.composer.typing.as_ref().map(|t| &t.1) {
            self.composer.selected = 0;
        }
        self.composer.typing = typing;
        if self.composer.typing.is_some() {
            self.composer.menu = None;
            if let Some(place) = self.composer_place() {
                self.list_files(place, cx);
            }
        }
        cx.notify();
    }

    /// List a folder's files for the @ list, once per folder and launch.
    fn list_files(&mut self, place: Place, cx: &mut Context<Self>) {
        if matches!(self.composer.files.get(&place), Some(Files::Loading | Files::Ready(_))) {
            return;
        }
        let Some(task) = self.ask_files(&place.host, Request::Files { path: place.path.display().to_string() }, cx) else { return };
        self.composer.files.insert(place.clone(), Files::Loading);
        cx.spawn(async move |this, cx| {
            let files = match task.await {
                Ok(Reply::Files { paths }) => Files::Ready(paths),
                Ok(_) => Files::Failed("This server's helper is too old to list files; reconnect to update it.".into()),
                Err(e) => Files::Failed(e),
            };
            let _ = this.update(cx, |this, cx| {
                this.composer.files.insert(place, files);
                cx.notify();
            });
        })
        .detach();
    }

    /// The @ list's rows for what's typed.
    fn mention_matches(&self) -> Vec<String> {
        let Some((_, query)) = &self.composer.typing else { return Vec::new() };
        match self.composer_place().and_then(|p| self.composer.files.get(&p)) {
            Some(Files::Ready(paths)) => attach::fuzzy(query, paths, MENTIONS_SHOWN).into_iter().map(str::to_owned).collect(),
            _ => Vec::new(),
        }
    }

    /// Put the picked path in the text where "@query" was.
    fn pick_mention(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some((range, _)) = self.composer.typing.take() else { return };
        if !self.composer.mentions.contains(&path) {
            self.composer.mentions.push(path.clone());
        }
        let token = format!("{} ", attach::token(&path));
        self.input.update(cx, |s, cx| {
            s.set_selected_range(range, cx);
            s.replace(token, window, cx);
        });
        cx.notify();
    }

    fn mode_list(&self) -> (Vec<ModeChoice>, Option<usize>) {
        match self.active_session() {
            Some(session) => (session.mode_choices(), session.current_mode()),
            None => (session::app_modes(), Some(self.draft.mode)),
        }
    }

    fn pick_mode(&mut self, i: usize, cx: &mut Context<Self>) {
        let (choices, _) = self.mode_list();
        let Some(choice) = choices.get(i).cloned() else { return };
        self.composer.menu = None;
        match self.active {
            Some(key) => {
                let effects = self.session_mut(key).map(|s| s.choose_mode(&choice)).unwrap_or_default();
                self.apply_effects(key, effects, cx);
            }
            None => self.draft.mode = i,
        }
        cx.notify();
    }

    fn list_len(&self) -> usize {
        if self.composer.typing.is_some() {
            self.mention_matches().len()
        } else if self.composer.menu == Some(Menu::Mode) {
            self.mode_list().0.len()
        } else {
            0
        }
    }

    fn list_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let n = self.list_len();
        if n == 0 {
            return;
        }
        let at = self.composer.selected.min(n - 1);
        self.composer.selected = if down { (at + 1) % n } else { (at + n - 1) % n };
        cx.notify();
    }

    pub fn list_up(&mut self, _: &ListUp, _: &mut Window, cx: &mut Context<Self>) {
        self.list_move(false, cx);
    }

    pub fn list_down(&mut self, _: &ListDown, _: &mut Window, cx: &mut Context<Self>) {
        self.list_move(true, cx);
    }

    pub fn list_pick(&mut self, _: &ListPick, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.typing.is_some() {
            if let Some(path) = self.mention_matches().into_iter().nth(self.composer.selected) {
                self.pick_mention(path, window, cx);
            } else {
                self.composer.typing = None;
                cx.notify();
            }
        } else if self.composer.menu == Some(Menu::Mode) {
            self.pick_mode(self.composer.selected, cx);
        }
    }

    pub fn pick_mode_key(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.composer.menu == Some(Menu::Mode) {
            self.pick_mode(i, cx);
        }
    }

    /// Esc closes the composer's menus and lists first.
    pub fn close_composer_menus(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.composer.menu.is_some() || self.composer.typing.is_some() || self.chip_popover.is_some();
        self.composer.menu = None;
        self.composer.typing = None;
        self.chip_popover = None;
        if open {
            cx.notify();
        }
        open
    }

    fn toggle_menu(&mut self, menu: Menu, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.menu = if self.composer.menu == Some(menu) { None } else { Some(menu) };
        self.composer.typing = None;
        if menu == Menu::Mode {
            self.composer.selected = self.mode_list().1.unwrap_or(0);
            // The menu's number keys work while the box has the keyboard.
            self.input.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    /// Pick a model or effort: for this session, and for sessions after it.
    fn pick_config(&mut self, id: &'static str, value: SessionConfigValueId, cx: &mut Context<Self>) {
        self.composer.menu = None;
        self.settings.agent_config.insert(id.to_string(), value.to_string());
        self.settings.save();
        if let Some(key) = self.active {
            let effects = self.session_mut(key).map(|s| s.set_config(id, value)).unwrap_or_default();
            self.apply_effects(key, effects, cx);
        }
        cx.notify();
    }

    /// A config option's choices for the composer: the session's, or, before
    /// one exists, the last session's with the user's picks applied.
    fn config_for(&self, session: Option<&Session>, id: &str) -> Option<(SessionConfigValueId, Vec<SessionConfigSelectOption>)> {
        if let Some(session) = session {
            return session.config_choices(id);
        }
        let (current, options) = session::config_choices(&self.agent_options, id)?;
        let picked = self.settings.agent_config.get(id).and_then(|v| options.iter().find(|o| o.value.to_string() == *v)).map(|o| o.value.clone());
        Some((picked.unwrap_or(current), options))
    }

    /// ⌘U and "+" → "Add files or photos": the native picker, several at once.
    pub fn add_files(&mut self, _: &AddFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.composer.menu = None;
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| this.attach_paths(paths, cx));
            }
        })
        .detach();
        cx.notify();
    }

    /// Read files into chips (off the main thread); refusals say why.
    pub fn attach_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let read = cx.background_spawn(async move { paths.iter().map(|p| attach::upload_file(p)).collect::<Vec<_>>() });
        cx.spawn(async move |this, cx| {
            let results = read.await;
            let _ = this.update(cx, |this, cx| {
                let mut refused = Vec::new();
                for result in results {
                    match result {
                        Ok(attachment) => this.composer.attachments.push(attachment),
                        Err(why) => refused.push(why),
                    }
                }
                this.composer.notice = (!refused.is_empty()).then(|| refused.join(" "));
                cx.notify();
            });
        })
        .detach();
    }

    /// A pasted image (or copied files) becomes a chip; text pastes as usual.
    pub fn paste_into_composer(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let mut taken = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    taken = true;
                    let ext = match image.format() {
                        ImageFormat::Jpeg => "jpg",
                        ImageFormat::Gif => "gif",
                        ImageFormat::Webp => "webp",
                        _ => "png",
                    };
                    let result = match mime_of(image.format()) {
                        Some(_) => attach::upload(&format!("pasted image.{ext}"), image.bytes().to_vec()),
                        None => Err("That image's format can't be attached; paste a PNG, JPEG, GIF or WebP.".into()),
                    };
                    match result {
                        Ok(attachment) => self.composer.attachments.push(attachment),
                        Err(why) => self.composer.notice = Some(why),
                    }
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    taken = true;
                    self.attach_paths(paths.paths().to_vec(), cx);
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if taken {
            cx.notify();
        }
        taken
    }

    // -----------------------------------------------------------------------
    // Sent chips
    // -----------------------------------------------------------------------

    /// Scroll the notebook to these cells and outline them briefly.
    pub fn reveal_cells(&self, cells: Vec<String>, cx: &mut Context<Self>) {
        self.send_to_page(&serde_json::json!({ "type": "reveal", "cells": cells }), cx);
    }

    /// A chip on a sent message: cells show in the notebook; the rest open a
    /// popover with what was sent.
    pub fn click_sent_chip(&mut self, key: u64, entry: usize, chip: usize, cx: &mut Context<Self>) {
        let Some(Entry::User { attachments, .. }) = self.sessions.iter().find(|s| s.key == key).and_then(|s| s.entries.get(entry)) else { return };
        let Some(attachment) = attachments.get(chip) else { return };
        if let Attachment::Cells { cells, .. } | Attachment::Region { cells, .. } = attachment {
            let ids = cells.iter().map(|c| c.id.clone()).collect();
            self.chip_popover = None;
            return self.reveal_cells(ids, cx);
        }
        let cell = attachment.cells().first().map(|c| c.id.clone());
        let same = self.chip_popover.as_ref().is_some_and(|p| (p.key, p.entry, p.chip) == (key, entry, chip));
        self.chip_popover = (!same).then_some(ChipPopover { key, entry, chip, now: None });
        if let (Some(cell), false) = (cell, same) {
            self.send_to_page(&serde_json::json!({ "type": "code", "cell": cell }), cx);
        }
        cx.notify();
    }

    /// The page said what a cell's code is now.
    pub fn on_cell_code(&mut self, cell: String, code: Option<String>, cx: &mut Context<Self>) {
        let Some(popover) = &self.chip_popover else { return };
        let asked = self
            .sessions
            .iter()
            .find(|s| s.key == popover.key)
            .and_then(|s| match s.entries.get(popover.entry) {
                Some(Entry::User { attachments, .. }) => attachments.get(popover.chip),
                _ => None,
            })
            .is_some_and(|a| a.cells().first().is_some_and(|c| c.id == cell));
        if asked && let Some(popover) = &mut self.chip_popover {
            popover.now = Some(code);
            cx.notify();
        }
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    /// The composer: the box and the row under it, with their menus. The chat
    /// passes its session; the new-session screen passes None.
    pub fn render_composer(&self, session: Option<&Session>, notebook_open: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = if session.is_some() { (1, 10) } else { (2, 10) };
        if self.composer.rows.replace(rows) != rows {
            let input = self.input.clone();
            window.defer(cx, move |_, cx| input.update(cx, |s, cx| s.set_auto_grow(rows.0, rows.1, cx)));
        }
        let empty = self.composer_empty(cx);
        let busy = session.is_some_and(|s| s.outbox.busy && s.id.is_some());
        let list_context = if self.composer.typing.is_some() {
            "MentionList"
        } else if self.composer.menu == Some(Menu::Mode) {
            "ModeMenu"
        } else {
            "Composer"
        };
        let chips = self.composer.attachments.iter().enumerate().map(|(i, a)| {
            chip(ElementId::NamedInteger("draft-chip".into(), i as u64), a)
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.composer.hovered = Some(i);
                    } else if this.composer.hovered == Some(i) {
                        this.composer.hovered = None;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .id(ElementId::NamedInteger("chip-remove".into(), i as u64))
                        .ml(px(1.))
                        .px(px(2.))
                        .cursor_pointer()
                        .text_color(theme::text_faint())
                        .hover(|s| s.text_color(theme::text_primary()))
                        .child("×")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if i < this.composer.attachments.len() {
                                this.composer.attachments.remove(i);
                            }
                            this.composer.hovered = None;
                            cx.notify();
                        })),
                )
        });
        let chips: Vec<_> = chips.collect();
        let send = self.send_button(empty, busy, session.is_some(), cx);
        let weak = cx.entity().downgrade();
        let text_box = Textarea::new(&self.input).appearance(false).text_size(theme::size_body()).on_paste(move |item, _, cx| {
            weak.update(cx, |this, cx| this.paste_into_composer(item, cx)).unwrap_or(false)
        });
        let token_marks = self.token_marks(cx);
        let the_box = div()
            .id("composer-box")
            .relative()
            .flex()
            .flex_col()
            .gap(px(6.))
            .min_h(px(38.))
            .pl(px(10.))
            .pr(px(7.))
            .when(!chips.is_empty(), |d| d.pt(px(7.)))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::composer_edge())
            .bg(theme::bg_card())
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.attach_paths(paths.paths().to_vec(), cx)))
            .when(!chips.is_empty(), |d| d.child(div().ml(px(10.)).flex().flex_wrap().gap(px(4.)).children(chips)))
            .child(div().flex().items_end().gap_2().child(div().relative().flex_1().min_w_0().child(text_box).child(token_marks)).child(div().mb(px(6.)).child(send)))
            .children(self.render_list(session, cx))
            .children(self.render_hover_preview());
        div()
            .key_context(list_context)
            .on_action(cx.listener(Self::list_up))
            .on_action(cx.listener(Self::list_down))
            .on_action(cx.listener(Self::list_pick))
            .on_action(cx.listener(|this, _: &PickMode1, _, cx| this.pick_mode_key(0, cx)))
            .on_action(cx.listener(|this, _: &PickMode2, _, cx| this.pick_mode_key(1, cx)))
            .on_action(cx.listener(|this, _: &PickMode3, _, cx| this.pick_mode_key(2, cx)))
            .on_action(cx.listener(|this, _: &PickMode4, _, cx| this.pick_mode_key(3, cx)))
            .flex()
            .flex_col()
            .gap_2()
            .children(self.composer.notice.clone().map(|n| div().text_size(theme::size_meta()).text_color(theme::accent_text()).child(n)))
            .child(the_box)
            .child(self.render_toolbar(session, notebook_open, cx))
            .into_any_element()
    }

    /// A tint behind each @ mention in the text, so it reads as one piece.
    fn token_marks(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let input = self.input.clone();
        let mentions = self.composer.mentions.clone();
        let _ = cx;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let state = input.read(cx);
                let text = state.value().to_string();
                for range in attach::token_ranges(&text, &mentions) {
                    let Some(b) = state.range_to_bounds(&range) else { continue };
                    let b = Bounds::from_corners(point(b.left() - px(2.), b.top()), point(b.right() + px(2.), b.bottom()));
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        window.paint_quad(fill(b, theme::accent().opacity(0.28)).corner_radii(px(3.)));
                    });
                }
            },
        )
        .absolute()
        .inset_0()
    }

    fn send_button(&self, empty: bool, busy: bool, in_chat: bool, cx: &mut Context<Self>) -> AnyElement {
        let base = div().id("send").flex_shrink_0().size(px(24.)).flex().items_center().justify_center().rounded_full();
        if empty && busy {
            return base
                .role(Role::Button)
                .aria_label("Stop (Esc)")
                .cursor_pointer()
                .bg(theme::bg_raised())
                .hover(|s| s.bg(theme::composer_edge()))
                .child(div().size(px(8.)).rounded(px(1.5)).bg(theme::text_secondary()))
                .on_click(cx.listener(|this, _, window, cx| this.interrupt(&crate::Interrupt, window, cx)))
                .into_any_element();
        }
        if empty {
            return base.bg(theme::bg_raised()).child(glyph(Glyph::ArrowUp, theme::text_faint())).into_any_element();
        }
        base.role(Role::Button)
            .aria_label("Send")
            .cursor_pointer()
            .bg(theme::accent())
            .child(glyph(Glyph::ArrowUp, theme::text_primary()))
            .on_click(cx.listener(move |this, _, window, cx| {
                if in_chat {
                    this.submit(false, window, cx);
                } else {
                    this.start_session(window, cx);
                }
            }))
            .into_any_element()
    }

    /// The open list above the box: the @ list, or a menu from the row under it.
    fn render_list(&self, session: Option<&Session>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let above = |d: Div| d.absolute().bottom(relative(1.)).mb(px(6.)).occlude();
        if self.composer.typing.is_some() {
            let status = match self.composer_place().and_then(|p| self.composer.files.get(&p)) {
                None => Some("Choose a folder first".to_string()),
                Some(Files::Loading) => Some("Listing files…".into()),
                Some(Files::Failed(why)) => Some(why.clone()),
                Some(Files::Ready(_)) => None,
            };
            let matches = self.mention_matches();
            let status = status.or_else(|| matches.is_empty().then(|| "No files match".into()));
            let rows = matches.into_iter().enumerate().map(|(i, path)| {
                let dir = path.ends_with('/');
                let pick = path.clone();
                popup_row(ElementId::NamedInteger("mention".into(), i as u64), i == self.composer.selected)
                    .child(glyph(if dir { Glyph::Folder } else { Glyph::File }, theme::text_muted()))
                    .child(div().min_w_0().truncate().font_family(theme::MONO).text_size(theme::size_code()).child(path))
                    .on_click(cx.listener(move |this, _, window, cx| this.pick_mention(pick.clone(), window, cx)))
            });
            let list = popup()
                .id("mention-list")
                .left_0()
                .right_0()
                .child(div().px(px(8.)).pt(px(2.)).pb(px(4.)).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child("Files in this session's folder"))
                .children(rows)
                .children(status.map(|s| div().px(px(8.)).py(px(4.)).text_size(theme::size_meta()).text_color(theme::text_muted()).child(s)));
            return Some(above(div().left_0().right_0()).child(list).into_any_element());
        }
        let menu = self.composer.menu?;
        let body = match menu {
            Menu::Plus => {
                let commands = session.is_some_and(|s| !s.commands.is_empty());
                popup()
                    .w(px(240.))
                    .child(
                        popup_row("plus-files".into(), false)
                            .child(glyph(Glyph::File, theme::text_muted()))
                            .child(div().flex_1().child("Add files or photos"))
                            .child(div().text_size(theme::size_meta()).text_color(theme::text_faint()).child("⌘U"))
                            .on_click(cx.listener(|this, _, window, cx| this.add_files(&AddFiles, window, cx))),
                    )
                    .child(
                        if commands { popup_row("plus-commands".into(), false) } else { inert_row("plus-commands".into()).text_color(theme::text_section()) }
                            .child(glyph(Glyph::Slash, if commands { theme::text_muted() } else { theme::text_section() }))
                            .child(div().flex_1().child("Slash commands"))
                            .when(commands, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.composer.menu = None;
                                    this.input.update(cx, |s, cx| {
                                        s.set_value("/", window, cx);
                                        s.focus(window, cx);
                                    });
                                    window.dispatch_action(Box::new(gpui_component::input::MoveToEnd), cx);
                                    cx.notify();
                                }))
                            }),
                    )
                    .into_any_element()
            }
            Menu::Mode => {
                let (choices, current) = self.mode_list();
                popup()
                    .w(px(300.))
                    .child(div().px(px(8.)).pt(px(2.)).pb(px(2.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child("Mode"))
                    .children(choices.into_iter().enumerate().map(|(i, choice)| {
                        popup_row(ElementId::NamedInteger("mode-choice".into(), i as u64), i == self.composer.selected)
                            .items_start()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(choice.name)
                                    .child(div().text_size(theme::size_meta()).text_color(theme::text_muted()).child(choice.description)),
                            )
                            .child(div().w(px(12.)).text_color(theme::accent_text()).child(if current == Some(i) { "✓" } else { "" }))
                            .children((i < 4).then(|| div().w(px(10.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child(format!("{}", i + 1))))
                            .on_click(cx.listener(move |this, _, _, cx| this.pick_mode(i, cx)))
                    }))
                    .into_any_element()
            }
            Menu::Config(id) => {
                let (current, options) = self.config_for(session, id)?;
                popup()
                    .w(px(260.))
                    .children(options.into_iter().enumerate().map(|(i, option)| {
                        let value = option.value.clone();
                        popup_row(ElementId::NamedInteger("pick".into(), i as u64), false)
                            .items_start()
                            .child(div().w(px(10.)).flex_shrink_0().text_color(theme::accent_text()).child(if option.value == current { "✓" } else { "" }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(option.name.clone())
                                    .children(option.description.clone().map(|d| div().text_size(theme::size_meta()).text_color(theme::text_muted()).child(d))),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.pick_config(id, value.clone(), cx)))
                    }))
                    .into_any_element()
            }
        };
        let place = match menu {
            Menu::Config(_) => div().right_0(),
            _ => div().left(px(-11.)),
        };
        Some(above(place).child(body).into_any_element())
    }

    /// The hovered chip's preview, above the box.
    fn render_hover_preview(&self) -> Option<AnyElement> {
        let attachment = self.composer.attachments.get(self.composer.hovered?)?;
        Some(
            div()
                .absolute()
                .bottom(relative(1.))
                .mb(px(6.))
                .left_0()
                .w(px(340.))
                .max_w(relative(1.))
                .child(
                    popup()
                        .p(px(8.))
                        .gap(px(6.))
                        .child(div().flex().items_center().gap(px(6.)).text_size(theme::size_meta()).text_color(theme::text_muted()).child(glyph(icon_glyph(attachment.icon()), theme::text_muted())).child(preview_heading(attachment)))
                        .child(preview_body(attachment)),
                )
                .into_any_element(),
        )
    }

    fn render_toolbar(&self, session: Option<&Session>, notebook_open: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let narrow = self.settings.layout.chat_width < 400.;
        let (choices, current) = self.mode_list();
        let mode = current.and_then(|i| choices.get(i)).map(|c| c.name.clone()).or_else(|| session.and_then(Session::mode_name));
        let usage = session.and_then(|s| s.usage).filter(|(_, size)| *size > 0);
        let open = |menu: Menu| self.composer.menu == Some(menu);
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(24.))
            .text_size(theme::size_meta())
            .text_color(theme::text_new())
            .child(
                tool_button("plus")
                    .role(Role::Button)
                    .aria_label("Add")
                    .when(open(Menu::Plus), |d| d.bg(theme::row_active()))
                    .child(glyph(Glyph::Plus, theme::text_secondary()))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_menu(Menu::Plus, window, cx))),
            )
            .child(if notebook_open {
                tool_button("point")
                    .gap(px(4.))
                    .when(self.annotating, |d| d.text_color(theme::accent_text()))
                    .child(glyph(Glyph::Pointer, if self.annotating { theme::accent_text() } else { theme::text_muted() }))
                    .when(!narrow, |d| d.child("Point"))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_annotation(&crate::ToggleAnnotation, window, cx)))
            } else {
                div()
                    .id("point")
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(5.))
                    .text_color(theme::text_section())
                    .child(glyph(Glyph::Pointer, theme::text_section()))
                    .when(!narrow, |d| d.child("Point"))
            })
            .children(mode.map(|name| {
                tool_button("mode")
                    .when(open(Menu::Mode), |d| d.bg(theme::row_active()))
                    .when(name == "Plan", |d| d.text_color(theme::accent_text()))
                    .child(name)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_menu(Menu::Mode, window, cx)))
            }))
            .child(div().flex_1())
            .children(
                ["model", "effort"]
                    .map(|id| {
                        let (current, options) = self.config_for(session, id)?;
                        let label = options.iter().find(|o| o.value == current).map(|o| o.name.clone())?;
                        Some(
                            tool_button(id)
                                .min_w_0()
                                .text_color(theme::text_secondary())
                                .when(open(Menu::Config(id)), |d| d.bg(theme::row_active()))
                                .child(div().min_w_0().truncate().child(label))
                                .on_click(cx.listener(move |this, _, window, cx| this.toggle_menu(Menu::Config(id), window, cx))),
                        )
                    })
                    .into_iter()
                    .flatten(),
            )
            .child(match usage {
                // Shown on hover beside the ring: a tooltip would open under the notebook.
                Some((used, size)) => div()
                    .id("context")
                    .group("context")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .px(px(5.))
                    .child(
                        div()
                            .text_color(gpui::transparent_black())
                            .group_hover("context", |s| s.text_color(theme::text_muted()))
                            .child(format!("{}% context", used * 100 / size)),
                    )
                    .child(context_ring(used as f32 / size as f32))
                    .into_any_element(),
                None => div().px(px(5.)).opacity(0.5).child(context_ring(0.)).into_any_element(),
            })
    }

    /// Chips above a sent message, and the popover of the one clicked.
    pub fn render_sent_chips(&self, key: u64, entry: usize, attachments: &[Attachment], cx: &mut Context<Self>) -> Option<AnyElement> {
        if attachments.is_empty() {
            return None;
        }
        let popover = self.chip_popover.as_ref().filter(|p| p.key == key && p.entry == entry);
        let chips = attachments.iter().enumerate().map(|(i, a)| {
            let open = popover.is_some_and(|p| p.chip == i);
            chip(ElementId::NamedInteger("sent-chip".into(), (key << 32) | ((entry as u64) << 8) | i as u64), a)
                .cursor_pointer()
                .when(open, |d| d.bg(theme::bg_raised()))
                .hover(|s| s.bg(theme::bg_raised()))
                .on_click(cx.listener(move |this, _, _, cx| this.click_sent_chip(key, entry, i, cx)))
        });
        let chips: Vec<_> = chips.collect();
        let popover = popover.and_then(|p| Some((p, attachments.get(p.chip)?))).map(|(p, a)| self.render_chip_popover(p, a, cx));
        Some(
            div()
                .relative()
                .flex()
                .flex_col()
                .items_end()
                .child(div().flex().flex_wrap().justify_end().gap(px(4.)).children(chips))
                .children(popover)
                .into_any_element(),
        )
    }

    fn render_chip_popover(&self, popover: &ChipPopover, attachment: &Attachment, cx: &mut Context<Self>) -> AnyElement {
        let sent_code = attachment.cells().first().map(|c| c.code.clone());
        let changed = match (&popover.now, &sent_code) {
            (Some(Some(now)), Some(sent)) => now.trim() != sent.trim(),
            (Some(None), Some(_)) => true,
            _ => false,
        };
        let cells: Vec<String> = attachment.cells().iter().map(|c| c.id.clone()).collect();
        let body = popup()
            .id("chip-popover")
            .occlude()
            .w(px(340.))
            .p(px(10.))
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(theme::size_meta())
                    .text_color(theme::text_muted())
                    .child(glyph(icon_glyph(attachment.icon()), if attachment.icon() == Icon::Error { theme::danger() } else { theme::text_muted() }))
                    .child(preview_heading(attachment))
                    .child(div().text_color(theme::text_faint()).child("· as sent")),
            )
            .child(preview_body(attachment))
            .when(!cells.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(theme::size_meta())
                        .child(div().flex_1().text_color(theme::text_muted()).child(if changed { "The cell has changed since." } else { "" }))
                        .child(
                            div()
                                .id("show-in-notebook")
                                .px(px(8.))
                                .py(px(3.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(theme::composer_edge())
                                .cursor_pointer()
                                .text_color(theme::text_primary())
                                .hover(|s| s.bg(theme::composer_edge()))
                                .child("Show in notebook →")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.chip_popover = None;
                                    this.reveal_cells(cells.clone(), cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.chip_popover = None;
                cx.notify();
            }));
        div().absolute().top(relative(1.)).right_0().mt(px(4.)).child(deferred(anchored().anchor(Anchor::TopRight).child(body)).with_priority(2)).into_any_element()
    }

    /// A queued message on one line: its chips, then its words cut to fit.
    pub fn render_queued(&self, n: usize, attachments: &[Attachment], text: &str) -> Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.))
            .overflow_hidden()
            .children(attachments.iter().enumerate().map(|(i, a)| chip(ElementId::NamedInteger("queued-chip".into(), ((n as u64) << 8) | i as u64), a)))
            .child(div().min_w_0().truncate().child(text.lines().next().unwrap_or("").to_string()))
    }
}

/// Keep the composer's list keys and mentions in step with the text box.
pub fn subscribe(input: &Entity<gpui_component::input::TextareaState>, window: &mut Window, cx: &mut Context<Workspace>) {
    cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| this.on_composer_input(event, window, cx)).detach();
}
