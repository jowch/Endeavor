//! What goes along with a chat message besides its words: chips for notebook
//! cells, selected text, error messages and uploaded files, and @ mentions of
//! files in the session's folder. Here: the chips' labels, the prompt blocks a
//! message becomes, reading uploads within their size caps, and the text
//! editing that keeps an @ mention whole.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    ContentBlock, EmbeddedResource, EmbeddedResourceResource, ImageContent, TextContent, TextResourceContents,
};

use crate::annotate::cell_uri;
use crate::session::defined_name;

/// A notebook cell as it was when attached.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub id: String,
    pub code: String,
}

impl Cell {
    /// What the cell defines (`rates = …` → `rates`), else "cell".
    pub fn name(&self) -> String {
        defined_name(&self.code).unwrap_or_else(|| "cell".into())
    }
}

/// What ⌘K or the agent button between cells asked for, besides the user's words.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CellAsk {
    About,
    /// Write the code for this empty cell.
    Fill,
    Before,
    After,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Attachment {
    /// Cells picked with Point, or the cell ⌘K asked about.
    Cells { notebook: String, cells: Vec<Cell>, ask: CellAsk },
    /// Text selected in a cell (the selection chip's "Ask Claude").
    Selection { notebook: String, cell: Cell, text: String },
    /// A cell's error, from Fix with Claude or Explain.
    Error { notebook: String, cell: Cell, text: String },
    /// A box drawn with Point: a PNG of that part of the notebook, and the
    /// cells it overlaps.
    Region { notebook: String, cells: Vec<Cell>, png: Arc<Vec<u8>> },
    Image { name: String, mime: &'static str, bytes: Arc<Vec<u8>> },
    Text { name: String, text: String },
}

/// A chip's label: plain words, then a name in mono (either may be empty).
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub plain: String,
    pub mono: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Cell,
    Cells,
    Selection,
    Error,
    Region,
    Image,
    File,
}

/// Longest name a chip shows before cutting it with "…".
const CHIP_NAME: usize = 24;

fn cut(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let head: String = name.chars().take(max - 1).collect();
    format!("{head}…")
}

fn lines(n: usize) -> String {
    if n == 1 { "1 line".into() } else { format!("{n} lines") }
}

impl Attachment {
    pub fn label(&self) -> Label {
        let label = |plain: &str, mono: &str| Label { plain: plain.into(), mono: cut(mono, CHIP_NAME) };
        match self {
            Attachment::Cells { cells, .. } if cells.len() == 1 => label("", &cells[0].name()),
            Attachment::Cells { cells, .. } => label(&format!("{} cells", cells.len()), ""),
            Attachment::Selection { text, .. } => label(&format!("selection · {}", lines(text.lines().count().max(1))), ""),
            Attachment::Error { cell, .. } => label("error in ", &cell.name()),
            Attachment::Region { .. } => label("region", ""),
            Attachment::Image { name, .. } | Attachment::Text { name, .. } => label("", name),
        }
    }

    pub fn icon(&self) -> Icon {
        match self {
            Attachment::Cells { cells, .. } if cells.len() == 1 => Icon::Cell,
            Attachment::Cells { .. } => Icon::Cells,
            Attachment::Selection { .. } => Icon::Selection,
            Attachment::Error { .. } => Icon::Error,
            Attachment::Region { .. } => Icon::Region,
            Attachment::Image { .. } => Icon::Image,
            Attachment::Text { .. } => Icon::File,
        }
    }

    /// The notebook cells it points at, for "show in notebook".
    pub fn cells(&self) -> Vec<&Cell> {
        match self {
            Attachment::Cells { cells, .. } | Attachment::Region { cells, .. } => cells.iter().collect(),
            Attachment::Selection { cell, .. } | Attachment::Error { cell, .. } => vec![cell],
            _ => Vec::new(),
        }
    }
}

