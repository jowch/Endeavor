# Endeavor roadmap

Living plan; update it as items land. Design rationale lives in
[pluto-agent-design-doc.md](pluto-agent-design-doc.md) and
[design-notes.md](design-notes.md). Open design gaps are in
[design-gaps.md](design-gaps.md).

_Last updated: 2026-10-03_

## Where things stand

Endeavor is a native (GPUI) app with the live Pluto frontend in one pane and a
Claude Code agent panel (over ACP) in the other. Notebooks run in a runtime
per host: a Rust core (`endeavor-remote core`) that serves the agent's tools,
with Julia running Pluto behind it as an adapter
([runtime-core.md](runtime-core.md)). The runtime runs on this computer, on a
server, or in a Slurm job on a cluster ([remote-sessions.md](remote-sessions.md)).

## Next

- **One port per runtime.** Pluto's page, MCP and the app's calls on one
  port answered by the core, with one token, instead of two ports. Built and
  checked live on This Mac, a server and Slurm. Plan in [one-port.md](one-port.md).
- **The notebook tools as a standalone product.** After one port: an
  `endeavor serve` command a user runs on a workstation or inside their own
  cluster job, which prints one link and one `ssh -L` line, and a stdio form
  installed as a Claude Code, Codex or Gemini plugin. Login, ssh and the
  tunnel stay with the user or their agent; approvals are the agent's own
  prompts. Outline in [one-port.md](one-port.md), "The standalone command".

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
- **Jupyter.** Design the notebook boundary against Jupyter's kernel and
  `.ipynb` model before writing a second backend.
- **Freeform annotation strokes** (arrows between cells), once there's a way
  for the agent to make sense of them (e.g. a screenshot alongside).
- **Windows.** It builds and its tests pass in CI. Running a local notebook
  is written but untried on a real machine; ssh, the notebook view and
  packaging are left. See [windows.md](windows.md), and [linux.md](linux.md)
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
