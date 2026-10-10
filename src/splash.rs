//! First-launch setup screen: the turtle walks in and looks around while one
//! quiet line and a thin bar report setup; a failed step lists the steps and
//! offers Retry. Shown until setup has finished once; later launches report
//! setup work (e.g. a new adapter version) in the status line instead.

use std::path::PathBuf;
use std::time::Instant;

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::theme;
use crate::theme::FocusRing as _;
use crate::theme::TextButton as _;
use crate::turtle::{self, Gaze, Pose, ease, lerp};

/// Setup steps, in the order they run. Julia isn't one: the core installs and
/// starts it when a Julia notebook first needs it, and the runtime starts
/// without it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Step {
    Runtime,
    Agent,
    Claude,
}

impl Step {
    pub const ALL: [Step; 3] = [Step::Runtime, Step::Agent, Step::Claude];

    /// `agent` names the assistant picked ("Claude agent", "Connecting to Codex").
    pub fn label(self, agent: &str) -> String {
        match self {
            Step::Runtime => "Notebook runtime".into(),
            Step::Agent => format!("{agent} agent"),
            Step::Claude => format!("Connecting to {agent}"),
        }
    }

    fn doing(self, agent: &str) -> String {
        match self {
            Step::Runtime => "Starting the notebook runtime".into(),
            Step::Agent => format!("Setting up the {agent} agent"),
            Step::Claude => format!("Connecting to {agent}"),
        }
    }

    fn failed(self, agent: &str) -> String {
        match self {
            Step::Runtime => "Couldn't start the notebook runtime.".into(),
            Step::Agent => format!("Couldn't set up the {agent} agent."),
            Step::Claude => format!("Couldn't connect to {agent}."),
        }
    }
}

/// A setup step is under way: what it's doing, and how far along if known.
#[derive(Debug)]
pub struct Progress {
    pub step: Step,
    pub detail: String,
    pub fraction: Option<f32>,
    /// A raw log line: setup screen only, not the status line.
    pub log: bool,
}

impl Progress {
    pub fn new(step: Step, detail: impl Into<String>) -> Self {
        Self { step, detail: detail.into(), fraction: None, log: false }
    }
}

pub struct Setup {
    step: Step,
    fraction: Option<f32>,
    /// What went wrong, and when (the turtle tucks its head in from then).
    error: Option<(String, Instant)>,
    /// The intro plays from here.
    shown: Instant,
    /// The assistant picked on this screen; until then the choice shows.
    pub agent: Option<crate::agent::Agent>,
}

impl Default for Setup {
    fn default() -> Self {
        Self { step: Step::ALL[0], fraction: None, error: None, shown: Instant::now(), agent: None }
    }
}

fn marker() -> Option<PathBuf> {
    crate::install::app_dir().ok().map(|d| d.join("setup-complete"))
}

impl Setup {
    /// First launch: setup hasn't finished yet on this Mac.
    pub fn needed() -> bool {
        marker().is_some_and(|m| !m.exists())
    }

    pub fn finish() {
        if let Some(m) = marker() {
            let _ = std::fs::create_dir_all(m.parent().unwrap()).and_then(|_| std::fs::write(m, ""));
        }
    }

    /// Steps only move forward; a late message from an earlier step is ignored.
    pub fn apply(&mut self, p: Progress) {
        if p.step >= self.step {
            (self.step, self.fraction) = (p.step, p.fraction);
        }
    }

    pub fn fail(&mut self, error: String) {
        self.error = Some((error, Instant::now()));
    }

    /// Back to `step` if setup is past it: a new pick of assistant starts its steps over.
    pub fn back_to(&mut self, step: Step) {
        if self.step > step {
            self.step = step;
            self.fraction = None;
        }
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn failed(&self) -> bool {
        self.error.is_some()
    }

    /// The assistant's name for the steps: the one picked, else Claude.
    pub fn agent_name(&self) -> &'static str {
        self.agent.unwrap_or_default().name()
    }

    /// The step under way, or the one that failed.
    pub fn step(&self) -> Step {
        self.step
    }

    fn overall(&self) -> f32 {
        let done = Step::ALL.iter().position(|s| *s == self.step).unwrap_or(0) as f32;
        (done + self.fraction.unwrap_or(0.)) / Step::ALL.len() as f32
    }
}

