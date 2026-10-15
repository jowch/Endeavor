//! Endeavor's own R on a Mac, for Settings › Notebooks › R: whether it's
//! installed, with Install and Remove (EndeavorMCP's `r::own_installed`,
//! `install_own` and `remove_own`). The runtime offers to install it when an
//! R notebook finds no R; these do it ahead of time, or undo it.

use endeavor_mcp::r::{self, OwnR};
use futures::StreamExt;
use gpui::*;

use crate::Workspace;
use crate::connection::Status;
use crate::hosts::HostId;
use crate::own_julia::Job;

/// What Settings knows about Endeavor's own R.
#[derive(Default)]
pub struct OwnRState {
    /// As last looked for; none until looked.
    pub found: Option<Option<OwnR>>,
    /// Install or Remove under way, or why the last one failed.
    pub job: Option<Job>,
    /// Remove it once this computer's runtime has stopped: it may be running it.
    remove_after_stop: bool,
}

impl Workspace {
    pub fn check_own_r(&mut self, cx: &mut Context<Self>) {
        let look = cx.background_executor().spawn(async { r::own_installed() });
        cx.spawn(async move |this, cx| {
            let found = look.await;
            let _ = this.update(cx, |this, cx| {
                this.own_r.found = Some(found);
                cx.notify();
            });
        })
        .detach();
    }

    /// Install: in its own thread, since a download takes minutes.
    pub fn install_own_r(&mut self, cx: &mut Context<Self>) {
        if matches!(self.own_r.job, Some(Job::Installing(_) | Job::Removing)) {
            return;
        }
        self.own_r.job = Some(Job::Installing(format!("Installing R {}", r::OWN_VERSION)));
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Result<String, Result<OwnR, String>>>();
        std::thread::spawn(move || {
            let progress = tx.clone();
            let installed = r::install_own(&mut |step| {
                eprintln!("{step}");
                let _ = progress.unbounded_send(Ok(step));
            });
            let _ = tx.unbounded_send(Err(installed));
        });
        cx.spawn(async move |this, cx| {
            while let Some(heard) = rx.next().await {
                let _ = this.update(cx, |this, cx| {
                    match heard {
                        Ok(step) => this.own_r.job = Some(Job::Installing(step)),
                        Err(Ok(own)) => {
                            eprintln!("Installed Endeavor's R: {}", own.rscript.display());
                            this.own_r.job = None;
                            this.own_r.found = Some(Some(own));
                        }
                        Err(Err(why)) => {
                            eprintln!("Couldn't install Endeavor's R: {why}");
                            this.own_r.job = Some(Job::Failed(format!("Couldn't install R. {why}")));
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Remove, after asking. This computer's notebooks stop first, since R notebooks may be running it.
    pub fn confirm_remove_own_r(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let running = self.local_running();
        let mut detail = String::new();
        if running {
            detail.push_str(concat!("Notebooks stop on ", crate::platform::this_computer!(lower), " first. Their files are already saved.\n\n"));
        }
        detail.push_str("The R packages your R notebooks installed with it go too. Endeavor offers it again when an R notebook finds no R, or install it here.");
        self.open_confirm(format!("Remove Endeavor's R {}?", r::OWN_VERSION), detail, "Remove", window, cx, move |this, _, cx| {
            if this.local_running() {
                this.own_r.remove_after_stop = true;
                this.own_r.job = Some(Job::Removing);
                this.stop_host(&HostId::ThisMac, cx);
            } else {
                this.remove_own_r(cx);
            }
        });
    }

    fn local_running(&self) -> bool {
        self.connections.get(&HostId::ThisMac).is_some_and(|c| c.status == Status::Ready)
    }

    /// This computer's runtime stopped (`stopped`: or didn't): a Remove waiting for it goes ahead.
    pub fn own_r_after_stop(&mut self, stopped: bool, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.own_r.remove_after_stop) {
            return;
        }
        if stopped {
            self.remove_own_r(cx);
        } else {
            self.own_r.job = Some(Job::Failed(concat!("Notebooks didn't stop on ", crate::platform::this_computer!(lower), ", so Endeavor's R wasn't removed. Try again.").into()));
            cx.notify();
        }
    }

    fn remove_own_r(&mut self, cx: &mut Context<Self>) {
        self.own_r.job = Some(Job::Removing);
        let remove = cx.background_executor().spawn(async { r::remove_own().map(|()| r::own_installed()) });
        cx.spawn(async move |this, cx| {
            let removed = remove.await;
            let _ = this.update(cx, |this, cx| {
                match removed {
                    Ok(found) => {
                        eprintln!("Removed Endeavor's R");
                        this.own_r.job = None;
                        this.own_r.found = Some(found);
                    }
                    Err(why) => {
                        eprintln!("Couldn't remove Endeavor's R: {why}");
                        this.own_r.job = Some(Job::Failed(format!("Couldn't remove R. {why}")));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
