//! A session on its way to open: what it waits for, the wait line above the
//! composer, and, for a past session whose history hasn't loaded and that has
//! no copy of its transcript in Endeavor (`transcript_copy`), the summary the
//! chat shows in place of its transcript.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::*;

use crate::Workspace;
use crate::connection::Status;
use crate::failure::{self, Lead};
use crate::hosts::HostId;
use crate::new_session::{Glyph, glyph_at};
use crate::session::Session;
use crate::theme;

/// What a session waits for before it can open.
#[derive(Clone, Debug, PartialEq)]
pub enum Waiting {
    /// Its server dropped and Endeavor keeps trying to reach it.
    Unreachable(String),
    /// Connecting to its server.
    Connecting(String),
    /// Julia is starting: on This Mac, or on the named server.
    Julia(Option<String>),
    /// Claude's process is restarting or stopped (its own line says so); `julia`: Julia is still starting too.
    Claude { julia: bool },
    /// Claude is loading the history.
    Conversation,
    /// Claude is loading the history, and Endeavor's copy of it shows meanwhile.
    Copy,
}

/// The wait line shows the time so far after this long.
const SHOW_TIME_AFTER: Duration = Duration::from_secs(5);

impl Waiting {
    /// The line above the composer, with the time so far once it's been a few
    /// seconds; None where another line says it (Claude's).
    pub fn line(&self, waited: Option<Duration>) -> Option<String> {
        let text = match self {
            Waiting::Unreachable(server) => return Some(format!("Can't reach {server}. Endeavor keeps trying.")),
            Waiting::Connecting(server) => format!("Connecting to {server}…"),
            Waiting::Julia(None) => "Starting Julia…".to_owned(),
            Waiting::Julia(Some(server)) => format!("Starting Julia on {server}…"),
            Waiting::Claude { .. } => return None,
            Waiting::Conversation => return Some("Loading the conversation…".to_owned()),
            Waiting::Copy => return Some("Loading…".to_owned()),
        };
        Some(match waited.filter(|w| *w >= SHOW_TIME_AFTER) {
            Some(w) => format!("{text} {}:{:02}", w.as_secs() / 60, w.as_secs() % 60),
            None => text,
        })
    }

    /// The summary's one muted line on why there's no conversation to read yet.
    pub fn note(&self) -> String {
        match self {
            Waiting::Unreachable(server) => format!("The conversation is kept on {server}, so it shows once Endeavor can reach it."),
            Waiting::Connecting(_) | Waiting::Julia(_) => "The conversation shows once Julia is running.".to_owned(),
            Waiting::Claude { julia: true } => "The conversation shows once Claude and Julia are running.".to_owned(),
            Waiting::Claude { julia: false } => "The conversation shows once Claude is running.".to_owned(),
            Waiting::Conversation | Waiting::Copy => "The conversation appears once all of it has loaded.".to_owned(),
        }
    }

    /// The composer's placeholder: a message waits and goes once the session is open.
    pub fn placeholder(&self) -> String {
        match self {
            Waiting::Unreachable(server) => format!("Write a message. It sends once {server} is back."),
            _ => "Write a message. It sends once the session is open.".to_owned(),
        }
    }
}

