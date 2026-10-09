//! What runs on each host, in words, and what Settings' "Where notebooks run"
//! offers for it.

use gpui::*;
use wire::files::RuntimeState;

use crate::theme;

/// What runs on a host, as far as the app knows.
#[derive(Clone, Debug, PartialEq)]
pub enum HostState {
    /// Not connected, so not known.
    Unknown,
    Connecting,
    /// Connected, asking what runs there.
    Checking,
    /// Connected, but the check failed.
    Connected,
    Starting,
    Stopping,
    /// `job`: a cluster job's id and when it ends.
    Running { notebooks: Option<usize>, job: Option<(String, Option<u64>)> },
    /// A cluster job waits in the queue, or (`starting`) has a node and Julia is starting.
    Queued { job: String, starting: bool },
    NotRunning,
    Replaced,
    /// The connection dropped; Endeavor reconnects by itself.
    Lost,
    Failed(String),
}

impl From<&RuntimeState> for HostState {
    fn from(found: &RuntimeState) -> HostState {
        match found {
            RuntimeState::NotRunning => HostState::NotRunning,
            RuntimeState::Running { notebooks, job, .. } => {
                HostState::Running { notebooks: notebooks.map(|n| n as usize), job: job.as_ref().map(|j| (j.id.clone(), j.ends_at)) }
            }
            RuntimeState::Queued { job, state, .. } => HostState::Queued { job: job.clone(), starting: state == "RUNNING" },
            RuntimeState::Starting => HostState::Starting,
        }
    }
}

impl HostState {
    /// "Running · 2 notebooks open", "Queued · job 16", …
    pub fn text(&self) -> String {
        match self {
            HostState::Unknown => "Not connected".into(),
            HostState::Connecting => "Connecting…".into(),
            HostState::Checking => "Checking…".into(),
            HostState::Connected => "Connected".into(),
            HostState::Starting => "Starting…".into(),
            HostState::Stopping => "Stopping…".into(),
            HostState::Running { job: Some((id, Some(ends))), .. } => format!("Running · job {id} · ends at {}", crate::when::clock(*ends)),
            HostState::Running { job: Some((id, None)), .. } => format!("Running · job {id}"),
            HostState::Running { notebooks: Some(0), .. } => "Running · no notebooks open".into(),
            HostState::Running { notebooks: Some(1), .. } => "Running · 1 notebook open".into(),
            HostState::Running { notebooks: Some(n), .. } => format!("Running · {n} notebooks open"),
            HostState::Running { notebooks: None, .. } => "Running".into(),
            HostState::Queued { job, starting: false } => format!("Queued · job {job} · waiting for a free node"),
            HostState::Queued { job, starting: true } => format!("Starting · job {job}"),
            HostState::NotRunning => "Not running".into(),
            HostState::Replaced => "In use from another connection".into(),
            HostState::Lost => "Can't reach · reconnects by itself".into(),
            HostState::Failed(_) => "Couldn't connect".into(),
        }
    }

    pub fn running(&self) -> bool {
        matches!(self, HostState::Running { .. })
    }

    fn stoppable(&self) -> bool {
        matches!(self, HostState::Running { .. } | HostState::Queued { .. } | HostState::Starting)
    }
}

/// The small filled dot that marks a host with Julia running.
pub fn running_dot() -> Div {
    div().flex_shrink_0().size(px(6.)).rounded_full().bg(theme::accent())
}

/// The one thing Settings offers for a host in a state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HostAct {
    /// Stop Julia (asks first).
    Stop,
    /// Cancel a job still waiting in the queue (asks first).
    CancelJob,
    /// Try to reach it again now.
    TryNow,
}

impl HostState {
    /// Settings' words for it: the state line, and a line of advice under it.
    pub fn words(&self) -> (String, Option<String>) {
        match self {
            HostState::NotRunning => ("Not running · starts when you open a session there".into(), None),
            HostState::Lost => (self.text(), Some("If it needs your university's VPN, check that it's on.".into())),
            HostState::Failed(reason) => (self.text(), (!reason.is_empty()).then(|| reason.clone())),
            _ => (self.text(), None),
        }
    }

