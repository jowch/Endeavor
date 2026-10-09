//! A runtime that another Endeavor started: one started before the app was
//! updated, still running on its host (docs/remote-sessions.md). The app uses
//! it as it is when its core offers the app's interface
//! (`endeavor_mcp::CORE_INTERFACE`), the rule EndeavorMCP's own clients use,
//! or when the app's own build started it. Any other runtime gets a note in
//! each session on its host until it restarts. One kind is held back further:
//! a runtime too old to ask the user before a run runs no code (`refusal`).

use serde_json::Value;

use crate::agent::Agent;

/// How the app can use a runtime, from what its `Ready` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    /// It offers the app's interface, or the app's own build started it.
    Usable,
    /// Another Endeavor started it, whose core offers this interface (none
    /// from a core too old to say). It holds runs for the user's answer.
    Other(Option<u32>),
    /// It says neither its build nor its interface: a build from before the
    /// core recorded its build (EndeavorMCP 808662c, 2026-10-03), which may
    /// be from before the runtime held runs for the user's answer (b0cab29,
    /// 2026-10-02). Runtimes started between the two count too, to be safe.
    NoRunGate,
}

impl Version {
    /// Of a runtime that build `build` started and whose core offers
    /// `interface`; `app` is the app's own build. This is
    /// `endeavor_mcp::usable_as_is` with the app's build in place of the
    /// library's: the app names a build by `remote::version()`, which the
    /// app's runtimes report, not by the library's `BUILD_VERSION`.
    pub fn of(build: Option<&str>, interface: Option<u32>, app: Option<&str>) -> Version {
        if interface == Some(endeavor_mcp::CORE_INTERFACE) || (build.is_some() && build == app) {
            Version::Usable
        } else if build.is_none() && interface.is_none() {
            Version::NoRunGate
        } else {
            Version::Other(interface)
        }
    }

    /// Of a server's runtime, which the library's helper started: the
    /// library's own build is the one this app runs there.
    pub fn of_server(runtime: &endeavor_mcp::client::RuntimeInfo) -> Version {
        if runtime.usable_as_is() {
            Version::Usable
        } else if runtime.build.is_none() && runtime.interface.is_none() {
            Version::NoRunGate
        } else {
            Version::Other(runtime.interface)
        }
    }

    pub fn usable(self) -> bool {
        self == Version::Usable
    }
}

/// How a core that offers `interface` compares with the app's: "an older",
/// "a newer" or "another". A core that says no interface is from before the
/// number existed.
fn which_version(interface: Option<u32>) -> &'static str {
    match interface {
        None => "an older",
        Some(theirs) if theirs < endeavor_mcp::CORE_INTERFACE => "an older",
        Some(theirs) if theirs > endeavor_mcp::CORE_INTERFACE => "a newer",
        Some(_) => "another",
    }
}

/// The note in each session on a host whose runtime the app can't use as it
/// is; none for one it can.
pub fn note(host: &str, agent: Agent, version: Version) -> Option<String> {
    match version {
        Version::Usable => None,
        Version::Other(interface) => Some(format!(
            "Julia on {host} was started by {} version of Endeavor. Restart Julia to use this one. Until then, some of {}'s notebook tools may not work as described.",
            which_version(interface),
            agent.name()
        )),
        Version::NoRunGate => Some(format!(
            "Julia on {host} was started by a version of Endeavor too old to ask before a run. Restart Julia to run code. Until then, {} can read and edit the notebook but not run it.",
            agent.name()
        )),
    }
}

/// The note when the host's runtime, which had one of `note`'s, is replaced
/// by one the app uses as it is.
pub fn restarted(host: &str) -> String {
    format!("Julia on {host} now runs this version of Endeavor. Ask to run asks before each run again.")
}

/// Why the app refuses an agent's call to `tool` with `arguments` on a
/// runtime of `version`, as the tool error the agent reads: a runtime that
/// may not ask before a run runs no code, in any mode.
pub fn refusal(version: Version, tool: &str, arguments: &Value) -> Option<String> {
    (version == Version::NoRunGate && endeavor_mcp::runs_code(tool, arguments)).then(|| refused("This notebook's Julia"))
}

/// The same refusal on `host`'s listener (`client::Messages::no_run_gate`), where the library makes it.
pub fn no_run_gate(host: &str) -> String {
    refused(&format!("Julia on {host}"))
}

fn refused(whose: &str) -> String {
    format!(
        "ArgumentError: older_runtime::{whose} was started by a version of Endeavor too old to ask the user before a run, so Endeavor doesn't let it run code. \
         Don't run code: tell the user to restart Julia. Reading and editing cells still work."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use endeavor_mcp::CORE_INTERFACE;
    use serde_json::json;

    #[test]
    fn a_runtime_is_used_as_it_is_when_it_offers_the_apps_interface_or_is_the_apps_build() {
        let app = Some("1.0.0-def");
        assert_eq!(Version::of(Some("1.0.0-abc"), Some(CORE_INTERFACE), app), Version::Usable, "another build, the same interface");
        assert_eq!(Version::of(Some("1.0.0-def"), None, app), Version::Usable, "the app's own build");
        assert_eq!(Version::of(Some("1.0.0-abc"), None, app), Version::Other(None), "a build that recorded itself but not its interface");
        assert_eq!(Version::of(None, Some(CORE_INTERFACE + 1), app), Version::Other(Some(CORE_INTERFACE + 1)));
        assert_eq!(Version::of(None, None, app), Version::NoRunGate, "too old to say either");
        assert_eq!(Version::of(None, None, None), Version::NoRunGate, "neither known, even when the app's own build isn't");
    }

    #[test]
    fn the_note_says_which_version_and_names_the_session_agent() {
        assert_eq!(note("lab", Agent::Claude, Version::Usable), None);
        assert_eq!(
            note("lab", Agent::Claude, Version::Other(None)).unwrap(),
            "Julia on lab was started by an older version of Endeavor. Restart Julia to use this one. Until then, some of Claude's notebook tools may not work as described."
        );
        assert!(note("lab", Agent::Codex, Version::Other(Some(CORE_INTERFACE + 1))).unwrap().starts_with("Julia on lab was started by a newer version"));
        assert_eq!(
            note("lab", Agent::Codex, Version::NoRunGate).unwrap(),
            "Julia on lab was started by a version of Endeavor too old to ask before a run. Restart Julia to run code. Until then, Codex can read and edit the notebook but not run it."
        );
    }

    #[test]
    fn a_runtime_with_no_run_gate_runs_no_code_in_any_mode() {
        let refused = |version: Version, tool: &str, arguments: Value| refusal(version, tool, &arguments).is_some();
        assert!(refused(Version::NoRunGate, "execute_cell", json!({ "cell_id": "a" })));
        assert!(refused(Version::NoRunGate, "edit_cell", json!({ "cell_id": "a", "code": "1", "run_after": true })));
        assert!(refused(Version::NoRunGate, "run_shell", json!({ "command": "ls" })));
        assert!(!refused(Version::NoRunGate, "read_cell", json!({ "cell_id": "a" })));
        assert!(!refused(Version::NoRunGate, "edit_cell", json!({ "cell_id": "a", "code": "1" })), "edits still go through");
        assert!(!refused(Version::Other(None), "execute_cell", json!({ "cell_id": "a" })), "it holds runs itself");
        assert!(!refused(Version::Other(Some(CORE_INTERFACE + 1)), "execute_cell", json!({ "cell_id": "a" })));
        assert!(!refused(Version::Usable, "execute_cell", json!({ "cell_id": "a" })));
    }
}
