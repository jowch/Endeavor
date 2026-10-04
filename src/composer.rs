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
use crate::hosts::{HostId, Place};
use crate::new_session::{Glyph, glyph};
use crate::session::{self, Entry, ModeChoice, Session};
use crate::slash::{self, Listing, Own};
use crate::{Workspace, theme, tool_button};
use crate::theme::FocusRing as _;

actions!(composer, [AddFiles, ListUp, ListDown, ListPick, ListFill, PickMode1, PickMode2, PickMode3, PickMode4]);

/// Keys for the composer's lists. Registered after gpui-component's, so they
/// beat the text box's own up, down, enter and tab while a list is open.
pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![KeyBinding::new("secondary-u", AddFiles, Some("Input")), KeyBinding::new("secondary-u", AddFiles, None)];
    for context in ["MentionList > Input", "SlashList > Input", "ModeMenu > Input", "ModeMenu"] {
        bindings.push(KeyBinding::new("up", ListUp, Some(context)));
        bindings.push(KeyBinding::new("down", ListDown, Some(context)));
        bindings.push(KeyBinding::new("enter", ListPick, Some(context)));
    }
    bindings.push(KeyBinding::new("tab", ListPick, Some("MentionList > Input")));
    bindings.push(KeyBinding::new("tab", ListFill, Some("SlashList > Input")));
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

/// The slash list's height: about 12 rows, then it scrolls.
const SLASH_LIST_MAX: f32 = 340.;

/// A choice after /mode or /model, as the toolbar's menus offer it.
#[derive(Clone)]
pub(crate) struct Choice {
    pub name: String,
    pub description: Option<String>,
    pub current: bool,
    pick: ChoicePick,
}

#[derive(Clone)]
enum ChoicePick {
    Mode(usize),
    Model(SessionConfigValueId),
}

/// The slash list as it stands for the box's text.
pub(crate) enum SlashView {
    /// "/" and a name being typed: the commands that match.
    Commands { commands: Vec<slash::Command>, listing: Listing, query: String, waiting: bool },
    /// "/mode " or "/model ": its choices.
    Choices { own: Own, choices: Vec<Choice> },
}

impl SlashView {
    fn len(&self) -> usize {
        match self {
            SlashView::Commands { listing: Listing::Names(rows) | Listing::Descriptions(rows), .. } => rows.len(),
            SlashView::Commands { .. } => 0,
            SlashView::Choices { choices, .. } => choices.len(),
        }
    }

    /// The row selected as the list opens or its text changes: the first, or
    /// the current choice when every choice shows.
    fn initial(&self) -> usize {
        match self {
            SlashView::Choices { choices, .. } => choices.iter().position(|c| c.current).unwrap_or(0),
            SlashView::Commands { .. } => 0,
        }
    }
}

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
    /// The highlighted row of the open list (the @, slash or mode list).
    selected: usize,
    /// The text the slash list was closed on with Esc; it opens again once
    /// the text changes.
    slash_closed: Option<String>,
    slash_scroll: ScrollHandle,
    /// Each folder's files, listed on the first @ there.
    files: HashMap<Place, Files>,
    /// Why the last files couldn't be attached.
    pub notice: Option<String>,
    /// The chip under the pointer, whose preview shows.
    hovered: Option<usize>,
    /// The box's height in lines as last set: (fewest, most).
    rows: std::cell::Cell<(usize, usize)>,
    /// Toolbar buttons' Tab-stop handles.
    focus_plus: FocusHandle,
    focus_point: FocusHandle,
    focus_mode: FocusHandle,
    /// The config chips' (model, effort, speed), in the agent's order.
    focus_config: [FocusHandle; 3],
    focus_send: FocusHandle,
    pub(crate) focus_context: FocusHandle,
    /// The context ring's popover.
    pub context: crate::context::RingPopover,
    /// Draft-chip remove buttons, resized to match `attachments` each render.
    /// A `RefCell` (like `rows` above) since render only has `&self`.
    focus_chip_remove: std::cell::RefCell<Vec<FocusHandle>>,
}

