//! Signing in to Claude. First launch asks under the splash: the assistant,
//! then the kind of account, then the browser sign-in (`claude auth login`),
//! which can be cancelled and reopened, and why one didn't finish. Later a
//! sign-in that ran out shows as a card above the composer; the messages
//! Claude couldn't answer wait and go once it's back.

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use serde::{Deserialize, Serialize};

use crate::Workspace;
use crate::new_session::{Glyph, glyph, glyph_at};
use crate::theme;
use crate::theme::FocusRing as _;

/// The two kinds of Claude account the CLI signs in with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// A Claude plan (Pro, Max, Team, Enterprise), at claude.ai.
    ClaudeAi,
    /// An Anthropic Console account, paid by use.
    Console,
}

impl Method {
    fn flag(self) -> &'static str {
        match self {
            Method::ClaudeAi => "--claudeai",
            Method::Console => "--console",
        }
    }

    /// "Signing in with a Claude plan".
    fn choice(self) -> &'static str {
        match self {
            Method::ClaudeAi => "a Claude plan",
            Method::Console => "an Anthropic Console account",
        }
    }

    /// Where the browser page is.
    fn site(self) -> &'static str {
        match self {
            Method::ClaudeAi => "claude.ai",
            Method::Console => "console.anthropic.com",
        }
    }
}

/// Why a sign-in didn't finish, as far as the CLI's last line tells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reason {
    /// The browser page closed, or nobody answered in time.
    BrowserClosed,
    /// A Claude account without a plan that includes Claude Code.
    FreePlan,
    /// A Console account without credit or access.
    ConsoleNoAccess,
    /// No network: the offline state takes over and sign-in waits.
    Offline,
    Other,
}

impl Reason {
    pub fn of(method: Method, line: &str) -> Reason {
        let line = line.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| line.contains(w));
        if has(&["enotfound", "econnrefused", "econnreset", "etimedout", "enetunreach", "getaddrinfo", "fetch failed", "network", "unable to connect", "socket hang up"]) {
            return Reason::Offline;
        }
        match method {
            Method::ClaudeAi if has(&["subscription", "plan", "pro or max", "does not have access", "doesn't have access", "not eligible"]) => return Reason::FreePlan,
            Method::Console if has(&["credit", "billing", "does not have access", "doesn't have access", "organization", "permission"]) => return Reason::ConsoleNoAccess,
            _ => {}
        }
        if has(&["timed out", "timeout", "denied", "cancel", "closed", "aborted"]) {
            return Reason::BrowserClosed;
        }
        Reason::Other
    }

    /// What happened, then what to do about it.
    pub fn says(self) -> (&'static str, Option<&'static str>) {
        match self {
            Reason::BrowserClosed => ("The browser page closed before sign-in finished.", None),
            Reason::FreePlan => (
                "This Claude account has no plan that includes Claude Code. A free account can't be used here.",
                Some("Choose a plan at claude.ai and try again, or sign in with an Anthropic Console account instead."),
            ),
            Reason::ConsoleNoAccess => ("This Console account can't use Claude yet. Ask whoever manages it, or add credit at console.anthropic.com.", None),
            Reason::Offline | Reason::Other => ("Sign-in didn't finish.", None),
        }
    }
}

/// Claude Code's sign-in on this computer: how it signed in, or None when signed out.
pub fn status() -> Result<Option<Method>, String> {
    Ok(profile()?.map(|p| p.method))
}

/// Who is signed in, as `claude auth status` tells it (Settings' Claude page).
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub method: Method,
    pub email: Option<String>,
    /// The plan's name, as people call it ("Claude Max").
    pub plan: String,
    pub org: Option<String>,
}

/// Claude Code's sign-in on this computer, with the account's details; None when signed out.
pub fn profile() -> Result<Option<Profile>, String> {
    let out = crate::agent::claude_cli(&["auth", "status"])?.output().map_err(|e| e.to_string())?;
    let status: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("Couldn't read Claude's sign-in status: {e}"))?;
    profile_of(&status)
}

