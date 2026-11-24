use serde_json::Value;
use wire::backend::Backend;

use crate::agent::Agent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Usable,
    Other(Option<u32>),
    NoRunGate,
}

impl Version {
    pub fn of(build: Option<&str>, interface: Option<u32>, app: Option<&str>) -> Version {
        if interface == Some(endeavor_mcp::CORE_INTERFACE) || (build.is_some() && build == app) {
            Version::Usable
        } else if build.is_none() && interface.is_none() {
            Version::NoRunGate
        } else {
            Version::Other(interface)
        }
    }

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

    pub fn has_julia_status(self) -> bool {
        match self {
            Version::Usable => endeavor_mcp::CORE_INTERFACE >= 5,
            Version::Other(interface) => interface.is_some_and(|i| i >= 5),
            Version::NoRunGate => false,
        }
    }

    pub fn has_session_kinds(self) -> bool {
        match self {
            Version::Usable => true,
            Version::Other(interface) => interface.is_some_and(|i| i >= 6),
            Version::NoRunGate => false,
        }
    }

    pub fn shows_python_in_the_app(self) -> bool {
        const SHOWS_PYTHON: u32 = 49;
        match self {
            Version::Usable => endeavor_mcp::CORE_INTERFACE >= SHOWS_PYTHON,
            Version::Other(interface) => interface.is_some_and(|i| i >= SHOWS_PYTHON),
            Version::NoRunGate => false,
        }
    }

    pub fn exports_python(self) -> bool {
        const EXPORTS_PYTHON: u32 = 50;
        match self {
            Version::Usable => endeavor_mcp::CORE_INTERFACE >= EXPORTS_PYTHON,
            Version::Other(interface) => interface.is_some_and(|i| i >= EXPORTS_PYTHON),
            Version::NoRunGate => false,
        }
    }

    pub fn reads_python_packages(self) -> bool {
        const READS_PYTHON_PACKAGES: u32 = 51;
        match self {
            Version::Usable => endeavor_mcp::CORE_INTERFACE >= READS_PYTHON_PACKAGES,
            Version::Other(interface) => interface.is_some_and(|i| i >= READS_PYTHON_PACKAGES),
            Version::NoRunGate => false,
        }
    }

    pub fn has_r_status(self) -> bool {
        match self {
            Version::Usable => endeavor_mcp::CORE_INTERFACE >= 9,
            Version::Other(interface) => interface.is_some_and(|i| i >= 9),
            Version::NoRunGate => false,
        }
    }
}

fn which_version(interface: Option<u32>) -> &'static str {
    match interface {
        None => "an older",
        Some(theirs) if theirs < endeavor_mcp::CORE_INTERFACE => "an older",
        Some(theirs) if theirs > endeavor_mcp::CORE_INTERFACE => "a newer",
        Some(_) => "another",
    }
}

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

pub fn restarted(host: &str) -> String {
    format!("Julia on {host} now runs this version of Endeavor. Ask to run asks before each run again.")
}

pub fn refusal(version: Version, host: &str, kind: Backend, tool: &str, arguments: &Value) -> Option<String> {
    if version == Version::NoRunGate && endeavor_mcp::runs_code(tool, arguments) {
        return Some(refused("This notebook's Julia"));
    }
    if !version.shows_python_in_the_app() && opens_python(tool, arguments, kind) {
        return Some(python_needs_a_restart(host));
    }
    let pathless = arguments["path"].as_str().is_none_or(|path| path.trim().is_empty());
    (tool == "new_notebook" && pathless && kind != Backend::Pluto && !version.has_session_kinds()).then(|| not_of_its_kind(host, kind))
}

fn opens_python(tool: &str, arguments: &Value, kind: Backend) -> bool {
    let path = arguments["path"].as_str().filter(|path| !path.trim().is_empty());
    matches!(tool, "open_notebook" | "new_notebook") && path.map_or(tool == "new_notebook" && kind == Backend::Marimo, |path| crate::new_session::kind_of_path(path) == Backend::Marimo)
}

fn python_needs_a_restart(host: &str) -> String {
    format!(
        "ArgumentError: older_runtime::Endeavor on {host} was started by an older version, which doesn't show Python notebooks in the app. \
         Don't stop it yourself: call request_restart, which asks the user whether to restart Endeavor on {host}. That closes its open notebooks, and starts this version."
    )
}

fn not_of_its_kind(host: &str, kind: Backend) -> String {
    let restart = format!(
        "don't stop it yourself: call request_restart, which asks the user whether to restart Endeavor on {host}. That closes its open notebooks, and starts this version."
    );
    let (language, way) = match kind {
        Backend::Marimo => ("Python", format!("To make one, {restart}")),
        _ => ("R", format!("Call new_notebook again with a path ending in .R, in this session's folder. If that is refused too, {restart}")),
    };
    format!(
        "ArgumentError: older_runtime::Endeavor on {host} was started by an older version, which makes a notebook without a path in Julia, so it can't make this session's {language} notebook that way. {way}"
    )
}

