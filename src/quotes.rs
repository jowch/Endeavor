//! Quotes (attach::Quote) on screen: the cards waiting above the composer, and
//! the quotes in a sent message.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::attach::{Attachment, Quote, Quoted};
use crate::new_session::{Glyph, glyph};
use crate::{Workspace, theme};

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
        let line_height = px(19.);
        return Some(ruled.text_size(px(13.)).line_height(line_height).max_h(line_height * max_lines as f32).child(text.trim().to_string()));
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
