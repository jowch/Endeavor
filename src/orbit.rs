//! The orbiting "working" mark: a dot circling a disc on a tilted ring.

use std::time::Duration;

use gpui::*;

use crate::theme;

/// The working mark: a dot circling a small disc on a tilted ring, one lap a
/// second. With reduce motion it stands still at the front of the ring.
pub fn orbit(id: ElementId, size: f32, cx: &App) -> AnyElement {
    orbit_with(id, size, theme::accent(), cx)
}

/// The orbit with a dot of another colour (grey while waiting for the network).
pub fn orbit_with(id: ElementId, size: f32, dot: Rgba, cx: &App) -> AnyElement {
    if cx.reduce_motion() {
        return orbit_at(0.25, size, dot).into_any_element();
    }
    div()
        .size(px(size))
        .with_animation(id, Animation::new(Duration::from_secs(1)).repeat(), move |d, t| d.child(orbit_at(t, size, dot)))
        .into_any_element()
}

/// The orbit `t` of the way round a lap (0.25: in front of the disc). The dot
/// passes behind the disc on the ring's far half, so the ring's back half, a
/// dot there, the disc, the front half and a dot there are drawn in that order.
fn orbit_at(t: f32, size_px: f32, dot_color: Rgba) -> impl IntoElement {
    use std::f32::consts::{PI, TAU};
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let (rx, tilt) = (size_px * 0.36, -25f32.to_radians());
            let ry = rx * 0.36;
            let on_ring = |a: f32| {
                let (x, y) = (rx * a.cos(), ry * a.sin());
                point(center.x + px(x * tilt.cos() - y * tilt.sin()), center.y + px(x * tilt.sin() + y * tilt.cos()))
            };
            let half = |from: f32, window: &mut Window| {
                const STEPS: usize = 24;
                let mut path = PathBuilder::stroke(px(1.));
                for i in 0..=STEPS {
                    let p = on_ring(from + PI * i as f32 / STEPS as f32);
                    if i == 0 { path.move_to(p) } else { path.line_to(p) }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, theme::text_section());
                }
            };
            let disc = |at: Point<Pixels>, r: f32, color: Rgba, window: &mut Window| {
                window.paint_quad(fill(Bounds::centered_at(at, size(px(2. * r), px(2. * r))), color).corner_radii(px(r)));
            };
            let angle = t * TAU;
            // sin < 0: the far half, above the disc on screen.
            let behind = angle > PI;
            let dot = |window: &mut Window| disc(on_ring(angle), size_px * 1.7 / 14., dot_color, window);
            half(PI, window);
            if behind {
                dot(window);
            }
            disc(center, size_px * 3.4 / 14., theme::orbit_sphere(), window);
            half(0., window);
            if !behind {
                dot(window);
            }
        },
    )
    .size(px(size_px))
}
