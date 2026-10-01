//! The slash-command list: Endeavor's own commands (/mode, /model), then the
//! commands and skills Claude Code sends, minus the ones that don't belong in
//! a notebook app. What's typed after "/" filters them by name, then by
//! description. A command name followed by a space is one piece (a token).

use std::ops::Range;

use agent_client_protocol::schema::v1::{AvailableCommand, AvailableCommandInput};

/// The list's keys, spelled out on Linux.
#[cfg(target_os = "macos")]
pub const FOOTER: &str = "↑↓ choose · ⏎ run · ⇥ fill in · esc close";
#[cfg(not(target_os = "macos"))]
pub const FOOTER: &str = "↑↓ choose · Enter run · Tab fill in · Esc close";
pub const ENTER: &str = if cfg!(target_os = "macos") { "⏎" } else { "Enter" };
/// /mode's key: the next mode.
pub const SHIFT_TAB: &str = if cfg!(target_os = "macos") { "⇧⇥" } else { "Shift+Tab" };

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Endeavor,
    /// Claude Code's commands and skills: ACP doesn't say which is which.
    ClaudeCode,
}

impl Group {
    pub fn heading(self) -> &'static str {
        match self {
            Group::Endeavor => "Endeavor",
            Group::ClaudeCode => "Claude Code",
        }
    }
}

/// Endeavor's own commands: they act in the app at once and are never sent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Own {
    Mode,
    Model,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub group: Group,
    pub own: Option<Own>,
    /// The faint word after the name in the list ("instructions").
    pub hint: Option<String>,
    /// The faint words after the caret once it's filled in. None: the
    /// command takes nothing, so ⏎ runs it.
    pub words: Option<String>,
    /// Not listed, but still a command when typed in full.
    pub hidden: bool,
}

/// Claude Code's commands the list leaves out. Typed in full they still go to
/// Claude Code as written.
const HIDDEN: &[&str] = &[
    // New session does what /clear does.
    "clear",
    // Endeavor's toolbar sets these; /mode and /model replace the first two.
    "model",
    "plan",
    "effort",
    "fast",
    // For code projects: branches, pull requests, an app to drive.
    "review",
    "security-review",
    "pr-comments",
    "ultrareview",
    "batch",
    "autofix-pr",
    "simplify",
    "code-review",
    "run",
    "run-skill-generator",
    "verify",
    "design-sync",
    // Claude Code's own setup, files and terminal.
    "add-dir",
    "output-style",
    "update-config",
    "fewer-permission-prompts",
    "debug",
    "claude-api",
    "insights",
    "team-onboarding",
    "loop",
    "schedule",
    "heapdump",
    "agents",
    "list-agents",
    "doctor",
    "skill-doctor",
    "auto-mode-setup",
    "autocompact",
    "advisor",
    "color",
    "config",
    "mcp",
    "import",
    "reload-plugins",
    "reload-skills",
    "rename",
    "workflow-authoring",
    "workflow-launch-exec",
    "__remote-workflow",
    "extra-usage",
    "design",
    "design-consent",
    "design-revoke",
];

pub fn hidden(name: &str) -> bool {
    // Endeavor's own skills are for Claude, which loads them itself.
    HIDDEN.contains(&name) || name.starts_with("endeavor:")
}

/// Endeavor's wording for a command's words where Claude Code's hint is
/// missing or reads as syntax: (list hint, hint after the caret).
fn wording(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "compact" => ("instructions", "instructions for the summary (optional)"),
        "goal" => ("condition", "what has to be true before Claude stops"),
        _ => return None,
    })
}

