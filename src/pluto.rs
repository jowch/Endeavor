//! The app's own line to the runtime's tools: its loopback `/endeavor/call`
//! endpoint (MCP JSON-RPC). Used for app-side checks that shouldn't depend on
//! the agent, like the end-of-turn run-state warning.

use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use wire::backend::Backend;

/// One runtime as its host's listener serves it: the MCP URL the agent's
/// config carries (its host is the app's way in too), the bearer token every
/// path but the pages requires, and the pages' token. They stay the same
/// across that host's runtimes.
#[derive(Clone, Debug, PartialEq)]
pub struct Bridge {
    pub url: String,
    pub token: String,
    /// What opens Pluto's pages (EndeavorMCP interface 8); `token` no longer does.
    /// An older runtime has none and takes `token` there.
    pub page_token: String,
}

impl Bridge {
    /// `127.0.0.1:PORT` from the bridge URL `http://127.0.0.1:PORT/mcp`.
    fn host(&self) -> Result<&str, String> {
        self.url.strip_prefix("http://").and_then(|rest| rest.split('/').next()).ok_or_else(|| format!("bad MCP url {}", self.url))
    }
}

/// Follow the runtime's notebook state (`GET /endeavor/events`): `on_event` gets
/// `{"notebooks": [list_notebooks summary], "cells": {id: [cell states]}}` now and
/// after every change, until the runtime goes away.
pub fn watch_notebooks(bridge: &Bridge, mut on_event: impl FnMut(Value)) -> Result<(), String> {
    let host = bridge.host()?;
    let mut stream = TcpStream::connect(host).map_err(|e| e.to_string())?;
    write!(stream, "GET /endeavor/events HTTP/1.0\r\nHost: {host}\r\nAuthorization: Bearer {}\r\n\r\n", bridge.token)
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
    call_tool_waiting(bridge, tool, arguments, CALL_WAIT)
}

fn call_tool_waiting(bridge: &Bridge, tool: &str, arguments: Value, wait: Duration) -> Result<Value, String> {
    let rpc = rpc_waiting(bridge, "tools/call", json!({ "name": tool, "arguments": arguments }), wait)?;
    let text = rpc["result"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| format!("tool error: {}", rpc["result"]))?;
    serde_json::from_str(text).map_err(|e| format!("bad tool result: {e}"))
}

/// Set an agent session's policy in the runtime ("plan" refuses its notebook
/// writes and runs, "ask" holds a run until the app answers, "auto" runs it);
/// with `edits` (Manual) it holds a change to the notebook too. `owner` is
/// the session's key, sent as its MCP header.
pub fn set_policy(bridge: &Bridge, owner: u64, policy: &str, edits: bool) -> Result<(), String> {
    rpc(bridge, "endeavor/set_policy", json!({ "owner": owner.to_string(), "policy": policy, "asks": true, "edits": edits })).map(|_| ())
}

/// Answer a run the runtime holds (its ask `id`), with the cells the user's
/// own run reached while it waited (each with its `last_run` before that run).
pub fn answer_run(bridge: &Bridge, id: u64, allow: bool, user_ran: &[(String, f64)]) -> Result<(), String> {
    let user_ran: Vec<Value> = user_ran.iter().map(|(id, last_run)| json!({ "cell_id": id, "last_run": last_run })).collect();
    app_call(bridge, "endeavor/answer_run", json!({ "id": id, "allow": allow, "user_ran": user_ran })).map(|_| ())
}

/// Bind an agent session (`owner`, its key) to its one notebook file; the runtime
/// then refuses its opening, creating, editing or running any other.
pub fn set_notebook(bridge: &Bridge, owner: u64, path: &str) -> Result<(), String> {
    rpc(bridge, "endeavor/set_notebook", json!({ "owner": owner.to_string(), "notebook": path })).map(|_| ())
}

/// The session `owner`'s working folder, where the runtime's `run_shell` runs
/// by default. Paths here are the runtime machine's (`Place::path`).
pub fn set_session_folder(bridge: &Bridge, owner: u64, folder: &str, kind: Backend) -> Result<(), String> {
    // The core forgets kinds when it restarts, so every call says it.
    let kind = crate::new_session::kind_name(kind);
    rpc(bridge, "endeavor/set_session_folder", json!({ "owner": owner.to_string(), "folder": folder, "kind": kind })).map(|_| ())
}

