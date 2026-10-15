//! Endeavor's own copy of each session's transcript, kept in its app support
//! folder (`transcripts/<session id>.json`) so a past session's history shows
//! the moment it opens, before the agent has replayed it. Display only: the
//! agent's copy stays the one that counts, and this one is never sent to it.
//!
//! When the agent's replay finishes, it replaces the copy (`merge`). The
//! replay carries no message ids, so the two are matched by order and text.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, EmbeddedResourceResource, PlanEntry, SessionId, SessionUpdate, TextContent, ToolCallId, ToolCallStatus, ToolKind};
use serde::{Deserialize, Serialize};

use crate::agent::SessionEvent;
use crate::attach::{self, Attachment};
use crate::celldiff::{CellDiff, ChangedCell};
use crate::hosts::Place;
use crate::outbox::Delivery;
use crate::session::{Approval, Entry, Session};

/// Longer strings in a tool call's input or output are cut to this many characters.
const STRING_MAX: usize = 4000;
/// Longer lists in a tool call's input or output keep their first this many items.
const LIST_MAX: usize = 100;
/// A picture in a message bigger than this (base64) isn't kept; the replay brings it back.
const IMAGE_MAX: usize = 150_000;

/// The note where the replay's history parts from the copy's.
pub const REPLACED: &str = "Earlier messages were replaced (compacted, or rewound outside Endeavor)";

/// One transcript entry as saved. Prompts, run-state notes and failure cards
/// aren't kept: they only mean something while the session that made them runs.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Saved {
    User {
        text: String,
        /// The message's chips, as the blocks the agent replays for them.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<ContentBlock>,
        #[serde(default, skip_serializing_if = "is_turn")]
        delivery: Delivery,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sent: Option<u64>,
    },
    Agent {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<u64>,
    },
    Tool {
        id: ToolCallId,
        title: String,
        kind: ToolKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
        status: ToolCallStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diffs: Vec<CellDiff>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval: Option<Approval>,
    },
    Thought {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        took_ms: Option<u64>,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    Note {
        text: String,
    },
    Changes {
        cells: Vec<ChangedCell>,
    },
    Reopened {
        at: u64,
    },
}

fn is_turn(delivery: &Delivery) -> bool {
    *delivery == Delivery::Turn
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    entries: Vec<serde_json::Value>,
}

fn secs(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn time(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

/// A tool call's input or output, with long strings and lists cut short.
fn trimmed(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(s) if s.chars().count() > STRING_MAX => Value::String(s.chars().take(STRING_MAX).chain(['…']).collect()),
        Value::Array(items) => Value::Array(items.iter().take(LIST_MAX).map(trimmed).collect()),
        Value::Object(fields) => Value::Object(fields.iter().map(|(k, v)| (k.clone(), trimmed(v))).collect()),
        other => other.clone(),
    }
}

/// A sent message's blocks as the agent replays them: links and text files
/// come back as text, a text file's contents after the rest
/// (claude-agent-acp's `promptToClaude`).
pub(crate) fn as_replayed(blocks: Vec<ContentBlock>) -> Vec<ContentBlock> {
    let text = |s: String| ContentBlock::Text(TextContent::new(s));
    let (mut content, mut context) = (Vec::new(), Vec::new());
    for block in blocks {
        match block {
            ContentBlock::ResourceLink(link) => content.push(text(link.uri)),
            ContentBlock::Resource(r) => {
                if let EmbeddedResourceResource::TextResourceContents(t) = r.resource {
                    content.push(text(t.uri.clone()));
                    context.push(text(format!("\n<context ref=\"{}\">\n{}\n</context>", t.uri, t.text)));
                }
            }
            other => content.push(other),
        }
    }
    content.into_iter().chain(context).collect()
}

/// A message's chips, read back from their blocks the way a replay reads them.
fn chips(blocks: Vec<ContentBlock>) -> Vec<Attachment> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut reader = Session::loading(0, SessionId::new("copy"), Place::local("/"), None, String::new());
    for block in blocks {
        reader.apply(SessionEvent::Update(SessionUpdate::UserMessageChunk(ContentChunk::new(block))));
    }
    match reader.entries.pop() {
        Some(Entry::User { attachments, .. }) => attachments,
        _ => Vec::new(),
    }
}

