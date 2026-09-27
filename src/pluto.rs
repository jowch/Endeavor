//! The app's own line to the runtime's tools: its bridge's loopback `/call` endpoint (MCP
//! JSON-RPC). Used for app-side checks that shouldn't depend on the agent, like
//! the end-of-turn run-state warning.

use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

/// One runtime's bridge as its host's listener serves it: the URL the agent's
/// MCP config and the app use, and the bearer token it requires. Both stay the
/// same across that host's runtimes.
#[derive(Clone, Debug, PartialEq)]
pub struct Bridge {
    pub url: String,
    pub token: String,
}

impl Bridge {
    /// `127.0.0.1:PORT` from the bridge URL `http://127.0.0.1:PORT/sse`.
    fn host(&self) -> Result<&str, String> {
        self.url.strip_prefix("http://").and_then(|rest| rest.split('/').next()).ok_or_else(|| format!("bad MCP url {}", self.url))
    }
}

/// Follow the runtime's notebook state (`GET /events`): `on_event` gets
/// `{"notebooks": [list_notebooks summary], "cells": {id: [cell states]}}` now and
/// after every change, until the runtime goes away.
pub fn watch_notebooks(bridge: &Bridge, mut on_event: impl FnMut(Value)) -> Result<(), String> {
    let host = bridge.host()?;
    let mut stream = TcpStream::connect(host).map_err(|e| e.to_string())?;
    write!(stream, "GET /events HTTP/1.0\r\nHost: {host}\r\nAuthorization: Bearer {}\r\n\r\n", bridge.token)
        .map_err(|e| e.to_string())?;
    for line in std::io::BufReader::new(stream).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(event) = line.strip_prefix("data: ").and_then(|json| serde_json::from_str(json).ok()) {
            on_event(event);
        }
    }
    Ok(())
}

/// Call a runtime tool and return its decoded JSON result.
pub fn call_tool(bridge: &Bridge, tool: &str, arguments: Value) -> Result<Value, String> {
    let rpc = rpc(bridge, "tools/call", json!({ "name": tool, "arguments": arguments }))?;
    let text = rpc["result"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| format!("tool error: {}", rpc["result"]))?;
    serde_json::from_str(text).map_err(|e| format!("bad tool result: {e}"))
}

/// Set an agent session's policy in the runtime ("plan" refuses its notebook
/// writes and runs); `owner` is the session's key, sent as its MCP header.
pub fn set_policy(bridge: &Bridge, owner: u64, policy: &str) -> Result<(), String> {
    rpc(bridge, "endeavor/set_policy", json!({ "owner": owner.to_string(), "policy": policy })).map(|_| ())
}

/// Bind an agent session (`owner`, its key) to its one notebook file; the runtime
/// then refuses its opening, creating, editing or running any other.
pub fn set_notebook(bridge: &Bridge, owner: u64, path: &str) -> Result<(), String> {
    rpc(bridge, "endeavor/set_notebook", json!({ "owner": owner.to_string(), "notebook": path })).map(|_| ())
}

/// Make `dir` the folder Pluto suggests when saving a new notebook.
pub fn set_folder(bridge: &Bridge, dir: &Path) -> Result<(), String> {
    rpc(bridge, "endeavor/set_folder", json!({ "path": dir })).map(|_| ())
}

/// Stop open notebooks after `hours` with no activity; 0 never stops them.
pub fn set_idle_limit(bridge: &Bridge, hours: u32) -> Result<(), String> {
    rpc(bridge, "endeavor/set_idle_limit", json!({ "hours": hours })).map(|_| ())
}

/// Notebooks the runtime stopped for being idle, from its event stream:
/// (path, the idle limit in hours, whether it was in safe preview).
pub fn idle_stopped(event: &serde_json::Value) -> Vec<(String, u64, bool)> {
    event["idle_stopped"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s["path"].as_str()?.to_owned(), s["hours"].as_u64()?, s["safe_preview"].as_bool().unwrap_or(true))))
        .collect()
}

/// Shut down the open notebook at `path`. Returns whether it was in safe preview,
/// or None if it wasn't open.
pub fn stop_notebook(bridge: &Bridge, path: &str) -> Result<Option<bool>, String> {
    let reply = rpc(bridge, "endeavor/stop_notebook", json!({ "path": path }))?;
    let result = &reply["result"];
    Ok(result["stopped"].as_bool().unwrap_or(false).then(|| result["safe_preview"].as_bool().unwrap_or(false)))
}

