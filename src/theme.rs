use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use gpui::{App, BoxShadow, InteractiveElement, ParentElement, Pixels, Rgba, SharedString, StatefulInteractiveElement, Styled, point, px, rgb, rgba};

static LIGHT: AtomicBool = AtomicBool::new(false);

pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

fn pick(dark: u32, light: u32) -> Rgba {
    rgb(if is_light() { light } else { dark })
}

fn pick_a(dark: u32, light: u32) -> Rgba {
    rgba(if is_light() { light } else { dark })
}

pub fn bg_sidebar() -> Rgba { pick(0x111113, 0xF3F3F5) }
pub fn bg_page() -> Rgba { pick(0x151517, 0xFCFCFD) }
pub fn bg_card() -> Rgba { pick(0x1C1C1F, 0xF4F4F6) }
pub fn bg_raised() -> Rgba { pick(0x26262A, 0xEAEAED) }
pub fn row_active() -> Rgba { pick(0x1E1E21, 0xE6E6EA) }
pub fn bg_choice() -> Rgba { pick(0x212124, 0xEFEFF2) }
pub fn bg_sunken() -> Rgba { pick(0x18181A, 0xF8F8FA) }
pub fn bg_urgent() -> Rgba { pick(0x1F1712, 0xFCF1EB) }
pub fn bg_tag() -> Rgba { pick(0x222225, 0xECECEF) }
pub fn text_tag() -> Rgba { pick(0x9A9997, 0x5C5C64) }
pub fn border() -> Rgba { pick(0x2A2A2E, 0xE1E1E6) }
pub fn sidebar_edge() -> Rgba { pick(0x1C1C1F, 0xE4E4E8) }
pub fn composer_edge() -> Rgba { pick(0x3A3A40, 0xD6D6DC) }
pub fn composer_focus_edge() -> Rgba { pick(0x6A6A72, 0x8A8A92) }
pub fn composer_bg() -> Rgba { pick(0x1C1C1F, 0xFFFFFF) }
pub fn control_edge() -> Rgba { pick(0x3A3A40, 0x8A8A92) }
pub fn divider() -> Rgba { pick(0x1F1F22, 0xE6E6EA) }
pub fn scrim() -> Rgba { pick_a(0x08080A9E, 0x18181E52) }

pub fn popover_bg() -> Rgba { pick(0x26262A, 0xFFFFFF) }
pub fn dialog_bg() -> Rgba { pick(0x1C1C1F, 0xFFFFFF) }
pub fn popover_edge() -> Rgba { pick(0x3A3A40, 0xD9D9DF) }
pub fn menu_hover() -> Rgba { pick(0x3A3A40, 0xEEEEF1) }
pub fn button_hover() -> Rgba { pick(0x2E2E33, 0xE2E2E6) }

pub fn panel_bg() -> Rgba { pick(0x1C1C1F, 0xFCFCFD) }
pub fn chip_bg() -> Rgba { pick(0x222225, 0xFFFFFF) }
pub fn chip_edge() -> Rgba { pick_a(0x00000000, 0xD9D9DFFF) }
pub fn nav_bg() -> Rgba { pick(0x1A1A1D, 0xF3F3F5) }
pub fn icon_grey() -> Rgba { pick(0x999999, 0x6B6B73) }
pub fn mark_bg() -> Rgba { pick_a(0xCC3F0047, 0xCC3F002E) }
pub fn lit_bg() -> Rgba { pick_a(0xCC3F001A, 0xCC3F0012) }
pub fn about_footer() -> Rgba { pick(0x18181B, 0xF8F8FA) }
pub fn approval_ring() -> Rgba { pick_a(0xCC3F001A, 0xCC3F001F) }

pub fn text_primary() -> Rgba { pick(0xF0EFEC, 0x1B1B1F) }
pub fn text_secondary() -> Rgba { pick(0xBDBCBA, 0x45454C) }
pub fn text_row_active() -> Rgba { pick(0xD4D3D0, 0x202025) }
pub fn text_new() -> Rgba { pick(0xA3A2A0, 0x505058) }
pub fn text_muted() -> Rgba { pick(0x8C8B8A, 0x5C5C64) }
pub fn text_row() -> Rgba { pick(0xBDBCBA, 0x5C5C64) }
pub fn text_faint() -> Rgba { pick(0x858483, 0x66666E) }
pub fn text_section() -> Rgba { pick(0x888786, 0x6B6B73) }

