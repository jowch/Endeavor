use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Bounds, Element, ElementId, FollowMode, GlobalElementId, InspectorElementId, IntoElement, LayoutId, ListState, Pixels, Styled, Window, px};

use crate::theme;

static QUIET: AtomicBool = AtomicBool::new(false);

pub fn hush(window: &Window) {
    QUIET.store(true, Ordering::Relaxed);
    window.on_next_frame(|_, _| QUIET.store(false, Ordering::Relaxed));
}

pub fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

pub fn set_reduced(reduce: bool, cx: &mut App) {
    theme::set_reduce_motion(reduce);
    cx.set_reduce_motion(reduce);
}

const LOOP_FRAME: Duration = Duration::from_millis(50);

thread_local! {
    static LOOPS: RefCell<HashMap<gpui::EntityId, (Instant, bool)>> = RefCell::new(HashMap::new());
}

pub fn loop_clock(window: &Window, cx: &mut App) -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    let now = Instant::now();
    let view = window.current_view();
    let ask = LOOPS.with(|loops| {
        let mut loops = loops.borrow_mut();
        let entry = loops.entry(view).or_insert((now, false));
        entry.0 = now;
        !std::mem::replace(&mut entry.1, true)
    });
    if ask {
        cx.spawn(async move |cx| {
            let mut wait = LOOP_FRAME;
            loop {
                cx.background_executor().timer(wait).await;
                let since = LOOPS.with(|loops| loops.borrow().get(&view).map(|(drawn, _)| drawn.elapsed()));
                match since {
                    Some(since) if since < LOOP_FRAME => wait = LOOP_FRAME - since,
                    _ => break,
                }
            }
            LOOPS.with(|loops| loops.borrow_mut().remove(&view));
            cx.update(|cx| cx.notify(view));
        })
        .detach();
    }
    (now.duration_since(*START.get_or_init(|| now)).as_millis() % 3_600_000) as f32 / 1000.
}

pub fn looping(period: Duration, window: &Window, cx: &mut App) -> f32 {
    let period = period.as_secs_f32().max(0.001);
    (loop_clock(window, cx) % period) / period
}

pub fn progress(start: Instant, now: Instant, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 1.;
    }
    (now.saturating_duration_since(start).as_secs_f32() / duration.as_secs_f32()).min(1.)
}

pub fn eased(start: Instant, duration: Duration, window: &Window) -> f32 {
    let t = progress(start, Instant::now(), duration);
    if t < 1. {
        window.request_animation_frame();
    }
    theme::ease_out(t)
}

struct Change<T> {
    value: T,
    was: Option<(T, Instant)>,
}

pub fn changed<T: Clone + PartialEq + 'static>(id: impl Into<ElementId>, value: &T, window: &mut Window, cx: &mut App) -> Option<(T, f32)> {
    let state = window.use_keyed_state(id, cx, |_, _| Change { value: value.clone(), was: None });
    let window = &*window;
    state.update(cx, |change, _| {
        if change.value != *value {
            let was = std::mem::replace(&mut change.value, value.clone());
            change.was = (!quiet()).then(|| (was, Instant::now()));
        }
        let (was, start) = change.was.as_ref()?;
        let p = eased(*start, theme::motion_fast(), window);
        if p >= 1. {
            change.was = None;
            return None;
        }
        Some((was.clone(), p))
    })
}

pub fn arrive<E: Styled>(e: E, p: f32, down: bool) -> E {
    if p >= 1. {
        return e;
    }
    let off = theme::motion_rise() * (1. - p);
    e.opacity(p).relative().top(if down { -off } else { off })
}

pub fn arriving<E: IntoElement + Styled + 'static>(e: E, id: impl Into<ElementId>, down: bool) -> Arriving<E> {
    Arriving { id: id.into(), element: Some(e), down }
}

pub struct Arriving<E> {
    id: ElementId,
    element: Option<E>,
    down: bool,
}

