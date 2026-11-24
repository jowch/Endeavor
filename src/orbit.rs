use std::time::Duration;

use gpui::*;

use crate::theme;

pub fn orbit(id: ElementId, size: f32, cx: &App) -> AnyElement {
    orbit_with(id, size, theme::accent(), cx)
}

pub fn orbit_with(id: ElementId, size: f32, dot: Rgba, cx: &App) -> AnyElement {
    let lap = (!cx.reduce_motion()).then_some(Duration::from_secs(1));
    div().id(id).size(px(size)).child(orbit_at(lap, size, dot)).into_any_element()
}

fn orbit_at(lap: Option<Duration>, size_px: f32, dot_color: Rgba) -> impl IntoElement {
    use std::f32::consts::{PI, TAU};
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            let t = lap.map_or(0.25, |lap| crate::motion::looping(lap, window, cx));
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
