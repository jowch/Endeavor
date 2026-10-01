//! Debug builds only: what's on screen, as JSON, for scripts that check the
//! app without reading screenshots (scripts/app-state.sh; docs/testing.md).
//!
//! With `ENDEAVOR_STATE_REQUEST` and `ENDEAVOR_STATE_OUT` set, the app looks
//! for the request file five times a second. When it's there, the app deletes
//! it, asks the notebook page for its part, and writes the dump to a temporary
//! file renamed to `ENDEAVOR_STATE_OUT`, so a reader never sees half a dump.
//! Everything here reads the state the views draw from, through the helpers
//! the views use themselves.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::*;
use serde_json::{Value, json};

use crate::approval::{self, Tone};
use crate::celldiff::CellChange;
use crate::connection::HostPane;
use crate::new_session::{EXAMPLES, NotebookChoice};
use crate::notebook_pane::PaneShows;
use crate::session::{self, Entry, Session};
use crate::signin::{Account, Stage};
use crate::transcript;
use crate::{Row, Workspace, runs};

/// Wraps the page's `alert` to record what it showed, then shows it as before.
pub const RECORD_ALERTS: &str = r#"(() => {
  const alerts = [];
  const shown = window.alert;
  Object.defineProperty(window, "__endeavorAlerts", { value: alerts });
  window.alert = function (message) {
    alerts.push(String(message));
    return shown.call(window, message);
  };
})();"#;

/// How long the page gets to answer before the dump goes without it.
const PAGE_WAIT: Duration = Duration::from_secs(2);