/// The summary's second line: "Last active yesterday at 16:40 · fit_decay.jl · lab-server".
pub fn last_active(updated: Option<SystemTime>, notebook: Option<&str>, server: Option<&str>) -> Option<String> {
    let when = updated.map(|at| format!("Last active {}", crate::when::day_at(at)));
    let parts: Vec<String> = when.into_iter().chain(notebook.map(str::to_owned)).chain(server.map(str::to_owned)).collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

impl Workspace {
    /// What `session` waits for: its server, while Endeavor can't reach it;
    /// otherwise what it waits for before it opens, while it's on its way.
    /// None once it's open, when it failed, or when something else holds it
    /// up that has its own place (offline, Julia stopped or failed).
    pub fn session_wait(&self, session: &Session) -> Option<Waiting> {
        if session.failed.is_some() || self.offline_since.is_some() {
            return None;
        }
        let host = &session.place.host;
        let server = (*host != HostId::ThisMac).then(|| self.hosts.name(host));
        let connection = self.connection(host);
        if let Some(server) = &server
            && connection.is_some_and(|c| c.lost.is_some())
        {
            return Some(Waiting::Unreachable(server.clone()));
        }
        if !(session.opening() || session.agent_waiting) {
            return None;
        }
        let julia_ready = self.bridge(host).is_some();
        if !self.claude.up() {
            return Some(Waiting::Claude { julia: !julia_ready });
        }
        if julia_ready {
            return session.opening().then_some(if session.showing_copy() { Waiting::Copy } else { Waiting::Conversation });
        }
        match (connection.map(|c| &c.status), server) {
            (Some(Status::Connecting), Some(server)) => Some(Waiting::Connecting(server)),
            (None | Some(Status::Connecting | Status::Starting | Status::Browsing), server) => Some(Waiting::Julia(server)),
            _ => None,
        }
    }

    /// The wait line's words for `session`, if it shows one.
    pub fn runtime_wait(&self, session: &Session) -> Option<String> {
        self.session_wait(session)?.line(session.opening_since.map(|t| t.elapsed()))
    }

    /// Above the composer while a session is on its way: muted, with a spinner,
    /// or, for a server out of reach, its own mark and a quiet Try now.
    pub fn render_runtime_wait(&self, session: &Session, cx: &mut Context<Self>) -> Option<AnyElement> {
        let waiting = self.session_wait(session)?;
        let text = waiting.line(session.opening_since.map(|t| t.elapsed()))?;
        if let Waiting::Unreachable(_) = waiting {
            let host = session.place.host.clone();
            let try_now = div()
                .id("opening-try-now")
                .role(Role::Button)
                .flex_shrink_0()
                .px(px(6.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_color(theme::text_secondary())
                .hover(|s| s.text_color(theme::text_primary()).bg(theme::row_active()))
                .child("Try now")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.reconnect_lost(&host, cx);
                    cx.notify();
                }));
            return Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(2.))
                    .text_size(theme::chat_meta())
                    .line_height(px(18.))
                    .text_color(theme::text_muted())
                    .child(glyph_at(Glyph::WifiOff, theme::text_muted(), 13. / 12.))
                    .child(div().flex_1().min_w_0().child(text))
                    .child(try_now)
                    .into_any_element(),
            );
        }
        Some(failure::wait_line(Lead::Spinner(ElementId::NamedInteger("runtime-wait".into(), session.key)), text, None, cx).into_any_element())
    }

    /// The summary's lines under the title, while `session`'s history hasn't loaded.
    pub fn opening_summary(&self, session: &Session) -> Option<(Option<String>, String)> {
        if !session.opening() || session.showing_copy() {
            return None;
        }
        let note = self.session_wait(session).unwrap_or(Waiting::Conversation).note();
        let updated = session.id.as_ref().and_then(|id| self.records.get(&id.to_string())?.updated).map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
        let notebook = session.notebook_path.as_deref().map(|p| crate::session::folder_name(std::path::Path::new(p)));
        Some((last_active(updated, notebook.as_deref(), session.server.as_deref()), note))
    }

    /// The chat of a past session whose history hasn't loaded: its title, when
    /// it was last active and its notebook, and one line on what it waits for.
    /// Laid over the whole chat column, so its centre lines up with the
    /// notebook pane's starting block.
    pub fn render_opening_summary(&self, session: &Session) -> Option<AnyElement> {
        let (meta, note) = self.opening_summary(session)?;
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .px(px(32.))
                .text_center()
                .child(div().text_size(theme::chat_subhead()).font_weight(FontWeight::SEMIBOLD).text_color(theme::text_primary()).child(session.title.clone()))
                .children(meta.map(|m| div().text_size(theme::chat_meta()).text_color(theme::text_muted()).child(m)))
                .child(div().max_w(px(380.)).text_size(theme::chat_meta()).text_color(theme::text_muted()).child(note))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `#[test]`.
    use super::{Waiting, last_active};
    use std::time::{Duration, SystemTime};

    #[test]
    fn the_wait_line_names_what_is_on_its_way_with_the_time_after_a_few_seconds() {
        let julia = Waiting::Julia(None);
        assert_eq!(julia.line(Some(Duration::from_secs(2))).as_deref(), Some("Starting Julia…"));
        assert_eq!(julia.line(Some(Duration::from_secs(12))).as_deref(), Some("Starting Julia… 0:12"));
        assert_eq!(Waiting::Julia(Some("lab-server".into())).line(None).as_deref(), Some("Starting Julia on lab-server…"));
        assert_eq!(Waiting::Connecting("lab-server".into()).line(Some(Duration::from_secs(75))).as_deref(), Some("Connecting to lab-server… 1:15"));
        assert_eq!(Waiting::Conversation.line(Some(Duration::from_secs(30))).as_deref(), Some("Loading the conversation…"));
        assert_eq!(Waiting::Copy.line(Some(Duration::from_secs(30))).as_deref(), Some("Loading…"), "Endeavor's copy shows meanwhile");
        assert_eq!(Waiting::Unreachable("lab-server".into()).line(None).as_deref(), Some("Can't reach lab-server. Endeavor keeps trying."));
        assert_eq!(Waiting::Claude { julia: true }.line(None), None, "Claude's own line says it");
    }

    #[test]
    fn the_summary_says_when_it_was_last_active_and_what_it_waits_for() {
        let yesterday = SystemTime::now() - Duration::from_secs(86400);
        let meta = last_active(Some(yesterday), Some("fit_decay.jl"), None).unwrap();
        assert!(meta.starts_with("Last active yesterday at ") && meta.ends_with(" · fit_decay.jl"), "{meta}");
        assert_eq!(last_active(None, Some("de.jl"), Some("lab-server")).as_deref(), Some("de.jl · lab-server"));
        assert_eq!(last_active(None, None, None), None);
        assert_eq!(Waiting::Julia(None).note(), "The conversation shows once Julia is running.");
        assert_eq!(Waiting::Conversation.note(), "The conversation appears once all of it has loaded.");
        assert_eq!(Waiting::Unreachable("lab-server".into()).note(), "The conversation is kept on lab-server, so it shows once Endeavor can reach it.");
        assert_eq!(Waiting::Julia(None).placeholder(), "Write a message. It sends once the session is open.");
        assert_eq!(Waiting::Unreachable("lab-server".into()).placeholder(), "Write a message. It sends once lab-server is back.");
    }
}