/// The blocks a message sends: what the attachments are (notebook context the
/// app explains, marked "[Endeavor]", then images and text files), then the
/// user's own words. Mentioned files stay in the words as `@path`.
///
/// Each notebook attachment is one text block: a sentence for the agent, then
/// an `<attached>` block with the cells as sent. The session's history keeps
/// the blocks as sent, so a reopened session parses them back into chips
/// (`replayed_notebook`); a region's image is the image block after it.
pub fn prompt_blocks(text: &str, attachments: &[Attachment], mentioned: &[String]) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();
    let note = |s: String| ContentBlock::Text(TextContent::new(s));
    if attachments.iter().any(|a| !a.cells().is_empty()) {
        blocks.push(note(
            "[Endeavor] The user attached notebook cells to this message. Each <cell> below shows a cell's \
             code as it was when attached; its uri, pluto://notebook/{notebook_id}/cell/{cell_id}, names \
             the cell (it is not a fetchable URL). Read its current code and output with the pluto MCP tools."
                .into(),
        ));
    }
    for attachment in attachments {
        if let Some(block) = notebook_block(attachment) {
            blocks.push(note(block));
        }
        match attachment {
            Attachment::Region { png, .. } => blocks.push(ContentBlock::Image(ImageContent::new(base64(png), "image/png"))),
            Attachment::Image { mime, bytes, .. } => blocks.push(ContentBlock::Image(ImageContent::new(base64(bytes), *mime))),
            Attachment::Text { name, text } => blocks.push(ContentBlock::Resource(EmbeddedResource::new(
                EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(text.clone(), format!("attachment:{name}"))),
            ))),
            Attachment::Cells { .. } | Attachment::Selection { .. } | Attachment::Error { .. } => {}
        }
    }
    if !mentioned.is_empty() {
        blocks.push(note(format!(
            "[Endeavor] Paths after @ in the message are files and folders in this session's folder, relative to it: {}. \
             Read them with the tools you use for this session's files.",
            mentioned.join(", ")
        )));
    }
    if !text.is_empty() {
        blocks.push(note(text.to_string()));
    }
    blocks
}

/// A notebook attachment's text block, e.g.
///
/// ```text
/// [Endeavor] The cell below failed with this error.
/// <attached kind="error">
/// <cell uri="pluto://notebook/…/cell/…">
/// fit = curve_fit(model, t, y, p0)
/// </cell>
/// <error>
/// BoundsError
/// </error>
/// </attached>
/// ```
fn notebook_block(attachment: &Attachment) -> Option<String> {
    let region = |n: usize| {
        let overlaps = match n {
            0 => "It overlaps no cells.".to_string(),
            1 => "It overlaps the cell below.".into(),
            n => format!("It overlaps the {n} cells below."),
        };
        format!("The user drew a box over part of the notebook; the image after this shows what was in it. {overlaps}")
    };
    let (kind, sentence, notebook, cells, extra) = match attachment {
        Attachment::Cells { notebook, cells, ask } => {
            let (kind, sentence) = match (ask, cells.len()) {
                (CellAsk::About, 1) => ("cells", "The message is about the cell below.".to_string()),
                (CellAsk::About, n) => ("cells", format!("The message is about the {n} cells below.")),
                (CellAsk::Fill, _) => ("fill", "The cell below is empty: write its code as the message asks.".into()),
                (CellAsk::Before, _) => ("before", "Add a new cell right before the cell below, as the message asks.".into()),
                (CellAsk::After, _) => ("after", "Add a new cell right after the cell below, as the message asks.".into()),
            };
            (kind, sentence, notebook, cells.as_slice(), None)
        }
        Attachment::Region { notebook, cells, .. } => ("region", region(cells.len()), notebook, cells.as_slice(), None),
        Attachment::Selection { notebook, cell, text } => {
            ("selection", "The message is about the text selected in the cell below.".into(), notebook, std::slice::from_ref(cell), Some(text))
        }
        Attachment::Error { notebook, cell, text } => ("error", "The cell below failed with this error.".into(), notebook, std::slice::from_ref(cell), Some(text)),
        Attachment::Image { .. } | Attachment::Text { .. } => return None,
    };
    let mut out = format!("[Endeavor] {sentence}\n<attached kind=\"{kind}\" notebook=\"{notebook}\">\n");
    for cell in cells {
        out += &format!("<cell uri=\"{}\">\n{}\n</cell>\n", cell_uri(notebook, &cell.id), cell.code);
    }
    if let Some(text) = extra {
        out += &format!("<{kind}>\n{text}\n</{kind}>\n");
    }
    out += "</attached>";
    Some(out)
}

