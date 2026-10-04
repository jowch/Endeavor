//! Colour and type tokens from docs/ui-spec.md, and the bundled fonts. Every
//! colour, font and text size in the native UI comes from here, and every
//! animation's duration and easing.
//!
//! Each colour has a dark and a light value. Which one the functions return is
//! one process-wide switch, set by `set_light` from Settings → Appearance (and
//! macOS's own setting under Match macOS) before the windows are redrawn, so a
//! view reads the resolved colour without being handed the appearance.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{App, BoxShadow, InteractiveElement, Pixels, Rgba, Styled, point, px, rgb, rgba};

static LIGHT: AtomicBool = AtomicBool::new(false);

/// Switch every token to its light (or dark) value. The caller refreshes the
/// windows afterwards.
pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// An opaque colour: `0xRRGGBB` for dark, then for light.
fn pick(dark: u32, light: u32) -> Rgba {
    rgb(if is_light() { light } else { dark })
}

/// A colour with alpha: `0xRRGGBBAA` for dark, then for light.
fn pick_a(dark: u32, light: u32) -> Rgba {
    rgba(if is_light() { light } else { dark })
}

/// Sessions sidebar.
pub fn bg_sidebar() -> Rgba { pick(0x111113, 0xF3F3F5) }
/// Chat, notebook, headers.
pub fn bg_page() -> Rgba { pick(0x151517, 0xFCFCFD) }
/// Composer, cards, inputs.
pub fn bg_card() -> Rgba { pick(0x1C1C1F, 0xF4F4F6) }
/// User bubble, secondary buttons.
pub fn bg_raised() -> Rgba { pick(0x26262A, 0xEAEAED) }
/// The active sidebar row.
pub fn row_active() -> Rgba { pick(0x1E1E21, 0xE6E6EA) }
/// The choice to pick when unsure (sign-in's first account kind).
pub fn bg_choice() -> Rgba { pick(0x212124, 0xEFEFF2) }
/// Below the card colour: other choices, queued messages, details.
pub fn bg_sunken() -> Rgba { pick(0x18181A, 0xF8F8FA) }
/// Behind a card that needs the user now (approvals, plans): the card colour
/// tinted with the accent.
pub fn bg_urgent() -> Rgba { pick(0x1F1712, 0xFCF1EB) }
/// Tags (the chat header's folder, inline code).
pub fn bg_tag() -> Rgba { pick(0x222225, 0xECECEF) }
pub fn text_tag() -> Rgba { pick(0x9A9997, 0x5C5C64) }
/// Card outlines; decorative.
pub fn border() -> Rgba { pick(0x2A2A2E, 0xE1E1E6) }
/// The sidebar's right edge.
pub fn sidebar_edge() -> Rgba { pick(0x1C1C1F, 0xE4E4E8) }
/// The message box's soft outline.
pub fn composer_edge() -> Rgba { pick(0x3A3A40, 0xD6D6DC) }
/// The message box's outline while it has the keyboard.
pub fn composer_focus_edge() -> Rgba { pick(0x55555C, 0x8A8A92) }
/// The message box's fill: the card colour in dark, white in light.
pub fn composer_bg() -> Rgba { pick(0x1C1C1F, 0xFFFFFF) }
/// The outline of Outlined buttons and text fields (3:1 in light).
pub fn control_edge() -> Rgba { pick(0x3A3A40, 0x8A8A92) }
/// Column dividers.
pub fn divider() -> Rgba { pick(0x1F1F22, 0xE6E6EA) }
/// The dimmed backdrop behind a panel or dialog (Settings, the confirm dialog).
pub fn scrim() -> Rgba { pick_a(0x08080A9E, 0x18181E52) }

/// Menus, popovers and dialogs.
pub fn popover_bg() -> Rgba { pick(0x26262A, 0xFFFFFF) }
/// Dialogs over the scrim (a server's settings, ssh's password prompt) and
/// the setup window's panel: the card colour in dark, white in light.
pub fn dialog_bg() -> Rgba { pick(0x1C1C1F, 0xFFFFFF) }
/// Their 1 px outline.
pub fn popover_edge() -> Rgba { pick(0x3A3A40, 0xD9D9DF) }
/// A hovered or keyboard-selected menu row.
pub fn menu_hover() -> Rgba { pick(0x3A3A40, 0xEEEEF1) }
/// A hovered secondary button.
pub fn button_hover() -> Rgba { pick(0x2E2E33, 0xE2E2E6) }

