//! Claude Code's adapter process, as the chat sees it. When it stops by itself
//! Endeavor starts it again, unless it also stopped in the minute before: a
//! process that keeps stopping is left stopped, with a card that says so.

use std::time::{Duration, Instant};

use gpui::*;

use crate::Workspace;
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
    /// It stopped twice in a minute; it starts again from the card's Restart Claude.
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

    /// Restart Claude on the card: a fresh start, with no earlier stop counted.
    pub fn restart_by_hand(&mut self) {
        self.last_stop = None;
        self.state = State::Restarting;
    }

    /// It's connected.
    pub fn ready(&mut self) {
        self.state = State::Up;
    }
}

impl Workspace {
    /// Claude's process stopped by itself: start it again, unless that's the
    /// second time in a minute. Either way, sessions keep what they have and
    /// open again once it's back; messages queue meanwhile.
    pub fn claude_stopped(&mut self, error: String, cx: &mut Context<Self>) {
        eprintln!("Claude's process stopped: {error}");
        let mut details = vec![error];
        details.extend(log_tail(8));
        let restart = self.claude.stopped(Instant::now(), details.join("\n"));
        for session in &mut self.sessions {
            session.agent_stopped();
        }
        self.sync_holds(cx);
        if restart {
            self.restart_agent(cx);
        } else {
            // About offers Update, in case a newer adapter helps.
            self.agent_failed = true;
        }
        cx.notify();
    }

    /// The card's Restart Claude.
    pub fn restart_claude(&mut self, cx: &mut Context<Self>) {
        self.claude.restart_by_hand();
        self.claude_details_open = false;
        self.agent_failed = false;
        self.restart_agent(cx);
    }

    /// Claude is connected: the sessions it lost open again where they were.
    pub fn claude_ready(&mut self, cx: &mut Context<Self>) {
        self.claude.ready();
        let waiting: Vec<u64> = self.sessions.iter().filter(|s| s.agent_waiting && s.failed.is_none()).map(|s| s.key).collect();
        for key in waiting {
            self.request_agent(key, cx);
        }
        self.sync_holds(cx);
    }

    /// Above the composer while Claude's process is restarting (waiting), or
    /// left stopped (failed: it needs the user).
    pub fn render_claude_trouble(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.claude.state {
            State::Up => None,
            State::Restarting => Some(failure::wait_line(Lead::Spinner("claude-restarting".into()), RESTARTING, None, cx).into_any_element()),
            State::Down => {
                let buttons = vec![
                    failure::action("restart-claude", Some(Glyph::Restart), "Restart Claude", Look::Primary)
                        .on_click(cx.listener(|this, _, _, cx| this.restart_claude(cx)))
                        .into_any_element(),
                    failure::action("claude-logs", None, "Show logs", Look::Plain).on_click(|_, _, _| crate::logs::reveal()).into_any_element(),
                ];
                let entity = cx.entity().downgrade();
                let details = failure::details("claude-details", self.claude.error.clone(), self.claude_details_open, move |_, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.claude_details_open = !this.claude_details_open;
                        cx.notify();
                    });
                });
                Some(failure::card(DOWN_TITLE, DOWN_BODY, buttons, Some(details)).into_any_element())
            }
        }
    }
}

pub const RESTARTING: &str = "Claude stopped unexpectedly. Restarting it…";
pub const DOWN_TITLE: &str = "Claude isn't running";
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
        assert!(p.stopped(t + Duration::from_secs(50), "again".into()), "Restart Claude starts the count over");

        let mut q = Process::default();
        q.stopped(t, "once".into());
        q.ready();
        assert!(q.stopped(t + Duration::from_secs(61), "a minute later".into()), "stops further apart each restart");
    }
}
