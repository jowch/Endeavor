# Endeavor roadmap

Living plan; update it as items land. Design rationale lives in
[pluto-agent-design-doc.md](pluto-agent-design-doc.md) and
[design-notes.md](design-notes.md). Open gaps are
[GitHub issues](https://github.com/jowch/Endeavor/issues).

_Last updated: 2026-10-22_

## Where things stand

Endeavor is a native (GPUI) app for analyzing data with an agent,
reproducibly: the live notebook in one pane and an ACP agent panel in the
other, with Claude Code or Codex (Antigravity tested less). Notebooks run in a
runtime per host: a Rust core (`endeavor core`) that serves the agent's tools,
with Julia running Pluto, or R running Ember for R notebooks, behind it as an adapter
([runtime-core.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/runtime-core.md)). The runtime runs on this computer, on a
server, or in a Slurm job on a cluster ([remote-sessions.md](remote-sessions.md)).

A "nightly" prerelease (`nightly.yml`) holds a Mac build, signed with a
Developer ID and notarized, and an unsigned Windows installer. Both update
themselves once a day (#80; the check that a real nightly updates itself on
each is still to pass). Linux builds from source. The changed-cells card can
put a turn's cells back as they were (Restore, #124). What has to hold before
it is announced is tracked in [#156](https://github.com/jowch/Endeavor/issues/156).

## Next

- **The release bar** ([#156](https://github.com/jowch/Endeavor/issues/156)):
  an agent installs the plugin from one link to the site (#154); an old helper
  or runtime never silently hides features (EndeavorMCP #226, #97); a nightly
  updates itself on Mac and Windows (#80); commands reachable on every
  platform (#148 on Linux, #10 on Windows); a newcomer reaches a first finished
  analysis with no help.
- **Python notebooks through marimo** ([#1](https://github.com/jowch/Endeavor/issues/1),
  [marimo.md](marimo.md)). Python notebooks open, are made and run in the app on
  this computer and on servers and clusters, with Point. Install uv is offered
  where there's no uv on a Mac or Linux computer or a server; on Windows and
  Slurm clusters the user installs uv. Share and export works on
  marimo's page, Live docs opens marimo's documentation panel in the
  bottom drawer, and Packages shows what the notebook runs on.
- **R notebooks on Windows.** R notebooks (Ember) run in the app on Macs and
  Linux, and on servers and clusters. On Windows the chip offers Julia and
  Python, not R, and lists no `.R` files, until Ember runs there (Ember #48, #49).
- **Checking the agent loop.** The smoke suite in EndeavorMCP
  ([smoke.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/smoke.md))
  runs fixed notebook tasks through Claude Code, and runs before a skill or
  runtime change lands. A check that an analysis reruns the same is not
  designed yet.

The notebook tools (runtime, MCP server, `endeavor serve`) live in
[EndeavorMCP](https://github.com/jowch/EndeavorMCP); their open work is in its
[docs/status.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/status.md).

## Before sharing the app widely

- **Windows signing** ([#12](https://github.com/jowch/Endeavor/issues/12)).
  The Mac nightly is signed and notarized. The Windows installer isn't
  signed, so SmartScreen warns, and Windows 11's Smart App Control is
  unchecked.
- **Versioned releases** ([#13](https://github.com/jowch/Endeavor/issues/13)).
  There is only the nightly prerelease so far.
- **Updates** ([#80](https://github.com/jowch/Endeavor/issues/80)). The
  Mac nightly checks `nightly.json` once a day, fetches a newer build,
  checks its Developer ID team and notarization and that its signed
  Info.plist names that build, and installs it on Restart or quit
  (`src/updater.rs`, [updates.md](updates.md)). The Windows nightly fetches the installer, checks its
  SHA-256 (it isn't signed, #12, so that's the only check), and runs it
  silently on Restart or quit. A download the app makes carries no Mark of
  the Web, so SmartScreen's file check doesn't stop it (checked on Windows
  10). Builds from source never check.

## Later

- **More ACP agents** (Gemini, and finishing Antigravity). Antigravity is
  pinned for Windows, Mac and Linux, and has run on Windows only so far
  ([antigravity-agent.md](antigravity-agent.md)).
  The work that makes Endeavor
  agent-neutral is partly done; what is left, and what to find out about each
  agent, is in [other-agents.md](other-agents.md). Steering is only
  available where the agent advertises it. Cursor was tried and parked: see
  [cursor-agent.md](cursor-agent.md).
- **Freeform annotation strokes** (arrows between cells), once there's a way
  for the agent to make sense of them (e.g. a screenshot alongside).
- **Windows.** It builds and its tests pass in CI, which also keeps a
  build to try and a per-user installer ([windows.md](windows.md#install)).
  A local notebook with a real Claude session has run from the installer on
  Windows 10, and so has a notebook on a Linux server over an ssh key (the
  build downloads a server's helper when it first connects). Signing is left,
  and so is a server that asks for a password (#204). See [windows.md](windows.md), and [linux.md](linux.md)
  for the Linux port's remaining work.
- **Upstream to mthelm85/PlutoMCP.jl.** The runtime started from a fork of
  PlutoMCP that carried several general improvements (`new_notebook`, run
  state, `view_cell_output`, earlier fixes); offer the ones that aren't
  Endeavor-specific.

## Known shortcuts

Deliberate simplifications with their upgrade path, named by file and
function. This is all of them; add a row with each new one. The runtime's own
shortcuts are EndeavorMCP's, tracked in its
[issues](https://github.com/jowch/EndeavorMCP/issues); the
two this list used to carry (`view_cell_output` has no timeout while the
worker is busy, and SIGTERM can leave Julia hung mid-exit) are
[EndeavorMCP #62](https://github.com/jowch/EndeavorMCP/issues/62).

| Where | Shortcut | Upgrade when |
|---|---|---|
| `diagram.rs` `cached` | GPUI keeps each picture it has drawn for the window's life, so each width a diagram is laid out at keeps its pixels (about 1 to 3 MB) | a session with many diagrams, resized often, grows noticeably |
| `notify.rs` `flash` | on Windows, "Claude is waiting for you" flashes the taskbar button instead of showing a notification, and coming back doesn't open the session that asked | a toast, with the app's AppUserModelID on the installer's Start menu shortcut |
| `connection.rs` `waits` | Python notebooks share R's waits and failure slot on a host | the rest of the app slice ([#1](https://github.com/jowch/Endeavor/issues/1)) |
| `cell_shot.rs` `cell_shot_job` | the agent's picture of an HTML output is taken only while the notebook pane is on screen | a picture asked for behind Settings |
| `frontend/src/diff.ts` | the page's diff is an O(n·m) table; when the two versions' line counts multiply past 250,000, no diff is shown | big cells need a diff |
| `server_dialog.rs` `drop_asks` | when a sign-in ends while another server's question waits behind it, that question comes to the front without the focus | two servers asking at once happens in practice |
| `connection.rs` `send_idle_limit` | a failed send leaves the runtime on its default idle stop (48 hours) until the next start | it fails in practice |
| `main.rs` `send_binding`, `follow_folder`, `send_policy` | sends to the runtime are fire and forget: a failed one leaves the session unbound until it opens, Pluto suggesting the previous folder, or the run policy stale until the next change | a failed send is seen |
| `main.rs` `check_run_state` | the run-state note warns about every open notebook on the host, not only the session's | it confuses someone |
| `agent.rs` `run`, session list | only the first page of a folder's past sessions is listed | a folder's history outgrows a page |
| `agent.rs` mode, close, delete | sent fire and forget; the agent confirms a mode change itself, and a failed close or delete leaves the file behind | it bites |
| `main.rs` `save_json_at`, `settings.rs`, `transcript_copy.rs` | small files are saved best effort: a failed save keeps the last good file (`write_atomic`) and loses only that change; without a transcript copy a session opens on its summary | a lost change is seen |
| `main.rs` `write_atomic` | `sync_all` runs on the caller's thread, the UI thread for most saves | a save on a hot path |
| `main.rs` context ring | arcs drawn as 48-segment polylines | it looks faceted |
| `menu.rs` `pick` | the app menu's Cut, Copy, Paste and Select All reach the notebook 100 ms after it gets the keyboard back, on Linux only; on Windows they act on the app's own text boxes | a paste is lost, or Windows' menu (#10) |
| `webview_windows.rs` `SETTLE_MS` | find answers "No matches" only once WebView2's match count has stayed put for 400 ms, and a late count from the search before can answer a new one (untried on Windows) | the find bar is wrong or slow on Windows |
| `save_image.rs` `make_room` | choosing Replace in the image Save dialog removes the old file before the new one is written, so a failed save loses it | a failed save is seen |
| `logs.rs` `start` | the log isn't rotated within a run | a long run makes a big file |
| `install.rs` `download`, first-run installs | Node 24.21.0 pinned in code (bump URL, SHA-256, size per release; Julia's pin is EndeavorMCP's, checked by `runtime.rs`); curl outlives a quit mid-download, and a relaunch that overlaps it fails the SHA check and starts over | an upgrade, or overlapping launches bite |
| `own_julia.rs` `install_own_julia` | Settings' Install and a Julia notebook opened meanwhile can both download Endeavor's Julia at once (Install waits while the runtime is starting Julia, not the other way round); the second fails its check or rename, and trying again works | it bites |
| `vendor/gpui-wry` | the notebook web view's GPUI glue is a copy of gpui-wry 0.6.6, so its GPUI pin moves with ours; upstream's successor, gpui-webview, can draw menus over the page on macOS and Linux with GPUI Fast | gpui-webview with GPUI Fast lets menus over the notebook drop their holes |
| `scripts/bundle.sh` | the Mac app is ad-hoc signed | sharing (above) |
| `.github/workflows/nightly.yml` | without the Developer ID secrets the Mac nightly builds ad-hoc signed and isn't published; Apple Silicon only | sharing (above) |
| `.github/workflows/windows.yml`, `scripts/installer.iss` | the Windows build and its installer aren't signed (SmartScreen warns) | the installer is shared (#12) |
