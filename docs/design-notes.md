# Endeavor design notes

Implementation notes behind [ui-spec.md](ui-spec.md), which is the source of
truth for how the UI looks and behaves. This file holds what the spec needs
underneath (mostly the runtime), the technical risks, and decisions with their
reasons. Settled, scheduled work goes to [roadmap.md](roadmap.md).

_Started 2026-09-25_

## Decisions

- **Plan mode, not Propose (2026-09-25).** Review-before-apply proposals
  (per-hunk Accept / Reject, ghost cells, "Accept all") were explored and
  dropped: approving every edit is heavy, and agents are moving the other way.
  Edits land live; diffs show what changed; approval is for **runs**. Plan mode
  covers "don't touch anything yet": read-only, ending in a plan to approve.
  Modes are Manual / Ask to run / Auto / Plan, per the spec.
- **Own runtime, Endeavor-only (2026-09-25).** See Runtime below.
- **Streaming edits deprioritized.** Claude writes whole-cell rewrites fast;
  there's little to watch. (The ACP adapter forwards partial tool input only per
  completed field, so a live "typing" preview would also need an adapter patch.)

## Unrun cells and stale output

The spec marks cells whose code differs from what last ran. Pluto can't supply
this for the agent's edits:

- Pluto's own "edited" state (`code_differs`) is browser-only: the editor's text
  vs the server's copy. That covers the **user's** unsubmitted typing
  (Pluto's grey bar), but the agent edits the server's copy, so the browser sees no
  difference.
- So the runtime works it out: an agent edit records its time, and the cell is
  unrun until Pluto's `last_run_timestamp` is newer. Submitting code in Pluto's
  UI runs it at once, so only the agent's tools can leave a cell unrun. A run
  from anywhere (Pluto's button included) clears it. The state belongs to the
  open notebook and goes when it shuts down, so a restart clears it too.
- The runtime pushes the set to the app, and the bundle sets `data-endeavor` /
  `data-author` on the cells.

Pluto's run button on an agent-edited cell runs it without the approval card.
That's intended: the gate is for runs the agent starts. But a user's run
(⌘S, ⇧⏎, ⌘⏎, the run button) also re-runs the agent-edited cells downstream
of it, and when a card is waiting to run one of those it would run unasked and
leave the card asking about something that already ran. So
`frontend/src/runguard.ts` catches those in the capture phase, before Pluto,
works out what they reach from `editor_state.notebook.cell_dependencies`
(`downstream_cells_map`, transitively), and asks first when that includes a
cell in the context's `waiting_runs` (every waiting run card's cells). Pluto's
actions aren't reachable from the page, so Run anyway replays the user's event
on its original target with the guard off. It answers the cards (`run_anyway`
→ `Session::allow_runs_of`) only once the cells are queued or running in
Pluto's state, so the approved call reaches the runtime after the user's run
has them. Before answering, the app tells the runtime (`endeavor/run_anyway`)
which cells the user's run reached, with each one's `last_run_timestamp` from
before it (`user_runs` in `NotebookState`, used once and for a minute at
most): a cell the tools never edited has no edit time to compare with, and
without this its approved run would run it, and everything after it, a
second time. The runtime then sees each target run (or running, which it
waits for) after the tools' edit or after that timestamp, and answers with
the usual receipt and an `already_ran::` warning instead of running them
again (`tool_edits` and `user_runs`). Runs the page
can't catch (the cell's ⋯ menu, a run started from another window) are covered
by the app: when every cell of a waiting `execute_cell` / `submit_changes`
card goes from unrun to run in the `/events` stream, the card is answered as
allowed with a note.

## Diffs in the CodeMirror gutter

`frontend/src/diff.ts`. Pluto serves a Parcel bundle, so
its CodeMirror can't be imported (a second copy's extensions don't compose).
The classes come from the live editor instead:

- the view: `.cm-content`'s `cmTile.root.view` (what `findFromDOM` reads);
- `StateEffect` from `EditorView.scrollIntoView(0).constructor` (for
  `appendConfig`); `Decoration` as the base class of any existing decoration;
  `Compartment` from `state.config.compartments` (Pluto uses them);
