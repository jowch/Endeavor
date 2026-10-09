# Endeavor roadmap

Living plan; update it as items land. Design rationale lives in
[pluto-agent-design-doc.md](pluto-agent-design-doc.md) and
[design-notes.md](design-notes.md). Open design gaps are in
[design-gaps.md](design-gaps.md).

_Last updated: 2026-10-09_

## Where things stand

Endeavor is a native (GPUI) app with the live Pluto frontend in one pane and a
Claude Code agent panel (over ACP) in the other. Notebooks run in a runtime
per host: a Rust core (`endeavor core`) that serves the agent's tools,
with Julia running Pluto behind it as an adapter
([runtime-core.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/runtime-core.md)). The runtime runs on this computer, on a
server, or in a Slurm job on a cluster ([remote-sessions.md](remote-sessions.md)).

## Next

- **A repeatable eval of the agent loop, with reproducibility checks on the
  analyses** ([EndeavorMCP #26](https://github.com/jowch/EndeavorMCP/issues/26)).
  Next after the app adopts EndeavorMCP; built to cover more than one engine.
- **Ember (R notebooks).** In development in its own repository. It joins
  EndeavorMCP as another adapter behind the core; on this side it needs the
  pane, header and logo for an R notebook and the notebook chip listing R
  files. Starts after the first version of the eval above; Ember's own
  groundwork (a version to pin, binaries) goes on in its repository meanwhile.
- **README, landing page and documentation website.**

The notebook tools (runtime, MCP server, `endeavor serve`) live in
[EndeavorMCP](https://github.com/jowch/EndeavorMCP); their open work is in its
[docs/status.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/status.md).

## Before sharing the app

- **Signing and notarization.** `scripts/bundle.sh` builds an ad-hoc signed
  `Endeavor.app` (resources in `Contents/Resources`); sharing it needs a
  Developer ID signature, hardened runtime, and notarization.

## Later

- **Other ACP agents** (Codex, Gemini). The work that makes Endeavor
  agent-neutral is partly done; what is left, and what to find out about each
  agent, is in [other-agents.md](other-agents.md). Steering is only
  available where the agent advertises it. Cursor was tried and parked: see
  [cursor-agent.md](cursor-agent.md).
- **Freeform annotation strokes** (arrows between cells), once there's a way
  for the agent to make sense of them (e.g. a screenshot alongside).
- **Windows.** It builds and its tests pass in CI, which also keeps a
  build to try and a per-user installer ([windows.md](windows.md#install)).
  Running a local notebook is written but untried on a real machine; ssh,
  the notebook view and signing are left. See [windows.md](windows.md), and [linux.md](linux.md)
  for the Linux port's remaining work.
- **Upstream to mthelm85/PlutoMCP.jl.** The runtime started from a fork of
  PlutoMCP that carried several general improvements (`new_notebook`, run
  state, `view_cell_output`, earlier fixes); offer the ones that aren't
  Endeavor-specific.

## Known shortcuts

Deliberate simplifications with their upgrade path (search the code for
`ponytail:`).

| Where | Shortcut | Upgrade when |
|---|---|---|
| `transcript.rs` | long diffs are cut, not scrollable | real notebooks hit it |
| runtime `view_cell_output` | no timeout when the worker is busy | a long run blocks it in practice |
| `install.rs` first-run installs | Julia 1.12.6 and Node 24.21.0 pinned in code (bump URL, SHA-256, size per release); curl outlives a quit mid-download | an upgrade, or overlapping launches bite |

Also noted: SIGTERM can leave Julia hung mid-exit. The helper stops a runtime
with the bridge's `endeavor/shutdown` first and only falls back to SIGTERM,
then SIGKILL after a grace period.
