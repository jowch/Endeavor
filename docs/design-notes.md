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

## The notebook view in Claude's desktop app

`frontend/src/view.ts` and the files it uses are EndeavorMCP's notebook view
in Claude's desktop app. What the desktop app does, as probed on a Mac in
November 2026, decides how the view works:

- **Messages.** The view's message goes into Claude's message box for the
  user to send, after anything they typed, with its leading whitespace
  trimmed. So a visible mark (an arrow) starts it and keeps its newlines; a
  dash read to Claude as the user's own text. Claude takes at most 8,000
  characters, at most 3 line breaks or 64 spaces and tabs in a row, and no
  invisible characters, and turns away anything else every time. The app only
  takes a view's message just after the user's press, and shows it after 2 to
  7 seconds, so the view says "Sending to Claude's chat…". Context for later
  turns (`ui/update-model-context`) never reached Claude in the Code tab, so
  it isn't used.
- **Quotes.** Unlike the app's, the view's quotes name what was picked and
  carry no code or output, except lines the user picked: a long cell's code
  went over 8,000 characters (#328), and Claude reads the rest with its tools.
- **Pictures.** The frame can't take a screenshot, so `crop.ts` draws a box's
  images, SVGs and canvases itself. Text, HTML, WebGL drawn without
  `preserveDrawingBuffer` (Makie's and plotly's 3D views) and images from
  another site don't come out. An SVG is drawn from a copy on its own, so the
  page's CSS and web fonts don't reach it. The picture's id goes in the
  message at once and the picture follows, since Claude reads it only after
  the user sends.
- **Point** ends once the message is on its way and the view has lost the
  keyboard: the user sends from Claude's box, where the view's keys don't
  reach.
- **Find.** Desktop's ⌘F searches Desktop's page, not the view's frame, so
  the view has its own Find.
- **Links.** The frame may not open windows, so links go through the host
  (`ui/open-link`). Any other link that loads a page would replace the view
  with a blank one, since the frame's page has no address of its own.
- **Space.** Claude's side panel is about 513 by 721 px, down to 320 wide,
  and reports no safe-area insets. Measured at 320 px, Pluto's own margins
  left 226 px of code (Ember's 170). So the view keeps only what the cells
  need: the room for the eye (and Ember's run button) left of a cell, the run
  time and add-cell buttons between cells, two digits of line numbers (Point
  shows them), and room under the last cell for Point's bar. From 760 px wide
  it is full screen: a wider centred column, with Docs and Status as a sheet
  down the right. Markdown headings are smaller and long words wrap.
  `space.ts` moves the column to the left edge while a sheet is open; that
  rule's selector must outweigh the full-screen column rule's. The column rule
  is (1,2,2), since its `:has(.plutoui-toc.aside)` counts two classes; the
  sheet rule is (1,2,4). Simplifying either selector can silently flip which
  one wins.
- **Palette.** While the host sends its style variables, the page takes
  Claude's backgrounds, greys and borders, keeping Endeavor's accent, code and
  diff colours.
- **Completions and new cells.** Claude's edits land while a cursor sits in a
  cell, and Pluto's completion list stayed up over the notebook for as long
  as Claude worked, so the view closes a list nobody is using. A cell Claude
  adds is brought into view while the user follows along.
- **CodeMirror** scrolls the cursor into view by the window's top edge and
  ignores `scroll-padding`, so the view keeps the cursor below its toolbar
  itself.

## Page and app messages

The notebook page and the app talk over one channel, `frontend/src/bridge.ts`.
The page sends `ToApp` messages over wry's `window.ipc`, and `src/annotate.rs`
parses them. The app sends `ToPage` messages by evaluating
`window.__endeavor.receive(<json>)`, and each feature registers handlers with
`on` (several may listen to one type, such as `cells`). Claude's desktop
notebook view (`view.ts`) uses the same messages, with a host handler in place
of IPC.

**Only the app's own messages count.** Notebook output can run JavaScript, and
a message it posts could become a prompt. So the app puts a per-launch secret
in place of `__ENDEAVOR_NONCE__` and drops messages without it. bridge.ts is
an initialization script, so it captures the secret and webkit's
`postMessage` before any page script runs; code that later patches
`window.ipc` or webkit's handler gets neither. `byUser` accepts only trusted
events, so a script can't click or type a prompt into being. Tests run with no
secret and no host, and can't make trusted events, so there `byUser` lets
them through. `test/bridge.test.mjs` checks all three. The app also bounds what
a message may carry (`annotate.rs`), since notebook output runs in the same page,
and the user sees every quote before it is sent.

Page to app:

- `ready`, and `mode` (Point on or off).
- `quote`: what Point picked, or a selection Reply quotes, with the user's
  comment. It is sent now, or, with `add`, added to the composer's message.
  `ref` is set by a host that answers each ↩ (the desktop view), which echoes
  it back in `quote_sent`.
- `shoot`: take a picture of this viewport rect (CSS pixels) for a quote; the
  app answers `shot`.
- `ask`: Fix with Claude or Explain on a cell's error.
- `prompt`: ⌘E on a cell, or the agent button between cells. `where` is
  `about` the cell, `fill` an empty cell, or a new cell `before` or `after` it.
  Sent now (queued while the agent works) or, with `add`, added to the
  composer.
- `code`: a cell's code, answering the app's `code` (null when the cell isn't
  there).
- `state`: the shown notebook's state for the header, whenever it changes.
- `asked_visible`: whether a cell the chat's card asks to run is on screen.
- `run_notebook`: Run notebook in the safe-preview callout.
- `run_anyway`: the user's run reaches cells a card asks to run; the app
  answers those cards.
- `answer_card`: ⏎ in the page with nothing focused answers card `card`, as
  its filled button does.
- `fix_package` and `restart`: the Status failure box's buttons.
- `error_ask_show` and `error_ask_cancel`: on an error box the agent is
  answering, show its message in the chat, or take a queued one out.
- `debug`: answering the app's `debug` with what the page shows (Point and its
  picks, each pick's source as the app names it, the drawer's tab, the
  callout, Reply's pill or prompt, the open prompt, and the alerts shown).

App to page:

- `annotate` (Point on or off), and `quote_sent` (whether the desktop view's
  chat took quote `ref`; when it didn't, the picks and comment go back to
  Point).
- `cells`: the shown notebook's cells, whenever they change and after `ready`.
- `theme`: `endeavor`, `pluto`, or `view` (the Endeavor look inside the
  desktop view, keeping Pluto's own header and panels).
- `reveal`: scroll to these cells and outline them briefly (a chip in the chat
  was clicked).
- `code`: ask for a cell's code. `shot`: picture `id` is taken, or couldn't
  be, so the page shows its overlay again.
- `drawer`: open Live docs or Status, or shut the drawer (null).
- `context`: where the notebook runs (`host`), the session's agent (absent
  means Claude), whether the agent is asking to run (`asking`), `readonly`
  (the server is out of reach), `crash` (Julia stopped twice, so it opened in
  safe preview; the body names cells in backticks), the card's `ask_cells`,
  the `rerun_cells` after them and the never-run `needed_ids` that run first,
  `working` (the prompts' ↩ queues), `error_asks`, every waiting run card's
  cells and names (`waiting_runs`, so the user's own run asks before reaching
  one), and the waiting `card` that ⏎ answers. `main.ts` registers
  `initState`'s handler first, so every feature's own `context` handler reads
  the new agent from the same message.
- `action`: Share and ⋮ items that act in the page with Pluto's functions.
- `debug`: a debug build's state dump asks what the page shows.

## The agent's picture of an HTML output (`cell_shot.rs`)

When the runtime has no picture of a cell's output for `view_cell_output`
(an HTML or JS output: a widget, an interactive plot), the listener asks the
app for one (`endeavor_mcp::Snapshot`). The app takes it from a second web
view of the same notebook, behind the user's one, scrolled to the cell, so
the user's view never moves. A rect off screen in the user's own view comes
back blank; a second view scrolled to it draws HTML and WebGL alike, even a
canvas that drew once.

- The second view is another Pluto client of the notebook. It loads the page
  as a reload would, so what the user did inside a widget (a selection) isn't
  in the picture. It has none of the user's view's setup (page.js), so Pluto
  loads JuliaMono from its CDN there: offline, code in a picture is in
  another monospace font. It hides what floats over the page (Pluto's bars,
  Live docs), since it is only for pictures (#163).
- It is placed exactly behind the user's view, with the same holes, and only
  while the user's view is on screen and not dimmed: anything over the user's
  view would show the second one through it. It is let go after a while with
  no picture, so it isn't a Pluto client for the rest of the day.
- On a Mac it goes directly under the user's view among its siblings: both are
  built on the same parent, and the one added last starts on top (#259). macOS
  ends a hidden view's process first when memory is short; that is this
  view's own, and the user's notebook isn't loaded again.
- On Linux it is an X child window stacked under the user's: WebKitGTK draws a
  hidden view's snapshot too, but runs no animation frames there, so a plot
  drawn from one would be blank. It is shown to GTK from the start and hidden
  by unmapping its X window, since a view shown again through GTK keeps the
  size GTK picks, not the pane's.
- On Windows it is a child window just under the user's, shown without
  activating (not wry's `set_visible`, whose SW_SHOW may activate), so the
  keyboard stays in the user's cell. The picture is WebView2's
  (`snapshot_windows.rs`).
- The picture waits two frames after the scroll, and on a page just loaded
  long enough for a widget that draws after it (WebGL).

## Julia or R stopping under a notebook (`crash.rs`)

The pane says so, with Restart Julia, which reopens the notebook and runs it
again. If Julia stops again during that run, the notebook opens in safe
preview instead, so a cell that takes Julia down can't do it in a loop. A
stop heard just after a restart is the restart's own, and a rerun never
seen running is taken as done after a while (an idle R notebook's list
doesn't change), so a stop after that is a first one.

A notebook's own Julia (Pluto runs each notebook in a process of its own) is
seen to stop in the runtime's notebook list (`exited`, with the cells that
were running) and through its page; the cell is remembered from while Julia
ran, since the list after a stop can't say. All of a host's Julia stopping is
heard from its helper, for each notebook that was running there.

R ending works the same way: the runtime closes every R notebook with it and
says why (`engine_ended`), and Start R again opens the notebook again,
running it only if it was allowed to run before, so one in safe preview
doesn't run without the user.

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
