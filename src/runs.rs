//! Runs of tool calls: the calls between two agent messages fold into one line
//! that says in plain words what they did ("Read 2 files, ran a command").

use std::ops::Range;

use agent_client_protocol::schema::v1::{ToolCallStatus, ToolKind};
use serde_json::Value;

use crate::celldiff;
use crate::session::Entry;

/// Entries a run is made of: tool calls, the thinking between them, and a
/// pending prompt (drawn above the composer, not in the transcript).
pub fn is_member(entry: &Entry) -> bool {
    matches!(entry, Entry::Tool { .. } | Entry::Thought { .. } | Entry::Permission { .. })
}

/// The run entry `ix` belongs to: the members around it, if one of them is a tool call.
pub fn run_at(entries: &[Entry], ix: usize) -> Option<Range<usize>> {
    if !is_member(entries.get(ix)?) {
        return None;
    }
    let start = entries[..ix].iter().rposition(|e| !is_member(e)).map_or(0, |i| i + 1);
    let end = entries[ix..].iter().position(|e| !is_member(e)).map_or(entries.len(), |i| ix + i);
    entries[start..end].iter().any(|e| matches!(e, Entry::Tool { .. })).then_some(start..end)
}

/// A call failed: the agent says so, or a notebook tool answered with an error.
pub fn failed(status: ToolCallStatus, title: &str, output: Option<&Value>) -> bool {
    status == ToolCallStatus::Failed
        || (celldiff::pluto_tool(title).is_some() && output.and_then(celldiff::tool_json).is_some_and(|r| r.get("error").is_some()))
}

/// One kind of work, worded as "`verb` `one`" for a single one and
/// "`verb` `many`" (with `{n}` the count) for several; `does` is the verb
/// before it happens ("edit a cell"), `verb` after ("edited a cell").
#[derive(Clone, Copy, PartialEq, Debug)]
struct Act {
    does: &'static str,
    verb: &'static str,
    one: &'static str,
    many: &'static str,
}

const fn act(does: &'static str, verb: &'static str, one: &'static str, many: &'static str) -> Act {
    Act { does, verb, one, many }
}

impl Act {
    fn object(&self, n: usize) -> String {
        if n == 1 { self.one.to_string() } else { self.many.replace("{n}", &n.to_string()) }
    }
}

/// A notebook tool call in plain words, as asked for: "edit a cell", "run 2 cells".
pub fn asked(title: &str, input: &Value) -> Option<String> {
    celldiff::pluto_tool(title)?;
    let (act, n) = act_of(title, ToolKind::Other, input);
    Some(format!("{} {}", act.does, act.object(n)))
}

const READ_FILE: Act = act("read", "read", "a file", "{n} files");
const EDIT_FILE: Act = act("edit", "edited", "a file", "{n} files");
const WRITE_FILE: Act = act("write", "wrote", "a file", "{n} files");
const DELETE_FILE: Act = act("delete", "deleted", "a file", "{n} files");
const MOVE_FILE: Act = act("move", "moved", "a file", "{n} files");
const LIST_FOLDER: Act = act("list", "listed", "a folder", "{n} folders");
const SEARCH: Act = act("run", "ran", "a search", "{n} searches");
const FETCH: Act = act("fetch", "fetched", "a page", "{n} pages");
const COMMAND: Act = act("run", "ran", "a command", "{n} commands");
const TODOS: Act = act("update", "updated", "the to-do list", "the to-do list {n} times");
const READ_CELL: Act = act("read", "read", "a cell", "{n} cells");
const READ_NOTEBOOK: Act = act("read", "read", "the notebook", "the notebook {n} times");
const VIEW_OUTPUT: Act = act("view", "viewed", "an output", "{n} outputs");
const EDIT_CELL: Act = act("edit", "edited", "a cell", "{n} cells");
const ADD_CELL: Act = act("add", "added", "a cell", "{n} cells");
const DELETE_CELL: Act = act("delete", "deleted", "a cell", "{n} cells");
const MOVE_CELL: Act = act("move", "moved", "a cell", "{n} cells");
const RUN_CELL: Act = act("run", "ran", "a cell", "{n} cells");
const RUN_CHANGED: Act = act("run", "ran", "the changed cells", "the changed cells {n} times");
const RUN_ALL: Act = act("run", "ran", "all cells", "all cells {n} times");
const SEARCH_NOTEBOOK: Act = act("search", "searched", "the notebook", "the notebook {n} times");
const LIST_NOTEBOOKS: Act = act("list", "listed", "the notebooks", "the notebooks {n} times");
const OPEN_NOTEBOOK: Act = act("open", "opened", "a notebook", "{n} notebooks");
const NEW_NOTEBOOK: Act = act("create", "created", "a notebook", "{n} notebooks");
const ALLOW_RUN: Act = act("let", "let", "the notebook run", "the notebook run");
const KEEP_ALIVE: Act = act("keep", "kept", "the notebook running", "the notebook running");
const OTHER: Act = act("use", "used", "a tool", "{n} tools");