fn profile_of(status: &serde_json::Value) -> Result<Option<Profile>, String> {
    let signed_in = status["loggedIn"].as_bool().ok_or("Couldn't read Claude's sign-in status.")?;
    if !signed_in {
        return Ok(None);
    }
    let method = if status["authMethod"] == "claude.ai" { Method::ClaudeAi } else { Method::Console };
    let text = |key: &str| status[key].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    Ok(Some(Profile { method, email: text("email"), plan: plan_name(method, text("subscriptionType").as_deref()), org: text("orgName") }))
}

/// `subscriptionType` as the plan's name.
pub fn plan_name(method: Method, subscription: Option<&str>) -> String {
    match (method, subscription) {
        (Method::Console, _) => "Anthropic Console, paid by use".into(),
        (Method::ClaudeAi, None) => "A Claude plan".into(),
        (Method::ClaudeAi, Some(kind)) => {
            let mut chars = kind.chars();
            let first: String = chars.next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            format!("Claude {first}{}", chars.as_str().to_lowercase())
        }
    }
}

/// `claude auth logout` (blocking).
fn log_out() -> Result<(), String> {
    let out = crate::agent::claude_cli(&["auth", "logout"])?.output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}

/// Set on `claude auth login`: the CLI opens its sign-in page through `$BROWSER`,
/// which is this app, so it can keep the page's address for "Open it again".
const URL_FILE_ENV: &str = "ENDEAVOR_LOGIN_URL_FILE";

/// Run as the CLI's `$BROWSER`: keep the address, then open it as usual.
pub fn browser_shim() {
    let (Some(file), Some(url)) = (std::env::var_os(URL_FILE_ENV), std::env::args().nth(1)) else { return };
    let _ = std::fs::write(file, &url);
    std::process::exit(if open_in_browser(&url) { 0 } else { 1 });
}

/// The system's own opener, as the CLI would use without us.
fn open_in_browser(url: &str) -> bool {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(url).status().is_ok_and(|s| s.success())
}

/// A browser sign-in under way.
pub struct Login {
    pub method: Method,
    id: u64,
    pid: i32,
    url_file: PathBuf,
}

impl Login {
    fn start(method: Method) -> Result<(Login, Child), String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let url_file = std::env::temp_dir().join(format!("endeavor-login-{}-{id}", std::process::id()));
        let _ = std::fs::remove_file(&url_file);
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut command = crate::agent::claude_cli(&["auth", "login", method.flag()])?;
        command.env("BROWSER", exe).env(URL_FILE_ENV, &url_file).stdout(Stdio::piped()).stderr(Stdio::piped());
        // Its own process group, so Cancel stops the CLI and whatever it started.
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let child = command.spawn().map_err(|e| format!("Couldn't start the sign-in: {e}"))?;
        Ok((Login { method, id, pid: child.id() as i32, url_file }, child))
    }

    fn cancel(&self) {
        unsafe { libc::kill(-self.pid, libc::SIGTERM) };
        let _ = std::fs::remove_file(&self.url_file);
    }

    /// The sign-in page's address, once the CLI has opened it.
    fn url(&self) -> Option<String> {
        std::fs::read_to_string(&self.url_file).ok().filter(|u| u.starts_with("https://"))
    }
}

/// How a browser sign-in ended.
pub enum Outcome {
    SignedIn(Method),
    Failed(Reason, String),
}

/// Wait for `claude auth login` to end (blocking), then check it took.
fn finish(child: Child, method: Method, url_file: &Path) -> Outcome {
    let out = child.wait_with_output();
    let _ = std::fs::remove_file(url_file);
    let out = match out {
        Ok(out) => out,
        Err(e) => return Outcome::Failed(Reason::Other, e.to_string()),
    };
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let last = text.lines().map(str::trim).rfind(|l| !l.is_empty() && !l.starts_with("Paste code")).unwrap_or_default().to_owned();
    if !out.status.success() {
        return Outcome::Failed(Reason::of(method, &last), last);
    }
    match status() {
        Ok(Some(method)) => Outcome::SignedIn(method),
        Ok(None) => Outcome::Failed(Reason::Other, last),
        Err(e) => Outcome::Failed(Reason::Other, e),
    }
}

