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
  Modes are Plan / Ask to run (default) / Auto, per the spec.
- **Own runtime, Endeavor-only (2026-09-25).** See Runtime below.
- **Streaming edits deprioritized.** Claude writes whole-cell rewrites fast;
  there's little to watch. (The ACP adapter forwards partial tool input only per
  completed field, so a live "typing" preview would also need an adapter patch.)

## Unrun stripe and stale output

The spec marks cells whose code differs from what last ran. Pluto can't supply
this for the agent's edits:

- Pluto's own "edited" state (`code_differs`) is browser-only: the editor's text
  vs the server's copy. That covers the **user's** unsubmitted typing
  (`you.stripe`), but the agent edits the server's copy, so the browser sees no
  difference.
- So the runtime works it out: an agent edit records its time, and the cell is
  unrun until Pluto's `last_run_timestamp` is newer. Submitting code in Pluto's
  UI runs it at once, so only the agent's tools can leave a cell unrun. A run
  from anywhere (Pluto's button included) clears it; reopening a notebook runs
  everything, so a restart clears it too.
- The runtime pushes the set to the app, and the bundle sets `data-endeavor` /
  `data-author` on the cells.

Pluto's run button on an agent-edited cell runs it without the approval card.
That's intended: the gate is for runs the agent starts.

## Diffs in the CodeMirror gutter (risk)

The spec draws unified diffs inside Pluto's editor (line tints, changed
characters, removed lines above additions) as CodeMirror decorations. That means
adding extensions to Pluto's own CodeMirror instances, which only works with the
**same** CodeMirror build Pluto bundles (extensions from a second copy don't
compose). Prototype this first: find how to reach Pluto's CodeMirror modules and
each cell's `EditorView` (the DOM exposes the view), and add an extension via a
compartment. Fallback: an overlay that covers the editor while a diff is shown.
Before-text per cell is already kept for the chat's diffs (`celldiff.rs`).

## Plan mode

- `claude-agent-acp` exposes Claude Code's session modes (plan among them) over
  ACP; the app currently drops modes (spec: Data wiring). ⇧⇥ cycles them via
  `session/set_mode`.
- Claude Code's plan mode restricts its own write tools, but notebook edits go
  through our MCP tools, so the runtime must enforce read-only too: in Plan, the
  runtime refuses edits and runs (runtime step 5). Same for Ask to run vs Auto:
  the runtime's policy, not the Claude hook, decides.

## Frontend tech

- Today: one injected script, `src/annotate.js` (122 lines), embedded with
  `include_str!`, plus fire-and-forget `evaluate_script`.
- The spec's bundle (gutter diffs, pointing overlay, ⌘K prompt, agent "+"
  button, overview rail, user-edit reporting) and the two-way Rust↔JS channel
  make this real frontend work: **TypeScript + esbuild** (`frontend/src/*.ts` →
  one bundle, as in Masque.jl). Commit the built bundle so `cargo build` doesn't
  need Node, or run esbuild from `build.rs`.
- **Not WASM**: this is DOM work in Pluto's page; WASM needs JS glue for every
  DOM call, is bigger, slower to build and harder to debug.
- The injected code depends on Pluto's internal DOM and classes (as annotation
  mode already does); Pluto is pinned, so breakage shows up on upgrade. The
  spec's per-notebook-type adapter keeps that surface small.

## Runtime: our own replacement for PlutoMCP

**Decided:** Endeavor gets its own Julia runtime package, seeded from the
PlutoMCP fork (MIT; keep its copyright notice with copied code) and living in
this repo (e.g. `runtime/EndeavorRuntime/`, a path dependency). Endeavor-only
for now; anything generally useful can go back to PlutoMCP.jl later. App and
runtime change in the same commit: no fork PR, merge and pin cycle, and no
protocol to keep backward-compatible.

What changes relative to PlutoMCP (~3,400 lines, 29 tools):