impl Workspace {
    /// Answer state requests for the life of the window, if the switches are set.
    pub fn watch_state_requests(&mut self, cx: &mut Context<Self>) {
        let (Some(request), Some(out)) = (std::env::var_os("ENDEAVOR_STATE_REQUEST"), std::env::var_os("ENDEAVOR_STATE_OUT")) else { return };
        let (request, out) = (PathBuf::from(request), PathBuf::from(out));
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                if !request.exists() {
                    continue;
                }
                let _ = std::fs::remove_file(&request);
                let Ok(asked) = this.update(cx, |this, cx| this.ask_page(cx)) else { return };
                let page = match asked {
                    Some(answer) => {
                        let timeout = cx.background_executor().timer(PAGE_WAIT);
                        match futures::future::select(answer, timeout).await {
                            futures::future::Either::Left((Ok(page), _)) => page,
                            _ => json!({ "error": "the page didn't answer in 2 s; an alert may be open" }),
                        }
                    }
                    None => Value::Null,
                };
                let Ok(state) = this.update(cx, |this, cx| this.debug_state(page, cx)) else { return };
                if let Err(e) = write(&out, &state) {
                    eprintln!("state dump: {e}");
                }
            }
        })
        .detach();
    }

    /// Ask the notebook page what it shows, if it's on screen.
    fn ask_page(&mut self, cx: &mut Context<Self>) -> Option<oneshot::Receiver<Value>> {
        if !self.webview.read(cx).visible() {
            return None;
        }
        let (tx, rx) = oneshot::channel();
        self.page_debug = Some(tx);
        self.send_to_page(&json!({ "type": "debug" }), cx);
        Some(rx)
    }

    /// The page's answer to `ask_page`.
    pub fn on_page_debug(&mut self, page: Value) {
        if let Some(tx) = self.page_debug.take() {
            let _ = tx.send(page);
        }
    }

    fn debug_state(&self, page: Value, cx: &App) -> Value {
        let active = self.active_session();
        json!({
            "window": self.window_state(),
            "offline": self.offline_since.map(|since| json!({ "for_secs": since.elapsed().as_secs(), "trying": self.probing })),
            "claude": {
                "state": match self.claude.state {
                    crate::claude_process::State::Up => "up",
                    crate::claude_process::State::Restarting => "restarting",
                    crate::claude_process::State::Down => "down",
                },
                "error": (!self.claude.up()).then(|| self.claude.error.clone()),
                "details_open": self.claude_details_open,
            },
            "usage_limit": self.usage_limit.as_ref().map(|l| json!({
                "resets_in_secs": l.until.map(|at| at.duration_since(std::time::SystemTime::now()).unwrap_or_default().as_secs()),
            })),
            "sign_in": self.sign_in_state(),
            "sidebar": self.sidebar_state(cx),
            "new_session": (self.active.is_none()).then(|| self.new_session_state()),
            "session": active.map(|s| self.session_state(s)),
            "notebook": match active {
                Some(s) => self.notebook_state(s, cx),
                None => self.draft_pane_state(cx),
            },
            "composer": self.composer_state(active, cx),
            "settings": self.settings_debug(cx),
            "confirm": self.confirm.as_ref().map(crate::confirm::Confirm::debug_state),
            "page": page,
        })
    }

    fn window_state(&self) -> Value {
        let modal = if self.server_dialog.is_some() {
            Some("server_dialog")
        } else if !self.asks.is_empty() {
            Some("ssh_prompt")
        } else if self.login_node_warning.is_some() {
            Some("login_node_warning")
        } else {
            None
        };
        let screen = match &self.setup {
            Some(_) if self.offline_since.is_none() && matches!(&self.account, Account::SignedOut(stage) if !matches!(stage, Stage::Expired)) => "sign_in",
            Some(_) => "splash",
            None if self.active.is_some() => "session",
            None => "new_session",
        };
        json!({
            "screen": screen,
            "setup": self.setup.as_ref().map(|s| json!({ "step": s.step().label(), "failed": s.failed(), "offline": self.offline_since.is_some() })),
            "modal": modal,
            "settings_open": self.settings_panel.is_some(),
            "menu_open": self.menu.is_some(),
            "menu": self.menu_state(),
        })
    }

    fn sign_in_state(&self) -> Value {
        let stage = match &self.account {
            Account::Unknown => return json!({ "account": "unknown" }),
            Account::SignedIn => return json!({ "account": "signed_in" }),
            Account::SignedOut(stage) => stage,
        };
        let stage = match stage {
            Stage::Assistant => "choose_assistant",
            Stage::Account => "choose_account",
            Stage::Expired => "expired",
            Stage::Waiting(_) => "waiting_for_browser",
            Stage::Failed { .. } => "failed",
        };
        // The card above the composer: not on the setup screen, nor while offline.
        let card = self.setup.is_none() && self.offline_since.is_none() && stage != "choose_assistant";
        json!({ "account": "signed_out", "stage": stage, "card": card })
    }

    /// A sidebar row's dump: its title, whether it's open or past and
    /// active, its mark (`needs_approval`, `archived`, or none), and whether
    /// it failed. Reads the same helpers the sidebar's own rendering does
    /// (`row_title`, `row_mark`, `row_session_id`), so the dump follows it.
    fn row_debug(&self, row: &Row) -> Value {
        let title = self.row_title(row).unwrap_or_default();
        let open = matches!(row, Row::Open(_));
        let active = matches!(row, Row::Open(key) if self.active == Some(*key));
        let failed = matches!(row, Row::Open(key) if self.sessions.iter().any(|s| s.key == *key && s.failed.is_some()));
        let needs_approval = matches!(row, Row::Open(key) if self.sessions.iter().any(|s| s.key == *key && self.row_mark(s)));
        let archived = self.row_session_id(row).is_some_and(|id| self.archived.contains(&id.to_string()));
        let mark = if needs_approval { Some("needs_approval") } else if archived { Some("archived") } else { None };
        json!({ "title": title, "open": open, "active": active, "mark": mark, "failed": failed })
    }

    fn sidebar_state(&self, cx: &App) -> Value {
        let query = self.sidebar_search.as_ref().map(|s| s.read(cx).value().to_string()).unwrap_or_default();
        let searching = !query.trim().is_empty();
        let filters = &self.settings.sidebar_filters;
        let show_empty = filters.show_empty_folders;
        let group_by_none = filters.group_by == crate::sidebar_filter::GroupBy::None;
        // Group by None: one heading-less entry with every row, flattened and
        // sorted the same way `render_session_bar` builds it, so the dump
        // can't drift from what's on screen.
        let folders: Vec<Value> = if group_by_none {
            let mut rows: Vec<(Row, String)> = self
                .sidebar_folders()
                .into_iter()
                .flat_map(|folder| self.folder_rows(&folder, &query, false, true).0)
                .map(|row| {
                    let title = self.row_title(&row).unwrap_or_default();
                    (row, title)
                })
                .collect();
            crate::sidebar_filter::sort_titles(&mut rows, filters.sort_by, |(_, t)| t.as_str());
            vec![json!({
                "heading": null,
                "collapsed": false,
                "needs_approval": null,
                "rows": rows.iter().map(|(row, _)| self.row_debug(row)).collect::<Vec<_>>(),
                "more": null,
            })]
        } else {
            self.sidebar_folders()
                .into_iter()
                .filter_map(|folder| {
                    let collapsed = !searching && self.settings.collapsed_folders.contains(&folder);
                    let (rows, more, total) = self.folder_rows(&folder, &query, collapsed, false);
                    if total == 0 && (searching || !show_empty) {
                        return None;
                    }
                    let needs_approval = self.sessions.iter().any(|s| s.place == folder && s.needs_approval());
                    Some(json!({
                        "heading": self.folder_heading(&folder),
                        "collapsed": collapsed,
                        "needs_approval": collapsed.then_some(needs_approval),
                        "rows": rows.iter().map(|row| self.row_debug(row)).collect::<Vec<_>>(),
                        "more": more.map(|(label, _)| label),
                    }))
                })
                .collect()
        };
        json!({
            "open": self.settings.layout.sidebar_open,
            "search": searching.then(|| query.clone()),
            "filters": {
                "status": filters.status.label(),
                "where": crate::sidebar_filter::where_label(&filters.where_hosts, |h| self.hosts.name(h)),
                "group_by": filters.group_by.label(),
                "sort_by": filters.sort_by.label(),
                "show_empty_folders": filters.show_empty_folders,
                "changed": !filters.is_default(),
            },
            "collapsed_folders": self.settings.collapsed_folders.iter().map(|f| self.folder_heading(f)).collect::<Vec<_>>(),
            "folders": folders,
            "restart": self.restart_row(),
            "status": self.status_line().1.to_string(),
            "status_mark": self.status_line().0.map(|m| m.label()),
        })
    }

    fn new_session_state(&self) -> Value {
        let resume = self.resumable();
        let chips: Vec<Value> = self.draft_chips().into_iter().map(|c| json!({ "chip": c.id, "label": c.label, "waiting": c.waiting })).collect();
        json!({
            "chips": chips,
            "mode": self.mode_label(None),
            "notice": self.draft.notice.as_ref().map(|n| n.to_string()),
            "connection_notice": self.connection_notice_text().map(|(text, _)| text),
            "resume": (!resume.is_empty()).then(|| resume.iter().map(|r| json!({
                "title": r.title,
                "meta": std::iter::once(r.folder.clone()).chain(r.notebook.clone()).collect::<Vec<_>>().join(" · "),
                "when": r.when,
            })).collect::<Vec<_>>()),
            "examples": resume.is_empty().then(|| EXAMPLES.iter().map(|e| json!({ "prompt": e.prompt, "uses_file": e.uses_file })).collect::<Vec<_>>()),
        })
    }

    fn session_state(&self, s: &Session) -> Value {
        json!({
            "title": s.title,
            "renaming_title": self.renaming.as_ref().is_some_and(|r| r.in_header && r.row == Row::Open(s.key)),
            "folder": self.folder_heading(&s.place),
            "failed": s.failed.as_ref().map(|f| json!({ "message": f.message, "can_copy": f.can_copy })),
            "transcript": self.transcript(s),
            "scroll": scroll_state(s),
            "activity": transcript::activity(s, self.offline_since).map(|a| [Some(a.verb), a.object, a.took].into_iter().flatten().collect::<Vec<_>>().join(" ")),
            "pinned_plan": s.pinned_plan().and_then(|ix| match &s.entries[ix] {
                Entry::Plan(entries) => Some(json!({ "progress": approval::progress(entries), "folded": s.plan_folded, "items": plan_items(entries) })),
                _ => None,
            }),
            "approval": approval::approval_view(s).map(|card| json!({
                "kind": if card.plan.is_some() { "plan" } else { "approval" },
                "title": card.heading,
                "code": card.code,
                "lines": card.lines.into_iter().map(|(text, tone)| json!({ "text": text, "tone": match tone { Tone::Muted => "muted", Tone::Secondary => "secondary" } })).collect::<Vec<_>>(),
                "plan": card.plan.map(|p| json!({ "title": p.title, "steps": p.steps, "open": p.open, "text": if p.open || p.steps.is_none() { Some(p.full) } else { None } })),
                "buttons": card.buttons.iter().map(|b| json!({ "label": b.label, "key": b.hint, "weight": b.weight.label() })).collect::<Vec<_>>(),
            })),
        })
    }

    /// The transcript's entries as drawn, in order: a run of tool calls is
    /// one entry, the pinned plan and pending prompts aren't in it.
    fn transcript(&self, s: &Session) -> Vec<Value> {
        let mut out = Vec::new();
        let mut ix = 0;
        while ix < s.entries.len() {
            if s.pinned_plan() == Some(ix) {
                ix += 1;
                continue;
            }
            if let Some(run) = runs::run_at(&s.entries, ix) {
                let rows: Vec<usize> = run.clone().filter(|&i| matches!(s.entries[i], Entry::Tool { .. } | Entry::Thought { .. })).collect();
                match rows[..] {
                    [only] => out.push(row(s, only, true)),
                    _ => out.push(json!({
                        "kind": "run",
                        "summary": transcript::run_summary(s, run.clone()).0,
                        "open": transcript::run_open(s, &run),
                        "rows": rows.iter().map(|&i| row(s, i, true)).collect::<Vec<_>>(),
                    })),
                }
                ix = run.end;
                continue;
            }
            if let Entry::RunState(warnings) = &s.entries[ix] {
                out.extend(warnings.iter().map(|w| json!({ "kind": "note", "text": transcript::run_state_line(w) })));
                ix += 1;
                continue;
            }
            out.extend(match &s.entries[ix] {
                Entry::User { text, attachments, delivery, sent, .. } => Some(json!({
                    "kind": "user",
                    "text": text.to_string(),
                    "actions": message_actions(!text.is_empty(), *sent, s.copied == Some(ix)),
                    "delivery": transcript::delivery_note(*delivery),
                    "chips": attachments.iter().map(chip_label).collect::<Vec<_>>(),
                    "quotes": quotes(attachments),
                    "unanswered": (s.unanswered == Some(ix)).then(|| self.unanswered_text()),
                })),
                Entry::Agent { text, at } => Some(json!({ "kind": "reply", "text": text, "actions": message_actions(true, *at, s.copied == Some(ix)) })),
                Entry::Note(text) => Some(json!({ "kind": "note", "text": text.to_string() })),
                Entry::Plan(entries) => Some(json!({ "kind": "plan", "progress": approval::progress(entries), "items": plan_items(entries) })),
                Entry::Tool { .. } | Entry::Thought { .. } => Some(row(s, ix, false)),
                Entry::Changes(cells) => Some(json!({
                    "kind": "changes",
                    "cells": cells.iter().map(|c| json!({
                        "name": c.name,
                        "cell": c.cell,
                        "tag": match c.change {
                            CellChange::New => Some("new"),
                            CellChange::Edited => None,
                            CellChange::Deleted => Some("deleted"),
                        },
                        "added": c.added,
                        "removed": c.removed,
                        "shows": c.change != CellChange::Deleted,
                    })).collect::<Vec<_>>(),
                })),
                Entry::Failed(f) if f.used && matches!(f.kind, session::FailedKind::NoAnswer { .. }) => None,
                Entry::Failed(f) => Some(json!({
                    "kind": if matches!(f.kind, session::FailedKind::CutOff) { "note" } else { "failed" },
                    "title": f.kind.title(),
                    "text": f.kind.body(),
                    "button": (!f.used).then(|| f.kind.action()),
                    "details": f.raw.as_ref().map(|raw| json!({ "open": f.details_open, "text": raw })),
                })),
                Entry::Permission { .. } | Entry::RunState(_) => None,
            });
            ix += 1;
        }
        out
    }

    fn notebook_state(&self, s: &Session, cx: &App) -> Value {
        let shown = self.pane_shows(s, cx);
        let file = s.notebook_path.as_deref().map(|p| session::folder_name(Path::new(p)));
        let header = s.notebook_path.as_ref().map(|_| {
            let info = self.header_info(s, shown == PaneShows::Page);
            json!({
                "file": file,
                "host": info.host,
                "tags": info.tags.iter().map(|t| t.label()).collect::<Vec<_>>(),
                "busy": info.busy,
                "job_ends": self.job_end(&s.place.host).map(|(at, soon)| json!({ "at": crate::when::clock(at), "soon": soon })),
            })
        });
        let shows = match &shown {
            PaneShows::Host(_) => "host",
            PaneShows::Missing => "missing",
            PaneShows::NoNotebook => "no_notebook",
            PaneShows::Stopped(_) => "stopped",
            PaneShows::Opening => "opening",
            PaneShows::Page => "page",
        };
        let page = (shown == PaneShows::Page).then(|| {
            let p = &self.page;
            let url = crate::webcontent::url(self.webview.read(cx).raw());
            let reported = s.notebook.as_deref().is_some_and(|id| p.notebook == id);
            json!({
                "notebook": s.notebook,
                "backend": crate::viewed_notebook_id(&url).map(|_| wire::backend::Backend::Pluto.name()),
                "look": self.settings.notebook_theme,
                "safe_preview": reported && p.safe,
                "read_only": self.read_only(s),
                "connected": !reported || p.connected,
            })
        });
        json!({
            "shows": shows,
            "file": file,
            "host_pane": match &shown { PaneShows::Host(pane) => Some(host_pane(pane, &self.hosts.name(&s.place.host))), _ => None },
            "stopped": match &shown { PaneShows::Stopped(st) => Some(json!({ "idle_hours": st.idle_hours, "safe_preview": st.safe_preview })), _ => None },
            "path": s.notebook_path,
            "header": header,
            "warning": self.read_only(s).then(|| {
                let (title, line) = self.pane_warning(s);
                [Some(title), line.map(str::to_owned)].into_iter().flatten().collect::<Vec<_>>().join(" ")
            }),
            "page": page,
            "opening": (shown == PaneShows::Opening).then(|| {
                let (secs, reloaded, button) = self.opening.state(std::time::Instant::now()).unwrap_or_default();
                json!({ "secs": secs, "reloaded": reloaded, "reload_button": button })
            }),
            "find": self.find.as_ref().map(|f| json!({
                "text": f.input.read(cx).value().to_string(),
                "result": crate::find_bar::result_text(f.found),
                "found": f.found,
                "shown": shown == PaneShows::Page,
            })),
        })
    }

    /// The notebook pane before a session starts.
    fn draft_pane_state(&self, cx: &App) -> Value {
        let header = match &self.draft.notebook {
            NotebookChoice::New => "New notebook".to_string(),
            NotebookChoice::Existing(path) => session::folder_name(path),
        };
        let Some(folder) = self.draft_pane_folder() else {
            let pane = self.host_pane_state(&self.draft.host, cx);
            let shows = if pane.is_some() { "host" } else { "empty" };
            return json!({ "shows": shows, "host_pane": pane.map(|p| host_pane(&p, &self.hosts.name(&self.draft.host))), "header": header });
        };
        match (&self.draft.notebook, &self.draft.preview) {
            (NotebookChoice::New, _) => json!({ "shows": "new_notebook", "saved_in": self.draft_tilde(folder), "header": header }),
            (NotebookChoice::Existing(_), None) => json!({ "shows": "loading", "header": header }),
            (NotebookChoice::Existing(_), Some(p)) => json!({ "shows": "safe_preview", "cells_shown": p.cells.len(), "cells": p.total, "header": header }),
        }
    }

    fn composer_state(&self, session: Option<&Session>, cx: &App) -> Value {
        let text = self.input.read(cx).value().to_string();
        let mut above: Vec<Value> = Vec::new();
        let mut notice = |kind: &str, text: String| above.push(json!({ "kind": kind, "text": text }));
        match session {
            Some(s) => {
                if let Some(typed) = text.strip_prefix('/').filter(|t| !t.contains(char::is_whitespace)) {
                    let names: Vec<String> = s.commands.iter().filter(|c| c.name.starts_with(typed)).take(8).map(|c| format!("/{}", c.name)).collect();
                    if !names.is_empty() {
                        notice("commands", names.join(" "));
                    }
                }
                if let Some(line) = self.offline_line(Some(s)) {
                    notice("offline", line.into());
                }
                if let Some(line) = self.usage_line() {
                    notice("usage_limit", line);
                }
                match self.claude.state {
                    crate::claude_process::State::Restarting => notice("claude_restarting", crate::claude_process::RESTARTING.into()),
                    crate::claude_process::State::Down => notice("claude_down", format!("{} · {}", crate::claude_process::DOWN_TITLE, crate::claude_process::DOWN_BODY)),
                    crate::claude_process::State::Up => {}
                }
                if let Some(heading) = self.queue_heading(s) {
                    notice("queue_heading", heading);
                }
            }
            None => {
                if let Some(line) = self.offline_line(None) {
                    notice("offline", line.into());
                }
                if let Some(n) = &self.draft.notice {
                    notice("draft", n.to_string());
                }
                if let Some((text, _)) = self.connection_notice_text() {
                    notice("connection", text);
                }
            }
        }
        if let Some(n) = &self.composer.notice {
            notice("files", n.clone());
        }
        let notebook_open = session.is_some_and(|s| s.notebook_path.is_some());
        let queue: Vec<Value> = session.into_iter().flat_map(|s| &s.outbox.items).map(|q| {
            let label = if q.in_flight() { Some("sending now…") } else if q.is_copying() { Some("copying files…") } else { None };
            json!({ "text": q.text.lines().next().unwrap_or(""), "chips": q.attachments.iter().map(chip_label).collect::<Vec<_>>(), "label": label })
        }).collect();
        json!({
            "text": text,
            "placeholder": self.placeholder,
            "chips": self.composer.attachments.iter().map(chip_label).collect::<Vec<_>>(),
            "quotes": quotes(&self.composer.attachments),
            "reply": self.reply_state(cx),
            "mode": self.mode_label(session),
            "model": self.config_label(session, "model"),
            "effort": self.config_label(session, "effort"),
            "point": json!({ "enabled": notebook_open, "on": self.annotating }),
            "above": above,
            "queue": queue,
            "tips": json!({
                "file": self.active.is_none() && self.file_tip_shows(),
                "point": self.point_tip_shows(self.webview.read(cx).visible() && notebook_open),
            }),
        })
    }
}

