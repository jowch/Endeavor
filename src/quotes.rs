//! Quotes (attach::Quote) on screen: Reply on text selected in one of Claude's
//! replies (a pill by the selection, then a small prompt), the cards waiting
//! above the composer, and the quotes in a sent message.

use std::sync::Arc;
use std::time::UNIX_EPOCH;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{InputEvent, Textarea, TextareaState};

use crate::attach::{Attachment, Quote, Quoted};
use crate::new_session::{Glyph, glyph};
use crate::session::Entry;
use crate::{ReplyToSelection, Workspace, theme};

/// Text selected in one of Claude's replies, and where the pointer let go of
/// it, from the reply's top left, so the pill and prompt open under that spot
/// and move with the reply as the transcript scrolls.
#[derive(Clone)]
pub struct Selected {
    key: u64,
    entry: usize,
    text: String,
    at: Point<Pixels>,
}

impl Selected {
    fn draft_key(&self) -> (u64, usize, String) {
        (self.key, self.entry, self.text.clone())
    }
}

/// The chat's Reply: the pill offered for a selection, or the prompt it opened.
pub enum Reply {
    Pill(Selected),
    Prompt { selected: Selected, input: Entity<TextareaState>, menu: bool },
}

/// Words typed in Reply's prompt and left there, by the selection they reply
/// to (session, reply, text): reopening it on that selection brings them back.
pub type Drafts = std::collections::HashMap<(u64, usize, String), String>;

/// The prompt's keys as this platform writes them.
const SEND_KEY: &str = if cfg!(target_os = "macos") { "↩" } else { "Enter" };
const ADD_KEY: &str = if cfg!(target_os = "macos") { "⌘↩" } else { "Ctrl+Enter" };

/// How far under the pointer the pill and the prompt open: past the rest of
/// the selection's last line.
const BELOW_POINTER: f32 = 16.;
const PROMPT_WIDTH: f32 = 380.;

/// A quote's excerpt on one line, as the prompt shows it.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A row of the prompt's menu: what it does, and its key.
fn menu_row(id: &'static str, label: &'static str, key: &'static str, icon: Glyph) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::MenuItem)
        .h(px(28.))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::menu_hover()))
        .child(glyph(icon, theme::icon_grey()))
        .child(div().text_color(theme::text_primary()).child(label))
        .child(div().ml_auto().pl(px(12.)).text_size(theme::chat_meta_small()).text_color(theme::text_faint()).child(key))
}

impl Workspace {
    /// The pointer let go in the transcript: offer Reply if it left text
    /// selected in a reply, else take the offer away.
    pub fn check_reply_selection(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        if matches!(self.reply, Some(Reply::Prompt { .. })) {
            return;
        }
        let Some(session) = self.active_session() else { return };
        let key = session.key;
        let found = session.selected_reply(cx).filter(|(entry, _)| matches!(session.entries.get(*entry), Some(Entry::Agent { .. })));
        let pill = found.map(|(entry, text)| {
            let origin = session.reply_bounds(entry, cx).map_or(Point::default(), |b| b.origin);
            Reply::Pill(Selected { key, entry, text, at: at - origin })
        });
        if pill.is_some() || matches!(self.reply, Some(Reply::Pill(_))) {
            self.reply = pill;
            cx.notify();
        }
    }