- **Keep:** tool behaviour (read/edit/add/move/delete/validate/search/run,
  `view_cell_output`), the dependency graph (the spec's "also re-runs N" and
  cell labels use it), notebook summaries, and the test cases that encode Pluto
  quirks.
- **Drop:** attaching to a Pluto the user started, start/stop tools, binding
  files and health checks (~1,000 lines). Endeavor owns the process.
- **Replace:** the unauthenticated SSE bridge and `/call` with an authenticated
  channel (token or Unix socket), and the `pending_run` side table with
  derived staleness (above).
- **Add:** pushed events via Pluto's `on_event` hook (`StateChangeEvent`,
  `NotebookExecutionDoneEvent`, open/shutdown), run policy and approvals in the
  server (agent-agnostic, replacing the Claude hook), cell versions (code
  hashes), attribution (agent vs user, for `data-author`).

No Pluto changes are needed: the runtime creates Pluto's session, so it sets
`on_event` and reads cell state directly. The cost is depending on Pluto
internals, so pin Pluto and keep the Pluto-touching code in one module.

Staged, each step shippable:

1. **Parity (done 2026-09-25):** `runtime/EndeavorRuntime`, seeded from the fork
   at 918e75d: 25 tools (start/stop session and `resolve_pluto_context` dropped),
   no binding, eval-log or external-client entry points (~1,900 lines fewer);
   tests ported (290 passing) and made to use a temp copy of their fixture.
2. **Security (done 2026-09-25):** the bridge requires `Authorization: Bearer`
   with a random per-launch token (app → Julia via `ENDEAVOR_TOKEN`, removed from
   Julia's environment before notebooks start; the agent's MCP config carries
   the header). Closes the shared-host hole. Pluto's own port stays protected
   by its secret. Still open: SSH remote sessions (forward both ports).
3. **Events (done 2026-09-25):** `GET /events` (server-sent events, same token)
   pushes the `list_notebooks` summary whenever it changes, driven by Pluto's
   `StateChangeEvent` / `NotebookExecutionDoneEvent` / open / shutdown and by
   every tool call. The app follows it for crash reopen and the end-of-turn
   run warning; the 10 s poll and the post-turn call are gone. Cell-level
   events (for the spec's cell states) extend this stream.
4. **Derived staleness:** the unrun set from edit time vs
   `last_run_timestamp`, pushed with the events.
5. **Policy in the server:** Plan / Ask to run / Auto per session; approvals
   (with the dependents count) pushed to the app; retire the Claude plugin hook.
6. **Versions and attribution:** code-hash versions (conflict detection when
   user and agent edit the same cell), author per change, and the before-text
   an undo would restore (the spec's open undo decision).

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
   request, linked), with the previous version. Runtime step 6 + a turn link.
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

1, 2 and 7 rest on runtime steps 3 and 6; 3 and 4 are what an outside reviewer
would find most convincing.

## Open questions

- Chat-panel radius and undo granularity (from the spec).
- Can the bundle reach Pluto's CodeMirror build (gutter-diff prototype)?
- Does GPUI do backdrop blur (spec's header risk)?
- Mode per session (spec) — and does Plan mode also stop the user's own runs?
  (Presumably not; it constrains the agent.)

## Order

The spec's phases, with the runtime steps they depend on:

1. Spec phase 1 (theme module, native restyle). No runtime dependency.
2. Runtime steps 1–2 (parity, security).
3. Spec phase 2 (theme CSS, adapter, cell states, overview rail), on runtime
   steps 3–4 (events, staleness).
4. TypeScript + esbuild frontend and the Rust↔JS channel; spec phase 3
   (bundle), starting with the gutter-diff prototype.
5. Spec phase 4 (ACP wiring: modes, models, usage, commands, plan pinning), with
   runtime step 5 (policy) for Plan / Ask to run / Auto.
6. Runtime step 6 when undo is decided.
