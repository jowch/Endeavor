//! The chat's outgoing messages. Enter sends when the agent is idle and queues
//! while it works; Cmd+Enter sends now (steers the running turn). Queued
//! messages go out one per turn, in order, and stay editable until they do.
//! While Claude can't be reached (offline, signed out) the outbox is held:
//! everything queues, and a turn that failed for that reason keeps its message
//! to send again first. A message whose files are still being copied waits in
//! the queue, and so does everything sent after it.
//!
//! A queued message being edited keeps its place: the queue waits for it if
//! its turn comes. A removed one can be put back where it was for
//! `UNDO_FOR`. After an error or a Stop the queue pauses until "Send next".

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::v1::ContentBlock;

use crate::agent::Turn;
use crate::attach::Attachment;

/// How long a removed message can be put back.
pub const UNDO_FOR: Duration = Duration::from_secs(5);

pub struct Queued {
    /// Names the message while it waits (row ids, where a removed one goes back).
    pub id: u64,
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
    /// How it reaches Claude, for its bubble.
    delivery: Delivery,
    /// Back in the composer being edited: its place waits for it.
    editing: bool,
    /// The edited message: it takes the place of the one being edited.
    edit: bool,
}

/// How a sent message reached Claude, as its bubble says.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Delivery {
    /// It started a turn of its own.
    #[default]
    Turn,
    /// Cmd+Enter put it into the running turn.
    Joined,
    /// Cmd+Enter, but the agent can't take a message mid-turn: the running
    /// turn was stopped, and this one started the next.
    AfterStop,
}

/// Names a message whose files are being copied, for `Outbox::copied`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Copying(u64);

static NEXT: AtomicU64 = AtomicU64::new(0);

impl Queued {
    pub fn new(text: String, attachments: Vec<Attachment>, blocks: Vec<ContentBlock>) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        Self { id, text, attachments, blocks, in_flight: false, copying: None, steer: false, delivery: Delivery::Turn, editing: false, edit: false }
    }

    /// A message that can't go until its files are copied.
    pub fn copying(text: String, attachments: Vec<Attachment>, blocks: Vec<ContentBlock>) -> (Self, Copying) {
        let ticket = Copying(NEXT.fetch_add(1, Ordering::Relaxed));
        (Self { copying: Some(ticket), ..Self::new(text, attachments, blocks) }, ticket)
    }

    /// The edit of the message being edited: it goes back in that one's place.
    pub fn as_edit(self, edit: bool) -> Self {
        Self { edit, ..self }
    }

    pub fn editing(&self) -> bool {
        self.editing
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    pub fn is_copying(&self) -> bool {
        self.copying.is_some()
    }
}

/// A message as the transcript shows it: the user's words, their chips, and
/// how it reached Claude.
pub struct Shown {
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub delivery: Delivery,
}

/// What to hand the agent, and the message to add to the transcript (None while
/// a SendNow's outcome is still unknown, and for a message sent again).
pub struct Dispatch {
    pub turn: Turn,
    pub shown: Option<Shown>,
}

/// A message just removed, which Undo puts back after the one it followed.
pub struct Removed {
    pub message: Queued,
    /// The message it came after; None: it was first.
    pub after: Option<u64>,
    pub at: Instant,
}

/// Why the queue waits for "Send next".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paused {
    Error,
    Stopped,
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
    pub removed: Option<Removed>,
    /// The last turn ended with an error or a Stop: nothing more goes until `send_next`.
    pub paused: Option<Paused>,
}

impl Outbox {
    /// For a session that doesn't exist yet: messages queue until `turn_ended()`
    /// is called once it's up, which sends the first of them.
    pub fn waiting() -> Self {
        Self { busy: true, ..Self::default() }
    }

