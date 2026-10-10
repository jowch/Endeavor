//! Offline is waiting, not failure: nothing turns red or orange, every offline
//! state has a quiet Try now, and Endeavor carries on by itself when the
//! network is back. Claude's messages wait in the queue, the notebook on This
//! Mac keeps working, a server's notebook stays readable but read-only, and a
//! first launch's setup pauses.

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime};

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::Workspace;
use crate::agent::Agent;
use crate::failure::{self, Lead};
use crate::trouble::{Reset, Trouble};
use crate::hosts::HostId;
use crate::new_session::{Glyph, glyph, glyph_at};
use crate::session::Session;
use crate::signin::{Look, button};
use crate::splash::{Setup, Step};
use crate::theme;
use crate::theme::FocusRing as _;

/// How long a server that can't be reached waits between tries.
pub const RETRY_EVERY: Duration = Duration::from_secs(15);

/// An agent's usage limit, reached: when it resets, if the agent said.
pub struct UsageLimit {
    pub until: Option<SystemTime>,
}

/// Each agent's usage limit while it's reached. Claude's limit holds only
/// Claude's sessions; Codex can still answer, and the other way round.
#[derive(Default)]
pub struct UsageLimits(BTreeMap<Agent, UsageLimit>);

impl UsageLimits {
    pub fn get(&self, agent: Agent) -> Option<&UsageLimit> {
        self.0.get(&agent)
    }