/// A tool call or a stretch of thinking, as its row reads.
fn row(s: &Session, ix: usize, in_run: bool) -> Value {
    match &s.entries[ix] {
        Entry::Thought { started, took, expanded, .. } => json!({ "kind": "thought", "text": transcript::thought_label(in_run, *started, *took), "open": expanded }),
        Entry::Tool { title, approval, diffs, expanded, .. } => {
            let Some(r) = transcript::tool_row(s, &s.entries[ix]) else { return Value::Null };
            let mut text = r.line.verb;
            text.extend(r.line.object.map(|o| format!(" {o}")));
            let diff = |d: &crate::celldiff::CellDiff| {
                let count = |c: crate::celldiff::Change| d.lines.iter().filter(|(change, _)| *change == c).count();
                json!({ "label": d.label, "added": count(crate::celldiff::Change::Added), "removed": count(crate::celldiff::Change::Removed) })
            };
            json!({
                "kind": "tool",
                "tool": title,
                "text": text,
                "added": r.added,
                "removed": r.removed,
                "approval": approval.filter(|a| *a != session::Approval::Denied).map(|a| a.label()),
                "state": r.state.map(|s| s.label()),
                "diffs": diffs.iter().chain(&r.file_diff).map(diff).collect::<Vec<_>>(),
                "open": expanded,
            })
        }
        _ => Value::Null,
    }
}