    /// ⌘E (or ⌘J), or a click on the pill: open the prompt for the offered
    /// selection, with its draft selected if it has one. ⌘E again closes it.
    pub fn reply_to_selection(&mut self, _: &ReplyToSelection, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.reply, Some(Reply::Prompt { .. })) {
            self.close_reply(cx);
            return;
        }
        if !matches!(self.reply, Some(Reply::Pill(_))) {
            self.check_reply_selection(window.mouse_position(), cx);
        }
        let Some(Reply::Pill(selected)) = self.reply.take() else { return };
        // ↩ sends, ⌘↩ adds to the message, ⇧↩ is a new line; six lines, then it scrolls.
        let input = cx.new(|cx| TextareaState::new(window, cx).placeholder("Reply to Claude").submit_on_enter(true).auto_grow(1, 6));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            match event {
                InputEvent::PressEnter { secondary, shift: false } => this.finish_reply(*secondary, window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }
        })
        .detach();
        input.update(cx, |s, cx| s.focus(window, cx));
        if let Some(draft) = self.reply_drafts.get(&selected.draft_key()).cloned() {
            let len = draft.len();
            input.update(cx, |s, cx| {
                s.set_value(draft, window, cx);
                s.set_selected_range(0..len, cx);
            });
        }
        self.reply = Some(Reply::Prompt { selected, input, menu: false });
        cx.notify();
    }

    /// ⏎ sends the quote and the reply now; ⌘⏎ (`add`) adds them to the
    /// composer's message instead.
    pub fn finish_reply(&mut self, add: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Reply::Prompt { selected, input, .. }) = self.reply.take() else { return };
        let comment = input.read(cx).value().trim().to_string();
        self.reply_drafts.remove(&selected.draft_key());
        let at = self.sessions.iter().find(|s| s.key == selected.key).and_then(|s| match s.entries.get(selected.entry) {
            Some(Entry::Agent { at: Some(at), .. }) => Some(crate::when::clock(at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())),
            _ => None,
        });
        let quote = Quote { from: Quoted::Reply { text: selected.text.clone(), at }, comment };
        self.use_quotes(vec![quote], add, cx);
        gpui_base::TextSelection::clear(window, cx);
        self.input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Esc, or a click elsewhere: close the prompt, keeping its words as the
    /// selection's draft (or take the pill away).
    pub fn close_reply(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(Reply::Prompt { selected, input, .. }) = &self.reply {
            let words = input.read(cx).value().to_string();
            if words.trim().is_empty() {
                self.reply_drafts.remove(&selected.draft_key());
            } else {
                self.reply_drafts.insert(selected.draft_key(), words);
            }
        }
        let open = self.reply.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    /// For the state dump: which of Reply's pieces shows (`pill`, `prompt`,
    /// `added`), the quoted text, what's typed, and whether the menu is open.
    #[cfg(debug_assertions)]
    pub fn reply_state(&self, cx: &App) -> serde_json::Value {
        match &self.reply {
            None => serde_json::Value::Null,
            Some(Reply::Pill(s)) => serde_json::json!({ "shows": "pill", "quote": s.text }),
            Some(Reply::Prompt { selected, input, menu }) => {
                serde_json::json!({ "shows": "prompt", "quote": selected.text, "text": input.read(cx).value().to_string(), "menu": menu })
            }
        }
    }

    /// The pill, the prompt or the confirmation, over the chat where the
    /// selection was.
    pub fn render_reply(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let reply = self.reply.as_ref()?;
        let (Reply::Pill(selected) | Reply::Prompt { selected, .. }) = reply;
        // The notebook's web view covers anything drawn over its pane, so
        // these stay inside the reply's column.
        let column = self.sessions.iter().find(|s| s.key == selected.key).and_then(|s| s.reply_bounds(selected.entry, cx));
        let prompt_width = column.map_or(px(PROMPT_WIDTH), |c| c.size.width.min(px(PROMPT_WIDTH)));
        let place = |width: Pixels, element: AnyElement| {
            let at = selected.at + column.map_or(Point::default(), |c| c.origin);
            let mut x = at.x - px(28.);
            if let Some(c) = column {
                x = x.min(c.right() - width).max(c.left());
            }
            deferred(anchored().position(point(x, at.y + px(BELOW_POINTER))).snap_to_window_with_margin(px(8.)).child(element)).with_priority(2).into_any_element()
        };
        let frame = || div().occlude().map(theme::popover).rounded(px(8.)).font_family(theme::SANS).text_size(theme::chat_meta());
        Some(match reply {
            Reply::Pill(_) => place(
                px(110.),
                frame()
                    .id("reply-pill")
                    .role(Role::Button)
                    .aria_label("Reply")
                    .h(px(30.))
                    .p(px(3.))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::menu_hover()))
                            .text_color(theme::text_primary())
                            .child(glyph(Glyph::Bubble, theme::icon_grey()))
                            .child("Reply")
                            .child(div().text_size(px(11.)).text_color(theme::text_faint()).child(crate::platform::shortcut!("E"))),
                    )
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        if matches!(this.reply, Some(Reply::Pill(_))) {
                            this.close_reply(cx);
                        }
                    }))
                    .on_click(cx.listener(|this, _, window, cx| this.reply_to_selection(&ReplyToSelection, window, cx)))
                    .into_any_element(),
            ),
            Reply::Prompt { input, menu, .. } => {
                let focused = input.read(cx).focus_handle(cx).is_focused(window);
                let empty = input.read(cx).value().trim().is_empty();
                let working = self.active_session().is_some_and(|s| s.outbox.busy && !s.agent_waiting);
                // Inside the prompt's frame, so a click on it isn't a click outside.
                let menu = menu.then(|| {
                    div()
                        .self_end()
                        .w(px(if cfg!(target_os = "macos") { 216. } else { 236. }))
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .map(theme::popover)
                        .rounded(px(10.))
                        .child(
                            menu_row("reply-send", if working { "Send after this turn" } else { "Send now" }, SEND_KEY, Glyph::ArrowUp)
                                .on_click(cx.listener(|this, _, window, cx| this.finish_reply(false, window, cx))),
                        )
                        .child(menu_row("reply-add", "Add to message", ADD_KEY, Glyph::Lines).on_click(cx.listener(|this, _, window, cx| this.finish_reply(true, window, cx))))
                });
                let send = div()
                    .id("reply-send-button")
                    .role(Role::Button)
                    .aria_label("Send")
                    .flex_shrink_0()
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .bg(if empty { theme::bg_raised() } else { theme::accent() })
                    .child(glyph(Glyph::ArrowUp, if empty { theme::text_faint() } else { gpui::white().into() }))
                    .on_click(cx.listener(|this, _, window, cx| this.finish_reply(false, window, cx)));
                let options = div()
                    .id("reply-options")
                    .role(Role::Button)
                    .aria_label("Send options")
                    .flex_shrink_0()
                    .w(px(18.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .when(menu.is_some(), |d| d.bg(theme::control_edge()))
                    .hover(|s| s.bg(theme::control_edge()))
                    .child(glyph(Glyph::Chevron, theme::text_muted()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(Reply::Prompt { menu, .. }) = &mut this.reply {
                            *menu = !*menu;
                            cx.notify();
                        }
                    }));
                let head = div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(4.))
                    .pr(px(2.))
                    .text_size(theme::chat_meta_small())
                    .line_height(px(17.))
                    .text_color(theme::text_faint())
                    .whitespace_nowrap()
                    .child(div().flex_1().min_w_0().flex().items_center().gap(px(6.)).overflow_hidden().child(glyph(Glyph::Lines, theme::text_faint())).child("Claude's reply"))
                    .child(div().flex_shrink_0().text_size(px(11.)).child(format!("{SEND_KEY} {} · {ADD_KEY} add to message", if working { "queue" } else { "send" })));
                place(
                    prompt_width,
                    frame()
                        .id("reply-prompt")
                        .role(Role::Dialog)
                        .aria_label("Reply")
                        .w(prompt_width)
                        .pt(px(7.))
                        .px(px(8.))
                        .pb(px(8.))
                        .rounded(px(10.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(head)
                        .child(
                            div()
                                .mx(px(4.))
                                .border_l_2()
                                .border_color(theme::control_edge())
                                .pl(px(8.))
                                .text_size(px(11.5))
                                .line_height(px(17.))
                                .text_color(theme::text_faint())
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(one_line(&selected.text)),
                        )
                        .child(
                            div()
                                .min_h(px(34.))
                                .flex()
                                .items_end()
                                .gap(px(4.))
                                .pr(px(4.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(if focused { theme::focus_ring() } else { theme::control_edge() })
                                .bg(theme::bg_page())
                                .child(
                                    // The Textarea's multi-line editor pads 8 px above and
                                    // below and 10 px at the sides, and takes no size: no
                                    // left padding here, and pulling it in by 2.5 px makes
                                    // one line 34 px, as designed.
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .my(px(-2.5))
                                        .child(Textarea::new(input).appearance(false).aria_label("Reply to Claude").text_size(theme::chat_body()).line_height(px(21.))),
                                )
                                .child(send)
                                .child(options),
                        )
                        .when(working, |d| d.child(div().px(px(4.)).text_size(theme::chat_meta_small()).text_color(theme::text_faint()).child("Claude is working. This goes after its turn.")))
                        .children(menu)
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.close_reply(cx);
                        }))
                        .into_any_element(),
                )
            }
        })
    }
}