/// Shell radius: the shell is 120 px wide.
const R: f32 = 60.;
/// Tall enough for the highest star.
const SCENE_HEIGHT: f32 = 3.7 * R + 2.;
const WALK: f32 = 1.6;
const GAZE_START: f32 = 3.7;
const GAZE_MOVE: f32 = 0.45;
/// With reduced motion: everything is in and the turtle looks up at the stars.
const STILL: f32 = 6.;

const STARS_GAZE: Gaze = Gaze { eye_x: 1., look: 1., dx: 0.06, dy: -0.28 };
const HIGH: Gaze = Gaze { eye_x: 0.55, look: 1.25, dx: 0.02, dy: -0.34 };
const YOU: Gaze = Gaze { eye_x: 0., look: 0.1, dx: 0., dy: -0.04 };
const BACK: Gaze = Gaze { eye_x: -1.3, look: 0.35, dx: -0.07, dy: -0.08 };
const DOWN: Gaze = Gaze { eye_x: 1., look: -0.7, dx: 0.05, dy: 0.07 };
const AHEAD: Gaze = Gaze::AHEAD;
/// (gaze, seconds held), hand-ordered so it doesn't feel like a metronome.
const GAZES: [(Gaze, f32); 14] = [
    (STARS_GAZE, 4.),
    (AHEAD, 1.6),
    (YOU, 2.2),
    (AHEAD, 1.2),
    (HIGH, 3.2),
    (STARS_GAZE, 1.8),
    (AHEAD, 1.4),
    (BACK, 1.8),
    (AHEAD, 2.),
    (DOWN, 1.4),
    (AHEAD, 1.8),
    (YOU, 1.6),
    (STARS_GAZE, 3.6),
    (AHEAD, 2.4),
];
/// (x, y) from where the turtle stops, in shell radii, and the radius in px.
const STARS: [(f32, f32, f32); 7] = [(1.5, -2.7, 3.7), (2.4, -2.1, 2.8), (0.8, -3.2, 3.), (3., -3., 4.6), (2., -3.6, 2.5), (-0.3, -2.9, 2.3), (3.6, -2.3, 2.5)];

/// After the intro the head glances through `GAZES` on a loop, easing between them.
fn gaze_at(t: f32) -> Gaze {
    if t < GAZE_START {
        return AHEAD;
    }
    let total: f32 = GAZES.iter().map(|(_, hold)| GAZE_MOVE + hold).sum();
    let mut u = (t - GAZE_START) % total;
    let mut from = GAZES[GAZES.len() - 1].0;
    for (to, hold) in GAZES {
        if u < GAZE_MOVE {
            return from.lerp(to, ease(u / GAZE_MOVE));
        }
        u -= GAZE_MOVE;
        if u < hold {
            return to;
        }
        u -= hold;
        from = to;
    }
    AHEAD
}

/// The turtle walks in from the left edge and stops just left of centre; the stars
/// come out one by one and twinkle. `t` is seconds since the splash showed; `tuck`
/// how far the head is in the shell.
fn scene(t: f32, tuck: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            if !cx.reduce_motion() {
                window.request_animation_frame();
            }
            let ground_y = f32::from(bounds.bottom()) - 1.;
            let left = f32::from(bounds.left());
            let end_x = f32::from(bounds.center().x) - 0.2 * R;
            for (i, (dx, dy, r)) in STARS.into_iter().enumerate() {
                let on = ease((t - 3. - i as f32 * 0.18) / 0.4);
                if on > 0. {
                    let a = on * (0.7 + 0.3 * (t * 2.2 + i as f32 * 1.7).sin());
                    let at = point(px(end_x + dx * R), px(ground_y + dy * R));
                    turtle::disc(window, at, (r, r), Rgba { a, ..theme::star() });
                }
            }
            let walked = (t / WALK).clamp(0., 1.);
            let x = lerp(left - 2. * R, end_x, 1. - (1. - walked).powi(2));
            let stepping = 1. - ease(walked);
            let phase = t * std::f32::consts::TAU / 0.8;
            let pose = Pose {
                bob: if t < WALK { -0.035 * phase.sin().abs() * stepping } else { 0. },
                blink: turtle::blink_at(t, 3.3, 0.9),
                breathe: 0.018 * (t * std::f32::consts::TAU / 3.4).sin(),
                gaze: gaze_at(t),
                ..Pose::default()
            }
            .walk(phase, 0.1 * stepping, 0.14 * stepping)
            .tuck(tuck);
            turtle::paint(window, point(px(x), px(ground_y)), R, &pose);
        },
    )
    .w_full()
    .h(px(SCENE_HEIGHT))
}

