//! The chat's outgoing messages. Enter sends when the agent is idle and queues
//! while it works; Cmd+Enter sends now (steers the running turn). Queued
//! messages go out one per turn, in order, and stay editable until they do.

use std::collections::VecDeque;

use agent_client_protocol::schema::v1::ContentBlock;

use crate::agent::Command;

pub struct Queued {
    /// Shown in the queue and, once sent, in the transcript.
    pub label: String,
    /// Chat text that can be pulled back into the input to edit; None for annotations.
    pub editable: Option<String>,
    pub blocks: Vec<ContentBlock>,
    /// Handed to the agent as a SendNow and awaiting Steered/Unsent.
    in_flight: bool,
}

impl Queued {
    pub fn new(label: String, editable: Option<String>, blocks: Vec<ContentBlock>) -> Self {
        Self { label, editable, blocks, in_flight: false }
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }
}

/// What to hand the agent, and the label to add to the transcript (None while a
/// SendNow's outcome is still unknown).
pub struct Dispatch {
    pub command: Command,
    pub shown: Option<String>,
}

#[derive(Default)]
pub struct Outbox {
    pub items: VecDeque<Queued>,
    pub busy: bool,
}

impl Outbox {
    pub fn submit(&mut self, mut q: Queued, now: bool) -> Option<Dispatch> {
        if !self.busy && self.items.is_empty() {
            self.busy = true;
            return Some(Dispatch { command: Command::Prompt(q.blocks), shown: Some(q.label) });
        }
        // One SendNow at a time: its fallback needs the front slot.
        if now && self.busy && !self.front_in_flight() {
            q.in_flight = true;
            let command = Command::SendNow(q.blocks.clone());
            self.items.push_front(q);
            return Some(Dispatch { command, shown: None });
        }
        self.items.push_back(q);
        self.next()
    }

    pub fn turn_ended(&mut self) -> Option<Dispatch> {
        self.busy = false;
        self.next()
    }

    /// The SendNow joined the running turn: returns its label for the transcript.
    pub fn steered(&mut self) -> Option<String> {
        self.front_in_flight().then(|| self.items.pop_front().unwrap().label)
    }

    /// The SendNow couldn't join the turn: it leads the queue instead.
    pub fn unsent(&mut self) -> Option<Dispatch> {
        if let Some(front) = self.items.front_mut() {
            front.in_flight = false;
        }
        self.next()
    }

    /// Remove a queued message; returns it (e.g. to edit). In-flight ones can't be pulled.
    pub fn take(&mut self, ix: usize) -> Option<Queued> {
        if self.items.get(ix)?.in_flight {
            return None;
        }
        self.items.remove(ix)
    }

    fn front_in_flight(&self) -> bool {
        self.items.front().is_some_and(|q| q.in_flight)
    }

    /// Start the next turn if idle. Waits while a SendNow's outcome is pending, so
    /// a message is never both steered in and re-sent.
    fn next(&mut self) -> Option<Dispatch> {
        if self.busy || self.front_in_flight() {
            return None;
        }
        let q = self.items.pop_front()?;
        self.busy = true;
        Some(Dispatch { command: Command::Prompt(q.blocks), shown: Some(q.label) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(s: &str) -> Queued {
        Queued::new(s.into(), Some(s.into()), vec![])
    }

    fn prompt_label(d: Option<Dispatch>) -> Option<String> {
        match d? {
            Dispatch { command: Command::Prompt(_), shown } => shown,
            _ => panic!("expected a Prompt"),
        }
    }

    #[test]
    fn sends_when_idle_and_queues_while_busy_in_order() {
        let mut o = Outbox::default();
        assert_eq!(prompt_label(o.submit(msg("a"), false)).as_deref(), Some("a"));
        assert!(o.submit(msg("b"), false).is_none());
        assert!(o.submit(msg("c"), false).is_none());
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("b"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("c"));
        assert!(o.turn_ended().is_none());
        assert!(!o.busy);
    }

    #[test]
    fn send_now_steers_ahead_of_the_queue() {
        let mut o = Outbox::default();
        o.submit(msg("a"), false);
        o.submit(msg("queued"), false);
        let d = o.submit(msg("urgent"), true).unwrap();
        assert!(matches!(d.command, Command::SendNow(_)) && d.shown.is_none());
        assert_eq!(o.steered().as_deref(), Some("urgent"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("queued"));
    }

    #[test]
    fn unsent_send_now_leads_the_queue_and_is_never_sent_twice() {
        let mut o = Outbox::default();
        o.submit(msg("a"), false);
        o.submit(msg("queued"), false);
        o.submit(msg("urgent"), true);
        // Turn ends before the steering outcome arrives: hold, don't re-send.
        assert!(o.turn_ended().is_none());
        assert_eq!(prompt_label(o.unsent()).as_deref(), Some("urgent"));
        assert!(o.steered().is_none());
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("queued"));
    }

    #[test]
    fn only_one_send_now_in_flight_and_it_cannot_be_pulled() {
        let mut o = Outbox::default();
        o.submit(msg("a"), false);
        o.submit(msg("first"), true);
        assert!(o.submit(msg("second"), true).is_none()); // queued behind
        assert!(o.take(0).is_none());
        assert_eq!(o.take(1).map(|q| q.label).as_deref(), Some("second"));
    }
}
