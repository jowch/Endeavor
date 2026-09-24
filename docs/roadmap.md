# Endeavor roadmap

Living plan; update it as items land. Design rationale lives in
[pluto-agent-design-doc.md](pluto-agent-design-doc.md) (section numbers below refer to it).

_Last updated: 2026-09-24_

## Where things stand

Endeavor is a native (GPUI) app with the live Pluto frontend in one pane and a
Claude Code agent panel (over ACP) in the other, both on one app-owned Julia
process running Pluto + PlutoMCP.

**Done**

- **Runtime (§11).** App-owned Julia with a stacked private depot; `boot.jl`
  reports the Pluto URL and MCP bridge; Julia exits when the app closes its
  stdin. Plain-language startup errors, crash detection, and **Restart Julia**
  on the same ports (the agent's MCP connection reconnects; open notebooks are
  reopened).
- **Notebook pane (§4.1).** Real Pluto frontend in a `gpui-wry` child webview;
  follows notebooks the agent opens or creates.
- **Agent panel (§4.3, §5).** ACP session with Claude Code on the app's PlutoMCP
  bridge; streaming transcript with Markdown, permission prompts, a visible
  editable message queue (Enter queues while busy), ⌘↩ send-now via
  `_session/steering`, Esc/Stop to interrupt, cell edits shown as diffs,
  expandable tool calls, thinking, and the agent's plan.
- **Annotation mode (§4.2).** ⌘⇧E; click cells, comment, send as
  `pluto://notebook/{id}/cell/{uuid}` links through the same queue.
- **Agent environment.** Endeavor's own Claude Code plugin (`plugin/`: the
  styx Pluto skills, ported) plus project settings; the user's personal Claude
  Code setup is opt-in (`ENDEAVOR_PERSONAL_CLAUDE=1`).
- **Execution gate.** The agent asks before running notebook code
  (`execute_cell`, `submit_changes`, `run_all_cells`, `allow_execution`,
  `delete_cell`, `run_after=true`): a plugin PreToolUse hook (`endeavor
  hook-pretool`) answers "ask", and the panel offers Allow / Allow & stop
  asking (this session) / Deny. Other ACP agents would need their own gate.
- **Context the agent gets for free.** Which notebook is on screen; an
  end-of-turn warning when edited cells were left unrun or still running.

**PlutoMCP (fork, `jowch/PlutoMCP.jl`)** carries the tools Endeavor relies on,
merged to `main` after paired `review-pr` / author-agent review:
[#10](https://github.com/jowch/PlutoMCP.jl/pull/10) `new_notebook`,
[#11](https://github.com/jowch/PlutoMCP.jl/pull/11) run state in `list_notebooks`,
[#12](https://github.com/jowch/PlutoMCP.jl/pull/12) `view_cell_output` (the agent sees plots as PNG).
Endeavor pins `main` (`runtime/Project.toml`).

## Next

In priority order.

1. **End-to-end UI check.** Several recent features are unit- and live-tested
   below the UI but not yet seen in the running app: the Restart Julia button
   and error notes, cell diffs, thinking/plan entries, ⌘↩ steering, Stop.
   Also still unconfirmed from the first spike: **window resize** keeping the
   webview inside its pane (§7.1). Window screenshots currently fail (macOS
   Screen Recording permission); restore that or check by hand.
2. **Where notebooks live.** `new_notebook` without a path lands in the app's
   depot (`~/Library/Application Support/endeavor/depot/pluto_notebooks`),
   which users won't find. Default to the project folder instead.
3. **Project folder.** The agent's working directory (and so which project
   CLAUDE.md applies) is wherever the app was launched. Add an explicit
   "open folder", like an editor, and use it for new notebooks too.
4. **Session persistence.** Keep the transcript across launches and resume
   the agent session (the adapter supports `loadSession`/resume).

## Before sharing the app

Everything here is a known `ponytail:` shortcut that's fine for one developer.

- **Managed Julia (§11):** first-run download + SHA-256 check into Application
  Support; `ENDEAVOR_JULIA` stays as the "use my Julia" override.
- **Bundle the ACP adapter.** It's fetched with `npx` on first launch (~85 s
  over VPN) and needs Node installed; ship it (and a runtime) with the app.
- **`.app` bundle:** move dev-tree paths (`runtime/`, `plugin/`) to bundle
  resources; code signing and notarization.
- **Settings UI:** personal Claude Code setup opt-in, Julia path, and the
  execution gate's "stop asking" (per session today), replacing environment
  variables.

## Later

- **Other ACP agents** (Codex, Gemini). The plugin is Claude Code-specific;
  other agents need the skills as plain prompt context (§5), and steering is
  only available where the agent advertises it.
- **Jupyter (§7.5, milestone 8).** Design the notebook-model boundary against
  Jupyter's kernel/`.ipynb` model before writing a second backend.
- **Plugin slash commands** in the panel (the plugin can carry them; the
  panel doesn't list `availableCommands` yet).
- **Multiple sessions / tabs**, and showing modes and usage.
- **Freeform annotation strokes** (arrows between cells), once there's a way
  for the agent to make sense of them (e.g. a screenshot alongside).
- **Upstream to mthelm85/PlutoMCP.jl.** The fork carries several general
  improvements (`new_notebook`, run state, `view_cell_output`, earlier fixes);
  offer the ones that aren't Endeavor-specific.

## Known shortcuts

Deliberate simplifications with their upgrade path (search the code for
`ponytail:`).

| Where | Shortcut | Upgrade when |
|---|---|---|
| `main.rs` notebook list | 10 s poll of `list_notebooks` to know what to reopen after a crash | a Pluto open/close event is exposed |
| `main.rs` run-state check | failures are silent | it matters that the warning was skipped |
| `main.rs` transcript | long diffs are cut at 60 lines, not scrollable | real notebooks hit it |
| `main.rs` events | modes, usage, available commands ignored | the panel grows those features |
| PlutoMCP `view_cell_output` | no timeout when the worker is busy | a long run blocks it in practice |
| `agent.rs`, `runtime.rs` | dev-tree paths, `npx` adapter, env-var opt-ins | packaging (above) |

Also noted: SIGTERM can leave Julia hung mid-exit. The app never sends it
(quitting closes stdin and `boot.jl` exits itself), but anything that manages
the process externally should use stdin EOF or SIGKILL.