/// Where signing in is, while signed out.
pub enum Stage {
    /// First launch: which assistant.
    Assistant,
    /// Which kind of account (a Claude plan or a Console account).
    Account,
    /// Mid-use: the sign-in ran out; sign in again the way it was done last.
    Expired,
    Waiting(Login),
    Failed { method: Method, reason: Reason, detail: String, at: Instant, details: bool },
}

pub enum Account {
    Unknown,
    SignedIn,
    SignedOut(Stage),
}

impl Account {
    pub fn signed_out(&self) -> bool {
        matches!(self, Account::SignedOut(_))
    }
}

/// A button as the boards draw them: 26px, 12px medium text.
#[derive(Clone, Copy, PartialEq)]
pub enum Look {
    /// Filled orange.
    Primary,
    /// Outlined grey (Try now, Cancel).
    Secondary,
    /// Text only, a fill on hover.
    Plain,
}

pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, look: Look) -> Stateful<Div> {
    button_frame(id, look).child(label.into())
}

/// A button to put an icon and a label in.
pub fn button_frame(id: impl Into<ElementId>, look: Look) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .px(px(10.))
        .rounded(px(6.))
        .cursor_pointer()
        .whitespace_nowrap()
        .text_size(theme::size_meta())
        .map(|d| match look {
            Look::Primary => d.bg(theme::accent()).border_1().border_color(theme::accent()).text_color(gpui::white()).font_weight(FontWeight::MEDIUM),
            Look::Secondary => d
                .bg(theme::bg_tag())
                .border_1()
                .border_color(theme::composer_edge())
                .text_color(theme::text_primary())
                .font_weight(FontWeight::MEDIUM)
                .hover(|s| s.bg(theme::bg_raised())),
            Look::Plain => d.text_color(theme::text_primary()).hover(|s| s.bg(theme::row_active())),
        })
}

/// Orange text that acts ("Open it again", "See Claude's plans").
fn link(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(4.))
        .cursor_pointer()
        .text_size(theme::size_meta())
        .text_color(theme::accent_text())
        .hover(|s| s.underline())
        .child(label)
}

fn title(text: impl IntoElement) -> Div {
    div().flex().items_center().gap(px(8.)).text_size(theme::size_subhead()).line_height(px(22.)).font_weight(FontWeight::MEDIUM).text_color(theme::text_primary()).child(text)
}

fn body(text: impl Into<SharedString>) -> Div {
    div().mt(px(-6.)).text_size(theme::size_body()).line_height(theme::line_body()).text_color(theme::text_secondary()).child(text.into())
}

fn meta(text: impl Into<SharedString>, color: Rgba) -> Div {
    div().text_size(theme::size_meta()).line_height(px(17.)).text_color(color).child(text.into())
}

/// One choice with its own button: the first is the one to pick when unsure.
fn choice(name: impl IntoElement, detail: &'static str, first: bool, action: Stateful<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .pl(px(14.))
        .pr(px(12.))
        .py(px(11.))
        .rounded(px(8.))
        .border_1()
        .border_color(if first { theme::composer_edge() } else { theme::border() })
        .bg(if first { theme::bg_choice() } else { theme::bg_sunken() })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().font_weight(FontWeight::MEDIUM).text_color(theme::text_primary()).child(name))
                .child(meta(detail, theme::text_new())),
        )
        .child(action)
}

/// A listed assistant that doesn't work yet.
fn unavailable(name: &'static str, by: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .h(px(34.))
        .pl(px(14.))
        .pr(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme::bg_tag())
        .child(div().flex_1().flex().gap(px(4.)).text_color(theme::text_faint()).child(name).child(div().text_color(theme::text_section()).child(by)))
        .child(div().text_size(theme::size_meta_small()).text_color(theme::text_faint()).child("Not available yet"))
}

impl Workspace {
    /// Signed out, and where to start signing in.
    fn first_stage(&self) -> Stage {
        if self.setup.is_some() {
            Stage::Assistant
        } else if self.settings.sign_in_method.is_some() {
            Stage::Expired
        } else {
            Stage::Account
        }
    }

    /// What the startup check found.
    pub fn on_signed_in(&mut self, method: Option<crate::signin::Method>, cx: &mut Context<Self>) {
        match method {
            Some(method) => {
                self.account = Account::SignedIn;
                self.remember_method(method);
            }
            None => self.signed_out(cx),
        }
        self.finish_setup(cx);
    }

