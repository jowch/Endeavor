//! Julia stopping by itself under a notebook (board XStopped): the pane says
//! so, with Restart Julia, which reopens the notebook and runs it again. If
//! Julia stops again during that run, the notebook opens in safe preview
//! instead, so a cell that takes Julia down can't do it in a loop.
//!
//! A notebook's own Julia (Pluto runs each notebook in a process of its own)
//! is seen to stop through its page, so only for the notebook on screen; all
//! of a host's Julia stopping is heard from its helper, for each notebook
//! that was running there.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::*;

use crate::Workspace;
use crate::failure;
use crate::hosts::HostId;
use crate::new_session::Glyph;
use crate::pluto;
use crate::signin::Look;

/// A notebook's file, on its host.
pub type At = (HostId, String);

/// A rerun that hasn't been seen running by then is taken as done.
const RERUN_GRACE: Duration = Duration::from_secs(120);

/// A stop heard this soon after a restart is the restart's own.
const RESTART_SETTLE: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq)]
pub enum Crash {
    /// The pane says so. `cell`: what was running then, when known.
    Stopped { cell: Option<String>, since: Instant },
    /// Restart Julia reopened it, and it runs its cells again.
    Rerunning { cell: Option<String>, since: Instant, ran: bool },
    /// It stopped again during that run: open in safe preview, with the
    /// callout naming the cell that was running each time.
    Again { first: Option<String>, then: Option<String> },
}

/// What a stop turned out to be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Heard {
    First,
    /// During the run after a restart.
    Again,
    /// Already said.
    Known,
}

#[derive(Debug, Default)]
pub struct Crashes {
    crashes: HashMap<At, Crash>,
    /// The cell each notebook ran as last seen while its Julia was alive, so
    /// a stop can name it.
    running: HashMap<At, Option<String>>,
}

impl Crashes {
    pub fn get(&self, at: &At) -> Option<&Crash> {
        self.crashes.get(at)
    }

    /// The pane's crash page shows for it.
    pub fn stopped_at(&self, at: &At) -> Option<Option<&str>> {
        match self.crashes.get(at) {
            Some(Crash::Stopped { cell, .. }) => Some(cell.as_deref()),
            _ => None,
        }
    }

    /// The cell the notebook ran when last seen alive.
    pub fn was_running(&self, at: &At) -> Option<String> {
        self.running.get(at).cloned().flatten()
    }

    /// Julia stopped under the notebook at `at`, while `cell` ran.
    pub fn stopped(&mut self, at: At, cell: Option<String>, now: Instant) -> Heard {
        match self.crashes.get(&at) {
            Some(Crash::Stopped { .. } | Crash::Again { .. }) => Heard::Known,
            // The restart itself takes the old process down.
            Some(Crash::Rerunning { since, .. }) if now.duration_since(*since) < RESTART_SETTLE => Heard::Known,
            Some(Crash::Rerunning { cell: first, .. }) => {
                let first = first.clone();
                self.crashes.insert(at, Crash::Again { first, then: cell });
                Heard::Again
            }
            None => {
                self.crashes.insert(at, Crash::Stopped { cell, since: now });
                Heard::First
            }
        }
    }

    /// It runs again (Restart Julia, or restarted some other way).
    pub fn restarted(&mut self, at: &At, now: Instant) {
        if let Some(Crash::Stopped { cell, .. }) = self.crashes.get(at) {
            let cell = cell.clone();
            self.crashes.insert(at.clone(), Crash::Rerunning { cell, since: now, ran: false });
        }
    }

    /// The runtime's word on a notebook: the cell it runs, if any, and
    /// whether it may run at all (false while its Julia is gone). A stopped
    /// notebook that may run again was restarted some other way; a rerun is
    /// done once it ran and stopped running.
    pub fn progress(&mut self, at: &At, running: bool, cell: Option<String>, allowed: bool, now: Instant) {
        if allowed {
            self.running.insert(at.clone(), cell.filter(|_| running));
        }
        if matches!(self.crashes.get(at), Some(Crash::Stopped { since, .. }) if allowed && now.duration_since(*since) > RESTART_SETTLE) {
            self.restarted(at, now);
        }
        if let Some(Crash::Rerunning { since, ran, .. }) = self.crashes.get_mut(at) {
            *ran |= running;
            if !running && allowed && (*ran || now.duration_since(*since) > RERUN_GRACE) {
                self.crashes.remove(at);
            }
        }
    }