    /// Settings' button for it, if any: at most one, and none for a host at
    /// rest (opening a session there starts it).
    pub fn action(&self) -> Option<HostAct> {
        match self {
            HostState::Queued { starting: false, .. } => Some(HostAct::CancelJob),
            s if s.stoppable() => Some(HostAct::Stop),
            HostState::Lost | HostState::Failed(_) => Some(HostAct::TryNow),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HostAct, HostState};

    #[test]
    fn settings_says_each_state_in_words_with_at_most_one_button() {
        let words = |s: HostState| (s.words(), s.action());
        let running = HostState::Running { notebooks: Some(2), job: None };
        assert_eq!(words(running), (("Running · 2 notebooks open".into(), None), Some(HostAct::Stop)));
        assert_eq!(words(HostState::NotRunning), (("Not running · starts when you open a session there".into(), None), None));
        assert_eq!(
            words(HostState::Lost),
            (("Can't reach · reconnects by itself".into(), Some("If it needs your university's VPN, check that it's on.".into())), Some(HostAct::TryNow))
        );
        assert_eq!(words(HostState::Queued { job: "16".into(), starting: false }), (("Queued · job 16 · waiting for a free node".into(), None), Some(HostAct::CancelJob)));
        assert_eq!(words(HostState::Queued { job: "16".into(), starting: true }).1, Some(HostAct::Stop));
        assert_eq!(words(HostState::Failed("ssh: connection refused".into())), (("Couldn't connect".into(), Some("ssh: connection refused".into())), Some(HostAct::TryNow)));
        assert_eq!(words(HostState::Checking), (("Checking…".into(), None), None));
    }
    use wire::files::RuntimeState;
    use wire::slurm::Job;

    #[test]
    fn states_read_plainly() {
        let running = |notebooks| HostState::Running { notebooks, job: None }.text();
        assert_eq!(running(Some(2)), "Running · 2 notebooks open");
        assert_eq!(running(Some(1)), "Running · 1 notebook open");
        assert_eq!(running(Some(0)), "Running · no notebooks open");
        assert_eq!(running(None), "Running");
        let ends = 1_790_000_000;
        let in_job = HostState::Running { notebooks: None, job: Some(("15".into(), Some(ends))) };
        assert_eq!(in_job.text(), format!("Running · job 15 · ends at {}", crate::when::clock(ends)));
        assert!(in_job.running() && in_job.stoppable());
        assert_eq!(HostState::Queued { job: "16".into(), starting: false }.text(), "Queued · job 16 · waiting for a free node");
        assert_eq!(HostState::Unknown.text(), "Not connected");
        assert_eq!(HostState::Failed("timed out".into()).text(), "Couldn't connect");
        assert!(!HostState::NotRunning.stoppable() && !HostState::Unknown.running());
    }

    #[test]
    fn what_a_check_found() {
        let job = Job { id: "15".into(), node: "n1".into(), ends_at: None, route: String::new() };
        let found = RuntimeState::Running { node: "n1".into(), notebooks: None, job: Some(job) };
        assert_eq!(HostState::from(&found).text(), "Running · job 15");
        let starting = RuntimeState::Queued { job: "16".into(), state: "RUNNING".into(), reason: "n1".into() };
        assert_eq!(HostState::from(&starting).text(), "Starting · job 16");
        let local = RuntimeState::Running { node: "mac".into(), notebooks: Some(3), job: None };
        assert_eq!(HostState::from(&local).text(), "Running · 3 notebooks open");
        assert_eq!(HostState::from(&RuntimeState::NotRunning), HostState::NotRunning);
        assert_eq!(HostState::from(&RuntimeState::Starting), HostState::Starting, "another client is starting it");
    }
}