fn saved(entry: &Entry) -> Option<Saved> {
    Some(match entry {
        Entry::User { text, attachments, delivery, sent, .. } => {
            // Read back only as chips, which don't keep the cell links' engine.
            let blocks = as_replayed(attach::prompt_blocks("", attachments, &[], wire::backend::Backend::Pluto))
                .into_iter()
                .filter(|b| !matches!(b, ContentBlock::Image(image) if image.data.len() > IMAGE_MAX))
                .collect();
            Saved::User { text: text.to_string(), blocks, delivery: *delivery, sent: sent.map(secs) }
        }
        Entry::Agent { text, at } => Saved::Agent { text: text.clone(), at: at.map(secs) },
        Entry::Tool { id, title, kind, path, status, input, output, diffs, approval, .. } => Saved::Tool {
            id: id.clone(),
            title: title.clone(),
            kind: *kind,
            path: path.clone(),
            status: *status,
            input: input.as_ref().map(trimmed),
            output: output.as_ref().map(trimmed),
            diffs: diffs.clone(),
            approval: *approval,
        },
        Entry::Thought { text, took, .. } => Saved::Thought { text: text.clone(), took_ms: took.map(|t| t.as_millis() as u64) },
        Entry::Plan(entries) => Saved::Plan { entries: entries.clone() },
        Entry::Note(text) => Saved::Note { text: text.to_string() },
        Entry::Changes(cells) => Saved::Changes { cells: cells.clone() },
        Entry::Reopened(at) => Saved::Reopened { at: secs(*at) },
        Entry::Permission { .. } | Entry::RunState(_) | Entry::Failed(_) => return None,
    })
}

fn entry(saved: Saved) -> Entry {
    match saved {
        Saved::User { text, blocks, delivery, sent } => Entry::User { text: text.into(), expanded: false, attachments: chips(blocks), delivery, sent: sent.map(time) },
        Saved::Agent { text, at } => Entry::Agent { text, at: at.map(time) },
        Saved::Tool { id, title, kind, path, status, input, output, diffs, approval } => {
            Entry::Tool { id, title, kind, path, status, input, output, diffs, expanded: false, approval }
        }
        Saved::Thought { text, took_ms } => Entry::Thought { text, expanded: false, started: None, took: took_ms.map(Duration::from_millis) },
        Saved::Plan { entries } => Entry::Plan(entries),
        Saved::Note { text } => Entry::Note(text.into()),
        Saved::Changes { cells } => Entry::Changes(cells),
        Saved::Reopened { at } => Entry::Reopened(time(at)),
    }
}

/// The copy's file contents, or None when there's nothing worth keeping.
pub fn to_json(entries: &[Entry]) -> Option<String> {
    if !entries.iter().any(keyed) {
        return None;
    }
    let entries = entries.iter().filter_map(saved).filter_map(|s| serde_json::to_value(s).ok()).collect();
    serde_json::to_string(&File { version: 1, entries }).ok()
}

/// A copy read back. Entries this version doesn't know are left out.
pub fn from_json(json: &str) -> Option<Vec<Entry>> {
    let file: File = serde_json::from_str(json).ok()?;
    let entries: Vec<Entry> = file.entries.into_iter().filter_map(|v| serde_json::from_value::<Saved>(v).ok()).map(entry).collect();
    entries.iter().any(keyed).then_some(entries)
}

/// Where a session's copy lives; None for an id that isn't a plain file name.
fn file(id: &str) -> Option<PathBuf> {
    let plain = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    plain.then(|| crate::app_file(&format!("transcripts/{id}.json"))).flatten()
}

pub fn load(id: &str) -> Option<Vec<Entry>> {
    from_json(&std::fs::read_to_string(file(id)?).ok()?)
}

pub fn save(id: &str, entries: &[Entry]) {
    let (Some(path), Some(json)) = (file(id), to_json(entries)) else { return };
    // ponytail: best effort; without a copy the session opens on its summary as before.
    let _ = crate::write_atomic(&path, json.as_bytes());
}

pub fn delete(id: &str) {
    if let Some(path) = file(id) {
        let _ = std::fs::remove_file(path);
    }
}

