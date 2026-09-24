# Design Doc: Cursor-Independent Pluto Agent App (GPUI + ACP)

## 1. Summary

A native Rust desktop app (GPUI) providing a Cursor-style agent panel plus a
split-screen live view of a running Pluto.jl notebook, connected to **any**
coding agent via the Agent Client Protocol (ACP) instead of being locked into
Cursor. The app reimplements, in an agent-agnostic form, the workflow the user
already relies on today: **PlutoMCP.jl + styx (Cursor plugin) + Cursor's
built-in "glass" browser tool**. That combination is known-good in practice;
the job is not to redesign the workflow, it's to extract it from Cursor.

Core new capability beyond today's styx setup: a **"glass design mode"** —
a translucent annotation overlay on the live notebook view that lets the user
point at one or more cells, attach a comment, and inject that as precise,
structured context into the agent's next prompt turn.

## 2. Goals / Non-Goals

**Goals**
- Agent-agnostic: works with any agent that speaks ACP (Claude Code, Codex
  CLI, Cursor CLI, etc.), not just Cursor.
- Split-screen: native agent panel + a real, live-rendered Pluto notebook
  side by side in one app.
- Glass design mode: point-and-comment annotation on individual cells or
  Pluto's native multi-cell selections, queued and batch-sent as context.
- Preserve everything that already works: PlutoMCP.jl session semantics,
  styx's safety guards, its audit trail.
- Path to Jupyter support later, without redesigning the core abstractions.

**Non-Goals (for v1)**
- Not rebuilding Pluto's rendering natively in GPUI — the notebook pane is a
  real embedded browser view of the actual Pluto frontend.
- Not supporting remote/multi-user Pluto servers — loopback-only, single user.
- Not designing a universal "skills" system — fold styx's skill content into
  plain ACP prompt context instead.

## 3. Architecture Overview

```
┌───────────────────────────────────────────────────────────────┐
│ GPUI App (Rust, native)                                        │
│ ┌───────────────────┐   ┌─────────────────────────────────┐   │
│ │ Agent Panel        │   │ Notebook Surface                 │   │
│ │ (ACP client)        │   │ (embedded webview: real Pluto    │   │
│ │ - transcript        │   │  frontend, wry-based)             │   │
│ │ - diffs             │   │ - Glass overlay (injected JS/CSS) │   │
│ │ - permission prompts│   │ - cell hover/click/select         │   │
│ │ - tool-call log      │   │ - pending annotation tray         │   │
│ └─────────┬──────────┘   └───────────┬───────────────────────┘   │
└───────────┼──────────────────────────┼───────────────────────┘
            │ ACP (JSON-RPC, stdio)      │ (same process, IPC via wry)
            ▼                            ▼
   ┌─────────────────────┐      ┌────────────────────────┐
   │ Any ACP agent         │      │ Pluto's own HTTP/WS      │
   │ (session/new supplies │◄─MCP─│ server (real, unmodified)│
   │  mcpServers list)      │      └────────────────────────┘
   └─────────┬─────────────┘
             │ MCP tool calls
   ┌─────────┴─────────────┬─────────────────────────┐
   ▼                        ▼                          
┌────────────────┐  ┌──────────────────────────┐
│ pluto MCP server │  │ browser-automation MCP     │
│ (PlutoMCP.jl-     │  │ server (new — replaces      │
│  based, existing) │  │  Cursor's native "glass")   │
└────────────────┘  └──────────────────────────┘
```

Key principle: the GPUI app never mutates the notebook directly. All writes
go through the agent → MCP → Pluto's `SessionActions` API, preserving
Pluto's reactivity guarantees. The GPUI app's own read path (for rendering)
goes through the embedded webview talking directly to Pluto's real
frontend/server — a second, independent connection to the same session.

## 4. Component Breakdown

### 4.1 Notebook Surface (embedded browser)
- Real Pluto frontend rendered via `wry`, embedded as a child view inside a
  GPUI window pane (highest technical risk — see §7).
- Inherits Pluto's per-session `?secret=` token; never expose beyond
  `127.0.0.1`.
- No native reimplementation of cell rendering, plots, LaTeX, etc. — get
  that for free from Pluto's own JS frontend.

