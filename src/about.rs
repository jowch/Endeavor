//! Endeavor ▸ About Endeavor: a small window with the icon, version and build,
//! credits, Website and Licences links, and a strip of update notices at the
//! bottom. Licences opens a second window listing the parts Endeavor ships or
//! installs and their licences.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::{Workspace, theme};

pub const WEBSITE: &str = "https://github.com/jowch/Endeavor";
pub const HELP: &str = "https://github.com/jowch/Endeavor#readme";
pub const REPORT_ISSUE: &str = "https://github.com/jowch/Endeavor/issues/new";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD: &str = env!("ENDEAVOR_BUILD");
const ICON: &[u8] = include_bytes!("../assets/icon/endeavor-256.png");

/// What the app knows about updates. The app can't update itself yet, so
/// `app` is always `None` for now; the notice for it is drawn by the same code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Updates {
    /// A newer Endeavor, downloaded and ready once the app restarts.
    pub app: Option<String>,
    pub adapter: Adapter,
}

/// The Claude Code adapter the app pins: a new app version can pin a new one,
/// which the app installs when it starts the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adapter {
    Current,
    /// Being installed now (or about to be, once Julia is up).
    Installing(String),
    /// Not installed because starting the agent failed; Update tries again.
    Available(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    CheckNow,
    Restart,
    UpdateAdapter,
}

/// One row of the update strip: its text, and its button (primary or not).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub button: Option<(&'static str, Action, bool)>,
    /// The quiet "everything is up to date" row: a check mark instead of a dot.
    pub ok: bool,
}

/// The strip's rows, top to bottom. The app update comes first and gets the
/// orange button because it needs a restart; the adapter's button is then secondary.
pub fn notices(updates: &Updates) -> Vec<Notice> {
    let mut rows = Vec::new();
    if let Some(version) = &updates.app {
        rows.push(Notice { text: format!("Endeavor {version} is ready to install."), button: Some(("Restart", Action::Restart, true)), ok: false });
    }
    let primary = updates.app.is_none();
    match &updates.adapter {
        Adapter::Current => {}
        Adapter::Installing(version) => rows.push(Notice { text: format!("Installing Claude Code adapter {version}…"), button: None, ok: false }),
        Adapter::Available(version) => rows.push(Notice {
            text: format!("Claude Code adapter {version} is available."),
            button: Some(("Update", Action::UpdateAdapter, primary)),
            ok: false,
        }),
    }
    if rows.is_empty() {
        rows.push(Notice { text: "Everything is up to date.".into(), button: Some(("Check now", Action::CheckNow, false)), ok: true });
    }
    rows
}

/// One part that updates, as Settings' About lists it: Endeavor, then the adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateRow {
    pub key: &'static str,
    pub name: &'static str,
    pub state: String,
    /// Its button: label, action, and whether it's the primary one.
    pub button: Option<(&'static str, Action, bool)>,
    /// Something to do (orange words).
    pub attention: bool,
}

/// The same updates as `notices`, one row per part.
pub fn parts(updates: &Updates) -> [UpdateRow; 2] {
    let app = match &updates.app {
        Some(version) => UpdateRow { key: "update-endeavor", name: "Endeavor", state: format!("Version {version} is ready. It installs when Endeavor restarts."), button: Some(("Restart", Action::Restart, true)), attention: true },
        None => UpdateRow { key: "update-endeavor", name: "Endeavor", state: "Up to date".into(), button: Some(("Check now", Action::CheckNow, false)), attention: false },
    };
    let primary = updates.app.is_none();
    let (state, button, attention) = match &updates.adapter {
        Adapter::Current => ("Up to date".to_owned(), None, false),
        Adapter::Installing(version) => (format!("Installing version {version}…"), None, false),
        Adapter::Available(version) => (format!("Version {version} is available"), Some(("Update", Action::UpdateAdapter, primary)), true),
    };
    [app, UpdateRow { key: "update-adapter", name: "Claude Code adapter", state, button, attention }]
}

/// The About and Licences windows, so each opens once and comes forward after.
#[derive(Default)]
struct Windows {
    about: Option<AnyWindowHandle>,
    licences: Option<AnyWindowHandle>,
}

impl Global for Windows {}

