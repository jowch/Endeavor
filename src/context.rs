//! The composer's context ring and its popover: how much of the model's
//! context the conversation uses, in words by level, and from 80% a way to
//! summarize now. Hovering the ring (after `HOVER_DELAY`) or reaching it with
//! Tab opens the popover; a click or Space pins it; Esc closes it.

use std::time::{Duration, UNIX_EPOCH};

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::agent::Agent;
use crate::session::Session;
use crate::theme::FocusRing as _;
use crate::{Workspace, context_ring, theme};

pub const HOVER_DELAY: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Under 50%.
    Plenty,
    /// 50% to under 80%.
    Half,
    /// 80% and over: the ring turns orange and the popover offers /compact.
    Full,
}

/// Percent used, rounded down.
pub fn percent(used: u64, size: u64) -> u64 {
    if size == 0 { 0 } else { used.saturating_mul(100) / size }
}

pub fn level(used: u64, size: u64) -> Level {
    match percent(used, size) {
        0..50 => Level::Plenty,
        50..80 => Level::Half,
        _ => Level::Full,
    }
}

/// Tokens in a few characters: 950, 68k, 1.2M (rounded down).
pub fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let tenths = n / 100_000;
        if tenths % 10 == 0 { format!("{}M", tenths / 10) } else { format!("{}.{}M", tenths / 10, tenths % 10) }
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

/// The popover's sentence. `summarized`: the last /compact's clock time and
/// how full it was before, which replaces the under-50% sentence.
pub fn sentence(usage: Option<(u64, u64)>, summarized: Option<(&str, Option<u64>)>, agent: Agent) -> String {
    let name = agent.name();
    let Some((used, size)) = usage else {
        return format!("Context fills as you and {name} talk. This ring shows how full it is.");
    };
    match (level(used, size), summarized) {
        (Level::Plenty, Some((at, Some(was)))) => format!("Summarized at {at} (was {was}%). Earlier messages are still in the transcript; {name} works from the summary."),
        (Level::Plenty, Some((at, None))) => format!("Summarized at {at}. Earlier messages are still in the transcript; {name} works from the summary."),
        (Level::Plenty, None) => format!("How much of this conversation {name} can still keep in mind. Plenty left."),
        (Level::Half, _) => format!("Over half used. Near the end, {name} summarizes the conversation so far on its own, and may drop details."),
        (Level::Full, _) => format!("Nearly full. {name} will soon summarize the conversation on its own. To choose what it keeps, summarize now with /compact."),
    }
}

/// The popover's first line: "34% of context used". VoiceOver reads it as the ring's name.
pub fn title(usage: Option<(u64, u64)>) -> String {
    match usage {
        Some((used, size)) => format!("{}% of context used", percent(used, size)),
        None => "Context".into(),
    }
}

/// Whether the popover is open: hovered long enough, or pinned with a click.
#[derive(Default)]
pub struct RingPopover {
    pub hovered: bool,
    /// Hovered for `HOVER_DELAY`.
    pub shown: bool,
    pub pinned: bool,
}

impl Workspace {
    /// Esc: close a pinned or open context popover. False when none was open.
    pub fn close_context_popover(&mut self, cx: &mut Context<Self>) -> bool {
        let p = &mut self.composer.context;
        if !(p.pinned || p.shown) {
            return false;
        }
        p.pinned = false;
        p.shown = false;
        cx.notify();
        true
    }

    fn hover_ring(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.composer.context.hovered = hovered;
        if !hovered {
            self.composer.context.shown = false;
            return cx.notify();
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOVER_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.composer.context.hovered {
                    this.composer.context.shown = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// "Summarize now…": "/compact " in the box with its hint, for the user to add what to keep.
    fn summarize_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.context = RingPopover::default();
        self.set_composer_text("/compact ".into(), window, cx);
        cx.notify();
    }

    /// The ring at the end of the composer's toolbar, greyed until Claude reports usage.
    pub fn render_context_ring(&self, session: Option<&Session>, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let usage = session.and_then(|s| s.usage).filter(|(_, size)| *size > 0);
        let focus = self.composer.focus_context.clone();
        let state = &self.composer.context;
        let open = state.pinned || state.shown || (focus.is_focused(window) && window.last_input_was_keyboard());
        let fraction = usage.map_or(0., |(used, size)| used as f32 / size as f32);
        div()
            .id("context")
            .role(Role::Button)
            .aria_label(title(usage))
            .relative()
            .flex_shrink_0()
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when(open, |d| d.bg(theme::bg_raised()))
            .border_2()
            .border_color(gpui::transparent_black())
            .track_focus(&focus)
            .tab_stop(true)
            .focus_ring()
            .child(div().when(usage.is_none(), |d| d.opacity(0.5)).child(context_ring(fraction)))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.hover_ring(*hovered, cx)))
            .on_click(cx.listener(|this, _, _, cx| {
                let p = &mut this.composer.context;
                p.pinned = !p.pinned;
                if !p.pinned {
                    p.shown = false;
                }
                cx.notify();
            }))
            .when(open, |d| d.child(self.render_context_popover(session, usage, cx)))
            .into_any_element()
    }