### 4.2 Glass Design Mode (annotation overlay)
- Implemented as injected JS/CSS inside the webview via
  `WebViewBuilder::with_initialization_script`, not as a native GPUI overlay
  — avoids cross-engine (native+web) compositing entirely.
- Overlay adds a `backdrop-filter: blur(...)` layer with pointer events,
  hover-highlight on `pluto-cell` elements, and click-to-select.
- **Multi-cell selection is not reimplemented** — Pluto's own frontend state
  already tracks `selected_cells: Array<string>` (cell UUIDs) in its
  `Editor.js`/`Notebook.js` component state. The overlay reads Pluto's
  existing selection (via its `selected` CSS class / state) rather than
  building independent multi-select logic.
- Each `pluto-cell` DOM element has a stable `id` attribute equal to the
  cell's UUID — the same UUID used in the notebook's `.jl` source file and
  by MCP tools. This is the single join key across DOM, MCP, and ACP layers.
- Two annotation types on the same overlay: element-anchored comments (bound
  to one or more cell UUIDs) and freeform strokes/arrows on a canvas layer,
  for pointing at relationships between cells.
- Annotations are **queued by default**, shown in a pending tray (same
  pattern as GitHub PR review comments: leave several, then "Send to
  agent" batches them into one prompt turn). This mirrors nearly every
  agent-harness UI and needs no novel design.

### 4.3 Context Injection Pipeline
1. User clicks/selects cell(s) in the glass overlay → overlay captures cell
   UUID(s) + optional comment text via `window.ipc.postMessage`.
2. GPUI app resolves each UUID through the **pluto MCP server**, which
   returns authoritative source, current output, and dependency-graph
   context for that cell (source of truth is always MCP, never the DOM).
3. GPUI app packages each resolved annotation as an ACP `ContentBlock`
   (`ResourceLink`/`Resource` pointing at a synthetic `pluto://notebook/{id}/
   cell/{uuid}` URI, plus a `Text` block with the user's comment).
4. On "Send," all queued annotations are attached to one `session/prompt`
   call.
- Staleness is a non-issue by construction: the agent only ever reads cell
  content through MCP at resolve/read time, never through the DOM snapshot
  the user clicked. A cheap mitigation (content hash / run-count) exists if
  this ever becomes a real problem in practice, but should not be built
  preemptively — it hasn't come up in current styx usage.
- Visual/screenshot capture (e.g. of a plot) is for the **human's** benefit
  in the overlay UI only; the agent's primary key into a cell is its UUID,
  resolved via MCP.

### 4.4 Concurrency: Who Owns the Browser
- styx's `check-design-mode.py` hook exists because Cursor's built-in
  browser tool and a human glass-mode session can race (agent navigates
  while human is mid-annotation). Its `browser_lock` tool (`{"action":
  "lock", "viewId": ...}`) is the concurrency primitive that prevents this.
- The new browser-automation MCP server (§5) must implement an equivalent
  lock/unlock primitive. The GPUI app acquires the lock while a human has
  design mode open; the agent's browser-automation calls are denied or
  queued while locked.
- Belt-and-suspenders: enforce the lock check *inside* the MCP servers
  themselves (server-side deny), not only via ACP's permission-request flow,
  since different ACP agents may have different default auto-approval
  policies for MCP tool calls.

## 5. Extracting From Cursor: Concrete Gap Analysis

Verified directly against styx's own repo contents and the ACP schema.

| Styx/Cursor piece | Status when leaving Cursor | Action |
|---|---|---|
| `mcp.json` → `pluto` server (PlutoMCP.jl via `pluto-mcp-launcher.sh`) | **Already portable.** It's a standard MCP server with zero Cursor-specific glue. | None — reuse as-is. |
| "Glass" browser tools (`browser_navigate`, `browser_snapshot`, `browser_lock`, `browser_tabs`) | **Not portable.** Confirmed absent from `mcp.json` — these are Cursor's own native built-in browser tool, not a bundled MCP server. | Build a new browser-automation MCP server (Chrome DevTools MCP / Playwright MCP as a base), adding the missing `lock`/`viewId` primitive. |
| `hooks.json` guards (`guard-write.py`, `check-design-mode.py`) | No ACP-wide hook/middleware concept exists. | Reimplement as: (a) server-side checks inside the pluto/browser MCP servers, (b) client-side deny on ACP's tool-call permission-request flow. |
| `hooks.json` loggers (`record-read.py`, `record-glass-view.py`) | ACP's `session/prompt` reports tool calls to the client as standard behavior. | Reimplement client-side: log every `session/update` tool-call notification the GPUI app receives. |
| `warn-pending-run.py` (runs on Cursor's "stop" hook) | No ACP "stop" hook equivalent. | Reimplement client-side: after `PromptResponse` returns, query pluto MCP for pending/running cells and surface a warning in the GPUI UI. |
| `skills/pluto-{semantics,session,workflow}` (`SKILL.md` progressive disclosure) | Not a standardized ACP or cross-agent concept. | Fold content into plain `ContentBlock::Text`/`Resource` sent as session-start context, not as an agent-specific "skill." |
| `rules/` (Cursor auto-attached context) | Cursor-specific. | Same treatment as skills — plain prompt context. |
| `commands/` (Cursor slash commands) | Cursor-specific UX. | Reimplement as GPUI-side UI shortcuts that prepend fixed text to a prompt. |
| `.cursor-plugin/plugin.json` manifest | Cursor-specific packaging. | No equivalent needed — the GPUI app itself is the "plugin host." |

**ACP mechanics that do the heavy lifting:**
- `session/new` takes a client-supplied `mcpServers: McpServer[]` list — the
  GPUI app assembles `[pluto, browser-automation]` and passes it at session
  creation, directly replacing `mcp.json`.
- `session/prompt` content blocks (`Text`, `Resource`, `ResourceLink`) are
  baseline-required types — this is exactly the mechanism for attaching
  resolved cell annotations.
- Tool-call reporting and permission requests are standard parts of a
  prompt turn — usable as the client-side hook point for both logging and
  design-mode locking, with the caveat that MCP tool auto-approval policy is
  agent-specific, hence the server-side backstop above.

## 6. Security Model

- Pluto binds to `127.0.0.1` by default and gates access with a rotating
  `?secret=` URL token specifically to prevent arbitrary remote code
  execution — never change this binding, and never let the app's proxying
  extend beyond loopback.
- The new browser-automation MCP bridge should use a per-boot-regenerated
  token for its own loopback channel (not a static/shared secret) — same
  pattern used by comparable existing tools (e.g. AgentsRoom's browser MCP
  bridge), rather than reusing Pluto's own secret across every layer.
- All notebook mutation must go through Pluto's `SessionActions` API via
  MCP, never a raw file write to the `.jl` source — bypassing this breaks
  Pluto's reactivity/consistency guarantees (this is very likely why
  `guard-write.py` exists in the first place).

## 7. Open Risks (Ranked)

1. **Embedding `wry` as a child view inside a GPUI window.** Platform-
   specific (NSView/HWND) interop between GPUI's own compositor and a
   foreign webview is unproven at this scale. De-risk with a small
   standalone spike before committing to the full architecture. If it's
   too fragile, fallback is a docked, app-positioned separate OS window
   (loses "true embedding" but avoids the interop problem).
2. **New browser-automation MCP server + lock primitive is a real build
   item**, not a port — nothing in styx today ships this outside Cursor.
3. **Agent-specific MCP permission defaults.** Don't assume every ACP agent
   surfaces a permission prompt for MCP tool calls; enforce locks and write
   guards server-side regardless.
4. **Skills/rules portability** depends on how well plain prompt-context
   substitutes for progressive skill disclosure across different agents —
   may need per-agent tuning.
5. **Jupyter extension path**: Jupyter's ZMQ kernel model and `.ipynb` JSON
   format are structurally different from Pluto's reactive DAG and
   plain-`.jl` format — `selected_cells` state reuse and the cell-UUID join
   key both need Jupyter-specific equivalents; design the `notebook-model`
   trait (below) with this divergence in mind from the start, not as an
   afterthought.

## 8. Suggested Component Crates

- `acp-client`: wraps the ACP protocol, session lifecycle, `session/new`
  MCP-server wiring, tool-call/permission-update streaming into GPUI view
  models.
- `notebook-model`: backend-agnostic trait (`CellId`, `Topology`,
  `run_cell`, `get_output`, `bond_values`) — `pluto` impl now, stub
  `jupyter` impl later.
- `glass-overlay`: the injected JS/CSS annotation layer + IPC handling +
  pending-annotation queue.
- `pluto-mcp-bridge`: thin Rust wrapper around the existing PlutoMCP.jl
  server (reuse, don't rewrite).
- `browser-mcp-bridge`: new — browser-automation MCP server with the
  `lock`/`viewId` primitive.

## 9. Rust vs. Electron Decision

**Decision: Rust (GPUI + wry).**

Electron's main advantage — one unified DOM spanning the entire window — only
matters if annotations need to span across the agent panel *and* the
notebook pane in one visual layer. They don't; design mode is scoped to
cells inside the notebook pane only. That removes Electron's strongest
argument for this specific feature set. Given the user's existing Rust/GPUI/
ACP alignment and Electron's materially heavier resource footprint for an
app that runs continuously alongside an already memory-hungry Julia process,
Rust is the better fit — contingent on de-risking the child-webview embedding
question (§7.1) early. If that spike fails, Electron (or a docked separate
window fallback) remains a legitimate pivot.

## 10. Milestone Sketch

1. Spike: embed `wry` as a child view in a minimal GPUI window; validate
   resize/focus/z-order behavior.
2. Stand up `acp-client` against a real ACP agent with `session/new`
   supplying the existing `pluto` MCP server only (no glass yet).
3. Build the browser-automation MCP server + lock primitive; wire into
   `session/new`.
4. Build the glass overlay: hover/click/select on `pluto-cell`, read
   Pluto's own `selected_cells` state, pending-annotation tray.
5. Wire the context-injection pipeline: DOM click → UUID → MCP resolve →
   ACP content block → batched `session/prompt`.
6. Port `guard-write`, `check-design-mode`, `record-read`,
   `record-glass-view`, `warn-pending-run` logic into the new MCP servers /
   GPUI client per the mapping in §5.
7. Fold `skills/` and `rules/` content into session-start prompt context.
8. Jupyter: design `notebook-model`'s trait boundary against Jupyter's ZMQ/
   `.ipynb` model before writing a second backend implementation.

## 11. Runtime Management (App-Managed Julia)

**Decision: the app owns one long-lived Julia process** running Pluto +
PlutoMCP's HTTP MCP bridge, from an app-managed Julia install by default.
Opt-in: user points at their own `julia` binary.

- **Install on first run, not bundled.** Download the official Julia tarball
  to `~/Library/Application Support/<app>/julia-<ver>/`, verified against a
  SHA-256 pinned in the app. Avoids ~500MB `.app` bloat and signing/notarizing
  every Julia dylib inside the bundle.
- **Private, stacked depot.** `JULIA_DEPOT_PATH=<app>/depot:` — writes go to
  the app depot; the trailing empty entry appends the default depots so the
  user's existing `~/.julia` packages/precompile caches are reused read-only.
  A fully isolated depot would recompile Makie & co. on first notebook open.
- **Pinned app environment.** App ships `Project.toml` + `Manifest.toml` with
  Pluto + PlutoMCP.jl (pinned by git URL + rev if unregistered). Notebook
  packages are unaffected: Pluto's per-notebook package manager embeds each
  notebook's env in its `.jl` file.
- **"Use my Julia" ≠ "use my global env".** The opt-in swaps only the binary;
  Pluto/PlutoMCP still come from the app's pinned project.
- **Topology.** PlutoMCP already supports this: the app's Julia runs Pluto +
  the HTTP MCP bridge (`start_pluto_stack!`) and reports
  `{pluto_url(with secret), mcp_url}` to the app on stdout. Agents get the
  bridge either as an HTTP MCP server (if they advertise
  `mcpCapabilities.http`) or via stdio `PlutoMCP.connect(mcp_port=…)`, which
  proxies to it. The app is also an MCP client of the same bridge — this
  closes the gap in §4.3/§4.4 where the app had no channel to an
  agent-spawned MCP server.
- **Lifetime.** Julia exits when its stdin hits EOF, so it dies with the app
  without PID bookkeeping.

Deferred: juliaup integration, PackageCompiler sysimage (TTFX paid once per
app launch, not per chat), automatic Julia upgrades (bump the pin per
release).