impl Composer {
    pub fn new(cx: &App) -> Self {
        Self {
            attachments: Vec::new(),
            mentions: Vec::new(),
            last_text: String::new(),
            menu: None,
            typing: None,
            selected: 0,
            slash_closed: None,
            slash_scroll: ScrollHandle::new(),
            files: HashMap::new(),
            notice: None,
            hovered: None,
            rows: std::cell::Cell::new((0, 0)),
            focus_plus: cx.focus_handle().tab_stop(true),
            focus_point: cx.focus_handle().tab_stop(true),
            focus_mode: cx.focus_handle().tab_stop(true),
            focus_config: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            focus_send: cx.focus_handle().tab_stop(true),
            focus_context: cx.focus_handle().tab_stop(true),
            context: Default::default(),
            focus_chip_remove: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// The remove button's handle for chip `i`, growing the pool if this is a new attachment.
    fn chip_remove_focus(&self, i: usize, cx: &App) -> FocusHandle {
        let mut pool = self.focus_chip_remove.borrow_mut();
        while pool.len() <= i {
            pool.push(cx.focus_handle().tab_stop(true));
        }
        pool[i].clone()
    }
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
        Icon::Quote => Glyph::Lines,
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
        .text_size(theme::chat_meta())
        .text_color(theme::text_secondary())
        .child(glyph(icon_glyph(attachment.icon()), if error { theme::danger() } else { theme::text_muted() }))
        .when(!label.plain.is_empty(), |d| d.child(div().flex_shrink_0().whitespace_nowrap().child(label.plain)))
        .when(!label.mono.is_empty(), |d| {
            d.child(div().min_w_0().truncate().font_family(theme::MONO).text_size(theme::chat_meta_small()).child(label.mono))
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
        .text_size(theme::chat_meta_small())
        .line_height(px(16.))
        .text_color(color)
        .overflow_hidden()
        .child(shown.join("\n").replace('\t', "    "))
}

/// What a chip carries, for its hover preview and its popover.
fn preview_body(attachment: &Attachment) -> Div {
    match attachment {
        Attachment::Cells { cells, .. } => div().flex().flex_col().gap(px(6.)).children(cells.iter().take(3).map(|c| text_panel(&c.code, 6, theme::text_secondary()))),
        Attachment::Error { text, .. } => text_panel(text, 8, theme::danger()),
        Attachment::Quote(quote) => match (quote.picture(), quote.excerpt()) {
            (Some(png), _) => thumbnail("image/png", png),
            (None, excerpt) => text_panel(excerpt.unwrap_or_default(), 8, theme::text_secondary()),
        },
        Attachment::Image { mime, bytes, .. } => thumbnail(mime, bytes),
        Attachment::Text { text, .. } => text_panel(text, 8, theme::text_secondary()),
        Attachment::Upload { .. } => note("A copy goes into this session's folder when you send, so Claude and the notebook can use it."),
        Attachment::Saved { .. } => note("Saved in this session's folder."),
    }
}

fn note(text: &'static str) -> Div {
    div().text_size(theme::chat_meta()).text_color(theme::text_secondary()).child(text)
}

fn thumbnail(mime: &str, bytes: &[u8]) -> Div {
    div().child(img(Arc::new(Image::from_bytes(image_format(mime), bytes.to_vec()))).max_w(px(320.)).max_h(px(200.)).rounded(px(4.)).object_fit(ObjectFit::Contain))
}

/// The preview's heading: the chip's name, and what it is.
fn preview_heading(attachment: &Attachment) -> String {
    match attachment {
        Attachment::Cells { cells, .. } if cells.len() == 1 => format!("{} · cell", cells[0].name()),
        Attachment::Cells { cells, .. } => format!("{} cells", cells.len()),
        Attachment::Error { cell, .. } => format!("Error in {}", cell.name()),
        Attachment::Quote(quote) => quote.source(),
        Attachment::Image { name, bytes, .. } => format!("{name} · {}", attach::size_text(bytes.len() as u64)),
        Attachment::Text { name, text } => format!("{name} · {}", attach::size_text(text.len() as u64)),
        Attachment::Upload { source, size } => {
            format!("{} · {}", source.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), attach::size_text(*size))
        }
        Attachment::Saved { path } => path.clone(),
    }
}

fn popup() -> Div {
    div()
        .p(px(4.))
        .flex()
        .flex_col()
        .map(theme::popover)
        .text_size(theme::chat_body())
        .text_color(theme::text_primary())
}

fn popup_row(id: ElementId, selected: bool) -> Stateful<Div> {
    inert_row(id).cursor_pointer().when(selected, |d| d.bg(theme::menu_hover())).hover(|s| s.bg(theme::menu_hover()))
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
        if text != old
            && let Some(view) = self.slash_view(cx)
        {
            self.composer.menu = None;
            self.composer.selected = view.initial();
            self.composer.slash_scroll.scroll_to_item(self.composer.selected);
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

    /// Put an @ mention of `path` in the text at the cursor, as its own word.
    fn insert_mention(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composer.mentions.contains(&path) {
            self.composer.mentions.push(path.clone());
        }
        let text = self.input.read(cx).value().to_string();
        let cursor = self.input.read(cx).cursor();
        let after_word = text.get(..cursor).and_then(|t| t.chars().next_back()).is_some_and(|c| !c.is_whitespace());
        let token = format!("{}{} ", if after_word { " " } else { "" }, attach::token(&path));
        self.input.update(cx, |s, cx| s.replace(token, window, cx));
    }

    /// Put the picked path in the text where "@query" was.
    fn pick_mention(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some((range, _)) = self.composer.typing.take() else { return };
        self.file_tip_done();
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

    /// Every slash command: Endeavor's, and Claude Code's once it has sent them.
    pub(crate) fn slash_commands(&self) -> Vec<slash::Command> {
        slash::commands(self.active_session().map(|s| s.commands.as_slice()).filter(|c| !c.is_empty()))
    }

    /// The choices after /mode or /model that match what follows it.
    fn own_choices(&self, own: Own, query: &str) -> Vec<Choice> {
        let all: Vec<Choice> = match own {
            Own::Mode => {
                let (choices, current) = self.mode_list();
                choices
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| Choice { name: c.name, description: Some(c.description), current: current == Some(i), pick: ChoicePick::Mode(i) })
                    .collect()
            }
            Own::Model => match self.config_for(self.active_session(), "model") {
                Some((current, options)) => options
                    .into_iter()
                    .map(|o| Choice { current: o.value == current, name: o.name, description: o.description, pick: ChoicePick::Model(o.value) })
                    .collect(),
                None => Vec::new(),
            },
        };
        let names: Vec<String> = all.iter().map(|c| c.name.clone()).collect();
        slash::choices(query, &names).into_iter().map(|i| all[i].clone()).collect()
    }

    /// The slash list for the box's text, unless Esc closed it.
    pub(crate) fn slash_view(&self, cx: &App) -> Option<SlashView> {
        if self.composer.typing.is_some() {
            return None;
        }
        let text = self.input.read(cx).value().to_string();
        if self.composer.slash_closed.as_deref() == Some(text.as_str()) {
            return None;
        }
        if let Some(query) = slash::typing(&text) {
            let commands = self.slash_commands();
            let listing = slash::filter(query, &commands);
            let waiting = self.active_session().is_none_or(|s| s.commands.is_empty());
            return Some(SlashView::Commands { commands, listing, query: query.to_string(), waiting });
        }
        let (own, rest) = slash::own_command(&text)?;
        Some(SlashView::Choices { own, choices: self.own_choices(own, rest) })
    }

    /// Put `text` in the box with the caret at its end.
    pub(crate) fn set_composer_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let end = text.len();
        self.input.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.set_selected_range(end..end, cx);
            s.focus(window, cx);
        });
        if let Some(view) = self.slash_view(cx) {
            self.composer.selected = view.initial();
            self.composer.slash_scroll.scroll_to_item(self.composer.selected);
        }
    }

    /// Send what's in the box: to the session, or to start one.
    fn send_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_some() {
            self.submit(false, window, cx);
        } else {
            self.start_session(window, cx);
        }
    }

