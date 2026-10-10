//! Endeavor's own Julia on this computer, for Settings › Notebooks › Julia:
//! whether it's installed, with Install and Remove (EndeavorMCP's
//! `julia::own_installed`, `install_own` and `remove_own`). The runtime
//! installs it by itself the first time a Julia notebook needs it; these do it
//! ahead of time, or undo it. Where the computer has juliaup, Endeavor's Julia
//! is juliaup's channel for the pinned version, not a second Julia.

use endeavor_mcp::julia::{self, OwnFrom, OwnJulia};
use futures::StreamExt;
use gpui::*;

use crate::Workspace;
use crate::connection::Status;
use crate::hosts::HostId;
use crate::pluto::JuliaStatus;

/// What Settings knows about Endeavor's own Julia.
#[derive(Default)]
pub struct OwnJuliaState {
    /// As last looked for; none until looked.
    pub found: Option<Option<OwnJulia>>,
    /// Install or Remove under way, or why the last one failed.
    pub job: Option<Job>,
    /// Remove it once this computer's runtime has stopped: it was running it.
    remove_after_stop: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    /// The step under way ("Downloading Julia 1.12.6… 42%").
    Installing(String),
    Removing,
    Failed(String),
}

/// What Settings says about it, and whether it offers Remove.
pub fn describe(found: &OwnJulia) -> (&'static str, Option<&'static str>, bool) {
    match found.from {
        OwnFrom::Download => ("Installed", None, true),
        OwnFrom::Juliaup { added: true, default: false } => ("Installed with juliaup", None, true),
        OwnFrom::Juliaup { added: true, default: true } => ("Installed with juliaup", Some("juliaup uses it as its default Julia, so Endeavor leaves it in place."), false),
        OwnFrom::Juliaup { added: false, .. } => ("Installed with juliaup", Some("You added this version to juliaup yourself, so Endeavor leaves it in place."), false),
    }
}

impl Workspace {
    /// Look for Endeavor's own Julia (asking juliaup, if there is one, takes a moment).
    pub fn check_own_julia(&mut self, cx: &mut Context<Self>) {
        let look = cx.background_executor().spawn(async { julia::own_installed() });
        cx.spawn(async move |this, cx| {
            let found = look.await;
            let _ = this.update(cx, |this, cx| {
                this.own_julia.found = Some(found);
                cx.notify();
            });
        })
        .detach();
    }

    /// This computer's runtime is starting Julia now (perhaps downloading it).
    pub fn julia_starting_here(&self) -> bool {
        self.connections.get(&HostId::ThisMac).is_some_and(|c| matches!(c.julia_status, Some(JuliaStatus::Starting { .. })) || c.julia.is_some())
    }

    /// This computer's runtime runs Endeavor's own Julia, or is starting it.
    fn own_julia_running(&self) -> bool {
        let Some(c) = self.connections.get(&HostId::ThisMac).filter(|c| c.status == Status::Ready) else { return false };
        let julia = match c.julia_status {
            Some(JuliaStatus::Ready | JuliaStatus::Starting { .. }) => true,
            Some(JuliaStatus::NotStarted | JuliaStatus::Failed { .. }) => false,
            // A runtime too old to say started Julia with itself.
            None => true,
        };
        let own = matches!(self.settings_checks.local_julia, Some(None)) || self.julia_broken();
        julia && own
    }