/// What a call did, and how many of it (cells edited, say).
fn act_of(title: &str, kind: ToolKind, input: &Value) -> (Act, usize) {
    let count = |field: &str| input[field].as_array().map_or(1, |a| a.len().max(1));
    if let Some(tool) = celldiff::pluto_tool(title) {
        return match tool {
            "read_cell" => (READ_CELL, 1),
            "read_notebook_code" => (READ_NOTEBOOK, 1),
            "view_cell_output" => (VIEW_OUTPUT, 1),
            "edit_cell" => (EDIT_CELL, 1),
            "edit_cells" => (EDIT_CELL, count("cells")),
            "add_cell" => (ADD_CELL, 1),
            "delete_cell" => (DELETE_CELL, 1),
            "move_cell" => (MOVE_CELL, 1),
            "execute_cell" => (RUN_CELL, 1),
            "submit_changes" if input["cell_ids"].is_array() => (RUN_CELL, count("cell_ids")),
            "submit_changes" => (RUN_CHANGED, 1),
            "run_all_cells" => (RUN_ALL, 1),
            "search_code" => (SEARCH_NOTEBOOK, 1),
            "list_notebooks" => (LIST_NOTEBOOKS, 1),
            "open_notebook" => (OPEN_NOTEBOOK, 1),
            "new_notebook" => (NEW_NOTEBOOK, 1),
            "allow_execution" => (ALLOW_RUN, 1),
            "keep_notebook_alive" => (KEEP_ALIVE, 1),
            "list_folder" => (LIST_FOLDER, 1),
            "read_file" => (READ_FILE, 1),
            "run_shell" => (COMMAND, 1),
            _ => (OTHER, 1),
        };
    }
    let act = match kind {
        ToolKind::Read => READ_FILE,
        ToolKind::Edit if title.starts_with("Write") => WRITE_FILE,
        ToolKind::Edit => EDIT_FILE,
        ToolKind::Delete => DELETE_FILE,
        ToolKind::Move => MOVE_FILE,
        ToolKind::Search => SEARCH,
        ToolKind::Fetch if input["url"].is_string() => FETCH,
        ToolKind::Fetch => SEARCH,
        ToolKind::Execute => COMMAND,
        ToolKind::Think => TODOS,
        _ => OTHER,
    };
    (act, 1)
}