    /// ⏎ (or a click) on the slash list's row `i`: a command that takes
    /// nothing runs; one that takes words, and Endeavor's, are filled in. ⇥
    /// (`fill`) only fills in. A choice after /mode or /model applies.
    fn slash_choose(&mut self, view: SlashView, i: usize, fill: bool, window: &mut Window, cx: &mut Context<Self>) {
        match view {
            SlashView::Commands { commands, listing, .. } => {
                let rows = match listing {
                    Listing::Names(rows) | Listing::Descriptions(rows) => rows,
                    // Not a command: ⏎ sends it as a message.
                    Listing::Nothing if !fill => return self.send_composer(window, cx),
                    Listing::Nothing => return,
                };
                let Some(command) = rows.get(i).map(|row| &commands[row.command]) else { return };
                if fill || command.own.is_some() || command.words.is_some() {
                    self.set_composer_text(format!("/{} ", command.name), window, cx);
                } else {
                    self.set_composer_text(format!("/{}", command.name), window, cx);
                    self.send_composer(window, cx);
                }
            }
            SlashView::Choices { choices, .. } => {
                if let Some(choice) = choices.into_iter().nth(i) {
                    self.apply_choice(choice.pick, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// A /mode or /model choice: applied at once, as the toolbar's menu
    /// would; the box clears and nothing is sent.
    fn apply_choice(&mut self, pick: ChoicePick, window: &mut Window, cx: &mut Context<Self>) {
        match pick {
            ChoicePick::Mode(i) => self.pick_mode(i, cx),
            ChoicePick::Model(value) => self.pick_config("model", value, cx),
        }
        self.composer.slash_closed = None;
        self.set_composer_text(String::new(), window, cx);
    }

    /// Sending "/mode …" or "/model …" (with the list closed, or ⌘⏎): it is
    /// Endeavor's, so it applies the best match and is never sent. With
    /// nothing after it, or no match, the list opens. False: not Endeavor's.
    pub(crate) fn run_own_command(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.input.read(cx).value().trim().to_string();
        let names = |own| self.own_choices(own, "").into_iter().map(|c| c.name).collect();
        let Some(send) = slash::own_send(&text, names) else { return false };
        self.composer.slash_closed = None;
        match send {
            slash::OwnSend::Apply(own, i) => {
                if let Some(choice) = self.own_choices(own, "").into_iter().nth(i) {
                    self.apply_choice(choice.pick, window, cx);
                }
            }
            slash::OwnSend::Open(own) if slash::own_command(&text).is_some_and(|(_, rest)| rest.is_empty()) => {
                let name = if own == Own::Mode { "mode" } else { "model" };
                self.set_composer_text(format!("/{name} "), window, cx);
            }
            slash::OwnSend::Open(_) => {}
        }
        cx.notify();
        true
    }

    /// The slash list and the command token, for the debug state.
    pub(crate) fn slash_state(&self, cx: &App) -> serde_json::Value {
        use serde_json::json;
        let text = self.input.read(cx).value().to_string();
        let commands = self.slash_commands();
        let token = slash::token(&text, &commands).map(|(len, c)| json!({ "text": &text[..len], "hint": c.words.clone().filter(|_| text.len() == len + 1) }));
        let selected = self.composer.selected;
        let list = match self.slash_view(cx) {
            None => serde_json::Value::Null,
            Some(SlashView::Commands { commands, listing, waiting, .. }) => {
                let (kind, rows) = match listing {
                    Listing::Names(rows) => ("names", rows),
                    Listing::Descriptions(rows) => ("descriptions", rows),
                    Listing::Nothing => ("nothing", Vec::new()),
                };
                let rows: Vec<_> = rows
                    .iter()
                    .enumerate()
                    .map(|(i, row)| {
                        let c = &commands[row.command];
                        let source = if kind == "descriptions" { &c.description } else { &c.name };
                        json!({
                            "group": c.group.heading(),
                            "name": format!("/{}", c.name),
                            "hint": c.hint,
                            "marked": row.marks.iter().map(|r| &source[r.clone()]).collect::<Vec<_>>(),
                            "selected": i == selected,
                        })
                    })
                    .collect();
                json!({ "shows": kind, "rows": rows, "waiting": waiting })
            }
            Some(SlashView::Choices { own, choices }) => json!({
                "shows": if own == Own::Mode { "modes" } else { "models" },
                "rows": choices.iter().enumerate().map(|(i, c)| json!({ "name": c.name, "current": c.current, "selected": i == selected })).collect::<Vec<_>>(),
            }),
        };
        json!({ "list": list, "token": token })
    }

    fn list_len(&self, cx: &App) -> usize {
        if let Some(view) = self.slash_view(cx) {
            view.len()
        } else if self.composer.typing.is_some() {
            self.mention_matches().len()
        } else if self.composer.menu == Some(Menu::Mode) {
            self.mode_list().0.len()
        } else {
            0
        }
    }

    fn list_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let n = self.list_len(cx);
        if n == 0 {
            return;
        }
        let at = self.composer.selected.min(n - 1);
        self.composer.selected = if down { (at + 1) % n } else { (at + n - 1) % n };
        self.composer.slash_scroll.scroll_to_item(self.composer.selected);
        cx.notify();
    }

    pub fn list_up(&mut self, _: &ListUp, _: &mut Window, cx: &mut Context<Self>) {
        self.list_move(false, cx);
    }

    pub fn list_down(&mut self, _: &ListDown, _: &mut Window, cx: &mut Context<Self>) {
        self.list_move(true, cx);
    }

    pub fn list_pick(&mut self, _: &ListPick, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.slash_view(cx) {
            self.slash_choose(view, self.composer.selected, false, window, cx);
        } else if self.composer.typing.is_some() {
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

    pub fn list_fill(&mut self, _: &ListFill, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.slash_view(cx) {
            self.slash_choose(view, self.composer.selected, true, window, cx);
        }
    }

    pub fn pick_mode_key(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.composer.menu == Some(Menu::Mode) {
            self.pick_mode(i, cx);
        }
    }

    /// Esc closes the composer's menus and lists first.
    pub fn close_composer_menus(&mut self, cx: &mut Context<Self>) -> bool {
        let slash = self.slash_view(cx).is_some();
        if slash {
            self.composer.slash_closed = Some(self.input.read(cx).value().to_string());
        }
        let open = slash || self.composer.menu.is_some() || self.composer.typing.is_some() || self.chip_popover.is_some();
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

    /// Pick a model or effort: for this session, and for the agent's sessions after it.
    fn pick_config(&mut self, id: &'static str, value: SessionConfigValueId, cx: &mut Context<Self>) {
        self.composer.menu = None;
        let agent = self.composer_agent(self.active_session());
        self.settings.config_picks_mut(agent).insert(id.to_string(), value.to_string());
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
        let agent = self.draft_agent();
        let (current, options) = session::config_choices(self.agent_options.get(&agent)?, id)?;
        let picked = self.settings.config_picks(agent).get(id).and_then(|v| options.iter().find(|o| o.value.to_string() == *v)).map(|o| o.value.clone());
        Some((picked.unwrap_or(current), options))
    }

    /// The agent the composer is for: the session's, or the new session's.
    pub fn composer_agent(&self, session: Option<&Session>) -> crate::agent::Agent {
        session.map_or_else(|| self.draft_agent(), |s| s.agent)
    }

    /// The toolbar's mode ("Ask to run", "Plan"…).
    pub fn mode_label(&self, session: Option<&Session>) -> Option<String> {
        let (choices, current) = self.mode_list();
        current.and_then(|i| choices.get(i)).map(|c| c.name.clone()).or_else(|| session.and_then(Session::mode_name))
    }

    /// The toolbar's pick for a config option ("model", "effort").
    pub fn config_label(&self, session: Option<&Session>, id: &str) -> Option<String> {
        let (current, options) = self.config_for(session, id)?;
        options.iter().find(|o| o.value == current).map(|o| o.name.clone())
    }

    /// ⌘U and "+" → "Add files or photos": the native picker, several at once.
    pub fn add_files(&mut self, _: &AddFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.menu = None;
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update_in(cx, |this, window, cx| this.attach_paths(paths, window, cx));
            }
        })
        .detach();
        cx.notify();
    }

    /// Where added files are copied into (or mentioned from): the session's
    /// folder, or the draft's.
    fn attach_folder(&self) -> attach::Folder {
        match self.composer_place() {
            Some(Place { host: HostId::ThisMac, path }) => attach::Folder::Here(path),
            Some(Place { host, .. }) if !self.helper_saves_files(&host) => attach::Folder::Unwritable,
            _ => attach::Folder::Elsewhere,
        }
    }

    /// Whether `host`'s helper can save files into a session's folder, as far
    /// as the app knows: yes until a connected one says otherwise.
    pub(crate) fn helper_saves_files(&self, host: &HostId) -> bool {
        self.connection(host).and_then(|c| c.hello.as_ref()).is_none_or(|h| h.uploads)
    }

    /// Added files become chips or @ mentions (`attach::add_file`, off the
    /// main thread); refusals say why.
    pub fn attach_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let folder = self.attach_folder();
        let unwritable = matches!(folder, attach::Folder::Unwritable);
        let read = cx.background_spawn(async move { paths.iter().map(|p| attach::add_file(p, &folder)).collect::<Vec<_>>() });
        cx.spawn_in(window, async move |this, cx| {
            let results = read.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let mut refused = Vec::new();
                for result in results {
                    match result {
                        Ok(added) => {
                            match added {
                                attach::Added::Chip(attachment) => this.composer.attachments.push(attachment),
                                attach::Added::Mention(path) => this.insert_mention(path, window, cx),
                            }
                            this.file_tip_done();
                        }
                        Err(why) => refused.push(why),
                    }
                }
                if unwritable {
                    refused.insert(0, attach::UNWRITABLE.into());
                }
                this.composer.notice = (!refused.is_empty()).then(|| refused.join(" "));
                cx.notify();
            });
        })
        .detach();
    }

    /// A pasted image (or copied files) becomes a chip; text pastes as usual.
    pub fn paste_into_composer(&mut self, item: &ClipboardItem, window: &mut Window, cx: &mut Context<Self>) -> bool {
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
                    self.attach_paths(paths.paths().to_vec(), window, cx);
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
        if let Attachment::Cells { cells, .. } = attachment {
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
        let busy = session.is_some_and(|s| self.links.get(s.agent).process.up() && s.outbox.busy && s.id.is_some() && !s.agent_waiting && !s.opening());
        let slash = self.slash_view(cx);
        let list_context = if slash.is_some() {
            "SlashList"
        } else if self.composer.typing.is_some() {
            "MentionList"
        } else if self.composer.menu == Some(Menu::Mode) {
            "ModeMenu"
        } else {
            "Composer"
        };
        let chips = self.composer.attachments.iter().enumerate().filter(|(_, a)| !matches!(a, Attachment::Quote(_))).map(|(i, a)| {
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
                        .role(Role::Button)
                        .aria_label("Remove attachment")
                        .ml(px(1.))
                        .px(px(2.))
                        .cursor_pointer()
                        .text_color(theme::text_faint())
                        .hover(|s| s.text_color(theme::text_primary()))
                        .aria_label("Remove attachment")
                        .border_2()
                        .border_color(gpui::transparent_black())
                        .track_focus(&self.composer.chip_remove_focus(i, cx))
                        .tab_stop(true)
                        .focus_ring_on(theme::bg_tag())
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
        let text_box = Textarea::new(&self.input).appearance(false).text_size(theme::chat_body()).line_height(theme::chat_line_body()).aria_label("Message").on_paste(move |item, window, cx| {
            weak.update(cx, |this, cx| this.paste_into_composer(item, window, cx)).unwrap_or(false)
        });
        let token_marks = self.token_marks(cx);
        let typing = self.input.read(cx).focus_handle(cx).is_focused(window);
        let the_box = div()
            .id("composer-box")
            .relative()
            .flex()
            .flex_col()
            .gap(px(6.))
            .min_h(px(38.))
            .pl(px(2.5))
            .pr(px(7.))
            .when(!chips.is_empty(), |d| d.pt(px(7.)))
            .rounded(px(8.))
            .border_1()
            .border_color(if typing { theme::composer_focus_edge() } else { theme::composer_edge() })
            .bg(theme::composer_bg())
            .shadow(theme::composer_shadow())
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.attach_paths(paths.paths().to_vec(), window, cx)))
            .when(!chips.is_empty(), |d| d.child(div().ml(px(10.)).flex().flex_wrap().gap(px(4.)).children(chips)))
            .child(div().flex().items_end().gap_2().child(div().relative().flex_1().min_w_0().child(text_box).child(token_marks)).child(div().mb(px(6.)).child(send)))
            .children(self.render_list(session, slash, cx))
            .children(self.render_hover_preview());
        div()
            .key_context(list_context)
            .on_action(cx.listener(Self::list_up))
            .on_action(cx.listener(Self::list_down))
            .on_action(cx.listener(Self::list_pick))
            .on_action(cx.listener(Self::list_fill))
            .on_action(cx.listener(|this, _: &PickMode1, _, cx| this.pick_mode_key(0, cx)))
            .on_action(cx.listener(|this, _: &PickMode2, _, cx| this.pick_mode_key(1, cx)))
            .on_action(cx.listener(|this, _: &PickMode3, _, cx| this.pick_mode_key(2, cx)))
            .on_action(cx.listener(|this, _: &PickMode4, _, cx| this.pick_mode_key(3, cx)))
            .flex()
            .flex_col()
            .gap_2()
            .children(self.composer.notice.clone().map(|n| div().text_size(theme::chat_meta()).text_color(theme::accent_text()).child(n)))
            .children(self.render_quote_cards(cx))
            .child(the_box)
            .child(self.render_toolbar(session, notebook_open, window, cx))
            .into_any_element()
    }

    /// A tint behind each @ mention in the text, and behind a slash command
    /// at its start, so each reads as one piece; after a command alone, the
    /// words it takes, faint.
    fn token_marks(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let input = self.input.clone();
        let mentions = self.composer.mentions.clone();
        let commands = self.slash_commands();
        let _ = cx;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let state = input.read(cx);
                let text = state.value().to_string();
                let command = slash::token(&text, &commands);
                let hint = command.and_then(|(len, c)| c.words.clone().filter(|_| text.len() == len + 1)).zip(state.range_to_bounds(&(text.len()..text.len())));
                let marks: Vec<_> = command.map(|(len, _)| 0..len).into_iter().chain(attach::token_ranges(&text, &mentions)).filter_map(|range| state.range_to_bounds(&range)).collect();
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    for b in marks {
                        let b = Bounds::from_corners(point(b.left() - px(2.), b.top()), point(b.right() + px(2.), b.bottom()));
                        window.paint_quad(fill(b, theme::accent().opacity(0.28)).corner_radii(px(3.)));
                    }
                    if let Some((hint, caret)) = hint {
                        let run = TextRun { len: hint.len(), font: font(theme::SANS), color: theme::text_faint().into(), background_color: None, underline: None, strikethrough: None };
                        let line = window.text_system().shape_line(hint.into(), theme::chat_body(), &[run], None);
                        let _ = line.paint(point(caret.left() + px(3.), caret.top()), caret.size.height, TextAlign::Left, None, window, cx);
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }

    fn send_button(&self, empty: bool, busy: bool, in_chat: bool, cx: &mut Context<Self>) -> AnyElement {
        let base = div().id("send").flex_shrink_0().size(px(24.)).flex().items_center().justify_center().rounded_full();
        let focus = self.composer.focus_send.clone();
        if empty && busy {
            return base
                .role(Role::Button)
                .aria_label("Stop (Esc)")
                .cursor_pointer()
                .bg(theme::bg_raised())
                .hover(|s| s.bg(theme::composer_edge()))
                .border_2()
                .border_color(gpui::transparent_black())
                .track_focus(&focus)
                .tab_stop(true)
                .focus_ring_on(theme::composer_bg())
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
            .border_2()
            .border_color(gpui::transparent_black())
            .track_focus(&focus)
            .tab_stop(true)
            .focus_ring_on(theme::composer_bg())
            .child(glyph(Glyph::ArrowUp, gpui::white().into()))
            .on_click(cx.listener(move |this, _, window, cx| {
                if in_chat {
                    this.submit(false, window, cx);
                } else {
                    this.start_session(window, cx);
                }
            }))
            .into_any_element()
    }

    /// The slash list above the box, as wide as it.
    fn render_slash(&self, view: SlashView, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.composer.selected;
        let heading = |text: String| div().px(px(8.)).pt(px(6.)).pb(px(2.)).text_size(theme::chat_meta_small()).text_color(theme::text_faint()).child(text);
        let line = |text: String| div().px(px(8.)).py(px(5.)).text_size(theme::chat_meta()).text_color(theme::text_faint()).child(text);
        let select = |i: usize| {
            cx.listener(move |this: &mut Self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>| {
                if *hovered && this.composer.selected != i {
                    this.composer.selected = i;
                    cx.notify();
                }
            })
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut after: Vec<Div> = Vec::new();
        match view {
            SlashView::Commands { commands, listing, query, waiting } => {
                let (found, described) = match listing {
                    Listing::Names(found) => (found, false),
                    Listing::Descriptions(found) => (found, true),
                    Listing::Nothing => (Vec::new(), false),
                };
                if found.is_empty() {
                    after.push(line(format!("No command named /{query}. {} sends “/{query}” to Claude as a message.", slash::ENTER)));
                }
                // The name column: just wide enough for the longest visible
                // name and its faint hint, capped so one long one doesn't
                // crowd every description out.
                let mono_width = |text: &str, size: Pixels| text.chars().count() as f32 * size.as_f32() * 0.625;
                let name_col = found
                    .iter()
                    .map(|row| {
                        let command = &commands[row.command];
                        let name = mono_width(&command.name, theme::chat_code()) + mono_width("/", theme::chat_code());
                        name + command.hint.as_deref().map_or(0., |h| 6. + mono_width(h, theme::chat_meta_small()))
                    })
                    .fold(0.0_f32, f32::max)
                    .min(self.settings.layout.chat_width * 0.4);
                let mark = HighlightStyle { color: Some(theme::text_primary().into()), ..Default::default() };
                let mut group = None;
                for (i, row) in found.into_iter().enumerate() {
                    let command = &commands[row.command];
                    let head = match described {
                        true => (i == 0).then(|| heading(format!("No command is named “{query}”. Described as:"))),
                        false => (group != Some(command.group)).then(|| heading(command.group.heading().into())),
                    };
                    group = Some(command.group);
                    let name = format!("/{}", command.name);
                    let name_marks: Vec<_> = if described { Vec::new() } else { row.marks.iter().map(|r| (r.start + 1..r.end + 1, mark)).collect() };
                    let description = command.description.lines().next().unwrap_or_default();
                    let (description, description_marks): (String, Vec<_>) = match row.marks.first().filter(|r| described && r.end <= description.len()) {
                        Some(r) => {
                            let (from, shown) = slash::around(description, r.start);
                            (shown, vec![(r.start - from..r.end - from, mark)])
                        }
                        None => (description.to_string(), Vec::new()),
                    };
                    let key = (command.own == Some(Own::Mode)).then_some(slash::SHIFT_TAB);
                    let item = popup_row(ElementId::NamedInteger("slash".into(), i as u64), i == selected)
                        .gap(px(12.))
                        .child(
                            div()
                                .w(px(name_col))
                                .flex_shrink_0()
                                .flex()
                                .items_baseline()
                                .gap(px(6.))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .font_family(theme::MONO)
                                        .text_size(theme::chat_code())
                                        .text_color(if query.is_empty() || described { theme::text_primary() } else { theme::text_secondary() })
                                        .child(StyledText::new(name).with_highlights(name_marks)),
                                )
                                .children(command.hint.clone().map(|h| div().min_w_0().truncate().font_family(theme::MONO).text_size(theme::chat_meta_small()).text_color(theme::text_faint()).child(h))),
                        )
                        .child(div().flex_1().min_w_0().truncate().text_size(theme::chat_meta()).text_color(theme::text_muted()).child(StyledText::new(description).with_highlights(description_marks)))
                        .children(key.map(|k| div().flex_shrink_0().text_size(theme::chat_meta()).text_color(theme::text_faint()).child(k)))
                        .on_hover(select(i))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(view) = this.slash_view(cx) {
                                this.slash_choose(view, i, false, window, cx);
                            }
                        }));
                    rows.push(div().flex().flex_col().children(head).child(item).into_any_element());
                }
                if waiting {
                    after.push(line("Claude's commands appear once it's connected".into()));
                }
            }
            SlashView::Choices { own, choices } => {
                let title = if own == Own::Mode { "Mode" } else { "Model" };
                if choices.is_empty() {
                    after.push(line(format!("No {} matches", title.to_lowercase())));
                }
                for (i, choice) in choices.into_iter().enumerate() {
                    let item = popup_row(ElementId::NamedInteger("slash-choice".into(), i as u64), i == selected)
                        .gap(px(12.))
                        .child(div().w(px(160.)).flex_shrink_0().truncate().child(choice.name))
                        .child(div().flex_1().min_w_0().truncate().text_size(theme::chat_meta()).text_color(theme::text_muted()).children(choice.description))
                        .child(div().w(px(12.)).flex_shrink_0().text_color(theme::accent_text()).child(if choice.current { "✓" } else { "" }))
                        .on_hover(select(i))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(view) = this.slash_view(cx) {
                                this.slash_choose(view, i, false, window, cx);
                            }
                        }));
                    rows.push(div().flex().flex_col().children((i == 0).then(|| heading(title.into()))).child(item).into_any_element());
                }
            }
        }
        let footer = div().mt(px(4.)).pt(px(5.)).px(px(8.)).pb(px(1.)).border_t_1().border_color(theme::border()).text_size(theme::chat_meta_small()).text_color(theme::text_faint()).child(slash::FOOTER);
        let list = popup()
            .id("slash-list")
            .child(div().id("slash-rows").flex().flex_col().max_h(px(SLASH_LIST_MAX)).overflow_y_scroll().track_scroll(&self.composer.slash_scroll).children(rows))
            .children(after)
            .child(footer);
        div().absolute().bottom(relative(1.)).mb(px(6.)).occlude().left_0().right_0().child(list).into_any_element()
    }

    /// The open list above the box: the slash list, the @ list, or a menu
    /// from the row under it.
    fn render_list(&self, session: Option<&Session>, slash: Option<SlashView>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let above = |d: Div| d.absolute().bottom(relative(1.)).mb(px(6.)).occlude();
        if let Some(view) = slash {
            return Some(self.render_slash(view, cx));
        }
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
                    .child(div().min_w_0().truncate().font_family(theme::MONO).text_size(theme::chat_code()).child(path))
                    .on_click(cx.listener(move |this, _, window, cx| this.pick_mention(pick.clone(), window, cx)))
            });
            let list = popup()
                .id("mention-list")
                .left_0()
                .right_0()
                .child(div().px(px(8.)).pt(px(2.)).pb(px(4.)).text_size(theme::chat_meta_small()).text_color(theme::text_faint()).child("Files in this session's folder"))
                .children(rows)
                .children(status.map(|s| div().px(px(8.)).py(px(4.)).text_size(theme::chat_meta()).text_color(theme::text_muted()).child(s)));
            return Some(above(div().left_0().right_0()).child(list).into_any_element());
        }
        let menu = self.composer.menu?;
        let body = match menu {
            Menu::Plus => {
                popup()
                    .w(px(240.))
                    .child(
                        popup_row("plus-files".into(), false)
                            .child(glyph(Glyph::File, theme::text_muted()))
                            .child(div().flex_1().child("Add files or photos"))
                            .child(div().text_size(theme::chat_meta()).text_color(theme::text_faint()).child(crate::platform::shortcut!("U")))
                            .on_click(cx.listener(|this, _, window, cx| this.add_files(&AddFiles, window, cx))),
                    )
                    .child(
                        popup_row("plus-commands".into(), false)
                            .child(glyph(Glyph::Slash, theme::text_muted()))
                            .child(div().flex_1().child("Slash commands"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.composer.menu = None;
                                this.composer.slash_closed = None;
                                this.set_composer_text("/".into(), window, cx);
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            }
            Menu::Mode => {
                let (choices, current) = self.mode_list();
                popup()
                    .w(px(300.))
                    .child(div().px(px(8.)).pt(px(2.)).pb(px(2.)).text_size(theme::chat_meta()).text_color(theme::text_faint()).child("Mode"))
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
                                    .child(div().text_size(theme::chat_meta()).text_color(theme::text_muted()).child(choice.description)),
                            )
                            .child(div().w(px(12.)).text_color(theme::accent_text()).child(if current == Some(i) { "✓" } else { "" }))
                            .children((i < 4).then(|| div().w(px(10.)).text_size(theme::chat_meta()).text_color(theme::text_faint()).child(format!("{}", i + 1))))
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
                                    .children(option.description.clone().map(|d| div().text_size(theme::chat_meta()).text_color(theme::text_muted()).child(d))),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.pick_config(id, value.clone(), cx)))
                    }))
                    .into_any_element()
            }
        };
        // Deferred so the menu sits above the click-outside backdrop, which would
        // otherwise take its clicks and close it.
        let (place, anchor) = match menu {
            Menu::Config(_) => (div().right_0(), Anchor::BottomRight),
            _ => (div().left(px(-11.)), Anchor::BottomLeft),
        };
        Some(place.absolute().top(px(-6.)).child(deferred(anchored().anchor(anchor).child(crate::motion::arriving(div().occlude().child(body), "composer-menu-in", false))).with_priority(1)).into_any_element())
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
                        .child(div().flex().items_center().gap(px(6.)).text_size(theme::chat_meta()).text_color(theme::text_muted()).child(glyph(icon_glyph(attachment.icon()), theme::text_muted())).child(preview_heading(attachment)))
                        .child(preview_body(attachment)),
                )
                .into_any_element(),
        )
    }

    fn render_toolbar(&self, session: Option<&Session>, notebook_open: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let narrow = self.settings.layout.chat_width < 400.;
        let mode = self.mode_label(session);
        let open = |menu: Menu| self.composer.menu == Some(menu);
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(24.))
            // Lines the + up with the text in the box, and the ring with the send button.
            .pl(px(4.5))
            .pr(px(8.))
            .text_size(theme::chat_meta())
            .text_color(theme::text_new())
            .child(
                tool_button("plus")
                    .role(Role::Button)
                    .aria_label("Add")
                    .when(open(Menu::Plus), |d| d.bg(theme::row_active()))
                    .track_focus(&self.composer.focus_plus)
                    .tab_stop(true)
                    .focus_ring()
                    .child(glyph(Glyph::Plus, theme::text_secondary()))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_menu(Menu::Plus, window, cx))),
            )
            .child(if notebook_open {
                tool_button("point")
                    .role(Role::Button)
                    .aria_label("Point")
                    .gap(px(4.))
                    .when(self.annotating, |d| d.text_color(theme::accent_text()))
                    .track_focus(&self.composer.focus_point)
                    .tab_stop(true)
                    .focus_ring()
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
                    .role(Role::Button)
                    .aria_label(format!("Mode: {name}"))
                    .when(open(Menu::Mode), |d| d.bg(theme::row_active()))
                    .when(name == "Plan", |d| d.text_color(theme::accent_text()))
                    .track_focus(&self.composer.focus_mode)
                    .tab_stop(true)
                    .focus_ring()
                    .child(name)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_menu(Menu::Mode, window, cx)))
            }))
            .child(div().flex_1())
            .children(
                self.composer_agent(session)
                    .facts()
                    .config
                    .iter()
                    .zip(&self.composer.focus_config)
                    .map(|(&(id, name), focus)| {
                        let label = self.config_label(session, id)?;
                        let short = label.strip_suffix(" (recommended)").unwrap_or(&label).to_owned();
                        Some(
                            tool_button(id)
                                .role(Role::Button)
                                .aria_label(format!("{name}: {label}"))
                                .min_w_0()
                                .text_color(theme::text_secondary())
                                .when(open(Menu::Config(id)), |d| d.bg(theme::row_active()))
                                .track_focus(focus)
                                .tab_stop(true)
                                .focus_ring()
                                .child(div().min_w_0().truncate().child(short))
                                .on_click(cx.listener(move |this, _, window, cx| this.toggle_menu(Menu::Config(id), window, cx))),
                        )
                    })
                    .flatten(),
            )
            .child(self.render_context_ring(session, window, cx))
    }

    /// Chips above a sent message (its quotes are in the bubble), and the
    /// popover of the one clicked.
    pub fn render_sent_chips(&self, key: u64, entry: usize, attachments: &[Attachment], cx: &mut Context<Self>) -> Option<AnyElement> {
        let popover = self.chip_popover.as_ref().filter(|p| p.key == key && p.entry == entry);
        let chips: Vec<_> = attachments
            .iter()
            .enumerate()
            .filter(|(_, a)| !matches!(a, Attachment::Quote(_)))
            .map(|(i, a)| {
                let open = popover.is_some_and(|p| p.chip == i);
                chip(ElementId::NamedInteger("sent-chip".into(), (key << 32) | ((entry as u64) << 8) | i as u64), a)
                    .cursor_pointer()
                    .when(open, |d| d.bg(theme::bg_raised()))
                    .hover(|s| s.bg(theme::bg_raised()))
                    .on_click(cx.listener(move |this, _, _, cx| this.click_sent_chip(key, entry, i, cx)))
            })
            .collect();
        if chips.is_empty() {
            return None;
        }
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
                    .text_size(theme::chat_meta())
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
                        .text_size(theme::chat_meta())
                        .child(div().flex_1().text_color(theme::text_muted()).child(if changed { "The cell has changed since." } else { "" }))
                        .child(
                            div()
                                .id("show-in-notebook")
                                .px(px(8.))
                                .py(px(3.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(theme::control_edge())
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
            });
        // Closed by the shared click-outside backdrop (main.rs); occlude()
        // keeps a click on the popover itself from reaching it.
        div().absolute().top(relative(1.)).right_0().mt(px(4.)).child(deferred(anchored().anchor(Anchor::TopRight).child(body)).with_priority(2)).into_any_element()
    }
}

/// Keep the composer's list keys and mentions in step with the text box.
pub fn subscribe(input: &Entity<gpui_component::input::TextareaState>, window: &mut Window, cx: &mut Context<Workspace>) {
    cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| this.on_composer_input(event, window, cx)).detach();
}
