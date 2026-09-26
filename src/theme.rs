//! Colour and type tokens from docs/ui-spec.md (dark only; neutrals carry a faint
//! cool tint), and the bundled fonts. Every colour, font and text size in the
//! native UI comes from here.

use std::borrow::Cow;

use gpui::{App, Pixels, Rgba, px, rgb, rgba};

/// Sessions sidebar.
pub fn bg_sidebar() -> Rgba { rgb(0x111113) }
/// Chat, notebook, headers.
pub fn bg_page() -> Rgba { rgb(0x151517) }
/// Composer, cards, inputs.
pub fn bg_card() -> Rgba { rgb(0x1C1C1F) }
/// User bubble, secondary buttons.
pub fn bg_raised() -> Rgba { rgb(0x26262A) }
/// The active sidebar row.
pub fn row_active() -> Rgba { rgb(0x1E1E21) }
/// Tags (the chat header's folder, inline code).
pub fn bg_tag() -> Rgba { rgb(0x222225) }
pub fn text_tag() -> Rgba { rgb(0x9A9A9A) }
/// Card and composer outlines.
pub fn border() -> Rgba { rgb(0x2A2A2E) }
/// The sidebar's right edge.
pub fn sidebar_edge() -> Rgba { rgb(0x1C1C1F) }
/// The composer's outline.
pub fn composer_edge() -> Rgba { rgb(0x3A3A40) }
/// Column dividers.
pub fn divider() -> Rgba { rgb(0x1F1F22) }

/// Chat body.
pub fn text_primary() -> Rgba { rgb(0xECECEC) }
/// Cell names, model/effort.
pub fn text_secondary() -> Rgba { rgb(0xBDBDBD) }
/// The active sidebar row's text.
pub fn text_row_active() -> Rgba { rgb(0xD4D4D4) }
/// "New session" in the sidebar.
pub fn text_new() -> Rgba { rgb(0xA3A3A3) }
/// Sidebar rows.
pub fn text_muted() -> Rgba { rgb(0x8C8C8C) }
/// Tool lines, timestamps.
pub fn text_faint() -> Rgba { rgb(0x7A7A7A) }
/// Section heads.
pub fn text_section() -> Rgba { rgb(0x5E5E5E) }

/// Filled primary (white text), unrun stripe, busy dot.
pub fn accent() -> Rgba { rgb(0xCC3F00) }
/// Orange text and icons on dark.
pub fn accent_text() -> Rgba { rgb(0xE08A5E) }

pub fn diff_add() -> Rgba { rgb(0x6CC784) }
pub fn diff_del() -> Rgba { rgb(0xE07A7A) }
/// Line tints (~12%).
pub fn diff_add_tint() -> Rgba { rgba(0x6CC7841F) }
pub fn diff_del_tint() -> Rgba { rgba(0xE07A7A1F) }
/// Errors use the removal red.
pub fn danger() -> Rgba { diff_del() }
/// The working indicator's centre sphere.
pub fn orbit_sphere() -> Rgba { rgb(0x9A9AA0) }

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