/// Settings' panel and its groups: the card colour in dark, the page in light.
pub fn panel_bg() -> Rgba { pick(0x1C1C1F, 0xFCFCFD) }
/// The new-session screen's chips (This Mac, the folder, the notebook).
pub fn chip_bg() -> Rgba { pick(0x222225, 0xFFFFFF) }
/// Their outline: none in dark, a hairline in light.
pub fn chip_edge() -> Rgba { pick_a(0x00000000, 0xD9D9DFFF) }
/// Settings' section list.
pub fn nav_bg() -> Rgba { pick(0x1A1A1D, 0xF3F3F5) }
/// Settings' row icons.
pub fn icon_grey() -> Rgba { pick(0x999999, 0x6B6B73) }
/// A search hit's highlight.
pub fn mark_bg() -> Rgba { pick_a(0xCC3F0047, 0xCC3F002E) }
/// The row a search result or link opened.
pub fn lit_bg() -> Rgba { pick_a(0xCC3F001A, 0xCC3F0012) }
/// The About window's footer.
pub fn about_footer() -> Rgba { pick(0x18181B, 0xF8F8FA) }
/// The approval card's 3 px halo.
pub fn approval_ring() -> Rgba { pick_a(0xCC3F001A, 0xCC3F001F) }

/// Chat body.
pub fn text_primary() -> Rgba { pick(0xF0EFEC, 0x1B1B1F) }
/// Cell names, model/effort.
pub fn text_secondary() -> Rgba { pick(0xBDBCBA, 0x45454C) }
/// The active sidebar row's text.
pub fn text_row_active() -> Rgba { pick(0xD4D3D0, 0x202025) }
/// "New session" in the sidebar.
pub fn text_new() -> Rgba { pick(0xA3A2A0, 0x505058) }
/// Sidebar rows outside the session list (the status line, "Show N more"…).
pub fn text_muted() -> Rgba { pick(0x8C8B8A, 0x5C5C64) }
/// Sidebar session rows (open and past): a dark-contrast raise over
/// `text_muted`, so folder headings (`text_section`) read as the lower level.
pub fn text_row() -> Rgba { pick(0xBDBCBA, 0x5C5C64) }
/// Tool lines, timestamps.
pub fn text_faint() -> Rgba { pick(0x858483, 0x66666E) }
/// Section heads.
pub fn text_section() -> Rgba { pick(0x888786, 0x6B6B73) }

/// Filled primary (white text), status bar of cells Claude touched, busy dot.
pub fn accent() -> Rgba { rgb(0xCC3F00) }
/// Orange text and icons.
pub fn accent_text() -> Rgba { pick(0xE08A5E, 0xB23600) }
/// The keyboard focus ring, shown only while a control is focused by Tab or
/// arrow keys, not by a mouse click.
pub fn focus_ring() -> Rgba { pick(0xE08A5E, 0xCC3F00) }

pub fn diff_add() -> Rgba { pick(0x6CC784, 0x1C7038) }
pub fn diff_del() -> Rgba { pick(0xE07A7A, 0xB42A36) }
/// Line tints.
pub fn diff_add_tint() -> Rgba { pick_a(0x6CC7841F, 0x1C8C461F) }
pub fn diff_del_tint() -> Rgba { pick_a(0xE07A7A1F, 0xC8283C1A) }
/// Errors use the removal red.
pub fn danger() -> Rgba { diff_del() }
/// The working indicator's centre sphere.
pub fn orbit_sphere() -> Rgba { pick(0x9A9AA0, 0x8A8A92) }
/// The stars over the setup screen's turtle (the sky stays dark).
pub fn star() -> Rgba { rgb(0xF2E6D0) }

/// The turtle's head and feet, and its eye: an illustration, the same in both.
pub fn turtle_skin() -> Rgba { rgb(0xE08A5E) }
pub fn turtle_eye() -> Rgba { rgb(0x151517) }