/// A notebook attachment back from its text block (see `notebook_block`). A
/// region comes back without its image, which is the next block.
pub fn replayed_notebook(text: &str) -> Option<Attachment> {
    let rest = text.trim().strip_prefix("[Endeavor] ")?;
    let (_, rest) = rest.split_once("\n<attached kind=\"")?;
    let (kind, rest) = rest.split_once("\" notebook=\"")?;
    let (notebook, mut rest) = rest.split_once("\">\n")?;
    let notebook = notebook.to_string();
    let mut cells = Vec::new();
    while let Some(after) = rest.strip_prefix("<cell uri=\"") {
        let (uri, after) = after.split_once("\">\n")?;
        let (code, after) = element_body(after, "cell")?;
        let id = uri.strip_prefix(&format!("pluto://notebook/{notebook}/cell/"))?;
        cells.push(Cell { id: id.into(), code: code.into() });
        rest = after;
    }
    let mut extra = None;
    if let Some(after) = rest.strip_prefix(&format!("<{kind}>\n")) {
        let (body, after) = element_body(after, kind)?;
        extra = Some(body.to_string());
        rest = after;
    }
    if rest != "</attached>" {
        return None;
    }
    let only = |cells: Vec<Cell>| -> Option<Cell> { if cells.len() == 1 { cells.into_iter().next() } else { None } };
    Some(match kind {
        "cells" => Attachment::Cells { notebook, cells, ask: CellAsk::About },
        "fill" => Attachment::Cells { notebook, cells, ask: CellAsk::Fill },
        "before" => Attachment::Cells { notebook, cells, ask: CellAsk::Before },
        "after" => Attachment::Cells { notebook, cells, ask: CellAsk::After },
        "selection" => Attachment::Selection { notebook, cell: only(cells)?, text: extra? },
        "error" => Attachment::Error { notebook, cell: only(cells)?, text: extra? },
        "region" => Attachment::Region { notebook, cells, png: Arc::new(Vec::new()) },
        _ => return None,
    })
}

/// `body\n</tag>\n…` → (body, …).
fn element_body<'a>(text: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let end = format!("\n</{tag}>\n");
    let at = text.find(&end)?;
    Some((&text[..at], &text[at + end.len()..]))
}

/// Text the app adds to a message, which a replayed session doesn't show as
/// the user's words: its "[Endeavor]" notes, cell links, and how the agent
/// echoes an attached text file.
pub fn is_app_text(text: &str) -> bool {
    text.starts_with("[Endeavor]") || text.starts_with("pluto://") || text.starts_with("attachment:") || text.trim_start().starts_with("<context ref=")
}

/// An attached text file as a replayed session gives it back: the agent
/// echoes it as `<context ref="attachment:name">…</context>`.
pub fn replayed_text_file(text: &str) -> Option<Attachment> {
    let rest = text.trim_start().strip_prefix("<context ref=\"attachment:")?;
    let (name, body) = rest.split_once("\">\n")?;
    let body = body.strip_suffix("\n</context>").unwrap_or(body);
    Some(Attachment::Text { name: name.into(), text: body.into() })
}

/// An attached image as a replayed session gives it back (its name is lost).
pub fn replayed_image(data: &str, mime: &str) -> Option<Attachment> {
    let mime = ["image/png", "image/jpeg", "image/gif", "image/webp"].into_iter().find(|m| *m == mime)?;
    let name = format!("image.{}", mime.trim_start_matches("image/").replace("jpeg", "jpg"));
    Some(Attachment::Image { name, mime, bytes: Arc::new(unbase64(data)?) })
}

// ---------------------------------------------------------------------------
// Uploads
// ---------------------------------------------------------------------------

/// The Claude API takes images up to 5 MB once base64-encoded (4/3 larger).
pub const IMAGE_MAX: u64 = 3_700_000;
pub const TEXT_MAX: u64 = 250_000;

fn image_type(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    })
}

/// "1.2 MB", "340 KB", "12 bytes".
pub fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000 => format!("{:.1} MB", b as f64 / 1_000_000.),
        b if b >= 1_000 => format!("{} KB", b / 1_000),
        b => format!("{b} bytes"),
    }
}