/// Make `dir` the folder Pluto suggests when saving a new notebook.
pub fn set_folder(bridge: &Bridge, dir: &str) -> Result<(), String> {
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

/// Notebooks the runtime closed because their engine (R) ended, from its event stream:
/// (path, the engine's language, why it ended). Listed until each opens again.
pub fn engine_ended(event: &serde_json::Value) -> Vec<(String, String, String)> {
    event["engine_ended"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e["path"].as_str()?.to_owned(), e["language"].as_str().unwrap_or("R").to_owned(), e["why"].as_str().unwrap_or_default().to_owned())))
        .collect()
}

/// The user pressed Stop in session `owner`: the runtime interrupts the runs that session
/// started and that are still going, which it doesn't hear of from the agent when no tool
/// call is under way. A runtime from before the call has nothing to stop.
pub fn stop_session(bridge: &Bridge, owner: u64) -> Result<(), String> {
    match app_call(bridge, "endeavor/stop_session", json!({ "owner": owner.to_string() })) {
        Err(error) if error.starts_with("Method not found") => Ok(()),
        reply => reply.map(|_| ()),
    }
}

/// Shut down the open notebook at `path`. Returns whether it was in safe preview,
/// or None if it wasn't open.
pub fn stop_notebook(bridge: &Bridge, path: &str) -> Result<Option<bool>, String> {
    let reply = rpc(bridge, "endeavor/stop_notebook", json!({ "path": path }))?;
    let result = &reply["result"];
    Ok(result["stopped"].as_bool().unwrap_or(false).then(|| result["safe_preview"].as_bool().unwrap_or(false)))
}

/// An app-only bridge method's `result`, or its error message.
fn app_call(bridge: &Bridge, method: &str, params: Value) -> Result<Value, String> {
    app_call_waiting(bridge, method, params, CALL_WAIT)
}

fn app_call_waiting(bridge: &Bridge, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
    let reply = rpc_waiting(bridge, method, params, wait)?;
    match reply.get("error") {
        Some(error) => Err(error["message"].as_str().unwrap_or("failed").trim_start_matches("ArgumentError: ").to_string()),
        None => Ok(reply["result"].clone()),
    }
}

/// Pluto's own Restart: a new process, then every cell runs. Refused in safe preview.
pub fn restart_notebook(bridge: &Bridge, notebook_id: &str) -> Result<(), String> {
    app_call(bridge, "endeavor/restart_notebook", json!({ "notebook_id": notebook_id })).map(|_| ())
}

/// Restore on a turn's card: put `cells` (`ChangedCell::restore`) back as they
/// were before the turn, all or nothing. Ok with the cells that changed since
/// the turn, by id, when nothing was restored for that reason; empty when it
/// was.
pub fn restore_cells(bridge: &Bridge, notebook_id: &str, cells: Vec<Value>) -> Result<Vec<String>, String> {
    let result = match app_call(bridge, "endeavor/restore_cells", json!({ "notebook_id": notebook_id, "cells": cells })) {
        Err(error) if error.starts_with("Method not found") => return Err("This notebook's runtime is from an older Endeavor. Restart it, then try again.".into()),
        Err(error) if error.contains("notebook_not_found::") => return Err("The notebook this turn changed isn't open now. Open it, then try again.".into()),
        Err(error) => return Err(crate::notice::reason(&error)),
        Ok(result) => result,
    };
    Ok(result["changed"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_owned)).collect())
}

/// The user's run reached `cells` (with each one's `last_run` before it),
/// which the agent's waiting cards ask to run: an older runtime won't run
/// them again for the approved calls. A runtime from this build hears it in
/// the answer to its own ask instead (`answer_run`).
pub fn run_anyway(bridge: &Bridge, notebook_id: &str, cells: &[(String, f64)]) -> Result<(), String> {
    let cells: Vec<Value> = cells.iter().map(|(id, last_run)| json!({ "cell_id": id, "last_run": last_run })).collect();
    app_call(bridge, "endeavor/run_anyway", json!({ "notebook_id": notebook_id, "cells": cells })).map(|_| ())
}

/// The result the runtime kept of session `owner`'s notebook call (`call_id`
/// is the agent's id for it), `{content, isError}`, for an agent that didn't
/// pass it on; none if the runtime has no such call.
pub fn tool_result(bridge: &Bridge, owner: u64, call_id: &str, tool: &str, arguments: &Value) -> Result<Option<Value>, String> {
    let result = app_call(bridge, "endeavor/tool_result", json!({ "owner": owner.to_string(), "call_id": call_id, "tool": tool, "arguments": arguments }))?;
    Ok((!result.is_null()).then_some(result))
}

