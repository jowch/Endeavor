//! Quiet motion (docs/ui-spec.md, Motion): what appears fades in and moves a
//! few pixels into place, a line whose words change crossfades, and the
//! transcript eases to its end as content arrives instead of jumping there.
//! Durations come from `theme` and are zero with Reduce motion.
//!
//! GPUI draws every frame from scratch, so whatever moves keeps its start time
//! somewhere that outlasts a frame: in the model (a transcript entry's
//! arrival), or in element state under an id, which lasts while the element is
//! drawn in consecutive frames. Nothing here holds up input: an element is
//! live from its first frame, and only its look changes.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Bounds, Element, ElementId, FollowMode, GlobalElementId, InspectorElementId, IntoElement, LayoutId, ListState, Pixels, Styled, Window, px};

use crate::theme;

static QUIET: AtomicBool = AtomicBool::new(false);

/// Nothing that first draws in this frame animates: the first frame after
/// launch, and one that shows another session, show everything in place.
pub fn hush(window: &Window) {
    QUIET.store(true, Ordering::Relaxed);
    window.on_next_frame(|_, _| QUIET.store(false, Ordering::Relaxed));
}

pub fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Reduce motion, from the system: the theme's durations and GPUI's own
/// animations both follow it.
pub fn set_reduced(reduce: bool, cx: &mut App) {
    theme::set_reduce_motion(reduce);
    cx.set_reduce_motion(reduce);
}

/// How far `now` is through `duration` from `start`: 0 to 1, and 1 for a zero
/// duration.
pub fn progress(start: Instant, now: Instant, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 1.;
    }
    (now.saturating_duration_since(start).as_secs_f32() / duration.as_secs_f32()).min(1.)
}

/// `progress` until now, eased out, asking for another frame until it's done.
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

/// While `value`, drawn under `id`, changes: what it was, and how far the
/// change has gone (eased, 0 to 1). None on its first draw, once the change is
/// over, and for a change in a quiet frame.
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

/// `e` faded in and moved `theme::motion_rise()` into place, `p` of the way
/// (0 to 1). `down`: it comes down from above (a menu from its button), else
/// up from below (a card from the composer).
pub fn arrive<E: Styled>(e: E, p: f32, down: bool) -> E {
    if p >= 1. {
        return e;
    }
    let off = theme::motion_rise() * (1. - p);
    e.opacity(p).relative().top(if down { -off } else { off })
}

/// `e`, arriving (`arrive`) from the first frame it's drawn in under `id`.
/// Drawn first in a quiet frame, it is in place at once.
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

/// What the transcript list shows, read before its layout.
#[derive(Clone, Copy, Debug)]
pub struct ListView {
    /// GPUI's list is following its end: each layout snaps there.
    pub following: bool,
    /// Its scroll position as the first item shown and the pixels into it.
    pub top: (usize, Pixels),
    /// How far it's scrolled from the start.
    pub scrolled: Pixels,
    /// How far it scrolls at its end; None when everything fits.
    pub end: Option<Pixels>,
}

/// What `Glide::step` does to the list before its layout.
#[derive(Debug, PartialEq)]
pub enum Move {
    Nothing,
    /// Stop the list snapping to its end in this layout, so what arrives lands
    /// below the view, and the next frame eases to it.
    Hold,
    /// Snap to the end and follow it, as the list does without motion.
    Snap,
    /// Scroll by this much, and draw another frame.
    By(Pixels),
}

/// Keeps the transcript's end in view as content arrives, easing there over
/// `theme::motion_standard()` instead of jumping. While the list follows its
/// end, each layout is held where the last one left it, so a growing reply or
/// a new entry lands below the view; the next frame sees how far the end
/// moved and eases there, starting again from where it is when more arrives.
/// The list itself picks following up again once the view reaches the end,
/// and stops when the user scrolls up, as without motion.
#[derive(Default)]
pub struct Glide {
    /// The list was following when this held it.
    holding: bool,
    /// Where this left the list; anything else moving it (the user's scroll,
    /// a jump to a message) hands it back.
    left: Option<(usize, Pixels)>,
    /// The current ease: where it started, when, and the end it goes to.
    from: Option<(Pixels, Instant, Pixels)>,
}

impl Glide {
    /// Following the end, held or not, for the "Jump to latest" pill.
    pub fn holding(&self) -> bool {
        self.holding
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
            *self = Glide { holding: true, left: Some(view.top), from: None };
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
            return Move::Nothing;
        }
        let (from, start) = match self.from {
            Some((from, start, to)) if to == end => (from, start),
            _ => {
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

    /// Where the list is after `step`'s move.
    pub fn left_at(&mut self, top: (usize, Pixels)) {
        self.left = Some(top);
    }
}

/// Run the transcript's glide before the list lays out. `pad`: the list's top
/// and bottom padding, which its scroll offsets leave out.
pub fn follow(glide: &RefCell<Glide>, list: &ListState, pad: Pixels, window: &Window) {
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
            window.request_animation_frame();
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
        // Reduce motion makes every duration zero: already there.
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
        // The reply grew 40 px below the held view.
        let Move::By(by) = glide.step(view(false, 3, 400., Some(440.)), t0, 160 * MS, false) else { panic!() };
        assert_eq!(by, px(0.));
        glide.left_at((3, px(0.)));
        let Move::By(by) = glide.step(view(false, 3, 400., Some(440.)), t0 + 80 * MS, 160 * MS, false) else { panic!() };
        assert_eq!(by, px(35.));
        // Done: at the end, where the list follows again by itself.
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
        // Reduce motion turned on, or another session shown, mid-hold.
        assert_eq!(glide.step(view(false, 3, 400., Some(440.)), t0, Duration::ZERO, false), Move::Snap);
        assert!(!glide.holding());
        glide.step(view(true, 3, 400., Some(400.)), t0, 160 * MS, false);
        assert_eq!(glide.step(view(false, 3, 400., Some(440.)), t0, 160 * MS, true), Move::Snap);
    }
}
