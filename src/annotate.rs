//! Messages from the page script (frontend/, annotation mode and asking about
//! cells), turned into chat messages with their attachments (design doc §4.2–4.3).

use crate::attach::{Attachment, Cell, CellAsk};
use wire::backend::Backend;

/// The page script (frontend/, built with `npm run build`; the bundle is committed
/// so building the app needs no Node).
const SCRIPT: &str = include_str!("../frontend/dist/page.js");

/// A per-launch secret the page script puts in every message. The page also runs
/// notebook output JS, which can post to the same channel; it can't read the
/// secret (it stays in the script's closure, see frontend/src/bridge.ts).
fn page_nonce() -> &'static str {
    static NONCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NONCE.get_or_init(|| {
        use std::io::Read;
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).expect("/dev/urandom");
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
}

/// The page script with this launch's secret filled in.
pub fn script() -> String {
    let script = SCRIPT.replace("__ENDEAVOR_NONCE__", &format!("\"{}\"", page_nonce()));
    #[cfg(debug_assertions)]
    let script = format!("{}\n{script}", crate::debug_state::RECORD_ALERTS);
    script
}

/// Upper bounds on page-supplied data. The page also runs notebook output JS,
/// so any of these messages may be forged; the user sees every annotation in
/// the tray before it is sent.
const MAX_CELLS: usize = 64;
const MAX_COMMENT: usize = 4000;
const MAX_CODE: usize = 20_000;
/// Bounds a drawn box's coordinates, in CSS pixels.
const MAX_SIDE: f64 = 100_000.;

/// A message the user sent from the notebook: their words, and what it's about.
#[derive(Debug, PartialEq)]
pub struct Ask {
    pub text: String,
    pub attachment: Attachment,
    /// Cmd+Enter: join the running turn instead of waiting in the queue.
    pub now: bool,
}

#[derive(Debug, PartialEq)]
pub enum Message {
    /// The page script loaded and wants the current cell states.
    Ready,
    Mode(bool),
    Ask(Ask),
    /// A box drawn with Point, to be sent once the app has its picture.
    Region(Region),
    /// A cell's code now (None: the page has no such cell), as the app asked.
    Code { cell: String, code: Option<String> },
    /// The shown notebook's state, for the notebook header.
    State(PageState),
    /// Run notebook, in the safe-preview callout.
    RunNotebook { notebook: String },
    /// Fix with Claude, in Status's box for a package that failed.
    FixPackage { notebook: String, name: String, log: String },
    /// Restart notebook, in the same box.
    Restart { notebook: String },
    /// The page's part of a state dump (debug_state.rs), as it sent it.
    #[cfg(debug_assertions)]
    Debug(serde_json::Value),
}

/// What the page reads from Pluto's state for the notebook header.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageState {
    pub notebook: String,
    /// In safe preview: nothing runs until the user says so.
    pub safe: bool,
    /// Work under way, e.g. "Installing packages · 2 of 5", "Running 4 of 7".
    pub busy: Option<String>,
    /// Pluto asks for a restart ("required" or "recommended"), e.g. after a package change.
    pub restart: Option<String>,
    pub save_failed: bool,
    /// A package that failed to install or precompile.
    pub package_failed: Option<String>,
    /// The notebook's process exited.
    pub dead: bool,
    /// The page's connection to Pluto is up.
    pub connected: bool,
    /// The drawer's open tab ("docs", "status").
    pub drawer: Option<String>,
}

/// A drawn box: the user's words, the cells it overlaps, and where it is in
/// the page's viewport (x, y, width, height in CSS pixels).
#[derive(Debug, PartialEq)]
pub struct Region {
    pub text: String,
    pub notebook: String,
    pub cells: Vec<Cell>,
    pub rect: [f64; 4],
    pub now: bool,
}

pub fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.matches('-').count() == 4 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Parse and validate one IPC message from the page; `None` drops it.
pub fn parse(body: &str) -> Option<Message> {
    parse_with(body, page_nonce())
}

fn capped(value: Option<&serde_json::Value>, max: usize) -> String {
    value.and_then(|v| v.as_str()).unwrap_or("").chars().take(max).collect()
}