pub fn accent() -> Rgba { rgb(0xCC3F00) }
pub fn accent_text() -> Rgba { pick(0xE08A5E, 0xB23600) }
pub fn focus_ring() -> Rgba { pick(0xE08A5E, 0xCC3F00) }

pub fn diff_add() -> Rgba { pick(0x6CC784, 0x1C7038) }
pub fn diff_del() -> Rgba { pick(0xE07A7A, 0xB42A36) }
pub fn diff_add_tint() -> Rgba { pick_a(0x6CC7841F, 0x1C8C461F) }
pub fn diff_del_tint() -> Rgba { pick_a(0xE07A7A1F, 0xC8283C1A) }
pub fn danger() -> Rgba { diff_del() }
pub fn orbit_sphere() -> Rgba { pick(0x9A9AA0, 0x8A8A92) }
pub fn star() -> Rgba { rgb(0xF2E6D0) }

pub fn turtle_skin() -> Rgba { rgb(0xE08A5E) }
pub fn turtle_eye() -> Rgba { rgb(0x151517) }

pub fn sky() -> Rgba { rgb(0x151517) }
pub fn sky_text() -> Rgba { rgb(0xECECEC) }
pub fn sky_muted() -> Rgba { rgb(0x8C8C8C) }

fn shadow(color: Rgba, y: f32, blur: f32) -> BoxShadow {
    BoxShadow { color: color.into(), offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(0.), inset: false }
}

pub fn popover_shadow() -> Vec<BoxShadow> {
    vec![shadow(pick_a(0x00000073, 0x14141E24), 12., 32.)]
}

pub fn popover<E: Styled>(e: E) -> E {
    e.rounded(px(8.)).border_1().border_color(popover_edge()).bg(popover_bg()).shadow(popover_shadow())
}

pub fn dialog_shadow(y: f32, blur: f32) -> Vec<BoxShadow> {
    vec![shadow(pick_a(0x00000099, 0x14141E2E), y, blur)]
}

pub fn composer_shadow() -> Vec<BoxShadow> {
    if is_light() { vec![shadow(rgba(0x14141E0F), 1., 3.)] } else { Vec::new() }
}

pub fn ring(gap: Rgba) -> Vec<BoxShadow> {
    let spread = |color: Rgba, by: f32| BoxShadow { color: color.into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(by), inset: false };
    vec![spread(focus_ring(), 4.), spread(gap, 2.)]
}

pub trait FocusRing: InteractiveElement + Sized {
    fn focus_ring(self) -> Self {
        self.focus_ring_on(bg_page())
    }

    fn focus_ring_on(self, surface: Rgba) -> Self {
        self.focus_visible(move |s| s.shadow(ring(surface)))
    }
}

impl<E: InteractiveElement> FocusRing for E {}

pub trait TextButton: StatefulInteractiveElement + ParentElement + Sized {
    fn button_text(self, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        self.aria_label(text.clone()).child(text)
    }
}

impl<E: StatefulInteractiveElement + ParentElement> TextButton for E {}

pub fn hex(color: Rgba) -> gpui::SharedString {
    let byte = |c: f32| (c * 255.).round() as u8;
    format!("#{:02X}{:02X}{:02X}", byte(color.r), byte(color.g), byte(color.b)).into()
}

pub const SANS: &str = "Endeavor Sans";
pub const MONO: &str = "JuliaMono";
pub const JULIA_MONO_REGULAR: &[u8] = include_bytes!("../fonts/JuliaMono-Regular.ttf");
pub const JULIA_MONO_BOLD: &[u8] = include_bytes!("../fonts/JuliaMono-Bold.ttf");
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