/// A run's folded line, by kind in the order each first appears:
/// "Read 2 files, listed a folder, ran a command".
pub fn summary<'a>(calls: impl IntoIterator<Item = (&'a str, ToolKind, &'a Value)>) -> String {
    let mut acts: Vec<(Act, usize)> = Vec::new();
    for (title, kind, input) in calls {
        let (act, n) = act_of(title, kind, input);
        match acts.iter_mut().find(|(a, _)| *a == act) {
            Some((_, total)) => *total += n,
            None => acts.push((act, n)),
        }
    }
    let phrases: Vec<String> = acts
        .iter()
        .map(|(act, n)| format!("{} {}", act.verb, act.object(*n)))
        .collect();
    let text = phrases.join(", ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::{run_at, summary};
    use crate::session::Entry;
    use agent_client_protocol::schema::v1::{ToolCallStatus, ToolKind};
    use serde_json::{Value, json};

    fn tool(title: &str) -> Entry {
        Entry::Tool {
            id: title.to_string().into(),
            title: title.into(),
            kind: ToolKind::Other,
            path: None,
            status: ToolCallStatus::Completed,
            input: None,
            output: None,
            diffs: Vec::new(),
            expanded: false,
            approval: None,
        }
    }

    fn thought() -> Entry {
        Entry::Thought { text: "hmm".into(), expanded: false }
    }

    #[test]
    fn runs_are_the_calls_between_messages() {
        let entries = vec![
            Entry::User { text: "go".into(), expanded: false },
            thought(),
            Entry::Agent("Looking.".into()),
            tool("a"),
            thought(),
            tool("b"),
            Entry::Agent("Done.".into()),
            tool("c"),
            Entry::Note("Allowed: execute_cell".into()),
            tool("d"),
        ];
        assert_eq!(run_at(&entries, 0), None, "a message is not in a run");
        assert_eq!(run_at(&entries, 1), None, "thinking without a call is not a run");
        assert_eq!(run_at(&entries, 3), Some(3..6));
        assert_eq!(run_at(&entries, 4), Some(3..6), "thinking between calls folds with them");
        assert_eq!(run_at(&entries, 5), Some(3..6));
        assert_eq!(run_at(&entries, 7), Some(7..8), "a note ends a run");
        assert_eq!(run_at(&entries, 9), Some(9..10), "the last run runs to the end");
        assert_eq!(run_at(&entries, 10), None);
    }

    #[test]
    fn a_run_is_summed_up_by_kind_in_plain_words() {
        let null = Value::Null;
        let cells = json!({"cells": [{"cell_id": "a"}, {"cell_id": "b"}]});
        let ids = json!({"cell_ids": ["a", "b", "c"]});
        let url = json!({"url": "https://docs.rs"});
        assert_eq!(
            summary([
                ("Read a.rs", ToolKind::Read, &null),
                ("ls", ToolKind::Execute, &null),
                ("Read b.rs", ToolKind::Read, &null),
                ("mcp__pluto__list_folder", ToolKind::Other, &null),
            ]),
            "Read 2 files, ran a command, listed a folder"
        );
        assert_eq!(
            summary([
                ("mcp__pluto__list_notebooks", ToolKind::Other, &null),
                ("mcp__pluto__read_cell", ToolKind::Other, &null),
                ("mcp__pluto__edit_cells", ToolKind::Other, &cells),
                ("mcp__pluto__execute_cell", ToolKind::Other, &null),
            ]),
            "Listed the notebooks, read a cell, edited 2 cells, ran a cell"
        );
        assert_eq!(
            summary([("mcp__pluto__submit_changes", ToolKind::Other, &ids), ("mcp__pluto__execute_cell", ToolKind::Other, &null)]),
            "Ran 4 cells"
        );
        assert_eq!(summary([("mcp__pluto__submit_changes", ToolKind::Other, &null)]), "Ran the changed cells");
        assert_eq!(
            summary([("Write a.txt", ToolKind::Edit, &null), ("Edit b.txt", ToolKind::Edit, &null), ("grep x", ToolKind::Search, &null)]),
            "Wrote a file, edited a file, ran a search"
        );
        assert_eq!(
            summary([("Fetch", ToolKind::Fetch, &url), ("Fetch", ToolKind::Fetch, &url), ("ToolSearch", ToolKind::Other, &null)]),
            "Fetched 2 pages, used a tool"
        );
        assert_eq!(
            summary([("mcp__pluto__read_notebook_code", ToolKind::Other, &null), ("mcp__pluto__read_notebook_code", ToolKind::Other, &null)]),
            "Read the notebook 2 times"
        );
    }

    #[test]
    fn notebook_calls_are_asked_for_in_plain_words() {
        let null = Value::Null;
        assert_eq!(super::asked("mcp__pluto__edit_cell", &null).as_deref(), Some("edit a cell"));
        assert_eq!(super::asked("mcp__pluto__execute_cell", &null).as_deref(), Some("run a cell"));
        assert_eq!(super::asked("mcp__pluto__submit_changes", &json!({"cell_ids": ["a", "b"]})).as_deref(), Some("run 2 cells"));
        assert_eq!(super::asked("mcp__pluto__run_all_cells", &null).as_deref(), Some("run all cells"));
        assert_eq!(super::asked("mcp__pluto__allow_execution", &null).as_deref(), Some("let the notebook run"));
        assert_eq!(super::asked("Bash", &null), None);
    }

    #[test]
    fn notebook_errors_count_as_failures() {
        let error = json!([{"type": "text", "text": "{\"error\":\"not_found\",\"message\":\"no cell\"}"}]);
        let fine = json!([{"type": "text", "text": "{\"cells\":[]}"}]);
        assert!(super::failed(ToolCallStatus::Completed, "mcp__pluto__read_cell", Some(&error)));
        assert!(!super::failed(ToolCallStatus::Completed, "mcp__pluto__read_cell", Some(&fine)));
        assert!(super::failed(ToolCallStatus::Failed, "Bash", None));
        assert!(!super::failed(ToolCallStatus::Completed, "Read", Some(&error)), "only notebook tools answer in JSON");
    }
}