/// `parse`, given the secret messages must carry (a missing one counts as "").
fn parse_with(body: &str, nonce: &str) -> Option<Message> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v.get("nonce").and_then(|n| n.as_str()).unwrap_or("") != nonce {
        return None;
    }
    let uuid = |key: &str| v.get(key)?.as_str().filter(|s| is_uuid(s)).map(str::to_owned);
    let now = v.get("now").and_then(|n| n.as_bool()).unwrap_or(false);
    match v.get("type")?.as_str()? {
        "ready" => Some(Message::Ready),
        // Fix with Claude / Explain on a cell's error.
        "ask" => {
            let (notebook, id) = (uuid("notebook")?, uuid("cell")?);
            let text = match v.get("kind")?.as_str()? {
                "fix" => "Fix the error in this cell.",
                "explain" => "Explain this error; don't change anything yet.",
                _ => return None,
            };
            let cell = Cell { id, code: capped(v.get("code"), MAX_CODE) };
            let attachment = Attachment::Error { notebook, cell, text: capped(v.get("error"), MAX_COMMENT) };
            Some(Message::Ask(Ask { text: text.into(), attachment, now: false }))
        }
        // ⌘K on a cell, the agent button between cells, or the selection chip.
        "prompt" => {
            let (notebook, id) = (uuid("notebook")?, uuid("cell")?);
            let text = capped(v.get("text"), MAX_COMMENT);
            let ask = match v.get("where")?.as_str()? {
                "about" => CellAsk::About,
                "fill" => CellAsk::Fill,
                "before" => CellAsk::Before,
                "after" => CellAsk::After,
                _ => return None,
            };
            let cell = Cell { id, code: capped(v.get("code"), MAX_CODE) };
            let attachment = match v.get("quote").and_then(|q| q.as_str()) {
                Some(quote) => Attachment::Selection { notebook, cell, text: quote.chars().take(MAX_COMMENT).collect() },
                None => Attachment::Cells { notebook, cells: vec![cell], ask },
            };
            Some(Message::Ask(Ask { text, attachment, now }))
        }
        "mode" => Some(Message::Mode(v.get("on")?.as_bool()?)),
        "annotation" => {
            let notebook = uuid("notebook")?;
            let cells = cells(&v)?;
            if cells.is_empty() {
                return None;
            }
            let text = capped(v.get("comment"), MAX_COMMENT);
            Some(Message::Ask(Ask { text, attachment: Attachment::Cells { notebook, cells, ask: CellAsk::About }, now }))
        }
        "region" => {
            let notebook = uuid("notebook")?;
            let rect = v.get("rect")?;
            let n = |key: &str| rect.get(key)?.as_f64().filter(|n| (-MAX_SIDE..MAX_SIDE).contains(n));
            let rect = [n("x")?, n("y")?, n("width")?, n("height")?];
            if rect[2] < 1. || rect[3] < 1. {
                return None;
            }
            let text = capped(v.get("comment"), MAX_COMMENT);
            Some(Message::Region(Region { text, notebook, cells: cells(&v)?, rect, now }))
        }
        "code" => {
            let cell = uuid("cell")?;
            let code = v.get("code").and_then(|c| c.as_str()).map(|c| c.chars().take(MAX_CODE).collect());
            Some(Message::Code { cell, code })
        }
        "state" => {
            let text = |key: &str| v.get(key).and_then(|s| s.as_str()).map(|s| s.chars().take(80).collect::<String>());
            let flag = |key: &str| v.get(key).and_then(|b| b.as_bool()).unwrap_or(false);
            Some(Message::State(PageState {
                notebook: uuid("notebook")?,
                safe: flag("safe"),
                busy: text("busy"),
                restart: text("restart").filter(|r| r == "required" || r == "recommended"),
                save_failed: flag("save_failed"),
                package_failed: text("package_failed"),
                dead: flag("dead"),
                connected: v.get("connected").and_then(|b| b.as_bool()).unwrap_or(true),
                drawer: text("drawer").filter(|d| d == "docs" || d == "status"),
            }))
        }
        #[cfg(debug_assertions)]
        "debug" => {
            let mut page = v.clone();
            page.as_object_mut()?.retain(|key, _| key != "type" && key != "nonce");
            Some(Message::Debug(page))
        }
        "run_notebook" => Some(Message::RunNotebook { notebook: uuid("notebook")? }),
        "restart" => Some(Message::Restart { notebook: uuid("notebook")? }),
        "fix_package" => Some(Message::FixPackage {
            notebook: uuid("notebook")?,
            name: capped(v.get("name"), 200),
            log: capped(v.get("log"), MAX_CODE),
        }),
        _ => None,
    }
}

