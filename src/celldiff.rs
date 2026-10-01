//! Cell edits as diffs. The runtime enforces read-before-edit, so every edit follows
//! a read of that cell in the same tool stream: remember each cell's code from
//! the agent's reads (and its own edits), and diff the next edit against it.

use std::collections::HashMap;

use serde_json::Value;

const MARKER: &str = "# ╔═╡ ";
/// Unchanged lines kept around each change when a diff is long.
const CONTEXT: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Change {
    Same,
    Added,
    Removed,
}

/// One edited cell or file: a short label and its line diff, and for a
/// notebook cell the edit itself.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CellDiff {
    pub label: String,
    pub lines: Vec<(Change, String)>,
    pub edit: Option<CellEdit>,
}

/// A notebook cell's code before and after one tool call: no `before` for a
/// cell the call added, no `after` for one it deleted.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CellEdit {
    pub cell: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl CellEdit {
    fn diff(self) -> CellDiff {
        let short = &self.cell[..self.cell.len().min(8)];
        let label = match (&self.before, &self.after) {
            (None, _) => "new cell".to_owned(),
            (_, None) => format!("cell {short} (deleted)"),
            _ => format!("cell {short}"),
        };
        let lines = line_diff(self.before.as_deref().unwrap_or(""), self.after.as_deref().unwrap_or(""));
        CellDiff { label, lines, edit: Some(self) }
    }
}

/// What a turn did to a cell, all its edits taken together.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CellChange {
    New,
    Edited,
    Deleted,
}

/// A row of the end-of-turn card: a cell the turn changed, by its net change.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChangedCell {
    pub cell: String,
    /// What the cell defines, else "cell" and its id's first characters.
    pub name: String,
    pub change: CellChange,
    pub added: usize,
    pub removed: usize,
}

/// The cells a turn's edits changed, in the order first touched, each by its
/// net change: a cell edited twice is one row, and one put back as it was, or
/// added and then deleted, is none. `name` says what a cell's code defines.
pub fn net_changes<'a>(edits: impl IntoIterator<Item = &'a CellEdit>, name: impl Fn(&str) -> Option<String>) -> Vec<ChangedCell> {
    let mut cells: Vec<CellEdit> = Vec::new();
    for edit in edits {
        match cells.iter_mut().find(|c| c.cell == edit.cell) {
            Some(first) => first.after.clone_from(&edit.after),
            None => cells.push(edit.clone()),
        }
    }
    cells
        .into_iter()
        .filter_map(|CellEdit { cell, before, after }| {
            let change = match (&before, &after) {
                (None, None) => return None,
                (Some(b), Some(a)) if b == a => return None,
                (None, Some(_)) => CellChange::New,
                (Some(_), None) => CellChange::Deleted,
                (Some(_), Some(_)) => CellChange::Edited,
            };
            let (old, new) = (before.as_deref().unwrap_or(""), after.as_deref().unwrap_or(""));
            let (added, removed) = line_diff(old, new).iter().fold((0, 0), |(a, r), (c, _)| match c {
                Change::Added => (a + 1, r),
                Change::Removed => (a, r + 1),
                Change::Same => (a, r),
            });
            let name = name(after.as_deref().unwrap_or(old)).unwrap_or_else(|| format!("cell {}", &cell[..cell.len().min(8)]));
            Some(ChangedCell { cell, name, change, added, removed })
        })
        .collect()
}

/// The JSON a runtime tool returned, from an ACP `rawOutput`: the MCP content
/// array (`[{type: "text", text}]`) or, for some results, the text itself.
pub fn tool_json(raw: &Value) -> Option<Value> {
    let text = raw.get(0).and_then(|c| c["text"].as_str()).or_else(|| raw.as_str())?;
    serde_json::from_str(text).ok()
}

/// The runtime bridge's name in each session's MCP config.
pub const MCP_SERVER: &str = "notebook";
/// Claude Code's name for a bridge tool is this prefix plus the tool's name.
pub const TOOL_PREFIX: &str = "mcp__notebook__";
/// The prefix before the server was named `notebook`: past sessions replay with it.
const OLD_TOOL_PREFIX: &str = "mcp__pluto__";

/// `mcp__notebook__edit_cell` (or, in a past session, `mcp__pluto__edit_cell`) -> `edit_cell`.
pub fn notebook_tool(title: &str) -> Option<&str> {
    title.strip_prefix(TOOL_PREFIX).or_else(|| title.strip_prefix(OLD_TOOL_PREFIX))
}

#[derive(Default)]
pub struct CellCodes(HashMap<String, String>);

impl CellCodes {
    /// The code last seen for a cell.
    pub fn get(&self, cell_id: &str) -> Option<&str> {
        self.0.get(cell_id).map(String::as_str)
    }

