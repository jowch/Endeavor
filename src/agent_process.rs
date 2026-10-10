//! Each agent's adapter process, as the chat sees it. When it stops by itself
//! Endeavor starts it again, unless it also stopped in the minute before: a
//! process that keeps stopping is left stopped, with a card that says so.

use std::time::{Duration, Instant};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::*;

use crate::Workspace;
use crate::agent::{Agent, Command};
use crate::failure::{self, Lead};
use crate::new_session::Glyph;
use crate::signin::Look;

/// Two stops this close together: it stops restarting.
pub const CRASH_LOOP: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Starting or running.
    Up,
    /// It stopped by itself and is starting again.
    Restarting,
    /// It stopped twice in a minute; it starts again from the card's Restart button.
    Down,
}

#[derive(Debug)]
pub struct Process {
    pub state: State,
    /// Why it last stopped, and the log's last lines then, for Details.
    pub error: String,
    last_stop: Option<Instant>,
}

impl Default for Process {
    fn default() -> Self {
        Process { state: State::Up, error: String::new(), last_stop: None }
    }
}

impl Process {
    pub fn up(&self) -> bool {
        self.state == State::Up
    }

    /// It stopped by itself at `now`: true to start it again.
    pub fn stopped(&mut self, now: Instant, error: String) -> bool {
        let again = self.last_stop.is_some_and(|last| now.duration_since(last) < CRASH_LOOP);
        self.last_stop = Some(now);
        self.error = error;
        self.state = if again { State::Down } else { State::Restarting };
        !again
    }

    /// Restart on the card: a fresh start, with no earlier stop counted.
    pub fn restart_by_hand(&mut self) {
        self.last_stop = None;
        self.state = State::Restarting;
    }

    /// It's connected.
    pub fn ready(&mut self) {
        self.state = State::Up;
    }
}

/// One agent's connection, as the app keeps it.
pub struct Link {
    pub tx: UnboundedSender<Command>,
    /// Its commands until its thread starts: Claude's at launch (on first
    /// launch, once This Mac's Julia is up), another agent's when one of its
    /// sessions first needs it.
    pub rx: Option<UnboundedReceiver<Command>>,
    pub process: Process,
    /// Connected.
    pub ready: bool,
    /// It stopped with an error (often a failed adapter install); About offers Update.
    pub failed: bool,
    /// The "isn't running" card's Details are open.
    pub details_open: bool,
}

impl Default for Link {
    fn default() -> Self {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        Link { tx, rx: Some(rx), process: Process::default(), ready: false, failed: false, details_open: false }
    }
}

impl Link {
    /// A fresh command channel for a new thread (the failed one dropped its own).
    pub fn renew(&mut self) -> UnboundedReceiver<Command> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.tx = tx;
        rx
    }
}

/// Every agent's link.
#[derive(Default)]
pub struct Links {
    claude: Link,
    codex: Link,
    antigravity: Link,
}

impl Links {
    pub fn get(&self, agent: Agent) -> &Link {
        match agent {
            Agent::Claude => &self.claude,
            Agent::Codex => &self.codex,
            Agent::Antigravity => &self.antigravity,
        }
    }

    pub fn get_mut(&mut self, agent: Agent) -> &mut Link {
        match agent {
            Agent::Claude => &mut self.claude,
            Agent::Codex => &mut self.codex,
            Agent::Antigravity => &mut self.antigravity,
        }
    }

    pub fn send(&self, agent: Agent, command: Command) {
        let _ = self.get(agent).tx.unbounded_send(command);
    }
}

impl Workspace {
    /// An agent's process stopped by itself: start it again, unless that's
    /// the second time in a minute. Either way, its sessions keep what they
    /// have and open again once it's back; messages queue meanwhile.
    pub fn agent_stopped(&mut self, agent: Agent, error: String, cx: &mut Context<Self>) {
        eprintln!("{}'s process stopped: {error}", agent.name());
        let mut details = vec![error];
        details.extend(log_tail(8));
        let restart = self.links.get_mut(agent).process.stopped(Instant::now(), details.join("\n"));
        for session in self.sessions.iter_mut().filter(|s| s.agent == agent) {
            session.agent_stopped();
        }
        self.sync_holds(cx);
        if restart {
            self.restart_agent(agent, cx);
        } else {
            // About offers Update, in case a newer adapter helps.
            self.links.get_mut(agent).failed = true;
        }
        cx.notify();
    }

