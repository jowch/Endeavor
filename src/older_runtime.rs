//! A runtime that another Endeavor started: one started before the app was
//! updated, still running on its host (docs/remote-sessions.md). The app uses
//! it as it is when its core offers the app's interface
//! (`endeavor_mcp::CORE_INTERFACE`), the rule EndeavorMCP's own clients use,
//! or when the app's own build started it. Any other runtime gets a note in
//! each session on its host until it restarts; the app holds nothing back.

use crate::agent::Agent;

/// Whether the app can use, as it is, a runtime that build `build` started
/// and whose core offers `interface`; `app` is the app's own build. False
/// when neither is known.
pub fn usable_as_is(build: Option<&str>, interface: Option<u32>, app: Option<&str>) -> bool {
    interface == Some(endeavor_mcp::CORE_INTERFACE) || (build.is_some() && build == app)
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
/// is, whose core offers `interface`.
pub fn note(host: &str, agent: Agent, interface: Option<u32>) -> String {
    format!(
        "Julia on {host} was started by {} version of Endeavor. Restart Julia to use this one. Until then, some of {}'s notebook tools may not work as described, and Ask to run may not ask before a run.",
        which_version(interface),
        agent.name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use endeavor_mcp::CORE_INTERFACE;

    #[test]
    fn a_runtime_is_used_as_it_is_when_it_offers_the_apps_interface_or_is_the_apps_build() {
        assert!(usable_as_is(Some("1.0.0-abc"), Some(CORE_INTERFACE), Some("1.0.0-def")), "another build, the same interface");
        assert!(usable_as_is(Some("1.0.0-abc"), None, Some("1.0.0-abc")), "the app's own build");
        assert!(!usable_as_is(Some("1.0.0-abc"), None, Some("1.0.0-def")), "too old to say its interface");
        assert!(!usable_as_is(Some("1.0.0-abc"), Some(CORE_INTERFACE + 1), Some("1.0.0-def")));
        assert!(!usable_as_is(None, None, None), "neither is known");
    }

    #[test]
    fn the_note_says_which_version_and_names_the_session_agent() {
        assert_eq!(
            note("lab", Agent::Claude, None),
            "Julia on lab was started by an older version of Endeavor. Restart Julia to use this one. Until then, some of Claude's notebook tools may not work as described, and Ask to run may not ask before a run."
        );
        assert!(note("lab", Agent::Codex, Some(CORE_INTERFACE + 1)).starts_with("Julia on lab was started by a newer version"));
        assert!(note("lab", Agent::Codex, Some(CORE_INTERFACE + 1)).contains("some of Codex's notebook tools"));
    }
}
