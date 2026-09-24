//! The app's own line to PlutoMCP: its bridge's loopback `/call` endpoint (MCP
//! JSON-RPC). Used for app-side checks that shouldn't depend on the agent, like
//! the end-of-turn run-state warning.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

/// Call a PlutoMCP tool and return its decoded JSON result.
pub fn call_tool(mcp_url: &str, tool: &str, arguments: Value) -> Result<Value, String> {
    // mcp_url is `http://127.0.0.1:PORT/sse`.
    let host = mcp_url
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .ok_or_else(|| format!("bad MCP url {mcp_url}"))?;
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    })
    .to_string();

    let mut stream = TcpStream::connect(host).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    // HTTP/1.0: the bridge sends no Content-Length, so read the body to EOF
    // instead of dealing with chunked encoding.
    write!(
        stream,
        "POST /call HTTP/1.0\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|e| e.to_string())?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;

    let (_, payload) = response.split_once("\r\n\r\n").ok_or("malformed HTTP response")?;
    let rpc: Value = serde_json::from_str(payload).map_err(|e| format!("bad JSON-RPC reply: {e}"))?;
    let text = rpc["result"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| format!("tool error: {}", rpc["result"]))?;
    serde_json::from_str(text).map_err(|e| format!("bad tool result: {e}"))
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

#[cfg(test)]
mod tests {
    use super::run_warnings;
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
    fn nothing_to_say_for_clean_or_unexpected_input() {
        assert!(run_warnings(&json!([{ "path": "/n/a.jl", "pending_run": [], "running": [] }])).is_empty());
        assert!(run_warnings(&json!({ "error": "x" })).is_empty());
    }
}

/// Live check against a running bridge: `ENDEAVOR_TEST_MCP_URL=http://127.0.0.1:PORT/sse
/// cargo test -- --ignored live_bridge`.
#[cfg(test)]
#[test]
#[ignore]
fn live_bridge() {
    let url = std::env::var("ENDEAVOR_TEST_MCP_URL").expect("ENDEAVOR_TEST_MCP_URL");
    let list = call_tool(&url, "list_notebooks", json!({})).expect("list_notebooks");
    println!("list_notebooks: {list}\nwarnings: {:?}", run_warnings(&list));
    assert!(list.as_array().is_some_and(|a| a.iter().all(|nb| nb.get("pending_run").is_some())));
}