    /// The card's Restart button.
    pub fn restart_by_hand(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let link = self.links.get_mut(agent);
        link.process.restart_by_hand();
        link.details_open = false;
        link.failed = false;
        self.restart_agent(agent, cx);
    }

    /// An agent is connected: its sessions waiting for it open where they were.
    pub fn agent_ready(&mut self, agent: Agent, cx: &mut Context<Self>) {
        self.links.get_mut(agent).process.ready();
        self.open_waiting(agent, cx);
    }

    /// Open the sessions waiting for `agent`, where it can now.
    pub fn open_waiting(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let waiting: Vec<u64> = self.sessions.iter().filter(|s| s.agent == agent && s.agent_waiting && s.failed.is_none()).map(|s| s.key).collect();
        for key in waiting {
            self.request_agent(key, cx);
        }
        self.sync_holds(cx);
    }

    /// Above the composer while the session's agent is restarting (waiting),
    /// or left stopped (failed: it needs the user).
    pub fn render_agent_trouble(&self, agent: Agent, cx: &mut Context<Self>) -> Option<AnyElement> {
        let link = self.links.get(agent);
        match link.process.state {
            State::Up => None,
            State::Restarting => Some(failure::wait_line(Lead::Spinner("agent-restarting".into()), restarting(agent), None, cx).into_any_element()),
            State::Down => {
                let buttons = vec![
                    failure::action("restart-agent", Some(Glyph::Restart), format!("Restart {}", agent.name()), Look::Primary)
                        .on_click(cx.listener(move |this, _, _, cx| this.restart_by_hand(agent, cx)))
                        .into_any_element(),
                    failure::action("agent-logs", None, "Show logs", Look::Plain).on_click(|_, _, _| crate::logs::reveal()).into_any_element(),
                ];
                let entity = cx.entity().downgrade();
                let details = failure::details("agent-details", link.process.error.clone(), link.details_open, move |_, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        let link = this.links.get_mut(agent);
                        link.details_open = !link.details_open;
                        cx.notify();
                    });
                });
                Some(failure::card(down_title(agent), DOWN_BODY, buttons, Some(details)).into_any_element())
            }
        }
    }
}

/// "Claude stopped unexpectedly. Restarting it…"
pub fn restarting(agent: Agent) -> String {
    format!("{} stopped unexpectedly. Restarting it…", agent.name())
}

/// "Claude isn't running"
pub fn down_title(agent: Agent) -> String {
    format!("{} isn't running", agent.name())
}

pub const DOWN_BODY: &str = "It stopped twice in a minute, so Endeavor stopped restarting it. The notebook still works.";

/// The last `n` lines of the app's log, where the adapter's own output goes.
pub fn log_tail(n: usize) -> Vec<String> {
    let Some(text) = crate::logs::path().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]`.
    use super::{Process, State};
    use std::time::{Duration, Instant};

    #[test]
    fn it_restarts_once_and_stops_after_two_stops_in_a_minute() {
        let t = Instant::now();
        let mut p = Process::default();
        assert!(p.stopped(t, "adapter exited with code 1".into()), "the first stop restarts it");
        assert_eq!(p.state, State::Restarting);
        p.ready();
        assert!(!p.stopped(t + Duration::from_secs(40), "adapter exited with code 1".into()), "a second within the minute doesn't");
        assert_eq!(p.state, State::Down);

        p.restart_by_hand();
        assert_eq!(p.state, State::Restarting);
        p.ready();
        assert!(p.stopped(t + Duration::from_secs(50), "again".into()), "Restart starts the count over");

        let mut q = Process::default();
        q.stopped(t, "once".into());
        q.ready();
        assert!(q.stopped(t + Duration::from_secs(61), "a minute later".into()), "stops further apart each restart");
    }
}
