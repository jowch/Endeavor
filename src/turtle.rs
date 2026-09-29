//! The Endeavor turtle, in profile facing right: a half-disc shell, two half-disc
//! feet and a round head with one eye. Sizes are fractions of the shell radius.

use std::f32::consts::PI;

use gpui::*;

use crate::theme;

const HEAD_R: f32 = 0.36;
const HEAD_X: f32 = 1.02;
const HEAD_Y: f32 = -0.14;
const FOOT_R: f32 = 0.21;
const FEET_X: [f32; 2] = [-0.6, 0.4];

/// Where the eye points: `eye_x` 1 looks ahead, 0 at the viewer, -1 back;
/// `look` raises it; `dx`/`dy` move the head.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gaze {
    pub eye_x: f32,
    pub look: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Gaze {
    pub const AHEAD: Gaze = Gaze { eye_x: 1., look: 0., dx: 0., dy: 0. };

    pub fn lerp(self, to: Gaze, k: f32) -> Gaze {
        Gaze { eye_x: lerp(self.eye_x, to.eye_x, k), look: lerp(self.look, to.look, k), dx: lerp(self.dx, to.dx, k), dy: lerp(self.dy, to.dy, k) }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Pose {
    /// The shell's height grows by this fraction.
    pub breathe: f32,
    /// The body's lift off the feet (negative is up).
    pub bob: f32,
    /// 1 open, near 0 shut.
    pub blink: f32,
    pub gaze: Gaze,
    pub head_scale: f32,
    pub head_behind: bool,
    /// Each foot's (dx, dy) from rest, back foot first.
    pub feet: [(f32, f32); 2],
}

impl Default for Pose {
    fn default() -> Self {
        Pose { breathe: 0., bob: 0., blink: 1., gaze: Gaze::AHEAD, head_scale: 1., head_behind: false, feet: [(0., 0.); 2] }
    }
}

impl Pose {
    /// Feet a walk cycle's `phase` along: each lifts in turn and swings forward while up.
    pub fn walk(mut self, phase: f32, stride: f32, lift: f32) -> Self {
        self.feet = [0., PI].map(|o| (-(phase + o).cos() * stride, -(phase + o).sin().max(0.) * lift));
        self
    }

    /// The head drawn `k` of the way into the shell (1: hidden but for a sliver).
    pub fn tuck(mut self, k: f32) -> Self {
        if k > 0. {
            self.head_behind = true;
            self.gaze = self.gaze.lerp(Gaze { eye_x: 1., look: 0., dx: -0.62, dy: -0.3 }, k);
            self.head_scale = 1. - 0.1 * k;
            if k > 0.3 {
                self.blink = 0.12;
            }
        }
        self
    }
}

/// The empty notebook pages' peek, `t` seconds in: one cycle every 4.2 s, mostly
/// resting with the head out; then it tucks into the shell, peeks out looking
/// up, and pops out again.
pub fn peek(t: f32) -> Pose {
    const PERIOD: f32 = 4.2;
    let u = t.rem_euclid(PERIOD);
    let up = Gaze { eye_x: 0.55, look: 1., ..Gaze::AHEAD };
    let span = |from: f32, to: f32| ease((u - from) / (to - from));
    let (tuck, gaze) = match u {
        u if u < 2.0 => (0., Gaze::AHEAD),
        u if u < 2.35 => (span(2.0, 2.35), Gaze::AHEAD),
        u if u < 2.8 => (1., Gaze::AHEAD),
        u if u < 3.3 => (lerp(1., 0.45, span(2.8, 3.3)), up),
        u if u < 3.8 => (0.45, up),
        u if u < 4.0 => (lerp(0.45, 0., span(3.8, 4.0)), up.lerp(Gaze::AHEAD, span(3.8, 4.0))),
        _ => (0., Gaze::AHEAD),
    };
    let mut pose = Pose { blink: if tuck == 0. { blink_at(t, 3.3, 0.9) } else { 1. }, gaze, ..Pose::default() }.tuck(tuck);
    if tuck > 0. && tuck < 0.6 {
        // Peeking: the eye stays open under the shell's rim.
        pose.blink = 1.;
    }
    pose
}

pub fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

/// Smoothstep of `k` clamped to 0..1.
pub fn ease(k: f32) -> f32 {
    let k = k.clamp(0., 1.);
    k * k * (3. - 2. * k)
}

/// Eye openness at `t` seconds for a blink every `period` seconds.
pub fn blink_at(t: f32, period: f32, offset: f32) -> f32 {
    const SHUT: f32 = 0.16;
    let u = (t + offset).rem_euclid(period);
    if u < SHUT { (1. - 2. * u / SHUT).abs() } else { 1. }
}

/// A filled ellipse `rx` by `ry`, flattened to a pill when they differ.
pub fn disc(window: &mut Window, center: Point<Pixels>, (rx, ry): (f32, f32), color: impl Into<Background>) {
    window.paint_quad(fill(Bounds::centered_at(center, size(px(2. * rx), px(2. * ry))), color).corner_radii(px(rx.min(ry))));
}

/// A filled half ellipse from angle `from` to `from + PI` (y down, so PI is the top
/// half), closed with a straight edge.
fn half_disc(window: &mut Window, center: Point<Pixels>, (rx, ry): (f32, f32), from: f32, color: impl Into<Background>) {
    const STEPS: usize = 48;
    let to = from + PI;
    let points: Vec<_> = (0..=STEPS)
        .map(|i| {
            let a = from + (to - from) * i as f32 / STEPS as f32;
            point(center.x + px(rx * a.cos()), center.y + px(ry * a.sin()))
        })
        .collect();
    let mut path = PathBuilder::fill();
    path.add_polygon(&points, true);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// The turtle standing with its feet on `ground`, shell radius `r` px.
pub fn paint(window: &mut Window, ground: Point<Pixels>, r: f32, p: &Pose) {
    let at = |x: f32, y: f32| point(ground.x + px(x * r), ground.y + px(y * r));
    for (fx, (dx, dy)) in FEET_X.into_iter().zip(p.feet) {
        half_disc(window, at(fx + dx, -FOOT_R + dy), (FOOT_R * r, FOOT_R * r), 0., theme::turtle_skin());
    }
    let base = -FOOT_R + p.bob;
    let hr = HEAD_R * r * p.head_scale;
    let head = at(HEAD_X + p.gaze.dx, base + HEAD_Y + p.gaze.dy);
    let paint_head = |window: &mut Window| {
        disc(window, head, (hr, hr), theme::turtle_skin());
        if p.head_scale > 0.3 {
            let eye = point(head.x + px(hr * 0.36 * p.gaze.eye_x), head.y - px(hr * (0.16 + 0.22 * p.gaze.look)));
            let er = hr * 0.13;
            disc(window, eye, (er, er * p.blink.max(0.12)), theme::turtle_eye());
        }
    };
    if p.head_behind {
        paint_head(window);
    }
    half_disc(window, at(0., base), (r, r * (1. + p.breathe)), PI, theme::accent());
    if !p.head_behind {
        paint_head(window);
    }
}

#[cfg(test)]
mod tests {
    use super::peek;

    #[test]
    fn the_peek_rests_tucks_peeks_up_and_pops_out() {
        let out = peek(1.0);
        assert!(!out.head_behind && out.gaze.look == 0., "resting, head out");
        assert!(peek(2.5).head_behind && peek(2.5).blink < 0.5, "tucked in, eye shut");
        let peeking = peek(3.5);
        assert!(peeking.head_behind && peeking.gaze.look > 0.4 && peeking.blink == 1., "peeking out, looking up");
        assert!(!peek(4.1).head_behind && !peek(4.2 + 1.0).head_behind, "out again, and the cycle repeats");
    }
}
