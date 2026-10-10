//! The one mark a sidebar row's leading bullet shows, strongest first: the
//! session needs you › its last turn stopped with an error › Claude is
//! working › a new reply › its server isn't reachable › messages wait to
//! send. With none, the bullet is an idle ring. A collapsed folder heading
//! shows the strongest of its rows' marks.

/// A row's mark. The order of the variants is their strength.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RowMark {
    NeedsYou,
    Error,
    Working { agent: &'static str },
    NewReply,
    ServerDown { host: String },
    /// Queued messages held because Claude or the server can't be reached;
    /// `until` finishes "…wait to send once": "Claude is back".
    Waiting { count: usize, until: String },
}

/// What a row's mark is chosen from.
#[derive(Default)]
pub struct RowFacts {
    pub needs_you: bool,
    pub error: bool,
    /// The session's turn is running, and not waiting for the network.
    pub working: bool,
    /// The agent a working session's mark names.
    pub agent: &'static str,
    pub new_reply: bool,
    /// The session's server, while it can't be reached.
    pub server_down: Option<String>,
    /// Messages held, and what they wait for.
    pub waiting: Option<(usize, String)>,
    /// This Mac is offline: every session is cut off at once, so rows don't
    /// say so one by one (the status line does).
    pub mac_offline: bool,
}

pub fn row_mark(f: &RowFacts) -> Option<RowMark> {
    if f.needs_you {
        return Some(RowMark::NeedsYou);
    }
    if f.error {
        return Some(RowMark::Error);
    }
    if f.working {
        return Some(RowMark::Working { agent: f.agent });
    }
    if f.new_reply {
        return Some(RowMark::NewReply);
    }
    if f.mac_offline {
        return None;
    }
    if let Some(host) = &f.server_down {
        return Some(RowMark::ServerDown { host: host.clone() });
    }
    f.waiting.as_ref().filter(|(count, _)| *count > 0).map(|(count, until)| RowMark::Waiting { count: *count, until: until.clone() })
}

/// A collapsed folder's mark: the strongest of its rows'.
pub fn strongest(marks: impl IntoIterator<Item = RowMark>) -> Option<RowMark> {
    marks.into_iter().min()
}

impl RowMark {
    /// The mark's tooltip, in words.
    pub fn words(&self) -> String {
        match self {
            RowMark::NeedsYou => "Waiting for your answer".into(),
            RowMark::Error => "Stopped with an error · open to try again".into(),
            RowMark::Working { agent } => format!("{agent} is working"),
            RowMark::NewReply => "New reply".into(),
            RowMark::ServerDown { host } => format!("{host} isn't reachable"),
            RowMark::Waiting { count: 1, until } => format!("1 message waits to send once {until}"),
            RowMark::Waiting { count, until } => format!("{count} messages wait to send once {until}"),
        }
    }

    /// What VoiceOver reads for the row: its title, then the mark's words.
    pub fn label(&self, title: &str) -> String {
        let words = self.words();
        let mut chars = words.chars();
        let words = match (self, chars.next()) {
            (RowMark::ServerDown { .. } | RowMark::Working { .. }, _) | (_, None) => words,
            (_, Some(first)) => first.to_lowercase().chain(chars).collect(),
        };
        format!("{title}, {words}")
    }

    /// The debug state's name for it.
    pub fn name(&self) -> &'static str {
        match self {
            RowMark::NeedsYou => "needs_approval",
            RowMark::Error => "error",
            RowMark::Working { .. } => "working",
            RowMark::NewReply => "new_reply",
            RowMark::ServerDown { .. } => "server_down",
            RowMark::Waiting { .. } => "waiting",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RowFacts, RowMark, row_mark, strongest};

    fn waiting(n: usize) -> Option<(usize, String)> {
        Some((n, "Claude is back".into()))
    }

    #[test]
    fn one_mark_strongest_first() {
        let all =
            RowFacts { needs_you: true, error: true, working: true, agent: "Claude", new_reply: true, server_down: Some("lab-cluster".into()), waiting: waiting(2), mac_offline: false };
        assert_eq!(row_mark(&all), Some(RowMark::NeedsYou));
        let f = RowFacts { needs_you: false, ..all };
        assert_eq!(row_mark(&f), Some(RowMark::Error));
        let f = RowFacts { error: false, ..f };
        assert_eq!(row_mark(&f), Some(RowMark::Working { agent: "Claude" }));
        let f = RowFacts { working: false, ..f };
        assert_eq!(row_mark(&f), Some(RowMark::NewReply));
        let f = RowFacts { new_reply: false, ..f };
        assert_eq!(row_mark(&f), Some(RowMark::ServerDown { host: "lab-cluster".into() }));
        let f = RowFacts { server_down: None, ..f };
        assert_eq!(row_mark(&f), Some(RowMark::Waiting { count: 2, until: "Claude is back".into() }));
        let f = RowFacts { waiting: waiting(0), ..f };
        assert_eq!(row_mark(&f), None);
    }

    #[test]
    fn a_working_codex_session_names_codex() {
        let f = RowFacts { working: true, agent: "Codex", ..Default::default() };
        assert_eq!(row_mark(&f), Some(RowMark::Working { agent: "Codex" }));
        assert_eq!(RowMark::Working { agent: "Codex" }.words(), "Codex is working");
    }

    #[test]
    fn offline_rows_show_nothing_extra() {
        let f = RowFacts { server_down: Some("lab-cluster".into()), waiting: waiting(2), mac_offline: true, ..Default::default() };
        assert_eq!(row_mark(&f), None);
        let f = RowFacts { needs_you: true, mac_offline: true, ..Default::default() };
        assert_eq!(row_mark(&f), Some(RowMark::NeedsYou));
        let f = RowFacts { working: true, agent: "Claude", mac_offline: true, ..Default::default() };
        assert_eq!(row_mark(&f), Some(RowMark::Working { agent: "Claude" }));
    }

    #[test]
    fn a_folder_shows_its_strongest_mark() {
        let marks = [RowMark::Waiting { count: 1, until: "Claude is back".into() }, RowMark::NewReply, RowMark::ServerDown { host: "lab".into() }];
        assert_eq!(strongest(marks), Some(RowMark::NewReply));
        assert_eq!(strongest([]), None);
        assert_eq!(strongest([RowMark::NewReply, RowMark::Working { agent: "Claude" }]), Some(RowMark::Working { agent: "Claude" }));
    }

    #[test]
    fn each_mark_says_what_it_means() {
        assert_eq!(RowMark::NeedsYou.words(), "Waiting for your answer");
        assert_eq!(RowMark::Error.words(), "Stopped with an error · open to try again");
        assert_eq!(RowMark::Working { agent: "Claude" }.words(), "Claude is working");
        assert_eq!(RowMark::Working { agent: "Claude" }.label("Decay fit"), "Decay fit, Claude is working");
        assert_eq!(RowMark::NewReply.words(), "New reply");
        assert_eq!(RowMark::ServerDown { host: "lab-cluster".into() }.words(), "lab-cluster isn't reachable");
        assert_eq!(RowMark::Waiting { count: 2, until: "lab-cluster is back".into() }.words(), "2 messages wait to send once lab-cluster is back");
        assert_eq!(RowMark::Waiting { count: 1, until: "Claude is back".into() }.words(), "1 message waits to send once Claude is back");
        assert_eq!(RowMark::NeedsYou.label("Residual plots"), "Residual plots, waiting for your answer");
        assert_eq!(RowMark::ServerDown { host: "Lab".into() }.label("Logistic growth"), "Logistic growth, Lab isn't reachable");
    }
}
