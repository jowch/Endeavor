//! What an opened tool call shows: its input as what it means (the command,
//! the file and lines read, a search's pattern and folder) and its output as
//! text or the key facts of a notebook tool's answer. Never raw JSON.

use std::path::Path;

use agent_client_protocol::schema::v1::ToolKind;
use serde_json::Value;

use crate::celldiff;

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// A command or code, in a mono block.
    Code(String),
    /// Label and value rows.
    Fields(Vec<(String, String)>),
    /// A sentence, with `backticked` names in mono.
    Line(String),
    /// Output, as plain mono text.
    Text(String),
    /// A failure, in the error colour.
    Error(String),
    /// A file's lines with their numbers.
    Numbered(Vec<(usize, String)>),
}

#[derive(Debug, Default, PartialEq)]
pub struct Details {
    pub input: Vec<Part>,
    pub output: Vec<Part>,
}

/// Long outputs are cut to this much.
const MAX_CHARS: usize = 20_000;
const MAX_LINES: usize = 2_000;

/// An opened call. `diffed`: its edits are shown as diffs, which then stand for
/// its input. `name` names a cell by its id, when the transcript has seen it.
pub fn details(
    title: &str,
    kind: ToolKind,
    path: Option<&Path>,
    input: &Value,
    output: Option<&Value>,
    failed: bool,
    diffed: bool,
    name: &dyn Fn(&str) -> Option<String>,
) -> Details {
    let tool = celldiff::pluto_tool(title);
    let input_parts = match tool {
        Some(tool) => notebook_input(tool, input, diffed, name),
        None => call_input(kind, path, input, diffed),
    };
    let output_parts = match output {
        None => Vec::new(),
        Some(output) => match tool {
            Some(tool) => notebook_output(tool, output, name),
            None => call_output(kind, output, failed, diffed),
        },
    };
    Details { input: input_parts, output: output_parts }
}

fn field<'a>(input: &'a Value, name: &str) -> Option<&'a str> {
    input[name].as_str().filter(|s| !s.trim().is_empty())
}

/// "lines 10–59" from a read's first line and count.
fn line_range(first: Option<u64>, count: Option<u64>) -> Option<String> {
    match (first, count) {
        (None, None) => None,
        (Some(first), Some(count)) => Some(format!("{first}–{}", first + count.max(1) - 1)),
        (Some(first), None) => Some(format!("from {first}")),
        (None, Some(count)) => Some(format!("1–{count}")),
    }
}

fn call_input(kind: ToolKind, path: Option<&Path>, input: &Value, diffed: bool) -> Vec<Part> {
    let file = path.map(|p| p.display().to_string()).or_else(|| ["file_path", "notebook_path", "path"].iter().find_map(|f| field(input, f)).map(str::to_owned));
    match kind {
        ToolKind::Execute if field(input, "command").is_some() => vec![Part::Code(input["command"].as_str().unwrap_or_default().to_string())],
        ToolKind::Read if file.is_some() => {
            let mut rows = vec![("file".to_string(), file.unwrap_or_default())];
            rows.extend(line_range(input["offset"].as_u64(), input["limit"].as_u64()).map(|r| ("lines".to_string(), r)));
            vec![Part::Fields(rows)]
        }
        ToolKind::Edit if diffed => Vec::new(),
        ToolKind::Search if field(input, "pattern").is_some() => {
            let mut rows = vec![("pattern".to_string(), input["pattern"].as_str().unwrap_or_default().to_string())];
            rows.push(("in".to_string(), field(input, "path").unwrap_or("the session folder").to_string()));
            for (key, label) in [("glob", "files"), ("type", "type")] {
                if let Some(value) = field(input, key) {
                    rows.push((label.to_string(), value.to_string()));
                }
            }
            vec![Part::Fields(rows)]
        }
        _ => {
            let rows = facts(input, &[]);
            if rows.is_empty() { Vec::new() } else { vec![Part::Fields(rows)] }
        }
    }
}