    /// Learn cells' current code from a completed read, or from `add_cell`'s
    /// reply: it's the only one whose diff (built from its input, since the
    /// input has no cell id yet) doesn't already become the next baseline, so
    /// an edit right after it would otherwise diff against nothing and show
    /// the whole cell as added.
    pub fn observe(&mut self, tool: &str, output: &Value) {
        match tool {
            "read_cell" | "add_cell" => {
                if let (Some(id), Some(code)) = (output["cell_id"].as_str(), output["code"].as_str()) {
                    self.0.insert(id.to_owned(), code.to_owned());
                }
            }
            "read_notebook_code" => {
                for (id, code) in split_notebook_code(output["code"].as_str().unwrap_or("")) {
                    self.0.insert(id, code);
                }
            }
            _ => {}
        }
    }

    /// Diffs for a completed edit tool call, from its input (and its result,
    /// which has a new cell's id), recording the new code.
    pub fn diff(&mut self, tool: &str, input: &Value, result: &Value) -> Vec<CellDiff> {
        let mut edit = |id: &str, new: &str| {
            let old = self.0.insert(id.to_owned(), new.to_owned()).unwrap_or_default();
            CellEdit { cell: id.to_owned(), before: Some(old), after: Some(new.to_owned()) }.diff()
        };
        match tool {
            "edit_cell" => match (input["cell_id"].as_str(), input["code"].as_str()) {
                (Some(id), Some(code)) => vec![edit(id, code)],
                _ => vec![],
            },
            "edit_cells" => input["cells"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| Some(edit(c["cell_id"].as_str()?, c["code"].as_str()?)))
                .collect(),
            "add_cell" => match (input["code"].as_str(), result["cell_id"].as_str()) {
                (Some(code), Some(id)) => vec![CellEdit { cell: id.to_owned(), before: None, after: Some(code.to_owned()) }.diff()],
                (Some(code), None) => vec![CellDiff { label: "new cell".into(), lines: line_diff("", code), edit: None }],
                (None, _) => vec![],
            },
            "delete_cell" => input["cell_id"]
                .as_str()
                .and_then(|id| {
                    let old = self.0.remove(id)?;
                    Some(vec![CellEdit { cell: id.to_owned(), before: Some(old), after: None }.diff()])
                })
                .unwrap_or_default(),
            _ => vec![],
        }
    }
}

/// Per-cell code from `read_notebook_code`, undoing its markdown/empty markers.
pub fn split_notebook_code(code: &str) -> Vec<(String, String)> {
    code.split(MARKER)
        .filter_map(|block| {
            let (id, body) = block.split_once('\n').unwrap_or((block, ""));
            let body = body.strip_suffix("\n\n").unwrap_or(body);
            let body = body.strip_prefix("# md:\n").unwrap_or(body);
            let body = if body == "# (empty)" { "" } else { body };
            (id.len() == 36).then(|| (id.to_owned(), body.to_owned()))
        })
        .collect()
}

