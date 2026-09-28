//! Settings' "Where notebooks run": each host with what runs there, and Stop,
//! Connect or Check, and its settings.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use wire::files::RuntimeState;

use crate::hosts::{Cluster, HostId, Server};
use crate::new_session::{Glyph, glyph};
use crate::{Workspace, theme};

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
            HostState::Running { job: Some((id, Some(ends))), .. } => format!("Running · job {id} ends {}", crate::when::clock(*ends)),
            HostState::Running { job: Some((id, None)), .. } => format!("Running · job {id}"),
            HostState::Running { notebooks: Some(0), .. } => "Running · no notebooks open".into(),
            HostState::Running { notebooks: Some(1), .. } => "Running · 1 notebook open".into(),
            HostState::Running { notebooks: Some(n), .. } => format!("Running · {n} notebooks open"),
            HostState::Running { notebooks: None, .. } => "Running".into(),
            HostState::Queued { job, starting: false } => format!("Queued · job {job}"),
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

fn small_button(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div().id(id).role(Role::Button).flex_shrink_0().px_2().rounded_sm().cursor_pointer().bg(theme::bg_raised()).hover(|s| s.text_color(theme::text_primary())).child(label)
}

impl Workspace {
    /// Ask before stopping: open notebooks close (their files are saved).
    fn confirm_stop(&mut self, host: HostId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.hosts.name(&host);
        let detail = match self.host_state(&host) {
            HostState::Queued { starting: false, .. } => "Its job waiting in the queue is cancelled.",
            _ => "Open notebooks close; their files are saved.",
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Stop Julia on {name}?"),
            Some(detail),
            // Cancel first, as for Delete: Return then cancels, and Escape too.
            &[PromptButton::cancel("Cancel"), PromptButton::new("Stop")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| this.stop_host(&host, cx));
            }
        })
        .detach();
    }

    /// Settings' "Where notebooks run": This Mac, then servers and clusters.
    pub fn render_hosts(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut rows = vec![self.host_list_row(0, HostId::ThisMac, Glyph::Laptop, "This Mac".into(), None, cx)];
        let servers = self.hosts.servers.iter().filter(|s| s.cluster.is_none());
        let clusters = self.hosts.servers.iter().filter(|s| s.cluster.is_some());
        for (i, server) in servers.chain(clusters).enumerate() {
            let (icon, kind) = if server.cluster.is_some() { (Glyph::Cluster, "cluster · Slurm") } else { (Glyph::Server, "server") };
            rows.push(self.host_list_row(i + 1, HostId::Server(server.id.clone()), icon, server.name.clone(), Some(kind), cx));
        }
        let add = |id: &'static str, text: &'static str, cluster: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(10.))
                .py(px(4.))
                .cursor_pointer()
                .text_color(theme::text_muted())
                .hover(|s| s.text_color(theme::text_primary()))
                .child(glyph(Glyph::Plus, theme::text_muted()))
                .child(text)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let template = Server { cluster: cluster.then(Cluster::default), ..Default::default() };
                    this.open_new_host(template, window, cx);
                }))
        };
        div().flex().flex_col().children(rows).child(add("hosts-add-server", "Add server…", false, cx)).child(add("hosts-add-cluster", "Add cluster…", true, cx))
    }

    fn host_list_row(&self, i: usize, host: HostId, icon: Glyph, name: String, kind: Option<&'static str>, cx: &mut Context<Self>) -> Stateful<Div> {
        let state = self.host_state(&host);
        let this_mac = host == HostId::ThisMac;
        let action = match &state {
            s if s.stoppable() => Some(("Stop", HostAction::Stop)),
            HostState::NotRunning | HostState::Failed(_) if this_mac => Some(("Start", HostAction::Start)),
            HostState::Lost => Some(("Connect", HostAction::Start)),
            HostState::Unknown | HostState::Failed(_) | HostState::Replaced => Some(("Connect", HostAction::Check)),
            HostState::NotRunning | HostState::Connected => Some(("Check", HostAction::Check)),
            _ => None,
        };
        let action = action.map(|(label, what)| {
            let host = host.clone();
            small_button(("host-action", i), label).on_click(cx.listener(move |this, _, window, cx| match what {
                HostAction::Stop => this.confirm_stop(host.clone(), window, cx),
                HostAction::Start => this.ensure_runtime(&host, cx),
                HostAction::Check => this.check_host(&host, cx),
            }))
        });
        let gear = match &host {
            HostId::ThisMac => None,
            HostId::Server(id) => {
                let id = id.clone();
                Some(
                    div()
                        .id(("host-gear", i))
                        .role(Role::Button)
                        .aria_label("Settings")
                        .flex_shrink_0()
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::bg_raised()))
                        .child(glyph(Glyph::Gear, theme::text_muted()))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_server_dialog(Some(id.clone()), window, cx))),
                )
            }
        };
        let why = match &state {
            HostState::Failed(reason) => Some(format!("· {reason}")),
            _ => None,
        };
        div()
            .id(("host-row", i))
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(5.))
            .child(glyph(icon, theme::text_muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
                            .children(kind.map(|k| div().flex_shrink_0().text_size(theme::size_meta()).text_color(theme::text_faint()).child(k))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(theme::size_meta())
                            .text_color(theme::text_muted())
                            .when(state.running(), |d| d.child(running_dot()))
                            .child(div().flex_shrink_0().child(state.text()))
                            .children(why.map(|w| div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(theme::text_faint()).child(w))),
                    ),
            )
            .children(action)
            .children(gear)
    }
}

#[derive(Clone, Copy)]
enum HostAction {
    Stop,
    Start,
    Check,
}

#[cfg(test)]
mod tests {
    use super::HostState;
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
        assert_eq!(in_job.text(), format!("Running · job 15 ends {}", crate::when::clock(ends)));
        assert!(in_job.running() && in_job.stoppable());
        assert_eq!(HostState::Queued { job: "16".into(), starting: false }.text(), "Queued · job 16");
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
    }
}