/// From this many quotes on, the cards above the composer take one line each.
const COMPACT_FROM: usize = 4;

fn quote_glyph(quote: &Quote) -> Glyph {
    match &quote.from {
        Quoted::Reply { .. } => Glyph::Lines,
        Quoted::Cell { .. } if quote.picture().is_some() => Glyph::Picture,
        Quoted::Cell { .. } => Glyph::Code,
        Quoted::Box { .. } => Glyph::Region,
    }
}

/// A quote's text with a rule on its left: code as numbered lines (at most
/// `max_lines`, each cut to fit), other text as `max_lines` wrapped lines.
fn excerpt(quote: &Quote, max_lines: usize) -> Option<Div> {
    let text = quote.excerpt()?;
    let ruled = div().border_l_2().border_color(theme::composer_edge()).pl(px(8.)).min_w_0().overflow_hidden().text_color(theme::text_secondary());
    let Some(first) = quote.first_line() else {
        return Some(ruled.text_size(px(13.)).line_height(px(19.)).line_clamp(max_lines).text_ellipsis().child(text.trim().to_string()));
    };
    let mut lines: Vec<(String, String)> = text.lines().enumerate().take(max_lines).map(|(i, l)| ((first + i).to_string(), l.replace('\t', "    "))).collect();
    if text.lines().count() > max_lines {
        lines.push((String::new(), "…".into()));
    }
    let width = lines.iter().map(|(n, _)| n.len()).max().unwrap_or(1);
    Some(ruled.font_family(theme::MONO).text_size(theme::chat_meta_small()).line_height(px(18.)).children(lines.into_iter().map(|(n, line)| {
        div()
            .flex()
            .gap(px(8.))
            .child(div().flex_shrink_0().w(px(7.5 * width as f32)).text_color(theme::text_faint()).child(n))
            .child(div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(line))
    })))
}

