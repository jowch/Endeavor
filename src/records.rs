//! sessions.json: Endeavor's own record of each session, which the sidebar
//! draws from at launch without waiting for any agent. Each agent's listing
//! then brings it up to date (`merge`). User names stay in titles.json and
//! archiving in archived.json.
//!
//! Only the person deleting a session removes its record. A listing that
//! leaves a session out (the agent signed out, a fresh profile, a list that
//! failed partway) marks it missing for this launch, and it stays.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::hosts::{HostId, Place};

pub use crate::agent::Agent;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub agent: Agent,
    /// None for a session saved as a bare id, until its agent lists it in a folder.
    pub place: Option<Place>,
    /// The agent's title, or the first message's while it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Last activity, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<u64>,
}

/// What one listing covers. On This Mac an agent lists one folder; a server's
/// sessions all share one agent folder, so its listing covers the whole host.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Folder(Place),
    Host(HostId),
}

impl Scope {
    fn covers(&self, place: &Place) -> bool {
        match self {
            Scope::Folder(folder) => folder == place,
            Scope::Host(host) => &place.host == host,
        }
    }
}

/// A session as an agent's listing gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub id: String,
    pub title: Option<String>,
    pub updated: Option<u64>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Records {
    records: HashMap<String, Record>,
    /// Scopes each agent has listed this launch (not saved).
    listed: HashSet<(Agent, Scope)>,
    /// Sessions the latest listing of their scope left out (not saved).
    missing: HashSet<String>,
}

