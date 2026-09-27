//! Adding a server and its settings (the Where menu's gear), with Test
//! connection; and the modal that shows ssh's password, two-factor and
//! host-key prompts.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use wire::askpass::Kind;

use crate::hosts::{HostId, Server};
use crate::new_session::{Glyph, glyph, menu_row};
use crate::remote::{self, Askpass, Cancel, Event, Question};
use crate::settings::IdleStop;
use crate::{Workspace, theme};

pub struct ServerDialog {
    /// The server being edited; None adds one.
    editing: Option<String>,
    name: Entity<InputState>,
    host: Entity<InputState>,
    julia: Entity<InputState>,
    idle_stop: Option<IdleStop>,
    idle_menu: bool,
    /// `Host` entries from ~/.ssh/config.
    ssh_hosts: Vec<String>,
    test: Option<TestRun>,
    confirm_remove: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

struct TestRun {
    id: u64,
    cancel: Arc<Cancel>,
    host: String,
    done: Vec<String>,
    /// What it's doing now, and the latest line of Julia's output.
    working: Option<String>,
    detail: Option<String>,
    failed: Option<String>,
}

enum TestUpdate {
    Event(Event),
    Done(Result<(), String>),
}

/// ssh's question on screen; the password field for a `Secret`.
pub struct AskModal {
    question: Question,
    input: Entity<InputState>,
}

impl Workspace {
    pub fn open_server_dialog(&mut self, editing: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let server = editing.as_deref().and_then(|id| self.hosts.server(id)).cloned().unwrap_or_default();
        let input = |value: String, placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_owned();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
        };
        let name = input(server.name.clone(), "Same as the SSH host", window, cx);
        let host = input(if server.ssh_host.is_empty() { String::new() } else { server.ssh_target() }, "alias, or user@host", window, cx);
        let julia = input(server.julia.clone().unwrap_or_default(), "module load julia", window, cx);
        // The suggestions follow what's typed.
        let subscriptions = vec![cx.subscribe(&host, |_: &mut Workspace, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        })];
        name.update(cx, |s, cx| s.focus(window, cx));
        self.server_dialog = Some(ServerDialog {
            editing,
            name,
            host,
            julia,
            idle_stop: server.idle_stop,
            idle_menu: false,
            ssh_hosts: crate::hosts::ssh_config_hosts(),
            test: None,
            confirm_remove: false,
            error: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn close_server_dialog(&mut self, cx: &mut Context<Self>) {
        if let Some(test) = self.server_dialog.take().and_then(|d| d.test) {
            test.cancel.cancel();
        }
        cx.notify();
    }

    /// The server as the dialog's fields describe it.
    fn dialog_server(&self, cx: &App) -> Result<Server, String> {
        let dialog = self.server_dialog.as_ref().ok_or("no dialog")?;
        let (ssh_host, port) = Server::parse_target(&dialog.host.read(cx).value())?;
        let name = dialog.name.read(cx).value().trim().to_owned();
        let julia = dialog.julia.read(cx).value().trim().replace('\n', "; ");
        Ok(Server {
            id: dialog.editing.clone().unwrap_or_else(Server::new_id),
            name: if name.is_empty() { ssh_host.clone() } else { name },
            ssh_host,
            port,
            julia: (!julia.is_empty()).then_some(julia),
            idle_stop: dialog.idle_stop,
        })
    }

    fn save_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let server = match self.dialog_server(cx) {
            Ok(server) => server,
            Err(e) => return self.dialog_error(e, cx),
        };
        let adding = self.server_dialog.as_ref().is_some_and(|d| d.editing.is_none());
        let id = server.id.clone();
        self.hosts.put(server);
        if let Err(e) = self.hosts.save() {
            return self.dialog_error(e, cx);
        }
        self.close_server_dialog(cx);
        let host = HostId::Server(id);
        if adding {
            self.set_draft_host(host, window, cx);
        } else {
            // Its idle override may have changed.
            self.send_idle_limit(&host, cx);
        }
        self.input.update(cx, |s, cx| s.focus(window, cx));
    }

    fn remove_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.server_dialog.as_ref().and_then(|d| d.editing.clone()) else { return };
        self.hosts.remove(&id);
        if let Err(e) = self.hosts.save() {
            return self.dialog_error(e, cx);
        }
        self.close_server_dialog(cx);
        let host = HostId::Server(id);
        if self.draft.host == host {
            self.set_draft_host(HostId::ThisMac, window, cx);
        }
        self.disconnect_host(&host, cx);
    }