/// A file picked with "+" (or dropped, or pasted) as an attachment, or why it
/// can't be one, in plain words.
pub fn upload(name: &str, bytes: Vec<u8>) -> Result<Attachment, String> {
    too_big(name, bytes.len() as u64)?;
    if let Some(mime) = image_type(name) {
        return Ok(Attachment::Image { name: name.into(), mime, bytes: Arc::new(bytes) });
    }
    match String::from_utf8(bytes).ok().filter(|t| !t.contains('\0')) {
        Some(text) => Ok(Attachment::Text { name: name.into(), text }),
        None => Err(format!("{name} isn't text or a PNG, JPEG, GIF or WebP image, so it can't be attached yet.")),
    }
}

fn too_big(name: &str, size: u64) -> Result<(), String> {
    let (kind, cap) = if image_type(name).is_some() { ("images", IMAGE_MAX) } else { ("text files", TEXT_MAX) };
    if size > cap {
        return Err(format!("{name} is {}; {kind} can be up to {}.", size_text(size), size_text(cap)));
    }
    Ok(())
}

/// Read a picked file into an attachment (a file over its cap isn't read).
pub fn upload_file(path: &Path) -> Result<Attachment, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let meta = std::fs::metadata(path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
    if meta.is_dir() {
        return Err(format!("{name} is a folder; attach the files in it instead."));
    }
    too_big(&name, meta.len())?;
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
    upload(&name, bytes)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unbase64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let digits: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').map(value).collect::<Option<_>>()?;
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &d)| n | (d as u32) << (18 - 6 * i));
        out.extend((0..chunk.len().saturating_sub(1)).map(|i| (n >> (16 - 8 * i)) as u8));
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// @ mentions
// ---------------------------------------------------------------------------

/// How a mentioned path reads in the text: `@data/decay.csv`, quoted if it has spaces.
pub fn token(path: &str) -> String {
    if path.contains(char::is_whitespace) { format!("@\"{path}\"") } else { format!("@{path}") }
}

/// The "@query" being typed just before `cursor` (a byte offset): its range,
/// "@" included, and the query. The "@" starts the text or follows a space.
pub fn typing_mention(text: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
    let before = text.get(..cursor)?;
    let at = before.rfind('@')?;
    let query = &before[at + 1..];
    let starts_word = before[..at].chars().next_back().is_none_or(char::is_whitespace);
    (starts_word && !query.contains(char::is_whitespace) && !query.starts_with('"')).then_some((at..cursor, query))
}

/// Where each mention's token is in `text`: a token counts only as a whole
/// word, followed by the end, a space or punctuation.
pub fn token_ranges(text: &str, mentions: &[String]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    for path in mentions {
        let token = token(path);
        let mut from = 0;
        while let Some(i) = text[from..].find(&token).map(|i| i + from) {
            let end = i + token.len();
            let before_ok = text[..i].chars().next_back().is_none_or(char::is_whitespace);
            let after_ok = text[end..].chars().next().is_none_or(|c| c.is_whitespace() || ",.;:!?)".contains(c));
            if before_ok && after_ok {
                ranges.push(i..end);
            }
            from = end;
        }
    }
    ranges.sort_by_key(|r| r.start);
    ranges
}