fn notebook_input(tool: &str, input: &Value, diffed: bool, name: &dyn Fn(&str) -> Option<String>) -> Vec<Part> {
    match tool {
        "run_shell" => {
            let mut parts = vec![Part::Code(input["command"].as_str().unwrap_or_default().to_string())];
            if let Some(cwd) = field(input, "cwd") {
                parts.push(Part::Fields(vec![("in".into(), cwd.into())]));
            }
            parts
        }
        "read_file" => {
            let mut rows = vec![("file".to_string(), field(input, "path").unwrap_or("").to_string())];
            rows.extend(line_range(input["offset"].as_u64(), input["limit"].as_u64()).map(|r| ("lines".to_string(), r)));
            vec![Part::Fields(rows)]
        }
        "list_folder" => vec![Part::Fields(vec![("folder".into(), field(input, "path").unwrap_or("the session folder").into())])],
        _ if diffed => Vec::new(),
        _ => {
            let mut rows = Vec::new();
            for (key, value) in input.as_object().into_iter().flatten() {
                match key.as_str() {
                    "notebook_id" => {}
                    "cell_id" | "after_cell_id" => {
                        let id = value.as_str().unwrap_or_default();
                        let label = if key == "cell_id" { "cell" } else { "after" };
                        rows.push((label.to_string(), cell_label(id, name)));
                    }
                    "cell_ids" => {
                        let ids: Vec<&str> = value.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                        rows.push(("cells".to_string(), cells_label(&ids, name)));
                    }
                    _ => rows.extend(fact(key, value)),
                }
            }
            if rows.is_empty() { Vec::new() } else { vec![Part::Fields(rows)] }
        }
    }
}

/// A cell by what it defines, else its id's first characters.
fn cell_label(id: &str, name: &dyn Fn(&str) -> Option<String>) -> String {
    match name(id) {
        Some(n) => format!("`{n}`"),
        None => format!("cell {}", id.get(..8).unwrap_or(id)),
    }
}

fn cells_label(ids: &[&str], name: &dyn Fn(&str) -> Option<String>) -> String {
    const LISTED: usize = 5;
    let mut names: Vec<String> = ids.iter().take(LISTED).map(|id| cell_label(id, name)).collect();
    if ids.len() > LISTED {
        names.push(format!("{} more", ids.len() - LISTED));
    }
    names.join(", ")
}

/// A JSON object's fields as label and value rows, without braces; `skip`
/// leaves fields out.
fn facts(value: &Value, skip: &[&str]) -> Vec<(String, String)> {
    match value.as_object() {
        Some(fields) => fields.iter().filter(|(k, _)| !skip.contains(&k.as_str())).filter_map(|(k, v)| fact(k, v)).collect(),
        None => compact(value).map(|v| vec![("value".to_string(), v)]).unwrap_or_default(),
    }
}

fn fact(key: &str, value: &Value) -> Option<(String, String)> {
    Some((key.replace('_', " "), compact(value)?))
}

/// A value on one line: scalars as themselves, lists joined, objects as
/// "key: value" pairs; nothing for null or empty.
fn compact(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(if *b { "yes" } else { "no" }.into()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(items) if items.is_empty() => None,
        Value::Array(items) if items.iter().all(|i| !i.is_object() && !i.is_array()) => {
            Some(items.iter().filter_map(compact).collect::<Vec<_>>().join(", "))
        }
        Value::Array(items) => Some(if items.len() == 1 { "1 item".into() } else { format!("{} items", items.len()) }),
        Value::Object(fields) => {
            let pairs: Vec<String> = fields
                .iter()
                .filter_map(|(k, v)| {
                    let v = match v {
                        Value::Object(_) => "…".to_string(),
                        _ => compact(v)?,
                    };
                    Some(format!("{}: {v}", k.replace('_', " ")))
                })
                .collect();
            (!pairs.is_empty()).then(|| pairs.join(", "))
        }
    }
}

/// A tool result as text: the text itself, or its content blocks' texts
/// (a tool reference by its name).
pub fn plain_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item["type"].as_str() {
                Some("text") => item["text"].as_str().map(str::to_owned),
                Some("tool_reference") => item["tool_name"].as_str().map(str::to_owned),
                Some("image") => Some("[image]".into()),
                _ => compact(item),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => compact(other).unwrap_or_default(),
    }
}

fn cut(text: &str) -> String {
    match text.char_indices().nth(MAX_CHARS) {
        Some((at, _)) => format!("{}\n…", &text[..at]),
        None => text.to_string(),
    }
}

