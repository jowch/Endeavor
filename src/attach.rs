use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    ContentBlock, EmbeddedResource, EmbeddedResourceResource, ImageContent, TextContent, TextResourceContents,
};

use wire::files::{self, DATA, Reply, Request, numbered};

use crate::annotate::{cell_uri, uri_cell};
use wire::backend::Backend;

use crate::session::cell_label;

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub id: String,
    pub code: String,
}

impl Cell {
    pub fn name(&self) -> String {
        cell_label(&self.code)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CellAsk {
    About,
    Fill,
    Before,
    After,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Attachment {
    Cells { notebook: String, cells: Vec<Cell>, ask: CellAsk },
    Error { notebook: String, cell: Cell, text: String },
    Quote(Quote),
    Image { name: String, mime: &'static str, bytes: Arc<Vec<u8>> },
    Text { name: String, text: String },
    Upload { source: PathBuf, size: u64 },
    Saved { path: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    pub from: Quoted,
    pub comment: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Quoted {
    Reply { text: String, at: Option<String> },
    Cell { notebook: String, cell: String, name: String, part: Part },
    Box { notebook: String, cells: Vec<String>, png: Arc<Vec<u8>> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Whole(String),
    Lines { first: usize, last: usize, text: String },
    Output(String),
    WholeOutput(String),
    Figure(Arc<Vec<u8>>),
}

impl Part {
    pub fn name(&self) -> String {
        match self {
            Part::Whole(_) => "whole cell".into(),
            Part::Lines { first, last, .. } if first == last => format!("line {first}"),
            Part::Lines { first, last, .. } => format!("lines {first}–{last}"),
            Part::Output(_) | Part::WholeOutput(_) => "output".into(),
            Part::Figure(_) => "figure".into(),
        }
    }
}

impl Quote {
    pub fn source(&self) -> String {
        match &self.from {
            Quoted::Reply { at: Some(at), .. } => format!("Claude's reply · {at}"),
            Quoted::Reply { at: None, .. } => "Claude's reply".into(),
            Quoted::Cell { name, part, .. } => format!("{name} · {}", part.name()),
            Quoted::Box { cells, .. } if cells.len() == 1 => "Box · 1 cell".into(),
            Quoted::Box { cells, .. } => format!("Box · {} cells", cells.len()),
        }
    }

    pub fn icon(&self) -> Icon {
        match &self.from {
            Quoted::Reply { .. } => Icon::Quote,
            Quoted::Cell { part: Part::Figure(_), .. } => Icon::Image,
            Quoted::Cell { .. } => Icon::Cell,
            Quoted::Box { .. } => Icon::Region,
        }
    }

    pub fn label(&self, max: usize) -> Label {
        match &self.from {
            Quoted::Cell { name, part, .. } => Label { plain: String::new(), mono: cut(name, max), after: format!("· {}", part.name()) },
            _ => Label { plain: self.source(), mono: String::new(), after: String::new() },
        }
    }

    pub fn excerpt(&self) -> Option<&str> {
        match &self.from {
            Quoted::Reply { text, .. } => Some(text.as_str()),
            Quoted::Cell { part: Part::Whole(text) | Part::Lines { text, .. } | Part::Output(text) | Part::WholeOutput(text), .. } => Some(text.as_str()).filter(|t| !t.is_empty()),
            Quoted::Cell { part: Part::Figure(_), .. } | Quoted::Box { .. } => None,
        }
    }

    pub fn first_line(&self) -> Option<usize> {
        match &self.from {
            Quoted::Cell { part: Part::Lines { first, .. }, .. } => Some(*first),
            Quoted::Cell { part: Part::Whole(_), .. } => Some(1),
            _ => None,
        }
    }

    pub fn picture(&self) -> Option<&Arc<Vec<u8>>> {
        match &self.from {
            Quoted::Cell { part: Part::Figure(png), .. } | Quoted::Box { png, .. } => Some(png).filter(|p| !p.is_empty()),
            _ => None,
        }
    }

    pub fn cells(&self) -> Vec<String> {
        match &self.from {
            Quoted::Reply { .. } => Vec::new(),
            Quoted::Cell { cell, .. } => vec![cell.clone()],
            Quoted::Box { cells, .. } => cells.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub plain: String,
    pub mono: String,
    pub after: String,
}

impl Label {
    pub fn text(&self) -> String {
        [&self.plain, &self.mono, &self.after].into_iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Cell,
    Cells,
    Quote,
    Error,
    Region,
    Image,
    File,
}

pub const CHIP_NAME: usize = 24;

fn cut(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let head: String = name.chars().take(max - 1).collect();
    format!("{head}…")
}

impl Attachment {
    pub fn label(&self) -> Label {
        self.label_cut_at(CHIP_NAME)
    }

    pub fn full_label(&self) -> Label {
        self.label_cut_at(usize::MAX)
    }

    fn label_cut_at(&self, max: usize) -> Label {
        let label = |plain: &str, mono: &str| Label { plain: plain.into(), mono: cut(mono, max), after: String::new() };
        match self {
            Attachment::Cells { cells, .. } if cells.len() == 1 => label("", &cells[0].name()),
            Attachment::Cells { cells, .. } => label(&format!("{} cells", cells.len()), ""),
            Attachment::Error { cell, .. } => label("error in ", &cell.name()),
            Attachment::Quote(quote) => quote.label(max),
            Attachment::Image { name, .. } | Attachment::Text { name, .. } => label("", name),
            Attachment::Upload { source, .. } => label("", &data_path(&file_name(source))),
            Attachment::Saved { path } => label("", path),
        }
    }

    pub fn icon(&self) -> Icon {
        match self {
            Attachment::Cells { cells, .. } if cells.len() == 1 => Icon::Cell,
            Attachment::Cells { .. } => Icon::Cells,
            Attachment::Error { .. } => Icon::Error,
            Attachment::Quote(quote) => quote.icon(),
            Attachment::Image { .. } => Icon::Image,
            Attachment::Text { .. } | Attachment::Upload { .. } | Attachment::Saved { .. } => Icon::File,
        }
    }

    pub fn cells(&self) -> Vec<&Cell> {
        match self {
            Attachment::Cells { cells, .. } => cells.iter().collect(),
            Attachment::Error { cell, .. } => vec![cell],
            _ => Vec::new(),
        }
    }
}

pub fn prompt_blocks(text: &str, attachments: &[Attachment], mentioned: &[String], kind: Backend) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();
    let note = |s: String| ContentBlock::Text(TextContent::new(s));
    let command = crate::slash::is_command(text);
    if command {
        blocks.push(note(text.to_string()));
    }
    if attachments.iter().any(|a| !a.cells().is_empty()) {
        blocks.push(note(format!(
            "[Endeavor] The user attached notebook cells to this message. Each <cell> below shows a cell's \
             code as it was when attached; its uri, notebook://{}/{{notebook_id}}/cell/{{cell_id}}, names \
             the cell (it is not a fetchable URL). Read its current code and output with the notebook MCP tools.",
            kind.name()
        )));
    }
    if attachments.iter().any(|a| matches!(a, Attachment::Quote(Quote { from: Quoted::Cell { .. } | Quoted::Box { .. }, .. }))) {
        blocks.push(note(quote_note(kind)));
    }
    for attachment in attachments {
        if let Some(block) = notebook_block(attachment, kind) {
            blocks.push(note(block));
        }
        match attachment {
            Attachment::Quote(quote) => {
                blocks.push(note(quote_block(quote, kind)));
                if let (Quoted::Box { .. }, Some(png)) = (&quote.from, quote.picture()) {
                    blocks.push(ContentBlock::Image(ImageContent::new(base64(png), "image/png")));
                }
            }
            Attachment::Image { mime, bytes, .. } => blocks.push(ContentBlock::Image(ImageContent::new(base64(bytes), *mime))),
            Attachment::Text { name, text } => blocks.push(ContentBlock::Resource(EmbeddedResource::new(
                EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(text.clone(), format!("attachment:{name}"))),
            ))),
            Attachment::Saved { path } => blocks.push(note(format!("{SAVED_NOTE}{path}"))),
            Attachment::Upload { .. } => {}
            Attachment::Cells { .. } | Attachment::Error { .. } => {}
        }
    }
    if !mentioned.is_empty() {
        blocks.push(note(format!(
            "[Endeavor] Paths after @ in the message are files and folders in this session's folder, relative to it: {}. \
             Read them with the tools you use for this session's files.",
            mentioned.join(", ")
        )));
    }
    if !text.is_empty() && !command {
        blocks.push(note(text.to_string()));
    }
    blocks
}

fn notebook_block(attachment: &Attachment, backend: Backend) -> Option<String> {
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
        Attachment::Error { notebook, cell, text } => ("error", "The cell below failed with this error.".into(), notebook, std::slice::from_ref(cell), Some(text)),
        Attachment::Quote(_) | Attachment::Image { .. } | Attachment::Text { .. } | Attachment::Upload { .. } | Attachment::Saved { .. } => return None,
    };
    let mut out = format!("[Endeavor] {sentence}\n<attached kind=\"{kind}\" notebook=\"{notebook}\">\n");
    for cell in cells {
        out += &format!("<cell uri=\"{}\">\n{}\n</cell>\n", cell_uri(backend, notebook, &cell.id), cell.code);
    }
    if let Some(text) = extra {
        out += &format!("<{kind}>\n{text}\n</{kind}>\n");
    }
    out += "</attached>";
    Some(out)
}

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
        let id = uri_cell(uri, &notebook)?;
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
        "error" => Attachment::Error { notebook, cell: only(cells)?, text: extra? },
        "selection" => {
            let cell = only(cells)?;
            Attachment::Quote(Quote { from: Quoted::Cell { notebook, name: cell.name(), cell: cell.id, part: Part::Output(extra?) }, comment: String::new() })
        }
        "region" => Attachment::Quote(Quote {
            from: Quoted::Box { notebook, cells: cells.into_iter().map(|c| c.id).collect(), png: Arc::new(Vec::new()) },
            comment: String::new(),
        }),
        _ => return None,
    })
}

fn quote_note(kind: Backend) -> String {
    format!(
        "[Endeavor] The user quoted parts of the notebook in this message, each as a <quote> \
    followed by their comment on it. A quote's uri, notebook://{}/{{notebook_id}}/cell/{{cell_id}}, names its cell \
    (it is not a fetchable URL); `lines` are line numbers in the cell's code, counting from 1. `part` says what was \
    quoted when it isn't code: `output` (the cell's output), `figure` (the cell's output, a picture) or `box` (a box \
    the user drew over the notebook; the image after the quote shows what was in it, and `uri` lists the cells under \
    it). A quote with a body holds what the user selected or the lines they picked, as it was then; the others \
    hold no code or output, only the name of what was picked. Read \
    what the cells hold now with the notebook MCP tools (read_cell, view_cell_output).",
        kind.name()
    )
}

const FROM_REPLY: &str = "> — Claude's reply";

pub fn quote_block(quote: &Quote, backend: Backend) -> String {
    let (quoted, gap) = match &quote.from {
        Quoted::Reply { text, at } => {
            let mut out: String = text.lines().map(|l| if l.is_empty() { ">\n".to_string() } else { format!("> {l}\n") }).collect();
            out += FROM_REPLY;
            if let Some(at) = at {
                out += &format!(", {at}");
            }
            (out, "\n\n")
        }
        Quoted::Cell { notebook, cell, name, part } => {
            let uri = cell_uri(backend, notebook, cell);
            let name = escaped_attr(name);
            let out = match part {
                Part::Whole(_) => format!("<quote cell=\"{name}\" uri=\"{uri}\"/>"),
                Part::Lines { first, last, text } => format!("<quote cell=\"{name}\" lines=\"{first}-{last}\" uri=\"{uri}\">\n{text}\n</quote>"),
                Part::Output(text) => format!("<quote cell=\"{name}\" part=\"output\" uri=\"{uri}\">\n{text}\n</quote>"),
                Part::WholeOutput(_) => format!("<quote cell=\"{name}\" part=\"output\" uri=\"{uri}\"/>"),
                Part::Figure(_) => format!("<quote cell=\"{name}\" part=\"figure\" uri=\"{uri}\"/>"),
            };
            (out, "\n")
        }
        Quoted::Box { notebook, cells, .. } => {
            let uris: Vec<String> = cells.iter().map(|c| cell_uri(backend, notebook, c)).collect();
            (format!("<quote part=\"box\" uri=\"{}\"/>", uris.join(" ")), "\n")
        }
    };
    if quote.comment.is_empty() { quoted } else { format!("{quoted}{gap}{}", quote.comment) }
}

pub fn replayed_quote(block: &str) -> Option<Attachment> {
    let from_reply = |block: &str| {
        let (quoted, comment) = block.split_once("\n\n").unwrap_or((block, ""));
        let (lines, from) = quoted.rsplit_once('\n').unwrap_or(("", quoted));
        let at = match from.strip_prefix(FROM_REPLY)? {
            "" => None,
            rest => Some(rest.strip_prefix(", ")?.to_string()),
        };
        let text: Option<Vec<&str>> = lines.lines().map(|l| l.strip_prefix("> ").or(l.strip_prefix('>'))).collect();
        Some(Quote { from: Quoted::Reply { text: text?.join("\n"), at }, comment: comment.into() })
    };
    let from_notebook = |block: &str| {
        let rest = block.strip_prefix("<quote ")?;
        let (attrs, closed, rest) = match (rest.find("/>"), rest.find(">\n")) {
            (Some(end), next) if next.is_none_or(|n| end < n) => (&rest[..end], true, &rest[end + 2..]),
            (_, Some(end)) => (&rest[..end], false, &rest[end + 2..]),
            _ => return None,
        };
        let attr = |key: &str| attrs.split_once(&format!(" {key}=\"")).or_else(|| attrs.strip_prefix(&format!("{key}=\"")).map(|v| ("", v))).and_then(|(_, v)| v.split_once('"')).map(|(v, _)| v);
        let (body, comment) = if closed {
            (None, rest)
        } else {
            const END: &str = "\n</quote>";
            let at = rest.match_indices(END).map(|(at, _)| at).find(|&at| matches!(rest[at + END.len()..].chars().next(), None | Some('\n')))?;
            (Some(&rest[..at]), &rest[at + END.len()..])
        };
        let comment = match comment {
            "" => "",
            c => c.strip_prefix('\n')?,
        };
        let notebook_of = |uri: &str| uri.strip_prefix("notebook://")?.split_once('/')?.1.split_once("/cell/").map(|(nb, cell)| (nb.to_string(), cell.to_string()));
        let from = match (attr("part"), body) {
            (Some("box"), None) => {
                let found: Option<Vec<(String, String)>> = attr("uri")?.split_whitespace().map(notebook_of).collect();
                let found = found?;
                Quoted::Box { notebook: found.first().map(|(nb, _)| nb.clone()).unwrap_or_default(), cells: found.into_iter().map(|(_, c)| c).collect(), png: Arc::new(Vec::new()) }
            }
            (part, body) => {
                let (notebook, cell) = notebook_of(attr("uri")?)?;
                let part = match (part, attr("lines"), body) {
                    (Some("figure"), _, None) => Part::Figure(Arc::new(Vec::new())),
                    (Some("output"), _, None) => Part::WholeOutput(String::new()),
                    (Some("output"), _, Some(text)) => Part::Output(text.into()),
                    (None, None, None) => Part::Whole(String::new()),
                    (None, Some(lines), Some(text)) => {
                        let (first, last) = lines.split_once('-')?;
                        Part::Lines { first: first.parse().ok()?, last: last.parse().ok()?, text: text.into() }
                    }
                    (None, None, Some(code)) => Part::Whole(code.into()),
                    _ => return None,
                };
                Quoted::Cell { notebook, cell, name: unescaped_attr(attr("cell")?), part }
            }
        };
        Some(Quote { from, comment: comment.into() })
    };
    from_notebook(block).or_else(|| from_reply(block)).map(Attachment::Quote)
}

pub fn take_staged_quotes(attachments: &mut Vec<Attachment>) -> Vec<Attachment> {
    attachments.extract_if(.., |a| matches!(a, Attachment::Quote(_))).collect()
}

fn escaped_attr(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unescaped_attr(value: &str) -> String {
    value.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn element_body<'a>(text: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let end = format!("\n</{tag}>\n");
    let at = text.find(&end)?;
    Some((&text[..at], &text[at + end.len()..]))
}

pub fn is_app_text(text: &str) -> bool {
    text.starts_with("[Endeavor]") || text.starts_with("notebook://") || text.starts_with("pluto://") || text.starts_with("attachment:") || text.trim_start().starts_with("<context ref=")
}

pub fn is_stopped_marker(text: &str) -> bool {
    text == "[Request interrupted by user]" || text == "[Request interrupted by user for tool use]"
}

pub fn without_agent_blocks(text: &str) -> String {
    ["system-reminder", "task-notification"].iter().fold(text.to_string(), |text, tag| {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        let mut out = String::new();
        let mut rest = text.as_str();
        while let Some(start) = rest.find(&open) {
            out.push_str(&rest[..start]);
            rest = rest[start..].find(&close).map_or("", |end| &rest[start + end + close.len()..]);
        }
        out.push_str(rest);
        out
    })
    .trim()
    .to_string()
}

pub fn replayed_text_file(text: &str) -> Option<Attachment> {
    let rest = text.trim_start().strip_prefix("<context ref=\"attachment:")?;
    let (name, body) = rest.split_once("\">\n")?;
    let body = body.strip_suffix("\n</context>").unwrap_or(body);
    Some(Attachment::Text { name: name.into(), text: body.into() })
}

pub fn replayed_saved_file(text: &str) -> Option<Attachment> {
    let path = text.strip_prefix(SAVED_NOTE)?;
    (!path.is_empty() && !path.contains('\n')).then(|| Attachment::Saved { path: path.into() })
}

pub fn replayed_image(data: &str, mime: &str) -> Option<Attachment> {
    let mime = ["image/png", "image/jpeg", "image/gif", "image/webp"].into_iter().find(|m| *m == mime)?;
    let name = format!("image.{}", mime.trim_start_matches("image/").replace("jpeg", "jpg"));
    Some(Attachment::Image { name, mime, bytes: Arc::new(unbase64(data)?) })
}

pub const IMAGE_MAX: u64 = 3_700_000;
pub const TEXT_MAX: u64 = 250_000;
pub const FILE_MAX: u64 = 2_000_000_000;

const SAVED_NOTE: &str = "[Endeavor] The user attached a file. The app saved a copy into this session's folder, where \
    your file tools can read it and notebook code can load it, at this path relative to the folder:\n";

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn data_path(name: &str) -> String {
    format!("{DATA}/{name}")
}

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

pub fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000 => format!("{:.1} MB", b as f64 / 1_000_000.),
        b if b >= 1_000 => format!("{} KB", b / 1_000),
        b => format!("{b} bytes"),
    }
}

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

pub fn upload_file(path: &Path) -> Result<Attachment, String> {
    let name = file_name(path);
    let meta = std::fs::metadata(path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
    if meta.is_dir() {
        return Err(format!("{name} is a folder; attach the files in it instead."));
    }
    too_big(&name, meta.len())?;
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
    upload(&name, bytes)
}

#[derive(Debug, PartialEq)]
pub enum Added {
    Chip(Attachment),
    Mention(String),
}

#[derive(Clone, Debug)]
pub enum Folder {
    Here(PathBuf),
    Elsewhere,
    Unwritable,
}

pub const UNWRITABLE: &str = "This server's copy of Endeavor's helper can't save files into the session's folder, \
    so files go in the message instead (text up to 250 KB).";

pub fn add_file(path: &Path, folder: &Folder) -> Result<Added, String> {
    let name = file_name(path);
    if matches!(folder, Folder::Unwritable) || image_type(&name).is_some() {
        return upload_file(path).map(Added::Chip);
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("Couldn't read {name}: {e}"))?;
    if meta.is_dir() {
        return Err(format!("{name} is a folder; attach the files in it instead."));
    }
    if let Folder::Here(folder) = folder
        && let Some(inside) = relative_inside(folder, path)
    {
        return Ok(Added::Mention(inside));
    }
    if meta.len() > FILE_MAX {
        return Err(format!("{name} is {}; files can be up to {}.", size_text(meta.len()), size_text(FILE_MAX)));
    }
    Ok(Added::Chip(Attachment::Upload { source: path.to_path_buf(), size: meta.len() }))
}

pub fn relative_inside(folder: &Path, file: &Path) -> Option<String> {
    let folder = folder.canonicalize().ok()?;
    let file = file.canonicalize().ok()?;
    let rest = file.strip_prefix(&folder).ok()?;
    let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn same_contents(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    if std::fs::metadata(a)?.len() != std::fs::metadata(b)?.len() {
        return Ok(false);
    }
    let (mut a, mut b) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut x, mut y) = (vec![0; 1 << 16], vec![0; 1 << 16]);
    loop {
        let n = a.read(&mut x)?;
        if n == 0 {
            return Ok(true);
        }
        b.read_exact(&mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}

pub fn save_into(folder: &Path, source: &Path) -> Result<String, String> {
    if let Some(inside) = relative_inside(folder, source) {
        return Ok(inside);
    }
    let name = file_name(source);
    let couldnt = |e: std::io::Error| format!("Couldn't copy {name} into the session's folder: {e}.");
    let data = folder.join(DATA);
    std::fs::create_dir_all(&data).map_err(couldnt)?;
    let mut n = 1;
    loop {
        let candidate = numbered(&name, n);
        let dest = data.join(&candidate);
        if !dest.exists() {
            let part = data.join(format!(".{candidate}.part"));
            std::fs::copy(source, &part).and_then(|_| std::fs::rename(&part, &dest)).map_err(|e| {
                let _ = std::fs::remove_file(&part);
                couldnt(e)
            })?;
            return Ok(data_path(&candidate));
        }
        if same_contents(&dest, source).map_err(couldnt)? {
            return Ok(data_path(&candidate));
        }
        n += 1;
    }
}

pub type Ask = dyn Fn(Request) -> Result<Reply, String> + Send;

#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub name: String,
    pub sent: u64,
    pub size: u64,
}

pub enum Dest {
    Here(PathBuf),
    Server { folder: String, ask: Box<Ask>, progress: Box<dyn Fn(Progress) + Send> },
    Message,
}

const PIECE: usize = 1 << 20;

pub fn send_to_server(folder: &str, source: &Path, ask: &Ask, progress: &dyn Fn(Progress)) -> Result<String, String> {
    use std::io::Read;
    let name = file_name(source);
    let couldnt = |why: String| format!("Couldn't copy {name} into the session's folder: {}.", why.trim_end_matches('.'));
    let io = |e: std::io::Error| couldnt(e.to_string());
    let size = std::fs::metadata(source).map_err(io)?.len();
    let sha256 = files::sha256_file(source).map_err(io)?;
    let folder = folder.to_owned();
    let path = match ask(Request::Place { folder: folder.clone(), name: name.clone(), size, sha256 }).map_err(couldnt)? {
        Reply::Place { path, have: true } => return Ok(path),
        Reply::Place { path, have: false } => path,
        other => return Err(couldnt(format!("the server answered {other:?}"))),
    };
    let mut file = std::fs::File::open(source).map_err(io)?;
    let mut piece = vec![0; PIECE];
    let mut offset = 0;
    loop {
        let mut n = 0;
        while n < PIECE {
            match file.read(&mut piece[n..]).map_err(io)? {
                0 => break,
                read => n += read,
            }
        }
        let last = n < PIECE;
        let bytes = piece[..n].to_vec();
        match ask(Request::Write { folder: folder.clone(), path: path.clone(), offset, bytes, last }).map_err(couldnt)? {
            Reply::Written => {}
            other => return Err(couldnt(format!("the server answered {other:?}"))),
        }
        offset += n as u64;
        progress(Progress { name: name.clone(), sent: offset, size });
        if last {
            return Ok(path);
        }
    }
}

pub fn place_uploads(attachments: Vec<Attachment>, dest: &Dest) -> (Vec<Attachment>, Vec<String>) {
    let mut placed = Vec::new();
    let mut refused = Vec::new();
    for attachment in attachments {
        let Attachment::Upload { source, .. } = &attachment else {
            placed.push(attachment);
            continue;
        };
        let result = match dest {
            Dest::Here(folder) => save_into(folder, source).map(|path| Attachment::Saved { path }),
            Dest::Server { folder, ask, progress } => send_to_server(folder, source, ask.as_ref(), progress.as_ref()).map(|path| Attachment::Saved { path }),
            Dest::Message => upload_file(source),
        };
        match result {
            Ok(attachment) => placed.push(attachment),
            Err(why) => refused.push(why),
        }
    }
    (placed, refused)
}

pub(crate) fn base64(bytes: &[u8]) -> String {
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

pub(crate) fn unbase64(text: &str) -> Option<Vec<u8>> {
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

pub fn token(path: &str) -> String {
    if path.contains(char::is_whitespace) { format!("@\"{path}\"") } else { format!("@{path}") }
}

pub fn typing_mention(text: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
    let before = text.get(..cursor)?;
    let at = before.rfind('@')?;
    let query = &before[at + 1..];
    let starts_word = before[..at].chars().next_back().is_none_or(char::is_whitespace);
    (starts_word && !query.contains(char::is_whitespace) && !query.starts_with('"')).then_some((at..cursor, query))
}

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

pub fn mentioned(text: &str, mentions: &[String]) -> Vec<String> {
    let mut found: Vec<(usize, String)> = mentions
        .iter()
        .filter_map(|path| token_ranges(text, std::slice::from_ref(path)).first().map(|r| (r.start, path.clone())))
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

pub fn keep_tokens_whole(old: &str, new: &str, mentions: &[String]) -> Option<(String, usize)> {
    if new.len() >= old.len() {
        return None;
    }
    let prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    let prefix = (0..=prefix).rev().find(|&i| old.is_char_boundary(i) && new.is_char_boundary(i))?;
    let max_suffix = new.len() - prefix;
    let suffix = old.bytes().rev().zip(new.bytes().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
    let suffix = (0..=suffix).rev().find(|&s| old.is_char_boundary(old.len() - s) && new.is_char_boundary(new.len() - s))?;
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
        assert_eq!(one.label(), Label { plain: "".into(), mono: "rates".into(), after: "".into() });
        let three = Attachment::Cells { notebook: NB.into(), cells: vec![rates.clone(), plot.clone(), rates.clone()], ask: CellAsk::About };
        assert_eq!(three.label(), Label { plain: "3 cells".into(), mono: "".into(), after: "".into() });
        assert_eq!(Attachment::Cells { notebook: NB.into(), cells: vec![plot.clone()], ask: CellAsk::About }.label().mono, "scatter(data.t, data.co…");
        let error = Attachment::Error { notebook: NB.into(), cell: rates, text: "BoundsError".into() };
        assert_eq!(error.label(), Label { plain: "error in ".into(), mono: "rates".into(), after: "".into() });
        assert_eq!(error.icon(), Icon::Error);
        let long = Attachment::Text { name: "a_really_long_file_name_for_a_chip.csv".into(), text: String::new() };
        assert_eq!(long.label().mono, "a_really_long_file_name…");
    }

    #[test]
    fn a_quote_chip_cuts_the_name_and_keeps_the_part() {
        let figure = |name: &str| Attachment::Quote(Quote { from: Quoted::Cell { notebook: NB.into(), cell: "c1".into(), name: name.into(), part: Part::Figure(Arc::new(Vec::new())) }, comment: String::new() });
        let long = figure("@bind apoe_delta_pick apoe_picker()");
        assert_eq!(long.label(), Label { plain: "".into(), mono: "@bind apoe_delta_pick a…".into(), after: "· figure".into() });
        assert_eq!(long.full_label().text(), "@bind apoe_delta_pick apoe_picker() · figure");
        assert_eq!(figure("plot_fit").label().text(), "plot_fit · figure");
        let reply = Attachment::Quote(Quote { from: Quoted::Reply { text: "x".into(), at: None }, comment: String::new() });
        assert_eq!(reply.label(), Label { plain: "Claude's reply".into(), mono: "".into(), after: "".into() });
    }

    #[test]
    fn prompt_blocks_explain_attachments_then_carry_the_words() {
        let fit = cell("c1", "fit = curve_fit(model, t, y, p0)");
        let attachments = [
            Attachment::Error { notebook: NB.into(), cell: fit.clone(), text: "BoundsError".into() },
            Attachment::Image { name: "gel.png".into(), mime: "image/png", bytes: Arc::new(b"hi!".to_vec()) },
            Attachment::Text { name: "notes.txt".into(), text: "t,y\n1,2".into() },
        ];
        let blocks = prompt_blocks("Fix the error in this cell.", &attachments, &["data/decay.csv".into()], Backend::Pluto);
        let all = texts(&blocks);
        assert!(all[0].starts_with("[Endeavor] The user attached notebook cells"));
        assert_eq!(
            all[1..],
            [
                format!(
                    "[Endeavor] The cell below failed with this error.\n<attached kind=\"error\" notebook=\"{NB}\">\n\
                     <cell uri=\"notebook://pluto/{NB}/cell/c1\">\nfit = curve_fit(model, t, y, p0)\n</cell>\n\
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
        assert_eq!(self::texts(&prompt_blocks("hi", &[], &[], Backend::Pluto)), ["hi"], "a plain message is just its words");
        let fill = Attachment::Cells { notebook: NB.into(), cells: vec![cell("c2", "")], ask: CellAsk::Fill };
        assert_eq!(
            self::texts(&prompt_blocks("plot it", &[fill], &[], Backend::Pluto))[1],
            format!(
                "[Endeavor] The cell below is empty: write its code as the message asks.\n<attached kind=\"fill\" notebook=\"{NB}\">\n\
                 <cell uri=\"notebook://pluto/{NB}/cell/c2\">\n\n</cell>\n</attached>"
            )
        );
    }

    #[test]
    fn claude_code_reads_a_slash_command_only_from_the_first_block() {
        let fit = cell("c1", "fit = curve_fit(model, t, y, p0)");
        let attachments = [Attachment::Error { notebook: NB.into(), cell: fit, text: "BoundsError".into() }];
        let blocks = texts(&prompt_blocks("/compact keep the fit", &attachments, &[], Backend::Pluto));
        assert_eq!(blocks[0], "/compact keep the fit");
        assert!(blocks[1].starts_with("[Endeavor] The user attached notebook cells"));
    }

    #[test]
    fn notebook_blocks_parse_back_as_sent() {
        let attachments = [
            Attachment::Cells { notebook: NB.into(), cells: vec![cell("c1", "rates = 1"), cell("c2", "")], ask: CellAsk::About },
            Attachment::Cells { notebook: NB.into(), cells: vec![cell("c3", "")], ask: CellAsk::Before },
            Attachment::Error { notebook: NB.into(), cell: cell("c4", "html\"<cell uri=\\\"x\\\">\" # </error>"), text: "LoadError:\n  in expression".into() },
        ];
        for attachment in attachments {
            let block = notebook_block(&attachment, Backend::Pluto).unwrap();
            assert_eq!(replayed_notebook(&block), Some(attachment), "{block}");
        }
        let selection = format!(
            "[Endeavor] The message is about the text selected in the cell below.\n<attached kind=\"selection\" notebook=\"{NB}\">\n\
             <cell uri=\"notebook://pluto/{NB}/cell/c1\">\ns = sum(xs)\n</cell>\n<selection>\nsum(xs)\n</selection>\n</attached>"
        );
        let quoted = |from| Some(Attachment::Quote(Quote { from, comment: String::new() }));
        assert_eq!(
            replayed_notebook(&selection),
            quoted(Quoted::Cell { notebook: NB.into(), cell: "c1".into(), name: "s".into(), part: Part::Output("sum(xs)".into()) }),
            "a selection from before quotes"
        );
        let region = format!("[Endeavor] The user drew a box.\n<attached kind=\"region\" notebook=\"{NB}\">\n<cell uri=\"notebook://pluto/{NB}/cell/c3\">\nplot()\n</cell>\n</attached>");
        assert_eq!(
            replayed_notebook(&region),
            quoted(Quoted::Box { notebook: NB.into(), cells: vec!["c3".into()], png: Arc::new(Vec::new()) }),
            "a region from before quotes"
        );
        let old = format!("[Endeavor] The message is about the cell below.\n<attached kind=\"cells\" notebook=\"{NB}\">\n<cell uri=\"pluto://notebook/{NB}/cell/c1\">\nrates = 1\n</cell>\n</attached>");
        let about = Attachment::Cells { notebook: NB.into(), cells: vec![cell("c1", "rates = 1")], ask: CellAsk::About };
        assert_eq!(replayed_notebook(&old), Some(about), "a session from before notebook:// links");
        assert_eq!(replayed_notebook("[Endeavor] The user is viewing Pluto notebook x."), None);
        assert_eq!(replayed_notebook("<attached kind=\"cells\">"), None, "only the app's own blocks");
    }

    fn quote(from: Quoted, comment: &str) -> Quote {
        Quote { from, comment: comment.into() }
    }

    fn in_rates(part: Part) -> Quoted {
        Quoted::Cell { notebook: NB.into(), cell: "c1".into(), name: "rates".into(), part }
    }

    #[test]
    fn quotes_go_as_a_quote_then_the_comment() {
        let quotes = [
            quote(Quoted::Reply { text: "refits the model on 1,000 resampled copies".into(), at: Some("14:02".into()) }, "Why 1,000 and not 10,000?"),
            quote(in_rates(Part::Lines { first: 3, last: 5, text: "rows = rand(1:n, n)\nfit(rows)\nend;".into() }), "Is sampling with replacement right here?"),
            quote(in_rates(Part::Figure(Arc::new(b"hi!".to_vec()))), "Why does the fit miss these early points?"),
            quote(Quoted::Box { notebook: NB.into(), cells: vec!["c1".into(), "c2".into()], png: Arc::new(Vec::new()) }, ""),
            quote(Quoted::Box { notebook: NB.into(), cells: vec!["c1".into()], png: Arc::new(b"box".to_vec()) }, "this"),
            quote(in_rates(Part::Whole("rates = 1\nfit()".into())), "and the cell"),
            quote(in_rates(Part::WholeOutput("0.42".into())), ""),
            quote(in_rates(Part::Output("42".into())), "this number"),
        ];
        let attachments: Vec<Attachment> = quotes.into_iter().map(Attachment::Quote).collect();
        let all = texts(&prompt_blocks("Check these before I write this up.", &attachments, &[], Backend::Pluto));
        assert!(all[0].starts_with("[Endeavor] The user quoted parts of the notebook"));
        assert_eq!(
            all[1..],
            [
                "> refits the model on 1,000 resampled copies\n> — Claude's reply, 14:02\n\nWhy 1,000 and not 10,000?".to_string(),
                format!("<quote cell=\"rates\" lines=\"3-5\" uri=\"notebook://pluto/{NB}/cell/c1\">\nrows = rand(1:n, n)\nfit(rows)\nend;\n</quote>\nIs sampling with replacement right here?"),
                format!("<quote cell=\"rates\" part=\"figure\" uri=\"notebook://pluto/{NB}/cell/c1\"/>\nWhy does the fit miss these early points?"),
                format!("<quote part=\"box\" uri=\"notebook://pluto/{NB}/cell/c1 notebook://pluto/{NB}/cell/c2\"/>"),
                format!("<quote part=\"box\" uri=\"notebook://pluto/{NB}/cell/c1\"/>\nthis"),
                "image image/png Ym94".into(),
                format!("<quote cell=\"rates\" uri=\"notebook://pluto/{NB}/cell/c1\"/>\nand the cell"),
                format!("<quote cell=\"rates\" part=\"output\" uri=\"notebook://pluto/{NB}/cell/c1\"/>"),
                format!("<quote cell=\"rates\" part=\"output\" uri=\"notebook://pluto/{NB}/cell/c1\">\n42\n</quote>\nthis number"),
                "Check these before I write this up.".into(),
            ],
            "lines go with their text; a cell, an output and a figure by name only; a box with its picture, and without one if none was taken"
        );
        let chat_only = [Attachment::Quote(quote(Quoted::Reply { text: "x".into(), at: None }, ""))];
        assert_eq!(texts(&prompt_blocks("", &chat_only, &[], Backend::Pluto)), ["> x\n> — Claude's reply"], "no notebook note, and no words");
    }

    #[test]
    fn quote_blocks_parse_back_as_sent() {
        let quotes = [
            quote(Quoted::Reply { text: "one\n\ntwo".into(), at: Some("Sat 9:10".into()) }, "and?\n\nmore"),
            quote(Quoted::Reply { text: "x".into(), at: None }, ""),
            quote(in_rates(Part::Whole(String::new())), "whole"),
            quote(in_rates(Part::Lines { first: 2, last: 2, text: "fit()\n</quote> # not the end".into() }), ""),
            quote(in_rates(Part::WholeOutput(String::new())), "hm"),
            quote(in_rates(Part::Output("0.42 ± 0.03".into())), "selected"),
            quote(in_rates(Part::Figure(Arc::new(Vec::new()))), ""),
            quote(Quoted::Box { notebook: NB.into(), cells: vec!["c1".into()], png: Arc::new(Vec::new()) }, "is this real?"),
            quote(Quoted::Cell { notebook: NB.into(), cell: "c1".into(), name: "HTML(\"\"\"<svg/> & co".into(), part: Part::WholeOutput(String::new()) }, "odd name"),
            quote(Quoted::Cell { notebook: NB.into(), cell: "c1".into(), name: "&quot; uri=\"x\"".into(), part: Part::Lines { first: 1, last: 1, text: "a".into() } }, ""),
        ];
        let odd = quote(Quoted::Cell { notebook: NB.into(), cell: "c1".into(), name: "Text(\"Sensor log <a> & b\")".into(), part: Part::Whole(String::new()) }, "");
        assert_eq!(quote_block(&odd, Backend::Pluto), format!("<quote cell=\"Text(&quot;Sensor log &lt;a&gt; &amp; b&quot;)\" uri=\"notebook://pluto/{NB}/cell/c1\"/>"));
        for q in quotes {
            let block = quote_block(&q, Backend::Pluto);
            assert_eq!(replayed_quote(&block), Some(Attachment::Quote(q)), "{block}");
        }
        let old = format!("<quote cell=\"rates\" part=\"output\" uri=\"notebook://pluto/{NB}/cell/c1\">\n0.42\n</quote>\nhm");
        assert_eq!(replayed_quote(&old), Some(Attachment::Quote(quote(in_rates(Part::Output("0.42".into())), "hm"))));
        let old = format!("<quote cell=\"rates\" uri=\"notebook://pluto/{NB}/cell/c1\">\nrates = 1\n</quote>");
        assert_eq!(replayed_quote(&old), Some(Attachment::Quote(quote(in_rates(Part::Whole("rates = 1".into())), ""))));
        assert_eq!(replayed_quote("> just a blockquote the user typed"), None);
        assert_eq!(replayed_quote("plain words"), None);
    }

    #[test]
    fn sending_picks_now_takes_the_quotes_staged_in_the_composer_and_leaves_the_rest() {
        let staged = Attachment::Quote(quote(in_rates(Part::Whole(String::new())), "first"));
        let file = Attachment::Text { name: "notes.txt".into(), text: "x".into() };
        let mut composer = vec![staged.clone(), file.clone()];
        assert_eq!(take_staged_quotes(&mut composer), [staged]);
        assert_eq!(composer, [file]);
    }

    #[test]
    fn a_reopened_quote_that_names_a_cell_or_output_has_no_excerpt_to_draw() {
        for part in [Part::Whole(String::new()), Part::WholeOutput(String::new())] {
            let q = quote(in_rates(part), "hm");
            let Some(Attachment::Quote(back)) = replayed_quote(&quote_block(&q, Backend::Pluto)) else { panic!("didn't parse") };
            assert_eq!(back.excerpt(), None);
        }
        assert_eq!(quote(in_rates(Part::Output("0.42".into())), "").excerpt(), Some("0.42"));
    }

    #[test]
    fn quotes_say_where_they_are_from() {
        let sources: Vec<String> = [
            Quoted::Reply { text: "x".into(), at: Some("14:02".into()) },
            Quoted::Reply { text: "x".into(), at: None },
            in_rates(Part::Lines { first: 3, last: 5, text: String::new() }),
            in_rates(Part::Lines { first: 4, last: 4, text: String::new() }),
            in_rates(Part::Whole(String::new())),
            in_rates(Part::Output(String::new())),
            in_rates(Part::Figure(Arc::new(Vec::new()))),
            Quoted::Box { notebook: NB.into(), cells: vec!["a".into(), "b".into()], png: Arc::new(Vec::new()) },
        ]
        .into_iter()
        .map(|from| quote(from, "").source())
        .collect();
        assert_eq!(
            sources,
            ["Claude's reply · 14:02", "Claude's reply", "rates · lines 3–5", "rates · line 4", "rates · whole cell", "rates · output", "rates · figure", "Box · 2 cells"]
        );
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
        assert!(is_app_text("notebook://pluto/x/cell/y"));
        assert!(is_app_text("pluto://notebook/x/cell/y"), "a link from before notebook:// links");
        assert!(is_app_text("\n<context ref=\"attachment:notes.txt\">\nx\n</context>"));
        assert!(!is_app_text("why do these bunch up?"));
    }

    #[test]
    fn a_stopped_turns_markers_are_recognised() {
        assert!(is_stopped_marker("[Request interrupted by user]"));
        assert!(is_stopped_marker("[Request interrupted by user for tool use]"));
        assert!(!is_stopped_marker("[Request interrupted by user] and then some"), "the exact marker only, not a prefix match");
        assert!(!is_stopped_marker("why do these bunch up?"));
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
        assert_eq!(keep_tokens_whole(old, "fit @data/decay.cs now", &mentions), Some(("fit  now".into(), 4)));
        assert_eq!(keep_tokens_whole(old, "fit data/decay.csv now", &mentions), Some(("fit  now".into(), 4)));
        assert_eq!(keep_tokens_whole(old, "fit  now", &mentions), None);
        assert_eq!(keep_tokens_whole(old, "fi @data/decay.csv now", &mentions), None);
        assert_eq!(keep_tokens_whole(old, "fit @data/decay.csv now!", &mentions), None);
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

    fn scratch(test: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("endeavor-attach-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (session, elsewhere) = (root.join("session"), root.join("elsewhere"));
        std::fs::create_dir_all(&session).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        (session, elsewhere)
    }

    fn write(path: &Path, contents: &[u8]) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
        path.to_path_buf()
    }

    #[test]
    fn a_file_from_elsewhere_is_copied_into_data_on_send() {
        let (session, elsewhere) = scratch("copy");
        let csv = write(&elsewhere.join("decay.csv"), b"t,y\n0,1.0\n");
        let added = add_file(&csv, &Folder::Here(session.clone())).unwrap();
        assert_eq!(added, Added::Chip(Attachment::Upload { source: csv.clone(), size: 10 }));
        let Added::Chip(chip) = added else { unreachable!() };
        assert_eq!(chip.label(), Label { plain: "".into(), mono: "data/decay.csv".into(), after: "".into() });
        assert!(!session.join("data").exists(), "nothing is copied when the file is picked");

        let (placed, refused) = place_uploads(vec![chip], &Dest::Here(session.clone()));
        assert_eq!((placed.as_slice(), refused.as_slice()), (&[Attachment::Saved { path: "data/decay.csv".into() }][..], &[][..]));
        assert_eq!(std::fs::read(session.join("data/decay.csv")).unwrap(), b"t,y\n0,1.0\n");
        assert_eq!(std::fs::read_dir(session.join("data")).unwrap().count(), 1, "no temporary file left behind");
    }

    #[test]
    fn a_name_clash_reuses_the_same_contents_and_numbers_different_ones() {
        let (session, elsewhere) = scratch("clash");
        let first = write(&elsewhere.join("a/decay.csv"), b"run 1");
        let same = write(&elsewhere.join("b/decay.csv"), b"run 1");
        let other = write(&elsewhere.join("c/decay.csv"), b"run 2");
        let third = write(&elsewhere.join("d/decay.csv"), b"run 3");
        assert_eq!(save_into(&session, &first).unwrap(), "data/decay.csv");
        assert_eq!(save_into(&session, &same).unwrap(), "data/decay.csv", "same contents: reused");
        assert_eq!(save_into(&session, &other).unwrap(), "data/decay (2).csv");
        assert_eq!(save_into(&session, &third).unwrap(), "data/decay (3).csv");
        assert_eq!(save_into(&session, &other).unwrap(), "data/decay (2).csv", "found again under its number");
        assert_eq!(std::fs::read(session.join("data/decay (3).csv")).unwrap(), b"run 3");
        assert_eq!(numbered("README", 2), "README (2)");
        assert_eq!(numbered(".env", 2), ".env (2)");
        assert_eq!(numbered("scan.tar.gz", 4), "scan.tar (4).gz");
    }

    #[test]
    fn a_file_already_in_the_folder_is_mentioned_not_copied() {
        let (session, _) = scratch("inside");
        let csv = write(&session.join("raw/run 1.csv"), b"t,y");
        assert_eq!(add_file(&csv, &Folder::Here(session.clone())), Ok(Added::Mention("raw/run 1.csv".into())));
        let other_route = session.join("raw/../raw/run 1.csv");
        assert_eq!(relative_inside(&session, &other_route).as_deref(), Some("raw/run 1.csv"));
        assert_eq!(save_into(&session, &csv).unwrap(), "raw/run 1.csv");
        assert!(!session.join("data").exists());
        assert_eq!(relative_inside(&session, &session), None, "the folder itself isn't a file in it");
    }

    #[test]
    fn added_files_keep_images_in_the_message_and_take_any_data() {
        let (session, elsewhere) = scratch("kinds");
        let gel = write(&elsewhere.join("gel.png"), b"png");
        assert!(matches!(add_file(&gel, &Folder::Here(session.clone())), Ok(Added::Chip(Attachment::Image { mime: "image/png", .. }))));
        let sheet = write(&elsewhere.join("plate.xlsx"), &[0x50, 0x4b, 0x00, 0xff]);
        assert!(matches!(add_file(&sheet, &Folder::Here(session.clone())), Ok(Added::Chip(Attachment::Upload { size: 4, .. }))));
        assert_eq!(add_file(&elsewhere, &Folder::Here(session.clone())), Err("elsewhere is a folder; attach the files in it instead.".into()));
        assert!(matches!(add_file(&gel, &Folder::Elsewhere), Ok(Added::Chip(Attachment::Image { .. }))));
        assert!(matches!(add_file(&sheet, &Folder::Elsewhere), Ok(Added::Chip(Attachment::Upload { size: 4, .. }))));
        let inside = write(&session.join("raw/run.csv"), b"t,y");
        assert_eq!(add_file(&inside, &Folder::Elsewhere), Ok(Added::Chip(Attachment::Upload { source: inside.clone(), size: 3 })));
        let notes = write(&elsewhere.join("notes.txt"), b"t,y");
        assert_eq!(add_file(&notes, &Folder::Unwritable), Ok(Added::Chip(Attachment::Text { name: "notes.txt".into(), text: "t,y".into() })));
        assert_eq!(add_file(&sheet, &Folder::Unwritable), Err("plate.xlsx isn't text or a PNG, JPEG, GIF or WebP image, so it can't be attached yet.".into()));
    }

    fn server(folder: &Path, progress: Arc<std::sync::Mutex<Vec<(u64, u64)>>>) -> Dest {
        Dest::Server {
            folder: crate::hosts::text(folder),
            ask: Box::new(|request| match files::answer(&request) {
                Reply::Error { message } => Err(message),
                reply => Ok(reply),
            }),
            progress: Box::new(move |p: Progress| progress.lock().unwrap().push((p.sent, p.size))),
        }
    }

    #[test]
    fn a_file_is_sent_to_a_server_in_pieces_by_the_same_clash_rule() {
        let (session, elsewhere) = scratch("server");
        let big: Vec<u8> = (0..PIECE * 2 + 5).map(|i| (i % 251) as u8).collect();
        let csv = write(&elsewhere.join("a/decay.csv"), &big);
        let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
        let dest = server(&session, progress.clone());
        let upload = |source: &Path| Attachment::Upload { source: source.to_path_buf(), size: 0 };
        let saved = |path: &str| Attachment::Saved { path: path.into() };

        let (placed, refused) = place_uploads(vec![upload(&csv)], &dest);
        assert_eq!((placed, refused), (vec![saved("data/decay.csv")], vec![]));
        assert_eq!(std::fs::read(session.join("data/decay.csv")).unwrap(), big);
        let size = big.len() as u64;
        assert_eq!(*progress.lock().unwrap(), [(PIECE as u64, size), (2 * PIECE as u64, size), (size, size)]);

        progress.lock().unwrap().clear();
        let same = write(&elsewhere.join("b/decay.csv"), &big);
        assert_eq!(place_uploads(vec![upload(&same)], &dest).0, [saved("data/decay.csv")]);
        assert!(progress.lock().unwrap().is_empty(), "nothing was sent");
        let other = write(&elsewhere.join("c/decay.csv"), b"t,y\n0,2\n");
        assert_eq!(place_uploads(vec![upload(&other)], &dest).0, [saved("data/decay (2).csv")]);
        assert_eq!(std::fs::read(session.join("data/decay (2).csv")).unwrap(), b"t,y\n0,2\n");
        assert_eq!(std::fs::read_dir(session.join("data")).unwrap().count(), 2, "no parts left behind");

        let failing = Dest::Server { folder: crate::hosts::text(&session.join("nope")), ask: Box::new(|_| Err("The connection closed.".into())), progress: Box::new(|_| {}) };
        let (placed, refused) = place_uploads(vec![upload(&other)], &failing);
        assert!(placed.is_empty());
        assert_eq!(refused, ["Couldn't copy decay.csv into the session's folder: The connection closed."]);
    }

    #[test]
    fn a_file_gone_before_sending_is_refused_plainly() {
        let (session, elsewhere) = scratch("gone");
        let upload = Attachment::Upload { source: elsewhere.join("gone.csv"), size: 3 };
        let (placed, refused) = place_uploads(vec![upload], &Dest::Here(session.clone()));
        assert!(placed.is_empty());
        assert_eq!(refused.len(), 1);
        assert!(refused[0].starts_with("Couldn't copy gone.csv into the session's folder: "), "{}", refused[0]);
    }

    #[test]
    fn a_saved_file_is_a_note_with_its_path() {
        let saved = Attachment::Saved { path: "data/decay (2).csv".into() };
        let blocks = texts(&prompt_blocks("fit this", std::slice::from_ref(&saved), &[], Backend::Pluto));
        assert_eq!(
            blocks,
            [
                "[Endeavor] The user attached a file. The app saved a copy into this session's folder, where your file \
                 tools can read it and notebook code can load it, at this path relative to the folder:\ndata/decay (2).csv",
                "fit this",
            ]
        );
        assert_eq!(replayed_saved_file(&blocks[0]), Some(saved));
        assert_eq!(replayed_saved_file("fit this"), None);
    }
}
