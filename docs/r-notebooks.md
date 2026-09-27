# R notebooks

How Endeavor supports reactive R notebooks. The notebook engine itself is
**turtleR** (working name), a standalone R package in its own repository,
https://github.com/jowch/turtleR; its design is in that repository's
`docs/design.md`. This page covers only what Endeavor adds. Nothing here is
built yet. It depends on the core in [runtime-core.md](runtime-core.md).

_Drafted 2026-09-26; engine design moved to turtleR 2026-09-27_

## Summary

turtleR plays the part Pluto plays for Julia: an R server process that reads
cells, builds the graph, schedules runs, writes the file, manages packages and
serves a UI forked from Pluto's, plus one R worker process per notebook.

Endeavor drives it the way it drives Pluto: a small adapter, `runtime-r/`,
runs in turtleR's server process and answers the core's calls through
turtleR's R API.

## Decisions

- **Order of work:** the Rust core against Pluto first, then R, then marimo.
- **Same tool names and shapes as Pluto and marimo.** R differences go in an
  R skill, not the tool list.
- **turtleR has no Endeavor code.** Anything Endeavor-specific lives in
  `runtime-r/`, the R skill, the page script and the app.
- **Opening follows turtleR's default:** notebooks open without running.
  Pluto notebooks keep Pluto's default (run everything on open).

## What Endeavor adds

- **Adapter, `runtime-r/`.** An R script loaded into turtleR's server
  process. It implements the engine interface in
  [runtime-core.md](runtime-core.md) (open, snapshot, graph, apply, run,
  interrupt, render_png, validate, and the cell-state events) with turtleR's
  R API. It hides nothing from `read_notebook_code` beyond the file's header
  and footer blocks.
- **R installation.** turtleR runs on the R it's started with. Endeavor
  downloads the notebook's recorded R version into its own directory without
  admin rights (rig's user mode, or the same portable builds fetched
  directly, the way it fetches Julia) and starts turtleR with it. On macOS,
  Linux and Windows hosts.
- **Compilers.** When turtleR stops because a package needs building from
  source and the tools are missing, Endeavor shows its instructions, with a
  button that runs `xcode-select --install` on macOS. Installing gfortran,
  Rtools or Linux compilers stays with the user.
- **Folder scans.** `crates/wire/src/notebooks.rs` detects turtleR's header
  (`### A turtleR notebook ###`) and cell markers for folder listings and
  the new-session preview.
- **Page script.** The injected script keeps working as long as turtleR's
  forked UI keeps Pluto's DOM hooks and CSS variable names; if turtleR renames
  them, the per-notebook-type page adapter in [ui-spec.md](ui-spec.md) handles
  the difference.
- **R skill.** Teaches the agent the reactive rules (one definition per
  global, dot-names for private values, global settings in the setup cell,
  `withr::with_*` for scoped settings) and the recommended style. Advice, not
  enforcement: turtleR enforces only what keeps notebooks free of hidden
  state.

## Build order

1. **Core** ([runtime-core.md](runtime-core.md) step 1).
2. **turtleR without UI** (turtleR build steps 1–2).
3. **Adapter** on the core. The agent can build and run R notebooks; the app
   shows cell state from `/events`.
4. **Packages and R installation** (turtleR step 3, plus Endeavor's R
   download).
5. **turtleR's UI** in the notebook pane (turtleR step 4); page script checks.
6. **R skill** and session prompt text.

The spikes listed in turtleR's design run first, in a separate session.