/// Rename or move an open notebook's file; returns its new path.
pub fn move_notebook(bridge: &Bridge, notebook_id: &str, path: &str) -> Result<String, String> {
    let result = app_call(bridge, "endeavor/move_notebook", json!({ "notebook_id": notebook_id, "path": path }))?;
    result["path"].as_str().map(str::to_owned).ok_or_else(|| "no path in reply".into())
}

/// Whether a file is on the runtime's machine, and its modification time.
pub fn file_info(bridge: &Bridge, path: &str) -> Result<Option<f64>, String> {
    let result = app_call(bridge, "endeavor/file_info", json!({ "path": path }))?;
    Ok(if result["exists"] == true { Some(result["modified"].as_f64().unwrap_or(0.)) } else { None })
}

/// Open the notebook at `path`, running it if `run`: `list_notebooks`' summary
/// of it. Julia starts first if it isn't running; while it's still starting
/// this says so (`julia_starting`).
pub fn open_notebook(bridge: &Bridge, path: &str, run: bool) -> Result<Value, String> {
    answer(call_tool_waiting(bridge, "open_notebook", json!({ "path": path, "run_notebook": run }), JULIA_CALL_WAIT)?)
}

/// A tool's result, or the error it answers with, as an app call says it: `kind::message`.
fn answer(result: Value) -> Result<Value, String> {
    match result["error"].as_str() {
        Some(kind) => Err(format!("{kind}::{}", result["message"].as_str().unwrap_or_default())),
        None => Ok(result),
    }
}

/// How long the app's calls wait for an answer.
const CALL_WAIT: Duration = Duration::from_secs(10);
/// How long a call that may start Julia waits: the core answers within 45 s,
/// that Julia is still starting or the notebook still opening if it must, and
/// this leaves room past that for the answer to arrive (over ssh, too).
const JULIA_CALL_WAIT: Duration = Duration::from_secs(55);

/// What Julia is doing, from a call's error that says it's still starting
/// ("julia_starting::Downloading Julia 1.12.6… 42%. The first start can take…").
pub fn julia_starting(error: &str) -> Option<&str> {
    let step = error.strip_prefix("julia_starting::")?;
    Some(step.split_once(". The first start").map_or(step, |(step, _)| step).trim())
}

/// Why Julia couldn't start, from a call's error that says so.
pub fn julia_failed(error: &str) -> Option<&str> {
    error.strip_prefix("julia_failed::").or_else(|| error.strip_prefix("julia_not_found::")).map(str::trim)
}

/// Why Julia couldn't start, for the person: the core words it for the agent,
/// whose part ("Tell the user…", "Ask the user…") the pane's Try again stands in for.
pub fn julia_failed_words(why: &str) -> String {
    let agent = |sentence: &str| ["Tell the user", "Ask the user"].iter().any(|start| sentence.trim_start().starts_with(start));
    let kept: Vec<&str> = why.split_inclusive(". ").filter(|sentence| !agent(sentence)).collect();
    let words = kept.concat();
    if words.trim().is_empty() { why.trim().to_owned() } else { words.trim().to_owned() }
}

/// What Julia is doing in a runtime, as `endeavor/julia_status` says it (EndeavorMCP interface 5).
#[derive(Clone, Debug, PartialEq)]
pub enum JuliaStatus {
    /// Nothing has needed Julia yet.
    NotStarted,
    /// The step under way, and how many seconds since the last sign of progress.
    Starting { step: String, quiet: u64 },
    Ready,
    /// The last start failed: its code (`julia_failed`, `julia_not_found`) and why.
    Failed { code: String, message: String },
}

impl JuliaStatus {
    fn from(result: &Value) -> Result<JuliaStatus, String> {
        let text = |key: &str| result[key].as_str().unwrap_or_default().to_owned();
        Ok(match result["state"].as_str() {
            Some("not_started") => JuliaStatus::NotStarted,
            Some("starting") => JuliaStatus::Starting { step: text("step"), quiet: result["quiet_seconds"].as_u64().unwrap_or(0) },
            Some("ready") => JuliaStatus::Ready,
            Some("failed") => JuliaStatus::Failed { code: text("code"), message: text("message") },
            _ => return Err(format!("unexpected Julia status {result}")),
        })
    }
}

/// Where Julia is, without starting it.
pub fn julia_status(bridge: &Bridge) -> Result<JuliaStatus, String> {
    JuliaStatus::from(&app_call(bridge, "endeavor/julia_status", json!({}))?)
}