/// Where the transcript is scrolled: the list's top item and the offset into
/// it, the offset from the top in pixels (by the heights the list knows), and
/// each item in view as `[entry index, top from the viewport's top, height]`.
fn scroll_state(s: &Session) -> Value {
    let top = s.list.logical_scroll_top();
    let viewport = s.list.viewport_bounds();
    let rows: Vec<Value> = (top.item_ix..s.list.item_count())
        .map_while(|ix| s.list.bounds_for_item(ix).map(|b| (ix, b)))
        .take_while(|(_, b)| b.top() < viewport.bottom())
        .filter(|(_, b)| b.size.height > px(0.))
        .map(|(ix, b)| json!([ix, f32::from(b.top() - viewport.top()).round(), f32::from(b.size.height).round()]))
        .collect();
    json!({
        "item": top.item_ix,
        "offset": f32::from(top.offset_in_item).round(),
        "px": f32::from(-s.list.scroll_px_offset_for_scrollbar().y).round(),
        "following": s.list.is_following_tail(),
        "at_end": s.list.is_scrolled_to_end(),
        "rows": rows,
    })
}

/// The row under a message, shown on hover: its Copy button, and the time as it reads.
fn message_actions(copy: bool, at: Option<std::time::SystemTime>, copied: bool) -> Value {
    json!({ "copy": copy.then_some(if copied { "Copied" } else { "Copy message" }), "time": at.map(crate::when::ago) })
}

