//! Messages from the page script (frontend/, Point, Reply on a selection and
//! asking about cells), turned into chat messages with their attachments
//! (design doc §4.2–4.3).

use crate::attach::{Attachment, Cell, CellAsk, Part, Quoted};
use crate::session::defined_name;
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

/// The words Fix with Claude and Explain send with a cell's error.
pub const FIX_ERROR: &str = "Fix the error in this cell.";
pub const EXPLAIN_ERROR: &str = "Explain this error; don't change anything yet.";

/// A message the user sent from the notebook: their words, and what it's about.
#[derive(Debug, PartialEq)]
pub struct Ask {
    pub text: String,
    pub attachment: Attachment,
    /// Add to the composer's message (the prompt's ⌘↩) instead of sending it.
    pub add: bool,
}

#[derive(Debug, PartialEq)]
pub enum Message {
    /// The page script loaded and wants the current cell states.
    Ready,
    Mode(bool),
    Ask(Ask),
    /// Picks with Point, or Reply on a selected text, with the user's comment.
    Quote(Picks),
    /// Take a picture of this part of the page's viewport (x, y, width,
    /// height in CSS pixels) for a quote to come, and say "shot" `id` when done.
    Shoot { id: u32, rect: [f64; 4] },
    /// A cell's code now (None: the page has no such cell), as the app asked.
    Code { cell: String, code: Option<String> },
    /// The shown notebook's state, for the notebook header.
    State(PageState),
    /// Run notebook, in the safe-preview callout.
    RunNotebook { notebook: String },
    /// Whether the cells the chat's card asks to run are on screen.
    AskedVisible(bool),
    /// Run anyway, when the user's own run reaches cells a card asks to run:
    /// allow the cards asking about `cells`, as their Run button does.
    RunAnyway { notebook: String, cells: Vec<String> },
    /// Fix with Claude, in Status's box for a package that failed.
    FixPackage { notebook: String, name: String, log: String },
    /// Restart notebook, in the same box.
    Restart { notebook: String },
    /// "Show in chat" on an error box whose Fix or Explain Claude is answering.
    ShowErrorAsk { cell: String },
    /// "Cancel" on an error box whose Fix or Explain waits in the queue.
    CancelErrorAsk { cell: String },
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

/// What the user quoted in the notebook, in the order picked: each quote,
/// and the picture (`Message::Shoot`'s id) that goes in it, for a figure or a
/// box. The comment belongs to the last quote.
#[derive(Debug, PartialEq)]
pub struct Picks {
    pub quotes: Vec<(Quoted, Option<u32>)>,
    pub comment: String,
    /// ⌘↩: add them to the composer's message instead of sending them now.
    pub add: bool,
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
    match v.get("type")?.as_str()? {
        "ready" => Some(Message::Ready),
        // Fix with Claude / Explain on a cell's error.
        "ask" => {
            let (notebook, id) = (uuid("notebook")?, uuid("cell")?);
            let text = match v.get("kind")?.as_str()? {
                "fix" => FIX_ERROR,
                "explain" => EXPLAIN_ERROR,
                _ => return None,
            };
            let cell = Cell { id, code: capped(v.get("code"), MAX_CODE) };
            let attachment = Attachment::Error { notebook, cell, text: capped(v.get("error"), MAX_COMMENT) };
            Some(Message::Ask(Ask { text: text.into(), attachment, add: false }))
        }
        // ⌘E on a cell, or the agent button between cells. Added to the
        // message, a question about a cell is a quote of the whole cell.
        "prompt" => {
            let (notebook, id) = (uuid("notebook")?, uuid("cell")?);
            let text = capped(v.get("text"), MAX_COMMENT);
            let add = v.get("add").and_then(|a| a.as_bool()).unwrap_or(false);
            let ask = match v.get("where")?.as_str()? {
                "about" => CellAsk::About,
                "fill" => CellAsk::Fill,
                "before" => CellAsk::Before,
                "after" => CellAsk::After,
                _ => return None,
            };
            let cell = Cell { id, code: capped(v.get("code"), MAX_CODE) };
            if add && ask == CellAsk::About {
                let name = defined_name(&cell.code).unwrap_or_else(|| "cell".into());
                let from = Quoted::Cell { notebook, cell: cell.id, name, part: Part::Whole(cell.code) };
                return Some(Message::Quote(Picks { quotes: vec![(from, None)], comment: text, add }));
            }
            Some(Message::Ask(Ask { text, attachment: Attachment::Cells { notebook, cells: vec![cell], ask }, add }))
        }
        "mode" => Some(Message::Mode(v.get("on")?.as_bool()?)),
        "quote" => {
            let notebook = uuid("notebook")?;
            let picks = v.get("picks")?.as_array()?;
            if picks.is_empty() || picks.len() > MAX_CELLS {
                return None;
            }
            let quotes = picks.iter().map(|pick| pick_quote(pick, &notebook)).collect::<Option<Vec<_>>>()?;
            let add = v.get("add").and_then(|a| a.as_bool()).unwrap_or(false);
            Some(Message::Quote(Picks { quotes, comment: capped(v.get("comment"), MAX_COMMENT), add }))
        }
        "shoot" => {
            let id = u32::try_from(v.get("id")?.as_u64()?).ok()?;
            let rect = v.get("rect")?;
            let n = |key: &str| rect.get(key)?.as_f64().filter(|n| (-MAX_SIDE..MAX_SIDE).contains(n));
            let rect = [n("x")?, n("y")?, n("width")?, n("height")?];
            if rect[2] < 1. || rect[3] < 1. {
                return None;
            }
            Some(Message::Shoot { id, rect })
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
        "asked_visible" => Some(Message::AskedVisible(v.get("visible")?.as_bool()?)),
        "run_anyway" => {
            let cells: Vec<String> = v.get("cells")?.as_array()?.iter().map(|c| c.as_str().filter(|s| is_uuid(s)).map(str::to_owned)).collect::<Option<_>>()?;
            (!cells.is_empty() && cells.len() <= MAX_CELLS).then_some(Message::RunAnyway { notebook: uuid("notebook")?, cells })
        }
        "restart" => Some(Message::Restart { notebook: uuid("notebook")? }),
        "error_ask_show" => Some(Message::ShowErrorAsk { cell: uuid("cell")? }),
        "error_ask_cancel" => Some(Message::CancelErrorAsk { cell: uuid("cell")? }),
        "fix_package" => Some(Message::FixPackage {
            notebook: uuid("notebook")?,
            name: capped(v.get("name"), 200),
            log: capped(v.get("log"), MAX_CODE),
        }),
        _ => None,
    }
}

/// One pick of a "quote" message: `part` (`cell`, `lines`, `output`, `figure`
/// or `box`), the cell and its code (a box: `cells`, the ids under it), the
/// quoted `text` and `lines` ([first, last]) where they apply, and `shot`, the
/// picture a figure or box takes its image from.
fn pick_quote(pick: &serde_json::Value, notebook: &str) -> Option<(Quoted, Option<u32>)> {
    let shot = pick.get("shot").and_then(|s| s.as_u64()).and_then(|s| u32::try_from(s).ok());
    let notebook = notebook.to_string();
    let png = || std::sync::Arc::new(Vec::new());
    let part = pick.get("part")?.as_str()?;
    if part == "box" {
        let cells: Vec<String> = pick.get("cells")?.as_array()?.iter().map(|c| c.as_str().filter(|s| is_uuid(s)).map(str::to_owned)).collect::<Option<_>>()?;
        if cells.len() > MAX_CELLS {
            return None;
        }
        return Some((Quoted::Box { notebook, cells, png: png() }, shot));
    }
    let cell = pick.get("cell")?.as_str().filter(|s| is_uuid(s))?.to_string();
    let code = capped(pick.get("code"), MAX_CODE);
    let text = || capped(pick.get("text"), MAX_CODE);
    let part = match part {
        "cell" => Part::Whole(code.clone()),
        "lines" => {
            let lines = pick.get("lines")?.as_array()?;
            let line = |i: usize| lines.get(i)?.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n >= 1);
            let (first, last) = (line(0)?, line(1)?);
            if last < first {
                return None;
            }
            Part::Lines { first, last, text: text() }
        }
        "output" => Part::Output(text()),
        "figure" => Part::Figure(png()),
        _ => return None,
    };
    let name = defined_name(&code).unwrap_or_else(|| "cell".into());
    Some((Quoted::Cell { notebook, cell, name, part }, shot))
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
                add: false,
            }))
        );
        assert!(matches!(ask("explain"), Some(Message::Ask(a)) if a.text.starts_with("Explain")));
        assert_eq!(ask("delete everything"), None);
        assert_eq!(parse_with(r#"{"type":"send"}"#, ""), None);
        let prompt = |place: &str, add: bool| {
            parse_with(&format!(r#"{{"type":"prompt","notebook":"{NB}","cell":"{C1}","code":"y = 2","where":"{place}","text":"plot it","add":{add}}}"#), "")
        };
        assert_eq!(
            prompt("after", false),
            Some(Message::Ask(Ask { text: "plot it".into(), attachment: Attachment::Cells { notebook: NB.into(), cells: vec![cell("y = 2")], ask: CellAsk::After }, add: false }))
        );
        assert!(matches!(prompt("fill", true), Some(Message::Ask(Ask { add: true, .. }))));
        // ⌘↩ on a question about a cell: the cell and the words become one quote card.
        assert_eq!(
            prompt("about", true),
            Some(Message::Quote(Picks {
                quotes: vec![(Quoted::Cell { notebook: NB.into(), cell: C1.into(), name: "y".into(), part: Part::Whole("y = 2".into()) }, None)],
                comment: "plot it".into(),
                add: true,
            }))
        );
        assert_eq!(prompt("anywhere", false), None);
        let picks = format!(
            r#"{{"type":"quote","notebook":"{NB}","comment":"why so slow?","add":true,"picks":[
                {{"part":"lines","cell":"{C1}","code":"s = sum(xs)\nt = 2","lines":[1,1],"text":"s = sum(xs)"}},
                {{"part":"figure","cell":"{C1}","code":"plot(xs)","shot":3}},
                {{"part":"output","cell":"{C1}","code":"","text":"0.42"}},
                {{"part":"cell","cell":"{C1}","code":"y = 2"}},
                {{"part":"box","cells":["{C1}"],"shot":4}}]}}"#
        );
        let in_c1 = |name: &str, part| Quoted::Cell { notebook: NB.into(), cell: C1.into(), name: name.into(), part };
        let png = || std::sync::Arc::new(Vec::new());
        assert_eq!(
            parse_with(&picks, ""),
            Some(Message::Quote(Picks {
                quotes: vec![
                    (in_c1("s", Part::Lines { first: 1, last: 1, text: "s = sum(xs)".into() }), None),
                    (in_c1("cell", Part::Figure(png())), Some(3)),
                    (in_c1("cell", Part::Output("0.42".into())), None),
                    (in_c1("y", Part::Whole("y = 2".into())), None),
                    (Quoted::Box { notebook: NB.into(), cells: vec![C1.into()], png: png() }, Some(4)),
                ],
                comment: "why so slow?".into(),
                add: true,
            }))
        );
        let shoot = r#"{"type":"shoot","id":3,"rect":{"x":12.5,"y":80,"width":300,"height":140}}"#;
        assert_eq!(parse_with(shoot, ""), Some(Message::Shoot { id: 3, rect: [12.5, 80., 300., 140.] }));
        let flat = r#"{"type":"shoot","id":3,"rect":{"x":0,"y":0,"width":0,"height":40}}"#;
        assert_eq!(parse_with(flat, ""), None, "a picture needs an area");
        assert_eq!(parse_with(&format!(r#"{{"type":"code","cell":"{C1}","code":"y = 3"}}"#), ""), Some(Message::Code { cell: C1.into(), code: Some("y = 3".into()) }));
        assert_eq!(parse_with(&format!(r#"{{"type":"code","cell":"{C1}","code":null}}"#), ""), Some(Message::Code { cell: C1.into(), code: None }));
    }

    #[test]
    fn rejects_forged_or_malformed_messages() {
        let quote = |notebook: &str, picks: &str| format!(r#"{{"type":"quote","notebook":"{notebook}","comment":"","picks":[{picks}]}}"#);
        let bad_cell = quote(NB, r#"{"part":"cell","cell":"../../x","code":""}"#);
        let no_cells = quote(NB, "");
        let bad_nb = quote("nope", &format!(r#"{{"part":"cell","cell":"{C1}","code":""}}"#));
        let backwards = quote(NB, &format!(r#"{{"part":"lines","cell":"{C1}","code":"","lines":[5,3]}}"#));
        let line_zero = quote(NB, &format!(r#"{{"part":"lines","cell":"{C1}","code":"","lines":[0,3]}}"#));
        let bad_part = quote(NB, &format!(r#"{{"part":"reply","cell":"{C1}","code":""}}"#));
        for body in [&bad_cell, &no_cells, &bad_nb, &backwards, &line_zero, &bad_part] {
            assert_eq!(parse_with(body, ""), None, "{body}");
        }
        for body in ["not json", r#"{"type":"other"}"#] {
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
        assert_eq!(parse_with(r#"{"type":"asked_visible","visible":false}"#, ""), Some(Message::AskedVisible(false)));
        assert_eq!(
            parse_with(&format!(r#"{{"type":"run_anyway","notebook":"{NB}","cells":["{C1}"]}}"#), ""),
            Some(Message::RunAnyway { notebook: NB.into(), cells: vec![C1.into()] })
        );
        assert_eq!(parse_with(&format!(r#"{{"type":"run_anyway","notebook":"{NB}","cells":["x"]}}"#), ""), None);
        assert_eq!(parse_with(&format!(r#"{{"type":"restart","notebook":"{NB}"}}"#), ""), Some(Message::Restart { notebook: NB.into() }));
        assert_eq!(parse_with(&format!(r#"{{"type":"error_ask_show","cell":"{C1}"}}"#), ""), Some(Message::ShowErrorAsk { cell: C1.into() }));
        assert_eq!(parse_with(&format!(r#"{{"type":"error_ask_cancel","cell":"{C1}"}}"#), ""), Some(Message::CancelErrorAsk { cell: C1.into() }));
        assert_eq!(parse_with(r#"{"type":"error_ask_cancel","cell":"x"}"#, ""), None);
        assert_eq!(
            parse_with(&format!(r#"{{"type":"fix_package","notebook":"{NB}","name":"Plots","log":"✗ Plots"}}"#), ""),
            Some(Message::FixPackage { notebook: NB.into(), name: "Plots".into(), log: "✗ Plots".into() })
        );
        assert_eq!(parse_with(r#"{"type":"run_notebook","notebook":"x"}"#, ""), None);
    }

    #[test]
    fn caps_comment_length() {
        let long = "x".repeat(MAX_COMMENT + 10);
        let body = format!(r#"{{"type":"quote","notebook":"{NB}","picks":[{{"part":"cell","cell":"{C1}","code":""}}],"comment":"{long}"}}"#);
        let Some(Message::Quote(picks)) = parse_with(&body, "") else { panic!() };
        assert_eq!(picks.comment.len(), MAX_COMMENT);
    }
}
