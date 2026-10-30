//! A restart of a session's runtime that the agent asks for (EndeavorMCP's
//! `request_restart`): for a runtime an older Endeavor started that keeps
//! running, or for Julia or R that stopped or stopped answering. The host's
//! listener asks the app (`hook`), the session shows a card with the agent's
//! reason and Restart, and only the user's press restarts anything, in every
//! mode. The older-runtime note's own Restart asks the same in a dialog.

use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use endeavor_mcp::RestartAnswer;
use gpui::*;

use crate::Workspace;
use crate::connection::Status;
use crate::hosts::HostId;

/// One agent's request, as the listener hands it to the app.
pub struct Ask {
    pub host: HostId,
    /// The session's key, as the agent's MCP config carries it.
    pub session: String,
    pub reason: String,
    /// Where the user's answer goes; dropped unanswered when a later request of the session replaces it.
    pub reply: mpsc::Sender<RestartAnswer>,
}

/// How often a waiting request checks whether the agent hung up.
const CHECK: Duration = Duration::from_millis(250);

/// The listener's side: each request is handed to the app on `asks`, and its answer waited for
/// until the deadline.
pub fn hook(host: HostId, asks: futures::channel::mpsc::UnboundedSender<Ask>) -> Box<endeavor_mcp::AskRestart> {
    let asks = Mutex::new(asks);
    Box::new(move |session, reason, gone, deadline| {
        let (reply, answer) = mpsc::channel();
        let ask = Ask { host: host.clone(), session: session.to_owned(), reason: reason.to_owned(), reply };
        if asks.lock().unwrap().unbounded_send(ask).is_err() {
            return RestartAnswer::Unavailable("ArgumentError: no_app::Endeavor is closing, so it can't ask the user.".into());
        }
        wait(&answer, gone, deadline)
    })
}

/// The answer on `answer`, or Unanswered at `deadline`, once the agent hung up (`gone`), or once
/// the app let the request go.
fn wait(answer: &mpsc::Receiver<RestartAnswer>, gone: &dyn Fn() -> bool, deadline: Instant) -> RestartAnswer {
    loop {
        match answer.recv_timeout(CHECK.min(deadline.saturating_duration_since(Instant::now()))) {
            Ok(answer) => return answer,
            Err(mpsc::RecvTimeoutError::Disconnected) => return RestartAnswer::Unanswered,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= deadline || gone() {
            return RestartAnswer::Unanswered;
        }
    }
}

/// The agent's reason when the host is already stopping or starting.
fn already(name: &str) -> String {
    format!("ArgumentError: restarting::Endeavor on {name} is already stopping or starting, so nothing was asked. Wait a moment, then make your call again.")
}

/// What the restart confirmations say stops: every notebook open on the host, by file name.
pub fn stops_text(notebooks: &serde_json::Value) -> String {
    let names: Vec<String> = notebooks
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|nb| nb["path"].as_str())
        .map(|path| std::path::Path::new(path).file_name().map_or(path.to_owned(), |n| n.to_string_lossy().into_owned()))
        .collect();
    match names.as_slice() {
        [] => "Notebooks open there stop; their files are saved.".into(),
        [one] => format!("{one} stops; its file is saved."),
        _ => format!("These stop, and their files are saved: {}.", names.join(", ")),
    }
}

impl Workspace {
    /// An agent asks for a restart: its session gets the card, unless there's no such session
    /// on that host, or the host is already stopping or starting.
    pub fn restart_asked(&mut self, ask: Ask, cx: &mut Context<Self>) {
        let Ask { host, session, reason, reply } = ask;
        let key = session.parse::<u64>().ok().filter(|key| self.sessions.iter().any(|s| s.key == *key && s.place.host == host));
        let Some(key) = key else {
            let _ = reply.send(RestartAnswer::Unavailable("ArgumentError: no_session::Endeavor has no session of yours to ask the user in. Tell the user why a restart would help.".into()));
            return;
        };
        if self.connection(&host).is_some_and(|c| c.stopping) || matches!(self.status(&host), Some(Status::Starting | Status::Connecting)) {
            let _ = reply.send(RestartAnswer::Unavailable(already(&self.hosts.name(&host))));
            return;
        }
        let stops = stops_text(&self.connection(&host).map(|c| c.notebooks.clone()).unwrap_or_default());
        let Some(session) = self.session_mut(key) else { return };
        let (id, effects) = session.ask_restart(&reason, &stops);
        // A request already up for the session is this one now; the call that waited on it stops waiting.
        self.restart_replies.insert(id, reply);
        self.apply_effects(key, effects, cx);
        cx.notify();
    }

