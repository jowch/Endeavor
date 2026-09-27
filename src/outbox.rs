//! The chat's outgoing messages. Enter sends when the agent is idle and queues
//! while it works; Cmd+Enter sends now (steers the running turn). Queued
//! messages go out one per turn, in order, and stay editable until they do.

use std::collections::VecDeque;

use agent_client_protocol::schema::v1::ContentBlock;

use crate::agent::Turn;
use crate::attach::Attachment;

pub struct Queued {
    /// The user's words: shown in the queue and, once sent, in the transcript,
    /// and put back in the composer to edit.
    pub text: String,
    /// Shown as chips with the words.
    pub attachments: Vec<Attachment>,
    pub blocks: Vec<ContentBlock>,
    /// Handed to the agent as a SendNow and awaiting Steered/Unsent.
    in_flight: bool,
}

impl Queued {
    pub fn new(text: String, attachments: Vec<Attachment>, blocks: Vec<ContentBlock>) -> Self {
        Self { text, attachments, blocks, in_flight: false }
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }
}

/// A message as the transcript shows it: the user's words and their chips.
pub type Shown = (String, Vec<Attachment>);

/// What to hand the agent, and the message to add to the transcript (None while
/// a SendNow's outcome is still unknown).
pub struct Dispatch {
    pub turn: Turn,
    pub shown: Option<Shown>,
}

#[derive(Default)]
pub struct Outbox {
    pub items: VecDeque<Queued>,
    pub busy: bool,
}

impl Outbox {
    /// For a session that doesn't exist yet: messages queue until `turn_ended()`
    /// is called once it's up, which sends the first of them.
    pub fn waiting() -> Self {
        Self { items: VecDeque::new(), busy: true }
    }

    pub fn submit(&mut self, mut q: Queued, now: bool) -> Option<Dispatch> {
        if !self.busy && self.items.is_empty() {
            self.busy = true;
            return Some(Dispatch { turn: Turn::Prompt(q.blocks), shown: Some((q.text, q.attachments)) });
        }
        // One SendNow at a time: its fallback needs the front slot.
        if now && self.busy && !self.front_in_flight() {
            q.in_flight = true;
            let turn = Turn::SendNow(q.blocks.clone());
            self.items.push_front(q);
            return Some(Dispatch { turn, shown: None });
        }
        self.items.push_back(q);
        self.next()
    }

    pub fn turn_ended(&mut self) -> Option<Dispatch> {
        self.busy = false;
        self.next()
    }

    /// The SendNow joined the running turn: returns it for the transcript.
    pub fn steered(&mut self) -> Option<Shown> {
        self.front_in_flight().then(|| self.items.pop_front().map(|q| (q.text, q.attachments))).flatten()
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
        Some(Dispatch { turn: Turn::Prompt(q.blocks), shown: Some((q.text, q.attachments)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(s: &str) -> Queued {
        Queued::new(s.into(), vec![], vec![])
    }

    fn prompt_label(d: Option<Dispatch>) -> Option<String> {
        match d? {
            Dispatch { turn: Turn::Prompt(_), shown } => shown.map(|(text, _)| text),
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
        assert!(matches!(d.turn, Turn::SendNow(_)) && d.shown.is_none());
        assert_eq!(o.steered().map(|(text, _)| text).as_deref(), Some("urgent"));
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
    fn a_new_session_holds_messages_until_it_starts() {
        let mut o = Outbox::waiting();
        assert!(o.submit(msg("first"), false).is_none());
        assert!(o.submit(msg("second"), false).is_none());
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("first"));
    }

    #[test]
    fn only_one_send_now_in_flight_and_it_cannot_be_pulled() {
        let mut o = Outbox::default();
        o.submit(msg("a"), false);
        o.submit(msg("first"), true);
        assert!(o.submit(msg("second"), true).is_none()); // queued behind
        assert!(o.take(0).is_none());
        assert_eq!(o.take(1).map(|q| q.text).as_deref(), Some("second"));
    }
}