/// Bring `handle` forward if it's still open.
fn raise(handle: Option<AnyWindowHandle>, cx: &mut App) -> bool {
    let Some(h) = handle.filter(|h| cx.windows().iter().any(|w| w.window_id() == h.window_id())) else { return false };
    // Chosen from this window's own menu, the window is busy until the action returns.
    cx.defer(move |cx| drop(h.update(cx, |_, window, _| window.activate_window())));
    true
}

fn small_window(title: &'static str, size: Size<Pixels>, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: Some(TitlebarOptions { title: Some(title.into()), appears_transparent: true, traffic_light_position: Some(point(px(14.), px(12.))) }),
        is_resizable: false,
        is_minimizable: true,
        ..Default::default()
    }
}

pub fn open_about(workspace: WeakEntity<Workspace>, cx: &mut App) {
    let open = cx.default_global::<Windows>().about;
    if raise(open, cx) {
        return;
    }
    let handle = cx.open_window(small_window("About Endeavor", size(px(360.), px(420.)), cx), |_, cx| {
        cx.new(|cx| {
            if let Some(ws) = workspace.upgrade() {
                cx.observe(&ws, |_, _, cx| cx.notify()).detach();
            }
            About { workspace, icon: Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec())) }
        })
    });
    if let Ok(handle) = handle {
        cx.global_mut::<Windows>().about = Some(handle.into());
    }
}

pub fn open_licences(cx: &mut App) {
    let open = cx.default_global::<Windows>().licences;
    if raise(open, cx) {
        return;
    }
    let handle = cx.open_window(small_window("Licences", size(px(520.), px(600.)), cx), |_, cx| cx.new(|_| Licences { shown: None }));
    if let Ok(handle) = handle {
        cx.global_mut::<Windows>().licences = Some(handle.into());
    }
}

struct About {
    workspace: WeakEntity<Workspace>,
    icon: Arc<Image>,
}

impl About {
    fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            // The notices are read afresh on every render.
            Action::CheckNow => cx.notify(),
            Action::Restart => cx.restart(),
            Action::UpdateAdapter => drop(self.workspace.update(cx, |ws, cx| ws.update_adapter(cx))),
        }
    }

    fn notice_row(&self, i: usize, notice: Notice, cx: &mut Context<Self>) -> Div {
        let marker = if notice.ok {
            div().text_color(theme::diff_add()).child("✓")
        } else {
            div().size(px(6.)).flex_shrink_0().rounded_full().bg(theme::accent())
        };
        let button = notice.button.map(|(label, action, primary)| {
            let b = div()
                .id(("notice", i))
                .role(Role::Button)
                .flex_shrink_0()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.act(action, cx)));
            if action == Action::CheckNow {
                b.text_color(theme::text_muted()).hover(|s| s.text_color(theme::text_secondary())).child(label)
            } else {
                b.h(px(24.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .font_weight(FontWeight::MEDIUM)
                    .when(primary, |b| b.bg(theme::accent()).text_color(gpui::white()))
                    .when(!primary, |b| b.bg(theme::bg_raised()).text_color(theme::text_primary()))
                    .child(label)
            }
        });
        div()
            .min_h(px(24.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(marker)
            .child(div().flex_1().min_w_0().text_color(theme::text_secondary()).child(notice.text))
            .children(button)
    }
}

fn link(id: &'static str, label: &'static str) -> Stateful<Div> {
    div().id(id).role(Role::Link).cursor_pointer().text_color(theme::accent_text()).hover(|s| s.underline()).child(label)
}

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let updates = self.workspace.upgrade().map(|ws| ws.read(cx).updates()).unwrap_or(Updates { app: None, adapter: Adapter::Current });
        let rows: Vec<Div> = notices(&updates).into_iter().enumerate().map(|(i, n)| self.notice_row(i, n, cx)).collect();
        let body = div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(40.))
            .px(px(32.))
            .text_center()
            .child(img(self.icon.clone()).size(px(112.)))
            .child(div().mt(px(12.)).text_size(theme::size_title()).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).child("Endeavor"))
            .child(
                div()
                    .mt(px(2.))
                    .flex()
                    .items_baseline()
                    .gap(px(4.))
                    .text_size(theme::size_meta())
                    .text_color(theme::text_muted())
                    .child(format!("Version {VERSION}"))
                    .child(div().font_family(theme::MONO).text_size(theme::size_meta_small()).child(format!("(build {BUILD})"))),
            )
            .child(div().mt(px(14.)).text_color(theme::text_secondary()).child("Build our future."))
            .child(
                div()
                    .mt(px(14.))
                    .text_size(theme::size_meta_small())
                    .line_height(px(17.))
                    .text_color(theme::text_faint())
                    .child("Notebooks by Pluto.jl. Runs on Julia.")
                    .child("Works with Claude, by Anthropic."),
            )
            .child(
                div()
                    .mt(px(10.))
                    .flex()
                    .gap(px(14.))
                    .text_size(theme::size_meta())
                    .child(link("website", "Website").on_click(|_, _, cx| cx.open_url(WEBSITE)))
                    .child(link("licences", "Licences").on_click(|_, _, cx| open_licences(cx))),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_card())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .text_size(theme::size_body())
            .child(body)
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme::border())
                    .bg(rgb(0x18181B))
                    .px(px(16.))
                    .py(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .text_size(theme::size_meta())
                    .children(rows),
            )
    }
}