/// Claude Code's hint as words: brackets and "|" taken out.
fn plain_hint(hint: &str) -> String {
    let kept: String = hint.chars().filter(|c| !"<>[]()".contains(*c)).map(|c| if c == '|' { ' ' } else { c }).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn endeavor() -> Vec<Command> {
    let own = |name: &str, description: &str, hint: &str, own| Command {
        name: name.into(),
        description: description.into(),
        group: Group::Endeavor,
        own: Some(own),
        hint: Some(hint.into()),
        words: None,
        hidden: false,
    };
    vec![
        own("mode", "Change what Claude may do without asking", "mode", Own::Mode),
        own("model", "Choose the model for this session", "name", Own::Model),
    ]
}

/// Every command, listed or not: Endeavor's, then Claude Code's in the order
/// it sent them (None: not sent yet). Claude Code's /model and /mode lose to
/// Endeavor's.
pub fn commands(agent: Option<&[AvailableCommand]>) -> Vec<Command> {
    let mut all = endeavor();
    for c in agent.unwrap_or_default() {
        if all.iter().any(|own| own.name == c.name) {
            continue;
        }
        let hint = match &c.input {
            Some(AvailableCommandInput::Unstructured(i)) => Some(plain_hint(&i.hint)).filter(|h| !h.is_empty()),
            _ => None,
        };
        let (hint, words) = match wording(&c.name) {
            Some((short, long)) => (Some(short.to_string()), Some(long.to_string())),
            None => (hint.clone(), hint),
        };
        all.push(Command { name: c.name.clone(), description: c.description.clone(), group: Group::ClaudeCode, own: None, hint, words, hidden: hidden(&c.name) });
    }
    all
}

/// One row of the filtered list, and the byte ranges to show brighter: in
/// the name (without its "/"), or in the description.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub command: usize,
    pub marks: Vec<Range<usize>>,
}

#[derive(Debug, PartialEq)]
pub enum Listing {
    /// Names match (all of them for an empty query).
    Names(Vec<Row>),
    /// No name matches; these descriptions do.
    Descriptions(Vec<Row>),
    Nothing,
}

/// How a name matches the query: 0 starts with it, 1 contains it, 2 has its
/// letters in order. With the byte ranges of the matched letters.
pub fn name_match(name: &str, query: &str) -> Option<(u8, Vec<Range<usize>>)> {
    let lower = name.to_lowercase();
    let query = query.to_lowercase();
    if lower.len() != name.len() {
        return None;
    }
    if lower.starts_with(&query) {
        return Some((0, if query.is_empty() { Vec::new() } else { vec![0..query.len()] }));
    }
    if let Some(at) = lower.find(&query) {
        return Some((1, vec![at..at + query.len()]));
    }
    let mut marks: Vec<Range<usize>> = Vec::new();
    let mut from = 0;
    for q in query.chars() {
        let at = lower[from..].find(q)? + from;
        let end = at + q.len_utf8();
        match marks.last_mut() {
            Some(last) if last.end == at => last.end = end,
            _ => marks.push(at..end),
        }
        from = end;
    }
    Some((2, marks))
}

/// The listed commands for what's typed after "/": by group, then names
/// starting with it, containing it, with its letters in order.
pub fn filter(query: &str, commands: &[Command]) -> Listing {
    let listed = || commands.iter().enumerate().filter(|(_, c)| !c.hidden);
    let mut named: Vec<(Group, u8, usize, Vec<Range<usize>>)> = listed().filter_map(|(i, c)| name_match(&c.name, query).map(|(tier, marks)| (c.group, tier, i, marks))).collect();
    if !named.is_empty() {
        named.sort_by_key(|(group, tier, i, _)| (*group as u8, *tier, *i));
        return Listing::Names(named.into_iter().map(|(_, _, command, marks)| Row { command, marks }).collect());
    }
    let query = query.to_lowercase();
    let described: Vec<Row> = listed()
        .filter_map(|(command, c)| {
            let lower = c.description.to_lowercase();
            (lower.len() == c.description.len()).then_some(())?;
            let at = lower.find(&query)?;
            Some(Row { command, marks: vec![at..at + query.len()] })
        })
        .collect();
    if described.is_empty() { Listing::Nothing } else { Listing::Descriptions(described) }
}

/// Choices (a mode's or a model's names) for what's typed after the
/// command: all for nothing typed; else those starting with it, then
/// containing it. Indices into `names`.
pub fn choices(query: &str, names: &[String]) -> Vec<usize> {
    let query = query.trim();
    let mut found: Vec<(u8, usize)> = names.iter().enumerate().filter_map(|(i, n)| name_match(n, query).filter(|(tier, _)| *tier < 2).map(|(tier, _)| (tier, i))).collect();
    found.sort();
    found.into_iter().map(|(_, i)| i).collect()
}