/// Start Julia now (after a failure: the person's Try again), and where it is then.
pub fn start_julia(bridge: &Bridge) -> Result<JuliaStatus, String> {
    JuliaStatus::from(&app_call(bridge, "endeavor/start_julia", json!({}))?)
}

/// What R notebooks' setup is doing, from a call's error that says it's still
/// installing ("r_installing::Installing Ember for R notebooks, which takes…").
pub fn r_installing(error: &str) -> Option<&str> {
    let step = error.strip_prefix("r_installing::")?;
    Some(step.split_once(", which").map_or(step, |(step, _)| step).trim())
}

/// The step the pane shows while a notebook is still opening.
pub const OPENING: &str = "Opening the notebook";

/// How long the app keeps calling again while a notebook is still opening.
/// The runtime's open has no limit of its own, so a Julia that hangs while
/// opening would otherwise leave the pane waiting for ever.
const OPENING_LIMIT: Duration = Duration::from_secs(600);

/// Why the app stopped waiting for a notebook that was still opening, for the person.
pub const OPEN_STUCK: &str = "The notebook was still opening after 10 minutes, so Endeavor stopped waiting. Try again to wait for it again.";

/// Why the app stopped waiting for Run notebook's new Julia process, for the person.
pub const RUN_STUCK: &str = "The notebook was still starting after 10 minutes, so Endeavor stopped waiting. Try again to wait for it again.";

/// Whether a call's answer says an open or a new notebook took longer than one
/// call waits ("notebook_opening::Opening '/x.jl' is taking longer…"): the open
/// goes on, and the same call again gets it. The step is always `OPENING`.
pub fn notebook_opening(error: &str) -> Option<&'static str> {
    error.starts_with("notebook_opening::").then_some(OPENING)
}

/// That the app stopped waiting for a notebook still opening (`until_julia`):
/// its words for the person.
pub fn open_stuck(error: &str) -> Option<&str> {
    error.strip_prefix("open_stuck::")
}

/// Why R notebooks couldn't start, from a call's error that says so.
pub fn r_failed(error: &str) -> Option<&str> {
    error.strip_prefix("r_failed::").or_else(|| error.strip_prefix("r_not_found::")).map(str::trim)
}

/// `call` again while it answers that Julia is still starting (the first
/// start downloads Julia and installs Pluto's packages, which takes minutes),
/// or that R notebooks are still being set up (R, then Ember), or that the
/// notebook is still opening, with `step` hearing what it's doing each time.
/// A notebook still opening after `OPENING_LIMIT` ends it with `open_stuck`.
pub fn until_julia<T>(call: impl FnMut() -> Result<T, String>, step: &dyn Fn(&str)) -> Result<T, String> {
    until_julia_within(call, step, OPENING_LIMIT)
}

fn until_julia_within<T>(mut call: impl FnMut() -> Result<T, String>, step: &dyn Fn(&str), limit: Duration) -> Result<T, String> {
    let mut opening_since = None;
    loop {
        match call() {
            Err(e) => match julia_starting(&e).or_else(|| r_installing(&e)).or_else(|| notebook_opening(&e)) {
                Some(doing) => {
                    if notebook_opening(&e).is_some() {
                        if opening_since.get_or_insert_with(std::time::Instant::now).elapsed() >= limit {
                            return Err(format!("open_stuck::{OPEN_STUCK}"));
                        }
                    } else {
                        opening_since = None;
                    }
                    step(doing);
                    std::thread::sleep(Duration::from_secs(1));
                }
                None => return Err(e),
            },
            done => return done,
        }
    }
}

/// Let the runtime install Endeavor's own R, which the person agreed to (EndeavorMCP interface 6).
pub fn allow_r_install(bridge: &Bridge) -> Result<(), String> {
    app_call(bridge, "endeavor/allow_r_install", json!({})).map(|_| ())
}

/// A new notebook in session `owner`'s folder, bound to it: (notebook id, path).
/// Julia starts first if it isn't running (see `open_notebook`).
pub fn new_notebook(bridge: &Bridge, owner: u64) -> Result<(String, String), String> {
    let result = app_call_waiting(bridge, "endeavor/new_notebook", json!({ "owner": owner.to_string() }), JULIA_CALL_WAIT)?;
    match (result["notebook_id"].as_str(), result["path"].as_str()) {
        (Some(id), Some(path)) => Ok((id.to_owned(), path.to_owned())),
        _ => Err(format!("unexpected reply {result}")),
    }
}

