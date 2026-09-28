//! The chat's outgoing messages. Enter sends when the agent is idle and queues
//! while it works; Cmd+Enter sends now (steers the running turn). Queued
//! messages go out one per turn, in order, and stay editable until they do.
//! While Claude can't be reached (offline, signed out) the outbox is held:
//! everything queues, and a turn that failed for that reason keeps its message
//! to send again first. A message whose files are still being copied waits in
//! the queue, and so does everything sent after it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// Its files are being copied; `blocks` gets the rest of the message after.
    copying: Option<Copying>,
    /// Sent with Cmd+Enter while its files were being copied: steer once they are.
    steer: bool,
}

/// Names a message whose files are being copied, for `Outbox::copied`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Copying(u64);

impl Queued {
    pub fn new(text: String, attachments: Vec<Attachment>, blocks: Vec<ContentBlock>) -> Self {
        Self { text, attachments, blocks, in_flight: false, copying: None, steer: false }
    }

    /// A message that can't go until its files are copied.
    pub fn copying(text: String, attachments: Vec<Attachment>, blocks: Vec<ContentBlock>) -> (Self, Copying) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let ticket = Copying(NEXT.fetch_add(1, Ordering::Relaxed));
        (Self { copying: Some(ticket), ..Self::new(text, attachments, blocks) }, ticket)
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    pub fn is_copying(&self) -> bool {
        self.copying.is_some()
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
        if q.is_copying() {
            q.steer = now;
            self.items.push_back(q);
            return None;
        }
        if !self.busy && !self.held && self.items.is_empty() && self.unanswered.is_none() {
            return Some(self.start(q));
        }
        if now && self.can_steer(self.items.len()) {
            return Some(self.steer(q));
        }
        self.items.push_back(q);
        self.next()
    }

    /// The message's files are copied: the rest of its blocks, and its chips
    /// as copied (None: nothing of it is left to send). It goes when it's its turn.
    pub fn copied(&mut self, ticket: Copying, done: Option<(Vec<Attachment>, Vec<ContentBlock>)>) -> Option<Dispatch> {
        // Gone: pulled back to edit, or dropped.
        let ix = self.items.iter().position(|q| q.copying == Some(ticket))?;
        let Some((attachments, blocks)) = done else {
            self.items.remove(ix);
            return self.next();
        };
        let q = &mut self.items[ix];
        q.copying = None;
        q.attachments = attachments;
        q.blocks.extend(blocks);
        if q.steer && self.can_steer(ix) {
            let q = self.items.remove(ix).expect("found above");
            return Some(self.steer(q));
        }
        self.next()
    }

    /// A message queued behind `ahead` others can join the running turn: one
    /// SendNow at a time (its fallback needs the front slot), and never ahead
    /// of a message still being copied.
    fn can_steer(&self, ahead: usize) -> bool {
        self.busy && !self.held && !self.front_in_flight() && !self.items.iter().take(ahead).any(Queued::is_copying)
    }

    fn steer(&mut self, mut q: Queued) -> Dispatch {
        q.in_flight = true;
        let turn = Turn::SendNow(q.blocks.clone());
        self.items.push_front(q);
        Dispatch { turn, shown: None }
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
    /// a message is never both steered in and re-sent, and while the next
    /// message's files are being copied, so messages go in the order sent.
    fn next(&mut self) -> Option<Dispatch> {
        if self.busy || self.held || self.front_in_flight() {
            return None;
        }
        if let Some(blocks) = self.unanswered.take() {
            self.busy = true;
            self.current = Some(blocks.clone());
            return Some(Dispatch { turn: Turn::Prompt(blocks), shown: None });
        }
        if self.items.front()?.is_copying() {
            return None;
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

    fn copying(s: &str) -> (Queued, Copying) {
        Queued::copying(s.into(), vec![], vec![])
    }

    fn done() -> Option<(Vec<Attachment>, Vec<ContentBlock>)> {
        Some((vec![], vec![]))
    }

    #[test]
    fn a_message_sent_while_an_earlier_one_copies_waits_behind_it() {
        let mut o = Outbox::default();
        let (big, ticket) = copying("big file");
        assert!(o.submit(big, false).is_none());
        assert!(o.submit(msg("after"), false).is_none());
        assert!(o.submit(msg("urgent"), true).is_none());
        assert_eq!(o.items.iter().map(|q| q.text.as_str()).collect::<Vec<_>>(), ["big file", "after", "urgent"]);
        assert_eq!(prompt_label(o.copied(ticket, done())).as_deref(), Some("big file"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("after"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("urgent"));
    }

    #[test]
    fn a_copied_message_waits_for_the_running_turn_and_those_queued_before_it() {
        let mut o = Outbox::default();
        o.submit(msg("a"), false);
        o.submit(msg("b"), false);
        let (big, ticket) = copying("big file");
        o.submit(big, false);
        assert!(o.copied(ticket, done()).is_none());
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("b"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("big file"));
    }

    #[test]
    fn a_copying_send_now_steers_once_copied() {
        let mut o = Outbox::default();
        o.submit(msg("a"), false);
        let (big, ticket) = copying("big file");
        assert!(o.submit(big, true).is_none());
        let d = o.copied(ticket, done()).unwrap();
        assert!(matches!(d.turn, Turn::SendNow(_)));
        assert_eq!(o.steered().map(|(text, _)| text).as_deref(), Some("big file"));
    }

    #[test]
    fn a_copy_left_with_nothing_to_send_or_pulled_back_lets_the_next_go() {
        let mut o = Outbox::default();
        let (big, ticket) = copying("");
        o.submit(big, false);
        o.submit(msg("after"), false);
        assert_eq!(prompt_label(o.copied(ticket, None)).as_deref(), Some("after"));
        let (big, ticket) = copying("edit me");
        o.submit(big, false);
        assert!(o.take(0).is_some());
        assert!(o.copied(ticket, done()).is_none());
        assert!(o.items.is_empty());
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
