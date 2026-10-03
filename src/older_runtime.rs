//! A runtime from another Endeavor build than the app: one started before
//! the app was updated, still running on its host (docs/remote-sessions.md).
//! The runtime reports the build it came from (`build` in its event stream;
//! a runtime too old to report one counts as older). Such a runtime gets a
//! note in each session on its host, and until it restarts the app holds
//! back what it can't do safely. Each such limit is one rule in `refusal`.

use serde_json::Value;

/// Whether a runtime that reported `reported` came from a build other than
/// the app's own, `app`.
pub fn is_older(reported: Option<&str>, app: Option<&str>) -> bool {
    reported.is_none() || reported != app
}

/// The note in each session on a host whose runtime is older.
pub fn note(host: &str) -> String {
    format!("Julia on {host} is from an older Endeavor. Restart Julia to get the latest changes. Until then, Ask to run doesn't let Claude run code.")
}

/// Why the app refuses an agent's call to `tool` with `arguments` on an older
/// runtime, in a session whose mode is `mode` (`Session::guard_mode`:
/// "manual", "ask", "auto" or "plan"), as the tool error the agent reads.
pub fn refusal(mode: &str, tool: &str, arguments: &Value) -> Option<String> {
    match mode {
        // Asking before a run is the runtime's job now, and an older runtime
        // doesn't ask.
        "ask" if endeavor_mcp::runs_code(tool, arguments) => Some(
            "ArgumentError: older_runtime::This notebook's Julia is from an older version of Endeavor, which can't ask the user before a run. \
             Don't run code: tell the user to restart Julia, or to switch to Auto to let runs go ahead without asking."
                .into(),
        ),
        // Nor before an edit in Manual: the agent's own prompt asks instead,
        // as it did before the runtime held edits, so the agent's own
        // settings decide what asks.
        "manual" => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_runtime_is_older_unless_it_reports_the_apps_build() {
        assert!(!is_older(Some("1.0.0-abc"), Some("1.0.0-abc")));
        assert!(is_older(Some("1.0.0-abc"), Some("1.0.0-def")));
        assert!(is_older(None, Some("1.0.0-abc")), "too old to say");
    }

    #[test]
    fn an_older_runtime_runs_nothing_in_ask_to_run() {
        let refused = |policy: &str, tool: &str, arguments: Value| refusal(policy, tool, &arguments).is_some();
        assert!(refused("ask", "execute_cell", json!({ "cell_id": "a" })));
        assert!(refused("ask", "edit_cell", json!({ "cell_id": "a", "code": "1", "run_after": true })));
        assert!(refused("ask", "run_shell", json!({ "command": "ls" })));
        assert!(!refused("ask", "edit_cell", json!({ "cell_id": "a", "code": "1" })), "edits still go through");
        assert!(!refused("ask", "read_cell", json!({ "cell_id": "a" })));
        assert!(!refused("auto", "execute_cell", json!({ "cell_id": "a" })), "Auto runs without asking anyway");
        assert!(!refused("manual", "execute_cell", json!({ "cell_id": "a" })), "in Manual the agent asks first itself");
        assert!(!refused("manual", "edit_cell", json!({ "cell_id": "a", "code": "1" })), "and before an edit");
        assert!(!refused("plan", "execute_cell", json!({ "cell_id": "a" })), "the runtime refuses runs in Plan itself");
    }
}
