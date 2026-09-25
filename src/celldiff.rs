//! Cell edits as diffs. The runtime enforces read-before-edit, so every edit follows
//! a read of that cell in the same tool stream: remember each cell's code from
//! the agent's reads (and its own edits), and diff the next edit against it.

use std::collections::HashMap;

use serde_json::Value;

const MARKER: &str = "# ╔═╡ ";
/// Unchanged lines kept around each change when a diff is long.
const CONTEXT: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Same,
    Added,
    Removed,
}

/// One edited cell: a short label and its line diff.
pub struct CellDiff {
    pub label: String,
    pub lines: Vec<(Change, String)>,
}

/// The JSON a runtime tool returned, from an ACP `rawOutput`: the MCP content
/// array (`[{type: "text", text}]`) or, for some results, the text itself.
pub fn tool_json(raw: &Value) -> Option<Value> {
    let text = raw.get(0).and_then(|c| c["text"].as_str()).or_else(|| raw.as_str())?;
    serde_json::from_str(text).ok()
}

/// `mcp__pluto__edit_cell` -> `edit_cell`.
pub fn pluto_tool(title: &str) -> Option<&str> {
    title.strip_prefix("mcp__pluto__")
}

#[derive(Default)]
pub struct CellCodes(HashMap<String, String>);

impl CellCodes {
    /// The code last seen for a cell.
    pub fn get(&self, cell_id: &str) -> Option<&str> {
        self.0.get(cell_id).map(String::as_str)
    }

    /// Learn cells' current code from a completed read.
    pub fn observe(&mut self, tool: &str, output: &Value) {
        match tool {
            "read_cell" => {
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

    /// Diffs for an edit tool call (from its input), recording the new code.
    pub fn diff(&mut self, tool: &str, input: &Value) -> Vec<CellDiff> {
        let mut edit = |id: &str, new: &str| {
            let old = self.0.insert(id.to_owned(), new.to_owned()).unwrap_or_default();
            CellDiff { label: format!("cell {}", &id[..id.len().min(8)]), lines: line_diff(&old, new) }
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
            // The new cell's id only arrives with the result; show its code as added.
            "add_cell" => input["code"]
                .as_str()
                .map(|code| vec![CellDiff { label: "new cell".into(), lines: line_diff("", code) }])
                .unwrap_or_default(),
            "delete_cell" => input["cell_id"]
                .as_str()
                .and_then(|id| {
                    let old = self.0.remove(id)?;
                    Some(vec![CellDiff { label: format!("cell {} (deleted)", &id[..id.len().min(8)]), lines: line_diff(&old, "") }])
                })
                .unwrap_or_default(),
            _ => vec![],
        }
    }
}

/// Per-cell code from `read_notebook_code`, undoing its markdown/empty markers.
fn split_notebook_code(code: &str) -> Vec<(String, String)> {
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
        let d = codes.diff("edit_cell", &json!({ "cell_id": C1, "code": "x = 2" }));
        assert_eq!(d[0].lines, vec![(Removed, "x = 1".into()), (Added, "x = 2".into())]);
        // The edit becomes the new baseline.
        let d = codes.diff("edit_cell", &json!({ "cell_id": C1, "code": "x = 3" }));
        assert_eq!(d[0].lines[0], (Removed, "x = 2".into()));
    }

    #[test]
    fn learns_every_cell_from_read_notebook_code() {
        let code = format!("{MARKER}{C1}\nusing Plots\n\n{MARKER}{C2}\n# md:\nmd\"# Title\"");
        let mut codes = CellCodes::default();
        codes.observe("read_notebook_code", &json!({ "code": code }));
        let d = codes.diff("edit_cells", &json!({ "cells": [
            { "cell_id": C1, "code": "using Plots" },
            { "cell_id": C2, "code": "md\"# New title\"" },
        ]}));
        assert!(d[0].lines.iter().all(|(c, _)| *c == Same), "unchanged cell");
        assert_eq!(d[1].lines, vec![(Removed, "md\"# Title\"".into()), (Added, "md\"# New title\"".into())]);
    }

    #[test]
    fn reads_tool_json_from_mcp_content() {
        let raw = json!([{ "type": "text", "text": "{\"cell_id\":\"x\",\"code\":\"y\"}" }]);
        assert_eq!(tool_json(&raw).unwrap()["code"], "y");
        assert_eq!(tool_json(&json!("{\"code\":\"z\"}")).unwrap()["code"], "z");
        assert_eq!(pluto_tool("mcp__pluto__edit_cell"), Some("edit_cell"));
        assert_eq!(pluto_tool("Bash"), None);
    }
}