/// Text without the agent's `<system-reminder>` blocks.
fn without_reminders(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<system-reminder>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</system-reminder>") {
            Some(end) => rest = &rest[start + end + "</system-reminder>".len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim_end().to_string()
}

/// A file read back with its line numbers ("    12\tcode" or "12→code"), or
/// None when the text isn't numbered.
fn numbered(text: &str) -> Option<Vec<(usize, String)>> {
    let mut lines = Vec::new();
    for line in text.lines().take(MAX_LINES) {
        let trimmed = line.trim_start();
        let digits = trimmed.len() - trimmed.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return None;
        }
        let rest = &trimmed[digits..];
        let code = rest.strip_prefix('\t').or_else(|| rest.strip_prefix('→'))?;
        lines.push((trimmed[..digits].parse().ok()?, code.to_string()));
    }
    (!lines.is_empty()).then_some(lines)
}

/// A failure's text: "Exit code 2" on its own line, in the error colour, then
/// the rest as output.
fn failure(text: &str) -> Vec<Part> {
    let text = without_reminders(text);
    let mut lines = text.splitn(2, '\n');
    let first = lines.next().unwrap_or("").trim();
    let rest = lines.next().unwrap_or("").trim_end();
    let is_exit = first.strip_prefix("Exit code ").is_some_and(|n| n.trim().parse::<i64>().is_ok());
    match (is_exit, rest.trim().is_empty()) {
        (true, true) => vec![Part::Error(first.into())],
        (true, false) => vec![Part::Error(first.into()), Part::Text(cut(rest))],
        (false, _) => vec![Part::Error(cut(text.trim()))],
    }
}

fn call_output(kind: ToolKind, output: &Value, failed: bool, diffed: bool) -> Vec<Part> {
    let text = plain_text(output);
    if failed {
        return if text.trim().is_empty() { Vec::new() } else { failure(&text) };
    }
    // The diff says it; "The file … has been updated" adds nothing.
    if diffed {
        return Vec::new();
    }
    let text = without_reminders(&text);
    if text.trim().is_empty() {
        return Vec::new();
    }
    if kind == ToolKind::Read
        && let Some(lines) = numbered(&text)
    {
        return vec![Part::Numbered(lines)];
    }
    vec![Part::Text(cut(&text))]
}

/// A notebook tool's answer as its key facts.
fn notebook_output(tool: &str, output: &Value, name: &dyn Fn(&str) -> Option<String>) -> Vec<Part> {
    let Some(json) = celldiff::tool_json(output) else {
        let text = plain_text(output);
        return if text.trim().is_empty() { Vec::new() } else { vec![Part::Text(cut(&text))] };
    };
    if let Some(error) = json["error"].as_str() {
        let message = json["message"].as_str().filter(|m| !m.is_empty()).unwrap_or(error);
        return vec![Part::Error(message.to_string())];
    }
    // A cell's name from the answer's own code, else from what the transcript has seen.
    let named = |id: &str| match (json["cell_id"].as_str() == Some(id)).then(|| json["code"].as_str()).flatten().and_then(crate::session::defined_name) {
        Some(n) => format!("`{n}`"),
        None => cell_label(id, name),
    };
    match tool {
        "run_shell" => {
            let mut parts = Vec::new();
            for stream in ["stdout", "stderr"] {
                if let Some(text) = field(&json, stream) {
                    parts.push(Part::Text(cut(text.trim_end())));
                }
            }
            match json["exit_code"].as_i64() {
                Some(0) => {}
                Some(code) => parts.push(Part::Error(format!("Exit code {code}"))),
                None if json["timed_out"] != true => parts.push(Part::Error("Stopped by a signal".into())),
                None => {}
            }
            if json["timed_out"] == true {
                parts.push(Part::Error("Timed out".into()));
            }
            parts
        }
        "read_file" => {
            let mut parts: Vec<Part> = field(&json, "text").and_then(numbered).map(Part::Numbered).into_iter().collect();
            if json["truncated"] == true {
                let (start, end, total) = (json["start_line"].as_u64().unwrap_or(1), json["end_line"].as_u64().unwrap_or(0), json["total_lines"].as_u64().unwrap_or(0));
                parts.push(Part::Line(format!("Lines {start}–{end} of {total}")));
            }
            parts
        }
        "list_folder" => {
            let entries: Vec<String> = json["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| {
                    let name = e["name"].as_str().unwrap_or_default();
                    if e["kind"] == "dir" { format!("{name}/") } else { name.to_string() }
                })
                .collect();
            let mut parts = vec![Part::Text(entries.join("\n"))];
            if json["truncated"] == true {
                parts.push(Part::Line(format!("{} of {} shown", entries.len(), json["total"].as_u64().unwrap_or(0))));
            }
            parts
        }
        "list_notebooks" => {
            let rows: Vec<String> = json
                .as_array()
                .into_iter()
                .flatten()
                .map(|nb| {
                    let path = nb["path"].as_str().unwrap_or_default();
                    let cells = nb["cell_count"].as_u64().unwrap_or(0);
                    let state = if nb["execution_allowed"] == false { " · safe preview" } else { "" };
                    format!("{path} · {cells} cells{state}")
                })
                .collect();
            if rows.is_empty() { vec![Part::Line("No notebooks open".into())] } else { vec![Part::Text(rows.join("\n"))] }
        }
        "read_notebook_code" => {
            let n = json["cell_ids"].as_array().map_or(0, Vec::len);
            vec![Part::Line(format!("{n} cells")), Part::Code(cut(json["code"].as_str().unwrap_or_default()))]
        }
        _ if json.get("mutation").is_some() => receipt(&json, &named),
        _ if json["code"].is_string() && json["cell_id"].is_string() => {
            let mut parts = vec![Part::Code(cut(json["code"].as_str().unwrap_or_default()))];
            parts.extend(cell_output(&json));
            parts
        }
        _ => {
            let rows = facts(&json, &["notebook_id", "cell_order", "execution_order"]);
            if rows.is_empty() { Vec::new() } else { vec![Part::Fields(rows)] }
        }
    }
}