    pub fn holds(&self, agent: Agent) -> bool {
        self.0.contains_key(&agent)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn hit(&mut self, agent: Agent, until: Option<SystemTime>) {
        self.0.insert(agent, UsageLimit { until });
    }

    /// Whether `agent` had a limit to end.
    pub fn end(&mut self, agent: Agent) -> bool {
        self.0.remove(&agent).is_some()
    }

    /// The agents whose limit has reset by `now`.
    pub fn reset_by(&self, now: SystemTime) -> Vec<Agent> {
        self.0.iter().filter(|(_, l)| l.until.is_some_and(|until| now >= until)).map(|(a, _)| *a).collect()
    }
}

/// A connect error that means the server couldn't be reached (as opposed to,
/// say, a refused key): worth trying again by itself.
pub fn unreachable(error: &str) -> bool {
    let error = error.to_lowercase();
    [
        "timed out",
        "timeout",
        "could not resolve",
        "network is unreachable",
        "no route to host",
        "connection refused",
        "connection reset",
        "connection closed",
        "broken pipe",
        "host is down",
        "closed unexpectedly",
        // remote::explain's words for the same.
        "didn't answer",
        "couldn't reach",
        "refused the connection",
        "couldn't find a server called",
    ]
    .iter()
    .any(|w| error.contains(w))
}

impl Workspace {
    /// Follow the system's network status for the life of the window.
    pub fn watch_network(&mut self, cx: &mut Context<Self>) {
        let mut changes = crate::network::watch();
        cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            while let Some(online) = changes.next().await {
                if this.update(cx, |this, cx| this.set_online(online, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn offline_since(&self) -> Option<Instant> {
        self.offline_since
    }

    fn set_online(&mut self, online: bool, cx: &mut Context<Self>) {
        if online == self.offline_since.is_none() {
            return;
        }
        self.offline_since = (!online).then(Instant::now);
        self.sync_holds(cx);
        if online {
            self.carry_on(cx);
        }
        cx.notify();
    }

    /// Try now: look at the network this moment and pick up whatever waits.
    pub fn try_now(&mut self, cx: &mut Context<Self>) {
        if self.probing {
            return;
        }
        self.probing = true;
        let probe = cx.background_executor().spawn(async { crate::network::probe() });
        cx.spawn(async move |this, cx| {
            let online = probe.await;
            // Only good news is taken from here: the system's status says when
            // the network goes, and it won't say again if this was wrong.
            let _ = this.update(cx, |this, cx| {
                this.probing = false;
                if online && this.offline_since.is_none() {
                    this.carry_on(cx);
                }
                if online {
                    this.set_online(true, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Back online: setup goes on, and servers that dropped reconnect.
    fn carry_on(&mut self, cx: &mut Context<Self>) {
        if self.setup.as_ref().is_some_and(Setup::failed) {
            self.retry_setup(cx);
        }
        let lost: Vec<HostId> = self.connections.iter().filter(|(_, c)| c.waiting_to_reconnect()).map(|(h, _)| h.clone()).collect();
        for host in lost {
            self.reconnect_lost(&host, cx);
        }
    }

    /// A session's turn failed. A usage limit or a lost sign-in waits at once.
    /// Otherwise, if Claude can't be reached from here either, that's offline
    /// even while the system has a network (a dead Wi-Fi, say): the message
    /// waits and goes again once it can be reached. Else the turn failed.
    pub fn turn_failed(&mut self, key: u64, kind: Option<String>, error: String, cx: &mut Context<Self>) {
        let trouble = crate::trouble::classify(kind.as_deref(), &error);
        if matches!(trouble, Trouble::UsageLimit(_) | Trouble::SignIn) {
            let Some(session) = self.session_mut(key) else { return };
            let effects = session.turn_failed(trouble, &error);
            return self.apply_effects(key, effects, cx);
        }
        let probe = self.offline_since.is_none().then(|| cx.background_executor().spawn(async { crate::network::probe() }));
        cx.spawn(async move |this, cx| {
            let reachable = match probe {
                Some(probe) => probe.await,
                None => false,
            };
            let _ = this.update(cx, |this, cx| {
                if !reachable && this.offline_since.is_none() {
                    this.set_online(false, cx);
                    this.recheck(cx);
                }
                let Some(session) = this.session_mut(key) else { return };
                let effects = session.turn_failed(trouble, &error);
                this.apply_effects(key, effects, cx);
            });
        })
        .detach();
    }

    /// A turn hit `agent`'s usage limit: that agent's sessions' messages
    /// wait until it resets (when the message said when), then go by
    /// themselves. The other agent's sessions carry on.
    pub fn hit_usage_limit(&mut self, agent: Agent, reset: Option<Reset>, cx: &mut Context<Self>) {
        let until = reset.and_then(|r| crate::trouble::resolve(r, SystemTime::now(), crate::trouble::local_offset()));
        self.usage_limits.hit(agent, until);
        self.sync_holds(cx);
        cx.notify();
    }

    /// Every second: past an agent's reset, its waiting messages go.
    pub fn check_usage_limit(&mut self, cx: &mut Context<Self>) {
        for agent in self.usage_limits.reset_by(SystemTime::now()) {
            self.end_usage_limit(agent, cx);
        }
    }

    /// `agent`'s limit has reset, or Try now: what waited goes. If the limit
    /// still holds, the next turn says so again.
    pub fn end_usage_limit(&mut self, agent: Agent, cx: &mut Context<Self>) {
        if self.usage_limits.end(agent) {
            self.sync_holds(cx);
            cx.notify();
        }
    }

    /// The usage-limit line above the session's composer, as it reads now:
    /// only while the session's own agent is at its limit.
    pub fn usage_line(&self, session: &Session) -> Option<String> {
        let limit = self.usage_limits.get(session.agent)?;
        let waiting = session.unanswered.is_some() || !session.outbox.items.is_empty();
        Some(crate::trouble::usage_line(limit.until, SystemTime::now(), crate::trouble::local_offset(), waiting, session.agent))
    }

    pub fn render_usage_line(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = self.usage_line(session)?;
        let agent = session.agent;
        let until = self.usage_limits.get(agent).and_then(|l| l.until);
        let try_now = until.is_none().then(|| {
            button("usage-try-now", "Try now", Look::Secondary).h(px(24.)).on_click(cx.listener(move |this, _, _, cx| this.end_usage_limit(agent, cx))).into_any_element()
        });
        Some(failure::wait_line(Lead::Clock, text, try_now, cx).into_any_element())
    }

    /// The composer's placeholder while messages wait for the agent: until
    /// the usage limit resets, or until the agent's process is back.
    pub fn waiting_placeholder(&self, agent: crate::agent::Agent) -> Option<SharedString> {
        use crate::agent_process::State;
        match self.links.get(agent).process.state {
            State::Restarting => return Some(format!("Write a message. It sends once {} is back.", agent.name()).into()),
            State::Down => return Some(format!("Write a message. It sends once {} is running.", agent.name()).into()),
            State::Up => {}
        }
        let limit = self.usage_limits.get(agent)?;
        Some(match limit.until {
            Some(at) => {
                let when = crate::trouble::when_text(at, SystemTime::now(), crate::trouble::local_offset());
                format!("Write a message. It sends {when}.").into()
            }
            None => "Write a message. It sends once your limit resets.".into(),
        })
    }

    /// Offline on Claude's API's word, not the system's, which won't say when
    /// it's back: look again every `RETRY_EVERY` until it is.
    fn recheck(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(RETRY_EVERY).await;
                let Ok(offline) = this.update(cx, |this, _| this.offline_since.is_some()) else { return };
                if !offline {
                    return;
                }
                if cx.background_executor().spawn(async { crate::network::probe() }).await {
                    let _ = this.update(cx, |this, cx| this.set_online(true, cx));
                    return;
                }
            }
        })
        .detach();
    }

    /// A server's connection dropped: try again after `wait`, unless it's
    /// back, given up on, or waiting for the network.
    pub fn retry_lost(&mut self, host: HostId, wait: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                let waiting = this.connections.get(&host).is_some_and(|c| c.waiting_to_reconnect());
                if waiting && this.offline_since.is_none() {
                    this.reconnect_lost(&host, cx);
                }
            });
        })
        .detach();
    }

    /// The session's notebook can be read but not changed: its server is out of reach.
    pub fn read_only(&self, session: &Session) -> bool {
        match &session.place.host {
            HostId::ThisMac => false,
            host => self.offline_since.is_some() || self.connections.get(host).is_some_and(|c| c.lost.is_some()),
        }
    }

    fn try_now_button(&self, id: &'static str, height: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        button(id, if self.probing { "Trying…" } else { "Try now" }, Look::Secondary).h(px(height)).on_click(cx.listener(|this, _, _, cx| this.try_now(cx)))
    }

    /// One grey line above the composer while offline.
    pub fn offline_line(&self, session: Option<&Session>) -> Option<&'static str> {
        self.offline_since?;
        let on_server = session.is_some_and(|s| s.place.host != HostId::ThisMac);
        let agent = session.map_or(crate::agent::Agent::Claude, |s| s.agent);
        Some(if on_server {
            crate::agent_text!(agent, "You're offline. ", " will continue when you're back.")
        } else {
            crate::agent_text!(agent, "You're offline. The notebook still works. ", " will continue when you're back.")
        })
    }

    pub fn render_offline_line(&self, session: Option<&Session>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = self.offline_line(session)?;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(2.))
                .text_size(theme::chat_meta())
                .line_height(px(17.))
                .text_color(theme::text_new())
                .child(glyph_at(Glyph::WifiOff, theme::text_muted(), 13. / 12.))
                .child(div().flex_1().min_w_0().child(text))
                .child(self.try_now_button("try-now-chat", 24., cx))
                .into_any_element(),
        )
    }

    /// The pane warning's words: its line, or its title and line.
    pub fn pane_warning(&self, session: &Session) -> (String, Option<&'static str>) {
        if self.offline_since.is_some() {
            ("Looks like you're offline. Endeavor will reconnect when you're back online.".into(), None)
        } else {
            (format!("Can't reach {}", self.hosts.name(&session.place.host)), Some("If it needs your university's VPN, check that it's on."))
        }
    }

    /// The one warning at the top of a server's notebook pane while it's out of reach.
    pub fn render_pane_warning(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.read_only(session) {
            return None;
        }
        let frame = div()
            .flex_shrink_0()
            .mx(px(16.))
            .mt(px(4.))
            .mb(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(6.))
            .py(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_card())
            .text_size(theme::size_meta())
            .line_height(px(17.))
            .text_color(theme::text_secondary())
            .child(glyph_at(Glyph::WifiOff, theme::text_muted(), 13. / 12.));
        let host = session.place.host.clone();
        let body = match self.pane_warning(session) {
            (text, None) => div().flex_1().min_w_0().child(text),
            (title, Some(text)) => div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    // Opens Settings at the server, where Try now and its connection settings are.
                    div()
                        .id("cant-reach-settings")
                        .role(Role::Link)
                        .aria_label(format!("{title}: open Where notebooks run"))
                        .border_2()
                        .border_color(gpui::transparent_black())
                        .track_focus(&self.dialog_focus("cant-reach-settings", cx))
                        .tab_stop(true)
                        .focus_ring_on(theme::bg_card())
                        .self_start()
                        .cursor_pointer()
                        .text_size(theme::size_body())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text_primary())
                        .hover(|s| s.underline())
                        .child(title)
                        .on_click(cx.listener(move |this, _, window, cx| this.open_settings_at_host(&host, window, cx))),
                )
                .child(div().text_color(theme::text_new()).child(text)),
        };
        Some(frame.child(body).child(self.try_now_button("try-now-pane", 24., cx)).into_any_element())
    }