    /// Claude said the sign-in ran out (a failed turn), or the check found it gone.
    pub fn signed_out(&mut self, cx: &mut Context<Self>) {
        if !self.account.signed_out() {
            self.account = Account::SignedOut(self.first_stage());
        }
        self.sync_holds(cx);
        cx.notify();
    }

    fn remember_method(&mut self, method: Method) {
        if self.settings.sign_in_method != Some(method) {
            self.settings.sign_in_method = Some(method);
            self.settings.save();
        }
    }

    /// The window came to the front: the sign-in may have run out meanwhile.
    /// Only a sign-out is taken from this check; signing in again is the card's.
    pub fn recheck_sign_in(&mut self, cx: &mut Context<Self>) {
        const MIN_GAP: Duration = Duration::from_secs(10);
        if !matches!(self.account, Account::SignedIn) || self.sign_in_checked.is_some_and(|at| at.elapsed() < MIN_GAP) {
            return;
        }
        self.sign_in_checked = Some(Instant::now());
        let check = cx.background_executor().spawn(async { status() });
        cx.spawn(async move |this, cx| {
            if let Ok(None) = check.await {
                let _ = this.update(cx, |this, cx| {
                    if matches!(this.account, Account::SignedIn) {
                        this.signed_out(cx);
                    }
                });
            }
        })
        .detach();
    }

    fn set_stage(&mut self, stage: Stage, cx: &mut Context<Self>) {
        if let Account::SignedOut(Stage::Waiting(login)) = &self.account {
            login.cancel();
        }
        self.account = Account::SignedOut(stage);
        cx.notify();
    }

    pub fn begin_sign_in(&mut self, method: Method, cx: &mut Context<Self>) {
        let (login, child) = match Login::start(method) {
            Ok(started) => started,
            Err(e) => return self.set_stage(Stage::Failed { method, reason: Reason::Other, detail: e, at: Instant::now(), details: false }, cx),
        };
        let (id, url_file) = (login.id, login.url_file.clone());
        self.set_stage(Stage::Waiting(login), cx);
        let done = cx.background_executor().spawn(async move { finish(child, method, &url_file) });
        cx.spawn(async move |this, cx| {
            let outcome = done.await;
            let _ = this.update(cx, |this, cx| this.sign_in_ended(id, outcome, cx));
        })
        .detach();
    }

    fn sign_in_ended(&mut self, id: u64, outcome: Outcome, cx: &mut Context<Self>) {
        // A cancelled or replaced sign-in's end is old news.
        let Account::SignedOut(Stage::Waiting(login)) = &self.account else { return };
        if login.id != id {
            return;
        }
        let method = login.method;
        match outcome {
            Outcome::SignedIn(method) => {
                self.account = Account::SignedIn;
                self.remember_method(method);
                self.status = "Signed in to Claude.".into();
                self.finish_setup(cx);
                self.sync_holds(cx);
                self.refresh_profile(cx);
            }
            // Not a sign-in failure: the offline state says what's going on.
            Outcome::Failed(Reason::Offline, _) => self.account = Account::SignedOut(self.back_stage()),
            Outcome::Failed(reason, detail) => self.account = Account::SignedOut(Stage::Failed { method, reason, detail, at: Instant::now(), details: false }),
        }
        cx.notify();
    }