- the removed-line widget is duck-typed (`WidgetType` isn't reachable).

Each diffed editor gets its own compartment holding a static decoration set
(block widgets can't come from a function source), swapped when the runtime's
`before` changes, and re-diffed in the same transaction when the user types
(`transactionExtender`). The before-text is the runtime's: the cell's code
before the agent's first edit since it last ran, forgotten when it runs.
Relies on CodeMirror internals (`cmTile`, `config.compartments`); Pluto is
pinned, so breakage shows on upgrade. Pluto also has a built-in unified diff
(`ai_suggestion.js`, the "ai-suggestion" DOM event), but it edits the cell's
text, so it doesn't fit edits that already landed.

## Plan mode

- `claude-agent-acp` offers modes Manual (`default`), Accept edits, Plan,
  Auto and Bypass permissions; **sessions start in Auto**. It also exposes
  config options `mode`, `model`, `effort`, `fast`. A mode switch is confirmed
  by a `config_option_update` for `mode` (a `current_mode_update` only when it
  falls back to another mode).
- **Mapping:** Manual → the agent's `default`; Ask to run and Auto → the
  agent's `auto`, with our run gate asking or not (`run_without_asking`);
  Plan → the agent's `plan` (`app_modes`, `src/session.rs`). The plan card's
  Start / Start in Auto both pick the adapter's `exit-plan-auto` and set the
  gate. Claude Code asks before any MCP tool without an allow rule, so the
  read-only notebook tools are let through by `allowedTools` in every mode.
  So are reads of the plugin's own files (a `Read(//…/plugin/**)` rule, with
  the path in Claude Code's form: `//c/Users/…` on Windows): Claude Code
  doesn't allow a plugin's files by itself, and the skills point the agent at
  reference files beside them.
- Claude Code's plan mode restricts its own write tools, but notebook edits go
  through our MCP tools, so the runtime enforces read-only too: in Plan, the
  runtime refuses edits and runs. Same for Ask to run vs Auto, and for
  Manual's edits: the runtime's policy decides, not the agent's settings.

## Frontend tech

- `frontend/` is TypeScript, built with esbuild into one bundle,
  `dist/page.js`, which is committed so `cargo build` doesn't need Node
  (as in Masque.jl). `src/annotate.rs` embeds it. `npm test` runs it in jsdom.
- `bridge.ts` is the two-way channel: page → app over `window.ipc` (typed
  `ToApp`), app → page through `window.__endeavor.receive(msg)` (typed
  `ToPage`; Rust `send_to_page`).
- **Not WASM**: this is DOM work in Pluto's page; WASM needs JS glue for every
  DOM call, is bigger, slower to build and harder to debug.
- The injected code depends on Pluto's internal DOM and classes; Pluto is
  pinned, so breakage shows up on upgrade. The spec's per-notebook-type
  adapter keeps that surface small.

## Runtime: our own replacement for PlutoMCP

**Decided:** Endeavor has its own runtime, seeded from the PlutoMCP fork (MIT;
keep its copyright notice with copied code). Anything generally useful can go
back to PlutoMCP.jl later. It first lived in this repo, changing in the same
commit as the app. It has since moved to
[EndeavorMCP](https://github.com/jowch/EndeavorMCP), which the app pins by
commit, so a runtime change lands there first and the app then moves its pin
(CLAUDE.md). The app and the runtime check each other's version
(`interface` in `runtime.json`).

What changed relative to PlutoMCP:

- **Kept:** tool behaviour (read/edit/add/move/delete/validate/search/run,
  `view_cell_output`), the dependency graph (the spec's "also re-runs N" and
  cell labels use it), notebook summaries, and the test cases that encode
  Pluto quirks.
- **Dropped:** attaching to a Pluto the user started, start/stop tools,
  binding files and health checks. Endeavor owns the process.
- **Replaced:** the unauthenticated SSE bridge with an authenticated one (a
  random per-launch bearer token), and the `pending_run` side table with
  derived staleness (above).
- **Added:** pushed events (`/events`) driven by Pluto's `on_event` hook,
  run policy and approvals in the runtime (agent-agnostic, replacing a
  Claude hook), cell versions, and attribution (agent vs user, for
  `data-author`), with each cell's code from before the agent's edit
  (`before`).

No Pluto changes are needed: the runtime creates Pluto's session, so it sets
`on_event` and reads cell state directly. The cost is depending on Pluto
internals, so Pluto is pinned and the Pluto-touching code stays in one place.
The language-neutral half has since moved into a Rust core, with Julia as
the Pluto adapter: see [runtime-core.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/runtime-core.md).

## Provenance and reproducibility (to revisit)

Idea (2026-09-25): the notebook, not an execution log, as the record of AI-assisted
analysis. JSONL/ipython-style logs record what ran, but humans can't verify them
without an LLM, and they separate artifacts (figures, tables) from the code that
made them. A Pluto notebook is reactive (what's on screen is a pure function of
the code on screen: no hidden or out-of-order state), embeds its package
environment, and keeps outputs next to the code. What it lacks is the *why* and
the *history*, which the runtime's events and attribution start to capture.
Handling this bookkeeping early and well is evidence for how the work was done.

Ideas, roughly by value for effort:

1. **Turn-level history in git.** After each agent turn (and on user saves),
   commit the notebook to a history branch or `.endeavor/history`: message = the
   prompt, body = cells changed and runs. Readable with ordinary tools; abandoned
   attempts stay in history. Built on runtime events + attribution.
2. **Cell provenance on hover.** Who changed the cell, when, and why (the turn's
   request, linked), with the previous version. Runtime attribution + a turn link.
3. **Figures that carry provenance.** Exported figures/tables stamped (PNG text
   chunks, SVG/PDF metadata) with notebook path, cell, history commit,
   environment hash and input-data hashes, so an artifact outside the notebook
   still traces back to exact code, environment and data.
4. **Reproduce button.** Re-run in a fresh process from the embedded environment
   and compare every output with the record (exact for text/tables, tolerance for
   numbers, hash for images). Catches unseeded randomness and outside
   dependencies. Reactivity makes "the same notebook" well defined.
5. **Data inputs.** Record files each cell read (content hash, size) at run time,
   and warn when they change. Cheap: static detection of literal paths via
   Pluto's parsed cell code; robust: a `data("…")` helper the agent uses.
6. **Results that can't drift.** Have the agent write findings as interpolated
   markdown (`md"Km = $(round(Km; digits=2))"`) instead of pasted numbers (a line
   in the Pluto skills); the runtime can flag prose with hardcoded numbers that
   also appear in outputs.
7. **Lab-notebook log.** A readable, append-only per-notebook record (request,
   what changed, what ran, which outputs changed), generated from events with no
   LLM: the readable companion to the git history.

1, 2 and 7 rest on the runtime's events and attribution; 3 and 4 are what an
outside reviewer would find most convincing.

## Open questions

- Undo granularity (from the spec).

Answered: Plan mode doesn't stop the user's own runs. The runtime refuses
writes and runs only for the agent's notebook tools in a session in Plan
(EndeavorMCP's `mcp.rs`); a run from the notebook page goes ahead.