/// The mentions still in `text`, in the order they appear.
pub fn mentioned(text: &str, mentions: &[String]) -> Vec<String> {
    let mut found: Vec<(usize, String)> = mentions
        .iter()
        .filter_map(|path| token_ranges(text, std::slice::from_ref(path)).first().map(|r| (r.start, path.clone())))
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// An edit that deleted part of a mention deletes all of it, so one backspace
/// removes `@data/decay.csv`. Given the text before and after an edit, returns
/// the text with the rest of any partly deleted token removed, and where the
/// cursor goes; None when no token was cut.
pub fn keep_tokens_whole(old: &str, new: &str, mentions: &[String]) -> Option<(String, usize)> {
    if new.len() >= old.len() {
        return None;
    }
    let prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    let prefix = (0..=prefix).rev().find(|&i| old.is_char_boundary(i) && new.is_char_boundary(i))?;
    let max_suffix = new.len() - prefix;
    let suffix = old.bytes().rev().zip(new.bytes().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
    let suffix = (0..=suffix).rev().find(|&s| old.is_char_boundary(old.len() - s) && new.is_char_boundary(new.len() - s))?;
    // Only pure deletions: nothing new was typed in their place.
    if prefix + suffix != new.len() {
        return None;
    }
    let deleted = prefix..old.len() - suffix;
    let cut: Vec<Range<usize>> = token_ranges(old, mentions)
        .into_iter()
        .filter(|t| deleted.start < t.end && t.start < deleted.end && !(deleted.start <= t.start && t.end <= deleted.end))
        .collect();
    if cut.is_empty() {
        return None;
    }
    let start = cut.iter().map(|t| t.start).min()?.min(deleted.start);
    let end = cut.iter().map(|t| t.end).max()?.max(deleted.end);
    Some((format!("{}{}", &old[..start], &old[end..]), start))
}

/// Paths matching what's typed after "@", best first: the letters in order
/// (not necessarily together); a match in the file's own name, at its start,
/// or in one piece ranks higher, and so do shorter paths. At most `max`.
pub fn fuzzy<'a>(query: &str, paths: &'a [String], max: usize) -> Vec<&'a str> {
    let query: Vec<char> = query.to_lowercase().chars().collect();
    let mut scored: Vec<(i64, &str)> = paths
        .iter()
        .filter_map(|path| {
            let lower = path.to_lowercase();
            let chars: Vec<char> = lower.chars().collect();
            let find = |mut at: usize| {
                let mut positions = Vec::with_capacity(query.len());
                for q in &query {
                    let found = chars[at..].iter().position(|c| c == q)? + at;
                    positions.push(found);
                    at = found + 1;
                }
                Some(positions)
            };
            let trimmed = lower.trim_end_matches('/');
            let name_start = trimmed.rfind('/').map_or(0, |i| trimmed[..=i].chars().count());
            let positions = find(name_start).or_else(|| find(0))?;
            let mut score = -(chars.len() as i64);
            if let Some(&first) = positions.first() {
                if first >= name_start {
                    score += 100;
                }
                if first == name_start {
                    score += 100;
                }
                score -= (positions.last().unwrap() - first) as i64 * 2;
            }
            Some((score, path.as_str()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(max).map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NB: &str = "6a1b2c3d-0000-4000-8000-1234567890ab";

    fn cell(id: &str, code: &str) -> Cell {
        Cell { id: id.into(), code: code.into() }
    }

    fn texts(blocks: &[ContentBlock]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match b {
                ContentBlock::Text(t) => t.text.clone(),
                ContentBlock::ResourceLink(l) => format!("link {} {}", l.name, l.uri),
                ContentBlock::Image(i) => format!("image {} {}", i.mime_type, i.data),
                ContentBlock::Resource(r) => match &r.resource {
                    EmbeddedResourceResource::TextResourceContents(t) => format!("resource {} {}", t.uri, t.text),
                    _ => "blob".into(),
                },
                _ => "other".into(),
            })
            .collect()
    }

    #[test]
    fn chip_labels() {
        let rates = cell("c1", "rates = map(1:1000) do _\n  fit()\nend");
        let plot = cell("c2", "scatter(data.t, data.counts)");
        let one = Attachment::Cells { notebook: NB.into(), cells: vec![rates.clone()], ask: CellAsk::About };
        assert_eq!(one.label(), Label { plain: "".into(), mono: "rates".into() });
        let three = Attachment::Cells { notebook: NB.into(), cells: vec![rates.clone(), plot.clone(), rates.clone()], ask: CellAsk::About };
        assert_eq!(three.label(), Label { plain: "3 cells".into(), mono: "".into() });
        assert_eq!(Attachment::Cells { notebook: NB.into(), cells: vec![plot.clone()], ask: CellAsk::About }.label().mono, "cell");
        let selection = Attachment::Selection { notebook: NB.into(), cell: rates.clone(), text: "a\nb".into() };
        assert_eq!(selection.label().plain, "selection · 2 lines");
        let error = Attachment::Error { notebook: NB.into(), cell: rates, text: "BoundsError".into() };
        assert_eq!(error.label(), Label { plain: "error in ".into(), mono: "rates".into() });
        assert_eq!(error.icon(), Icon::Error);
        let long = Attachment::Text { name: "a_really_long_file_name_for_a_chip.csv".into(), text: String::new() };
        assert_eq!(long.label().mono, "a_really_long_file_name…");
    }

    #[test]
    fn prompt_blocks_explain_attachments_then_carry_the_words() {
        let fit = cell("c1", "fit = curve_fit(model, t, y, p0)");
        let attachments = [
            Attachment::Error { notebook: NB.into(), cell: fit.clone(), text: "BoundsError".into() },
            Attachment::Image { name: "gel.png".into(), mime: "image/png", bytes: Arc::new(b"hi!".to_vec()) },
            Attachment::Text { name: "notes.txt".into(), text: "t,y\n1,2".into() },
        ];
        let blocks = prompt_blocks("Fix the error in this cell.", &attachments, &["data/decay.csv".into()]);
        let all = texts(&blocks);
        assert!(all[0].starts_with("[Endeavor] The user attached notebook cells"));
        assert_eq!(
            all[1..],
            [
                format!(
                    "[Endeavor] The cell below failed with this error.\n<attached kind=\"error\" notebook=\"{NB}\">\n\
                     <cell uri=\"pluto://notebook/{NB}/cell/c1\">\nfit = curve_fit(model, t, y, p0)\n</cell>\n\
                     <error>\nBoundsError\n</error>\n</attached>"
                ),
                "image image/png aGkh".into(),
                "resource attachment:notes.txt t,y\n1,2".into(),
                "[Endeavor] Paths after @ in the message are files and folders in this session's folder, relative to it: \
                 data/decay.csv. Read them with the tools you use for this session's files."
                    .into(),
                "Fix the error in this cell.".into(),
            ]
        );
        assert_eq!(self::texts(&prompt_blocks("hi", &[], &[])), ["hi"], "a plain message is just its words");
        let fill = Attachment::Cells { notebook: NB.into(), cells: vec![cell("c2", "")], ask: CellAsk::Fill };
        assert_eq!(
            self::texts(&prompt_blocks("plot it", &[fill], &[]))[1],
            format!(
                "[Endeavor] The cell below is empty: write its code as the message asks.\n<attached kind=\"fill\" notebook=\"{NB}\">\n\
                 <cell uri=\"pluto://notebook/{NB}/cell/c2\">\n\n</cell>\n</attached>"
            )
        );
    }

    #[test]
    fn notebook_blocks_parse_back_as_sent() {
        let attachments = [
            Attachment::Cells { notebook: NB.into(), cells: vec![cell("c1", "rates = 1"), cell("c2", "")], ask: CellAsk::About },
            Attachment::Cells { notebook: NB.into(), cells: vec![cell("c3", "")], ask: CellAsk::Before },
            Attachment::Selection { notebook: NB.into(), cell: cell("c1", "s = sum(xs)\n"), text: "sum(xs)".into() },
            // Code that looks like the block's own tags stays code.
            Attachment::Error { notebook: NB.into(), cell: cell("c4", "html\"<cell uri=\\\"x\\\">\" # </error>"), text: "LoadError:\n  in expression".into() },
            Attachment::Region { notebook: NB.into(), cells: Vec::new(), png: Arc::new(Vec::new()) },
        ];
        for attachment in attachments {
            let block = notebook_block(&attachment).unwrap();
            assert_eq!(replayed_notebook(&block), Some(attachment), "{block}");
        }
        assert_eq!(replayed_notebook("[Endeavor] The user is viewing Pluto notebook x."), None);
        assert_eq!(replayed_notebook("<attached kind=\"cells\">"), None, "only the app's own blocks");
    }

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xff, 0xfe, 0x00, 0x01]), "//4AAQ==");
        for bytes in [&b""[..], b"f", b"fo", b"foo", &[0xff, 0xfe, 0x00, 0x01]] {
            assert_eq!(unbase64(&base64(bytes)).as_deref(), Some(bytes));
        }
        assert_eq!(unbase64("no!"), None);
    }

    #[test]
    fn replayed_attachments_come_back_as_chips() {
        let echoed = "\n<context ref=\"attachment:notes.txt\">\nt,y\n1,2\n</context>";
        assert_eq!(replayed_text_file(echoed), Some(Attachment::Text { name: "notes.txt".into(), text: "t,y\n1,2".into() }));
        assert_eq!(replayed_text_file("just words"), None);
        assert_eq!(replayed_image("aGkh", "image/jpeg"), Some(Attachment::Image { name: "image.jpg".into(), mime: "image/jpeg", bytes: Arc::new(b"hi!".to_vec()) }));
        assert_eq!(replayed_image("aGkh", "image/tiff"), None);
    }

    #[test]
    fn uploads_are_images_or_text_within_caps() {
        assert!(matches!(upload("Gel Scan.PNG", vec![1, 2, 3]), Ok(Attachment::Image { mime: "image/png", .. })));
        assert!(matches!(upload("fit.jl", b"x = 1".to_vec()), Ok(Attachment::Text { text, .. }) if text == "x = 1"));
        assert_eq!(upload("big.jpg", vec![0; 5_200_000]), Err("big.jpg is 5.2 MB; images can be up to 3.7 MB.".into()));
        assert_eq!(upload("log.txt", vec![b'a'; 300_000]), Err("log.txt is 300 KB; text files can be up to 250 KB.".into()));
        assert_eq!(
            upload("paper.pdf", vec![0x25, 0x50, 0x00, 0xff]),
            Err("paper.pdf isn't text or a PNG, JPEG, GIF or WebP image, so it can't be attached yet.".into())
        );
        assert_eq!(size_text(12), "12 bytes");
        assert_eq!(size_text(1_234_567), "1.2 MB");
    }

    #[test]
    fn replayed_app_text_is_recognised() {
        assert!(is_app_text("[Endeavor] The user is viewing"));
        assert!(is_app_text("pluto://notebook/x/cell/y"));
        assert!(is_app_text("\n<context ref=\"attachment:notes.txt\">\nx\n</context>"));
        assert!(!is_app_text("why do these bunch up?"));
    }

    #[test]
    fn typing_a_mention() {
        assert_eq!(typing_mention("see @da", 7), Some((4..7, "da")));
        assert_eq!(typing_mention("@", 1), Some((0..1, "")));
        assert_eq!(typing_mention("mail me@home", 12), None, "not a word start");
        assert_eq!(typing_mention("@data/x.csv then", 16), None, "a space ends it");
        assert_eq!(typing_mention("see @da and more", 7), Some((4..7, "da")), "the cursor, not the end");
    }

    #[test]
    fn tokens_are_whole_words() {
        let mentions = vec!["data/decay.csv".to_string(), "my notes.txt".to_string()];
        let text = "compare @data/decay.csv, @\"my notes.txt\" and @data/decay.csvx";
        assert_eq!(token_ranges(text, &mentions), [8..23, 25..40]);
        assert_eq!(mentioned(text, &mentions), ["data/decay.csv", "my notes.txt"]);
        assert_eq!(mentioned("nothing here", &mentions), Vec::<String>::new());
    }

    #[test]
    fn one_backspace_deletes_a_whole_mention() {
        let mentions = vec!["data/decay.csv".to_string()];
        let old = "fit @data/decay.csv now";
        // Backspace at the token's end.
        assert_eq!(keep_tokens_whole(old, "fit @data/decay.cs now", &mentions), Some(("fit  now".into(), 4)));
        // Forward delete at its start.
        assert_eq!(keep_tokens_whole(old, "fit data/decay.csv now", &mentions), Some(("fit  now".into(), 4)));
        // Deleting a selection that covers it whole is already whole.
        assert_eq!(keep_tokens_whole(old, "fit  now", &mentions), None);
        // Edits elsewhere, and typing, are left alone.
        assert_eq!(keep_tokens_whole(old, "fi @data/decay.csv now", &mentions), None);
        assert_eq!(keep_tokens_whole(old, "fit @data/decay.csv now!", &mentions), None);
        // A selection from inside the token to past it takes the whole token.
        assert_eq!(keep_tokens_whole(old, "fit @data/dnow", &mentions), Some(("fit now".into(), 4)));
    }

    #[test]
    fn fuzzy_ranks_name_matches_first() {
        let paths: Vec<String> = ["data/", "data/decay.csv", "data/raw/decay_old.csv", "docs/design.md", "fit_decay.jl", "README.md"].map(String::from).to_vec();
        assert_eq!(fuzzy("decay", &paths, 3), ["data/decay.csv", "data/raw/decay_old.csv", "fit_decay.jl"]);
        assert_eq!(fuzzy("dd", &paths, 10), ["docs/design.md", "data/raw/decay_old.csv", "README.md", "data/decay.csv"]);
        assert_eq!(fuzzy("", &paths, 2), ["data/", "README.md"]);
        assert!(fuzzy("zzz", &paths, 5).is_empty());
    }
}