/// A cell's output, or its error.
fn cell_output(cell: &Value) -> Vec<Part> {
    if let Some(error) = cell.get("error").filter(|e| !e.is_null()) {
        let message = error["message"].as_str().or(error.as_str()).map(str::to_owned).or_else(|| compact(error));
        return message.map(Part::Error).into_iter().collect();
    }
    field(cell, "output").map(|o| Part::Text(cut(o))).into_iter().collect()
}

/// A notebook edit or run's receipt: what it applied to, what ran and how it
/// went, the outputs that changed and any warnings.
fn receipt(json: &Value, named: &dyn Fn(&str) -> String) -> Vec<Part> {
    let mutation = &json["mutation"];
    let ids = |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect() };
    let listed = |ids: &[String]| {
        const LISTED: usize = 5;
        let mut names: Vec<String> = ids.iter().take(LISTED).map(|id| named(id)).collect();
        if ids.len() > LISTED {
            names.push(format!("{} more", ids.len() - LISTED));
        }
        names.join(", ")
    };
    let mut parts = Vec::new();
    let touched = match mutation["cell_id"].as_str() {
        Some(id) => vec![id.to_string()],
        None => ids(&mutation["cell_ids"]),
    };
    let applied = if json["applied"] == false { "Not applied" } else { "Applied" };
    parts.push(Part::Line(if touched.is_empty() { applied.to_string() } else { format!("{applied} to {}", listed(&touched)) }));

    let mut rows = Vec::new();
    let ran = ids(&json["affected_cells"]);
    if !ran.is_empty() {
        rows.push(("ran".to_string(), listed(&ran)));
    }
    if let Some(status) = json["execution"]["status"].as_str() {
        let status = match status {
            "completed" => "finished",
            "staged" => "not run yet",
            "timeout" => "timed out",
            other => other,
        };
        rows.push(("status".to_string(), status.to_string()));
    }
    let waiting = ids(&json["pending_run"]);
    if !waiting.is_empty() {
        rows.push(("waiting to run".to_string(), listed(&waiting)));
    }
    parts.push(Part::Fields(rows));

    for changed in json["outputs"]["changed"].as_array().into_iter().flatten() {
        let who = named(changed["cell_id"].as_str().unwrap_or_default());
        match changed.get("error").filter(|e| !e.is_null()) {
            Some(error) => {
                let message = error["message"].as_str().map(str::to_owned).or_else(|| compact(error)).unwrap_or_default();
                parts.push(Part::Error(format!("{who}: {message}")));
            }
            None => {
                if let Some(summary) = field(changed, "output_summary") {
                    parts.push(Part::Line(format!("{who} →")));
                    parts.push(Part::Text(cut(summary)));
                }
            }
        }
    }
    for warning in json["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let text = warning.split_once("::").map_or(warning, |(_, m)| m);
        parts.push(Part::Line(text.to_string()));
    }
    parts.retain(|p| !matches!(p, Part::Fields(rows) if rows.is_empty()));
    parts
}

