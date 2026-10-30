//! Mermaid diagrams in agent replies. A finished ```` ```mermaid ```` block is
//! drawn as a picture (bezel-mermaid, a Rust port of Obsidian's Pele); a block
//! still streaming in, one that doesn't parse, or one too big stays code.

use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex};

use gpui::{Image, ImageFormat};
use gpui_component::text::{MarkdownNode, MarkdownParseContext, markdown_ast as mdast};
#[cfg(debug_assertions)]
use serde_json::{Value, json};

/// The custom Markdown node a drawn block becomes. Its data is a [`Block`].
pub const NODE: &str = "mermaid";

/// Sources and drawings past these stay code: a diagram that big is no
/// longer readable in the chat, and drawing it would stall the reply.
const SOURCE_MAX: usize = 20_000;
const SIDE_MAX: f64 = 4_000.;

/// Drawn at twice its size and shown at its size, so it stays sharp on a
/// Retina screen (GPUI rasterizes an SVG image at the size it names).
const SCALE: f64 = 2.;

/// A diagram too wide for the chat is laid out again for the chat's width
/// (a flowchart that runs across runs down instead), in steps of this much,
/// so dragging the chat's edge doesn't draw it at every pixel.
const WIDTH_STEP: f32 = 80.;

/// A `mermaid` block that draws.
pub struct Block {
    pub source: String,
}

/// A diagram drawn for one width, in both themes so switching theme doesn't
/// draw it again.
pub struct Diagram {
    pub kind: String,
    pub width: f32,
    pub height: f32,
    dark: Arc<Image>,
    light: Arc<Image>,
}

impl Diagram {
    pub fn image(&self, light: bool) -> Arc<Image> {
        if light { self.light.clone() } else { self.dark.clone() }
    }
}

/// The custom block parser: a closed `mermaid` fence that draws becomes a
/// [`NODE`] node; anything else is left to the Markdown view.
pub fn parse_block(node: &mdast::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
    let mdast::Node::Code(code) = node else { return None };
    if !is_mermaid(code.lang.as_deref()) || !closed(cx.node_source(node)?) {
        return None;
    }
    let label = format!("{} diagram", fitted(&code.value, crate::CHAT_CONTENT_MAX).ok()?.kind);
    let markdown = format!("```mermaid\n{}\n```", code.value);
    let block = Block { source: code.value.clone() };
    Some(MarkdownNode::new(NODE, block).text(code.value.clone()).markdown(markdown).accessibility_label(label))
}

fn is_mermaid(lang: Option<&str>) -> bool {
    lang.is_some_and(|lang| lang.eq_ignore_ascii_case("mermaid"))
}

/// Whether a fenced block's source has its closing fence: a line of the
/// opener's character, at least as long. A block still streaming in runs to
/// the end of the reply without one, and drawing it half-written would
/// flicker between errors and partial drawings.
fn closed(source: &str) -> bool {
    let source = source.trim_end();
    let Some((first, rest)) = source.split_once('\n') else { return false };
    let first = first.trim_start();
    let Some(fence) = first.chars().next().filter(|c| *c == '`' || *c == '~') else { return false };
    let opener = first.chars().take_while(|c| *c == fence).count();
    let last = rest.rsplit('\n').next().unwrap_or(rest).trim();
    last.len() >= opener && last.chars().all(|c| c == fence)
}

/// `source` drawn to fit `width`: as wide as it likes when that fits, else
/// laid out again for the width. The wide drawing decides whether the block
/// draws at all, so a block never draws at one chat width and stays code at
/// another; a narrow layout that fails or is too big falls back to it,
/// scaled down. Kept for the last few diagrams and widths, since the chat
/// draws every frame.
pub fn fitted(source: &str, width: f32) -> Result<Arc<Diagram>, String> {
    fit(source, width, SIDE_MAX)
}

