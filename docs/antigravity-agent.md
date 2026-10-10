# Antigravity as a third agent

Endeavor runs Antigravity, Google's agent, through its own ACP server,
`agy_acp_server` 1.3.0. It runs on this computer only. It is pinned for
Windows x64, Mac (Apple Silicon and Intel) and Linux (x64 and arm64); on
other platforms it shows as "Not available yet". Only Windows has run a real
session so far.
The findings it was built from came from a test on a Windows machine on
2026-10-10.

## What the server is like

- **One zip, no Node.** The server is a PyInstaller program: a launcher that
  unpacks about 313 MB into the temp folder on every start, then runs a
  second process, plus `localharness_external.exe`. It takes about 12 s to
  start from cold.
- **It doesn't exit when its stdin closes.** Every stop ends it the hard
  way, so the launcher never deletes its `_MEI*` folder.
- **Sign-in.** It offers `oauth-personal`, `oauth-business`,
  `gemini-api-key` and `agent-platform` through ACP's `authenticate`, and
  keeps the sign-in in `~/.gemini/antigravity-acp/acp_token.json`. Before
  that, `session/new` fails with "Authentication required" (-32000). The
  browser sign-in waits 5 minutes.
- **Modes.** `default`, `auto_edit` and `yolo` are permission presets, the
  same as its `mode` config option. Its other option is `model`.
- **Tool calls.** A notebook call is titled `notebook_<tool>`, with its
  arguments under `rawInput.arguments` beside copies of some of them, and
  `_meta.mcp` naming the tool and server on the `tool_call` and its first
  update. Its result is only a label ("New notebook"), not the tool's JSON.
- **Permissions.** It asks before every tool call, reads included, with
  allow-always, allow-once and reject-once options. It runs its own
  PowerShell and `view_file` even with the client's terminal off.

## What Endeavor does for it

- **Install** (`agent.rs`, `Install::Program`). The app downloads the
  pinned zip, checks its SHA-256 and unpacks it into
  `antigravity-acp-1.3.0` in its folder. The zip has its files at the top,
  so `install::tarball` takes an empty `top`. On a Mac and Linux the program
  is `agy_acp_server.par` (a native program despite the name), with a helper
  `localharness_external` beside it. The Mac's bsdtar unpacks the zip; on
  Linux, `unzip` does, since GNU tar can't. The Mac programs are signed by
  Google LLC and, downloaded by curl, carry no quarantine flag.
- **Its own temp folder** (`private_temp`). The server runs with `TEMP`,
  `TMP` and `TMPDIR` set to `antigravity-temp` in the app's folder, which
  the app empties before each start. A killed server's `_MEI*` folder is
  gone at the next start instead of piling up in the user's temp folder.
- **Its process tree** ends with the connection on Windows: the agent's job
  (`agent_job.rs`) ends the launcher, its second process and the harness.
- **The dialect** (`antigravity.rs`). Every session stays in `default`, set
  back if it was left in another preset. Endeavor shows its own Manual and
  Ask to run, made from the runtime's gate, as for Codex; there is no plan
  mode. The `mode` option and Antigravity's own mode updates are hidden. A
  notebook call, and its permission request, get the name Claude's adapter
  gives it (`mcp__notebook__<tool>`) and its arguments as they are, so cell
  diffs and run cards work. The label result isn't JSON, so the session asks
  the runtime for the call's result, as it does for Codex.
- **Permissions** (`asks_every_write`). Notebook prompts are answered
  allow-once, and the runtime decides by the session's mode. Only a call
  whose `_meta` names a notebook tool counts as one. A shell command's title
  is its command line, which the model writes, so one that reads like a
  notebook call is renamed ("Antigravity: …"). The session also lets a
  prompt through only when its title names a notebook tool exactly. Its
  PowerShell commands get the usual cards: Deny, Always this session and
  Allow. Always this session is the app's own rule for that command in this
  session, and Antigravity hears allow-once. Its own "Allow Always" is
  dropped, because Antigravity would keep it as a rule of its own, which the
  app can't show or remove and which would hold in Manual too. Its command
  comes as `CommandLine`; the app reads it as the command, so the line says
  "Ran Get-Location". Its `view_file` doesn't ask at all: it reads any file on this
  computer without a card.
- **Sign-in.** The app checks for the token file before connecting. While
  signed out, a card above the composer offers **Sign in**, which sends ACP's
  `authenticate` with `oauth-personal`; the server opens Google's page in the
  browser. A session that fails to open for want of sign-in also brings the
  card back.
- **In the app.** Antigravity is in the agent menu (where available), in
  Settings with its sign-in status, in the first-run assistant list, and in
  the state dump (`antigravity`).

## Left open

- **Other platforms.** The Mac and Linux zips are pinned from their
  checksums, and the Mac arm64 program runs, but no session has run on a Mac
  or Linux yet. Whether the `.par` program unpacks itself like the Windows
  one (`private_temp`) is unchecked.
- **Other sign-ins.** Only the personal Google sign-in is offered; a work
  account, an API key and Agent Platform aren't.
- **A first sign-in, end to end.** The card's states, Try again and a
  restart mid-sign-in are checked on Windows, but a completed sign-in from
  the card isn't yet. What is unknown is whether `session/new` works in the
  same server process straight after `authenticate`, without a restart.
- **A server that never exits.** Every stop waits out the ACP library's
  one-second grace, then kills it. The job ends its whole tree and the temp
  folder is emptied at the next start, but the server gets no clean exit.
- **Its own tools stay on.** Its PowerShell and file tools can't be turned
  off from ACP, so it stays on this computer. `view_file` reads anything
  without asking, the runtime's token file in the app's folder included.
  The token is only of use through a command, and every command gets a
  card. It is listed with EndeavorMCP #53.
- **Stop leaves a running cell running.** That is so for every agent: the
  runtime only drops a call still waiting for an answer (EndeavorMCP #59).
- **Reopened sessions.** A reopened session keeps its title, but its
  replay lists reads of Antigravity's own tool files (`edit_cell.json`, …)
  that the live session didn't show, so it doesn't match the app's copy:
  the history shows "Earlier messages were replaced" at the top. Whether
  replayed notebook calls carry `_meta` isn't known yet; without it they
  show under Antigravity's own names, without their diffs (Endeavor #58).
- **Read prompts.** Reads ask too; the app answers them allow-once, so the
  user doesn't see them, but each one costs a round trip.
