//! One-off failures (an export, a rename, a run, a new notebook, signing
//! out): a notice under the control that was used, saying what didn't
//! happen, why, and a way on (board XOpen). It stays until it's closed or
//! tried again, and they no longer go to the sidebar's status line.

use std::path::Path;

use gpui::*;

use crate::Workspace;
use crate::failure;
use crate::overlay;
use crate::signin::Look;

/// Where a notice hangs: under the notebook header's file name (rename),
/// under its right-hand buttons (Share, ⋮), at the top of the notebook pane
/// for controls inside it (the safe-preview callout's Run notebook, the
/// pane's New notebook), or at the top of the window over Settings (Sign out).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    NotebookLeft,
    NotebookRight,
    Pane,
    Settings,
}

impl Spot {
    pub fn label(self) -> &'static str {
        match self {
            Spot::NotebookLeft => "notebook_name",
            Spot::NotebookRight => "notebook_buttons",
            Spot::Pane => "notebook_pane",
            Spot::Settings => "settings",
        }
    }
}

/// What the notice's button does again.
#[derive(Clone, Debug, PartialEq)]
pub enum Retry {
    /// Export to…: pick where again.
    Export { key: u64, kind: &'static str, extension: &'static str },
    /// Choose another name: the name box again.
    Rename(u64),
    /// Move to…: pick a folder again.
    MoveTo(u64),
    RunNotebook(String),
    RestartNotebook(u64),
    StopNotebook(u64, String),
    NewNotebook(u64),
    SignOut,
}

impl Retry {
    pub fn label(&self) -> &'static str {
        match self {
            Retry::Export { .. } => "Export to…",
            Retry::Rename(_) => "Choose another name",
            Retry::MoveTo(_) => "Move to…",
            _ => "Try again",
        }
    }
}

pub struct Notice {
    pub spot: Spot,
    pub title: String,
    pub reason: String,
    /// The error as it came, under Details when it says more than the reason.
    pub raw: String,
    pub retry: Option<Retry>,
    pub details_open: bool,
}

impl Notice {
    pub fn new(spot: Spot, title: impl Into<String>, error: &str, retry: Option<Retry>) -> Notice {
        Notice { spot, title: title.into(), reason: reason(error), raw: error.trim().to_owned(), retry, details_open: false }
    }

    pub fn details(&self) -> bool {
        !self.raw.is_empty() && self.raw != self.reason
    }
}

/// An error in a sentence: the plain reason when it can be told, else its
/// first line without the error's type ("ArgumentError: kind::").
pub fn reason(error: &str) -> String {
    let lower = error.to_lowercase();
    if let Some(name) = error.split("file_exists::").nth(1).and_then(|rest| rest.split('\'').nth(1)) {
        let file = Path::new(name).file_name().map_or(name.to_owned(), |f| f.to_string_lossy().into_owned());
        return format!("A file called {file} is already in this folder.");
    }
    if lower.contains("permission denied") || lower.contains("read-only file system") || lower.contains("operation not permitted") {
        return "Endeavor isn't allowed to write there.".into();
    }
    if lower.contains("no space left") {
        return "The disk is full.".into();
    }
    if lower.contains("connection refused") || lower.contains("connection reset") || lower.contains("broken pipe") {
        return "Julia isn't running.".into();
    }
    let line = error.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Something went wrong.");
    let line = line.split_once(": ").filter(|(head, _)| head.ends_with("Error")).map_or(line, |(_, rest)| rest);
    let line = line.split_once("::").filter(|(head, _)| !head.contains(' ')).map_or(line, |(_, rest)| rest);
    let line = line.trim();
    if line.ends_with('.') { line.to_owned() } else { format!("{line}.") }
}

/// An export couldn't be saved in `folder`: the board's words for a folder it can't write.
pub fn export_reason(error: &str, folder: &Path) -> String {
    let said = reason(error);
    if said == "Endeavor isn't allowed to write there." {
        return format!("The folder is read-only: {}. Choose another folder.", crate::new_session::tilde(folder));
    }
    said
}

impl Workspace {
    pub fn show_notice(&mut self, notice: Notice, cx: &mut Context<Self>) {
        eprintln!("{}: {}", notice.title, notice.raw);
        self.notice = Some(notice);
        cx.notify();
    }