    fn dialog_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(dialog) = &mut self.server_dialog {
            dialog.error = Some(error);
            cx.notify();
        }
    }

    /// Start Test connection, or stop the one under way.
    fn test_server(&mut self, cx: &mut Context<Self>) {
        if let Some(test) = self.server_dialog.as_ref().and_then(|d| d.test.as_ref()).filter(|t| t.working.is_some()) {
            test.cancel.cancel();
            return;
        }
        let server = match self.dialog_server(cx) {
            Ok(server) => server,
            Err(e) => return self.dialog_error(e, cx),
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(Cancel::default());
        let Some(dialog) = &mut self.server_dialog else { return };
        dialog.error = None;
        dialog.test = Some(TestRun {
            id,
            cancel: cancel.clone(),
            host: server.ssh_host.clone(),
            done: Vec::new(),
            working: Some(format!("Connecting to {}…", server.ssh_host)),
            detail: None,
            failed: None,
        });
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<TestUpdate>();
        let questions = self.questions_tx.clone();
        std::thread::spawn(move || {
            let on = |event| drop(tx.unbounded_send(TestUpdate::Event(event)));
            let result = Askpass::start(server.name.clone(), questions).and_then(|askpass| remote::test(&server, Some(&askpass), &cancel, &on));
            let _ = tx.unbounded_send(TestUpdate::Done(result));
        });
        cx.spawn(async move |this, cx| {
            while let Some(update) = rx.next().await {
                if this.update(cx, |this, cx| this.on_test_update(id, update, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn on_test_update(&mut self, id: u64, update: TestUpdate, cx: &mut Context<Self>) {
        if matches!(update, TestUpdate::Done(_)) {
            // Its ssh is gone, so nothing is waiting for these answers any more.
            for ask in self.asks.drain(..) {
                ask.question.answer(None);
            }
        }
        let Some(test) = self.server_dialog.as_mut().and_then(|d| d.test.as_mut()).filter(|t| t.id == id) else { return };
        let step = |test: &mut TestRun, done: String, next: Option<String>| {
            test.done.push(done);
            test.working = next;
            test.detail = None;
        };
        match update {
            TestUpdate::Event(Event::Connected { os, arch }) => step(test, format!("Connected ({os} {arch})"), Some("Checking Endeavor's helper…".into())),
            TestUpdate::Event(Event::Helper { installed }) => {
                let done = if installed { "Installed helper" } else { "Helper already installed" };
                step(test, done.into(), Some("Starting Julia…".into()));
            }
            TestUpdate::Event(Event::FoundJulia { path, version }) => step(test, format!("Found Julia {version} at {path}"), Some("Starting the runtime…".into())),
            TestUpdate::Event(Event::Progress(line)) => {
                let text = line.trim_start_matches(['┌', '│', '└', ' ']).trim();
                if !text.is_empty() {
                    test.detail = Some(text.to_owned());
                }
            }
            TestUpdate::Event(Event::Started { node, reattached }) => {
                let done = if reattached { format!("Runtime already running on {node}") } else { format!("Runtime started on {node}") };
                step(test, done, Some(if reattached { "Leaving it running…" } else { "Stopping it…" }.into()));
            }
            TestUpdate::Event(Event::Finished { stopped }) => step(test, if stopped { "Stopped".into() } else { "Left running".into() }, None),
            TestUpdate::Done(result) => {
                test.working = None;
                test.detail = None;
                test.failed = result.err();
            }
        }
        cx.notify();
    }

    /// A question from ssh: shown now, or after the one on screen.
    pub fn on_question(&mut self, question: Question, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).masked(true));
        cx.subscribe_in(&input, window, |this: &mut Workspace, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.answer_ask(true, window, cx);
            }
        })
        .detach();
        if self.asks.is_empty() {
            input.update(cx, |s, cx| s.focus(window, cx));
        }
        self.asks.push_back(AskModal { question, input });
        cx.notify();
    }

    fn answer_ask(&mut self, yes: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.asks.pop_front() else { return };
        let text = match (yes, ask.question.ask.kind) {
            (false, Kind::YesNo) => Some("no".into()),
            (false, _) => None,
            (true, Kind::Secret) => Some(ask.input.read(cx).value().to_string()),
            (true, Kind::YesNo) => Some("yes".into()),
            (true, Kind::Confirm) => Some(String::new()),
        };
        ask.question.answer(text);
        if let Some(next) = self.asks.front() {
            next.input.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    pub fn render_server_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.server_dialog.as_ref()?;
        let editing = dialog.editing.is_some();
        let title = match dialog.editing.as_deref().and_then(|id| self.hosts.server(id)) {
            Some(server) => server.name.clone(),
            None => "Add server".into(),
        };
        let testing = dialog.test.as_ref().is_some_and(|t| t.working.is_some());
        let query = dialog.host.read(cx).value().trim().to_lowercase();
        let suggestions: Vec<String> = dialog
            .ssh_hosts
            .iter()
            .filter(|h| h.to_lowercase().contains(&query) && h.to_lowercase() != query)
            .take(6)
            .cloned()
            .collect();
        let host_field = field(&dialog.host, true);
        let suggestions = (!suggestions.is_empty()).then(|| {
            div()
                .pb(px(6.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .text_size(theme::size_meta())
                .text_color(theme::text_faint())
                .child("From ~/.ssh/config:")
                .children(suggestions.into_iter().enumerate().map(|(i, host)| {
                    div()
                        .id(("ssh-suggestion", i))
                        .px(px(6.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .bg(theme::bg_tag())
                        .hover(|s| s.bg(theme::bg_raised()))
                        .font_family(theme::MONO)
                        .text_color(theme::text_secondary())
                        .child(host.clone())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(dialog) = &mut this.server_dialog {
                                dialog.host.update(cx, |s, cx| s.set_value(host.clone(), window, cx));
                            }
                        }))
                }))
        });
        let idle_label = match dialog.idle_stop {
            None => format!("Same as Settings ({})", idle_name(self.settings.idle_stop)),
            Some(value) => idle_name(value).to_owned(),
        };
        let idle_menu = dialog.idle_menu.then(|| {
            let current = dialog.idle_stop;
            let rows = std::iter::once((None, format!("Same as Settings ({})", idle_name(self.settings.idle_stop))))
                .chain(IdleStop::ALL.iter().map(|&(value, label)| (Some(value), label.to_owned())))
                .enumerate()
                .map(|(i, (value, label))| {
                    menu_row(("idle-option", i), current == value, false).child(label).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(dialog) = &mut this.server_dialog {
                            dialog.idle_stop = value;
                            dialog.idle_menu = false;
                            cx.notify();
                        }
                    }))
                });
            let rows: Vec<_> = rows.collect();
            div().absolute().top(px(32.)).right_0().child(
                deferred(
                    div()
                        .id("idle-menu")
                        .occlude()
                        .w(px(220.))
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::composer_edge())
                        .bg(theme::bg_raised())
                        .children(rows),
                )
                .with_priority(2),
            )
        });
        let footer = if dialog.confirm_remove {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().text_size(theme::size_meta()).text_color(theme::text_secondary()).child(format!(
                    "Remove {title}? Endeavor forgets it; anything running there keeps running."
                )))
                .child(button("cancel-remove", "Cancel", false).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(dialog) = &mut this.server_dialog {
                        dialog.confirm_remove = false;
                        cx.notify();
                    }
                })))
                .child(button("confirm-remove", "Remove", false).text_color(theme::danger()).on_click(cx.listener(|this, _, window, cx| this.remove_server(window, cx))))
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .when(editing, |d| {
                    d.child(
                        div()
                            .id("remove-server")
                            .cursor_pointer()
                            .text_color(theme::danger())
                            .child("Remove server…")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(dialog) = &mut this.server_dialog {
                                    dialog.confirm_remove = true;
                                    cx.notify();
                                }
                            })),
                    )
                })
                .child(div().flex_1())
                .child(button("cancel-server", "Cancel", false).on_click(cx.listener(|this, _, _, cx| this.close_server_dialog(cx))))
                .child(button("save-server", if editing { "Save" } else { "Add" }, true).on_click(cx.listener(|this, _, window, cx| this.save_server(window, cx))))
        };
        let card = div()
            .id("server-dialog")
            .w(px(520.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(theme::composer_edge())
            .bg(theme::bg_card())
            .text_size(theme::size_body())
            .child(
                div()
                    .px(px(20.))
                    .pt(px(18.))
                    .pb(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(glyph(Glyph::Server, theme::text_muted()))
                    .child(div().text_size(theme::size_subhead()).font_weight(FontWeight::SEMIBOLD).child(title)),
            )
            .child(
                div()
                    .px(px(20.))
                    .pb(px(14.))
                    .flex()
                    .flex_col()
                    .child(section("Connection"))
                    .child(row("Name", div().w(px(260.)).child(field(&dialog.name, false))))
                    .child(row(
                        "SSH host",
                        div().flex().gap(px(8.)).child(div().w(px(200.)).child(host_field)).child(
                            button("test-connection", if testing { "Stop test" } else { "Test connection" }, false)
                                .w(px(124.))
                                .justify_center()
                                .on_click(cx.listener(|this, _, _, cx| this.test_server(cx))),
                        ),
                    ))
                    .child(hint("An alias from ~/.ssh/config, or user@host (add :port if it isn't 22). Endeavor uses the keys and settings there."))
                    .children(suggestions)
                    .children(dialog.test.as_ref().map(render_test))
                    .child(row("How to get Julia", div().w(px(260.)).child(field(&dialog.julia, true))))
                    .child(hint(
                        "A path to julia, or a shell line that puts it on the PATH. Empty: the julia on the server's PATH, else Endeavor downloads its own.",
                    ))
                    .child(divider())
                    .child(section("Notebooks"))
                    .child(row(
                        "Stop idle notebooks after",
                        div().relative().child(
                            div()
                                .id("idle-stop")
                                .w(px(220.))
                                .h(px(28.))
                                .px(px(10.))
                                .flex()
                                .items_center()
                                .justify_between()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(theme::composer_edge())
                                .cursor_pointer()
                                .child(idle_label)
                                .child(glyph(Glyph::Chevron, theme::text_faint()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(dialog) = &mut this.server_dialog {
                                        dialog.idle_menu = !dialog.idle_menu;
                                        cx.notify();
                                    }
                                })),
                        )
                        .children(idle_menu),
                    ))
                    .children(dialog.error.clone().map(|e| div().pt(px(10.)).text_size(theme::size_meta()).text_color(theme::danger()).child(e))),
            )
            .child(div().px(px(20.)).py(px(12.)).border_t_1().border_color(theme::composer_edge()).child(footer));
        Some(modal_backdrop("server-dialog-backdrop").child(card).into_any_element())
    }

    pub fn render_askpass(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ask = self.asks.front()?;
        let question = &ask.question;
        let (no, yes) = match question.ask.kind {
            Kind::Secret => ("Cancel", "Continue"),
            Kind::YesNo | Kind::Confirm => ("No", "Yes"),
        };
        let card = div()
            .id("askpass")
            .w(px(420.))
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .rounded(px(10.))
            .border_1()
            .border_color(theme::composer_edge())
            .bg(theme::bg_card())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(glyph(Glyph::Server, theme::text_muted()))
                    .child(div().text_size(theme::size_subhead()).font_weight(FontWeight::SEMIBOLD).child(question.host.clone())),
            )
            .child(
                div()
                    .text_color(theme::text_secondary())
                    .when(question.ask.kind != Kind::Secret, |d| d.font_family(theme::MONO).text_size(theme::size_code()))
                    .child(question.ask.prompt.clone()),
            )
            .when(question.ask.kind == Kind::Secret, |d| d.child(field(&ask.input, false)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(button("askpass-no", no, false).on_click(cx.listener(|this, _, window, cx| this.answer_ask(false, window, cx))))
                    .child(button("askpass-yes", yes, true).on_click(cx.listener(|this, _, window, cx| this.answer_ask(true, window, cx)))),
            );
        Some(modal_backdrop("askpass-backdrop").child(card).into_any_element())
    }
}