/// Line diff via longest common subsequence (cells are small, so O(n·m) is fine),
/// trimmed to `CONTEXT` unchanged lines around each change.
pub fn line_diff(old: &str, new: &str) -> Vec<(Change, String)> {
    let a: Vec<&str> = if old.is_empty() { vec![] } else { old.lines().collect() };
    let b: Vec<&str> = if new.is_empty() { vec![] } else { new.lines().collect() };
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push((Change::Same, a[i].to_owned()));
            (i, j) = (i + 1, j + 1);
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            // On ties, removals first: `- old` above `+ new`.
            out.push((Change::Removed, a[i].to_owned()));
            i += 1;
        } else {
            out.push((Change::Added, b[j].to_owned()));
            j += 1;
        }
    }
    let near_change = |k: usize| {
        let lo = k.saturating_sub(CONTEXT);
        let hi = (k + CONTEXT).min(out.len() - 1);
        out[lo..=hi].iter().any(|(c, _)| *c != Change::Same)
    };
    let keep: Vec<bool> = (0..out.len()).map(near_change).collect();
    out.into_iter().zip(keep).filter_map(|(line, k)| k.then_some(line)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use Change::*;

    const C1: &str = "11111111-2222-4333-8444-555555555555";
    const C2: &str = "66666666-7777-4888-9999-aaaaaaaaaaaa";

    #[test]
    fn diffs_lines_with_context() {
        let d = line_diff("a\nb\nc", "a\nB\nc");
        assert_eq!(d, vec![(Same, "a".into()), (Removed, "b".into()), (Added, "B".into()), (Same, "c".into())]);
        assert_eq!(line_diff("", "x"), vec![(Added, "x".into())]);
        let long: String = (0..20).map(|n| format!("l{n}\n")).collect();
        let changed = long.replace("l10\n", "L10\n");
        assert_eq!(line_diff(&long, &changed).len(), 2 * CONTEXT + 2, "trimmed to context");
    }

    #[test]
    fn diffs_an_edit_against_the_last_read() {
        let mut codes = CellCodes::default();
        codes.observe("read_cell", &json!({ "cell_id": C1, "code": "x = 1" }));
        let d = codes.diff("edit_cell", &json!({ "cell_id": C1, "code": "x = 2" }), &json!({}));
        assert_eq!(d[0].lines, vec![(Removed, "x = 1".into()), (Added, "x = 2".into())]);
        // The edit becomes the new baseline.
        let d = codes.diff("edit_cell", &json!({ "cell_id": C1, "code": "x = 3" }), &json!({}));
        assert_eq!(d[0].lines[0], (Removed, "x = 2".into()));
    }

    #[test]
    fn an_edit_to_a_cell_add_cell_made_diffs_against_its_added_code() {
        let mut codes = CellCodes::default();
        let reply = json!({ "cell_id": C1, "code": "x = 1" });
        codes.observe("add_cell", &reply);
        let d = codes.diff("add_cell", &json!({ "code": "x = 1" }), &reply);
        assert_eq!((d[0].label.as_str(), &d[0].lines), ("new cell", &vec![(Added, "x = 1".into())]));
        let d = codes.diff("edit_cell", &json!({ "cell_id": C1, "code": "x = 2" }), &json!({}));
        assert_eq!(d[0].lines, vec![(Removed, "x = 1".into()), (Added, "x = 2".into())], "not the whole cell again");
    }

    #[test]
    fn learns_every_cell_from_read_notebook_code() {
        let code = format!("{MARKER}{C1}\nusing Plots\n\n{MARKER}{C2}\n# md:\nmd\"# Title\"");
        let mut codes = CellCodes::default();
        codes.observe("read_notebook_code", &json!({ "code": code }));
        let cells = json!({ "cells": [
            { "cell_id": C1, "code": "using Plots" },
            { "cell_id": C2, "code": "md\"# New title\"" },
        ]});
        let d = codes.diff("edit_cells", &cells, &json!({}));
        assert!(d[0].lines.iter().all(|(c, _)| *c == Same), "unchanged cell");
        assert_eq!(d[1].lines, vec![(Removed, "md\"# Title\"".into()), (Added, "md\"# New title\"".into())]);
    }

    #[test]
    fn a_turn_counts_each_cell_once_by_its_net_change() {
        const C3: &str = "bbbbbbbb-7777-4888-9999-aaaaaaaaaaaa";
        const C4: &str = "cccccccc-7777-4888-9999-aaaaaaaaaaaa";
        const C5: &str = "dddddddd-7777-4888-9999-aaaaaaaaaaaa";
        let mut codes = CellCodes::default();
        let mut diffs = Vec::new();
        let mut call = |tool: &str, input: Value, result: Value| {
            codes.observe(tool, &result);
            diffs.extend(codes.diff(tool, &input, &result));
        };
        let code = format!("{MARKER}{C1}\nrate = 0.1\nk = 2\n\n{MARKER}{C3}\nplot(rate)\n\n{MARKER}{C4}\nn = 5");
        call("read_notebook_code", json!({}), json!({ "code": code }));
        call("edit_cell", json!({ "cell_id": C1, "code": "rate = 0.2\nk = 2" }), json!({}));
        call("add_cell", json!({ "code": "fit = 1" }), json!({ "cell_id": C2, "code": "fit = 1" }));
        call("edit_cells", json!({ "cells": [{ "cell_id": C1, "code": "rate = 0.3\nk = 2\nm = 1" }, { "cell_id": C4, "code": "n = 6" }] }), json!({}));
        call("edit_cell", json!({ "cell_id": C4, "code": "n = 5" }), json!({}));
        call("edit_cell", json!({ "cell_id": C2, "code": "fit = 2\nfit2 = 3" }), json!({}));
        call("delete_cell", json!({ "cell_id": C3 }), json!({}));
        call("add_cell", json!({ "code": "tmp = 0" }), json!({ "cell_id": C5, "code": "tmp = 0" }));
        call("delete_cell", json!({ "cell_id": C5 }), json!({}));
        let cells = net_changes(diffs.iter().filter_map(|d| d.edit.as_ref()), crate::session::defined_name);
        let row = |cell: &str, name: &str, change, added, removed| ChangedCell { cell: cell.into(), name: name.into(), change, added, removed };
        assert_eq!(
            cells,
            [row(C1, "rate", CellChange::Edited, 2, 1), row(C2, "fit", CellChange::New, 2, 0), row(C3, "cell bbbbbbbb", CellChange::Deleted, 0, 1)],
            "C4 is back as it was; C5 was added and deleted again"
        );
    }

    #[test]
    fn reads_tool_json_from_mcp_content() {
        let raw = json!([{ "type": "text", "text": "{\"cell_id\":\"x\",\"code\":\"y\"}" }]);
        assert_eq!(tool_json(&raw).unwrap()["code"], "y");
        assert_eq!(tool_json(&json!("{\"code\":\"z\"}")).unwrap()["code"], "z");
        assert_eq!(notebook_tool("mcp__notebook__edit_cell"), Some("edit_cell"));
        assert_eq!(notebook_tool("mcp__pluto__edit_cell"), Some("edit_cell"));
        assert_eq!(notebook_tool("Bash"), None);
    }
}