/// The entries a replay brings back, to match on: messages and tool calls.
/// Thoughts, plans, notes and cards are carried along but not matched.
fn keyed(entry: &Entry) -> bool {
    matches!(entry, Entry::User { .. } | Entry::Agent { .. } | Entry::Tool { .. })
}

/// The same message or call, by its text (a call by its id or its title).
fn same(a: &Entry, b: &Entry) -> bool {
    match (a, b) {
        (Entry::User { text: x, .. }, Entry::User { text: y, .. }) => x.trim() == y.trim(),
        (Entry::Agent { text: x, .. }, Entry::Agent { text: y, .. }) => x.trim() == y.trim(),
        (Entry::Tool { id: i, title: t, .. }, Entry::Tool { id: j, title: u, .. }) => i == j || t == u,
        _ => false,
    }
}

/// A matched entry takes what the replay knows better: a call's whole input
/// and output, a message's chips with their pictures. What only Endeavor
/// knows stays: when a message was sent, how a prompt was answered, what's
/// open, and a notebook call's result the runtime gave where the agent's
/// replay has none (Cursor replays `{"success": true}`).
fn carry(copy: &mut Entry, replay: Entry) {
    match replay {
        Entry::Tool { id, title, kind, path, status, input, output, diffs, approval, .. } => {
            if let Entry::Tool { id: i, title: t, kind: k, path: p, status: s, input: n, output: o, diffs: d, approval: a, .. } = copy {
                let result = |output: &Option<serde_json::Value>| output.as_ref().and_then(crate::celldiff::tool_json).is_some();
                if result(o) && !result(&output) {
                    (*i, *t, *k, *p, *n) = (id, title, kind, path, input);
                } else {
                    (*i, *t, *k, *p, *s, *n, *o, *d) = (id, title, kind, path, status, input, output, diffs);
                }
                *a = a.or(approval);
            }
        }
        Entry::User { attachments, .. } => {
            if let Entry::User { attachments: mine, .. } = copy {
                *mine = attachments;
            }
        }
        _ => {}
    }
}

/// How the replay compared with the copy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Swap {
    /// Every entry matched: nothing moves.
    Same,
    /// The replay matched everything and added this many messages after it.
    Added { messages: usize },
    /// Earlier entries differ (compacted, or rewound, outside Endeavor).
    Changed,
}

#[derive(Debug, PartialEq)]
pub struct Merged {
    pub swap: Swap,
    /// Where the entry that was at the top of the view is now; `found`: it's
    /// still there (else this is where the thread parts, and the view goes there).
    pub anchor: usize,
    pub found: bool,
}

/// Replace the copy (shown, with `top` the entry at the top of the view)
/// with the agent's replay, keeping the copy's entries wherever they match
/// so the view stays where it was.
pub fn merge(copy: &mut Vec<Entry>, replay: Vec<Entry>, top: usize) -> Merged {
    let top = top.min(copy.len().saturating_sub(1));
    let ck: Vec<usize> = (0..copy.len()).filter(|&i| keyed(&copy[i])).collect();
    let rk: Vec<usize> = (0..replay.len()).filter(|&i| keyed(&replay[i])).collect();
    let matched = ck.iter().zip(&rk).take_while(|(c, r)| same(&copy[**c], &replay[**r])).count();
    let after = |keys: &[usize]| if matched == 0 { 0 } else { keys[matched - 1] + 1 };
    let (keep, from) = (after(&ck), after(&rk));

    if matched == ck.len() {
        let mut replay: Vec<Option<Entry>> = replay.into_iter().map(Some).collect();
        for (c, r) in ck.iter().zip(&rk) {
            carry(&mut copy[*c], replay[*r].take().expect("each replayed entry is taken once"));
        }
        // What the copy's last turn ended with (its card, its notes) the copy has already.
        let first_new = rk.get(matched).copied().unwrap_or(replay.len());
        let added: Vec<Entry> = replay.drain(first_new.max(from)..).flatten().collect();
        let swap = if added.is_empty() {
            Swap::Same
        } else {
            Swap::Added { messages: added.iter().filter(|e| matches!(e, Entry::User { .. } | Entry::Agent { .. })).count() }
        };
        copy.extend(added);
        return Merged { swap, anchor: top, found: true };
    }

    let anchor = if top < keep {
        Some((top, true))
    } else {
        ck.iter()
            .find(|&&i| i >= top)
            .and_then(|&i| replay[from..].iter().position(|r| same(&copy[i], r)).map(|r| (keep + 1 + r, i == top)))
    };
    let (anchor, found) = anchor.unwrap_or((keep, false));
    let mut replay: Vec<Option<Entry>> = replay.into_iter().map(Some).collect();
    for (c, r) in ck.iter().zip(&rk).take(matched) {
        carry(&mut copy[*c], replay[*r].take().expect("each replayed entry is taken once"));
    }
    copy.truncate(keep);
    copy.push(Entry::Note(REPLACED.into()));
    copy.extend(replay.drain(from..).flatten());
    Merged { swap: Swap::Changed, anchor, found }
}