    pub fn forget(&mut self, at: &At) {
        self.crashes.remove(at);
    }

    /// The notebook opened after a second stop was let run.
    pub fn forget_again(&mut self, at: &At) {
        if matches!(self.crashes.get(at), Some(Crash::Again { .. })) {
            self.crashes.remove(at);
        }
    }

    /// Every notebook of `host` that stopped and hasn't run since.
    pub fn stopped_on(&self, host: &HostId) -> Vec<String> {
        self.crashes.iter().filter(|((h, _), c)| h == host && matches!(c, Crash::Stopped { .. })).map(|((_, path), _)| path.clone()).collect()
    }

    /// The callout's words for a notebook opened in safe preview after a second stop.
    pub fn callout(&self, at: &At) -> Option<(&'static str, String)> {
        let Some(Crash::Again { first, then }) = self.crashes.get(at) else { return None };
        let body = match (first, then) {
            (Some(a), Some(b)) if a == b => format!("So it's open without running. `{a}` was running both times; check it, then run the notebook."),
            (_, Some(b)) => format!("So it's open without running. `{b}` was running when it stopped; check it, then run the notebook."),
            _ => "So it's open without running. Check its cells, then run the notebook.".to_owned(),
        };
        Some(("Julia stopped again while running this notebook", body))
    }
}

impl Workspace {
    /// Session `key`'s notebook file, on its host.
    pub fn notebook_at(&self, key: u64) -> Option<At> {
        let session = self.sessions.iter().find(|s| s.key == key)?;
        Some((session.place.host.clone(), session.notebook_path.clone()?))
    }

    /// The page says whether its notebook's own Julia is there: gone by itself
    /// is a stop, back is a restart.
    pub fn page_process(&mut self, key: u64, dead: bool, was_dead: bool, cx: &mut Context<Self>) {
        let Some(at) = self.notebook_at(key) else { return };
        if dead && !was_dead {
            let cell = self.crashes.was_running(&at);
            match self.crashes.stopped(at.clone(), cell, Instant::now()) {
                Heard::First => eprintln!("Julia stopped under {}", at.1),
                Heard::Again => {
                    eprintln!("Julia stopped again under {} while it ran after a restart; opening it in safe preview", at.1);
                    self.reopen_in_safe_preview(key, at.1, cx);
                }
                Heard::Known => {}
            }
        } else if !dead && was_dead {
            self.crashes.restarted(&at, Instant::now());
        }
        cx.notify();
    }

    /// All of `host`'s Julia stopped by itself, under the notebooks in `paths`.
    /// A second stop while they ran after Restart Julia starts it again with
    /// every notebook in safe preview.
    pub fn julia_died(&mut self, host: &HostId, paths: Vec<String>, cx: &mut Context<Self>) {
        let now = Instant::now();
        let mut again = false;
        for path in paths {
            let at = (host.clone(), path);
            let cell = self.crashes.was_running(&at);
            again |= self.crashes.stopped(at, cell, now) == Heard::Again;
        }
        if again {
            eprintln!("Julia on {} stopped again while its notebooks ran after a restart; starting it with them in safe preview", self.hosts.name(host));
            for path in self.crashes.stopped_on(host) {
                self.crashes.forget(&(host.clone(), path));
            }
            self.start_host(host, cx);
        }
    }

    /// Restart Julia on the crash page: a notebook's own Julia restarts and
    /// runs it again; all of a host's starts again, rerunning the notebooks
    /// that were running.
    pub fn restart_after_crash(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(at) = self.notebook_at(key) else { return };
        let host = at.0.clone();
        let now = Instant::now();
        if self.connection(&host).is_some_and(|c| c.crashed) {
            let paths = self.crashes.stopped_on(&host);
            for path in &paths {
                self.crashes.restarted(&(host.clone(), path.clone()), now);
            }
            if let Some(connection) = self.connections.get_mut(&host) {
                connection.resume = paths.into_iter().map(|p| (p, None)).collect();
            }
            self.start_host(&host, cx);
        } else {
            self.crashes.restarted(&at, now);
            self.restart_notebook(key, cx);
        }
        cx.notify();
    }