/// What a run would run, for the approval card (the runtime's `run_preview`).
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct RunPreview {
    /// The whole notebook (`count` is then its size).
    pub all: bool,
    pub count: usize,
    pub cells: Vec<PreviewCell>,
    /// Other cells that re-run with these.
    pub dependents: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PreviewCell {
    /// What the cell defines, e.g. "fit, model".
    pub name: Option<String>,
    pub code: String,
}

pub fn run_preview(bridge: &Bridge, tool: &str, arguments: &Value) -> Result<RunPreview, String> {
    let reply = rpc(bridge, "endeavor/run_preview", json!({ "tool": tool, "arguments": arguments }))?;
    if let Some(error) = reply.get("error") {
        return Err(error["message"].as_str().unwrap_or("run_preview failed").to_string());
    }
    serde_json::from_value(reply["result"].clone()).map_err(|e| e.to_string())
}

/// One JSON-RPC request to the bridge's app-only `/call` endpoint.
fn rpc(bridge: &Bridge, method: &str, params: Value) -> Result<Value, String> {
    let host = bridge.host()?;
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();

    let mut stream = TcpStream::connect(host).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    // HTTP/1.0: the bridge sends no Content-Length, so read the body to EOF
    // instead of dealing with chunked encoding.
    write!(
        stream,
        "POST /call HTTP/1.0\r\nHost: {host}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        bridge.token,
        body.len()
    )
    .map_err(|e| e.to_string())?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;

    let (_, payload) = response.split_once("\r\n\r\n").ok_or("malformed HTTP response")?;
    serde_json::from_str(payload).map_err(|e| format!("bad JSON-RPC reply: {e}"))
}

/// Warnings for notebooks left with edited-but-unrun or still-running cells,
/// from `list_notebooks` (which reports run state without counting as a read).
pub fn run_warnings(notebooks: &Value) -> Vec<String> {
    let mut warnings = Vec::new();
    for nb in notebooks.as_array().into_iter().flatten() {
        let ids = |key: &str| -> Vec<&str> {
            nb[key].as_array().into_iter().flatten().filter_map(Value::as_str).collect()
        };
        let running = ids("running");
        // Cells queued for a run are also pending until they finish; count them once.
        let unrun = ids("pending_run").into_iter().filter(|id| !running.contains(id)).count();
        let name = nb["path"]
            .as_str()
            .and_then(|p| Path::new(p).file_name()?.to_str())
            .unwrap_or("notebook");
        let cells = |n: usize| if n == 1 { "1 cell".to_string() } else { format!("{n} cells") };

        if unrun > 0 && nb["execution_allowed"] == false {
            warnings.push(format!(
                "{name}: {} edited but not run. The notebook is in safe preview; click “Run notebook code” to run it.",
                cells(unrun)
            ));
        } else if unrun > 0 {
            warnings.push(format!("{name}: {} edited but never run.", cells(unrun)));
        }
        if !running.is_empty() {
            warnings.push(format!("{name}: {} still running.", cells(running.len())));
        }
    }
    warnings
}

/// A cell the user changed between two events: (notebook id, cell id, name).
pub type UserEdit = (String, String, Option<String>);

/// Cells whose code the user changed between two `/events` snapshots
/// (`{notebook_id: [cell state]}`): authored by "user", with a new version. The
/// first snapshot (`old` null) reports nothing.
pub fn user_edits(old: &Value, new: &Value) -> Vec<UserEdit> {
    let mut edits = Vec::new();
    let (Some(old), Some(new)) = (old.as_object(), new.as_object()) else { return edits };
    for (notebook, cells) in new {
        let before = |id: &str| old.get(notebook)?.as_array()?.iter().find(|c| c["cell_id"] == id).map(|c| c["version"].clone());
        for cell in cells.as_array().into_iter().flatten() {
            let Some(id) = cell["cell_id"].as_str() else { continue };
            if cell["author"] == "user" && before(id).is_some_and(|v| v != cell["version"]) {
                edits.push((notebook.clone(), id.to_owned(), cell["name"].as_str().map(str::to_owned)));
            }
        }
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::{run_warnings, user_edits};
    use serde_json::json;

    #[test]
    fn warns_about_unrun_and_running_cells() {
        let list = json!([
            { "path": "/n/clean.jl", "pending_run": [], "running": [], "execution_allowed": true },
            { "path": "/n/preview.jl", "pending_run": ["a", "b"], "running": [], "execution_allowed": false },
            { "path": "/n/busy.jl", "pending_run": ["c", "d"], "running": ["c"], "execution_allowed": true },
        ]);
        let w = run_warnings(&list);
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w[0].starts_with("preview.jl: 2 cells edited but not run") && w[0].contains("safe preview"));
        assert_eq!(w[1], "busy.jl: 1 cell edited but never run.");
        assert_eq!(w[2], "busy.jl: 1 cell still running.");
    }

    #[test]
    fn finds_the_users_edits() {
        let snap = |author: &str, version: &str| json!({ "nb": [
            { "cell_id": "a", "author": author, "version": version, "name": "fit" },
            { "cell_id": "b", "author": null, "version": "1", "name": null },
        ]});
        assert!(user_edits(&json!(null), &snap("user", "2")).is_empty(), "first snapshot");
        assert_eq!(user_edits(&snap("", "1"), &snap("user", "2")), vec![("nb".into(), "a".into(), Some("fit".into()))]);
        assert!(user_edits(&snap("user", "2"), &snap("user", "2")).is_empty(), "no new edit");
        assert!(user_edits(&snap("user", "1"), &snap("agent", "2")).is_empty(), "the agent's");
        assert_eq!(user_edits(&snap("user", "2"), &snap("user", "3")).len(), 1, "a second edit");
    }

    #[test]
    fn nothing_to_say_for_clean_or_unexpected_input() {
        assert!(run_warnings(&json!([{ "path": "/n/a.jl", "pending_run": [], "running": [] }])).is_empty());
        assert!(run_warnings(&json!({ "error": "x" })).is_empty());
    }
}

/// Live check against a running bridge: `ENDEAVOR_TEST_MCP_URL=http://127.0.0.1:PORT/sse
/// ENDEAVOR_TEST_TOKEN=… cargo test -- --ignored live_bridge`.
#[cfg(test)]
#[test]
#[ignore]
fn live_bridge() {
    let url = std::env::var("ENDEAVOR_TEST_MCP_URL").expect("ENDEAVOR_TEST_MCP_URL");
    let bridge = Bridge { url, token: std::env::var("ENDEAVOR_TEST_TOKEN").unwrap_or_default() };
    let list = call_tool(&bridge, "list_notebooks", json!({})).expect("list_notebooks");
    println!("list_notebooks: {list}\nwarnings: {:?}", run_warnings(&list));
    assert!(list.as_array().is_some_and(|a| a.iter().all(|nb| nb.get("pending_run").is_some())));
}