impl<E: IntoElement + Styled + 'static> IntoElement for Arriving<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: IntoElement + Styled + 'static> Element for Arriving<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, AnyElement) {
        let born = window.with_element_state(id.expect("has an id"), |born: Option<Option<Instant>>, _| {
            let born = born.unwrap_or_else(|| (!quiet()).then(Instant::now));
            (born, born)
        });
        let p = born.map_or(1., |born| eased(born, theme::motion_fast(), window));
        let element = self.element.take().expect("laid out once");
        let mut element = arrive(element, p, self.down).into_any_element();
        (element.request_layout(window, cx), element)
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, element: &mut AnyElement, window: &mut Window, cx: &mut App) {
        element.prepaint(window, cx);
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, element: &mut AnyElement, _: &mut (), window: &mut Window, cx: &mut App) {
        element.paint(window, cx);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ListView {
    pub following: bool,
    pub top: (usize, Pixels),
    pub scrolled: Pixels,
    pub end: Option<Pixels>,
}

#[derive(Debug, PartialEq)]
pub enum Move {
    Nothing,
    Hold,
    Snap,
    By(Pixels),
}

#[derive(Default)]
pub struct Glide {
    holding: bool,
    left: Option<(usize, Pixels)>,
    from: Option<(Pixels, Instant, Pixels)>,
    chained: bool,
}

impl Glide {
    pub fn holding(&self) -> bool {
        self.holding
    }

    pub fn chained(&self) -> bool {
        self.chained
    }

    pub fn step(&mut self, view: ListView, now: Instant, duration: Duration, quiet: bool) -> Move {
        if duration.is_zero() || quiet {
            let held = std::mem::take(self).holding && !view.following;
            return if held { Move::Snap } else { Move::Nothing };
        }
        if self.holding && self.left != Some(view.top) {
            *self = Glide::default();
        }
        if view.following {
            *self = Glide { holding: true, left: Some(view.top), from: None, chained: false };
            return Move::Hold;
        }
        if !self.holding {
            return Move::Nothing;
        }
        let Some(end) = view.end else {
            *self = Glide::default();
            return Move::Snap;
        };
        if end - view.scrolled <= px(0.5) {
            self.from = None;
            self.chained = false;
            return Move::Nothing;
        }
        let (from, start) = match self.from {
            Some((from, start, to)) if to == end => (from, start),
            was => {
                self.chained = was.is_some();
                self.from = Some((view.scrolled, now, end));
                (view.scrolled, now)
            }
        };
        let t = progress(start, now, duration);
        if t >= 1. {
            self.from = None;
        }
        Move::By(from + (end - from) * theme::ease_out(t) - view.scrolled)
    }

    pub fn left_at(&mut self, top: (usize, Pixels)) {
        self.left = Some(top);
    }
}

pub fn follow(glide: &RefCell<Glide>, list: &ListState, pad: Pixels, window: &Window, cx: &mut App) {
    let at = |list: &ListState| {
        let top = list.logical_scroll_top();
        (top.item_ix, top.offset_in_item)
    };
    let max = list.max_offset_for_scrollbar().y;
    let view = ListView { following: list.is_following_tail(), top: at(list), scrolled: -list.scroll_px_offset_for_scrollbar().y, end: (max > px(0.)).then(|| max + pad) };
    let mut glide = glide.borrow_mut();
    match glide.step(view, Instant::now(), theme::motion_standard(), quiet()) {
        Move::Nothing => {}
        Move::Hold => list.pause_following_tail(),
        Move::Snap => list.set_follow_mode(FollowMode::Tail),
        Move::By(by) => {
            list.scroll_by(by);
            glide.left_at(at(list));
            if glide.chained() {
                loop_clock(window, cx);
            } else {
                window.request_animation_frame();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn progress_runs_from_zero_to_one_over_its_duration() {
        let start = Instant::now();
        assert_eq!(progress(start, start, 120 * MS), 0.);
        assert_eq!(progress(start, start + 60 * MS, 120 * MS), 0.5);
        assert_eq!(progress(start, start + 500 * MS, 120 * MS), 1.);
        assert_eq!(progress(start, start, Duration::ZERO), 1.);
    }

    #[test]
    fn reduce_motion_and_the_debug_scale_set_the_durations() {
        assert_eq!(theme::motion_fast(), 120 * MS);
        assert_eq!(theme::scaled(120., true, 1.), Duration::ZERO);
        assert_eq!(theme::scaled(160., false, 10.), 1600 * MS);
        assert_eq!(theme::scaled(160., false, 0.), Duration::ZERO);
    }

    fn view(following: bool, top: usize, scrolled: f32, end: Option<f32>) -> ListView {
        ListView { following, top: (top, px(0.)), scrolled: px(scrolled), end: end.map(px) }
    }

    #[test]
    fn content_arriving_while_following_eases_to_the_new_end() {
        let mut glide = Glide::default();
        let t0 = Instant::now();
        assert_eq!(glide.step(view(true, 3, 400., Some(400.)), t0, 160 * MS, false), Move::Hold);
        let Move::By(by) = glide.step(view(false, 3, 400., Some(440.)), t0, 160 * MS, false) else { panic!() };
        assert_eq!(by, px(0.));
        glide.left_at((3, px(0.)));
        let Move::By(by) = glide.step(view(false, 3, 400., Some(440.)), t0 + 80 * MS, 160 * MS, false) else { panic!() };
        assert_eq!(by, px(35.));
        let Move::By(by) = glide.step(view(false, 3, 435., Some(440.)), t0 + 200 * MS, 160 * MS, false) else { panic!() };
        assert_eq!(by, px(5.));
        assert_eq!(glide.step(view(false, 3, 440., Some(440.)), t0 + 220 * MS, 160 * MS, false), Move::Nothing);
        assert!(glide.holding());
    }

    #[test]
    fn more_content_mid_ease_starts_again_from_where_the_view_is() {
        let mut glide = Glide::default();
        let t0 = Instant::now();
        glide.step(view(true, 3, 400., Some(400.)), t0, 160 * MS, false);
        glide.step(view(false, 3, 400., Some(440.)), t0, 160 * MS, false);
        glide.left_at((3, px(0.)));
        let Move::By(by) = glide.step(view(false, 3, 420., Some(480.)), t0 + 80 * MS, 160 * MS, false) else { panic!() };
        assert_eq!(by, px(0.), "a new ease starts where the view is, with no jump");
        assert!(glide.chained(), "content arriving mid-ease paces the glide");
    }

    #[test]
    fn a_glide_from_rest_runs_every_frame() {
        let mut glide = Glide::default();
        let t0 = Instant::now();
        glide.step(view(true, 3, 400., Some(400.)), t0, 160 * MS, false);
        glide.step(view(false, 3, 400., Some(440.)), t0, 160 * MS, false);
        assert!(!glide.chained());
        glide.left_at((3, px(0.)));
        assert_eq!(glide.step(view(false, 3, 440., Some(440.)), t0 + 200 * MS, 160 * MS, false), Move::Nothing);
        glide.step(view(false, 3, 440., Some(480.)), t0 + 300 * MS, 160 * MS, false);
        assert!(!glide.chained());
    }

    #[test]
    fn the_users_scroll_hands_the_list_back() {
        let mut glide = Glide::default();
        let t0 = Instant::now();
        glide.step(view(true, 3, 400., Some(400.)), t0, 160 * MS, false);
        assert_eq!(glide.step(view(false, 1, 120., Some(440.)), t0, 160 * MS, false), Move::Nothing);
        assert!(!glide.holding(), "scrolled up to read: nothing follows");
    }

    #[test]
    fn without_motion_the_list_snaps_as_it_always_did() {
        let mut glide = Glide::default();
        let t0 = Instant::now();
        assert_eq!(glide.step(view(true, 3, 400., Some(400.)), t0, Duration::ZERO, false), Move::Nothing);
        glide.step(view(true, 3, 400., Some(400.)), t0, 160 * MS, false);
        assert_eq!(glide.step(view(false, 3, 400., Some(440.)), t0, Duration::ZERO, false), Move::Snap);
        assert!(!glide.holding());
        glide.step(view(true, 3, 400., Some(400.)), t0, 160 * MS, false);
        assert_eq!(glide.step(view(false, 3, 400., Some(440.)), t0, 160 * MS, true), Move::Snap);
    }
}