    pub fn close_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }

    fn retry_notice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(retry) = self.notice.take().and_then(|n| n.retry) else { return };
        match retry {
            Retry::Export { key, kind, extension } => self.export(key, kind, extension, cx),
            Retry::Rename(key) => self.start_notebook_rename(key, window, cx),
            Retry::MoveTo(key) => self.move_notebook_to(key, cx),
            Retry::RunNotebook(notebook) => self.run_notebook(notebook, cx),
            Retry::RestartNotebook(key) => self.restart_notebook(key, cx),
            Retry::StopNotebook(key, path) => self.stop_notebook(key, path, cx),
            Retry::NewNotebook(key) => self.new_notebook_here(key, cx),
            Retry::SignOut => self.sign_out_now(cx),
        }
        cx.notify();
    }

    /// The notice over the notebook pane's side (`over_settings` false) or
    /// over Settings, drawn above everything and cutting a hole in the web view.
    pub fn render_notice(&self, over_settings: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = self.notice.as_ref().filter(|n| (n.spot == Spot::Settings) == over_settings)?;
        let buttons: Vec<AnyElement> = notice
            .retry
            .as_ref()
            .map(|retry| failure::action("notice-retry", None, retry.label(), Look::Primary).on_click(cx.listener(|this, _, window, cx| this.retry_notice(window, cx))).into_any_element())
            .into_iter()
            .collect();
        let entity = cx.entity().downgrade();
        let close = {
            let entity = entity.clone();
            move |_: &mut Window, cx: &mut App| {
                let _ = entity.update(cx, |this, cx| this.close_notice(cx));
            }
        };
        let details = notice.details().then(|| {
            failure::details("notice-details", notice.raw.clone(), notice.details_open, move |_, cx| {
                let _ = entity.update(cx, |this, cx| {
                    if let Some(n) = &mut this.notice {
                        n.details_open = !n.details_open;
                    }
                    cx.notify();
                });
            })
        });
        let hole = (!over_settings && self.webview.read(cx).visible()).then(|| {
            let webview = self.webview.read(cx);
            let (handle, under) = (webview.handle(), webview.bounds());
            canvas(move |bounds, _, _| overlay::set_hole(handle.raw(), overlay::Hole::Notice, Some(Bounds { origin: bounds.origin - under.origin, size: bounds.size })), |_, _, _, _| ())
                .absolute()
                .size_full()
        });
        let body = failure::notice(notice.title.clone(), notice.reason.clone(), buttons, details, close).relative().children(hole);
        let (placed, wrapper) = match notice.spot {
            Spot::NotebookLeft => (anchored().anchor(Anchor::TopLeft), div().absolute().top(px(46.)).left(px(16.))),
            Spot::NotebookRight | Spot::Pane => (anchored().anchor(Anchor::TopRight), div().absolute().top(px(46.)).right(px(16.))),
            Spot::Settings => (anchored().anchor(Anchor::TopCenter), div().absolute().top(px(12.)).left_1_2()),
        };
        Some(wrapper.child(deferred(placed.child(body)).with_priority(if over_settings { 11 } else { 1 })).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{Retry, export_reason, reason};
    use std::path::Path;

    #[test]
    fn errors_read_as_a_plain_reason() {
        assert_eq!(reason("ArgumentError: file_exists::'/Users/sam/lab/growth.jl' already exists"), "A file called growth.jl is already in this folder.");
        assert_eq!(reason("Permission denied (os error 13)"), "Endeavor isn't allowed to write there.");
        assert_eq!(reason("ArgumentError: execution_blocked::The notebook is in safe preview; Run notebook starts it"), "The notebook is in safe preview; Run notebook starts it.");
        assert_eq!(reason("Couldn't reach the runtime: Connection refused (os error 61)"), "Julia isn't running.");
        assert_eq!(reason("not logged in\nmore"), "not logged in.");
        assert_eq!(
            export_reason("Read-only file system (os error 30)", Path::new("/Volumes/Shared/lab")),
            "The folder is read-only: /Volumes/Shared/lab. Choose another folder."
        );
        assert_eq!(Retry::Export { key: 1, kind: "notebookexport", extension: "html" }.label(), "Export to…");
        assert_eq!(Retry::Rename(1).label(), "Choose another name");
        assert_eq!(Retry::NewNotebook(1).label(), "Try again");
    }
}