    fn render_context_popover(&self, session: Option<&Session>, usage: Option<(u64, u64)>, cx: &mut Context<Self>) -> AnyElement {
        let summarized = session.and_then(|s| s.summarized).map(|(at, was)| (crate::when::clock(at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()), was));
        let text = sentence(usage, summarized.as_ref().map(|(at, was)| (at.as_str(), *was)), self.composer_agent(session));
        let full = usage.is_some_and(|(used, size)| level(used, size) == Level::Full);
        let tone = if full { theme::accent() } else { theme::text_secondary() };
        let body = div()
            .occlude()
            .w(px(if usage.is_some() { 300. } else { 260. }))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(10.))
            .bg(theme::popover_bg())
            .border_1()
            .border_color(theme::popover_edge())
            .shadow(theme::popover_shadow())
            .flex()
            .flex_col()
            .gap(px(6.))
            .cursor_default()
            .when_some(usage, |d, (used, size)| {
                d.child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(6.))
                        .child(div().text_size(theme::size_body()).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_primary()).child(title(usage)))
                        .child(div().flex_1())
                        .child(div().text_size(theme::size_meta()).text_color(theme::text_muted()).child(format!("{} of {}", tokens(used), tokens(size)))),
                )
                .child(
                    div()
                        .h(px(4.))
                        .rounded(px(2.))
                        .bg(theme::border())
                        .overflow_hidden()
                        .child(div().h_full().w(relative((used as f32 / size as f32).clamp(0., 1.))).bg(tone)),
                )
            })
            .child(div().text_size(px(12.5)).line_height(px(18.)).text_color(theme::text_secondary()).child(text))
            .when(full, |d| {
                d.child(
                    div().mt(px(2.)).flex().child(
                        div()
                            .id("summarize-now")
                            .role(Role::Button)
                            .aria_label("Summarize now…")
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .px(px(10.))
                            .rounded(px(5.))
                            .bg(theme::accent())
                            .text_color(gpui::white())
                            .text_size(theme::size_meta())
                            .cursor_pointer()
                            .child("Summarize now…")
                            .on_click(cx.listener(|this, _, window, cx| this.summarize_now(window, cx))),
                    ),
                )
            });
        div()
            .absolute()
            .bottom(relative(1.))
            .right(px(-2.))
            .mb(px(8.))
            .child(deferred(anchored().anchor(Anchor::BottomRight).child(body)).with_priority(2))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Level, level, sentence, title, tokens};
    use crate::agent::Agent;

    #[test]
    fn levels_turn_at_50_and_80_percent() {
        assert_eq!(level(99_999, 200_000), Level::Plenty);
        assert_eq!(level(100_000, 200_000), Level::Half);
        assert_eq!(level(159_999, 200_000), Level::Half);
        assert_eq!(level(160_000, 200_000), Level::Full);
        assert_eq!(title(Some((68_000, 200_000))), "34% of context used");
        assert_eq!(title(Some((172_999, 200_000))), "86% of context used");
    }

    #[test]
    fn tokens_read_as_k_and_m() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(68_400), "68k");
        assert_eq!(tokens(200_000), "200k");
        assert_eq!(tokens(1_000_000), "1M");
        assert_eq!(tokens(1_250_000), "1.2M");
    }

    #[test]
    fn the_sentence_follows_the_level() {
        assert_eq!(sentence(None, None, Agent::Claude), "Context fills as you and Claude talk. This ring shows how full it is.");
        assert_eq!(sentence(Some((68_000, 200_000)), None, Agent::Claude), "How much of this conversation Claude can still keep in mind. Plenty left.");
        assert_eq!(
            sentence(Some((124_000, 200_000)), None, Agent::Claude),
            "Over half used. Near the end, Claude summarizes the conversation so far on its own, and may drop details."
        );
        assert_eq!(
            sentence(Some((172_000, 200_000)), None, Agent::Claude),
            "Nearly full. Claude will soon summarize the conversation on its own. To choose what it keeps, summarize now with /compact."
        );
    }

    #[test]
    fn a_codex_session_names_codex_in_the_sentence() {
        assert_eq!(sentence(None, None, Agent::Codex), "Context fills as you and Codex talk. This ring shows how full it is.");
        assert_eq!(
            sentence(Some((172_000, 200_000)), None, Agent::Codex),
            "Nearly full. Codex will soon summarize the conversation on its own. To choose what it keeps, summarize now with /compact."
        );
    }

    #[test]
    fn after_a_summary_it_says_when() {
        assert_eq!(
            sentence(Some((22_000, 200_000)), Some(("14:02", Some(86))), Agent::Claude),
            "Summarized at 14:02 (was 86%). Earlier messages are still in the transcript; Claude works from the summary."
        );
        assert_eq!(
            sentence(Some((124_000, 200_000)), Some(("14:02", Some(86))), Agent::Claude),
            "Over half used. Near the end, Claude summarizes the conversation so far on its own, and may drop details."
        );
    }
}
