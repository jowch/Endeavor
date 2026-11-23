use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind, ToolCallId, ToolKind};
use gpui::Pixels;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Subject {
    File(String),
    Site(String),
    Command(String),
    Tool(String),
}

pub struct Asked<'a> {
    pub title: &'a str,
    pub kind: Option<ToolKind>,
    pub input: &'a Value,
    pub path: Option<&'a Path>,
}

impl Asked<'_> {
    pub fn subject(&self) -> Subject {
        let field = |name: &str| self.input[name].as_str().map(str::trim).filter(|s| !s.is_empty());
        if let Some(command) = field("command").filter(|_| self.kind.is_none_or(|k| k == ToolKind::Execute)) {
            return Subject::Command(command.to_owned());
        }
        if self.kind == Some(ToolKind::Fetch)
            && let Some(url) = field("url")
        {
            return Subject::Site(host_of(url).to_owned());
        }
        if matches!(self.kind, Some(ToolKind::Edit | ToolKind::Delete | ToolKind::Move))
            && let Some(path) = self.path.map(|p| p.display().to_string()).or_else(|| field("file_path").or(field("notebook_path")).map(str::to_owned))
        {
            return Subject::File(path);
        }
        Subject::Tool(self.title.trim().to_owned())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    Same(Subject),
    CommandsStarting(Vec<String>),
}

impl Rule {
    pub fn for_prompt(asked: &Asked, options: &[PermissionOption]) -> Rule {
        match asked.subject() {
            Subject::Command(command) => {
                let prefixes = options.iter().filter(|o| o.kind == PermissionOptionKind::AllowAlways).find_map(|o| suggested_prefixes(&o.name)).unwrap_or_default();
                if prefixes.is_empty() { Rule::Same(Subject::Command(command)) } else { Rule::CommandsStarting(prefixes) }
            }
            subject => Rule::Same(subject),
        }
    }

    pub fn allows(&self, asked: &Asked) -> bool {
        match (self, asked.subject()) {
            (Rule::Same(rule), subject) => *rule == subject,
            (Rule::CommandsStarting(prefixes), Subject::Command(command)) => {
                let chained = ["&&", "||", ";", "|", "`", "$(", ">", "<", "\n", "&"].iter().any(|s| command.contains(s));
                !chained && prefixes.iter().any(|p| command == *p || command.strip_prefix(p.as_str()).is_some_and(|rest| rest.starts_with(' ')))
            }
            _ => false,
        }
    }
}

pub fn suggested_prefixes(label: &str) -> Option<Vec<String>> {
    let list = label.strip_prefix("Yes, and don't ask again for ")?.strip_suffix(" commands")?;
    if list == "similar" {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for chunk in list.split(", ") {
        let chunk = chunk.strip_prefix("and ").unwrap_or(chunk);
        parts.extend(chunk.split(" and ").map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned));
    }
    (!parts.is_empty()).then_some(parts)
}

pub fn folder_rule_option(options: &[PermissionOption]) -> Option<&PermissionOption> {
    options.iter().find(|o| o.kind == PermissionOptionKind::AllowAlways && o.name.starts_with("Yes, and don't ask again for "))
}

pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.rsplit_once('@').map_or(host, |(_, h)| h)
}

#[derive(Default)]
pub struct Asks {
    pub rules: Vec<Rule>,
    pub total: usize,
    pub answered: usize,
    pub answered_at: HashMap<ToolCallId, SystemTime>,
    pub card_height: Rc<Cell<Option<Pixels>>>,
    pub code_open: bool,
    pub always_menu: bool,
    pub cells_visible: Option<bool>,
}

impl Asks {
    pub fn asked(&mut self, waiting: usize) {
        if waiting == 0 {
            self.total = 0;
            self.answered = 0;
            self.card_height.set(None);
        }
        self.total += 1;
        self.cells_visible = None;
    }

    pub fn answered(&mut self, call: ToolCallId) {
        self.answered += 1;
        self.answered_at.insert(call, SystemTime::now());
        self.code_open = false;
        self.always_menu = false;
        self.cells_visible = None;
    }

    pub fn count(&self) -> Option<String> {
        (self.total > 1).then(|| format!("{} of {}", (self.answered + 1).min(self.total), self.total))
    }