    /// The user answered the card of request `id` in session `key`: Restart restarts its runtime.
    pub fn restart_answered(&mut self, key: u64, id: u64, restart: bool, cx: &mut Context<Self>) {
        if let Some(reply) = self.restart_replies.remove(&id) {
            // The call may have stopped waiting: the restart goes ahead all the same.
            let _ = reply.send(if restart { RestartAnswer::Restarting } else { RestartAnswer::Declined });
        }
        if restart {
            self.restart_session_runtime(key, cx);
        }
    }

    /// Restart what session `key`'s notebook runs on: after a crash, as the crash page's Restart
    /// does; a runtime that runs, stopped and started again; one that doesn't, started.
    fn restart_session_runtime(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(host) = self.sessions.iter().find(|s| s.key == key).map(|s| s.place.host.clone()) else { return };
        let crashed = self.notebook_at(key).is_some_and(|at| self.crashes.stopped_at(&at).is_some());
        if crashed || self.connection(&host).is_some_and(|c| c.crashed && matches!(c.status, Status::Died(_))) {
            return self.restart_after_crash(key, cx);
        }
        match self.status(&host) {
            Some(Status::Ready) => self.restart_host(&host, cx),
            Some(Status::Browsing | Status::Died(_)) => self.start_host(&host, cx),
            Some(Status::Starting | Status::Connecting) => {}
            None | Some(Status::Failed(_) | Status::Replaced) => self.connect_host(&host, true, cx),
        }
    }

    /// The Restart beside the older-runtime note: the same dialog as the notebook pane's.
    pub fn confirm_restart(&mut self, host: &HostId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.hosts.name(host);
        let on = if *host == HostId::ThisMac { String::new() } else { format!(" on {name}") };
        let host = host.clone();
        let body = self.restart_body(&host);
        self.open_confirm(format!("Restart Julia{on}?"), body, "Restart", window, cx, move |this, _, cx| this.restart_host(&host, cx));
    }

    /// A restart dialog's text for `host`'s runtime that an older Endeavor started.
    pub fn restart_body(&self, host: &HostId) -> String {
        let stops = stops_text(&self.connection(host).map(|c| c.notebooks.clone()).unwrap_or_default());
        let mut body = format!("An older version of Endeavor started it. {stops}");
        if let Some(lost) = self.unrun_edit_warning(host) {
            body = format!("{body}\n\n{lost}");
        }
        body
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]`.
    use super::{RestartAnswer, stops_text, wait};
    use serde_json::json;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn a_wait_ends_with_the_answer_at_the_deadline_or_when_the_app_lets_it_go() {
        let (reply, answer) = mpsc::channel();
        reply.send(RestartAnswer::Declined).unwrap();
        assert_eq!(wait(&answer, &|| false, Instant::now() + Duration::from_secs(5)), RestartAnswer::Declined);
        assert_eq!(wait(&answer, &|| false, Instant::now() + Duration::from_millis(30)), RestartAnswer::Unanswered, "the deadline");
        assert_eq!(wait(&answer, &|| true, Instant::now() + Duration::from_secs(5)), RestartAnswer::Unanswered, "the agent hung up");
        drop(reply);
        assert_eq!(wait(&answer, &|| false, Instant::now() + Duration::from_secs(5)), RestartAnswer::Unanswered, "replaced by a later request");
    }

    #[test]
    fn the_confirmation_names_the_notebooks_that_stop() {
        assert_eq!(stops_text(&json!([])), "Notebooks open there stop; their files are saved.");
        assert_eq!(stops_text(&json!([{ "path": "/w/fit.jl" }])), "fit.jl stops; its file is saved.");
        assert_eq!(stops_text(&json!([{ "path": "/w/fit.jl" }, { "path": "/w/a.R" }])), "These stop, and their files are saved: fit.jl, a.R.");
    }
}