/// Picked cells: `cells` (their ids, notebook order) and `codes` (the same order).
fn cells(v: &serde_json::Value) -> Option<Vec<Cell>> {
    let ids: Vec<String> = v.get("cells")?.as_array()?.iter().map(|c| c.as_str().filter(|s| is_uuid(s)).map(str::to_owned)).collect::<Option<_>>()?;
    if ids.len() > MAX_CELLS {
        return None;
    }
    let codes = v.get("codes").and_then(|c| c.as_array());
    Some(ids.into_iter().enumerate().map(|(i, id)| Cell { id, code: capped(codes.and_then(|c| c.get(i)), MAX_CODE) }).collect())
}

pub fn cell_uri(backend: Backend, notebook: &str, cell: &str) -> String {
    format!("notebook://{}/{notebook}/cell/{cell}", backend.name())
}

/// The cell id in a link to a cell of `notebook`: `cell_uri`'s form, or
/// `pluto://notebook/…/cell/…`, which sessions from before it used.
pub fn uri_cell<'a>(uri: &'a str, notebook: &str) -> Option<&'a str> {
    let rest = match uri.strip_prefix("notebook://") {
        Some(rest) => rest.split_once('/')?.1,
        None => uri.strip_prefix("pluto://notebook/")?,
    };
    rest.strip_prefix(notebook)?.strip_prefix("/cell/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NB: &str = "6a1b2c3d-0000-4000-8000-1234567890ab";
    const C1: &str = "11111111-2222-4333-8444-555555555555";

    fn cell(code: &str) -> Cell {
        Cell { id: C1.into(), code: code.into() }
    }

    #[test]
    fn needs_the_page_secret() {
        let body = r#"{"type":"mode","on":true,"nonce":"abc"}"#;
        assert_eq!(parse_with(body, "abc"), Some(Message::Mode(true)));
        assert_eq!(parse_with(body, "xyz"), None);
        assert_eq!(parse_with(r#"{"type":"mode","on":true}"#, "abc"), None, "notebook JS has no secret");
        assert!(script().contains(&format!("\"{}\"", page_nonce())) && !script().contains("__ENDEAVOR_NONCE__"));
    }

    #[test]
    fn parses_valid_messages() {
        assert_eq!(parse_with(r#"{"type":"mode","on":true}"#, ""), Some(Message::Mode(true)));
        assert_eq!(parse_with(r#"{"type":"ready"}"#, ""), Some(Message::Ready));
        let ask = |kind: &str| {
            parse_with(
                &format!(r#"{{"type":"ask","kind":"{kind}","notebook":"{NB}","cell":"{C1}","code":"x = lsq(1)","error":"UndefVarError: lsq"}}"#),
                "",
            )
        };
        assert_eq!(
            ask("fix"),
            Some(Message::Ask(Ask {
                text: "Fix the error in this cell.".into(),
                attachment: Attachment::Error { notebook: NB.into(), cell: cell("x = lsq(1)"), text: "UndefVarError: lsq".into() },
                now: false,
            }))
        );
        assert!(matches!(ask("explain"), Some(Message::Ask(a)) if a.text.starts_with("Explain")));
        assert_eq!(ask("delete everything"), None);
        assert_eq!(parse_with(r#"{"type":"send"}"#, ""), None);
        let prompt = |place: &str| {
            parse_with(&format!(r#"{{"type":"prompt","notebook":"{NB}","cell":"{C1}","code":"","where":"{place}","text":"plot it","now":true}}"#), "")
        };
        assert_eq!(
            prompt("after"),
            Some(Message::Ask(Ask { text: "plot it".into(), attachment: Attachment::Cells { notebook: NB.into(), cells: vec![cell("")], ask: CellAsk::After }, now: true }))
        );
        assert_eq!(prompt("anywhere"), None);
        let quoted = parse_with(&format!(r#"{{"type":"prompt","notebook":"{NB}","cell":"{C1}","code":"s = sum(xs)","where":"about","text":"why?","now":false,"quote":"sum(xs)"}}"#), "");
        assert_eq!(
            quoted,
            Some(Message::Ask(Ask { text: "why?".into(), attachment: Attachment::Selection { notebook: NB.into(), cell: cell("s = sum(xs)"), text: "sum(xs)".into() }, now: false }))
        );
        let body = format!(r#"{{"type":"annotation","notebook":"{NB}","cells":["{C1}"],"codes":["y = 2"],"comment":"why so slow?"}}"#);
        assert_eq!(
            parse_with(&body, ""),
            Some(Message::Ask(Ask { text: "why so slow?".into(), attachment: Attachment::Cells { notebook: NB.into(), cells: vec![cell("y = 2")], ask: CellAsk::About }, now: false }))
        );
        let region = format!(
            r#"{{"type":"region","notebook":"{NB}","cells":["{C1}"],"codes":["scatter(t, y)"],"comment":"what's this bump?","now":false,"rect":{{"x":12.5,"y":80,"width":300,"height":140}}}}"#
        );
        assert_eq!(
            parse_with(&region, ""),
            Some(Message::Region(Region { text: "what's this bump?".into(), notebook: NB.into(), cells: vec![cell("scatter(t, y)")], rect: [12.5, 80., 300., 140.], now: false }))
        );
        let flat = format!(r#"{{"type":"region","notebook":"{NB}","cells":[],"comment":"","rect":{{"x":0,"y":0,"width":0,"height":40}}}}"#);
        assert_eq!(parse_with(&flat, ""), None, "a box needs an area");
        assert_eq!(parse_with(&format!(r#"{{"type":"code","cell":"{C1}","code":"y = 3"}}"#), ""), Some(Message::Code { cell: C1.into(), code: Some("y = 3".into()) }));
        assert_eq!(parse_with(&format!(r#"{{"type":"code","cell":"{C1}","code":null}}"#), ""), Some(Message::Code { cell: C1.into(), code: None }));
    }

    #[test]
    fn rejects_forged_or_malformed_messages() {
        let bad_cell = format!(r#"{{"type":"annotation","notebook":"{NB}","cells":["../../x"],"comment":""}}"#);
        let no_cells = format!(r#"{{"type":"annotation","notebook":"{NB}","cells":[],"comment":""}}"#);
        let bad_nb = format!(r#"{{"type":"annotation","notebook":"nope","cells":["{C1}"],"comment":""}}"#);
        for body in [bad_cell.as_str(), no_cells.as_str(), bad_nb.as_str(), "not json", r#"{"type":"other"}"#] {
            assert_eq!(parse_with(body, ""), None, "{body}");
        }
    }

    #[test]
    fn parses_the_notebook_panes_messages() {
        let state = format!(
            r#"{{"type":"state","notebook":"{NB}","safe":false,"busy":"Installing packages · 2 of 5","restart":"sometime","save_failed":true,"package_failed":"Plots","dead":false,"connected":false,"drawer":"status"}}"#
        );
        assert_eq!(
            parse_with(&state, ""),
            Some(Message::State(PageState {
                notebook: NB.into(),
                safe: false,
                busy: Some("Installing packages · 2 of 5".into()),
                restart: None,
                save_failed: true,
                package_failed: Some("Plots".into()),
                dead: false,
                connected: false,
                drawer: Some("status".into()),
            }))
        );
        assert_eq!(parse_with(&format!(r#"{{"type":"run_notebook","notebook":"{NB}"}}"#), ""), Some(Message::RunNotebook { notebook: NB.into() }));
        assert_eq!(parse_with(&format!(r#"{{"type":"restart","notebook":"{NB}"}}"#), ""), Some(Message::Restart { notebook: NB.into() }));
        assert_eq!(
            parse_with(&format!(r#"{{"type":"fix_package","notebook":"{NB}","name":"Plots","log":"✗ Plots"}}"#), ""),
            Some(Message::FixPackage { notebook: NB.into(), name: "Plots".into(), log: "✗ Plots".into() })
        );
        assert_eq!(parse_with(r#"{"type":"run_notebook","notebook":"x"}"#, ""), None);
    }

    #[test]
    fn caps_comment_length() {
        let long = "x".repeat(MAX_COMMENT + 10);
        let body = format!(r#"{{"type":"annotation","notebook":"{NB}","cells":["{C1}"],"comment":"{long}"}}"#);
        let Some(Message::Ask(a)) = parse_with(&body, "") else { panic!() };
        assert_eq!(a.text.len(), MAX_COMMENT);
    }
}