    /// The queue's heading while its messages wait for Claude to be reachable.
    pub fn queue_heading(&self, session: &Session) -> Option<String> {
        if !self.holds(session) || session.outbox.items.is_empty() {
            return None;
        }
        Some(if self.offline_since.is_some() {
            "These send in order when you're back.".to_owned()
        } else if self.signed_out_of_agent(session.agent) {
            "These send in order once you sign in.".to_owned()
        } else if !self.links.get(session.agent).process.up() {
            format!("These send in order once {} is back.", session.agent.name())
        } else if self.usage_limits.holds(session.agent) {
            "These send in order once your limit resets.".to_owned()
        } else {
            format!("These send in order once {} is back.", self.hosts.name(&session.place.host))
        })
    }

    pub fn render_queue_heading(&self, session: &Session) -> Option<AnyElement> {
        let when = self.queue_heading(session)?;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(theme::chat_meta())
                .line_height(px(17.))
                .text_color(theme::text_muted())
                .child(glyph(Glyph::Clock, theme::text_muted()))
                .child(when)
                .into_any_element(),
        )
    }

    /// First launch without a network: each step says whether it needs the
    /// internet, and setup carries on by itself once it's back.
    pub fn render_offline_setup(&self, setup: &Setup, cx: &mut Context<Self>) -> AnyElement {
        let current = setup.step();
        let steps = Step::ALL.map(|step| {
            let needs = |paused: bool| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(theme::text_muted())
                    .when(paused, |d| d.child("paused ·"))
                    .child(glyph_at(Glyph::WifiOff, theme::text_muted(), 11. / 12.))
                    .child("needs internet")
            };
            let (mark, name_color, state) = match step.cmp(&current) {
                std::cmp::Ordering::Less => (glyph(Glyph::Check, theme::diff_add()).into_any_element(), theme::text_secondary(), div().text_color(theme::text_muted()).child("done")),
                std::cmp::Ordering::Equal => (div().size(px(8.)).rounded_full().border(px(1.5)).border_color(theme::text_muted()).into_any_element(), theme::text_primary(), needs(true)),
                std::cmp::Ordering::Greater => (div().size(px(6.)).rounded_full().bg(theme::composer_edge()).into_any_element(), theme::text_muted(), needs(false)),
            };
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(24.))
                .text_size(theme::size_meta())
                .child(div().w(px(12.)).flex().justify_center().child(mark))
                .child(div().flex_1().text_color(name_color).child(step.label(setup.agent_name())))
                .child(state)
        });
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .rounded(px(10.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg_card())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(theme::size_subhead())
                    .line_height(px(22.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(glyph_at(Glyph::WifiOff, theme::text_secondary(), 15. / 12.))
                    .child("No internet connection"),
            )
            .child(div().mt(px(-6.)).text_color(theme::text_secondary()).child("Setup needs the internet once. It will continue when you're back online."))
            .child(div().py(px(4.)).flex().flex_col().children(steps))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(crate::orbit::orbit("setup-offline-orbit".into(), 14., cx))
                    .child(div().flex_1().text_size(theme::size_meta()).text_color(theme::text_muted()).child("Waiting for a connection"))
                    .child(self.try_now_button("try-now-setup", 26., cx)),
            )
            .child(div().h(px(1.)).mx(px(-16.)).bg(theme::border()))
            .child(div().text_size(theme::size_meta()).line_height(px(17.)).text_color(theme::text_muted()).child("After setup, only Claude needs the internet."))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{UsageLimits, unreachable};
    use crate::agent::Agent;
    use std::time::{Duration, SystemTime};

    #[test]
    fn a_claude_limit_leaves_codex_reachable() {
        let now = SystemTime::now();
        let mut limits = UsageLimits::default();
        limits.hit(Agent::Claude, Some(now + Duration::from_secs(60)));
        assert!(limits.holds(Agent::Claude));
        assert!(!limits.holds(Agent::Codex));
        limits.hit(Agent::Codex, None);
        // Claude's reset ends Claude's limit only; Codex's, with no time, waits for Try now.
        assert_eq!(limits.reset_by(now), []);
        assert_eq!(limits.reset_by(now + Duration::from_secs(60)), [Agent::Claude]);
        assert!(limits.end(Agent::Claude));
        assert!(!limits.end(Agent::Claude));
        assert!(limits.holds(Agent::Codex) && !limits.is_empty());
    }

    #[test]
    fn network_failures_are_worth_retrying_and_refusals_are_not() {
        assert!(unreachable("ssh: connect to host lab port 22: Operation timed out"));
        assert!(unreachable("ssh: Could not resolve hostname lab: nodename nor servname provided"));
        assert!(unreachable("The connection to Julia closed unexpectedly."));
        assert!(!unreachable("Permission denied (publickey)."));
        assert!(!unreachable("Host key verification failed."));
    }
}
