//! The execution gate: the agent asks before it runs notebook code. Claude Code
//! calls `endeavor hook-pretool` before each pluto tool (plugin/hooks/hooks.json);
//! for anything that runs code the hook answers "ask", which reaches the app as an
//! ACP permission request. "Allow & stop asking" is remembered by the app, so no
//! approval is written into the user's settings files.

use std::io::Read;

use serde_json::{Value, json};

/// Does this pluto tool call run notebook code?
pub fn runs_code(tool: &str, input: &Value) -> bool {
    match tool {
        // delete_cell re-runs the deleted cell's dependents (and can't be undone).
        "execute_cell" | "submit_changes" | "run_all_cells" | "allow_execution" | "delete_cell" => true,
        "add_cell" | "edit_cell" => input["run_after"].as_bool() == Some(true),
        _ => false,
    }
}

/// The hook's answer for one Claude Code PreToolUse payload: "ask" for pluto
/// calls that run code, nothing (normal flow) otherwise.
pub fn pretool_decision(payload: &Value) -> Option<Value> {
    let tool = payload["tool_name"].as_str()?.strip_prefix("mcp__pluto__")?;
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

/// Is this permission request one the gate raised (a pluto tool)?
pub fn is_pluto(title: &str) -> bool {
    title.starts_with("mcp__pluto__")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(tool: &str, input: Value) -> bool {
        pretool_decision(&json!({ "tool_name": tool, "tool_input": input })).is_some()
    }

    #[test]
    fn asks_only_before_running_code() {
        assert!(decide("mcp__pluto__execute_cell", json!({})));
        assert!(decide("mcp__pluto__submit_changes", json!({})));
        assert!(decide("mcp__pluto__allow_execution", json!({})));
        assert!(decide("mcp__pluto__delete_cell", json!({})));
        assert!(decide("mcp__pluto__add_cell", json!({ "code": "1", "run_after": true })));
        assert!(!decide("mcp__pluto__add_cell", json!({ "code": "1" })));
        assert!(!decide("mcp__pluto__edit_cell", json!({ "code": "1", "run_after": false })));
        assert!(!decide("mcp__pluto__edit_cells", json!({ "cells": [] })));
        assert!(!decide("mcp__pluto__read_cell", json!({})));
        assert!(!decide("Bash", json!({ "command": "rm -rf /" })), "only pluto tools are gated here");
    }

    #[test]
    fn decision_is_claude_code_ask_json() {
        let d = pretool_decision(&json!({ "tool_name": "mcp__pluto__run_all_cells", "tool_input": {} })).unwrap();
        assert_eq!(d["hookSpecificOutput"]["permissionDecision"], "ask");
        assert_eq!(d["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    }
}