/// A part Endeavor ships or installs, and its licence: the full text when the
/// repo has it, else a link to it.
struct Part {
    name: &'static str,
    note: &'static str,
    licence: &'static str,
    text: Option<&'static str>,
    url: Option<&'static str>,
}

const PARTS: &[Part] = &[
    Part { name: "Endeavor", note: "This app.", licence: "MIT", text: Some(include_str!("../LICENSE")), url: None },
    Part {
        name: "Pluto.jl",
        note: "The notebook. Installed on first launch.",
        licence: "MIT",
        text: None,
        url: Some("https://github.com/JuliaPluto/Pluto.jl/blob/main/LICENSE"),
    },
    Part {
        name: "Julia",
        note: "The language the notebooks run. Installed on first launch.",
        licence: "MIT",
        text: None,
        url: Some("https://github.com/JuliaLang/julia/blob/master/LICENSE.md"),
    },
    Part {
        name: "Endeavor Sans",
        note: "The interface font: Schibsted Grotesk, with Greek from Inter and Arimo and math symbols from Noto Sans Math.",
        licence: "SIL Open Font License 1.1",
        text: Some(include_str!("../fonts/OFL.txt")),
        url: None,
    },
    Part {
        name: "JuliaMono",
        note: "The code font, unmodified.",
        licence: "SIL Open Font License 1.1",
        text: Some(include_str!("../fonts/JuliaMono-LICENSE.txt")),
        url: None,
    },
    Part {
        name: "Node.js",
        note: "Runs the Claude Code adapter. Installed on first launch.",
        licence: "MIT, with the licences of its bundled parts",
        text: None,
        url: Some("https://github.com/nodejs/node/blob/main/LICENSE"),
    },
    Part {
        name: "Claude Code adapter",
        note: "@agentclientprotocol/claude-agent-acp, which connects the app to Claude Code. Installed on first launch with about 110 npm packages, mostly MIT.",
        licence: "Apache 2.0",
        text: None,
        url: Some("https://www.npmjs.com/package/@agentclientprotocol/claude-agent-acp"),
    },
    Part {
        name: "Claude Agent SDK",
        note: "Claude Code itself, installed with the adapter.",
        licence: "Anthropic's terms",
        text: None,
        url: Some("https://www.npmjs.com/package/@anthropic-ai/claude-agent-sdk"),
    },
    Part {
        name: "Rust crates",
        note: "About 550 crates built into the app, among them GPUI, gpui-component and wry.",
        licence: "Mostly MIT or Apache 2.0; a few BSD, Zlib, ISC, MPL 2.0 and Unicode 3.0",
        text: None,
        url: None,
    },
];

struct Licences {
    /// The part whose full licence text is showing.
    shown: Option<usize>,
}