    pub fn submit(&mut self, mut q: Queued, now: bool) -> Option<Dispatch> {
        if std::mem::take(&mut q.edit)
            && let Some(ix) = self.editing()
        {
            q.steer = now && q.is_copying();
            self.items[ix] = q;
            return if now && !self.items[ix].is_copying() { self.send_now(ix).or_else(|| self.next()) } else { self.next() };
        }
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

    /// The running turn failed: its message, for Try again.
    pub fn take_current(&mut self) -> Option<Vec<ContentBlock>> {
        self.current.take()
    }

    /// Try again: a failed turn's message goes again, ahead of the queue
    /// (when Claude can be reached and nothing else runs).
    pub fn send_again(&mut self, blocks: Vec<ContentBlock>) -> Option<Dispatch> {
        self.unanswered = Some(blocks);
        self.paused = None;
        self.next()
    }

    /// Claude's process stopped: the running turn is gone, and nothing goes
    /// until the session is open again (`turn_ended`). A Cmd+Enter message
    /// waiting to join the turn queues instead.
    pub fn restart(&mut self) {
        self.busy = true;
        self.current = None;
        for q in &mut self.items {
            q.in_flight = false;
        }
    }

    pub fn has_unanswered(&self) -> bool {
        self.unanswered.is_some()
    }

    /// The SendNow joined the running turn: returns it for the transcript.
    pub fn steered(&mut self) -> Option<Shown> {
        let q = self.front_in_flight().then(|| self.items.pop_front()).flatten()?;
        Some(Shown { text: q.text, attachments: q.attachments, delivery: Delivery::Joined })
    }

    /// The SendNow couldn't join, so the running turn is being stopped for it:
    /// it leads the queue, and goes as soon as the turn ends.
    pub fn stopping_for(&mut self) -> Option<Dispatch> {
        if let Some(front) = self.items.front_mut().filter(|q| q.in_flight) {
            front.delivery = Delivery::AfterStop;
        }
        self.unsent()
    }

    /// The running turn is being stopped to send the next message.
    pub fn stopping(&self) -> bool {
        self.items.front().is_some_and(|q| q.delivery == Delivery::AfterStop)
    }

    /// The SendNow couldn't join the turn: it leads the queue instead.
    pub fn unsent(&mut self) -> Option<Dispatch> {
        if let Some(front) = self.items.front_mut() {
            front.in_flight = false;
        }
        self.next()
    }

    /// Remove a queued message; returns it. In-flight ones can't be pulled.
    pub fn take(&mut self, ix: usize) -> Option<Queued> {
        if self.items.get(ix)?.in_flight {
            return None;
        }
        self.items.remove(ix)
    }

    /// The message being edited, if one is.
    pub fn editing(&self) -> Option<usize> {
        self.items.iter().position(|q| q.editing)
    }

    /// ✎: message `ix` goes into the composer and its place waits for it.
    /// Returns its words and chips. One at a time; in-flight ones can't be edited.
    pub fn begin_edit(&mut self, ix: usize) -> Option<(String, Vec<Attachment>)> {
        if self.editing().is_some() {
            return None;
        }
        let q = self.items.get_mut(ix).filter(|q| !q.in_flight)?;
        q.editing = true;
        Some((q.text.clone(), q.attachments.clone()))
    }

    /// Esc while editing: the message stays as it was, and the queue goes on.
    pub fn cancel_edit(&mut self) -> Option<Dispatch> {
        let ix = self.editing()?;
        self.items[ix].editing = false;
        self.next()
    }

    /// ✕: the message leaves the queue at once; `undo_remove` puts it back
    /// for `UNDO_FOR`. A newer removal replaces an older one's Undo.
    pub fn remove(&mut self, ix: usize, now: Instant) -> Option<Dispatch> {
        if self.items.get(ix).is_none_or(|q| q.in_flight || q.editing) {
            return None;
        }
        let after = ix.checked_sub(1).map(|i| self.items[i].id);
        let message = self.items.remove(ix)?;
        self.removed = Some(Removed { message, after, at: now });
        self.next()
    }

    /// Undo: the removed message goes back after the one it followed (first,
    /// if that one has gone).
    pub fn undo_remove(&mut self) -> Option<Dispatch> {
        let Removed { message, after, .. } = self.removed.take()?;
        let ix = after.and_then(|id| self.items.iter().position(|q| q.id == id)).map_or(0, |i| i + 1);
        self.items.insert(ix, message);
        self.next()
    }

    /// The removed message, while it can still be put back.
    pub fn removed_at(&self, now: Instant) -> Option<&Removed> {
        self.removed.as_ref().filter(|r| now.duration_since(r.at) < UNDO_FOR)
    }

    /// Where the removed message's row shows: before the item at this index.
    pub fn removed_slot(&self) -> Option<usize> {
        let r = self.removed.as_ref()?;
        Some(r.after.and_then(|id| self.items.iter().position(|q| q.id == id)).map_or(0, |i| i + 1))
    }

    pub fn forget_removed(&mut self, now: Instant) {
        if self.removed.as_ref().is_some_and(|r| now.duration_since(r.at) >= UNDO_FOR) {
            self.removed = None;
        }
    }

    /// The turn ended with an error or a Stop: what's queued waits for `send_next`.
    pub fn pause(&mut self, why: Paused) {
        self.paused = Some(why);
    }

    /// The queue's pause, while there's something for it to hold.
    pub fn paused(&self) -> Option<Paused> {
        self.paused.filter(|_| !self.items.is_empty())
    }

    /// "Send next" on a paused queue.
    pub fn send_next(&mut self) -> Option<Dispatch> {
        self.paused = None;
        self.next()
    }

    /// Whether message `ix` can go now (↑): not while Claude can't be
    /// reached, nor one being edited, sent or copied, nor while another
    /// message is joining the turn.
    pub fn can_send_now(&self, ix: usize) -> bool {
        let Some(q) = self.items.get(ix) else { return false };
        !self.held && !q.in_flight && !q.editing && !q.is_copying() && !self.front_in_flight()
    }

    /// ↑ (or ⌘⏎ on an edit): message `ix` goes now, into the running turn,
    /// or as the next turn when nothing runs (a paused queue goes on after it).
    pub fn send_now(&mut self, ix: usize) -> Option<Dispatch> {
        if !self.can_send_now(ix) {
            return None;
        }
        let q = self.items.remove(ix)?;
        self.paused = None;
        Some(if self.busy { self.steer(q) } else { self.start(q) })
    }

    /// The line over the queue (when the agent can be reached): paused, waiting
    /// for the message being edited, or how the messages go. `agent`: its name,
    /// for "Paused after you stopped Claude".
    pub fn heading(&self, agent: &str) -> Option<String> {
        let n = self.items.len();
        if n == 0 {
            return None;
        }
        Some(match self.paused() {
            Some(Paused::Error) => "Paused after an error".into(),
            Some(Paused::Stopped) => format!("Paused after you stopped {agent}"),
            None if !self.busy && self.items.front().is_some_and(|q| q.editing) => "Waiting for the message you're editing".into(),
            None if n == 1 => "Sends after this turn".into(),
            None => format!("{n} queued · one goes after each turn, in this order"),
        })
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
        if self.paused().is_some() && self.unanswered.is_none() {
            return None;
        }
        if let Some(blocks) = self.unanswered.take() {
            self.busy = true;
            self.current = Some(blocks.clone());
            return Some(Dispatch { turn: Turn::Prompt(blocks), shown: None });
        }
        if self.items.front().is_some_and(|q| q.is_copying() || q.editing) {
            return None;
        }
        let q = self.items.pop_front()?;
        Some(self.start(q))
    }

    fn start(&mut self, q: Queued) -> Dispatch {
        self.busy = true;
        self.paused = None;
        self.current = Some(q.blocks.clone());
        Dispatch { turn: Turn::Prompt(q.blocks), shown: Some(Shown { text: q.text, attachments: q.attachments, delivery: q.delivery }) }
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
            Dispatch { turn: Turn::Prompt(_), shown } => shown.map(|s| s.text),
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
        assert_eq!(o.steered().map(|s| s.text).as_deref(), Some("urgent"));
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
    fn a_send_now_says_whether_it_joined_or_stopped_the_turn() {
        let mut o = Outbox::default();
        o.submit(msg("a"), false);
        o.submit(msg("joins"), true);
        let joined = o.steered().unwrap();
        assert_eq!((joined.text.as_str(), joined.delivery), ("joins", Delivery::Joined));

        o.submit(msg("stops"), true);
        assert!(o.stopping_for().is_none());
        assert!(o.stopping());
        let next = o.turn_ended().unwrap().shown.unwrap();
        assert_eq!((next.text.as_str(), next.delivery), ("stops", Delivery::AfterStop));
        assert!(!o.stopping());

        o.submit(msg("queued"), false);
        assert_eq!(o.turn_ended().unwrap().shown.unwrap().delivery, Delivery::Turn);
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
        assert_eq!(o.steered().map(|s| s.text).as_deref(), Some("big file"));
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

    fn texts(o: &Outbox) -> Vec<&str> {
        o.items.iter().map(|q| q.text.as_str()).collect()
    }

    #[test]
    fn an_edited_message_keeps_its_place() {
        let mut o = Outbox::default();
        o.submit(msg("running"), false);
        for t in ["a", "b", "c"] {
            o.submit(msg(t), false);
        }
        assert_eq!(o.begin_edit(1), Some(("b".to_string(), vec![])));
        assert_eq!(o.begin_edit(2), None, "one edit at a time");
        assert!(o.submit(msg("later"), false).is_none());
        assert!(o.submit(msg("b, edited").as_edit(true), false).is_none());
        assert_eq!(texts(&o), ["a", "b, edited", "c", "later"]);
        assert_eq!(o.editing(), None);
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("a"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("b, edited"));
    }

    #[test]
    fn the_queue_waits_for_a_message_being_edited_and_esc_puts_it_back_unchanged() {
        let mut o = Outbox::default();
        o.submit(msg("running"), false);
        o.submit(msg("a"), false);
        o.submit(msg("b"), false);
        o.begin_edit(0);
        assert!(o.turn_ended().is_none());
        assert_eq!(o.heading("Claude").as_deref(), Some("Waiting for the message you're editing"));
        assert_eq!(prompt_label(o.cancel_edit()).as_deref(), Some("a"));
        assert_eq!(texts(&o), ["b"]);
    }

    #[test]
    fn an_edit_whose_turn_came_goes_once_it_is_back() {
        let mut o = Outbox::default();
        o.submit(msg("running"), false);
        o.submit(msg("a"), false);
        o.begin_edit(0);
        assert!(o.turn_ended().is_none());
        assert_eq!(prompt_label(o.submit(msg("a2").as_edit(true), false)).as_deref(), Some("a2"));
    }

    #[test]
    fn a_removed_message_comes_back_where_it_was() {
        let mut o = Outbox::default();
        o.submit(msg("running"), false);
        for t in ["a", "b", "c"] {
            o.submit(msg(t), false);
        }
        let t0 = Instant::now();
        assert!(o.remove(1, t0).is_none());
        assert_eq!(texts(&o), ["a", "c"]);
        assert_eq!(o.removed_slot(), Some(1));
        assert_eq!(o.heading("Claude").as_deref(), Some("2 queued · one goes after each turn, in this order"));
        assert!(o.removed_at(t0 + Duration::from_millis(4900)).is_some());
        assert!(o.removed_at(t0 + Duration::from_secs(5)).is_none());
        o.undo_remove();
        assert_eq!(texts(&o), ["a", "b", "c"]);
        assert!(o.removed.is_none());
    }

    #[test]
    fn undo_after_the_one_before_it_was_sent_puts_it_first() {
        let mut o = Outbox::default();
        o.submit(msg("running"), false);
        for t in ["a", "b", "c"] {
            o.submit(msg(t), false);
        }
        let t0 = Instant::now();
        o.remove(1, t0);
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("a"));
        assert_eq!(o.removed_slot(), Some(0));
        o.undo_remove();
        assert_eq!(texts(&o), ["b", "c"]);
        o.remove(0, t0);
        o.forget_removed(t0 + UNDO_FOR);
        assert!(o.undo_remove().is_none());
        assert_eq!(texts(&o), ["c"]);
    }

    #[test]
    fn a_paused_queue_waits_for_send_next() {
        let mut o = Outbox::default();
        o.submit(msg("running"), false);
        o.submit(msg("a"), false);
        o.submit(msg("b"), false);
        o.pause(Paused::Error);
        assert!(o.turn_ended().is_none());
        assert_eq!(o.heading("Claude").as_deref(), Some("Paused after an error"));
        assert!(o.submit(msg("c"), false).is_none());
        assert_eq!(texts(&o), ["a", "b", "c"]);
        assert_eq!(prompt_label(o.send_next()).as_deref(), Some("a"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("b"));

        o.pause(Paused::Stopped);
        assert!(o.turn_ended().is_none());
        assert_eq!(o.heading("Claude").as_deref(), Some("Paused after you stopped Claude"));
        assert_eq!(o.heading("Codex").as_deref(), Some("Paused after you stopped Codex"));
    }

    #[test]
    fn a_pause_with_nothing_queued_lets_the_next_message_go() {
        let mut o = Outbox::default();
        o.submit(msg("running"), false);
        o.pause(Paused::Stopped);
        assert!(o.turn_ended().is_none());
        assert_eq!(o.paused(), None);
        assert_eq!(prompt_label(o.submit(msg("new"), false)).as_deref(), Some("new"));
    }

    #[test]
    fn send_now_on_a_row_joins_the_turn_or_starts_one() {
        let mut o = Outbox::default();
        o.submit(msg("running"), false);
        o.submit(msg("a"), false);
        o.submit(msg("b"), false);
        let d = o.send_now(1).unwrap();
        assert!(matches!(d.turn, Turn::SendNow(_)));
        assert!(!o.can_send_now(1), "one joins at a time");
        assert_eq!(o.steered().map(|s| s.text).as_deref(), Some("b"));
        o.pause(Paused::Error);
        o.submit(msg("c"), false);
        assert!(o.turn_ended().is_none());
        assert_eq!(prompt_label(o.send_now(1)).as_deref(), Some("c"));
        assert_eq!(prompt_label(o.turn_ended()).as_deref(), Some("a"), "the queue goes on after it");
        o.hold();
        o.submit(msg("d"), false);
        assert!(!o.can_send_now(0));
    }

    #[test]
    fn the_heading_says_how_the_queue_goes() {
        let mut o = Outbox::default();
        assert_eq!(o.heading("Claude"), None);
        o.submit(msg("running"), false);
        o.submit(msg("a"), false);
        assert_eq!(o.heading("Claude").as_deref(), Some("Sends after this turn"));
        o.submit(msg("b"), false);
        o.submit(msg("c"), false);
        assert_eq!(o.heading("Claude").as_deref(), Some("3 queued · one goes after each turn, in this order"));
    }
}
