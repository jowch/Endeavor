# Testing

How to check Endeavor from a script. Screenshots are for how things look.
Everything else (which screen is up, what the transcript says, which card is
waiting, what the notebook pane shows) comes from the state dump.

## Checking changes

- **A feature change.** Check the behavior you changed through the state dump.
  Drive the app to the state, then read the part of the dump that shows it,
  for example `scripts/app-state.sh .notebook.header.tags`. Take a screenshot
  only when how it looks is part of the change.
- **A runtime, transport or helper change.** Run the Julia tests (below).
  Then do the smoke test in the app, with a real Claude turn:
  1. Ask Claude for something that needs code. Claude makes its own notebook,
     and the pane follows it: `.notebook.shows` is `page`, and
     `.notebook.file` names the new file.
  2. The transcript has a run summary and a cell diff: a `run` entry, and a
     `tool` row with `diffs`.
  3. Turn Point on, pick a cell, and send a comment. `.page.picked` has the
     cell, the user entry's `quotes` name it, and the reply is about that cell.
  4. A session on a server makes one edit: a `tool` row whose `added` is more
     than 0, with `.notebook.header.host` naming the server.

  Check each step through the dump. Take one screenshot at the end, for how
  it looks.

## The state dump

A debug build writes what's on screen as JSON when a script asks for it. Release
builds don't have it.

1. Start the app with two file paths set:

   ```sh
   ENDEAVOR_STATE_REQUEST=/tmp/e/state.request \
   ENDEAVOR_STATE_OUT=/tmp/e/state.json \
   target/debug/endeavor
   ```

2. Ask for the state with the same two variables set:

   ```sh
   scripts/app-state.sh                    # the whole dump
   scripts/app-state.sh .window.screen     # a jq filter: "new_session"
   scripts/app-state.sh '.session.approval.buttons[].label'
   ```

The script deletes the old dump, creates the request file, and waits up to 10 s
for the new dump. The app checks for the request file five times a second. When
the file is there, the app deletes it and asks the notebook page for its part.
Then the app writes the dump to a temporary file and renames it to
`ENDEAVOR_STATE_OUT`, so a reader never sees half a dump. The page gets 2 s to
answer. The code is in `src/debug_state.rs`, and the page's part is in
`frontend/src/debug.ts`.

To wait for something, poll the dump. For example, loop until
`scripts/app-state.sh '.session.activity == null'` prints `true`.

### What it holds

- `window`. The screen: `new_session`, `session`, `sign_in` or `splash`.
  Also the setup step, an open dialog (`server_dialog`, `ssh_prompt`,
  `login_node_warning`), whether Settings is open (`settings_open`), and
  whether a menu is open (`menu_open`). `menu` is the open ⋮ or ⌄ menu: what
  it's `for` (`row`, `session` for the chat header's title, `notebook` or
  `share`) and its `items`, each with its `label` and `key`.
- `offline`. Null when online. Otherwise, how long the app has been offline.
- `claude`. Claude Code's adapter process: `state` is `up`, `restarting`
  (it stopped by itself and is starting again) or `down` (it stopped twice in
  a minute); `error` is why it last stopped, with the log's last lines (null
  while up); `details_open` is whether the "Claude isn't running" card's
  Details are open.
- `usage_limit`. Null unless a turn hit the account's usage limit. Then
  `resets_in_secs`, or null when Claude Code's message gave no time.
- `notice`. A one-off failure's notice (a failed export, rename, move, run,
  restart, stop, new notebook or sign out), or null: its `spot` (where it
  hangs: `notebook_name`, `notebook_buttons`, `notebook_pane` or
  `settings`), `title`, `text` (the plain reason), `button` and `details`
  (`open`, and the raw error) when the error says more than the reason.
- `sign_in`. `account` is `unknown`, `signed_in` or `signed_out`. When signed
  out, `stage` says where sign-in is, and `card` says whether its card shows
  above the composer.
- `sidebar`. The folders in order. Each row has its title, `active`, and
  `mark` (`needs_approval`, `working` or `archived`). Also the "Show N more"
  line, the Restart Julia row, the status line (`status`) and its mark
  (`status_mark`: `offline`, `signed_out`, `spinner` while Claude restarts,
  `red_dot` when something needs the user, or null).
- `new_session`, on the new-session screen. The chips (where, resources, folder,
  notebook) with their labels, the mode, the notice and connection notice, and
  either `resume` (Pick up where you left off) or `examples`.