impl Records {
    /// Reads sessions.json in any of its shapes: records; before that, each
    /// session's place; before servers, a list of ids.
    pub fn parse(json: &str) -> Records {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Records(HashMap<String, Record>),
            Places(HashMap<String, Place>),
            Ids(Vec<String>),
        }
        let record = |place| Record { agent: Agent::Claude, place, title: None, updated: None };
        let records = match serde_json::from_str::<Saved>(json) {
            Ok(Saved::Records(records)) => records,
            Ok(Saved::Places(places)) => places.into_iter().map(|(id, place)| (id, record(Some(place)))).collect(),
            Ok(Saved::Ids(ids)) => ids.into_iter().map(|id| (id, record(None))).collect(),
            Err(_) => HashMap::new(),
        };
        Records { records, ..Records::default() }
    }

    /// What sessions.json holds.
    pub fn saved(&self) -> &HashMap<String, Record> {
        &self.records
    }

    pub fn get(&self, id: &str) -> Option<&Record> {
        self.records.get(id)
    }

    /// Whether any session has a folder to show under.
    pub fn any_placed(&self) -> bool {
        self.records.values().any(|r| r.place.is_some())
    }

    /// Sessions with a folder, newest first (no time last).
    pub fn placed(&self) -> Vec<(&str, &Record)> {
        let mut placed: Vec<_> = self.records.iter().filter(|(_, r)| r.place.is_some()).map(|(id, r)| (id.as_str(), r)).collect();
        placed.sort_by(|a, b| b.1.updated.cmp(&a.1.updated).then_with(|| a.0.cmp(b.0)));
        placed
    }

    /// A folder's sessions, newest first.
    pub fn in_folder(&self, folder: &Place) -> Vec<(&str, &Record)> {
        self.placed().into_iter().filter(|(_, r)| r.place.as_ref() == Some(folder)).collect()
    }

    /// Whether `agent` has listed the sessions at `place` this launch.
    pub fn was_listed(&self, agent: Agent, place: &Place) -> bool {
        self.listed.iter().any(|(by, scope)| *by == agent && scope.covers(place))
    }

    /// A session started (or a copy made) in `place`: recorded if it's new,
    /// with now as its last activity. True if anything changed.
    pub fn started(&mut self, id: &str, agent: Agent, place: &Place, now: u64) -> bool {
        match self.records.get_mut(id) {
            Some(record) if record.place.as_ref() == Some(place) => false,
            Some(record) => {
                record.place = Some(place.clone());
                true
            }
            None => {
                self.records.insert(id.to_owned(), Record { agent, place: Some(place.clone()), title: None, updated: Some(now) });
                true
            }
        }
    }

    /// A session's title changed, or it had activity at `now`. True if anything changed.
    pub fn touch(&mut self, id: &str, title: Option<&str>, now: Option<u64>) -> bool {
        let Some(record) = self.records.get_mut(id) else { return false };
        let before = record.clone();
        if let Some(title) = title {
            record.title = Some(title.to_owned());
        }
        if now.is_some() {
            record.updated = now;
        }
        *record != before
    }

    /// The person deleted the session: the one way a record goes.
    pub fn remove(&mut self, id: &str) -> bool {
        self.missing.remove(id);
        self.records.remove(id).is_some()
    }

    /// Whether the latest listing of the session's folder left it out.
    pub fn is_missing(&self, id: &str) -> bool {
        self.missing.contains(id)
    }

    /// Bring the record up to date with one agent's listing of `scope`: new
    /// titles and times for Endeavor's own sessions. Others the agent lists,
    /// such as the Claude Code CLI's, aren't added, except one in `known`
    /// (ids Endeavor still has a name, notebook or mode for) listed in a
    /// folder, which brings back a record an older Endeavor dropped. That
    /// agent's sessions in `scope` a `complete` listing (no further page)
    /// leaves out are marked missing and kept, since an empty or short list
    /// can mean the agent is signed out or looking at another profile; `keep`
    /// (open sessions, which an agent may not list until their first turn)
    /// aren't marked. Other agents' sessions are untouched. True if a record
    /// changed.
    pub fn merge(&mut self, agent: Agent, scope: Scope, listed: Vec<Listed>, complete: bool, keep: &HashSet<String>, known: &HashSet<String>) -> bool {
        let before = self.records.clone();
        let ids: HashSet<&str> = listed.iter().map(|l| l.id.as_str()).collect();
        for (id, r) in &self.records {
            if r.agent != agent || !r.place.as_ref().is_some_and(|p| scope.covers(p)) {
                continue;
            }
            if ids.contains(id.as_str()) || keep.contains(id) {
                self.missing.remove(id);
            } else if complete {
                self.missing.insert(id.clone());
            }
        }
        for Listed { id, title, updated } in listed {
            if let (Scope::Folder(folder), false) = (&scope, self.records.contains_key(&id))
                && known.contains(&id)
            {
                self.records.insert(id.clone(), Record { agent, place: Some(folder.clone()), title: None, updated: None });
            }
            let Some(record) = self.records.get_mut(&id).filter(|r| r.agent == agent) else { continue };
            if let (None, Scope::Folder(folder)) = (&record.place, &scope) {
                record.place = Some(folder.clone());
            }
            if title.is_some() {
                record.title = title;
            }
            if updated.is_some() {
                record.updated = updated;
            }
        }
        self.listed.insert((agent, scope));
        self.records != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(place: Option<Place>, title: Option<&str>, updated: Option<u64>) -> Record {
        Record { agent: Agent::Claude, place, title: title.map(str::to_owned), updated }
    }

    fn listed(id: &str, title: Option<&str>, updated: Option<u64>) -> Listed {
        Listed { id: id.into(), title: title.map(str::to_owned), updated }
    }

    fn server(id: &str, path: &str) -> Place {
        Place { host: HostId::Server(id.into()), path: path.into() }
    }

    #[test]
    fn reads_every_saved_shape() {
        let ids = Records::parse(r#"["a", "b"]"#);
        assert_eq!(ids.get("a"), Some(&record(None, None, None)));
        assert_eq!(ids.placed(), vec![]);

        let places = Records::parse(r#"{"a": "/work/fits", "b": {"host": {"server": "lab"}, "path": "/home/me/x"}}"#);
        assert_eq!(places.get("a"), Some(&record(Some(Place::local("/work/fits")), None, None)));
        assert_eq!(places.get("b"), Some(&record(Some(server("lab", "/home/me/x")), None, None)));

        let records = Records::parse(
            r#"{"a": {"agent": "claude", "place": "/work/fits", "title": "Fit decay", "updated": 1790000000},
                "b": {"agent": "claude", "place": null}}"#,
        );
        assert_eq!(records.get("a"), Some(&record(Some(Place::local("/work/fits")), Some("Fit decay"), Some(1790000000))));
        assert_eq!(records.get("b"), Some(&record(None, None, None)));

        assert_eq!(Records::parse("not json"), Records::default());
    }

    #[test]
    fn saves_and_reads_back() {
        let mut records = Records::default();
        records.started("a", Agent::Claude, &server("lab", "/home/me/x"), 100);
        records.touch("a", Some("Fit decay"), Some(200));
        let json = serde_json::to_string_pretty(records.saved()).unwrap();
        assert!(json.contains(r#""agent": "claude""#), "{json}");
        assert_eq!(Records::parse(&json).get("a"), Some(&record(Some(server("lab", "/home/me/x")), Some("Fit decay"), Some(200))));
    }

    #[test]
    fn folder_rows_are_newest_first() {
        let mut records = Records::parse(
            r#"{"old": {"agent": "claude", "place": "/f", "updated": 10},
                "new": {"agent": "claude", "place": "/f", "updated": 30},
                "untimed": {"agent": "claude", "place": "/f"},
                "elsewhere": {"agent": "claude", "place": "/g", "updated": 20}}"#,
        );
        let ids = |records: &Records| records.in_folder(&Place::local("/f")).into_iter().map(|(id, _)| id.to_owned()).collect::<Vec<_>>();
        assert_eq!(ids(&records), ["new", "old", "untimed"]);
        assert!(records.touch("old", None, Some(40)));
        assert_eq!(ids(&records), ["old", "new", "untimed"]);
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn merge_updates_and_marks_what_a_folder_listing_leaves_out() {
        let mut records = Records::parse(
            r#"{"kept": {"agent": "claude", "place": "/f", "title": "Old title", "updated": 10},
                "deleted": {"agent": "claude", "place": "/f", "title": "Gone", "updated": 5},
                "open": {"agent": "claude", "place": "/f"},
                "other_folder": {"agent": "claude", "place": "/g"},
                "bare": {"agent": "claude", "place": null}}"#,
        );
        let keep = HashSet::from(["open".to_owned()]);
        let changed = records.merge(
            Agent::Claude,
            Scope::Folder(Place::local("/f")),
            vec![listed("kept", Some("New title"), Some(50)), listed("cli", Some("From the CLI"), Some(40)), listed("bare", None, Some(30))],
            true,
            &keep,
            &none(),
        );
        assert!(changed);
        let f = Some(Place::local("/f"));
        assert_eq!(records.get("kept"), Some(&record(f.clone(), Some("New title"), Some(50))));
        assert_eq!(records.get("cli"), None, "a session Endeavor didn't make isn't added");
        assert_eq!(records.get("bare"), Some(&record(f.clone(), None, Some(30))));
        assert_eq!(records.get("open"), Some(&record(f.clone(), None, None)));
        assert_eq!(records.get("other_folder"), Some(&record(Some(Place::local("/g")), None, None)));
        assert_eq!(records.get("deleted"), Some(&record(f.clone(), Some("Gone"), Some(5))), "a session the listing leaves out is kept");
        assert!(records.is_missing("deleted"));
        assert!(!records.is_missing("kept") && !records.is_missing("open") && !records.is_missing("other_folder") && !records.is_missing("bare"));
        assert!(records.was_listed(Agent::Claude, &Place::local("/f")));
        assert!(!records.was_listed(Agent::Claude, &Place::local("/g")));
    }

    #[test]
    fn merge_of_a_server_listing_covers_the_host_and_adds_nothing_it_cant_place() {
        let mut records = Records::parse(
            r#"{"a": {"agent": "claude", "place": {"host": {"server": "lab"}, "path": "/x"}},
                "b": {"agent": "claude", "place": {"host": {"server": "lab"}, "path": "/y"}},
                "c": {"agent": "claude", "place": {"host": {"server": "hpc"}, "path": "/x"}}}"#,
        );
        records.merge(Agent::Claude, Scope::Host(HostId::Server("lab".into())), vec![listed("a", Some("A"), Some(9)), listed("stranger", None, None)], true, &none(), &HashSet::from(["stranger".to_owned()]));
        assert_eq!(records.get("a"), Some(&record(Some(server("lab", "/x")), Some("A"), Some(9))));
        assert_eq!(records.get("b"), Some(&record(Some(server("lab", "/y")), None, None)));
        assert!(records.is_missing("b") && !records.is_missing("a") && !records.is_missing("c"));
        assert_eq!(records.get("c"), Some(&record(Some(server("hpc", "/x")), None, None)));
        assert_eq!(records.get("stranger"), None);
        assert!(records.was_listed(Agent::Claude, &server("lab", "/anything")));
    }

    #[test]
    fn merge_leaves_other_agents_sessions_alone() {
        let mut records = Records::parse(r#"{"mine": {"agent": "claude", "place": "/f"}, "theirs": {"agent": "codex", "place": "/f", "title": "T"}}"#);
        records.merge(Agent::Claude, Scope::Folder(Place::local("/f")), vec![listed("theirs", Some("Renamed"), Some(5))], true, &none(), &none());
        assert_eq!(records.get("mine"), Some(&record(Some(Place::local("/f")), None, None)));
        assert!(records.is_missing("mine") && !records.is_missing("theirs"));
        assert_eq!(records.get("theirs"), Some(&Record { agent: Agent::Codex, place: Some(Place::local("/f")), title: Some("T".into()), updated: None }));
    }

    #[test]
    fn an_unchanged_listing_changes_nothing() {
        let mut records = Records::parse(r#"{"a": {"agent": "claude", "place": "/f", "title": "A", "updated": 9}}"#);
        assert!(!records.merge(Agent::Claude, Scope::Folder(Place::local("/f")), vec![listed("a", Some("A"), Some(9))], true, &none(), &none()));
        assert!(!records.merge(Agent::Claude, Scope::Folder(Place::local("/f")), vec![listed("a", None, None)], true, &none(), &none()));
        assert_eq!(records.get("a"), Some(&record(Some(Place::local("/f")), Some("A"), Some(9))));
    }

    fn two_sessions() -> Records {
        Records::parse(
            r#"{"a": {"agent": "claude", "place": "/f", "title": "A", "updated": 9},
                "b": {"agent": "claude", "place": "/f", "title": "B", "updated": 8}}"#,
        )
    }

    #[test]
    fn an_empty_listing_keeps_every_record() {
        let mut records = two_sessions();
        let saved = records.saved().clone();
        assert!(!records.merge(Agent::Claude, Scope::Folder(Place::local("/f")), vec![], true, &none(), &none()), "nothing to save");
        assert_eq!(records.saved(), &saved);
        assert!(records.is_missing("a") && records.is_missing("b"));
        assert_eq!(records.in_folder(&Place::local("/f")).len(), 2, "both still show");
    }

    #[test]
    fn signed_out_then_back_in() {
        // Signed out (or a fresh profile), the agent lists nothing; signed back
        // in, it lists them all again and nothing was lost on the way.
        let mut records = two_sessions();
        let saved = records.saved().clone();
        let f = Scope::Folder(Place::local("/f"));
        records.merge(Agent::Claude, f.clone(), vec![], true, &none(), &none());
        let json = serde_json::to_string(records.saved()).unwrap();
        let mut relaunched = Records::parse(&json);
        assert_eq!(relaunched.saved(), &saved, "sessions.json still holds both");
        assert!(!relaunched.is_missing("a"), "missing is for this launch only");
        relaunched.merge(Agent::Claude, f, vec![listed("a", Some("A"), Some(9)), listed("b", Some("B"), Some(8))], true, &none(), &none());
        assert_eq!(relaunched.saved(), &saved);
        assert!(!relaunched.is_missing("a") && !relaunched.is_missing("b"));
    }

    #[test]
    fn a_short_listing_keeps_the_rest() {
        let mut records = two_sessions();
        let f = Scope::Folder(Place::local("/f"));
        records.merge(Agent::Claude, f.clone(), vec![listed("a", Some("A2"), Some(20))], true, &none(), &none());
        assert_eq!(records.get("a").and_then(|r| r.title.as_deref()), Some("A2"));
        assert_eq!(records.get("b"), Some(&record(Some(Place::local("/f")), Some("B"), Some(8))));
        assert!(records.is_missing("b") && !records.is_missing("a"));
        // The next, full listing has it again.
        records.merge(Agent::Claude, f, vec![listed("a", None, None), listed("b", None, None)], true, &none(), &none());
        assert!(!records.is_missing("b"));
    }

    #[test]
    fn a_first_page_marks_nothing_missing() {
        // The agent has more pages (Codex pages over every thread, then filters
        // by folder), so a session left out may just be on a later page.
        let mut records = two_sessions();
        let known = HashSet::from(["lost".to_owned()]);
        let changed = records.merge(Agent::Claude, Scope::Folder(Place::local("/f")), vec![listed("a", Some("A2"), Some(20)), listed("lost", None, None)], false, &none(), &known);
        assert!(changed);
        assert_eq!(records.get("a").and_then(|r| r.title.as_deref()), Some("A2"), "titles and times still update");
        assert!(records.get("lost").is_some(), "known ids still come back");
        assert!(!records.is_missing("b"));
        assert!(records.get("b").is_some());
    }

    #[test]
    fn only_deleting_removes_a_record() {
        let mut records = two_sessions();
        records.merge(Agent::Claude, Scope::Folder(Place::local("/f")), vec![], true, &none(), &none());
        assert!(records.remove("b"));
        assert_eq!(records.get("b"), None);
        assert!(!records.is_missing("b"));
        assert!(records.get("a").is_some());
    }

    #[test]
    fn a_listed_session_endeavor_still_knows_comes_back() {
        // An older Endeavor dropped "lost" from sessions.json, but titles.json
        // or notebooks.json still has it; the agent lists it in /f.
        let mut records = Records::parse(r#"{"a": {"agent": "claude", "place": "/f"}}"#);
        let known = HashSet::from(["lost".to_owned()]);
        let changed = records.merge(
            Agent::Claude,
            Scope::Folder(Place::local("/f")),
            vec![listed("a", None, None), listed("lost", Some("Fit decay"), Some(7)), listed("cli", Some("From the CLI"), None)],
            true,
            &none(),
            &known,
        );
        assert!(changed);
        assert_eq!(records.get("lost"), Some(&record(Some(Place::local("/f")), Some("Fit decay"), Some(7))));
        assert_eq!(records.get("cli"), None);
    }
}