pub fn size_meta_small() -> Pixels { px(11.) }
pub fn size_meta() -> Pixels { px(12.) }
pub fn size_body() -> Pixels { px(13.) }
pub fn line_body() -> Pixels { px(19.5) }
pub fn size_subhead() -> Pixels { px(15.) }
pub fn size_title() -> Pixels { px(21.) }
pub fn size_code() -> Pixels { px(12.) }

static CHAT_SCALE: AtomicU32 = AtomicU32::new(1.0f32.to_bits());

const CHAT_STEPS: (i32, i32) = (-3, 7);

fn chat_step_of(scale: f32) -> i32 {
    (scale.ln() / 1.1f32.ln()).round() as i32
}

fn chat_scale_at(step: i32) -> f32 {
    1.1f32.powi(step.clamp(CHAT_STEPS.0, CHAT_STEPS.1))
}

pub fn chat_scale_step(scale: f32, bigger: bool) -> f32 {
    let step = if scale > 0. { chat_step_of(scale) } else { 0 };
    chat_scale_at(step + if bigger { 1 } else { -1 })
}

pub fn set_chat_scale(scale: f32) {
    let scale = if scale > 0. { chat_scale_at(chat_step_of(scale)) } else { 1. };
    CHAT_SCALE.store(scale.to_bits(), Ordering::Relaxed);
}

pub fn chat_scale() -> f32 {
    f32::from_bits(CHAT_SCALE.load(Ordering::Relaxed))
}

pub fn chat_px(n: f32) -> Pixels {
    px(scaled_size(n, chat_scale()))
}

pub(crate) fn scaled_size(n: f32, scale: f32) -> f32 {
    (n * scale * 2.).round() / 2.
}

pub fn chat_meta_small() -> Pixels { chat_px(12.) }
pub fn chat_meta() -> Pixels { chat_px(13.) }
pub fn chat_body() -> Pixels { chat_px(14.) }
pub fn chat_line_body() -> Pixels { chat_px(21.) }
pub fn chat_subhead() -> Pixels { chat_px(16.) }
pub fn chat_code() -> Pixels { chat_px(13.) }

static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

pub fn set_reduce_motion(reduce: bool) {
    REDUCE_MOTION.store(reduce, Ordering::Relaxed);
}

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

pub fn motion_fast() -> Duration { motion(120.) }
pub fn motion_standard() -> Duration { motion(160.) }
pub fn motion_pulse() -> Duration { motion(1400.) }
pub fn motion_rise() -> Pixels { px(4.) }

pub fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_known_controls_say_they_are_open_without_handling_expand() {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
            let path = entry.unwrap().path();
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let opens = text.matches(".aria_expanded(").count() + text.matches("Role::ComboBox").count();
            if path.extension().is_some_and(|e| e == "rs") && opens > 0 && !text.contains("on_a11y_action") {
                found.push((path.file_name().unwrap().to_string_lossy().into_owned(), opens));
            }
        }
        found.sort();
        let known = [("settings_panel.rs".to_string(), 2), ("sidebar.rs".to_string(), 1)];
        assert_eq!(found, known, "docs/accessibility.md: a control that says it is open needs an Expand and Collapse handler on Windows");
    }

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

    #[test]
    fn chat_sizes_scale_to_half_pixels() {
        assert_eq!(scaled_size(14., 1.), 14.);
        assert_eq!(scaled_size(14., 1.1), 15.5);
        assert_eq!(scaled_size(13., 1. / 1.1), 12.);
        assert_eq!(scaled_size(21., 2.), 42.);
    }

    #[test]
    fn chat_steps_come_back_to_the_default() {
        let mut scale = 1.;
        for _ in 0..12 {
            scale = chat_scale_step(scale, true);
        }
        assert_eq!(scale, chat_scale_at(7));
        for _ in 0..7 {
            scale = chat_scale_step(scale, false);
        }
        assert_eq!(scale, 1.);
        for _ in 0..12 {
            scale = chat_scale_step(scale, false);
        }
        assert_eq!(scaled_size(14., scale), 10.5);
        assert_eq!(chat_scale_step(1.33, false), chat_scale_at(2));
    }
}
