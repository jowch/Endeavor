//! The chat's outgoing messages. Enter sends when the agent is idle and queues
//! while it works; Cmd+Enter sends now (steers the running turn). Queued
//! messages go out one per turn, in order, and stay editable until they do.
//! While Claude can't be reached (offline, signed out) the outbox is held:
//! everything queues, and a turn that failed for that reason keeps its message
//! to send again first.

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
/// a SendNow's outcome is still unknown, and for a message sent again).
pub struct Dispatch {
    pub turn: Turn,
    pub shown: Option<Shown>,
}

#[derive(Default)]
pub struct Outbox {
    pub items: VecDeque<Queued>,
    pub busy: bool,
    /// Nothing goes out until `release`: Claude can't be reached.
    pub held: bool,
    /// The running turn's message, so one Claude couldn't answer can go again.
    current: Option<Vec<ContentBlock>>,
    /// A message Claude couldn't answer: it goes again, first, on `release`.
    unanswered: Option<Vec<ContentBlock>>,
}

impl Outbox {
    /// For a session that doesn't exist yet: messages queue until `turn_ended()`
    /// is called once it's up, which sends the first of them.
    pub fn waiting() -> Self {
        Self { busy: true, ..Self::default() }
    }

    pub fn submit(&mut self, mut q: Queued, now: bool) -> Option<Dispatch> {
        if !self.busy && !self.held && self.items.is_empty() && self.unanswered.is_none() {
            return Some(self.start(q));
        }
        // One SendNow at a time: its fallback needs the front slot.
        if now && self.busy && !self.held && !self.front_in_flight() {
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
        self.current = None;
        self.next()
    }

    /// The turn ended without an answer Claude could give (signed out, offline):
    /// hold, and keep its message to send again first.
    pub fn turn_unanswered(&mut self) {
        self.busy = false;
        self.held = true;
        self.unanswered = self.current.take().or(self.unanswered.take());
    }

    pub fn hold(&mut self) {
        self.held = true;
    }

    /// Claude can be reached again: the unanswered message goes, else the next queued one.
    pub fn release(&mut self) -> Option<Dispatch> {
        self.held = false;
        self.next()
    }

    pub fn has_unanswered(&self) -> bool {
        self.unanswered.is_some()
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
        if self.busy || self.held || self.front_in_flight() {
            return None;
        }
        if let Some(blocks) = self.unanswered.take() {
            self.busy = true;
            self.current = Some(blocks.clone());
            return Some(Dispatch { turn: Turn::Prompt(blocks), shown: None });
        }
        let q = self.items.pop_front()?;
        Some(self.start(q))
    }

    fn start(&mut self, q: Queued) -> Dispatch {
        self.busy = true;
        self.current = Some(q.blocks.clone());
        Dispatch { turn: Turn::Prompt(q.blocks), shown: Some((q.text, q.attachments)) }
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
    fn a_held_outbox_queues_and_sends_in_order_on_release() {
        let mut o = Outbox::default();
        o.hold();
        assert!(o.submit(msg("a"), false).is_none());
        assert!(o.submit(msg("b"), true).is_none());
        assert_eq!(o.items.len(), 2);
        assert_eq!(prompt_label(o.release()).as_deref(), Some("a"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("b"));
    }

    #[test]
    fn an_unanswered_message_goes_again_first_without_a_new_bubble() {
        let mut o = Outbox::default();
        let words = || vec![ContentBlock::from("plot it".to_string())];
        o.submit(Queued::new("plot it".into(), vec![], words()), false);
        o.turn_unanswered();
        assert!(o.held && !o.busy && o.has_unanswered());
        assert!(o.submit(msg("later"), false).is_none());
        let again = o.release().unwrap();
        assert!(again.shown.is_none());
        assert!(matches!(again.turn, Turn::Prompt(blocks) if blocks == words()));
        assert!(!o.has_unanswered());
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("later"));
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
