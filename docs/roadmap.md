# Endeavor roadmap

Living plan; update it as items land. Design rationale lives in
[pluto-agent-design-doc.md](pluto-agent-design-doc.md) (section numbers below refer to it).
Ideas still being explored (review in place, proposals, modes) live in
[design-notes.md](design-notes.md).

_Last updated: 2026-09-24_

## Where things stand

Endeavor is a native (GPUI) app with the live Pluto frontend in one pane and a
Claude Code agent panel (over ACP) in the other, both on one app-owned Julia
process running Pluto + PlutoMCP.

**Done**

- **Runtime (§11).** App-owned Julia: Julia 1.12.6 is downloaded on first
  launch (progress in the status line, resumable) into Application Support,
  checked against a pinned SHA-256; Settings can switch to your own julia
  instead. The ACP adapter is installed the same way: pinned Node.js 24.21.0
  (SHA-256) plus `npm ci` of `adapter/package-lock.json` (integrity-checked),
  so no system Node or `npx`. Stacked private depot; `boot.jl`
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
- **Sessions.** Session bar | chat | notebook. Launch shows a new-session
  screen (working folder via picker or recent folders, optional first
  message). Sessions run in parallel on one ACP connection, grouped by folder
  with busy / needs-approval marks; each folder's past Endeavor sessions
  (tracked in Application Support's `sessions.json`; other Claude Code
  sessions in the folder are not listed) reopen with their transcript. The
  folder sets the agent's working directory and project settings, and new
  notebooks are created there. The notebook pane follows the active session.
  Pluto's own "new notebook" starts unsaved in Pluto's scratch folder (its
  usual behavior); its "Save notebook" box suggests the active session's folder.
  Sessions take the agent's generated titles; double-click to rename (names
  kept in `titles.json`); × closes an open session (it stays in history) or,
  with a confirm, deletes a past one (`session/delete`); "Show more" past 8.
- **Logs.** Launched from Finder, output goes to
  `~/Library/Logs/Endeavor/endeavor.log` (previous run: `endeavor.old.log`),
  with Pluto's secret redacted; "Show logs" in Settings and on setup errors.
- **Claude sign-in.** Signed out, the setup screen (or, later, the session
  bar) offers Claude subscription / Anthropic Console sign-in via the bundled
  CLI's `auth login` in the browser; setup finishes once signed in.
  Verified end to end with a throwaway config folder.
- **First-launch setup screen** (`splash.rs`): placeholder logo, the setup
  steps (Julia, Pluto packages, Claude agent, connecting) with a progress bar
  and live detail, and Retry on failure. Shown until setup finishes once
  (`setup-complete` marker); later installs report in the status line.
- **Settings** (session bar): use my Claude Code setup, run notebook code
  without asking in new sessions, and Endeavor's Julia vs. a chosen julia
  (applies when Julia next starts). Kept in `settings.json`.
- **Packaging.** `scripts/bundle.sh` builds `Endeavor.app` with its resources
  in `Contents/Resources`.
- **Annotation mode (§4.2).** ⌘⇧E; click cells, comment, send as
  `pluto://notebook/{id}/cell/{uuid}` links through the same queue.
- **Agent environment.** Endeavor's own Claude Code plugin (`plugin/`: the
  styx Pluto skills, ported) plus project settings; the user's personal Claude
  Code setup is opt-in (Settings).
- **Execution gate.** The agent asks before running notebook code
  (`execute_cell`, `submit_changes`, `run_all_cells`, `allow_execution`,
  `delete_cell`, `run_after=true`): a plugin PreToolUse hook (`endeavor
  hook-pretool`) answers "ask", and the panel offers Allow / Allow & stop
  asking (this session) / Deny. Other ACP agents would need their own gate.
- **Verified in the app (2026-09-25 click-through,** driven with synthetic
  input): folder picker, new/second/reopened sessions, run approvals and
  "stop asking", cell diffs, queue, ⌘↩ steering, Esc, annotation mode, Julia
  crash + restart, and window resize keeping the webview in its pane (§7.1).
- **Context the agent gets for free.** Which notebook is on screen; an
  end-of-turn warning when edited cells were left unrun or still running.

**PlutoMCP (fork, `jowch/PlutoMCP.jl`)** carries the tools Endeavor relies on,
merged to `main` after paired `review-pr` / author-agent review:
[#10](https://github.com/jowch/PlutoMCP.jl/pull/10) `new_notebook`,
[#11](https://github.com/jowch/PlutoMCP.jl/pull/11) run state in `list_notebooks`,
[#12](https://github.com/jowch/PlutoMCP.jl/pull/12) `view_cell_output` (the agent sees plots as PNG).
Endeavor pins `main` (`runtime/Project.toml`).

## Next

In priority order. Nothing queued: the next work is the packaging list below.

## Before sharing the app

Everything here is a known `ponytail:` shortcut that's fine for one developer.

- **Logo and style system.** The setup screen's rocket + wordmark and the
  app's inline colors are placeholders for the designed logo and UI style
  system (in progress separately).
- **Sign-in expiring mid-use.** Sign-in is checked at startup only; a login
  that expires while the app runs shows up as failed turns, not the sign-in
  panel.

- **Signing and notarization.** `scripts/bundle.sh` builds an ad-hoc signed
  `Endeavor.app` (resources in `Contents/Resources`); sharing it needs a
  Developer ID signature, hardened runtime, and notarization.

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
| `install.rs` first-run installs | Julia 1.12.6 and Node 24.21.0 pinned in code (bump URL, SHA-256, size per release); curl outlives a quit mid-download | an upgrade, or overlapping launches bite |

Also noted: SIGTERM can leave Julia hung mid-exit. The app never sends it
(quitting closes stdin and `boot.jl` exits itself), but anything that manages
the process externally should use stdin EOF or SIGKILL.