/// The setup window's night sky, a dark band in both appearances, and the
/// name and tagline on it.
pub fn sky() -> Rgba { rgb(0x151517) }
pub fn sky_text() -> Rgba { rgb(0xECECEC) }
pub fn sky_muted() -> Rgba { rgb(0x8C8C8C) }

fn shadow(color: Rgba, y: f32, blur: f32) -> BoxShadow {
    BoxShadow { color: color.into(), offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(0.), inset: false }
}

/// Under menus and popovers.
pub fn popover_shadow() -> Vec<BoxShadow> {
    vec![shadow(pick_a(0x00000073, 0x14141E24), 12., 32.)]
}

/// A menu or popover's frame: white with a hairline and a soft shadow in light.
pub fn popover<E: Styled>(e: E) -> E {
    e.rounded(px(8.)).border_1().border_color(popover_edge()).bg(popover_bg()).shadow(popover_shadow())
}

/// Under the confirm dialog, Settings and other panels over the scrim.
pub fn dialog_shadow(y: f32, blur: f32) -> Vec<BoxShadow> {
    vec![shadow(pick_a(0x00000099, 0x14141E2E), y, blur)]
}

/// The message box's faint shadow (light only).
pub fn composer_shadow() -> Vec<BoxShadow> {
    if is_light() { vec![shadow(rgba(0x14141E0F), 1., 3.)] } else { Vec::new() }
}

/// A 2 px ring 2 px outside the control, with the gap in `gap` (the colour the
/// control sits on). Drop shadows fill under the whole control, so the gap
/// shadow goes on top of the ring's and a see-through control shows `gap`.
pub fn ring(gap: Rgba) -> Vec<BoxShadow> {
    let spread = |color: Rgba, by: f32| BoxShadow { color: color.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(by), inset: false };
    vec![spread(focus_ring(), 4.), spread(gap, 2.)]
}

/// Keyboard focus for buttons: the outside ring. Outlined buttons recolour
/// their own outline instead (`focus_visible(|s| s.border_color(focus_ring()))`).
pub trait FocusRing: InteractiveElement + Sized {
    /// On the page colour.
    fn focus_ring(self) -> Self {
        self.focus_ring_on(bg_page())
    }

    /// On another surface, such as a card.
    fn focus_ring_on(self, surface: Rgba) -> Self {
        self.focus_visible(move |s| s.shadow(ring(surface)))
    }
}

impl<E: InteractiveElement> FocusRing for E {}

/// "#2A2A2E", for the component library's theme config.
pub fn hex(color: Rgba) -> gpui::SharedString {
    let byte = |c: f32| (c * 255.).round() as u8;
    format!("#{:02X}{:02X}{:02X}", byte(color.r), byte(color.g), byte(color.b)).into()
}

/// The interface font (fonts/README.md) and the code font, for cell names, paths,
/// counts, code and tool names. Bundled, so they look the same everywhere.
pub const SANS: &str = "Endeavor Sans";
pub const MONO: &str = "JuliaMono";
pub const JULIA_MONO_REGULAR: &[u8] = include_bytes!("../fonts/JuliaMono-Regular.ttf");
pub const JULIA_MONO_BOLD: &[u8] = include_bytes!("../fonts/JuliaMono-Bold.ttf");
/// Only for Pluto's page; the app sets no mono italic.
pub const JULIA_MONO_ITALIC: &[u8] = include_bytes!("../fonts/JuliaMono-RegularItalic.ttf");

pub fn load_fonts(cx: &mut App) {
    let fonts = [
        include_bytes!("../fonts/EndeavorSans-Regular.ttf").as_slice(),
        include_bytes!("../fonts/EndeavorSans-Medium.ttf"),
        include_bytes!("../fonts/EndeavorSans-SemiBold.ttf"),
        JULIA_MONO_REGULAR,
        JULIA_MONO_BOLD,
    ];
    cx.text_system().add_fonts(fonts.into_iter().map(Cow::Borrowed).collect()).expect("bundled fonts");
}