/// What's typed after "/" while it's still the command's name: the box
/// starts with "/" and holds no space yet.
pub fn typing(text: &str) -> Option<&str> {
    text.strip_prefix('/').filter(|rest| !rest.contains(char::is_whitespace))
}

/// The command at the start of the text, when its name is followed by a
/// space: one tinted piece. With the byte length of "/name".
pub fn token<'a>(text: &str, commands: &'a [Command]) -> Option<(usize, &'a Command)> {
    let rest = text.strip_prefix('/')?;
    let end = rest.find(char::is_whitespace)?;
    let command = commands.iter().find(|c| c.name == rest[..end])?;
    Some((end + 1, command))
}

/// A message that starts with a slash command, as far as sending goes: its
/// first word is "/name" with no other "/" (a path is not a command).
pub fn is_command(text: &str) -> bool {
    text.strip_prefix('/').and_then(|rest| rest.split(char::is_whitespace).next()).is_some_and(|name| !name.is_empty() && !name.contains('/'))
}

/// Endeavor's command at the start of the text and what follows it, when
/// the text is one: "/model sonnet" → (Model, "sonnet").
pub fn own_command(text: &str) -> Option<(Own, &str)> {
    let rest = text.trim_start().strip_prefix('/')?;
    let (name, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let own = match name {
        "mode" => Own::Mode,
        "model" => Own::Model,
        _ => return None,
    };
    Some((own, after.trim()))
}

/// A description shown from a little before byte `at`, so a match far into
/// it stays in view: where the shown text starts in the original (less the
/// "…" put in front), and the text.
pub fn around(description: &str, at: usize) -> (usize, String) {
    const LEAD: usize = 16;
    if at <= LEAD + 8 {
        return (0, description.to_string());
    }
    let from = description[..at - LEAD].rfind(' ').map_or(at - LEAD, |i| i + 1);
    let from = (from..=at).find(|&i| description.is_char_boundary(i)).unwrap_or(at);
    let ellipsis = "…";
    (from - ellipsis.len(), format!("{ellipsis}{}", &description[from..]))
}

/// What sending "/mode …" or "/model …" does: never send it. It applies
/// the best of the choices (`names` lists them), or opens the list when
/// nothing follows the command or nothing matches.
#[derive(Debug, PartialEq)]
pub enum OwnSend {
    Apply(Own, usize),
    Open(Own),
}

pub fn own_send(text: &str, names: impl Fn(Own) -> Vec<String>) -> Option<OwnSend> {
    let (own, rest) = own_command(text.trim())?;
    Some(match choices(rest, &names(own)).first() {
        Some(&i) if !rest.is_empty() => OwnSend::Apply(own, i),
        _ => OwnSend::Open(own),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(names: &[(&str, Option<&str>, &str)]) -> Vec<AvailableCommand> {
        names
            .iter()
            .map(|(name, hint, description)| {
                let c = AvailableCommand::new(*name, *description);
                match hint {
                    Some(h) => c.input(AvailableCommandInput::Unstructured(agent_client_protocol::schema::v1::UnstructuredCommandInput::new(*h))),
                    None => c,
                }
            })
            .collect()
    }

    /// Part of the list Claude Code sent on 2026-10-01 (adapter 0.81.2).
    fn sample() -> Vec<Command> {
        commands(Some(&agent(&[
            ("dataviz", None, "Use this skill whenever you are about to create ANY chart, graph, plot"),
            ("endeavor:pluto-workflow", None, "(endeavor) Use when editing Pluto notebook cells"),
            ("code-review", Some("[low|medium|high] [--fix]"), "Review the current diff"),
            ("compact", Some("<optional custom summarization instructions>"), "Free up context by summarizing the conversation so far"),
            ("context", None, "Show current context usage"),
            ("config", Some("key=value"), "Set a setting by key"),
            ("model", Some("<model>"), "Set the AI model for Claude Code"),
            ("init", None, "Initialize a new CLAUDE.md file with codebase documentation"),
            ("usage", None, "Show session cost, plan usage, and what's contributing to your limits"),
            ("recap", None, "Generate a one-line session recap now"),
            ("goal", None, "Set a goal — keep working until the condition is met"),
            ("rename", Some("[name]"), "Rename the current conversation"),
        ])))
    }

    fn names(listing: &Listing, commands: &[Command]) -> Vec<String> {
        let rows = match listing {
            Listing::Names(rows) | Listing::Descriptions(rows) => rows,
            Listing::Nothing => return Vec::new(),
        };
        rows.iter().map(|r| format!("/{}", commands[r.command].name)).collect()
    }

    #[test]
    fn slash_alone_lists_everything_shown_in_groups() {
        let all = sample();
        assert_eq!(names(&filter("", &all), &all), ["/mode", "/model", "/dataviz", "/compact", "/context", "/init", "/usage", "/recap", "/goal"]);
        assert!(matches!(filter("", &all), Listing::Names(_)));
    }

    #[test]
    fn names_starting_with_the_text_come_first_then_containing_then_letters_in_order() {
        let all = sample();
        assert_eq!(names(&filter("co", &all), &all), ["/compact", "/context"]);
        // "a": starts /a…: none; contains: dataviz, compact, usage, recap, goal; in order: none extra.
        assert_eq!(names(&filter("a", &all), &all), ["/dataviz", "/compact", "/usage", "/recap", "/goal"]);
        // "ct": contains in /compact? no; /context? no. Letters in order: compact (c…t), context (c…t).
        assert_eq!(names(&filter("ct", &all), &all), ["/compact", "/context"]);
        // "it": starts: none; contains: init; in order: compact? c-o-m-p-a-c-t has no i. dataviz: d-a-t-a-v-i-z no t after i.
        assert_eq!(names(&filter("it", &all), &all), ["/init"]);
        // Endeavor's group stays first even when its match is weaker.
        assert_eq!(names(&filter("mo", &all), &all), ["/mode", "/model"]);
        assert_eq!(names(&filter("de", &all), &all), ["/mode", "/model"]);
    }

    #[test]
    fn matched_letters_are_marked() {
        let all = sample();
        let Listing::Names(rows) = filter("cpt", &all) else { panic!("names") };
        assert_eq!(rows, [Row { command: all.iter().position(|c| c.name == "compact").unwrap(), marks: vec![0..1, 3..4, 6..7] }]);
        let Listing::Names(rows) = filter("Con", &all) else { panic!("names") };
        assert_eq!(rows[0].marks, [0..3]);
    }

    #[test]
    fn with_no_name_matching_descriptions_are_searched() {
        let all = sample();
        let listing = filter("plot", &all);
        assert!(matches!(&listing, Listing::Descriptions(rows) if rows[0].marks == [66..70]));
        assert_eq!(names(&listing, &all), ["/dataviz"]);
        assert_eq!(filter("plto", &all), Listing::Nothing);
    }

    #[test]
    fn a_match_far_into_a_description_stays_in_view() {
        let text = "Use this skill whenever you are about to create ANY chart, graph, plot";
        let (from, shown) = around(text, 66);
        assert_eq!(shown, "…ANY chart, graph, plot");
        assert_eq!(&shown[66 - from..66 - from + 4], "plot");
        assert_eq!(around("Show current context usage", 13), (0, "Show current context usage".to_string()));
    }

    #[test]
    fn hidden_commands_stay_out_of_the_list() {
        let all = sample();
        let shown = names(&filter("", &all), &all);
        for gone in ["/code-review", "/config", "/rename", "/endeavor:pluto-workflow"] {
            assert!(!shown.contains(&gone.to_string()), "{gone}");
        }
        // Claude Code's /model gives way to Endeavor's.
        assert_eq!(all.iter().filter(|c| c.name == "model").map(|c| c.group).collect::<Vec<_>>(), [Group::Endeavor]);
        for name in ["clear", "plan", "review", "security-review", "pr-comments", "ultrareview", "batch", "autofix-pr", "simplify", "run", "add-dir", "output-style", "update-config", "fewer-permission-prompts", "debug", "claude-api", "insights", "team-onboarding", "loop", "schedule", "heapdump", "agents", "endeavor:pluto-session"] {
            assert!(hidden(name), "{name}");
        }
        for name in ["compact", "context", "usage", "recap", "goal", "init", "dataviz", "deep-research", "usage-credits"] {
            assert!(!hidden(name), "{name}");
        }
    }

    #[test]
    fn hints_read_as_words() {
        let all = sample();
        let get = |name: &str| all.iter().find(|c| c.name == name).unwrap();
        assert_eq!((get("compact").hint.as_deref(), get("compact").words.as_deref()), (Some("instructions"), Some("instructions for the summary (optional)")));
        assert_eq!(get("goal").words.as_deref(), Some("what has to be true before Claude stops"));
        assert_eq!(get("rename").words.as_deref(), Some("name"));
        assert_eq!(get("code-review").words.as_deref(), Some("low medium high --fix"));
        assert_eq!(get("context").words, None);
        assert_eq!(get("mode").words, None);
    }

    #[test]
    fn before_claude_sends_its_list_only_endeavors_show() {
        let all = commands(None);
        assert_eq!(names(&filter("", &all), &all), ["/mode", "/model"]);
    }

    #[test]
    fn a_command_name_and_a_space_make_a_token() {
        let all = sample();
        assert_eq!(token("/compact ", &all).map(|(len, c)| (len, c.name.as_str())), Some((8, "compact")));
        assert_eq!(token("/compact keep the fits", &all).map(|(len, _)| len), Some(8));
        // One backspace: back to text, and the list again.
        assert_eq!(token("/compact", &all), None);
        assert_eq!(typing("/compact"), Some("compact"));
        assert_eq!(typing("/compact "), None);
        // Hidden ones typed in full are still commands.
        assert_eq!(token("/config x=1", &all).map(|(_, c)| c.name.as_str()), Some("config"));
        assert_eq!(token("/nope x", &all), None);
        assert_eq!(token("/mode ", &all).map(|(_, c)| c.own), Some(Some(Own::Mode)));
    }

    #[test]
    fn endeavors_commands_are_known_by_their_text() {
        assert_eq!(own_command("/model sonnet"), Some((Own::Model, "sonnet")));
        assert_eq!(own_command("/mode"), Some((Own::Mode, "")));
        assert_eq!(own_command("/modes"), None);
        assert_eq!(own_command("/compact"), None);
        assert!(is_command("/compact keep it") && is_command("/context"));
        assert!(!is_command("/Users/me/data.csv is where it is") && !is_command("/ alone") && !is_command("plain"));
    }

    #[test]
    fn endeavors_commands_apply_and_are_never_sent() {
        let names = |own: Own| match own {
            Own::Model => vec!["Opus".to_string(), "Sonnet".to_string(), "Haiku".to_string()],
            Own::Mode => vec!["Manual".to_string(), "Ask to run".to_string(), "Auto".to_string(), "Plan".to_string()],
        };
        assert_eq!(own_send("/model sonnet", names), Some(OwnSend::Apply(Own::Model, 1)));
        assert_eq!(own_send("  /mode pl ", names), Some(OwnSend::Apply(Own::Mode, 3)));
        assert_eq!(own_send("/mode a", names), Some(OwnSend::Apply(Own::Mode, 1)));
        assert_eq!(own_send("/model", names), Some(OwnSend::Open(Own::Model)));
        assert_eq!(own_send("/model gpt", names), Some(OwnSend::Open(Own::Model)));
        // Everything else goes to Claude as written.
        assert_eq!(own_send("/compact keep the fits", names), None);
        assert_eq!(own_send("/models", names), None);
        assert_eq!(own_send("use /model sonnet", names), None);
    }

    #[test]
    fn choices_filter_by_what_follows() {
        let models = vec!["Opus".to_string(), "Sonnet".to_string(), "Haiku".to_string()];
        assert_eq!(choices("", &models), [0, 1, 2]);
        assert_eq!(choices("so", &models), [1]);
        assert_eq!(choices("ku", &models), [2]);
        assert_eq!(choices("o", &models), [0, 1]);
        assert_eq!(choices("x", &models), Vec::<usize>::new());
    }
}