fn fit(source: &str, width: f32, side_max: f64) -> Result<Arc<Diagram>, String> {
    let wide = cached(source, crate::CHAT_CONTENT_MAX)?;
    fits(&wide, side_max)?;
    if wide.width <= width {
        return Ok(wide);
    }
    match cached(source, (width / WIDTH_STEP).floor().max(1.) * WIDTH_STEP) {
        Ok(narrow) if fits(&narrow, side_max).is_ok() => Ok(narrow),
        _ => Ok(wide),
    }
}

fn fits(d: &Diagram, side_max: f64) -> Result<(), String> {
    let (w, h) = (d.width as f64, d.height as f64);
    if w > side_max || h > side_max || w <= 0. || h <= 0. {
        return Err(format!("{w:.0} × {h:.0} px"));
    }
    Ok(())
}

// ponytail: an image GPUI has rasterized stays in its cache for the window's
// life, so each width a diagram is drawn at keeps its pixels. Fine for a few
// diagrams; a session full of them, resized often, grows.
fn cached(source: &str, width: f32) -> Result<Arc<Diagram>, String> {
    type Entry = ((u64, u32), Result<Arc<Diagram>, String>);
    static CACHE: LazyLock<Mutex<VecDeque<Entry>>> = LazyLock::new(Default::default);
    let lock = || CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let key = (hasher.finish(), width as u32);
    {
        let mut cache = lock();
        if let Some(at) = cache.iter().position(|(k, _)| *k == key) {
            // Most recently used goes last, so it is dropped last.
            let entry = cache.remove(at).expect("found");
            let hit = entry.1.clone();
            cache.push_back(entry);
            return hit;
        }
    }
    // Drawn outside the lock: a big diagram on the background parser
    // mustn't hold up the chat's drawing of others.
    let drawn = draw(source, width).map(Arc::new);
    let mut cache = lock();
    if !cache.iter().any(|(k, _)| *k == key) {
        if cache.len() >= 32 {
            cache.pop_front();
        }
        cache.push_back((key, drawn.clone()));
    }
    drawn
}

/// Draws `source` in both themes, as wide as `width` at most where its type
/// can draw itself narrower.
fn draw(source: &str, width: f32) -> Result<Diagram, String> {
    if source.len() > SOURCE_MAX {
        return Err(format!("longer than {SOURCE_MAX} bytes"));
    }
    let dark = mermaid::render(source, &options(DARK, width), &mermaid::Metrics).map_err(|e| e.message)?;
    let light = mermaid::render(source, &options(LIGHT, width), &mermaid::Metrics).map_err(|e| e.message)?;
    let image = |svg: &str| Arc::new(Image::from_bytes(ImageFormat::Svg, for_gpui(svg, SCALE).into_bytes()));
    Ok(Diagram {
        kind: format!("{:?}", dark.kind).to_lowercase(),
        width: dark.width as f32,
        height: dark.height as f32,
        dark: image(&dark.svg),
        light: image(&light.svg),
    })
}

fn options(colors: mermaid::Colors, width: f32) -> mermaid::Options {
    mermaid::Options {
        // Labels are measured with Arial's advances (`Metrics`); these are
        // Arial or fonts drawn to its metrics on each platform.
        font_family: Some("Helvetica, Arial, 'Liberation Sans', sans-serif".into()),
        font_size: 14.,
        max_width: Some(width as f64),
        // No links or images: the chat opens nothing a diagram names.
        links: false,
        images: false,
        limit: SOURCE_MAX,
        colors: Some(colors),
        ..Default::default()
    }
}