/// What shows under the turtle and the name.
pub enum Below {
    /// Setup's progress line and bar, or its failure.
    Progress,
    /// A panel under a line of its own and, optionally, the bar filled this far
    /// (sign-in); `tucked` is when the turtle drew its head in.
    Panel { line: &'static str, bar: Option<f32>, panel: AnyElement, tucked: Option<Instant> },
    /// A card in place of the progress (no network).
    Card(AnyElement),
}

/// `retry` restarts the failed step; `change`, when the assistant's own step
/// failed, goes back to the choice of assistant.
pub fn render(
    setup: &Setup,
    below: Below,
    retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    change: Option<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    focus: &dyn Fn(&'static str) -> FocusHandle,
    cx: &App,
) -> Div {
    const BAR: f32 = 240.;
    const WIDE: f32 = 360.;
    const PANEL: f32 = 376.;
    let muted = theme::text_muted();
    let still = cx.reduce_motion();
    let t = if still { STILL } else { setup.shown.elapsed().as_secs_f32() };
    let tucked = match &below {
        Below::Panel { tucked, .. } => *tucked,
        Below::Progress => setup.error.as_ref().map(|(_, at)| *at),
        Below::Card(_) => None,
    };
    let tuck = tucked.map_or(0., |at| if still { 1. } else { ease(at.elapsed().as_secs_f32() / 0.4) });
    let (name_in, tagline_in) = (ease((t - 1.8) / 0.6), ease((t - 2.3) / 0.6));
    let n = Step::ALL.iter().position(|s| *s == setup.step).unwrap_or(0) + 1;
    let progress = div()
        .mt(px(36.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.))
        .opacity(tagline_in)
        .child(div().text_size(theme::size_meta()).text_color(muted).child(format!("{} · {n} of {}", setup.step.doing(setup.agent_name()), Step::ALL.len())))
        .child(
            div()
                .w(px(BAR))
                .h(px(2.))
                .rounded_full()
                .bg(theme::border())
                .child(div().h_full().rounded_full().bg(theme::accent()).w(px(BAR * setup.overall()))),
        );
    let failure = setup.error.as_ref().map(|(error, _)| {
        let steps = Step::ALL.map(|step| {
            let (mark, color) = match step.cmp(&setup.step) {
                std::cmp::Ordering::Less => ("✓", theme::diff_add()),
                std::cmp::Ordering::Equal => ("⚠", theme::danger()),
                std::cmp::Ordering::Greater => ("○", theme::text_section()),
            };
            div()
                .flex()
                .gap_2()
                .child(div().w_4().text_color(color).child(mark))
                .child(div().when(step > setup.step, |d| d.text_color(muted)).child(step.label(setup.agent_name())))
        });
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .track_focus(&focus(id))
                .tab_stop(true)
                .focus_ring()
                .px_3()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .bg(theme::bg_raised())
                .text_color(theme::text_primary())
                .button_text(label)
        };
        div()
            .mt(px(36.))
            .w(px(WIDE))
            .flex()
            .flex_col()
            .gap_3()
            .child(div().flex().flex_col().gap_1().text_size(theme::size_meta()).children(steps))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(setup.step.failed(setup.agent_name()))
                    .child(div().text_size(theme::size_meta()).text_color(muted).child(error.clone()))
                    .child(div().text_size(theme::size_meta()).text_color(muted).child("Check your connection and retry, or see the logs.")),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(button("retry-setup", "Retry").bg(theme::accent()).text_color(gpui::white()).on_click(retry))
                    .child(button("setup-logs", "Show logs").on_click(|_, _, _| crate::logs::reveal()))
                    .children(change.map(|change| button("setup-change-assistant", "Choose another assistant").on_click(change))),
            )
    });
    // The night sky is a band over the steps: dark in both appearances, while
    // the steps sit on the page. The two parts grow alike, so the whole stays
    // centred; in dark they are one colour.
    let sky = div()
        .w_full()
        .flex_grow(1.)
        .flex()
        .flex_col()
        .items_center()
        .justify_end()
        .bg(theme::sky())
        .when(theme::is_light(), |d| d.pb(px(28.)))
        .child(scene(t, tuck))
        .child(
            div()
                .mt(px(20.))
                .relative()
                .top(px((1. - name_in) * 10.))
                .opacity(name_in)
                .text_size(theme::size_title())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::sky_text())
                .child("Endeavor"),
        )
        .child(div().mt(px(4.)).relative().top(px((1. - tagline_in) * 8.)).opacity(tagline_in).text_color(theme::sky_muted()).child("Build our future"));
    div().size_full().flex().flex_col().child(sky).child(
        // Room for the failure message and sign-in, so the turtle stays put when they appear.
        div().w_full().flex_grow(1.).flex().flex_col().items_center().child(div().min_h(px(300.)).flex().flex_col().items_center().map(|d| match below {
            Below::Panel { line, bar, panel, .. } => d.child(
                div()
                    .mt(px(26.))
                    .w(px(PANEL))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .opacity(tagline_in)
                    .child(
                        div()
                            .mb(px(4.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(10.))
                            .child(div().text_size(theme::size_meta()).text_color(muted).child(format!("{line} · {n} of {}", Step::ALL.len())))
                            .children(bar.map(|f| {
                                div().w(px(BAR)).h(px(2.)).rounded_full().bg(theme::border()).child(div().h_full().rounded_full().bg(theme::accent()).w(px(BAR * f)))
                            })),
                    )
                    .child(panel),
            ),
            Below::Card(card) => d.child(div().mt(px(26.)).w(px(PANEL)).opacity(tagline_in).child(card)),
            Below::Progress => match failure {
                Some(failure) => d.child(failure),
                None => d.child(progress),
            },
        })),
    )
}