    /// Where Cancel and Back go.
    fn back_stage(&self) -> Stage {
        if self.setup.is_none() && self.settings.sign_in_method.is_some() { Stage::Expired } else { Stage::Account }
    }

    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) -> bool {
        if !matches!(self.account, Account::SignedOut(Stage::Waiting(_))) {
            return false;
        }
        let back = self.back_stage();
        self.set_stage(back, cx);
        true
    }

    pub fn open_sign_in_again(&mut self, cx: &mut Context<Self>) {
        let Account::SignedOut(Stage::Waiting(login)) = &self.account else { return };
        match login.url() {
            Some(url) => {
                cx.background_executor().spawn(async move { open_in_browser(&url) }).detach();
            }
            // The CLI hasn't said where yet: start over, which opens a new page.
            None => self.begin_sign_in(login.method, cx),
        }
    }

    /// Settings' Sign out: asks first, then signs Claude Code out on this computer.
    pub fn sign_out_of_claude(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_confirm("Sign out of Claude?", "Claude stops answering in every session until you sign in again.", "Sign out", window, cx, |_, _, cx| {
            cx.spawn(async move |this, cx| {
                let done = cx.background_executor().spawn(async { log_out() }).await;
                let _ = this.update(cx, |this, cx| {
                    match done {
                        Ok(()) => {
                            this.account = Account::SignedOut(Stage::Account);
                            this.sync_holds(cx);
                        }
                        Err(e) => this.status = format!("Couldn't sign out: {e}").into(),
                    }
                    this.refresh_profile(cx);
                    cx.notify();
                });
            })
            .detach();
        });
    }

    /// The expired card's Enter: sign in the way it was done last.
    pub fn sign_in_again(&mut self, cx: &mut Context<Self>) -> bool {
        if !matches!(self.account, Account::SignedOut(Stage::Expired)) || self.setup.is_some() {
            return false;
        }
        self.begin_sign_in(self.settings.sign_in_method.unwrap_or(Method::ClaudeAi), cx);
        true
    }

    /// The splash's sign-in panel: the progress line's words, whether its bar
    /// shows, the panel, and when the turtle tucked in (a failure).
    pub fn render_sign_in_panel(&self, cx: &mut Context<Self>) -> Option<(AnyElement, bool, Option<Instant>)> {
        let Account::SignedOut(stage) = &self.account else { return None };
        let panel = div().flex().flex_col().gap(px(12.)).p(px(16.)).rounded(px(10.)).border_1().border_color(theme::border()).bg(theme::dialog_bg());
        let tucked = match stage {
            Stage::Failed { at, .. } => Some(*at),
            _ => None,
        };
        let panel = match stage {
            Stage::Assistant => panel
                .child(title("Choose your assistant"))
                .child(body("Endeavor works with an AI assistant that writes and runs code in your notebook."))
                .child(choice(
                    div().flex().gap(px(4.)).child("Claude").child(div().font_weight(FontWeight::NORMAL).text_color(theme::text_muted()).child("by Anthropic")),
                    "Next, you sign in with your Claude account.",
                    true,
                    button("assistant-continue", "Continue", Look::Primary).on_click(cx.listener(|this, _, _, cx| this.set_stage(Stage::Account, cx))),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(unavailable("Cursor", "by Anysphere"))
                        .child(unavailable("Codex", "by OpenAI"))
                        .child(unavailable("Gemini", "by Google")),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(meta("You can change this later in", theme::text_muted()))
                        .child(
                            div().flex().child(
                                link("sign-in-settings", "Settings")
                                    .role(Role::Link)
                                    .aria_label("Open Settings at Assistants")
                                    .line_height(px(17.))
                                    .border_2()
                                    .border_color(gpui::transparent_black())
                                    .track_focus(&self.dialog_focus("sign-in-settings", cx))
                                    .tab_stop(true)
                                    .focus_ring()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_settings_at(crate::settings_panel::Page::Section(crate::settings_panel::Section::Assistants), window, cx)
                                    })),
                            )
                            .child(meta(".", theme::text_muted())),
                        ),
                ),
            Stage::Account => panel.children(self.account_choice(cx)),
            Stage::Expired => return None,
            Stage::Waiting(login) => panel
                .child(title(div().flex().items_center().gap(px(8.)).child(crate::orbit::orbit("sign-in-orbit".into(), 14., cx)).child("Finish signing in in your browser")))
                .child(body(format!("We opened {} in your browser. Sign in there, then come back. This screen moves on by itself.", login.method.site())))
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .text_size(theme::size_meta())
                        .line_height(px(17.))
                        .text_color(theme::text_new())
                        .child("Signing in with")
                        .child(div().text_color(theme::text_primary()).child(login.method.choice())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(meta("Page didn't open?", theme::text_muted()))
                        .child(link("sign-in-reopen", "Open it again").on_click(cx.listener(|this, _, _, cx| this.open_sign_in_again(cx))))
                        .child(div().flex_1())
                        .child(button("sign-in-cancel", "Cancel", Look::Secondary).on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_sign_in(cx);
                        }))),
                ),
            Stage::Failed { .. } => panel.children(self.failure(cx)),
        };
        Some((panel.into_any_element(), !matches!(stage, Stage::Failed { .. }), tucked))
    }

    /// "Sign in to Claude": the two kinds of account, each with its Sign in.
    fn account_choice(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let back = div().mt(px(-6.)).mb(px(-6.)).child(
            div()
                .id("sign-in-back")
                .flex()
                .items_center()
                .gap(px(2.))
                .h(px(24.))
                .ml(px(-4.))
                .pl(px(2.))
                .pr(px(6.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(theme::size_meta())
                .text_color(theme::text_new())
                .hover(|s| s.bg(theme::row_active()))
                .child(glyph(Glyph::Back, theme::text_new()))
                .child("Back")
                .on_click(cx.listener(|this, _, _, cx| {
                    let back = if this.setup.is_some() { Stage::Assistant } else { Stage::Expired };
                    this.set_stage(back, cx);
                })),
        );
        // Mid-use with no earlier sign-in, the card has nowhere to go back to.
        let has_back = self.setup.is_some() || self.settings.sign_in_method.is_some();
        vec![
            has_back.then(|| back.into_any_element()),
            Some(title("Sign in to Claude").into_any_element()),
            Some(body("Endeavor uses your own Claude account to answer you. Which kind do you have?").into_any_element()),
            Some(
                choice(
                    "A Claude plan",
                    "Pro, Max, Team or Enterprise. You pay monthly and chat with Claude at claude.ai.",
                    true,
                    button("sign-in-claude", "Sign in", Look::Primary).on_click(cx.listener(|this, _, _, cx| this.begin_sign_in(Method::ClaudeAi, cx))),
                )
                .into_any_element(),
            ),
            Some(
                choice(
                    "An Anthropic Console account",
                    "You or your lab pay for what you use, at console.anthropic.com. Often set up by a lab or company.",
                    false,
                    button("sign-in-console", "Sign in", Look::Secondary).on_click(cx.listener(|this, _, _, cx| this.begin_sign_in(Method::Console, cx))),
                )
                .into_any_element(),
            ),
            Some(meta("Not sure? If you chat with Claude at claude.ai, choose the first.", theme::text_new()).into_any_element()),
            Some(div().h(px(1.)).mx(px(-16.)).bg(theme::border()).into_any_element()),
            Some(
                div()
                    .mt(px(-2.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(glyph(Glyph::Lock, theme::text_muted()))
                    .child(meta("Sign-in opens in your browser. Your password stays there.", theme::text_muted()))
                    .into_any_element(),
            ),
            Some(
                div()
                    .mt(px(-6.))
                    .pl(px(20.))
                    .child(
                        link("see-plans", "Have neither? See Claude's plans")
                            .child(glyph_at(Glyph::External, theme::accent_text(), 11. / 12.))
                            .on_click(|_, _, cx| cx.open_url(PLANS)),
                    )
                    .into_any_element(),
            ),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// "Sign-in didn't finish": why, and what to do.
    fn failure(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Account::SignedOut(Stage::Failed { method, reason, detail, details, .. }) = &self.account else { return Vec::new() };
        let (method, reason, details) = (*method, *reason, *details);
        let (says, advice) = reason.says();
        let other = (reason == Reason::FreePlan).then(|| {
            button("sign-in-console-instead", "Use a Console account", Look::Secondary).on_click(cx.listener(|this, _, _, cx| this.begin_sign_in(Method::Console, cx)))
        });
        let toggle = (!detail.is_empty()).then(|| {
            div()
                .id("sign-in-details")
                .flex()
                .items_center()
                .gap(px(3.))
                .cursor_pointer()
                .text_size(theme::size_meta())
                .text_color(theme::text_muted())
                .hover(|s| s.text_color(theme::text_primary()))
                .child("Details")
                .child(div().child(if details { "⌄" } else { "›" }))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Account::SignedOut(Stage::Failed { details, .. }) = &mut this.account {
                        *details = !*details;
                    }
                    cx.notify();
                }))
        });
        vec![
            Some(title(div().flex().items_center().gap(px(8.)).child(glyph_at(Glyph::Warning, theme::danger(), 1.2)).child("Sign-in didn't finish")).into_any_element()),
            Some(div().mt(px(-6.)).text_color(theme::text_primary()).child(says).into_any_element()),
            advice.map(|a| meta(a, theme::text_new()).into_any_element()),
            Some(
                div()
                    .mt(px(2.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        button_frame("sign-in-retry", Look::Primary)
                            .child(glyph(Glyph::Restart, gpui::white().into()))
                            .child("Try again")
                            .on_click(cx.listener(move |this, _, _, cx| this.begin_sign_in(method, cx))),
                    )
                    .children(other)
                    .child(div().flex_1())
                    .children(toggle)
                    .into_any_element(),
            ),
            details.then(|| {
                div()
                    .p(px(8.))
                    .rounded(px(6.))
                    .bg(theme::bg_sunken())
                    .font_family(theme::MONO)
                    .text_size(theme::size_meta_small())
                    .text_color(theme::text_muted())
                    .child(detail.clone())
                    .into_any_element()
            }),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// Mid-use, above the composer: sign in again, waiting for the browser, or why it didn't finish.
    pub fn render_sign_in_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Account::SignedOut(stage) = &self.account else { return None };
        // Offline isn't a sign-in problem: that line says what's going on, and sign-in waits.
        if self.setup.is_some() || self.offline_since().is_some() {
            return None;
        }
        let card = |urgent: bool| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(8.))
                .border_1()
                .border_color(if urgent { theme::accent() } else { theme::composer_edge() })
                .bg(theme::bg_card())
                .when(urgent, |d| {
                    d.shadow(vec![BoxShadow {
                        color: Hsla::from(theme::accent()).opacity(0.14),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(3.),
                        inset: false,
                    }])
                })
        };
        let heading = || div().flex().items_center().gap(px(8.)).text_size(px(14.)).line_height(px(20.)).font_weight(FontWeight::MEDIUM).text_color(theme::text_primary());
        let lines = |lines: Vec<String>| div().flex().flex_col().gap(px(2.)).text_size(theme::size_meta()).line_height(px(17.)).text_color(theme::text_new()).children(lines);
        let row = || div().flex().justify_end().items_center().gap(px(6.));
        let hint = |text: &'static str| div().text_size(theme::size_meta_small()).opacity(0.8).child(text);
        Some(
            match stage {
                Stage::Expired => {
                    let last = self.settings.sign_in_method.unwrap_or(Method::ClaudeAi);
                    let kept = self.sessions.iter().any(|s| s.unanswered.is_some() || !s.outbox.items.is_empty());
                    let why = if kept {
                        "Your sign-in has run out, so Claude can't answer. Your message is kept and sends after you sign in."
                    } else {
                        "Your sign-in has run out, so Claude can't answer."
                    };
                    card(true)
                        .child(heading().child("Sign in to Claude again"))
                        .child(lines(vec![why.into(), format!("The notebook still works. Last time you used {}.", last.choice())]))
                        .child(
                            row()
                                .child(button("sign-in-another", "Use another account", Look::Plain).on_click(cx.listener(|this, _, _, cx| this.set_stage(Stage::Account, cx))))
                                .child(button("sign-in-again", "Sign in", Look::Primary).child(hint("↵")).on_click(cx.listener(move |this, _, _, cx| this.begin_sign_in(last, cx)))),
                        )
                }
                Stage::Waiting(login) => card(false)
                    .child(heading().child(crate::orbit::orbit("sign-in-card-orbit".into(), 14., cx)).child("Finish signing in in your browser"))
                    .child(lines(vec![format!("We opened {}. Sign in there; this card goes away by itself and your message sends.", login.method.site())]))
                    .child(
                        row()
                            .child(div().flex_1().text_size(theme::size_meta()).text_color(theme::text_muted()).child("Page didn't open?"))
                            .child(button("sign-in-reopen", "Open it again", Look::Plain).on_click(cx.listener(|this, _, _, cx| this.open_sign_in_again(cx))))
                            .child(
                                button("sign-in-cancel", "Cancel", Look::Plain)
                                    .child(div().text_size(theme::size_meta_small()).text_color(theme::text_muted()).child("esc"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_sign_in(cx);
                                    })),
                            ),
                    ),
                Stage::Account => card(false).gap(px(12.)).px(px(14.)).py(px(12.)).children(self.account_choice(cx)),
                Stage::Failed { .. } => card(false).gap(px(12.)).px(px(14.)).py(px(12.)).children(self.failure(cx)),
                Stage::Assistant => return None,
            }
            .into_any_element(),
        )
    }

    /// Under a message Claude couldn't answer.
    pub fn render_unanswered(&self) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(theme::size_meta())
            .line_height(px(17.))
            .text_color(theme::text_muted())
            .child(glyph(Glyph::Clock, theme::text_muted()))
            .child(self.unanswered_text())
            .into_any_element()
    }

    pub fn unanswered_text(&self) -> &'static str {
        if self.account.signed_out() {
            "Not answered yet. It sends again once you sign in."
        } else {
            "Not answered yet. It sends again when you're back."
        }
    }

    /// Claude can't answer now: messages wait.
    pub fn out_of_reach(&self) -> bool {
        self.account.signed_out() || self.offline_since().is_some()
    }

    /// A session's messages wait: Claude can't be reached, or its server is
    /// reconnecting (its tools would fail).
    pub fn holds(&self, session: &crate::session::Session) -> bool {
        self.out_of_reach() || self.read_only(session)
    }

    /// Hold each session's messages while they can't go; send what waited once
    /// they can.
    pub fn sync_holds(&mut self, cx: &mut Context<Self>) {
        let holds: Vec<bool> = self.sessions.iter().map(|s| self.holds(s)).collect();
        let mut sends = Vec::new();
        for (session, hold) in self.sessions.iter_mut().zip(holds) {
            if hold {
                session.hold();
            } else {
                sends.push((session.key, session.release()));
            }
        }
        for (key, effects) in sends {
            self.apply_effects(key, effects, cx);
        }
    }
}