/// `svg` as GPUI's renderer (usvg) reads it: the root's `width` and `height`
/// multiplied by `factor` (the view box keeps the drawing the same), and
/// without the root style's custom properties. With `Options::colors` set
/// nothing reads them, and usvg drops every declaration after one, so the
/// labels lost their font and size.
fn for_gpui(svg: &str, factor: f64) -> String {
    let Some(end) = svg.find('>') else { return svg.to_owned() };
    let mut root = svg[..end].to_owned();
    for name in [" width=\"", " height=\""] {
        let Some(start) = root.find(name).map(|i| i + name.len()) else { return svg.to_owned() };
        let Some(len) = root[start..].find('"') else { return svg.to_owned() };
        let Ok(value) = root[start..start + len].parse::<f64>() else { return svg.to_owned() };
        root.replace_range(start..start + len, &format!("{}", value * factor));
    }
    if let Some(start) = root.find(" style=\"").map(|i| i + " style=\"".len())
        && let Some(len) = root[start..].find('"')
    {
        let style: Vec<&str> = root[start..start + len].split(';').filter(|d| !d.trim_start().starts_with("--")).collect();
        root.replace_range(start..start + len, &style.join(";"));
    }
    root + &svg[end..]
}

/// The chat's colours (theme.rs): the card behind, body text, the accent
/// for highlights, and a series that reads on both.
const DARK: mermaid::Colors = mermaid::Colors {
    bg: rgb(0x1C1C1F),
    fg: rgb(0xF0EFEC),
    muted: rgb(0x8C8B8A),
    line: rgb(0xBDBCBA),
    surface: rgb(0x26262A),
    surface_alt: rgb(0x222225),
    border: rgb(0x6A6A72),
    accent: rgb(0xE0662E),
    critical: rgb(0xE5534B),
    series: SERIES,
};
const LIGHT: mermaid::Colors = mermaid::Colors {
    bg: rgb(0xF4F4F6),
    fg: rgb(0x1B1B1F),
    muted: rgb(0x5C5C64),
    line: rgb(0x45454C),
    surface: rgb(0xFFFFFF),
    surface_alt: rgb(0xEAEAED),
    border: rgb(0x8A8A92),
    accent: rgb(0xCC3F00),
    critical: rgb(0xCF222E),
    series: SERIES,
};
const SERIES: [mermaid::Rgba; 8] = [
    rgb(0x4E79A7), rgb(0xF28E2B), rgb(0x59A14F), rgb(0xE15759),
    rgb(0x76B7B2), rgb(0xEDC948), rgb(0xB07AA1), rgb(0x9C755F),
];

const fn rgb(hex: u32) -> mermaid::Rgba {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255]
}

/// For the state dump: each `mermaid` block in a reply, and whether it drew.
#[cfg(debug_assertions)]
pub fn summary(text: &str) -> Vec<Value> {
    let Ok(root) = markdown::to_mdast(text, &markdown::ParseOptions::gfm()) else { return Vec::new() };
    let mut out = Vec::new();
    walk(&root, text, &mut out);
    out
}

