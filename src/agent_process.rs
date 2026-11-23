use std::time::{Duration, Instant};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::*;

use crate::Workspace;
use crate::agent::{Agent, Command};
use crate::failure::{self, Lead};
use crate::new_session::Glyph;
use crate::signin::Look;

pub const CRASH_LOOP: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Up,
    Restarting,
    Down,
}

#[derive(Debug)]
pub struct Process {
    pub state: State,
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

    pub fn stopped(&mut self, now: Instant, error: String) -> bool {
        let again = self.last_stop.is_some_and(|last| now.duration_since(last) < CRASH_LOOP);
        self.last_stop = Some(now);
        self.error = error;
        self.state = if again { State::Down } else { State::Restarting };
        !again
    }

    pub fn restart_by_hand(&mut self) {
        self.last_stop = None;
        self.state = State::Restarting;
    }

    pub fn ready(&mut self) {
        self.state = State::Up;
    }
}

pub struct Link {
    pub tx: UnboundedSender<Command>,
    pub rx: Option<UnboundedReceiver<Command>>,
    pub process: Process,
    pub ready: bool,
    pub failed: bool,
    pub details_open: bool,
    pub generation: u64,
}

impl Default for Link {
    fn default() -> Self {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        Link { tx, rx: Some(rx), process: Process::default(), ready: false, failed: false, details_open: false, generation: 0 }
    }
}

impl Link {
    pub fn renew(&mut self) -> UnboundedReceiver<Command> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.tx = tx;
        self.generation += 1;
        rx
    }
}

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
    pub fn agent_stopped(&mut self, agent: Agent, error: String, cx: &mut Context<Self>) {
        eprintln!("{}'s process stopped: {error}", agent.name());
        let mut details = vec![error];
        details.extend(log_tail(8));
        let restart = self.links.get_mut(agent).process.stopped(Instant::now(), details.join("\n"));
        if agent == Agent::Antigravity && self.antigravity_account == crate::agent::Account::SigningIn {
            self.antigravity_account = crate::agent::Account::Failed;
        }
        for session in self.sessions.iter_mut().filter(|s| s.agent == agent) {
            session.agent_stopped();
        }
        self.sync_holds(cx);
        if restart {
            self.restart_agent(agent, cx);
        } else {
            self.links.get_mut(agent).failed = true;
        }
        cx.notify();
    }

    pub fn restart_by_hand(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let link = self.links.get_mut(agent);
        link.process.restart_by_hand();
        link.details_open = false;
        link.failed = false;
        self.restart_agent(agent, cx);
    }

    pub fn agent_ready(&mut self, agent: Agent, cx: &mut Context<Self>) {
        self.links.get_mut(agent).process.ready();
        self.open_waiting(agent, cx);
    }

    pub fn open_waiting(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let waiting: Vec<u64> = self.sessions.iter().filter(|s| s.agent == agent && s.agent_waiting && s.failed.is_none()).map(|s| s.key).collect();
        for key in waiting {
            self.request_agent(key, cx);
        }
        self.sync_holds(cx);
    }

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

pub fn restarting(agent: Agent) -> String {
    format!("{} stopped unexpectedly. Restarting it…", agent.name())
}

pub fn down_title(agent: Agent) -> String {
    format!("{} isn't running", agent.name())
}

pub const DOWN_BODY: &str = "It stopped twice in a minute, so Endeavor stopped restarting it. The notebook still works.";

pub fn log_tail(n: usize) -> Vec<String> {
    let Some(text) = crate::logs::path().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

#[cfg(test)]
mod tests {
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
