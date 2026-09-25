//! Colour tokens from docs/ui-spec.md (dark only; neutrals carry a faint cool tint).
//! Every colour in the native UI comes from here.

use gpui::{Rgba, rgb, rgba};

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
/// Card and composer outlines.
pub fn border() -> Rgba { rgb(0x2A2A2E) }
/// Column dividers.
pub fn divider() -> Rgba { rgb(0x1F1F22) }

/// Chat body.
pub fn text_primary() -> Rgba { rgb(0xECECEC) }
/// Cell names, model/effort.
pub fn text_secondary() -> Rgba { rgb(0xBDBDBD) }
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