    /// Close the notebook and open it again without running it.
    fn reopen_in_safe_preview(&mut self, key: u64, path: String, cx: &mut Context<Self>) {
        let Some(bridge) = self.session_bridge(key) else { return };
        let stop = cx.background_executor().spawn({
            let path = path.clone();
            async move { pluto::stop_notebook(&bridge, &path) }
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = stop.await {
                eprintln!("Couldn't close {path} to open it in safe preview: {e}");
            }
            let _ = this.update(cx, |this, cx| this.open_for_session(key, path, false, cx));
        })
        .detach();
    }

    /// The runtime's notebook list changed: follow each notebook's running
    /// cell, and the runs after Restart Julia.
    pub fn crash_progress(&mut self, host: &HostId) {
        let Some(connection) = self.connections.get(host) else { return };
        let now = Instant::now();
        let seen: Vec<(String, bool, Option<String>, bool)> = connection
            .notebooks
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|nb| {
                let running = nb["running"].as_array().is_some_and(|r| !r.is_empty());
                Some((nb["path"].as_str()?.to_owned(), running, running_cell(nb, &connection.cells), nb["execution_allowed"] == true))
            })
            .collect();
        for (path, running, cell, allowed) in seen {
            self.crashes.progress(&(host.clone(), path), running, cell, allowed, now);
        }
    }

    /// The crash page, for session `key`'s notebook: what's kept, the cell
    /// that was running, Restart Julia, and Julia's log on This Mac.
    pub fn render_crash_page(&self, key: u64, cx: &mut Context<Self>) -> AnyElement {
        let at = self.notebook_at(key);
        let cell = at.as_ref().and_then(|at| self.crashes.stopped_at(at).flatten().map(str::to_owned).or_else(|| self.crashes.was_running(at)));
        let (kept, running) = page_text(cell.as_deref());
        let mut body = vec![div().child(kept).into_any_element()];
        body.extend(running.map(|r| div().child(failure::code_text(&r)).into_any_element()));
        let mut buttons = vec![failure::action("restart-julia", Some(Glyph::Restart), "Restart Julia", Look::Primary)
            .on_click(cx.listener(move |this, _, _, cx| this.restart_after_crash(key, cx)))
            .into_any_element()];
        if at.as_ref().is_some_and(|(host, _)| *host == HostId::ThisMac) {
            buttons.push(failure::action("julia-log", None, "Show log", Look::Plain).on_click(|_, _, _| reveal_julia_log()).into_any_element());
        }
        failure::page(Glyph::Warning, PAGE_TITLE, body, buttons, None).into_any_element()
    }
}

pub const PAGE_TITLE: &str = "Julia stopped unexpectedly";

/// This Mac's Julia log, in Finder.
fn reveal_julia_log() {
    if let Ok(dir) = crate::install::app_dir() {
        crate::platform::reveal(&dir.join("runtime/runtime.log"));
    }
}

/// The crash page's words: what's kept, and the cell that was running.
pub fn page_text(cell: Option<&str>) -> (String, Option<String>) {
    let kept = "The notebook file is saved; its outputs are gone until the cells run again.".to_owned();
    (kept, cell.map(|c| format!("It was running `{c}` when it stopped. Very large data can run out of memory.")))
}

/// The cell a notebook runs, by name when the runtime knows it, from the
/// runtime's notebook entry (`list_notebooks` shape) and its cells (`/events` shape).
pub fn running_cell(nb: &serde_json::Value, cells: &serde_json::Value) -> Option<String> {
    let id = nb["running"].as_array()?.first()?.as_str()?;
    let notebook = nb["notebook_id"].as_str()?;
    let cell = cells[notebook].as_array()?.iter().find(|c| c["cell_id"] == id)?;
    cell["name"].as_str().map(str::to_owned)
}