#[cfg(test)]
mod tests {
    use super::{Details, Part, details};
    use agent_client_protocol::schema::v1::ToolKind;
    use serde_json::{Value, json};

    fn open(title: &str, kind: ToolKind, input: Value, output: Option<Value>, failed: bool, diffed: bool) -> Details {
        let names = |id: &str| (id == "c-fit").then(|| "fit".to_string());
        details(title, kind, None, &input, output.as_ref(), failed, diffed, &names)
    }

    fn fields(rows: &[(&str, &str)]) -> Part {
        Part::Fields(rows.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    #[test]
    fn commands_show_as_code_and_their_output_as_text() {
        let d = open("ls -la", ToolKind::Execute, json!({"command": "ls -la", "description": "List files"}), Some(json!("a.txt\nb.txt")), false, false);
        assert_eq!(d, Details { input: vec![Part::Code("ls -la".into())], output: vec![Part::Text("a.txt\nb.txt".into())] });
        let d = open("false", ToolKind::Execute, json!({"command": "false"}), Some(json!("Exit code 1\nno such file")), true, false);
        assert_eq!(d.output, vec![Part::Error("Exit code 1".into()), Part::Text("no such file".into())]);
    }

    #[test]
    fn reads_show_the_file_and_lines_then_numbered_contents() {
        let d = open(
            "Read data.csv",
            ToolKind::Read,
            json!({"file_path": "/d/data.csv", "offset": 10, "limit": 2}),
            Some(json!("10\tx,y\n11\t1,2\n<system-reminder>\nignore\n</system-reminder>")),
            false,
            false,
        );
        assert_eq!(d.input, vec![fields(&[("file", "/d/data.csv"), ("lines", "10–11")])]);
        assert_eq!(d.output, vec![Part::Numbered(vec![(10, "x,y".into()), (11, "1,2".into())])]);
        let d = open("Read a", ToolKind::Read, json!({"file_path": "/a"}), Some(json!("     1→fn main() {}")), false, false);
        assert_eq!(d.output, vec![Part::Numbered(vec![(1, "fn main() {}".into())])]);
    }

    #[test]
    fn edits_leave_their_input_and_success_to_the_diff() {
        let d = open("Edit a.rs", ToolKind::Edit, json!({"file_path": "/a.rs", "old_string": "a", "new_string": "b"}), Some(json!("The file /a.rs has been updated successfully.")), false, true);
        assert_eq!(d, Details::default());
    }

    #[test]
    fn searches_show_pattern_and_folder() {
        let d = open("grep", ToolKind::Search, json!({"pattern": "fn main", "path": "/src", "glob": "*.rs"}), Some(json!("/src/main.rs")), false, false);
        assert_eq!(d.input, vec![fields(&[("pattern", "fn main"), ("in", "/src"), ("files", "*.rs")])]);
        assert_eq!(d.output, vec![Part::Text("/src/main.rs".into())]);
    }

    #[test]
    fn unknown_tools_show_fields_not_json() {
        let d = open(
            "ToolSearch",
            ToolKind::Other,
            json!({"query": "select:WebFetch", "max_results": 5, "opts": {"a": 1, "b": [1, 2]}, "list": [{"x": 1}, {"x": 2}]}),
            Some(json!([{"type": "tool_reference", "tool_name": "WebFetch"}])),
            false,
            false,
        );
        assert_eq!(d.input, vec![fields(&[("query", "select:WebFetch"), ("max results", "5"), ("opts", "a: 1, b: 1, 2"), ("list", "2 items")])]);
        assert_eq!(d.output, vec![Part::Text("WebFetch".into())]);
    }

    #[test]
    fn host_tools_show_what_they_ran_and_read() {
        let out = |v: Value| Some(json!([{"type": "text", "text": v.to_string()}]));
        let d = open(
            "mcp__pluto__run_shell",
            ToolKind::Other,
            json!({"command": "make", "cwd": "/w"}),
            out(json!({"exit_code": 2, "stdout": "cc a.c\n", "stderr": "a.c:1: error\n", "timed_out": false, "cwd": "/w"})),
            true,
            false,
        );
        assert_eq!(d.input, vec![Part::Code("make".into()), fields(&[("in", "/w")])]);
        assert_eq!(d.output, vec![Part::Text("cc a.c".into()), Part::Text("a.c:1: error".into()), Part::Error("Exit code 2".into())]);
        let d = open(
            "mcp__pluto__read_file",
            ToolKind::Other,
            json!({"path": "/w/a.txt", "limit": 1}),
            out(json!({"path": "/w/a.txt", "text": "     1\thello\n", "start_line": 1, "end_line": 1, "total_lines": 3, "truncated": true})),
            false,
            false,
        );
        assert_eq!(d.input, vec![fields(&[("file", "/w/a.txt"), ("lines", "1–1")])]);
        assert_eq!(d.output, vec![Part::Numbered(vec![(1, "hello".into())]), Part::Line("Lines 1–1 of 3".into())]);
        let d = open(
            "mcp__pluto__list_folder",
            ToolKind::Other,
            json!({}),
            out(json!({"path": "/w", "entries": [{"name": "src", "kind": "dir"}, {"name": "a.jl", "kind": "file", "size": 3}], "total": 2, "truncated": false})),
            false,
            false,
        );
        assert_eq!(d.input, vec![fields(&[("folder", "the session folder")])]);
        assert_eq!(d.output, vec![Part::Text("src/\na.jl".into())]);
    }

    #[test]
    fn notebook_answers_show_key_facts() {
        let out = |v: Value| Some(json!([{"type": "text", "text": v.to_string()}]));
        let receipt = json!({
            "applied": true,
            "mutation": {"type": "edit_cell", "cell_id": "c-new"},
            "cell_order": ["c-fit", "c-new"],
            "affected_cells": ["c-new", "c-fit"],
            "execution": {"status": "completed"},
            "outputs": {"changed": [{"cell_id": "c-new", "output_summary": "0.25"}, {"cell_id": "c-fit", "output_summary": "", "error": {"message": "UndefVarError: `z`"}}]},
            "pending_run": [],
            "warnings": ["async_execution::cells running; pending_run clears when execution finishes"],
            "cell_id": "c-new",
            "code": "residuals = y .- ŷ",
        });
        let d = open("mcp__pluto__edit_cell", ToolKind::Other, json!({"notebook_id": "n", "cell_id": "c-new", "code": "residuals = y .- ŷ"}), out(receipt), false, true);
        assert_eq!(d.input, Vec::new(), "the diff stands for it");
        assert_eq!(
            d.output,
            vec![
                Part::Line("Applied to `residuals`".into()),
                fields(&[("ran", "`residuals`, `fit`"), ("status", "finished")]),
                Part::Line("`residuals` →".into()),
                Part::Text("0.25".into()),
                Part::Error("`fit`: UndefVarError: `z`".into()),
                Part::Line("cells running; pending_run clears when execution finishes".into()),
            ]
        );
        let d = open("mcp__pluto__execute_cell", ToolKind::Other, json!({"notebook_id": "n", "cell_id": "c-fit", "wait_for_completion": true}), None, false, false);
        assert_eq!(d.input, vec![fields(&[("cell", "`fit`"), ("wait for completion", "yes")])]);
        let d = open("mcp__pluto__read_cell", ToolKind::Other, json!({"notebook_id": "n", "cell_id": "c-9abcdef012"}), out(json!({"error": "not_found", "message": "No cell c-9abcdef012"})), true, false);
        assert_eq!(d.input, vec![fields(&[("cell", "cell c-9abcde")])]);
        assert_eq!(d.output, vec![Part::Error("No cell c-9abcdef012".into())]);
        let d = open("mcp__pluto__read_cell", ToolKind::Other, json!({}), out(json!({"cell_id": "c", "code": "x = 1", "output": "1", "errored": false})), false, false);
        assert_eq!(d.output, vec![Part::Code("x = 1".into()), Part::Text("1".into())]);
        let d = open("mcp__pluto__list_notebooks", ToolKind::Other, json!({}), out(json!([{"path": "/w/a.jl", "cell_count": 4, "execution_allowed": false}])), false, false);
        assert_eq!(d.output, vec![Part::Text("/w/a.jl · 4 cells · safe preview".into())]);
    }
}