    /// Install: in its own thread, since a download takes minutes.
    // ponytail: a Julia notebook opened while this runs can download too; roadmap.md.
    pub fn install_own_julia(&mut self, cx: &mut Context<Self>) {
        if matches!(self.own_julia.job, Some(Job::Installing(_) | Job::Removing)) {
            return;
        }
        self.own_julia.job = Some(Job::Installing(format!("Installing Julia {}", julia::JULIA_VERSION)));
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Result<String, Result<OwnJulia, String>>>();
        std::thread::spawn(move || {
            let progress = tx.clone();
            let installed = julia::install_own(&mut |step| {
                eprintln!("{step}");
                let _ = progress.unbounded_send(Ok(step));
            });
            let _ = tx.unbounded_send(Err(installed));
        });
        cx.spawn(async move |this, cx| {
            while let Some(heard) = rx.next().await {
                let _ = this.update(cx, |this, cx| {
                    match heard {
                        Ok(step) => this.own_julia.job = Some(Job::Installing(step)),
                        Err(Ok(own)) => {
                            eprintln!("Installed Endeavor's Julia: {}", own.julia.display());
                            this.own_julia.job = None;
                            this.own_julia.found = Some(Some(own));
                        }
                        Err(Err(why)) => {
                            eprintln!("Couldn't install Endeavor's Julia: {why}");
                            this.own_julia.job = Some(Job::Failed(format!("Couldn't install Julia. {why}")));
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Remove, after asking: notebooks that run it stop first.
    pub fn confirm_remove_own_julia(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let running = self.own_julia_running();
        let mut detail = String::new();
        if running {
            detail.push_str(concat!("Julia stops on ", crate::platform::this_computer!(lower), " first, with any notebooks open there. Their files are already saved.\n\n"));
        }
        if matches!(self.own_julia.found, Some(Some(OwnJulia { from: OwnFrom::Juliaup { .. }, .. }))) {
            detail.push_str(&format!("This removes Julia {} from juliaup, for anything else that uses it too.\n\n", julia::JULIA_VERSION));
        }
        detail.push_str("Endeavor installs it again the next time you open a Julia notebook, or with Install.");
        self.open_confirm(format!("Remove Endeavor's Julia {}?", julia::JULIA_VERSION), detail, "Remove", window, cx, move |this, _, cx| {
            if this.own_julia_running() {
                this.own_julia.remove_after_stop = true;
                this.own_julia.job = Some(Job::Removing);
                this.stop_host(&HostId::ThisMac, cx);
            } else {
                this.remove_own_julia(cx);
            }
        });
    }

    /// This computer's runtime stopped (`stopped`: or didn't): a Remove waiting for it goes ahead.
    pub fn own_julia_after_stop(&mut self, stopped: bool, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.own_julia.remove_after_stop) {
            return;
        }
        if stopped {
            self.remove_own_julia(cx);
        } else {
            self.own_julia.job = Some(Job::Failed(concat!("Julia didn't stop on ", crate::platform::this_computer!(lower), ", so Endeavor's Julia wasn't removed. Try again.").into()));
            cx.notify();
        }
    }

    fn remove_own_julia(&mut self, cx: &mut Context<Self>) {
        self.own_julia.job = Some(Job::Removing);
        let remove = cx.background_executor().spawn(async { julia::remove_own().map(|()| julia::own_installed()) });
        cx.spawn(async move |this, cx| {
            let removed = remove.await;
            let _ = this.update(cx, |this, cx| {
                match removed {
                    Ok(found) => {
                        eprintln!("Removed Endeavor's Julia");
                        this.own_julia.job = None;
                        this.own_julia.found = Some(found);
                    }
                    Err(why) => {
                        eprintln!("Couldn't remove Endeavor's Julia: {why}");
                        this.own_julia.job = Some(Job::Failed(format!("Couldn't remove Julia. {why}")));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::describe;
    use endeavor_mcp::julia::{OwnFrom, OwnJulia};

    /// A Windows computer's first Julia notebook: the core installs juliaup for
    /// the account (the Microsoft Store with winget, else juliaup's App
    /// Installer file), then adds the pinned Julia's channel, as Settings'
    /// Install does. `.github/workflows/juliaup.yml` runs it.
    #[cfg(windows)]
    #[test]
    #[ignore = "installs juliaup and Julia for this account; run on a computer without juliaup"]
    fn installs_juliaup_and_julia_where_there_is_none() {
        use std::time::Instant;
        let version = endeavor_mcp::julia::JULIA_VERSION;
        assert_eq!(endeavor_mcp::julia::own_installed(), None, "this computer already has Endeavor's Julia");
        let started = Instant::now();
        // When juliaup was in and Julia's channel began, to split the time between the two steps.
        let (mut installing, mut adding) = (false, None);
        let own = endeavor_mcp::julia::install_own(&mut |step| {
            installing |= step.starts_with("Installing juliaup");
            if installing && adding.is_none() && !step.starts_with("Installing juliaup") {
                adding = Some(started.elapsed());
            }
            eprintln!("{step}");
        })
        .unwrap_or_else(|why| panic!("{why}"));
        let total = started.elapsed();
        assert!(installing, "install_own never said it was installing juliaup");
        assert!(matches!(own.from, OwnFrom::Juliaup { added: true, .. }), "{own:?}");
        let installed = adding.unwrap_or(total);
        let minutes = |d: std::time::Duration| format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60);
        eprintln!("juliaup installed in {}, Julia {version} added in {}: {}", minutes(installed), minutes(total - installed), own.julia.display());
        let said = std::process::Command::new(&own.julia).arg("--version").output().unwrap_or_else(|why| panic!("{} --version: {why}", own.julia.display()));
        assert_eq!(String::from_utf8_lossy(&said.stdout).trim(), format!("julia version {version}"));
        assert_eq!(endeavor_mcp::julia::own_installed().map(|own| own.julia), Some(own.julia.clone()), "Settings finds it");
        // For the workflow's summary.
        if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
            use std::io::Write;
            let line = format!("juliaup installed in {}; Julia {version} added in {}.\n", minutes(installed), minutes(total - installed));
            let _ = std::fs::OpenOptions::new().append(true).open(summary).and_then(|mut f| f.write_all(line.as_bytes()));
        }
    }

    #[test]
    fn remove_is_offered_only_for_a_julia_endeavor_put_there_and_may_take_away() {
        let own = |from| OwnJulia { julia: "/x/julia".into(), from };
        assert_eq!(describe(&own(OwnFrom::Download)), ("Installed", None, true));
        assert_eq!(describe(&own(OwnFrom::Juliaup { added: true, default: false })), ("Installed with juliaup", None, true));
        assert!(!describe(&own(OwnFrom::Juliaup { added: true, default: true })).2, "juliaup won't remove its default");
        assert!(!describe(&own(OwnFrom::Juliaup { added: false, default: false })).2, "the person's own channel stays");
    }
}
