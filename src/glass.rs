//! Glass design mode, Rust side: messages from `glass.js` and the prompt blocks
//! queued annotations turn into (design doc §4.2–4.3).

use agent_client_protocol::schema::v1::{ContentBlock, ResourceLink, TextContent};

pub const SCRIPT: &str = include_str!("glass.js");

/// Upper bounds on page-supplied data. The page also runs notebook output JS,
/// so any of these messages may be forged; the user sees every annotation in
/// the tray before it is sent.
const MAX_CELLS: usize = 64;
const MAX_COMMENT: usize = 4000;

#[derive(Debug, PartialEq)]
pub struct Annotation {
    pub notebook: String,
    pub cells: Vec<String>,
    pub comment: String,
}

#[derive(Debug, PartialEq)]
pub enum Message {
    Mode(bool),
    Annotation(Annotation),
    /// Send everything queued now.
    Send,
}

pub fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.matches('-').count() == 4 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Parse and validate one IPC message from the page; `None` drops it.
pub fn parse(body: &str) -> Option<Message> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    match v.get("type")?.as_str()? {
        "glass" => Some(Message::Mode(v.get("on")?.as_bool()?)),
        "send" => Some(Message::Send),
        "annotation" => {
            let notebook = v.get("notebook")?.as_str().filter(|s| is_uuid(s))?.to_owned();
            let cells: Vec<String> = v
                .get("cells")?
                .as_array()?
                .iter()
                .map(|c| c.as_str().filter(|s| is_uuid(s)).map(str::to_owned))
                .collect::<Option<_>>()?;
            let comment: String = v.get("comment")?.as_str()?.chars().take(MAX_COMMENT).collect();
            (!cells.is_empty() && cells.len() <= MAX_CELLS)
                .then_some(Message::Annotation(Annotation { notebook, cells, comment }))
        }
        _ => None,
    }
}

pub fn cell_uri(notebook: &str, cell: &str) -> String {
    format!("pluto://notebook/{notebook}/cell/{cell}")
}

/// Prompt blocks for queued annotations: one explanatory preface, then per
/// annotation a `ResourceLink` per cell followed by the user's comment.
pub fn prompt_blocks(annotations: &[Annotation]) -> Vec<ContentBlock> {
    if annotations.is_empty() {
        return Vec::new();
    }
    let mut blocks = vec![ContentBlock::Text(TextContent::new(
        "[Endeavor] The user annotated notebook cells in glass mode. Each \
         pluto://notebook/{notebook_id}/cell/{cell_id} link names a cell; read its current \
         code and output with the pluto MCP tools (the links are not fetchable URLs).",
    ))];
    for a in annotations {
        for cell in &a.cells {
            blocks.push(ContentBlock::ResourceLink(ResourceLink::new(
                format!("cell {}", &cell[..8]),
                cell_uri(&a.notebook, cell),
            )));
        }
        let comment = if a.comment.is_empty() { "(no comment)" } else { &a.comment };
        blocks.push(ContentBlock::Text(TextContent::new(format!(
            "Comment on the {} cell(s) above: {comment}",
            a.cells.len()
        ))));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    const NB: &str = "6a1b2c3d-0000-4000-8000-1234567890ab";
    const C1: &str = "11111111-2222-4333-8444-555555555555";

    #[test]
    fn parses_valid_messages() {
        assert_eq!(parse(r#"{"type":"glass","on":true}"#), Some(Message::Mode(true)));
        assert_eq!(parse(r#"{"type":"send"}"#), Some(Message::Send));
        let body = format!(r#"{{"type":"annotation","notebook":"{NB}","cells":["{C1}"],"comment":"why so slow?"}}"#);
        let Some(Message::Annotation(a)) = parse(&body) else { panic!("rejected valid annotation") };
        assert_eq!((a.notebook.as_str(), a.cells.len(), a.comment.as_str()), (NB, 1, "why so slow?"));
    }

    #[test]
    fn rejects_forged_or_malformed_messages() {
        let bad_cell = format!(r#"{{"type":"annotation","notebook":"{NB}","cells":["../../x"],"comment":""}}"#);
        let no_cells = format!(r#"{{"type":"annotation","notebook":"{NB}","cells":[],"comment":""}}"#);
        let bad_nb = format!(r#"{{"type":"annotation","notebook":"nope","cells":["{C1}"],"comment":""}}"#);
        for body in [bad_cell.as_str(), no_cells.as_str(), bad_nb.as_str(), "not json", r#"{"type":"other"}"#] {
            assert_eq!(parse(body), None, "{body}");
        }
    }

    #[test]
    fn caps_comment_length() {
        let long = "x".repeat(MAX_COMMENT + 10);
        let body = format!(r#"{{"type":"annotation","notebook":"{NB}","cells":["{C1}"],"comment":"{long}"}}"#);
        let Some(Message::Annotation(a)) = parse(&body) else { panic!() };
        assert_eq!(a.comment.len(), MAX_COMMENT);
    }

    #[test]
    fn prompt_blocks_link_each_cell_then_comment() {
        let a = Annotation { notebook: NB.into(), cells: vec![C1.into()], comment: "".into() };
        let blocks = prompt_blocks(&[a]);
        assert_eq!(blocks.len(), 3);
        let ContentBlock::ResourceLink(link) = &blocks[1] else { panic!("expected link") };
        assert_eq!(link.uri, cell_uri(NB, C1));
        let ContentBlock::Text(t) = &blocks[2] else { panic!("expected comment") };
        assert!(t.text.ends_with("(no comment)"));
    }
}
