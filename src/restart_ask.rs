use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use endeavor_mcp::RestartAnswer;
use gpui::*;

use crate::Workspace;
use crate::connection::Status;
use crate::hosts::HostId;

pub struct Ask {
    pub host: HostId,
    pub session: String,
    pub reason: String,
    pub reply: mpsc::Sender<RestartAnswer>,
}

pub struct Pending {
    pub key: u64,
    pub reply: mpsc::Sender<RestartAnswer>,
    pid: Option<u32>,
    only_notebook: bool,
}

const CHECK: Duration = Duration::from_millis(250);

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

fn already(name: &str) -> String {
    format!("ArgumentError: restarting::Endeavor on {name} is already stopping or starting, so nothing was asked. Wait a moment, then make your call again.")
}

fn already_restarted(name: &str) -> String {
    format!("ArgumentError: restarted::Endeavor on {name} started again since you asked, so nothing was restarted. Check whether the problem is still there.")
}

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
        let pid = self.runtime_pid(&host);
        let only = self.only_notebook_crashed(key);
        let stops = stops_text(&self.connection(&host).map(|c| c.notebooks.clone()).unwrap_or_default());
        let Some(session) = self.session_mut(key) else { return };
        let (id, effects) = session.ask_restart(&reason, &stops, only.as_deref());
        self.restart_replies.insert(id, Pending { key, reply, pid, only_notebook: only.is_some() });
        self.apply_effects(key, effects, cx);
        cx.notify();
    }

    pub fn restart_answered(&mut self, key: u64, id: u64, restart: bool, cx: &mut Context<Self>) {
        let Some(pending) = self.restart_replies.remove(&id) else { return };
        if restart && self.restart_outdated(&pending) {
            let host = self.sessions.iter().find(|s| s.key == key).map(|s| s.place.host.clone());
            let name = host.map_or_else(String::new, |h| self.hosts.name(&h));
            let _ = pending.reply.send(RestartAnswer::Unavailable(already_restarted(&name)));
            let what = if pending.only_notebook { "the notebook".to_owned() } else { format!("Julia on {name}") };
            if let Some(session) = self.session_mut(key) {
                session.note(format!("Nothing was restarted: {what} started again since the agent asked."));
            }
            return cx.notify();
        }
        let _ = pending.reply.send(if restart { RestartAnswer::Restarting } else { RestartAnswer::Declined });
        if restart {
            self.restart_session_runtime(key, cx);
        }
    }

    pub fn restart_session_closed(&mut self, key: u64) {
        self.restart_replies.retain(|_, pending| pending.key != key);
    }

    fn restart_outdated(&self, pending: &Pending) -> bool {
        let Some(host) = self.sessions.iter().find(|s| s.key == pending.key).map(|s| s.place.host.clone()) else { return true };
        let now = self.runtime_pid(&host);
        (now.is_some() && now != pending.pid) || (pending.only_notebook && self.only_notebook_crashed(pending.key).is_none())
    }

    fn runtime_pid(&self, host: &HostId) -> Option<u32> {
        self.connection(host).and_then(|c| c.runtime.as_ref()).map(|r| r.pid)
    }

    fn only_notebook_crashed(&self, key: u64) -> Option<String> {
        let at = self.notebook_at(key)?;
        self.crashes.stopped_at(&at)?;
        if self.connection(&at.0).is_some_and(|c| c.crashed) {
            return None;
        }
        Some(std::path::Path::new(&at.1).file_name().map_or(at.1.clone(), |n| n.to_string_lossy().into_owned()))
    }

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
            None | Some(Status::Failed(_)) => self.connect_host(&host, true, cx),
        }
    }

    pub fn confirm_restart(&mut self, host: &HostId, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.restart_body(host);
        self.restart_dialog(host, body, window, cx);
    }

    fn restart_dialog(&mut self, host: &HostId, body: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.hosts.name(host);
        let on = if *host == HostId::ThisMac { String::new() } else { format!(" on {name}") };
        let host = host.clone();
        self.open_confirm(format!("Restart Julia{on}?"), body, "Restart", window, cx, move |this, _, cx| this.restart_host(&host, cx));
    }

    pub fn confirm_restart_julia(&mut self, host: &HostId, window: &mut Window, cx: &mut Context<Self>) {
        let mut body = stops_text(&self.connection(host).map(|c| c.notebooks.clone()).unwrap_or_default());
        if let Some(lost) = self.unrun_edit_warning(host) {
            body = format!("{body}\n\n{lost}");
        }
        self.restart_dialog(host, body, window, cx);
    }

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
