//! First-launch setup screen: the logo, setup steps with a progress bar, and
//! Retry when a step fails. Shown until setup has finished once; later launches
//! report setup work (e.g. a new adapter version) in the status line instead.

use std::path::PathBuf;

use gpui::prelude::FluentBuilder as _;
use gpui::*;

use crate::Workspace;

/// Setup steps, in the order they run.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Step {
    #[default]
    Julia,
    Packages,
    Agent,
    Claude,
}

impl Step {
    const ALL: [Step; 4] = [Step::Julia, Step::Packages, Step::Agent, Step::Claude];

    fn label(self) -> &'static str {
        match self {
            Step::Julia => "Julia",
            Step::Packages => "Pluto and its packages",
            Step::Agent => "Claude agent",
            Step::Claude => "Connecting to Claude",
        }
    }
}

/// A setup step is under way: what it's doing, and how far along if known.
#[derive(Debug)]
pub struct Progress {
    pub step: Step,
    pub detail: String,
    pub fraction: Option<f32>,
    /// A raw log line (e.g. Julia precompiling): setup screen only, not the status line.
    pub log: bool,
}

impl Progress {
    pub fn new(step: Step, detail: impl Into<String>) -> Self {
        Self { step, detail: detail.into(), fraction: None, log: false }
    }
}

#[derive(Default)]
pub struct Setup {
    step: Step,
    detail: String,
    fraction: Option<f32>,
    pub error: Option<String>,
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
            (self.step, self.detail, self.fraction) = (p.step, p.detail, p.fraction);
        }
    }

    fn overall(&self) -> f32 {
        let done = Step::ALL.iter().position(|s| *s == self.step).unwrap_or(0) as f32;
        (done + self.fraction.unwrap_or(0.)) / Step::ALL.len() as f32
    }
}

// ponytail: placeholder logo and inline colors until the logo and the style system land.
pub fn render(setup: &Setup, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    const BAR: f32 = 360.;
    let muted = rgb(0x8a8a8a);
    let steps = Step::ALL.map(|step| {
        let (mark, color) = match step.cmp(&setup.step) {
            std::cmp::Ordering::Less => ("✓", rgb(0x6fbf73)),
            std::cmp::Ordering::Equal if setup.error.is_some() => ("⚠", rgb(0xd16969)),
            std::cmp::Ordering::Equal => ("●", rgb(0xc8a040)),
            std::cmp::Ordering::Greater => ("○", rgb(0x5a5a5a)),
        };
        let active = step == setup.step;
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().w_4().text_color(color).child(mark))
                    .child(div().when(!active && step > setup.step, |d| d.text_color(muted)).child(step.label())),
            )
            .when(active && setup.error.is_none() && !setup.detail.is_empty(), |d| {
                d.child(div().pl_6().text_xs().text_color(muted).overflow_hidden().whitespace_nowrap().child(setup.detail.clone()))
            })
    });
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_4()
        .child(div().text_3xl().child("🚀"))
        .child(div().text_2xl().child("Endeavor"))
        .child(div().text_sm().text_color(muted).child("Setting up. The first launch takes a few minutes."))
        .child(
            div()
                .w(px(BAR))
                .h(px(6.))
                .rounded_full()
                .bg(rgb(0x333333))
                .child(div().h_full().rounded_full().bg(rgb(0x2f5d3a)).w(px(BAR * setup.overall()))),
        )
        .child(div().w(px(BAR)).flex().flex_col().gap_2().text_sm().children(steps))
        .children(setup.error.clone().map(|error| {
            div()
                .w(px(BAR))
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().text_color(rgb(0xd16969)).child(error))
                .child(
                    div()
                        .id("retry-setup")
                        .self_start()
                        .px_3()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(0x3a3a3c))
                        .child("Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.retry_setup(cx))),
                )
        }))
}

#[cfg(test)]
mod tests {
    use super::{Progress, Setup, Step};

    #[test]
    fn steps_only_move_forward_and_fill_the_bar() {
        let mut s = Setup::default();
        s.apply(Progress { fraction: Some(0.5), ..Progress::new(Step::Julia, "Downloading") });
        assert_eq!(s.overall(), 0.125);
        s.apply(Progress::new(Step::Agent, "Installing"));
        s.apply(Progress::new(Step::Packages, "late Julia log line"));
        assert_eq!((s.step, s.detail.as_str()), (Step::Agent, "Installing"));
        assert_eq!(s.overall(), 0.5);
    }
}
