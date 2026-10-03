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
  so no system Node or `npx`. Stacked private depot; the
  `endeavor-remote` helper starts `boot.jl` detached and relays its ports to
  the app; Julia stops when the app quits unless the user keeps it running. Plain-language startup errors, crash detection, and **Restart Julia**
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
  (recorded in Application Support's `sessions.json` and shown at launch;
  other Claude Code sessions in the folder are not listed) reopen with their
  transcript. The
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
- **Claude sign-in** (`signin.rs`). First launch asks under the splash: the
  assistant (Claude; Cursor, Codex and Gemini listed as not available yet),
  then a Claude plan or an Anthropic Console account, each with its own Sign
  in, which runs the bundled CLI's `auth login` in the browser. Waiting shows
  the choice with "Open it again" (the page address, kept by acting as the
  CLI's `$BROWSER`) and Cancel (stops the CLI's process group); a failure
  names the reason it can (browser closed, free Claude account, Console
  account without access) with the CLI's last line under Details. Setup
  finishes once signed in. **Sign-in expiring mid-use**: a turn the adapter
  fails with `authRequired`, or the `auth status` check when the window comes
  to the front, signs the app out: the unanswered message stays, marked "Not
  answered yet", every session's messages wait, a card above the composer
  offers Sign in the way it was done last (`sign_in_method` in
  settings.json) or another account, and the sidebar says "Signed out of
  Claude."; after signing in the kept message sends by itself.
- **Offline** (`network.rs`, `offline.rs`). The system's network status
  (Network framework's path monitor on macOS; netlink route changes and
  `/proc/net/*route` on Linux) holds Claude's messages while offline and
  sends them in order once back; first-launch setup pauses and carries on by
  itself; a server's notebook stays up read-only while its connection is out
  of reach, retried every 15 seconds, and reloads once it's back. Try now
  checks right away (a TCP connect to Claude's API). Debug builds take
  `ENDEAVOR_FORCE_OFFLINE` (a file: offline while it exists) and
  `ENDEAVOR_TEST_UNREACHABLE` (the `local-test` server can't be reached).
- **First-launch setup screen** (`splash.rs`, `turtle.rs`): the turtle walks
  in and looks around; one line and a thin bar report the setup steps (Julia,
  Pluto packages, Claude agent, connecting); on failure the step list, Retry
  and Show logs. Shown until setup finishes once
  (`setup-complete` marker); later installs report in the status line.
- **Settings** (session bar): use my Claude Code setup, run notebook code
  without asking in new sessions, and Endeavor's Julia vs. a chosen julia
  (applies when Julia next starts). Kept in `settings.json`.
- **Packaging.** `scripts/bundle.sh` builds `Endeavor.app` with its resources
  in `Contents/Resources`.
- **Annotation mode (§4.2).** ⌘⇧K; click cells, comment, send as
  `pluto://notebook/{id}/cell/{uuid}` links through the same queue.
- **Agent environment.** Endeavor's own Claude Code plugin (`plugin/`: the
  styx Pluto skills, ported) plus project settings; the user's personal Claude
  Code setup is opt-in (Settings).
- **Execution gate.** The agent asks before running notebook code
  (`execute_cell`, `submit_changes`, `run_all_cells`, `allow_execution`,
  `delete_cell`, `run_after=true`): the runtime holds the call until the
  user answers the run card (Allow / Allow & stop asking (this session) /
  Deny), so it works for any ACP agent.
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

In priority order.

1. **Own runtime package** replacing the PlutoMCP fork: `runtime/EndeavorRuntime`,
   staged plan in [design-notes.md](design-notes.md#runtime-our-own-replacement-for-plutomcp).
   Steps 1–4 done (parity, token auth on the tool channel, pushed notebook
   events, staleness derived from Pluto's run times); next: policy in the
   server (Plan / Ask to run / Auto), versions and attribution.

## Before sharing the app

Everything here is a known `ponytail:` shortcut that's fine for one developer.

- **Logo and style system.** The app's inline colors are placeholders for the designed logo and UI style
  system (in progress separately).
- **Signing and notarization.** `scripts/bundle.sh` builds an ad-hoc signed
  `Endeavor.app` (resources in `Contents/Resources`); sharing it needs a
  Developer ID signature, hardened runtime, and notarization.

## Later

- **Other ACP agents** (Codex, Gemini). The plugin is Claude Code-specific;
  other agents need the skills as plain prompt context (§5), and steering is
  only available where the agent advertises it.
  Cursor was tried and parked: see [cursor-agent.md](cursor-agent.md).
- **Jupyter (§7.5, milestone 8).** Design the notebook-model boundary against
  Jupyter's kernel/`.ipynb` model before writing a second backend.
- **Plugin slash commands** in the panel (the plugin can carry them; the
  panel doesn't list `availableCommands` yet).
- **Multiple sessions / tabs**, and showing modes and usage.
- **Freeform annotation strokes** (arrows between cells), once there's a way
  for the agent to make sense of them (e.g. a screenshot alongside).
- **Windows.** About 6–8 weeks for one person; most of it is replacing Unix
  process control and the ssh code. See [windows.md](windows.md), and
  [linux.md](linux.md) for the Linux port's remaining work.
- **Upstream to mthelm85/PlutoMCP.jl.** The fork carries several general
  improvements (`new_notebook`, run state, `view_cell_output`, earlier fixes);
  offer the ones that aren't Endeavor-specific.

## Known shortcuts

Deliberate simplifications with their upgrade path (search the code for
`ponytail:`).

| Where | Shortcut | Upgrade when |
|---|---|---|
| `main.rs` transcript | long diffs are cut at 60 lines, not scrollable | real notebooks hit it |
| `main.rs` events | modes, usage, available commands ignored | the panel grows those features |
| PlutoMCP `view_cell_output` | no timeout when the worker is busy | a long run blocks it in practice |
| `install.rs` first-run installs | Julia 1.12.6 and Node 24.21.0 pinned in code (bump URL, SHA-256, size per release); curl outlives a quit mid-download | an upgrade, or overlapping launches bite |

Also noted: SIGTERM can leave Julia hung mid-exit. The helper stops a runtime
with the bridge's `endeavor/shutdown` first and only falls back to SIGTERM,
then SIGKILL after a grace period.