/// Leave safe preview and run the notebook (the user's Run notebook). Its new
/// Julia process can take longer than one call, installing packages: then this
/// answers `notebook_opening`, and the same call again gets the result.
pub fn allow_execution(bridge: &Bridge, notebook_id: &str) -> Result<(), String> {
    answer(call_tool_waiting(bridge, "allow_execution", json!({ "notebook_id": notebook_id }), JULIA_CALL_WAIT)?).map(|_| ())
}

/// GET `target` (a path and query) on Pluto's server, such as an export,
/// through the host's loopback relay.
/// The pages' token opens it; the MCP token gets a 401 there.
pub fn fetch(bridge: &Bridge, target: &str) -> Result<Vec<u8>, String> {
    let host = bridge.host()?;
    let mut stream = TcpStream::connect(host).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).map_err(|e| e.to_string())?;
    write!(stream, "GET {target} HTTP/1.0\r\nHost: {host}\r\nAuthorization: Bearer {}\r\n\r\n", bridge.page_token).map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).map_err(|e| e.to_string())?;
    http_body(&response)
}

/// A whole HTTP/1.x response's body (plain or chunked), if its status is 200.
fn http_body(response: &[u8]) -> Result<Vec<u8>, String> {
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").ok_or("malformed HTTP response")?;
    let head = String::from_utf8_lossy(&response[..split]).to_ascii_lowercase();
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status != "200" {
        return Err(format!("The notebook server answered {status}"));
    }
    let mut body = &response[split + 4..];
    if !head.contains("transfer-encoding: chunked") {
        return Ok(body.to_vec());
    }
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|w| w == b"\r\n").ok_or("bad chunk")?;
        let size = usize::from_str_radix(String::from_utf8_lossy(&body[..line_end]).split(';').next().unwrap_or("").trim(), 16).map_err(|e| e.to_string())?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        out.extend_from_slice(body.get(..size).ok_or("short chunk")?);
        body = body.get(size + 2..).unwrap_or(&[]);
    }
}

/// What a run would run, for the approval card (the runtime's `run_preview`).
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct RunPreview {
    /// The whole notebook (`count` is then its size).
    pub all: bool,
    pub count: usize,
    pub cells: Vec<PreviewCell>,
    /// Cells these depend on that never ran, which run first.
    #[serde(default)]
    pub needed_ids: Vec<String>,
    /// Other cells that re-run with these.
    pub dependents: usize,
    /// Those cells' ids (a runtime older than this field sends none).
    #[serde(default)]
    pub dependent_ids: Vec<String>,
    /// The packages a whole-notebook run loads, in notebook order.
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PreviewCell {
    #[serde(default)]
    pub id: Option<String>,
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

/// One JSON-RPC request to the runtime's app-only `/endeavor/call` endpoint.
fn rpc(bridge: &Bridge, method: &str, params: Value) -> Result<Value, String> {
    rpc_waiting(bridge, method, params, CALL_WAIT)
}

fn rpc_waiting(bridge: &Bridge, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
    let host = bridge.host()?;
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();

    let mut stream = TcpStream::connect(host).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(wait)).map_err(|e| e.to_string())?;
    // HTTP/1.0: the bridge sends no Content-Length, so read the body to EOF
    // instead of dealing with chunked encoding.
    write!(
        stream,
        "POST /endeavor/call HTTP/1.0\r\nHost: {host}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        bridge.token,
        body.len()
    )
    .map_err(|e| e.to_string())?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;

    let (_, payload) = response.split_once("\r\n\r\n").ok_or("malformed HTTP response")?;
    serde_json::from_str(payload).map_err(|e| format!("bad JSON-RPC reply: {e}"))
}

/// A notebook left with cells edited but not run, or still running.
#[derive(Clone, Debug, PartialEq)]
pub struct RunWarning {
    pub path: String,
    pub kind: RunWarningKind,
    pub cells: Vec<String>,
    /// Edited cells can't run until the user lets the notebook run.
    pub safe_preview: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RunWarningKind {
    Unrun,
    Running,
}

impl std::fmt::Display for RunWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let name = Path::new(&self.path).file_name().and_then(|n| n.to_str()).unwrap_or("notebook");
        let (cells, one) = match self.cells.len() {
            1 => ("1 cell".to_string(), true),
            n => (format!("{n} cells"), false),
        };
        let (was, has, is) = if one { ("was", "hasn't", "is") } else { ("were", "haven't", "are") };
        match self.kind {
            RunWarningKind::Unrun if self.safe_preview => {
                let they = if one { "it runs" } else { "they run" };
                write!(f, "{cells} in {name} {was} changed but {has} run. The notebook is in safe preview, so {they} once you press Run notebook at its top.")
            }
            RunWarningKind::Unrun => write!(f, "{cells} in {name} {was} changed but {has} run"),
            RunWarningKind::Running => write!(f, "{cells} in {name} {is} still running"),
        }
    }
}

