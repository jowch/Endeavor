# Testing

How to check Endeavor from a script. Screenshots are for how things look.
Everything else (which screen is up, what the transcript says, which card is
waiting, what the notebook pane shows) comes from the state dump.

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

- `window`. The screen: `new_session`, `session`, `settings`, `sign_in` or
  `splash`. Also the setup step, an open dialog (`server_dialog`, `ssh_prompt`,
  `login_node_warning`), and whether a menu is open.
- `offline`. Null when online. Otherwise, how long the app has been offline.
- `sign_in`. `account` is `unknown`, `signed_in` or `signed_out`. When signed
  out, `stage` says where sign-in is, and `card` says whether its card shows
  above the composer.
- `sidebar`. The folders in order. Each row has its title, `active`, and
  `mark` (`needs_approval`, `working` or `archived`). Also the "Show N more"
  line, the Restart Julia row, and the status line.
- `new_session`, on the new-session screen. The chips (where, resources, folder,
  notebook) with their labels, the mode, the notice and connection notice, and
  either `resume` (Pick up where you left off) or `examples`.
- `session`, for the active session. The title, and `transcript`: the entries
  in order, as drawn. The entry kinds are `user` (with chips and the "Not
  answered yet" line), `reply`, `note` (such as "Turn failed"), `plan`,
  `thought`, `tool` and `run`. A run of tool calls is one `run` entry with its
  summary line and its rows. A `tool` row has its text ("Edited `fit`"), its
  +/− counts, how it was answered, its state (`…`, `failed` or `denied`) and
  its cell diffs. Also `activity` (the working line), `pinned_plan`, and
  `approval`: the card above the composer (`approval` or `plan`) with its
  title, code, lines and buttons.
- `notebook`. What the notebook pane shows:
  - `page`: the notebook's page. `page` then has the notebook id, the
    backend, the look, safe preview, read-only and whether the page is
    connected.
  - `opening`: "Opening <file>…".
  - `host`: the host isn't ready. `host_pane.kind` is one of `cant_reach`,
    `starting`, `stopping`, `julia_not_running`, `replaced` or
    `not_connected`, with the host and the reason.
  - `stopped`: the notebook was stopped. `stopped.idle_hours` is set when it
    stopped for being idle.
  - `missing`: its file isn't there.
  - `no_notebook`: the turtle's "No notebook in this session yet".

  `header` has the file, the host chip, the tags ("Safe preview",
  "Read-only", "Restart needed"…), the work under way, and when a cluster job
  ends. `warning` is the "Can't reach" box above the pane. On the new-session
  screen, `shows` is `new_notebook`, `safe_preview`, `loading`, `host` or
  `empty`.
- `composer`. The text, chips, placeholder, mode, model and effort, and Point
  (whether it can be used and whether it's on). `above` lists the lines above
  the box (slash commands, offline, queue heading, notices). `queue` lists the
  waiting messages, each with its label (`sending now…`, `copying files…`).
  `tips` says whether the file tip and the Point tip show.
- `page`. What the notebook page reports: Point on or off, the picked cells
  and the drawn box, Point's status line and comment, the drawer's tab, whether
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

### Other debug switches

- `ENDEAVOR_FORCE_OFFLINE`: a file path. The app is offline while the file
  exists.
- `ENDEAVOR_TEST_UNREACHABLE`: a file path. While the file exists, the
  `local-test` server can't be reached. To show "Can't reach", connect a
  session to a server whose SSH host is `local-test`, create the file, and end
  that server's `endeavor-remote connect` process.
