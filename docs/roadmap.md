# Endeavor roadmap

Living plan; update it as items land. Design rationale lives in
[pluto-agent-design-doc.md](pluto-agent-design-doc.md) and
[design-notes.md](design-notes.md). Open gaps are
[GitHub issues](https://github.com/jowch/Endeavor/issues).

_Last updated: 2026-10-10_

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
  files, and an R setting in Settings and the server dialog (a server record's
  `r` and Settings' `r` already reach the helper; nothing in the app sets them
  yet). Starts after the first version of the eval above; Ember's own
  groundwork (a version to pin, binaries) goes on in its repository meanwhile.
- **README, landing page and documentation website.**

The notebook tools (runtime, MCP server, `endeavor serve`) live in
[EndeavorMCP](https://github.com/jowch/EndeavorMCP); their open work is in its
[docs/status.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/status.md).

## Before sharing the app

- **Signing and notarization.** `scripts/bundle.sh` builds an ad-hoc signed
  `Endeavor.app` (resources in `Contents/Resources`); sharing it needs a
  Developer ID signature, hardened runtime, and notarization. The Windows
  build isn't signed either.
- **A release pipeline** ([#13](https://github.com/jowch/Endeavor/issues/13)).
  CI keeps a Windows build and installer to try, and a "nightly"
  prerelease holds the newest Mac app and Windows installer from `main`
  (`nightly.yml`). There are no versioned releases yet.
- **Updates** ([#80](https://github.com/jowch/Endeavor/issues/80)). The app
  can't update itself or check for a newer version, so Check now is hidden.
- **The Windows installer** ([#12](https://github.com/jowch/Endeavor/issues/12)):
  a per-user installer that adds WebView2 if it's missing. CI builds it; it
  is unsigned and has been tried on one Windows machine.

## Later

- **Other ACP agents** (Codex, Antigravity, Gemini). Antigravity runs on
  Windows x64 only so far ([antigravity-agent.md](antigravity-agent.md)).
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
  Windows 10; servers (the build has no server helpers), a download page and
  signing are left. See [windows.md](windows.md), and [linux.md](linux.md)
  for the Linux port's remaining work.
- **Upstream to mthelm85/PlutoMCP.jl.** The runtime started from a fork of
  PlutoMCP that carried several general improvements (`new_notebook`, run
  state, `view_cell_output`, earlier fixes); offer the ones that aren't
  Endeavor-specific.

## Known shortcuts

Deliberate simplifications with their upgrade path, marked `ponytail:` in the
code. This is all of them; add a row with each new marker. The runtime's own
shortcuts are EndeavorMCP's, tracked in its
[issues](https://github.com/jowch/EndeavorMCP/issues); the
two this list used to carry (`view_cell_output` has no timeout while the
worker is busy, and SIGTERM can leave Julia hung mid-exit) are
[EndeavorMCP #62](https://github.com/jowch/EndeavorMCP/issues/62).

| Where | Shortcut | Upgrade when |
|---|---|---|
| `transcript.rs` | long diffs are cut, not scrollable | real notebooks hit it |
| `frontend/src/diff.ts` | the page's diff is an O(n·m) table; when the two versions' line counts multiply past 250,000, no diff is shown | big cells need a diff |
| `connection.rs` `open_line` | Reconnect on a server that dropped waits for the library's next try (up to 30 s apart) instead of trying at once; a server whose connection settings change gets a new listener port, so its sessions' agents need a restart | `client::Session` can be asked to try now, and can keep its port |
| `server_dialog.rs` `drop_asks` | when a sign-in ends while another server's question waits behind it, that question comes to the front without the focus | two servers asking at once happens in practice |
| `connection.rs` idle limit | a failed send leaves the runtime on its default idle stop (48 hours) until the next start | it fails in practice |
| `main.rs` `send_binding`, `follow_folder`, `send_policy` | sends to the runtime are fire and forget: a failed one leaves the session unbound until it opens, Pluto suggesting the previous folder, or the run policy stale until the next change | a failed send is seen |
| `main.rs` `check_run_state` | the run-state note warns about every open notebook on the host, not only the session's | it confuses someone |
| `agent.rs` session list | only the first page of a folder's past sessions is listed | a folder's history outgrows a page |
| `agent.rs` mode, close, delete | sent fire and forget; the agent confirms a mode change itself, and a failed close or delete leaves the file behind | it bites |
| `main.rs` `save_json_at`, `settings.rs`, `transcript_copy.rs` | small files are saved best effort: a failed save keeps the last good file (`write_atomic`) and loses only that change; without a transcript copy a session opens on its summary | a lost change is seen |
| `main.rs` `write_atomic` | `sync_all` runs on the caller's thread, the UI thread for most saves | a save on a hot path |
| `main.rs` context ring | arcs drawn as 48-segment polylines | it looks faceted |
| `logs.rs` | the log isn't rotated within a run | a long run makes a big file |
| `install.rs` first-run installs | Julia 1.12.6 and Node 24.21.0 pinned in code (bump URL, SHA-256, size per release); curl outlives a quit mid-download, and a relaunch that overlaps it fails the SHA check and starts over | an upgrade, or overlapping launches bite |
| `scripts/bundle.sh` | the Mac app is ad-hoc signed | sharing (above) |
| `.github/workflows/nightly.yml` | without the Developer ID secrets the Mac nightly builds ad-hoc signed and isn't published; Apple Silicon only | sharing (above) |
| `.github/workflows/windows.yml`, `scripts/installer.iss` | the Windows build and its installer aren't signed (SmartScreen warns), and the build has no server helpers | the installer is shared (#12) |