#[cfg(debug_assertions)]
fn walk(node: &mdast::Node, text: &str, out: &mut Vec<Value>) {
    if let mdast::Node::Code(code) = node
        && is_mermaid(code.lang.as_deref())
    {
        let source = node.position().and_then(|p| text.get(p.start.offset..p.end.offset)).unwrap_or("");
        out.push(if !closed(source) {
            json!({ "drawn": false, "why": "unclosed" })
        } else {
            match fitted(&code.value, crate::CHAT_CONTENT_MAX) {
                Ok(d) => json!({ "drawn": true, "kind": d.kind, "width": d.width, "height": d.height }),
                Err(why) => json!({ "drawn": false, "why": why }),
            }
        });
    }
    for child in node.children().into_iter().flatten() {
        walk(child, text, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_fences() {
        assert!(closed("```mermaid\nflowchart LR\n  A --> B\n```"));
        assert!(closed("~~~mermaid\nA\n~~~~\n"));
        assert!(!closed("```mermaid\nflowchart LR\n  A --> B"));
        assert!(!closed("```mermaid\nflowchart LR\n``"));
        assert!(!closed("```mermaid"));
        assert!(!closed("````mermaid\nA\n```"), "shorter than the opener");
        assert!(!closed("```mermaid\nA\n~~~"), "another character");
        assert!(closed("````mermaid\nA\n`````"));
    }

    #[test]
    fn draws_a_flowchart_at_twice_its_size() {
        let d = draw("flowchart LR\n  A[Load] --> B[Fit]", 760.).unwrap();
        assert_eq!(d.kind, "flowchart");
        assert!(d.width > 0. && d.height > 0.);
        let svg = String::from_utf8(d.dark.bytes().to_vec()).unwrap();
        assert!(svg.contains(&format!("width=\"{}\"", d.width as f64 * SCALE)), "{}", &svg[..200]);
        assert!(!svg.contains("var(--"), "colours are written in, not CSS variables");
    }

    #[test]
    fn errors_and_limits_stay_code() {
        assert!(fitted("flowchart LR\n  A -->", 760.).is_err());
        assert!(fitted("not a diagram", 760.).is_err());
        assert!(fitted(&"flowchart LR\n".repeat(SOURCE_MAX), 760.).is_err());
        let chain = "flowchart LR\n  A --> B --> C";
        let wide = fitted(chain, 760.).unwrap();
        assert!(fit(chain, 760., (wide.width.max(wide.height) - 1.) as f64).is_err(), "too big at full width");
    }

    #[test]
    fn a_narrow_layout_too_big_falls_back_to_the_wide_one() {
        let chain = "flowchart LR\n  N0 --> N1 --> N2 --> N3 --> N4 --> N5 --> N6 --> N7";
        let wide = fitted(chain, 760.).unwrap();
        let narrow = fitted(chain, 200.).unwrap();
        assert!(narrow.height > wide.width.max(wide.height), "{} × {} vs {} × {}", narrow.width, narrow.height, wide.width, wide.height);
        // A cap the wide drawing fits and the narrow one doesn't.
        let cap = wide.width.max(wide.height) as f64 + 1.;
        let shown = fit(chain, 200., cap).unwrap();
        assert!(Arc::ptr_eq(&shown, &wide), "shows the wide drawing, scaled down");
    }

    #[test]
    fn a_wide_flowchart_runs_down_in_a_narrow_chat() {
        let source = "flowchart LR\n  A[Load CSV] --> B{Missing values?}\n  B -->|yes| C[Impute]\n  B -->|no| D[Fit model]\n  C --> D\n  D --> E((Report))";
        let wide = fitted(source, 760.).unwrap();
        assert!(wide.width > 400., "{}", wide.width);
        let narrow = fitted(source, 400.).unwrap();
        assert!(narrow.width <= 400. && narrow.height > wide.height, "{} × {}", narrow.width, narrow.height);
        assert!(Arc::ptr_eq(&narrow, &fitted(source, 410.).unwrap()), "same step, same drawing");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn summary_reports_each_block() {
        let text = "Here:\n\n```mermaid\npie\n  \"a\" : 1\n```\n\n```mermaid\nflowchart LR\n  A -->\n```\n\n```mermaid\nflowchart LR\n  A --> B";
        let s = summary(text);
        assert_eq!(s.len(), 3, "{s:?}");
        assert_eq!(s[0]["drawn"], true);
        assert_eq!(s[0]["kind"], "pie");
        assert_eq!(s[1]["drawn"], false);
        assert_eq!(s[2]["why"], "unclosed");
    }

    #[test]
    fn rewrites_only_the_root() {
        let svg = r#"<svg viewBox="0 0 10 5" width="10" height="5" style="--_bg:#fff;color:#000;--_r:4px;font-family:Arial"><rect width="3" height="2"/></svg>"#;
        assert_eq!(for_gpui(svg, 2.), r#"<svg viewBox="0 0 10 5" width="20" height="10" style="color:#000;font-family:Arial"><rect width="3" height="2"/></svg>"#);
    }
}