/// Debug builds only: `ENDEAVOR_SPLASH_PREVIEW=1` opens the setup screen with made-up
/// progress, installing nothing. Add `fail` to stop partway, `still` for
/// the reduced-motion frame (e.g. `fail,still`).
#[cfg(debug_assertions)]
pub mod preview {
    use std::time::Duration;

    use gpui::*;

    use super::{Progress, Setup, Step};
    use crate::theme;

    pub struct Preview {
        setup: Setup,
        ticks: usize,
        fail: bool,
    }

    pub fn open(cx: &mut App) -> Option<Entity<Preview>> {
        let mode = std::env::var("ENDEAVOR_SPLASH_PREVIEW").ok()?;
        if mode.contains("still") {
            cx.set_reduce_motion(true);
        }
        Some(cx.new(|cx| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(600)).await;
                    if this.update(cx, |this: &mut Preview, cx| this.tick(cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
            Preview { setup: Setup::default(), ticks: 0, fail: mode.contains("fail") }
        }))
    }

    impl Preview {
        fn tick(&mut self, cx: &mut Context<Self>) {
            if self.setup.error.is_some() {
                return;
            }
            self.ticks = (self.ticks + 1).min(39);
            let step = Step::ALL[self.ticks * Step::ALL.len() / 40];
            self.setup.apply(Progress { fraction: Some((self.ticks % 10) as f32 / 10.), ..Progress::new(step, "") });
            if self.fail && self.ticks == 15 {
                self.fail = false;
                self.setup.fail("Installing the agent failed: connection reset by peer (preview)".into());
            }
            cx.notify();
        }
    }

    impl Render for Preview {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let retry = cx.listener(|this, _, _, cx| {
                this.setup.clear_error();
                cx.notify();
            });
            div().size_full().bg(theme::bg_page()).text_color(theme::text_primary()).text_size(theme::size_body()).child(super::render(&self.setup, super::Below::Progress, retry, None::<fn(&ClickEvent, &mut Window, &mut App)>, &|_| cx.focus_handle(), cx))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AHEAD, GAZE_MOVE, GAZE_START, Progress, STARS_GAZE, STILL, Setup, Step, gaze_at};

    #[test]
    fn steps_only_move_forward_and_fill_the_bar() {
        let mut s = Setup::default();
        assert_eq!(s.step, Step::ALL[0]);
        s.apply(Progress { fraction: Some(0.5), ..Progress::new(Step::Agent, "Installing") });
        let before = Step::ALL.iter().position(|s| *s == Step::Agent).unwrap() as f32;
        assert_eq!(s.overall(), (before + 0.5) / Step::ALL.len() as f32);
        s.apply(Progress::new(Step::Claude, "Connecting"));
        s.apply(Progress::new(Step::Runtime, "a late line"));
        assert_eq!(s.step, Step::Claude);
        assert_eq!(s.overall(), (Step::ALL.len() - 1) as f32 / Step::ALL.len() as f32);
    }

    #[test]
    fn the_head_looks_ahead_then_up_at_the_stars() {
        assert_eq!(gaze_at(1.), AHEAD);
        assert_eq!(gaze_at(GAZE_START + GAZE_MOVE + 0.1), STARS_GAZE);
        assert_eq!(gaze_at(STILL), STARS_GAZE);
    }
}