fn render_test(test: &TestRun) -> impl IntoElement {
    let line = |mark: &'static str, color: Rgba, text: String| {
        div().flex().gap(px(8.)).child(div().w(px(12.)).flex_shrink_0().text_color(color).child(mark)).child(div().flex_1().min_w_0().child(text))
    };
    div()
        .mt(px(8.))
        .p(px(10.))
        .flex()
        .flex_col()
        .gap(px(3.))
        .rounded(px(6.))
        .bg(theme::bg_page())
        .text_size(theme::size_meta())
        .text_color(theme::text_secondary())
        .children(test.done.iter().map(|d| line("✓", theme::accent_text(), d.clone())))
        .children(test.working.clone().map(|w| line("…", theme::text_faint(), w)))
        .children(test.detail.clone().map(|d| {
            div().pl(px(20.)).overflow_hidden().whitespace_nowrap().text_ellipsis().font_family(theme::MONO).text_size(theme::size_meta_small()).text_color(theme::text_faint()).child(d)
        }))
        .children(test.failed.clone().map(|f| line("✕", theme::danger(), f)))
        .when(test.working.is_none() && test.failed.is_none(), |d| {
            d.child(div().pt(px(2.)).text_color(theme::text_faint()).child(format!("{} works with Endeavor.", test.host)))
        })
}

