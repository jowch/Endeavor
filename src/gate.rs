//! The execution gate: the agent asks before it runs notebook code. Claude Code
//! calls `endeavor hook-pretool` before each notebook tool (plugin/hooks/hooks.json);
//! for anything that runs code the hook answers "ask", which reaches the app as an
//! ACP permission request. "Always this session" is remembered by the app, so no
//! approval is written into the user's settings files.

use std::io::Read;

use serde_json::{Value, json};

pub use endeavor_remote::runs_code;

/// The hook's answer for one Claude Code PreToolUse payload: "ask" for notebook
/// calls that run code, nothing (normal flow) otherwise.
pub fn pretool_decision(payload: &Value) -> Option<Value> {
    let tool = crate::celldiff::notebook_tool(payload["tool_name"].as_str()?)?;
    runs_code(tool, &payload["tool_input"]).then(|| {
        json!({ "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "ask",
            "permissionDecisionReason": "Runs notebook code; Endeavor asks first.",
        } })
    })
}

/// `endeavor hook-pretool`: read the hook payload on stdin, print the decision.
pub fn run_pretool_hook() -> ! {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    // Unparseable input: say nothing, so Claude Code's normal flow applies.
    if let Some(decision) = serde_json::from_str(&input).ok().as_ref().and_then(pretool_decision) {
        println!("{decision}");
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(tool: &str, input: Value) -> bool {
        pretool_decision(&json!({ "tool_name": tool, "tool_input": input })).is_some()
    }

    #[test]
    fn asks_only_before_running_code() {
        assert!(decide("mcp__notebook__execute_cell", json!({})));
        assert!(decide("mcp__notebook__submit_changes", json!({})));
        assert!(decide("mcp__notebook__allow_execution", json!({})));
        assert!(decide("mcp__notebook__delete_cell", json!({})));
        assert!(decide("mcp__notebook__run_shell", json!({ "command": "uname -a" })));
        assert!(!decide("mcp__notebook__read_file", json!({ "path": "/tmp/x" })));
        assert!(decide("mcp__notebook__add_cell", json!({ "code": "1", "run_after": true })));
        assert!(!decide("mcp__notebook__add_cell", json!({ "code": "1" })));
        assert!(!decide("mcp__notebook__edit_cell", json!({ "code": "1", "run_after": false })));
        assert!(!decide("mcp__notebook__edit_cells", json!({ "cells": [] })));
        assert!(!decide("mcp__notebook__read_cell", json!({})));
        assert!(!decide("Bash", json!({ "command": "rm -rf /" })), "only notebook tools are gated here");
    }

    #[test]
    fn decision_is_claude_code_ask_json() {
        let d = pretool_decision(&json!({ "tool_name": "mcp__notebook__run_all_cells", "tool_input": {} })).unwrap();
        assert_eq!(d["hookSpecificOutput"]["permissionDecision"], "ask");
        assert_eq!(d["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    }
}