- `session`, for the active session. `failed` is null unless it couldn't
  open; then the page in place of its transcript: `kind` (`in_cli` for one
  open in the Claude Code CLI, else `other`), `title`, `text`, `buttons` and
  `details` (`other` only). The title, `renaming_title` (the name
  box in the chat header, from the session menu's Rename), and `transcript`: the entries
  in order, as drawn. The entry kinds are `user` (with chips, the "Not
  answered yet" line, and `delivery`, the line under a message sent with ⌘⏎
  while Claude worked, and `quotes`: each quote's `source`, `excerpt`,
  whether it has a `picture`, and its `comment`), `reply` (both with `actions`: the hover row's `copy`
  label, "Copied" just after a copy, and `time`, null for replayed history),
  `note` (such as "You stopped Claude"; the note after Claude's process
  restarted under a reply also has its `button`, "Continue", null once
  pressed), `failed` (a turn that didn't finish: its `title`, `text`,
  `button` "Try again" or "Continue", null once pressed, and `details`, the
  raw error; a Try again card leaves the transcript once pressed), `plan`,
  `thought`, `tool` and `run`. A run of tool calls is one `run` entry with its
  summary line and its rows. A `tool` row has its text ("Edited `fit`"), its
  +/− counts, how it was answered, its state (`…`, `failed` or `denied`) and
  its cell diffs. A `changes` entry is the end-of-turn card: its `cells`,
  each with the `name` shown, the `cell` id, `tag` (`new`, `deleted` or
  null), the net `added` and `removed` line counts over the turn, and
  `shows` (false for a deleted cell, whose row has no `›`). Also `activity` (the working line), `pinned_plan`, and
  `approval`: the card above the composer (`approval` or `plan`) with its
  title, code, lines, `plan` (its title, numbered `steps`, whether it's
  `open`, and the `text` shown when it is) and buttons (each with its label, key and `weight`:
  `quiet` on the left, `outlined`, or `primary`).
  `scroll` is where the transcript is scrolled: the list's top `item` and
  the `offset` into it, `px` from the top by the heights the list knows,
  `following` (it keeps to the end), `at_end`, and `rows`, each item in view
  as `[entry index, top from the viewport's top, height]`. Between two dumps
  a row in both should move by exactly the amount scrolled.
- `notebook`. What the notebook pane shows:
  - `page`: the notebook's page. `page` then has the notebook id, the
    backend, the look, safe preview, read-only and whether the page is
    connected.
  - `opening`: "Opening <file>…". `opening` then has `secs`, how long it
    has said so while Julia has the notebook open (counted again after a
    reload), `reloaded`, whether the app loaded the page again, and
    `reload_button`, whether "Reload notebook" shows.
  - `host`: the host isn't ready. `host_pane.kind` is one of `cant_reach`,
    `starting`, `stopping`, `julia_not_running`, `replaced` or
    `not_connected`, with the host and the reason.
  - `stopped`: the notebook was stopped. `stopped.idle_hours` is set when it
    stopped for being idle.
  - `crashed`: its own Julia stopped by itself: "Julia stopped unexpectedly".
    The runtime's notebook list says so (`exited`, with the cells that were
    running), and so does the page.
    (All of a host's Julia stopping shows as `host`, with `host_pane.kind`
    `julia_crashed`, and the same page.)

  `crash` is null unless Julia stopped by itself under the notebook: its
  `state` (`stopped`, `rerunning` after Restart Julia until that run is
  done, `again` when it stopped again during that run and the notebook
  opened in safe preview), the `cell` that was running, the `page` (its
  `title`, `text` and `buttons`, while `stopped`) and the safe-preview
  `callout` (its `title` and `body`, while `again`).
  - `missing`: its file isn't there.
  - `no_notebook`: the turtle's "No notebook in this session yet".

  `find` is the find bar (⌘F in the notebook), null while it's closed: its
  `text`, `found` (null before a search has an answer) and `result`, the
  words after the box ("Not found").
  `header` has the file, the host chip, the tags ("Safe preview",
  "Read-only", "Restart needed"…), the work under way, and when a cluster job
  ends. `warning` is the "Can't reach" box above the pane. On the new-session
  screen, `shows` is `new_notebook`, `safe_preview`, `loading`, `host` or
  `empty`.