const PLANS: &str = "https://claude.com/pricing";

#[cfg(test)]
mod tests {
    use super::{Method, Profile, Reason, plan_name, profile_of};

    #[test]
    fn the_plan_reads_as_people_name_it() {
        assert_eq!(plan_name(Method::ClaudeAi, Some("max")), "Claude Max");
        assert_eq!(plan_name(Method::ClaudeAi, Some("pro")), "Claude Pro");
        assert_eq!(plan_name(Method::ClaudeAi, Some("enterprise")), "Claude Enterprise");
        assert_eq!(plan_name(Method::ClaudeAi, None), "A Claude plan");
        assert_eq!(plan_name(Method::Console, None), "Anthropic Console, paid by use");
    }

    #[test]
    fn auth_status_gives_the_account() {
        let status = serde_json::json!({ "loggedIn": true, "authMethod": "claude.ai", "email": "s.okafor@lab.example.edu", "subscriptionType": "max", "orgName": "Okafor Lab" });
        let expected = Profile { method: Method::ClaudeAi, email: Some("s.okafor@lab.example.edu".into()), plan: "Claude Max".into(), org: Some("Okafor Lab".into()) };
        assert_eq!(profile_of(&status), Ok(Some(expected)));
        assert_eq!(profile_of(&serde_json::json!({ "loggedIn": false, "authMethod": "none" })), Ok(None));
        let console = profile_of(&serde_json::json!({ "loggedIn": true, "authMethod": "console", "orgName": "" })).unwrap().unwrap();
        assert_eq!((console.method, console.org), (Method::Console, None));
    }

    #[test]
    fn the_clis_last_line_names_the_reason() {
        assert_eq!(Reason::of(Method::ClaudeAi, "Login failed: Your account does not have access to Claude Code"), Reason::FreePlan);
        assert_eq!(Reason::of(Method::Console, "Login failed: credit balance is too low"), Reason::ConsoleNoAccess);
        assert_eq!(Reason::of(Method::ClaudeAi, "Sign-in timed out before the browser flow completed. Try again."), Reason::BrowserClosed);
        assert_eq!(Reason::of(Method::ClaudeAi, "Login failed: request to https://claude.com failed, reason: getaddrinfo ENOTFOUND claude.com"), Reason::Offline);
        assert_eq!(Reason::of(Method::Console, "Login failed: something else"), Reason::Other);
    }
}