/// What waits below a view the replay left in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Below {
    New(usize),
    Changed,
}

/// The floating button above the composer: its words, or None when it
/// doesn't show. It shows while a replay has left something below the view
/// (`below`), or the user has scrolled up from the end (`following` stopped,
/// `at_end` false; None while the list can't tell).
pub fn pill(following: bool, at_end: Option<bool>, below: Option<Below>) -> Option<String> {
    let shows = below.is_some() || (!following && at_end == Some(false));
    shows.then(|| match below {
        Some(Below::New(1)) => "1 new message ↓".to_owned(),
        Some(Below::New(n)) if n > 1 => format!("{n} new messages ↓"),
        _ => "Jump to latest ↓".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]`.
    use super::{Below, Merged, REPLACED, Swap, from_json, merge, pill, to_json};
    use crate::attach::{Attachment, Quote, Quoted};
    use crate::celldiff::{CellChange, ChangedCell};
    use crate::outbox::Delivery;
    use crate::session::{Approval, Entry};
    use agent_client_protocol::schema::v1::{ToolCallId, ToolCallStatus, ToolKind};
    use serde_json::json;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn user(text: &str) -> Entry {
        Entry::User { text: text.to_owned().into(), expanded: false, attachments: Vec::new(), delivery: Delivery::Turn, sent: None }
    }

    fn agent(text: &str) -> Entry {
        Entry::Agent { text: text.into(), at: None }
    }

    fn tool(id: &str, title: &str) -> Entry {
        Entry::Tool {
            id: ToolCallId::new(id),
            title: title.into(),
            kind: ToolKind::Read,
            path: None,
            status: ToolCallStatus::Completed,
            input: None,
            output: None,
            diffs: Vec::new(),
            expanded: false,
            approval: None,
        }
    }

    /// Each entry in a word or two, to compare against.
    fn words(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::User { text, .. } => format!("you: {text}"),
                Entry::Agent { text, .. } => format!("claude: {text}"),
                Entry::Tool { title, .. } => format!("tool: {title}"),
                Entry::Note(text) => format!("note: {text}"),
                Entry::Changes(_) => "changes".into(),
                Entry::Thought { .. } => "thought".into(),
                Entry::Reopened(_) => "reopened".into(),
                _ => "other".into(),
            })
            .collect()
    }

    #[test]
    fn the_copy_reads_back_as_it_was_saved() {
        let at = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let quote = Attachment::Quote(Quote { from: Quoted::Reply { text: "the rate".into(), at: Some("14:02".into()) }, comment: "why?".into() });
        let saved = vec![
            Entry::User { text: "Fit the decay".into(), expanded: true, attachments: vec![quote.clone()], delivery: Delivery::Joined, sent: Some(at) },
            Entry::Thought { text: "Use a log fit".into(), expanded: false, started: None, took: Some(Duration::from_millis(2500)) },
            Entry::Tool {
                id: ToolCallId::new("t1"),
                title: "mcp__pluto__edit_cell".into(),
                kind: ToolKind::Edit,
                path: None,
                status: ToolCallStatus::Completed,
                input: Some(json!({"code": "x".repeat(5000), "lines": (0..150).collect::<Vec<_>>()})),
                output: Some(json!("ok")),
                diffs: Vec::new(),
                expanded: true,
                approval: Some(Approval::ForSession),
            },
            Entry::Agent { text: "The rate is 0.09.".into(), at: Some(at) },
            Entry::Changes(vec![ChangedCell { cell: "c1".into(), name: "rates".into(), change: CellChange::Edited, added: 1, removed: 1 }]),
            Entry::Note("You stopped Claude".into()),
            Entry::Reopened(at),
            Entry::RunState(Vec::new()),
        ];
        let back = from_json(&to_json(&saved).unwrap()).unwrap();
        assert_eq!(words(&back), ["you: Fit the decay", "thought", "tool: mcp__pluto__edit_cell", "claude: The rate is 0.09.", "changes", "note: You stopped Claude", "reopened"]);
        let Entry::User { attachments, delivery, sent, .. } = &back[0] else { panic!() };
        assert_eq!((attachments.as_slice(), *delivery, *sent), ([quote].as_slice(), Delivery::Joined, Some(at)));
        let Entry::Thought { took, .. } = &back[1] else { panic!() };
        assert_eq!(*took, Some(Duration::from_millis(2500)));
        let Entry::Tool { input, approval, expanded, .. } = &back[2] else { panic!() };
        let input = input.as_ref().unwrap();
        assert_eq!(input["code"].as_str().unwrap().chars().count(), 4001, "long strings are cut, with an ellipsis");
        assert_eq!(input["lines"].as_array().unwrap().len(), 100);
        assert_eq!((*approval, *expanded), (Some(Approval::ForSession), false));
        let Entry::Changes(cells) = &back[4] else { panic!() };
        assert_eq!(cells[0].name, "rates");
        let Entry::Reopened(when) = &back[6] else { panic!() };
        assert_eq!(*when, at);
    }

    #[test]
    fn a_copy_with_no_messages_isnt_kept_and_unknown_entries_are_skipped() {
        assert_eq!(to_json(&[Entry::Note("Notebook moved".into())]), None);
        assert!(from_json("not json").is_none());
        let back = from_json(r#"{"version": 2, "entries": [{"type": "hologram"}, {"type": "agent", "text": "Hi"}]}"#).unwrap();
        assert_eq!(words(&back), ["claude: Hi"]);
    }

    #[test]
    fn a_replay_that_matches_changes_nothing_and_keeps_what_only_endeavor_knew() {
        let sent = SystemTime::now();
        let mut copy = vec![
            Entry::User { text: "Fit the decay".into(), expanded: false, attachments: Vec::new(), delivery: Delivery::Turn, sent: Some(sent) },
            tool("t1", "Read data.csv"),
            agent("Done."),
            Entry::Changes(Vec::new()),
            Entry::Note("You edited `rates`".into()),
        ];
        let replay = vec![user(" Fit the decay"), tool("t1", "Read data.csv"), Entry::Thought { text: "".into(), expanded: false, started: None, took: None }, agent("Done.\n"), Entry::Changes(Vec::new())];
        let merged = merge(&mut copy, replay, 2);
        assert_eq!(merged, Merged { swap: Swap::Same, anchor: 2, found: true });
        assert_eq!(words(&copy), ["you: Fit the decay", "tool: Read data.csv", "claude: Done.", "changes", "note: You edited `rates`"]);
        let Entry::User { sent: kept, .. } = &copy[0] else { panic!() };
        assert_eq!(*kept, Some(sent));
    }

    #[test]
    fn a_replay_that_only_adds_at_the_end_appends_and_counts_the_messages() {
        let mut copy = vec![user("Fit the decay"), agent("Done."), Entry::Changes(Vec::new())];
        let replay = vec![user("Fit the decay"), agent("Done."), Entry::Changes(Vec::new()), user("Now plot it"), tool("t2", "Edit plot"), agent("Plotted."), user("Thanks")];
        let merged = merge(&mut copy, replay, 0);
        assert_eq!(merged, Merged { swap: Swap::Added { messages: 3 }, anchor: 0, found: true });
        assert_eq!(words(&copy), ["you: Fit the decay", "claude: Done.", "changes", "you: Now plot it", "tool: Edit plot", "claude: Plotted.", "you: Thanks"]);
    }

    #[test]
    fn a_matched_call_takes_the_replays_whole_output_but_keeps_its_answer() {
        let mut copy = vec![user("Run it"), tool("t1", "Run cell")];
        if let Entry::Tool { approval, expanded, .. } = &mut copy[1] {
            *approval = Some(Approval::Allowed);
            *expanded = true;
        }
        let mut replayed = tool("t1", "Run cell");
        if let Entry::Tool { output, .. } = &mut replayed {
            *output = Some(json!("the whole output"));
        }
        merge(&mut copy, vec![user("Run it"), replayed], 0);
        let Entry::Tool { output, approval, expanded, .. } = &copy[1] else { panic!() };
        assert_eq!((output.clone(), *approval, *expanded), (Some(json!("the whole output")), Some(Approval::Allowed), true));
    }

    #[test]
    fn a_replay_without_the_result_keeps_the_one_the_runtime_gave() {
        let result = json!([{ "type": "text", "text": "{\"applied\":true,\"cell_id\":\"a\"}" }]);
        let mut copy = vec![user("Fix it"), tool("t1", "mcp__notebook__edit_cell")];
        if let Entry::Tool { output, status, .. } = &mut copy[1] {
            (*output, *status) = (Some(result.clone()), ToolCallStatus::Failed);
        }
        let mut replayed = tool("t1", "mcp__notebook__edit_cell");
        if let Entry::Tool { output, status, .. } = &mut replayed {
            (*output, *status) = (Some(json!({ "success": true })), ToolCallStatus::Completed);
        }
        merge(&mut copy, vec![user("Fix it"), replayed], 0);
        let Entry::Tool { output, status, .. } = &copy[1] else { panic!() };
        assert_eq!((output.clone(), *status), (Some(result), ToolCallStatus::Failed));
    }

    #[test]
    fn when_earlier_messages_differ_the_thread_updates_around_the_view() {
        // Compacted: the first turn is gone from the replay; the view was on the second reply.
        let mut copy = vec![user("Load the data"), agent("Loaded."), user("Fit the decay"), agent("The rate is 0.09."), Entry::Note("You edited `rates`".into())];
        let replay = vec![user("Summary of earlier work"), user("Fit the decay"), agent("The rate is 0.09."), user("Plot it")];
        let merged = merge(&mut copy, replay, 3);
        assert_eq!(merged, Merged { swap: Swap::Changed, anchor: 3, found: true });
        assert_eq!(
            words(&copy),
            [format!("note: {REPLACED}").as_str(), "you: Summary of earlier work", "you: Fit the decay", "claude: The rate is 0.09.", "you: Plot it"]
        );
    }

    #[test]
    fn a_rewind_keeps_what_still_matches_and_the_view_goes_where_the_thread_parts() {
        let mut copy = vec![user("Load the data"), agent("Loaded."), user("Fit the decay"), agent("The rate is 0.09.")];
        let replay = vec![user("Load the data"), agent("Loaded."), user("Fit a line instead"), agent("The slope is 2.")];
        // The view's top was on the reply the rewind removed.
        let merged = merge(&mut copy, replay, 3);
        assert_eq!(merged, Merged { swap: Swap::Changed, anchor: 2, found: false });
        assert_eq!(words(&copy), ["you: Load the data", "claude: Loaded.", format!("note: {REPLACED}").as_str(), "you: Fit a line instead", "claude: The slope is 2."]);
        // A view above where they part stays on its own entry.
        let mut copy = vec![user("Load the data"), agent("Loaded."), user("Fit the decay")];
        let merged = merge(&mut copy, vec![user("Load the data"), agent("Loaded."), user("Fit a line")], 1);
        assert_eq!(merged, Merged { swap: Swap::Changed, anchor: 1, found: true });
    }

    #[test]
    fn the_button_shows_when_the_view_has_left_the_end() {
        assert_eq!(pill(true, Some(true), None), None);
        assert_eq!(pill(true, Some(true), Some(Below::New(3))).as_deref(), Some("3 new messages ↓"), "until the user goes to them");
        assert_eq!(pill(false, Some(false), None).as_deref(), Some("Jump to latest ↓"));
        assert_eq!(pill(false, None, Some(Below::New(3))).as_deref(), Some("3 new messages ↓"));
        assert_eq!(pill(false, Some(false), Some(Below::New(1))).as_deref(), Some("1 new message ↓"));
        assert_eq!(pill(false, Some(false), Some(Below::New(0))).as_deref(), Some("Jump to latest ↓"));
        assert_eq!(pill(false, None, Some(Below::Changed)).as_deref(), Some("Jump to latest ↓"));
        assert_eq!(pill(false, None, None), None, "a list too short to scroll");
    }
}