/// The notebooks that were let run (`list_notebooks` shape): the ones a stop of all of Julia stops.
pub fn running_notebooks(notebooks: &serde_json::Value) -> Vec<String> {
    let allowed = notebooks.as_array().into_iter().flatten().filter(|nb| nb["execution_allowed"] == true);
    allowed.filter_map(|nb| nb["path"].as_str().map(str::to_owned)).collect()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]`.
    use super::{At, Crash, Crashes, Heard, page_text, running_cell, running_notebooks};
    use crate::hosts::HostId;
    use std::time::{Duration, Instant};
    use serde_json::json;

    fn at() -> At {
        (HostId::ThisMac, "/lab/fit_decay.jl".into())
    }

    #[test]
    fn a_second_stop_during_the_rerun_opens_it_without_running() {
        let t = Instant::now();
        let mut c = Crashes::default();
        assert_eq!(c.stopped(at(), Some("rates".into()), t), Heard::First);
        assert_eq!(c.stopped(at(), Some("rates".into()), t), Heard::Known, "said once");
        c.restarted(&at(), t);
        assert_eq!(c.stopped(at(), None, t + Duration::from_secs(1)), Heard::Known, "the restart's own stop");
        c.progress(&at(), true, Some("rates".into()), true, t + Duration::from_secs(2));
        assert_eq!(c.stopped(at(), Some("rates".into()), t + Duration::from_secs(20)), Heard::Again);
        assert_eq!(
            c.callout(&at()),
            Some(("Julia stopped again while running this notebook", "So it's open without running. `rates` was running both times; check it, then run the notebook.".into()))
        );
        c.forget(&at());
        assert_eq!(c.get(&at()), None);
    }

    #[test]
    fn a_rerun_that_finishes_ends_the_watch() {
        let t = Instant::now();
        let mut c = Crashes::default();
        c.stopped(at(), None, t);
        c.restarted(&at(), t);
        c.progress(&at(), false, None, true, t + Duration::from_secs(1));
        assert!(matches!(c.get(&at()), Some(Crash::Rerunning { .. })), "not seen running yet");
        c.progress(&at(), true, Some("rates".into()), true, t + Duration::from_secs(3));
        c.progress(&at(), false, None, true, t + Duration::from_secs(9));
        assert_eq!(c.get(&at()), None);
        assert_eq!(c.stopped(at(), None, t + Duration::from_secs(60)), Heard::First, "a later stop is a first one again");

        let mut quiet = Crashes::default();
        quiet.stopped(at(), None, t);
        quiet.restarted(&at(), t);
        quiet.progress(&at(), false, None, true, t + Duration::from_secs(121));
        assert_eq!(quiet.get(&at()), None, "a rerun never seen running is done after a while");
    }

    #[test]
    fn the_page_names_the_cell_that_was_running() {
        assert_eq!(
            page_text(Some("rates")),
            (
                "The notebook file is saved; its outputs are gone until the cells run again.".into(),
                Some("It was running `rates` when it stopped. Very large data can run out of memory.".into())
            )
        );
        let notebooks = json!([
            { "notebook_id": "n1", "path": "/lab/fit_decay.jl", "running": ["c2"], "execution_allowed": true },
            { "notebook_id": "n2", "path": "/lab/preview.jl", "running": [], "execution_allowed": false },
        ]);
        let cells = json!({ "n1": [{ "cell_id": "c1", "name": "data" }, { "cell_id": "c2", "name": "rates" }] });
        assert_eq!(running_cell(&notebooks[0], &cells).as_deref(), Some("rates"));
        assert_eq!(running_cell(&notebooks[1], &cells), None);
        assert_eq!(running_notebooks(&notebooks), ["/lab/fit_decay.jl"], "a notebook in safe preview wasn't running");

        // The cell is remembered from while Julia ran: the list after a stop can't say.
        let mut c = Crashes::default();
        let t = Instant::now();
        c.progress(&at(), true, Some("rates".into()), true, t);
        c.progress(&at(), false, None, false, t);
        assert_eq!(c.was_running(&at()).as_deref(), Some("rates"));
    }
}