fn idle_name(value: IdleStop) -> &'static str {
    IdleStop::ALL.iter().find(|(v, _)| *v == value).map_or("", |(_, label)| label)
}

/// A dimmed cover over the window, centering its child; clicks stay in it.
/// Only the cover occludes: an occluding card keeps clicks from its text fields.
fn modal_backdrop(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x0000_0099))
        .text_color(theme::text_primary())
}

fn field(state: &Entity<InputState>, mono: bool) -> Div {
    field_frame(mono).child(div().flex_1().child(Input::new(state).appearance(false).text_size(if mono { theme::size_code() } else { theme::size_body() })))
}

fn field_frame(mono: bool) -> Div {
    div()
        .h(px(28.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(theme::composer_edge())
        .bg(theme::bg_page())
        .when(mono, |d| d.font_family(theme::MONO))
}

fn button(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .flex_shrink_0()
        .h(px(28.))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .cursor_pointer()
        .when(primary, |d| d.bg(theme::accent()).text_color(theme::text_primary()))
        .when(!primary, |d| d.border_1().border_color(theme::composer_edge()).hover(|s| s.bg(theme::bg_raised())))
        .child(label)
}

fn row(label: &'static str, control: impl IntoElement) -> Div {
    div().min_h(px(36.)).flex().items_center().justify_between().gap(px(12.)).child(div().text_color(theme::text_secondary()).child(label)).child(control)
}

fn section(text: &'static str) -> Div {
    div().pt(px(10.)).pb(px(2.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child(text)
}

fn hint(text: &'static str) -> Div {
    div().pb(px(4.)).text_size(theme::size_meta()).text_color(theme::text_faint()).child(text)
}

fn divider() -> Div {
    div().mt(px(10.)).h(px(1.)).bg(theme::composer_edge())
}