/// Section heads, tags, keyboard hints.
pub fn size_meta_small() -> Pixels { px(11.) }
/// Tool lines, toolbar, timestamps, captions.
pub fn size_meta() -> Pixels { px(12.) }
/// Chat messages, sidebar rows, cards, settings.
pub fn size_body() -> Pixels { px(13.) }
pub fn line_body() -> Pixels { px(19.5) }
/// Card titles ("Run 3 cells?"), medium weight.
pub fn size_subhead() -> Pixels { px(15.) }
/// Titles, semibold.
pub fn size_title() -> Pixels { px(21.) }
/// JuliaMono inside body text: a size smaller, since it looks larger than the
/// interface font at the same size.
pub fn size_code() -> Pixels { px(12.) }

// The chat column (the transcript, the composer and its cards): the same ladder,
// one step up, so chat's reading text sits a size larger than the rest of the
// interface (sidebar, Settings, menus, the notebook header), which keeps
// `size_meta_small` .. `size_code` unchanged.

/// Chat: keyboard hints. One step up from `size_meta_small`.
pub fn chat_meta_small() -> Pixels { px(12.) }
/// Chat: tool lines, timestamps, "Edited…"/"Ran…" rows, notes ("You stopped
/// Claude"), the chips row under the composer. One step up from `size_meta`.
pub fn chat_meta() -> Pixels { px(13.) }
/// Chat: message text (user bubbles and replies, including markdown), the
/// composer's typed text and placeholder, and the approval/plan cards' body
/// text. One step up from `size_body`.
pub fn chat_body() -> Pixels { px(14.) }
pub fn chat_line_body() -> Pixels { px(21.) }
/// Chat: card titles ("Run 3 cells?"). One step up from `size_subhead`.
pub fn chat_subhead() -> Pixels { px(16.) }
/// JuliaMono inside chat text: a size smaller than `chat_body`, as `size_code`
/// is for `size_body`.
pub fn chat_code() -> Pixels { px(13.) }

// Motion: short fades and small slides, nothing at all with Reduce motion.
// Like the appearance, one process-wide switch (`set_reduce_motion`), so a
// view reads a duration without being handed the setting.

static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

/// Reduce motion on: every duration below is zero. `motion::set_reduced` also
/// tells GPUI, which stills the orbit and the component library's popovers.
pub fn set_reduce_motion(reduce: bool) {
    REDUCE_MOTION.store(reduce, Ordering::Relaxed);
}

/// Debug builds: `ENDEAVOR_MOTION_SCALE=10` plays every animation ten times
/// slower, to check it by eye or in screenshots; 0 turns motion off.
#[cfg(debug_assertions)]
fn motion_scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| std::env::var("ENDEAVOR_MOTION_SCALE").ok().and_then(|s| s.parse().ok()).filter(|s: &f32| *s >= 0.).unwrap_or(1.))
}

#[cfg(not(debug_assertions))]
fn motion_scale() -> f32 {
    1.
}

fn motion(ms: f32) -> Duration {
    scaled(ms, REDUCE_MOTION.load(Ordering::Relaxed), motion_scale())
}

pub(crate) fn scaled(ms: f32, reduce: bool, scale: f32) -> Duration {
    if reduce {
        return Duration::ZERO;
    }
    Duration::from_micros((ms * scale * 1000.).round() as u64)
}

/// Something appearing (a card, a menu, a new transcript entry), and a line
/// whose words change.
pub fn motion_fast() -> Duration { motion(120.) }
/// The transcript catching up with its end as content arrives.
pub fn motion_standard() -> Duration { motion(160.) }
/// How far something that appears moves into place.
pub fn motion_rise() -> Pixels { px(4.) }

/// Fast at first, settling at the end: for anything arriving.
pub fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test, since the appearance is process-wide.
    #[test]
    fn tokens_follow_the_appearance() {
        assert_eq!(hex(border()).as_ref(), "#2A2A2E");
        assert_eq!(hex(accent_text()).as_ref(), "#E08A5E");
        assert_eq!(mark_bg(), rgba(0xCC3F0047));
        set_light(true);
        let light = (hex(bg_page()), hex(focus_ring()), hex(accent()), scrim());
        set_light(false);
        assert_eq!(light, ("#FCFCFD".into(), "#CC3F00".into(), "#CC3F00".into(), rgba(0x18181E52)));
        assert_eq!(hex(focus_ring()).as_ref(), "#E08A5E");
    }
}