- `composer`. The text, chips, `quotes` (the cards above the box, as in a
  user entry), placeholder, mode, model and effort, and Point (whether it can
  be used and whether it's on). `reply` is Reply on a selection in the chat:
  `shows` is `pill`, `prompt` (with the `quote`, the `text` typed and whether
  the `menu` is open); null when none shows. `above` lists the lines above
  the box (slash commands, offline, `usage_limit`, `claude_restarting`,
  `claude_down`, queue heading, notices). `queue` lists the
  waiting messages, each with its label (`sending now…`, `copying files…`).
  `tips` says whether the file tip and the Point tip show.
- `settings`, while Settings is open. The `section` and `page` (a
  sub-page is `claude` or `julia`), the `search` text, and the `list` on the
  left (each section's name, whether it's current, its dot, and while
  searching its count of results). Then the page as drawn: `title`, `back`,
  `subtitle`, and `groups`, each with its `heading`, `foot` and `items`. A
  row has its `title`, `state` (its status line, such as "Running · 2
  notebooks open") and `tone` (`attention` is orange, `danger` red), `desc`,
  `extra` (a hint, the chosen Julia's path and version, or Repair's
  progress), its `controls` (buttons with `enabled`, toggles with `on`, the
  idle-stop list with `open`), and `highlighted` for the row a search result
  or a link opened. While searching, the rows are the results, each with its
  `crumb` ("Notebooks › Languages").
- `confirm`, the open confirm dialog (Stop a host, Cancel a job, Repair
  Julia, Sign out, Delete session), or null. `title` and `text` are its
  words, and `buttons` has `label` and `primary` for each (Cancel, then
  the action).
- `page`. What the notebook page reports: Point on or off, the picked cells,
  `picks` (each pick's source, "rates · lines 2–3"), whether a box is drawn,
  Point's status line and comment, `reply` (`pill` or `prompt` when Reply on a
  selection in the notebook shows), the drawer's tab, whether
  the safe-preview callout shows, and `alerts`, every `window.alert` the page
  showed. In debug builds the page's `alert` is wrapped to record its text,
  and it still shows. Null when the page isn't on screen. While an alert is
  open the page can't answer, and `page` is `{"error": …}`.

A dump from a session that made a notebook, cut down:

```json
{
  "window": { "screen": "session", "setup": null, "modal": null, "menu_open": false },
  "offline": null,
  "sign_in": { "account": "signed_in" },
  "sidebar": {
    "open": true,
    "folders": [
      { "heading": "proj", "rows": [ { "title": "Notebook with x calculation", "open": true, "active": true, "mark": null, "failed": false } ], "more": null }
    ],
    "restart": null,
    "status": "Claude connected."
  },
  "session": {
    "title": "Notebook with x calculation",
    "transcript": [
      { "kind": "user", "text": "Make a notebook with one cell that sets x = 21 * 2, run it, and tell me x.", "chips": [], "unanswered": null },
      { "kind": "run", "summary": "Used 3 tools, created a notebook, read 2 cells, edited a cell", "open": false, "rows": [
        { "kind": "tool", "tool": "mcp__notebook__edit_cell", "text": "Edited x", "added": 1, "removed": 0, "approval": "allowed", "state": null,
          "diffs": [ { "label": "cell 65cbac04", "added": 1, "removed": 0 } ], "open": false }
      ] },
      { "kind": "reply", "text": "Created `x_times_two.jl` with one cell `x = 21 * 2`, ran it, and `x = 42`." }
    ],
    "activity": null,
    "approval": null
  },
  "notebook": {
    "shows": "page",
    "file": "x_times_two.jl",
    "header": { "file": "x_times_two.jl", "host": "Local", "tags": [], "busy": null, "job_ends": null },
    "warning": null,
    "page": { "backend": "pluto", "look": "endeavor", "safe_preview": false, "read_only": false, "connected": true }
  },
  "composer": { "text": "", "placeholder": "Type / for commands", "mode": "Manual", "model": "Sonnet 5", "effort": "High", "above": [], "queue": [] },
  "page": { "point": true, "picked": ["65cbac04-bbb5-11f1-8318-67c2c02a111c"], "point_status": "1 cell selected", "drawer": null, "callout": false, "alerts": [] }
}
```

The dump reads the state the views draw from, through the same helpers the
views use (`pane_shows`, `header_info`, `approval_view`, `tool_row`,
`draft_chips` and the others). A change to what a view shows goes in that
helper, so the dump follows it.

## Runtime tests against real Julia

`crates/endeavor-remote/tests/e2e_julia.rs` starts the helper and the core
with the real Julia adapter, the way the app starts This Mac's runtime. The
test then talks to the runtime the way Claude Code and the app do. It sends MCP
over `POST /mcp` with the `X-Endeavor-Session` and `X-Endeavor-Host` headers,
makes the app's `/call`s, and sends the helper's file requests. Plain
`cargo test` skips it. To run it:

```sh
cargo test -p endeavor-remote --test e2e_julia -- --ignored --nocapture
```

It takes about 40 s and prints how long each step took. Julia starts once, and
the test goes through these steps in order:

1. The MCP handshake. The server session gets the host tools and the This Mac
   session doesn't.
2. `new_notebook`, then add a cell, edit it, run it, and read its output (`42`).
3. `list_notebooks` marks `this_session` right for two sessions. A second
   notebook for the same session is refused, and so is a change to the other
   session's notebook.
4. The run policy. `endeavor/run_preview` says what an asked run would run,
   including a dependent cell. Plan mode refuses edits and runs but allows
   reads.
5. Uploads through the helper's `Place` and `Write`. The same file is reused.
   A different file with the same name becomes `decay (2).csv`.
6. Restart, as the app's Restart Julia does it. The test stops and starts the
   runtime, then reopens each notebook. The unchanged notebook runs again. The
   notebook whose file changed opens in safe preview.
7. A notebook in safe preview doesn't run code until `allow_execution`.
8. A notebook's own Julia killed during a run that `execute_cell` waits
   for. The call fails with `process_exited` and "Julia stopped unexpectedly
   while running `rates`. …", and `list_notebooks` has `exited` with that cell.
9. Idle stop with a limit of about two seconds, seen on the app's `/events`
   stream. `ENDEAVOR_IDLE_CHECK_SECS` makes the core check every second
   instead of every five minutes.

The test looks for Julia in this order. The first one found is used.

1. `ENDEAVOR_E2E_JULIA`.
2. The app's own Julia, at `~/Library/Application Support/endeavor/julia-*`.
3. `julia` on the login shell's PATH.

If none is found, the test prints `SKIPPED` and passes. With the app's Julia,
the app's depot supplies the packages. The test puts its own depot in front of
it, under `target/tmp/e2e-julia`, so Julia writes there and not into the app's
folder.

## Other debug switches

- `ENDEAVOR_FORCE_OFFLINE`: a file path. The app is offline while the file
  exists.
- `ENDEAVOR_TEST_UNREACHABLE`: a file path. While the file exists, the
  `local-test` server can't be reached. To show "Can't reach", connect a
  session to a server whose SSH host is `local-test`, create the file, and end
  that server's `endeavor-remote connect` process. Opening Settings connects
  every server too, so the same works from Where notebooks run without a
  session.
- `ENDEAVOR_TEST_PICK_JULIA`: a file path. While the file exists, Settings'
  Choose… for another Julia takes the path written in it instead of opening
  the file picker, through the same code as a real pick.
- `ENDEAVOR_TEST_ADAPTER_UPDATE`: a file path. While the file exists, an
  update to the Claude Code adapter shows as available (About, Settings'
  About and the dot on the sidebar's gear). Don't press its Update.
- `ENDEAVOR_CLAUDE_CLI`: a program that stands in for `claude` in `claude
  auth status`, `login` and `logout`, so sign-in, the account on Settings'
  Claude page and Sign out can be tested without touching the real sign-in.
- `ENDEAVOR_TEST_STUCK_OPENING`: a file path. When the file appears, the
  notebook's page is taken down, as a runtime going away does, and the pane
  says "Opening". After 5 s the app loads the page again by itself. While the
  file says `keep`, loading a notebook loads a blank page instead, so the pane
  stays stuck and shows "Reload notebook". Delete the file before clicking it.
- `ENDEAVOR_TEST_TURN_ERROR`: a file path. While the file exists, every
  turn ends as the file says, without reaching Claude. Its first line is the
  adapter's error kind (`server_error`, `overloaded`, `rate_limit`,
  `transport_lost`, or `-` for none), or `max_tokens` / `max_turn_requests`
  for a turn that stops at a limit; the next line is the error's message.
  A limit and `transport_lost` come after a short made-up reply. Examples:
  `server_error` with `API Error: 529 {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}`
  gives "Claude couldn't answer"; `rate_limit` with
  `You've hit your session limit · resets 3:05pm (America/Los_Angeles)`
  gives the usage-limit line, counting down to 3:05 PM.
- Claude's process stopping: end the test app's adapter, its child
  `node …/claude-agent-acp/dist/index.js` (`pgrep -P <app pid> -f
  claude-agent-acp`). Twice within a minute leaves it stopped. Julia
  stopping: end the runtime's Julia (the child of `endeavor-remote --helper
  core`) for the host-wide page. For one notebook's page, end that
  notebook's own Julia, a child of the runtime's Julia (`pgrep -P <runtime
  Julia pid>`; one per running notebook), with `kill -9`, idle or while a
  cell runs. The pane says "Julia stopped unexpectedly" within a second, and
  names the cell that was running, if one was. To kill it mid-run, give the
  notebook a cell that runs a while, such as `slow = (sleep(600); 1)`.
- `ENDEAVOR_TEST_NO_STEERING`: a file path. While the file exists, ⌘⏎ during
  a turn takes the path for an agent that can't steer: the turn stops, and the
  message goes next, marked "Stopped Claude's work to send this".