fn picture(png: &Arc<Vec<u8>>, w: f32, h: f32) -> Div {
    div()
        .flex_shrink_0()
        .w(px(w))
        .h(px(h))
        .rounded(px(5.))
        .overflow_hidden()
        .bg(theme::bg_page())
        .border_1()
        .border_color(theme::border())
        .child(img(Arc::new(Image::from_bytes(ImageFormat::Png, png.to_vec()))).size_full().object_fit(ObjectFit::Contain))
}

fn icon_button(id: ElementId, label: &'static str, icon: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .text_color(theme::text_muted())
        .hover(|s| s.bg(theme::bg_raised()).text_color(theme::text_primary()))
        .child(icon)
}

impl Workspace {
    /// The quotes waiting to go with the next message, above the composer: a
    /// heading with Clear, then a card each in the order added (its source,
    /// the excerpt or picture, the comment), one line each from four on.
    pub fn render_quote_cards(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let quotes: Vec<(usize, &Quote)> = self
            .composer
            .attachments
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match a {
                Attachment::Quote(q) => Some((i, q)),
                _ => None,
            })
            .collect();
        if quotes.is_empty() {
            return None;
        }
        let compact = quotes.len() >= COMPACT_FROM;
        let count = match quotes.len() {
            1 => "1 quote goes out with this message".to_string(),
            n => format!("{n} quotes go out with this message"),
        };
        let heading = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(theme::chat_meta())
            .text_color(theme::text_muted())
            .child(glyph(Glyph::Lines, theme::text_muted()))
            .child(div().flex_1().child(count))
            .child(
                div()
                    .id("clear-quotes")
                    .role(Role::Button)
                    .cursor_pointer()
                    .text_size(theme::chat_meta_small())
                    .hover(|s| s.text_color(theme::text_primary()))
                    .child("Clear")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.composer.attachments.retain(|a| !matches!(a, Attachment::Quote(_)));
                        cx.notify();
                    })),
            );
        let cards = quotes.into_iter().map(|(i, quote)| {
            let remove = icon_button(ElementId::NamedInteger("remove-quote".into(), i as u64), "Remove quote", div().text_size(px(15.)).child("×")).on_click(cx.listener(
                move |this, _, _, cx| {
                    if matches!(this.composer.attachments.get(i), Some(Attachment::Quote(_))) {
                        this.composer.attachments.remove(i);
                    }
                    cx.notify();
                },
            ));
            let cells = quote.cells();
            let show = (!cells.is_empty()).then(|| {
                icon_button(ElementId::NamedInteger("show-quote".into(), i as u64), "Show in notebook", glyph(Glyph::External, theme::text_muted()))
                    .on_click(cx.listener(move |this, _, _, cx| this.reveal_cells(cells.clone(), cx)))
            });
            let card = div().rounded(px(8.)).border_1().border_color(theme::border()).bg(theme::bg_sunken());
            let source = div().flex_shrink_0().text_size(theme::chat_meta_small()).text_color(theme::text_faint()).child(quote.source());
            if compact {
                let comment = match quote.comment.as_str() {
                    "" => div().text_color(theme::text_muted()).child("No comment"),
                    c => div().text_color(theme::text_primary()).child(c.lines().next().unwrap_or_default().to_string()),
                };
                return card
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(10.))
                    .pr(px(6.))
                    .child(glyph(quote_glyph(quote), theme::text_faint()))
                    .child(source)
                    .child(comment.flex_1().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().text_size(theme::chat_meta()))
                    .child(remove)
                    .into_any_element();
            }
            let header = div().flex().items_center().gap(px(6.)).child(glyph(quote_glyph(quote), theme::text_faint())).child(source).child(div().flex_1()).children(show).child(remove);
            card.flex()
                .gap(px(10.))
                .pl(px(10.))
                .pr(px(6.))
                .pt(px(8.))
                .pb(px(9.))
                .children(quote.picture().map(|png| picture(png, 72., 46.)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .child(header)
                        .children(excerpt(quote, if quote.first_line().is_some() { 3 } else { 1 }))
                        .when(!quote.comment.is_empty(), |d| {
                            d.child(div().text_size(theme::chat_body()).line_height(theme::chat_line_body()).text_color(theme::text_primary()).child(quote.comment.clone()))
                        }),
                )
                .into_any_element()
        });
        Some(div().flex().flex_col().gap(px(6.)).child(heading).children(cards).into_any_element())
    }

    /// A sent message's quotes, in its bubble before its words: a reply's as
    /// a blockquote, the notebook's as a chip that shows the cell, then the
    /// lines or the picture; each followed by its comment.
    pub fn render_sent_quotes(&self, key: u64, entry: usize, attachments: &[Attachment], cx: &mut Context<Self>) -> Vec<AnyElement> {
        let quotes = attachments.iter().enumerate().filter_map(|(i, a)| match a {
            Attachment::Quote(q) => Some((i, q)),
            _ => None,
        });
        let mut parts = Vec::new();
        for (n, (i, quote)) in quotes.enumerate() {
            if n > 0 {
                parts.push(div().h(px(1.)).my(px(2.)).bg(theme::border()).into_any_element());
            }
            let cells = quote.cells();
            if !matches!(quote.from, Quoted::Reply { .. }) {
                let label = quote.source();
                parts.push(
                    div()
                        .flex()
                        .child(
                            div()
                                .id(ElementId::NamedInteger("sent-quote".into(), (key << 32) | ((entry as u64) << 8) | i as u64))
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .px(px(6.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(theme::composer_edge())
                                .bg(theme::bg_raised())
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::composer_edge()))
                                .text_size(theme::chat_meta_small())
                                .text_color(theme::text_secondary())
                                .child(glyph(quote_glyph(quote), theme::text_muted()))
                                .child(div().font_family(theme::MONO).text_size(px(11.5)).child(label))
                                .on_click(cx.listener(move |this, _, _, cx| this.reveal_cells(cells.clone(), cx))),
                        )
                        .into_any_element(),
                );
            }
            if let Some(png) = quote.picture() {
                parts.push(div().flex().border_l_2().border_color(theme::composer_edge()).pl(px(10.)).child(picture(png, 132., 80.)).into_any_element());
            }
            if let Some(excerpt) = excerpt(quote, 6) {
                parts.push(excerpt.into_any_element());
            }
            if !quote.comment.is_empty() {
                parts.push(div().child(quote.comment.clone()).into_any_element());
            }
        }
        parts
    }
}