pub fn no_run_gate(host: &str) -> String {
    refused(&format!("Julia on {host}"))
}

fn refused(whose: &str) -> String {
    format!(
        "ArgumentError: older_runtime::{whose} was started by a version of Endeavor too old to ask the user before a run, so Endeavor doesn't let it run code. \
         Don't run code, and don't stop it yourself: call request_restart, which asks the user whether to restart Endeavor there. Reading and editing cells still work."
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
        let refused = |version: Version, tool: &str, arguments: Value| refusal(version, "lab", Backend::Pluto, tool, &arguments).is_some();
        assert!(refused(Version::NoRunGate, "execute_cell", json!({ "cell_id": "a" })));
        assert!(refused(Version::NoRunGate, "edit_cell", json!({ "cell_id": "a", "code": "1", "run_after": true })));
        assert!(refused(Version::NoRunGate, "run_shell", json!({ "command": "ls" })));
        assert!(!refused(Version::NoRunGate, "read_cell", json!({ "cell_id": "a" })));
        assert!(!refused(Version::NoRunGate, "edit_cell", json!({ "cell_id": "a", "code": "1" })), "edits still go through");
        assert!(!refused(Version::Other(None), "execute_cell", json!({ "cell_id": "a" })), "it holds runs itself");
        assert!(!refused(Version::Other(Some(CORE_INTERFACE + 1)), "execute_cell", json!({ "cell_id": "a" })));
        assert!(!refused(Version::Usable, "execute_cell", json!({ "cell_id": "a" })));
        let why = refusal(Version::NoRunGate, "lab", Backend::Pluto, "execute_cell", &json!({ "cell_id": "a" })).unwrap();
        assert!(why.contains("call request_restart") && why.contains("don't stop it yourself"), "the agent asks the user, and only that: {why}");
    }

    #[test]
    fn a_runtime_from_before_the_python_pane_has_the_user_restart_it_for_a_python_notebook() {
        let refused = |version: Version, tool: &str, kind: Backend, arguments: Value| refusal(version, "lab", kind, tool, &arguments);
        let why = refused(Version::Other(Some(48)), "open_notebook", Backend::Pluto, json!({ "path": "/w/fit.py" })).unwrap();
        assert!(why.contains("doesn't show Python notebooks in the app") && why.contains("call request_restart"), "{why}");
        assert!(refused(Version::Other(Some(48)), "new_notebook", Backend::Marimo, json!({})).is_some(), "a Python session's new notebook");
        assert!(refused(Version::Other(Some(48)), "open_notebook", Backend::Marimo, json!({ "path": "/w/fit.jl" })).is_none(), "the path decides");
        assert!(refused(Version::Other(Some(49)), "open_notebook", Backend::Pluto, json!({ "path": "/w/fit.py" })).is_none());
        assert!(refused(Version::Usable, "new_notebook", Backend::Marimo, json!({ "path": "/w/fit.py" })).is_none());
        assert!(refused(Version::Other(Some(48)), "read_cell", Backend::Marimo, json!({ "path": "/w/fit.py" })).is_none());
    }

    #[test]
    fn a_runtime_without_session_kinds_refuses_an_r_or_python_sessions_pathless_new_notebook() {
        let why = |version: Version, kind: Backend, arguments: Value| refusal(version, "lab", kind, "new_notebook", &arguments);
        let r = why(Version::Other(Some(5)), Backend::Ember, json!({})).unwrap();
        assert!(r.starts_with("ArgumentError: older_runtime::Endeavor on lab was started by an older version") && r.contains("R notebook"), "{r}");
        assert!(r.contains("a path ending in .R") && r.contains("call request_restart") && r.contains("don't stop it yourself"), "{r}");
        let python = why(Version::Other(None), Backend::Marimo, json!({ "path": null })).unwrap();
        assert!(python.contains("Python notebook") && python.contains("call request_restart") && !python.contains(".R"), "{python}");
        assert!(why(Version::NoRunGate, Backend::Ember, json!({ "path": " " })).is_some(), "an empty path is no path");
        assert!(why(Version::Other(Some(5)), Backend::Ember, json!({ "path": "a.R" })).is_none(), "a path says its kind");
        assert!(why(Version::Other(Some(5)), Backend::Pluto, json!({})).is_none());
        assert!(why(Version::Other(Some(6)), Backend::Ember, json!({})).is_none(), "interface 6 keeps session kinds");
        assert!(why(Version::Other(Some(CORE_INTERFACE + 1)), Backend::Ember, json!({})).is_none());
        assert!(why(Version::Usable, Backend::Marimo, json!({})).is_none());
        assert!(refusal(Version::Other(Some(5)), "lab", Backend::Ember, "list_notebooks", &json!({})).is_none());
    }
}