impl Render for Licences {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let parts = PARTS.iter().enumerate().map(|(i, part)| {
            let open = self.shown == Some(i);
            let action = match (part.text, part.url) {
                (Some(_), _) => Some(
                    link("show", if open { "Hide licence" } else { "Show licence" })
                        .id(("show", i))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.shown = if this.shown == Some(i) { None } else { Some(i) };
                            cx.notify();
                        })),
                ),
                (None, Some(url)) => Some(link("open", "Licence").id(("open", i)).on_click(move |_, _, cx| cx.open_url(url))),
                (None, None) => None,
            };
            div()
                .py(px(10.))
                .border_b_1()
                .border_color(theme::divider())
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .child(div().font_weight(FontWeight::MEDIUM).child(part.name))
                        .child(div().flex_1().text_size(theme::size_meta()).text_color(theme::text_muted()).child(part.licence))
                        .children(action.map(|a| a.text_size(theme::size_meta()))),
                )
                .child(div().text_size(theme::size_meta()).text_color(theme::text_faint()).child(part.note))
                .when_some(part.text.filter(|_| open), |d, text| {
                    d.child(
                        div()
                            .mt(px(6.))
                            .p(px(10.))
                            .rounded(px(6.))
                            .bg(theme::bg_page())
                            .font_family(theme::MONO)
                            .text_size(theme::size_meta_small())
                            .text_color(theme::text_secondary())
                            .child(text),
                    )
                })
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_card())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .text_size(theme::size_body())
            .child(
                div()
                    .h(px(36.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Licences"),
            )
            .child(div().id("parts").flex_1().min_h_0().overflow_y_scroll().px(px(20.)).pb(px(16.)).children(parts))
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Adapter, BUILD, Notice, Updates, notices, parts};

    #[test]
    fn settings_lists_each_part_with_its_own_button() {
        let current = parts(&Updates { app: None, adapter: Adapter::Current });
        let rows: Vec<_> = current.iter().map(|p| (p.name, p.state.as_str(), p.button.map(|b| b.0))).collect();
        assert_eq!(rows, [("Endeavor", "Up to date", Some("Check now")), ("Claude Code adapter", "Up to date", None)]);
        let adapter = parts(&Updates { app: None, adapter: Adapter::Available("0.4".into()) });
        assert_eq!((adapter[1].state.as_str(), adapter[1].button, adapter[1].attention), ("Version 0.4 is available", Some(("Update", Action::UpdateAdapter, true)), true));
        let both = parts(&Updates { app: Some("0.2.0".into()), adapter: Adapter::Available("0.4".into()) });
        assert_eq!((both[0].button, both[1].button), (Some(("Restart", Action::Restart, true)), Some(("Update", Action::UpdateAdapter, false))));
    }

    #[test]
    fn up_to_date_offers_check_now() {
        let rows = notices(&Updates { app: None, adapter: Adapter::Current });
        assert_eq!(
            rows,
            vec![Notice { text: "Everything is up to date.".into(), button: Some(("Check now", Action::CheckNow, false)), ok: true }]
        );
    }

    #[test]
    fn adapter_update_gets_the_primary_button_alone() {
        let rows = notices(&Updates { app: None, adapter: Adapter::Available("0.82.0".into()) });
        assert_eq!(
            rows,
            vec![Notice {
                text: "Claude Code adapter 0.82.0 is available.".into(),
                button: Some(("Update", Action::UpdateAdapter, true)),
                ok: false
            }]
        );
    }

    #[test]
    fn app_update_comes_first_and_takes_the_primary_button() {
        let rows = notices(&Updates { app: Some("0.2.0".into()), adapter: Adapter::Available("0.82.0".into()) });
        let buttons: Vec<_> = rows.iter().map(|r| (r.text.as_str(), r.button)).collect();
        assert_eq!(
            buttons,
            vec![
                ("Endeavor 0.2.0 is ready to install.", Some(("Restart", Action::Restart, true))),
                ("Claude Code adapter 0.82.0 is available.", Some(("Update", Action::UpdateAdapter, false))),
            ]
        );
    }

    #[test]
    fn installing_adapter_has_no_button() {
        let rows = notices(&Updates { app: None, adapter: Adapter::Installing("0.82.0".into()) });
        assert_eq!(rows, vec![Notice { text: "Installing Claude Code adapter 0.82.0…".into(), button: None, ok: false }]);
    }

    #[test]
    fn build_is_a_short_hash() {
        assert!(BUILD == "unknown" || (BUILD.len() >= 7 && BUILD.chars().all(|c| c.is_ascii_hexdigit())), "{BUILD}");
    }
}
