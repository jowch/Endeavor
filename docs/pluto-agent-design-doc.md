# Design Doc: Pluto Agent App (GPUI + ACP)

The founding decisions behind Endeavor and their reasons. How the UI looks
and behaves is in [ui-spec.md](ui-spec.md); the runtime is in
[runtime-core.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/runtime-core.md). Section numbers are kept from the first
draft, since code comments cite them.

## 1. Summary

A native Rust desktop app (GPUI) with an agent panel beside a live view of a
running Pluto.jl notebook, connected to coding agents through the Agent
Client Protocol (ACP) instead of being locked into one editor. It started as
an agent-neutral version of a workflow that worked in Cursor: PlutoMCP.jl, the
styx plugin's skills, and Cursor's built-in browser. The job was to take that
workflow out of Cursor, not to redesign it.

The capability it adds beyond that setup: pointing at cells (and parts of
them) in the live notebook and sending them, with a comment, as precise
context in the agent's next turn.

## 2. Goals / Non-Goals

**Goals**
- Agent-agnostic: works with any agent that speaks ACP (Claude Code, Codex
  CLI, Cursor CLI, etc.), not one editor's agent.
- Split-screen: native agent panel + a real, live-rendered Pluto notebook
  side by side in one app.
- Point-and-comment on cells, queued and sent as context.
- Keep what already worked: PlutoMCP.jl's session semantics, styx's safety
  guards and its skills.
- A path to other notebook kinds later without redesigning the core
  abstractions.

**Non-Goals**
- Not rebuilding Pluto's rendering natively in GPUI. The notebook pane is a
  real embedded web view of the actual Pluto frontend.
- Not multi-user Pluto servers. Remote notebooks over SSH keep loopback on
  both ends; see [remote-sessions.md](remote-sessions.md).
- Not a universal "skills" system. Claude Code gets the skills as a plugin;
  other agents get them from the notebook MCP server
  ([other-agents.md](other-agents.md), work item 4).

Key principle: the app never mutates the notebook directly. All writes go
through the agent's MCP tools and Pluto's own session API, which keeps Pluto's
reactivity guarantees. The notebook pane reads Pluto's own frontend, a
second, independent connection to the same session.

## 6. Security Model

- Pluto binds to `127.0.0.1` by default and gates access with a `?secret=`
  URL token specifically to prevent arbitrary remote code execution. Never
  change this binding, and never let the app's proxying extend beyond
  loopback. Remote sessions keep both ends on loopback and carry traffic over
  SSH; see [remote-sessions.md](remote-sessions.md).
- The runtime's bridge (the agent's MCP tools and the app's calls) uses its
  own random per-launch bearer token, not Pluto's secret.
- All notebook mutation goes through Pluto's session API via MCP, never a
  raw write to the `.jl` file. A raw write bypasses Pluto's reactivity and
  consistency guarantees and is overwritten by Pluto's next save.

## 7. Open Risks

- **Other notebook kinds.** Jupyter's ZMQ kernel model and `.ipynb` JSON
  format differ structurally from Pluto's reactive graph and plain-`.jl`
  format. Cell identity and the dependency tools need Jupyter-specific
  equivalents; design the notebook boundary with this in mind
  ([runtime-core.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/runtime-core.md) has the engine interface; marimo, the
  closer match, is in [marimo.md](marimo.md)).
- **Agent-specific MCP permission defaults.** Don't assume every ACP agent
  shows a permission prompt for MCP tool calls; the runtime enforces its
  rules (read-before-edit, run approval, plan mode) itself.

## 9. Rust vs. Electron Decision

**Decision: Rust (GPUI + wry).**

Electron's main advantage, one DOM spanning the whole window, only matters if
annotations need to span the agent panel *and* the notebook pane in one
visual layer. They don't: pointing is scoped to the notebook pane. That
removes Electron's strongest argument for this feature set. Given the
existing Rust/GPUI/ACP alignment and Electron's heavier resource footprint
for an app that runs continuously beside an already memory-hungry Julia
process, Rust is the better fit. The risk was embedding wry as a child view
in a GPUI window; that works (the web view follows its pane through resizes
and sidebar changes).

## 11. Runtime Management (App-Managed Julia)

**Decision: the app manages Julia** for the runtime, from an app-managed
Julia install by default. Opt-in: the user points at their own `julia`
binary.

- **Install on first run, not bundled.** Download the official Julia tarball
  to `~/Library/Application Support/endeavor/julia-<ver>/`, verified against a
  SHA-256 pinned in the app. Avoids ~500MB `.app` bloat and signing/notarizing
  every Julia dylib inside the bundle.
- **Private, stacked depot.** `JULIA_DEPOT_PATH=<app>/depot:` — writes go to
  the app depot; the trailing empty entry appends the default depots so the
  user's existing `~/.julia` packages/precompile caches are reused read-only.
  A fully isolated depot would recompile Makie & co. on first notebook open.
- **Pinned app environment.** The app ships `runtime/Project.toml` +
  `Manifest.toml` with Pluto and EndeavorRuntime (the Pluto adapter, a path
  dependency). Notebook packages are unaffected: Pluto's per-notebook package
  manager embeds each notebook's env in its `.jl` file.
- **"Use my Julia" ≠ "use my global env".** The opt-in swaps only the binary;
  Pluto and the adapter still come from the app's pinned project.
- **Topology.** The `endeavor` helper starts the runtime's core, which
  serves the bridge (the agent's MCP over Streamable HTTP, and the app's
  `/call` and `/events`) and starts Julia running Pluto as its child. The
  core writes the ports, token and Pluto's secret to `runtime.json`, which
  the helper reads ([remote-sessions.md](remote-sessions.md),
  [runtime-core.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/runtime-core.md)). The app is a client of the same
  bridge, so it sees what the agent's tools do.
- **Lifetime.** The runtime runs detached, started by the helper, which the
  app reaches over its stdin/stdout. It stops when the app quits, unless the
  "Keep notebooks running after Endeavor quits" setting is on; then the next
  launch reattaches.

Deferred: juliaup integration, PackageCompiler sysimage (TTFX paid once per
app launch, not per chat), automatic Julia upgrades (bump the pin per
release).