/// Notebooks left with edited-but-unrun or still-running cells, from
/// `list_notebooks` (which reports run state without counting as a read).
pub fn run_warnings(notebooks: &Value) -> Vec<RunWarning> {
    let mut warnings = Vec::new();
    for nb in notebooks.as_array().into_iter().flatten() {
        let ids = |key: &str| -> Vec<String> {
            nb[key].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
        };
        let running = ids("running");
        // Cells queued for a run are also pending until they finish; count them once.
        let unrun: Vec<String> = ids("pending_run").into_iter().filter(|id| !running.contains(id)).collect();
        let path = nb["path"].as_str().unwrap_or_default().to_owned();
        let safe_preview = nb["execution_allowed"] == false;
        for (kind, cells) in [(RunWarningKind::Unrun, unrun), (RunWarningKind::Running, running)] {
            if !cells.is_empty() {
                warnings.push(RunWarning { path: path.clone(), kind, cells, safe_preview });
            }
        }
    }
    warnings
}

/// What is still true of `said` in `notebooks`: each warning keeps only the
/// cells that are still unrun or still running, and goes once none are. It
/// never gains cells, so an old note doesn't take on a later turn's work.
pub fn still_true(said: &[RunWarning], notebooks: &Value) -> Vec<RunWarning> {
    let now = run_warnings(notebooks);
    said.iter()
        .filter_map(|w| {
            let current = now.iter().find(|c| c.path == w.path && c.kind == w.kind)?;
            let cells: Vec<String> = w.cells.iter().filter(|id| current.cells.contains(id)).cloned().collect();
            (!cells.is_empty()).then(|| RunWarning { cells, safe_preview: current.safe_preview, ..w.clone() })
        })
        .collect()
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

/// The cells that were unrun (the agent changed them) in `old` and have run
/// since in `new`, by notebook, from two `/events` updates' `cells`.
pub fn cells_that_ran(old: &Value, new: &Value) -> Vec<(String, Vec<String>)> {
    let (Some(old), Some(new)) = (old.as_object(), new.as_object()) else { return Vec::new() };
    let mut out = Vec::new();
    for (notebook, cells) in new {
        let was_unrun = |id: &str| old.get(notebook).and_then(Value::as_array).into_iter().flatten().any(|c| c["cell_id"] == id && c["unrun"] == true);
        let ran: Vec<String> = cells
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["unrun"] == false && c["running"] != true)
            .filter_map(|c| c["cell_id"].as_str())
            .filter(|id| was_unrun(id))
            .map(str::to_owned)
            .collect();
        if !ran.is_empty() {
            out.push((notebook.clone(), ran));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{RunWarning, cells_that_ran, engine_ended, julia_failed, julia_starting, run_warnings, still_true, until_julia, user_edits};

    #[test]
    fn the_events_list_the_notebooks_r_took_with_it() {
        // As the core sends it (EndeavorMCP notebooks.rs `compose`).
        let event = serde_json::json!({ "engine_ended": [{ "path": "/lab/growth.R", "language": "R", "why": "exit status: 1; R's last output: Error: boom" }] });
        assert_eq!(engine_ended(&event), [("/lab/growth.R".to_owned(), "R".to_owned(), "exit status: 1; R's last output: Error: boom".to_owned())]);
        assert!(engine_ended(&serde_json::json!({})).is_empty(), "an older runtime sends none");
    }

    #[test]
    fn a_call_that_waits_for_julia_says_what_julia_is_doing() {
        // As the core words it (EndeavorMCP core.rs `JuliaStarter::wait`).
        let starting = "julia_starting::Downloading Julia 1.12.6… 42%. The first start can take several minutes (Julia, then Pluto's packages). Try again in a minute.";
        assert_eq!(julia_starting(starting), Some("Downloading Julia 1.12.6… 42%"));
        assert_eq!(julia_starting("julia_starting::Julia is starting"), Some("Julia is starting"));
        assert_eq!(julia_starting("notebook_not_found::No notebook"), None);
        assert_eq!(julia_failed("julia_failed::Couldn't start /x/julia: gone"), Some("Couldn't start /x/julia: gone"));
        assert_eq!(julia_failed(starting), None);
    }

    #[test]
    fn julia_status_reads_each_state_and_a_failure_keeps_only_the_persons_words() {
        use super::{JuliaStatus, julia_failed_words};
        let read = |v: serde_json::Value| JuliaStatus::from(&v);
        assert_eq!(read(serde_json::json!({ "state": "not_started" })), Ok(JuliaStatus::NotStarted));
        assert_eq!(read(serde_json::json!({ "state": "starting", "step": "Downloading Julia 1.12.6… 42%", "quiet_seconds": 3 })), Ok(JuliaStatus::Starting { step: "Downloading Julia 1.12.6… 42%".into(), quiet: 3 }));
        assert_eq!(read(serde_json::json!({ "state": "ready" })), Ok(JuliaStatus::Ready));
        assert_eq!(read(serde_json::json!({ "state": "failed", "code": "julia_failed", "message": "no" })), Ok(JuliaStatus::Failed { code: "julia_failed".into(), message: "no".into() }));
        assert!(read(serde_json::json!({})).is_err());
        // As the core words a start that stalled (EndeavorMCP core.rs `JuliaStarter::start`).
        let stalled = "Julia made no progress for 30 minutes while starting (no new step, nothing new in the runtime's log), so it was stopped. The runtime's log (/x/runtime.log) shows where it stopped. Tell the user; trying again starts Julia afresh, so ask them before trying again.";
        assert_eq!(julia_failed_words(stalled), "Julia made no progress for 30 minutes while starting (no new step, nothing new in the runtime's log), so it was stopped. The runtime's log (/x/runtime.log) shows where it stopped.");
        assert_eq!(julia_failed_words("Couldn't start /x/julia: gone"), "Couldn't start /x/julia: gone");
        assert_eq!(julia_failed_words("Tell the user."), "Tell the user.", "never nothing");
    }

    #[test]
    fn until_julia_tries_again_while_julia_starts_and_hears_each_step() {
        let answers = std::cell::RefCell::new(vec![Ok(7), Err("julia_starting::Julia 1.12.6 is starting and loading Pluto. The first start…".to_owned()), Err("julia_starting::Julia is starting".to_owned())]);
        let heard = std::cell::RefCell::new(Vec::new());
        let result = until_julia(|| answers.borrow_mut().pop().unwrap(), &|step| heard.borrow_mut().push(step.to_owned()));
        assert_eq!(result, Ok(7));
        assert_eq!(*heard.borrow(), ["Julia is starting", "Julia 1.12.6 is starting and loading Pluto"]);
        // As the core words an open that outlasted its call (EndeavorMCP notebooks.rs `open_waited`): it goes on, and the same call again gets it.
        let answers = std::cell::RefCell::new(vec![Ok(8), Err("notebook_opening::Creating '/p/a.jl' is taking longer than one call waits: the first notebook…".to_owned())]);
        let heard = std::cell::RefCell::new(Vec::new());
        assert_eq!(until_julia(|| answers.borrow_mut().pop().unwrap(), &|step| heard.borrow_mut().push(step.to_owned())), Ok(8));
        assert_eq!(*heard.borrow(), [super::OPENING]);
        // A notebook that stays opening past the limit ends the wait, with words for the person.
        let stuck: Result<(), String> = super::until_julia_within(|| Err("notebook_opening::Opening '/p/a.jl' is taking longer…".into()), &|_| {}, std::time::Duration::ZERO);
        assert_eq!(stuck.as_ref().err().and_then(|e| super::open_stuck(e)), Some(super::OPEN_STUCK));
        // Run notebook's still-starting answer, a normal tool result (EndeavorMCP #128), is waited out the same way.
        let answers = std::cell::RefCell::new(vec![json!({ "ran": true }), json!({ "error": "notebook_opening", "message": "Starting the notebook is taking longer than one call waits…" })]);
        assert_eq!(until_julia(|| super::answer(answers.borrow_mut().pop().unwrap()), &|_| {}), Ok(json!({ "ran": true })));
        let failed: Result<(), String> = until_julia(|| Err("julia_failed::no".into()), &|_| panic!("not starting"));
        assert_eq!(failed, Err("julia_failed::no".into()));
    }

    #[test]
    fn cells_that_ran_were_unrun_and_now_are_not() {
        let old = json!({ "n1": [
            { "cell_id": "a", "unrun": false, "running": false },
            { "cell_id": "b", "unrun": true, "running": false },
            { "cell_id": "c", "unrun": true, "running": false },
            { "cell_id": "d", "unrun": true, "running": false },
        ] });
        let new = json!({ "n1": [
            { "cell_id": "a", "unrun": false, "running": false },
            { "cell_id": "b", "unrun": false, "running": false },
            { "cell_id": "c", "unrun": true, "running": true },
            { "cell_id": "d", "unrun": false, "running": true },
        ], "n2": [{ "cell_id": "e", "unrun": false, "running": false }] });
        assert_eq!(cells_that_ran(&old, &new), vec![("n1".to_owned(), vec!["b".to_owned()])]);
    }
    use serde_json::json;

    fn texts(warnings: &[RunWarning]) -> Vec<String> {
        warnings.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn warns_about_unrun_and_running_cells() {
        let list = json!([
            { "path": "/n/clean.jl", "pending_run": [], "running": [], "execution_allowed": true },
            { "path": "/n/preview.jl", "pending_run": ["a", "b"], "running": [], "execution_allowed": false },
            { "path": "/n/busy.jl", "pending_run": ["c", "d"], "running": ["c"], "execution_allowed": true },
        ]);
        assert_eq!(
            texts(&run_warnings(&list)),
            [
                "2 cells in preview.jl were changed but haven't run. The notebook is in safe preview, so they run once you press Run notebook at its top.",
                "1 cell in busy.jl was changed but hasn't run",
                "1 cell in busy.jl is still running",
            ]
        );
    }

    #[test]
    fn a_warning_keeps_only_what_is_still_true() {
        let said = run_warnings(&json!([
            { "path": "/n/busy.jl", "pending_run": ["a", "b", "c"], "running": ["a", "b"], "execution_allowed": true },
        ]));
        assert_eq!(texts(&said), ["1 cell in busy.jl was changed but hasn't run", "2 cells in busy.jl are still running"]);

        let one_done = json!([{ "path": "/n/busy.jl", "pending_run": ["b", "c"], "running": ["b"], "execution_allowed": true }]);
        assert_eq!(texts(&still_true(&said, &one_done)), ["1 cell in busy.jl was changed but hasn't run", "1 cell in busy.jl is still running"]);

        let all_done = json!([{ "path": "/n/busy.jl", "pending_run": [], "running": [], "execution_allowed": true }]);
        assert!(still_true(&said, &all_done).is_empty());

        let later_work = json!([{ "path": "/n/busy.jl", "pending_run": ["d"], "running": ["b", "d"], "execution_allowed": true }]);
        assert_eq!(texts(&still_true(&said, &later_work)), ["1 cell in busy.jl is still running"], "a later run of other cells isn't added");

        let closed = json!([]);
        assert!(still_true(&said, &closed).is_empty());
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
    fn reads_plain_and_chunked_http_bodies() {
        assert_eq!(super::http_body(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").unwrap(), b"hello");
        assert_eq!(super::http_body(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nhel\r\n2\r\nlo\r\n0\r\n\r\n").unwrap(), b"hello");
        assert_eq!(super::http_body(b"HTTP/1.1 404 Not Found\r\n\r\nno").unwrap_err(), "The notebook server answered 404");
    }

    #[test]
    fn nothing_to_say_for_clean_or_unexpected_input() {
        assert!(run_warnings(&json!([{ "path": "/n/a.jl", "pending_run": [], "running": [] }])).is_empty());
        assert!(run_warnings(&json!({ "error": "x" })).is_empty());
    }
}

/// Live check against a running bridge: `ENDEAVOR_TEST_MCP_URL=http://127.0.0.1:PORT/mcp
/// ENDEAVOR_TEST_TOKEN=… cargo test -- --ignored live_bridge`.
#[cfg(test)]
#[test]
#[ignore]
fn live_bridge() {
    let url = std::env::var("ENDEAVOR_TEST_MCP_URL").expect("ENDEAVOR_TEST_MCP_URL");
    let token = std::env::var("ENDEAVOR_TEST_TOKEN").unwrap_or_default();
    let bridge = Bridge { url, page_token: token.clone(), token };
    let list = call_tool(&bridge, "list_notebooks", json!({})).expect("list_notebooks");
    println!("list_notebooks: {list}\nwarnings: {:?}", run_warnings(&list));
    assert!(list.as_array().is_some_and(|a| a.iter().all(|nb| nb.get("pending_run").is_some())));
}