    pub fn forget_batch(&mut self) {
        self.total = 0;
        self.answered = 0;
        self.card_height.set(None);
        self.code_open = false;
        self.always_menu = false;
    }

    pub fn allowed(&self, asked: &Asked) -> bool {
        self.rules.iter().any(|r| r.allows(asked))
    }
}

pub const NOTIFY_QUIET: Duration = Duration::from_secs(10 * 60);

pub fn may_notify(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.duration_since(last) >= NOTIFY_QUIET)
}

pub fn folder_rules(folder: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(settings_file(folder)) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    json["permissions"]["allow"].as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

pub fn settings_file(folder: &Path) -> std::path::PathBuf {
    folder.join(crate::agent::CLAUDE_CODE.folder_rules.unwrap_or(".claude/settings.local.json"))
}

pub fn remove_folder_rule(folder: &Path, rule: &str) -> Result<(), String> {
    let file = settings_file(folder);
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let new = without_rule(&text, rule).ok_or("The rule isn't in the settings file any more.")?;
    std::fs::write(&file, new).map_err(|e| e.to_string())
}

pub fn without_rule(text: &str, rule: &str) -> Option<String> {
    let mut json: Value = serde_json::from_str(text).ok()?;
    let allow = json["permissions"]["allow"].as_array_mut()?;
    let at = allow.iter().position(|r| r.as_str() == Some(rule))?;
    allow.remove(at);
    let literal = serde_json::to_string(rule).ok()?;
    for (start, _) in text.match_indices(&literal) {
        let end = start + literal.len();
        let after = &text[end..];
        let after_ws = after.len() - after.trim_start().len();
        let cut = if after.trim_start().starts_with(',') {
            let rest = &after[after_ws + 1..];
            start..end + after_ws + 1 + (rest.len() - rest.trim_start().len())
        } else {
            let before = text[..start].trim_end();
            match before.strip_suffix(',') {
                Some(b) => b.len()..end,
                None => start..end,
            }
        };
        let candidate = format!("{}{}", &text[..cut.start], &text[cut.end..]);
        if serde_json::from_str::<Value>(&candidate).ok().as_ref() == Some(&json) {
            return Some(candidate);
        }
    }
    serde_json::to_string_pretty(&json).ok().map(|s| s + "\n")
}

pub fn rule_words(rule: &str) -> String {
    let (tool, content) = match rule.split_once('(') {
        Some((tool, rest)) => (tool, rest.strip_suffix(')').map(str::trim)),
        None => (rule, None),
    };
    let files = |verb: &str, any: &str| match content {
        None | Some("") | Some("**") => any.to_owned(),
        Some(path) => match path.strip_suffix("/**").or_else(|| path.strip_suffix("/*")) {
            Some(dir) => format!("{verb} files in `{}/`", dir.trim_start_matches("./")),
            None => format!("{verb} `{path}`"),
        },
    };
    match tool {
        "Bash" | "PowerShell" => match content {
            None | Some("") => "Run any command".into(),
            Some(c) => match c.strip_suffix(":*").or_else(|| c.strip_suffix(" *")) {
                Some(prefix) => format!("Run commands starting with `{prefix}`"),
                None => format!("Run `{c}`"),
            },
        },
        "Edit" | "MultiEdit" => files("Edit", "Edit any file"),
        "Write" => files("Create", "Create any file"),
        "Read" => files("Read", "Read any file"),
        "NotebookEdit" => files("Edit", "Edit any Jupyter notebook"),
        "WebFetch" => match content.and_then(|c| c.strip_prefix("domain:")) {
            Some(domain) => format!("Fetch `{domain}`"),
            None => "Fetch any web page".into(),
        },
        "WebSearch" => "Search the web".into(),
        _ => match tool.strip_prefix("mcp__").map(|t| t.split_once("__").unwrap_or((t, ""))) {
            Some((server, "")) => format!("Use any `{server}` tool"),
            Some(("notebook", name)) => format!("Notebook: {}", name.replace('_', " ")),
            Some((server, name)) => format!("Use `{name}` from `{server}`"),
            None => format!("`{rule}`"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn asked<'a>(title: &'a str, kind: ToolKind, input: &'a Value) -> Asked<'a> {
        Asked { title, kind: Some(kind), input, path: None }
    }

    #[test]
    fn always_this_session_matches_the_same_file_site_or_command_prefix() {
        let edit = json!({ "file_path": "/p/notes.md", "old_string": "a", "new_string": "b" });
        let other_edit = json!({ "file_path": "/p/other.md", "old_string": "a", "new_string": "b" });
        let rule = Rule::for_prompt(&asked("Edit", ToolKind::Edit, &edit), &[]);
        assert_eq!(rule, Rule::Same(Subject::File("/p/notes.md".into())));
        assert!(rule.allows(&asked("Edit", ToolKind::Edit, &json!({ "file_path": "/p/notes.md", "old_string": "c", "new_string": "d" }))));
        assert!(!rule.allows(&asked("Edit", ToolKind::Edit, &other_edit)));

        let fetch = json!({ "url": "https://juliastats.org/Bootstrap.jl/stable/" });
        let rule = Rule::for_prompt(&asked("Fetch", ToolKind::Fetch, &fetch), &[]);
        assert_eq!(rule, Rule::Same(Subject::Site("juliastats.org".into())));
        assert!(rule.allows(&asked("Fetch", ToolKind::Fetch, &json!({ "url": "https://juliastats.org/other" }))));
        assert!(!rule.allows(&asked("Fetch", ToolKind::Fetch, &json!({ "url": "https://example.org/" }))));

        let options = [
            PermissionOption::new("allow-once", "Yes", PermissionOptionKind::AllowOnce),
            PermissionOption::new("allow-with-updates", "Yes, and don't ask again for npm test commands", PermissionOptionKind::AllowAlways),
            PermissionOption::new("reject", "No", PermissionOptionKind::RejectOnce),
        ];
        let test = json!({ "command": "npm test -- --watch" });
        let rule = Rule::for_prompt(&asked("npm test -- --watch", ToolKind::Execute, &test), &options);
        assert_eq!(rule, Rule::CommandsStarting(vec!["npm test".into()]));
        let run = |c: &str| rule.allows(&asked(c, ToolKind::Execute, &json!({ "command": c })));
        assert!(run("npm test"));
        assert!(run("npm test frontend"));
        assert!(!run("npm testing"));
        assert!(!run("npm test && rm -rf build"), "a prefix doesn't cover what's chained after it");
        assert!(!run("npm run build"));

        let exact = json!({ "command": "uname -a" });
        let rule = Rule::for_prompt(&asked("uname -a", ToolKind::Execute, &exact), &options[..1]);
        assert_eq!(rule, Rule::Same(Subject::Command("uname -a".into())));
        assert!(!rule.allows(&asked("uname -m", ToolKind::Execute, &json!({ "command": "uname -m" }))));

        let cell = json!({ "cell_id": "a", "code": "x = 1" });
        let rule = Rule::for_prompt(&asked("mcp__notebook__edit_cell", ToolKind::Other, &cell), &[]);
        assert!(rule.allows(&asked("mcp__notebook__edit_cell", ToolKind::Other, &json!({ "cell_id": "b", "code": "y = 2" }))));
        assert!(!rule.allows(&asked("mcp__notebook__add_cell", ToolKind::Other, &cell)));
    }

    #[test]
    fn in_this_folder_only_for_options_that_write_a_folder_rule() {
        let with = |name: &str| [PermissionOption::new("allow-once", "Yes", PermissionOptionKind::AllowOnce), PermissionOption::new("allow-with-updates", name, PermissionOptionKind::AllowAlways)];
        assert!(folder_rule_option(&with("Yes, and don't ask again for npm test commands")).is_some());
        assert!(folder_rule_option(&with("Yes, and don't ask again for juliastats.org")).is_some());
        assert!(folder_rule_option(&with("Yes, allow all edits during this session")).is_none());
        assert!(folder_rule_option(&with("Yes, during this session")).is_none());
        assert!(folder_rule_option(&with("Yes, and always allow access to data/ from this project")).is_none());
    }

    #[test]
    fn reads_the_prefixes_claude_code_suggests() {
        assert_eq!(suggested_prefixes("Yes, and don't ask again for npm test commands"), Some(vec!["npm test".into()]));
        assert_eq!(suggested_prefixes("Yes, and don't ask again for git status and git diff commands"), Some(vec!["git status".into(), "git diff".into()]));
        assert_eq!(suggested_prefixes("Yes, and don't ask again for ls, cat, and head commands"), Some(vec!["ls".into(), "cat".into(), "head".into()]));
        assert_eq!(suggested_prefixes("Yes, and don't ask again for similar commands"), None);
        assert_eq!(suggested_prefixes("Yes, and don't ask again for juliastats.org"), None);
    }

    #[test]
    fn queued_prompts_count_the_batch() {
        let mut asks = Asks::default();
        asks.asked(0);
        assert_eq!(asks.count(), None, "one prompt alone has no count");
        asks.asked(1);
        asks.asked(2);
        assert_eq!(asks.count().as_deref(), Some("1 of 3"));
        asks.answered(ToolCallId::new("a"));
        assert_eq!(asks.count().as_deref(), Some("2 of 3"));
        asks.asked(2);
        assert_eq!(asks.count().as_deref(), Some("2 of 4"), "a prompt joining the batch adds to it");
        asks.answered(ToolCallId::new("b"));
        asks.answered(ToolCallId::new("c"));
        asks.answered(ToolCallId::new("d"));
        asks.asked(0);
        assert_eq!(asks.count(), None, "a new batch starts once none wait");
        assert!(asks.answered_at.contains_key(&ToolCallId::new("b")));
    }

    #[test]
    fn notifications_wait_ten_minutes_after_one() {
        let now = Instant::now();
        assert!(may_notify(None, now));
        assert!(!may_notify(Some(now), now + Duration::from_secs(599)));
        assert!(may_notify(Some(now), now + Duration::from_secs(600)));
    }

    #[test]
    fn removing_a_folder_rule_keeps_the_rest_of_the_file() {
        let text = "{\n  \"permissions\": {\n    \"allow\": [\n      \"Bash(npm test:*)\",\n      \"WebFetch(domain:juliastats.org)\"\n    ],\n    \"deny\": []\n  },\n  \"zeta\": 1,\n  \"alpha\": 2\n}\n";
        assert_eq!(
            without_rule(text, "Bash(npm test:*)").as_deref(),
            Some("{\n  \"permissions\": {\n    \"allow\": [\n      \"WebFetch(domain:juliastats.org)\"\n    ],\n    \"deny\": []\n  },\n  \"zeta\": 1,\n  \"alpha\": 2\n}\n")
        );
        assert_eq!(
            without_rule(text, "WebFetch(domain:juliastats.org)").as_deref(),
            Some("{\n  \"permissions\": {\n    \"allow\": [\n      \"Bash(npm test:*)\"\n    ],\n    \"deny\": []\n  },\n  \"zeta\": 1,\n  \"alpha\": 2\n}\n")
        );
        let twice = r#"{"note": "Bash(ls)", "permissions": {"allow": ["Bash(ls)"]}}"#;
        assert_eq!(without_rule(twice, "Bash(ls)").as_deref(), Some(r#"{"note": "Bash(ls)", "permissions": {"allow": []}}"#));
        assert_eq!(without_rule(text, "Bash(rm:*)"), None);
    }

    #[test]
    fn folder_rules_read_in_plain_words() {
        assert_eq!(rule_words("Bash(npm test:*)"), "Run commands starting with `npm test`");
        assert_eq!(rule_words("Bash(uname -a)"), "Run `uname -a`");
        assert_eq!(rule_words("Edit(data/**)"), "Edit files in `data/`");
        assert_eq!(rule_words("Edit(./notes.md)"), "Edit `./notes.md`");
        assert_eq!(rule_words("WebFetch(domain:juliastats.org)"), "Fetch `juliastats.org`");
        assert_eq!(rule_words("WebSearch"), "Search the web");
        assert_eq!(rule_words("mcp__notebook__read_cell"), "Notebook: read cell");
        assert_eq!(rule_words("mcp__github"), "Use any `github` tool");
        assert_eq!(rule_words("Skill(foo)"), "`Skill(foo)`");
    }
}
