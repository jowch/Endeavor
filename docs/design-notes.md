# Endeavor design notes

Ideas still being explored, to review together before anything is built.
Settled work goes to [roadmap.md](roadmap.md); this file holds the ideas, the
options and the open questions. The design mockups (review in place, per-hunk
accept, ghost cells, Fix with Claude) are the current reference.

_Started 2026-09-25_

## Edited but not run

Claude's edits can be marked as "edited, not yet run" in the notebook. Pluto
can't do it alone, so Endeavor and PlutoMCP each need a small change.

- **Why Pluto shows nothing today.** Pluto's "edited" look (`code_differs`) is
  browser-only: the editor's text differs from the server's copy. Claude edits
  the server's copy through PlutoMCP, so the browser sees no difference.
- **Endeavor side.** PlutoMCP already tracks edited-but-unrun cells
  (`pending_run`). Our injected page script (`src/annotate.js` today) adds a
  class to those cells and re-applies it when Pluto redraws; the list refreshes
  after each turn and on the 10 s poll.
- **PlutoMCP side.** Pending clears only when Claude's own run tools run a cell.
  A run from Pluto's run button leaves the cell marked, and Claude gets a false
  "left unrun" warning. Fix: record the edit time and treat the cell as run once
  Pluto's `last_run_timestamp` is newer (catches a run from anywhere).
- **Pluto's run button** on a marked cell runs it immediately, without the
  approval prompt. The gate is for runs Claude starts, so that seems right.

Open: reuse Pluto's own `code_differs` look (no new CSS), or a distinct
"Claude edited" look (our own CSS, owned by the style system)? Being explored
with the design agent. Either way we depend on Pluto's internal class and
element names, as annotation mode already does; we pin Pluto, so breakage shows
up on upgrade.

## Proposals: review in place

From the mockup: Claude's changes arrive as proposals, shown as inline diffs with
per-hunk Accept / Reject, suggested new cells as ghost cells (Add / Dismiss), and
a "2 proposals · Reject all · Accept all" bar.

The key decision is where a proposal lives:

| | A. Apply, then revert on reject (recommended) | B. True staging |
|---|---|---|
| Pluto | Edit lands (not run); UI shows a diff against the pre-Claude code | Unchanged until accepted |
| Reject | Endeavor writes the old code back via PlutoMCP | Drop the proposal |
| Per-hunk | Rebuild the code with that hunk undone | Apply only accepted hunks |
| Claude's view | What it reads is what it wrote | Two versions to reason about; tools need redesign |
| File on disk | Changes at once (Pluto autosaves); a quit keeps proposals | Unchanged until accepted |

Start with A and word the UI as "not run yet" rather than "nothing changed";
revisit B if the file changing on disk matters in practice.

How the pieces map:

- **Inline diff with per-hunk controls.** A page-script overlay covers the cell's
  editor while a proposal is open. We can't insert deleted lines inside Pluto's
  CodeMirror (it redraws its own DOM). Syntax colouring needs a small Julia
  highlighter in our bundle. Old code per cell is already tracked for chat diffs
  (`celldiff.rs`).
- **Ghost cells.** Under A the cell already exists in Pluto, not run; the page
  script draws it as a ghost. Dismiss deletes it; Add accepts it.
- **Accept all / Reject all** come from the same pending list.
- **Shortcuts** (⌘Y / ⌘N in the mockup) live in the page script too.

## Propose / Edit / Auto

The mockup's three modes extend the execution gate:

- **Auto**: edits apply and run without asking (today's "Allow & stop asking" /
  the Settings default).
- **Edit**: edits apply; runs ask (today's behaviour).
- **Propose**: edits need review (accept/reject) before they count; runs ask.

Open: per session or per notebook? Does Accept in Propose mode also run the cell?

## Smaller ideas

- **Fix with Claude / Explain** on Pluto's error box: a button sends the cell
  link and error text through the chat queue, as annotation mode does. Easy.
- **"@ a cell" in the input**: completion over the notebook's cells, inserting
  the same `pluto://notebook/…/cell/…` links annotations use. Easy.

## Streaming edits and "Claude is editing"

Mockup: a cell with a striped border, "Claude is editing", a Claude cursor and a
Take over button.

- Claude writes a tool call (`{notebook_id, cell_id, code}`) token by token, so
  the new code really does stream, but as a whole-cell rewrite, not keystrokes.
- The ACP adapter forwards partial tool input only per completed top-level
  field. So "Claude is editing *this cell*" (once `cell_id` is written) is
  possible today; the code itself arrives all at once.
- Live typing needs the adapter to forward the growing `code` string (a small
  upstream or pinned patch), then a preview overlay in the cell. Take over =
  cancel the turn; the preview vanishes and the cell is untouched.

Deprioritized: Claude is fast, so there's little to watch. The presence marker
may still be worth it later.

## Frontend tech

- Today: one injected script, `src/annotate.js` (122 lines), embedded with
  `include_str!`.
- Most of the ideas above are page-script work, so move to **TypeScript +
  esbuild** (`frontend/src/*.ts` → one bundle, as in Masque.jl) when they start.
  Commit the built bundle so `cargo build` doesn't need Node, or run esbuild
  from `build.rs`.
- Until then: `// @ts-check` + JSDoc types on the plain JS.
- **Not WASM**: this is DOM work in Pluto's page; WASM needs JS glue for every
  DOM call, is bigger, slower to build and harder to debug. Its one benefit
  (shared types with the app's `annotate.rs` messages) is cheaper to get by
  mirroring small message shapes.

## Open questions

- Native Pluto "edited" look vs a distinct Claude look?
- Proposals: is apply-then-revert acceptable given the file changes on disk?
- Modes: per session or per notebook; does Accept run?
- Does the style system own the page-script CSS too?

## Suggested order

1. TypeScript + esbuild frontend.
2. Edited-but-not-run marking, with the PlutoMCP run-from-anywhere fix.
3. Fix with Claude / Explain.
4. Proposals: inline diffs, per-hunk accept/reject.
5. Ghost cells.
6. Propose / Edit / Auto modes.