fn plan_items(entries: &[agent_client_protocol::schema::v1::PlanEntry]) -> Vec<Value> {
    entries.iter().map(|e| json!({ "text": e.content, "status": format!("{:?}", e.status).to_lowercase() })).collect()
}

/// The quotes among a message's attachments: where each is from, what it
/// quotes (null for a picture), whether it has a picture, and the comment.
fn quotes(attachments: &[crate::attach::Attachment]) -> Vec<Value> {
    attachments
        .iter()
        .filter_map(|a| match a {
            crate::attach::Attachment::Quote(q) => Some(json!({ "source": q.source(), "excerpt": q.excerpt(), "picture": q.picture().is_some(), "comment": q.comment })),
            _ => None,
        })
        .collect()
}

fn chip_label(a: &crate::attach::Attachment) -> String {
    let label = a.label();
    [label.plain, label.mono].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The pane of a host that isn't ready: "cant_reach", "starting", "stopping",
/// "julia_not_running", "replaced" or "not_connected", the host, and why.
fn host_pane(pane: &HostPane, host: &str) -> Value {
    let (kind, reason) = match pane {
        HostPane::Lost => ("cant_reach", None),
        HostPane::Starting => ("starting", None),
        HostPane::Stopping => ("stopping", None),
        HostPane::NotRunning(reason) => ("julia_not_running", Some(reason)),
        HostPane::Replaced => ("replaced", None),
        HostPane::NotConnected(reason) => ("not_connected", Some(reason)),
    };
    json!({ "kind": kind, "host": host, "reason": reason.filter(|r| !r.is_empty()) })
}

fn write(out: &Path, state: &Value) -> std::io::Result<()> {
    let tmp = out.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
    std::fs::rename(tmp, out)
}
