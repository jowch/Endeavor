use gpui::*;
use wire::files::RuntimeState;

use crate::theme;

#[derive(Clone, Debug, PartialEq)]
pub enum HostState {
    Unknown,
    Connecting,
    Checking,
    Connected,
    Starting,
    Stopping,
    Running { notebooks: Option<usize>, job: Option<(String, Option<u64>)> },
    Queued { job: String, starting: bool, reason: Option<String> },
    NotRunning,
    Lost,
    SigningIn,
    Failed(String),
}

impl From<&RuntimeState> for HostState {
    fn from(found: &RuntimeState) -> HostState {
        match found {
            RuntimeState::NotRunning => HostState::NotRunning,
            RuntimeState::Running { notebooks, job, .. } => {
                HostState::Running { notebooks: notebooks.map(|n| n as usize), job: job.as_ref().map(|j| (j.id.clone(), j.ends_at)) }
            }
            RuntimeState::Queued { job, state, reason } => {
                let starting = state == "RUNNING";
                HostState::Queued { job: job.clone(), starting, reason: (!starting).then(|| reason.clone()) }
            }
            RuntimeState::Starting => HostState::Starting,
        }
    }
}

impl HostState {
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
            HostState::Queued { job, starting: false, reason } => {
                let why = reason.as_deref().and_then(wire::slurm::reason_text).unwrap_or_else(|| "waiting for a free node".into());
                format!("Queued · job {job} · {why}")
            }
            HostState::Queued { job, starting: true, .. } => format!("Starting · job {job}"),
            HostState::NotRunning => "Not running".into(),
            HostState::Lost => "Can't reach · reconnects by itself".into(),
            HostState::SigningIn => "Signing in…".into(),
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

pub fn running_dot() -> Div {
    div().flex_shrink_0().size(px(6.)).rounded_full().bg(theme::accent())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HostAct {
    Stop,
    CancelJob,
    TryNow,
}

impl HostState {
    pub fn words(&self) -> (String, Option<String>) {
        match self {
            HostState::NotRunning => ("Not running · starts when you open a session there".into(), None),
            HostState::Lost => (self.text(), Some("If it needs your university's VPN, check that it's on.".into())),
            HostState::Failed(reason) => (self.text(), (!reason.is_empty()).then(|| reason.clone())),
            _ => (self.text(), None),
        }
    }

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
        assert_eq!(words(HostState::SigningIn), (("Signing in…".into(), None), None));
        assert_eq!(words(HostState::Queued { job: "16".into(), starting: false, reason: None }), (("Queued · job 16 · waiting for a free node".into(), None), Some(HostAct::CancelJob)));
        assert_eq!(words(HostState::Queued { job: "16".into(), starting: true, reason: None }).1, Some(HostAct::Stop));
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
        assert_eq!(HostState::Queued { job: "16".into(), starting: false, reason: None }.text(), "Queued · job 16 · waiting for a free node");
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
        let waiting = RuntimeState::Queued { job: "17".into(), state: "PENDING".into(), reason: "Priority".into() };
        assert_eq!(HostState::from(&waiting).text(), "Queued · job 17 · other jobs are ahead in the queue");
        let local = RuntimeState::Running { node: "mac".into(), notebooks: Some(3), job: None };
        assert_eq!(HostState::from(&local).text(), "Running · 3 notebooks open");
        assert_eq!(HostState::from(&RuntimeState::NotRunning), HostState::NotRunning);
        assert_eq!(HostState::from(&RuntimeState::Starting), HostState::Starting, "another client is starting it");
    }
}
