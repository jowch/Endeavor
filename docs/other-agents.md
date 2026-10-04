# Other agents: Cursor, Codex and Gemini

Endeavor runs one agent today: Claude Code, through the
`@agentclientprotocol/claude-agent-acp` adapter. The sign-in screen lists
Cursor, Codex and Gemini as "Not available yet".

This note covers two things:

- the work that makes Endeavor agent-neutral, which any second agent needs
  (items 1 to 4 are done, and item 10 is done for Claude);
- what has to be found out about each agent before choosing which one to add.

Cursor was tested in a spike on 2026-09-27 and again live on 2026-10-02; its
findings are in [cursor-agent.md](cursor-agent.md). Codex was tested on
2026-10-04 and is being added; its findings are in
[codex-agent.md](codex-agent.md). Gemini hasn't been tested yet.

## Where Endeavor depends on Claude Code

Each of these works only because of how Claude Code or its adapter behaves.

- **Skills.** The Pluto skills in `plugin/` reach Claude as a Claude Code
  plugin, set in the session options (`session_options`, `src/agent.rs`).
  Other agents get them from the notebook MCP server (work item 4).
- **Reads without asking.** `allowedTools` in the session options lets the
  read-only notebook tools through without a prompt. Cursor asks for every
  call.
- **Personal setup.** "Load your personal Claude Code setup" works through the
  `settingSources` and `strictMcpConfig` options.
- **Modes.** Manual, Ask to run, Auto and Plan are Claude's `default`, `auto`
  and `plan` modes, plus Endeavor's run gate (`app_modes`, `src/session.rs`).
  Agents without `plan` and `auto` fall back to showing their own modes.
- **Plan approval.** Claude's end of plan mode arrives as a permission request
  carrying the plan, which becomes the plan card (`src/approval.rs`). Cursor
  sends its own `cursor/create_plan` request instead.
- **Model and effort.** These are the adapter's `model` and `effort` config
  options. The last pick is saved and applied to every new session. For
  Cursor, applying a model rewrites the user's global Cursor default.
- **Send now.** Steering uses the adapter's `_session/steering`. Without it,
  Endeavor stops the turn and sends the message first after it; that fallback
  works for any agent.
- **Sign-in.** `claude auth login` and `claude auth status` (`src/signin.rs`).
- **Starting the agent.** `src/agent.rs` installs and starts the npm adapter
  (`ADAPTER_PACKAGE`), with Claude's options in the session `_meta`.
- **Reopened sessions.** Replay expects Endeavor's "[Endeavor]" notes and
  quote blocks as separate text blocks. Cursor joins a message's blocks into
  one.
- **The name "Claude" in the app.** About 120 strings in `src/` and
  `frontend/src/`, such as "Claude connected.", "Fix with Claude",
  "Reply to Claude" and "Claude's reply".

## Work to make Endeavor agent-neutral

In order. Each part is also useful to Claude, or harmless to it.

1. **Recognise tools by name.** Done. Every tool call, update and
   permission request passes through `celldiff::name_notebook_call` as it
   reaches the session. A notebook call named another way gets Claude's
   title, `mcp__notebook__<tool>`, and its arguments become its input. It
   recognises a `rawInput` that wraps the call (Cursor's `{providerIdentifier,
   toolName, args}`, or `server`/`tool`/`arguments`), and titles that name
   only the server and a notebook tool ("notebook: edit_cell",
   "notebook-read_cell: read_cell", "edit_cell (notebook MCP Server)"); a
   server name must end the word there, so `notebook_guide` isn't read as the
   server alone. The tool must be one the runtime offers
   (`endeavor_mcp::is_tool`). A permission request with no `rawInput`
   (Cursor's) takes the input already recorded on the matching
   `tool_call_update`. Everything after that, cards, approvals, run previews,
   diffs, the transcript and the pane following a new notebook, asks
   `celldiff::notebook_tool`. Claude Code's own names stay where they are
   Claude settings: the `allowedTools` list and permission rule words
   (`permits::rule_words`).
2. **Get tool results from the runtime.** Done. The core keeps each agent
   session's last 64 tool results in memory
   (`crates/endeavor-mcp/src/results.rs`), recorded before the reply goes
   out: the call's text content, whether it failed, and its key. The key is
   the session (`X-Endeavor-Session`), the id the agent's client gave the
   call when it sends one, and the tool and its arguments (in one canonical
   form, so `1` and `1.0` match). Claude Code sends
   `_meta["claudecode/toolUseId"]` with every `tools/call`, the same id its
   ACP adapter gives the tool call; Cursor's `_meta` is unknown. The app asks
   with the `/call` method `endeavor/tool_result {owner, call_id, tool,
   arguments}`, which answers `{content, isError}` for the call with that id,
   else for the oldest one not yet looked up with the same tool and
   arguments, else null.

   The app asks only when a finished notebook call's output isn't the tool's
   result (`Effect::FetchResult` in `on_tool_update`). The reply becomes the
   call's output, a failed result marks the call failed, and the rest
   follows as if the agent had sent it: cell code, diffs, the pane following
   a new notebook. Claude's results still come from its own replies, which
   reopened sessions need. Endeavor's transcript copy keeps the runtime's
   result when a replay brings back only the agent's placeholder. Not asked
   while a reopened session replays: the runtime keys results by the app's
   session key, which a new launch doesn't share.
3. **Ask before a run without a hook.** Done, in the runtime.
   `endeavor_mcp::runs_code` is the one list of calls that run code
   (`execute_cell`, `submit_changes`, `run_all_cells`, `allow_execution`,
   `delete_cell`, `run_shell`, and `edit_cell` or `add_cell` with
   `run_after`). The app tells the runtime each session's policy ("ask",
   "auto" or "plan") with `asks: true`; an app that doesn't send it doesn't
   get its runs held. In "ask", after the plan, host and one-notebook
   refusals, such a call waits in the runtime
   (`crates/endeavor-mcp/src/asks.rs`) and shows in the event stream's
   `asks` (`{id, owner, call_id, tool, arguments, since}`). The app shows it
   as a run card and answers with `endeavor/answer_run {id, allow,
   user_ran}`; `user_ran` carries the cells the user's own run reached
   meanwhile (Run anyway). Allowed, the call goes on. Denied, an edit that
   was to run after is made, staged and not run, with a `not_approved`
   warning in its receipt; any other call fails with `not_approved`. The
   waiting call holds no lock, and gives up when the agent cancels it (MCP
   `notifications/cancelled`) or its connection closes (checked every
   250 ms). With no app following the event stream, a call that would wait
   fails at once with `no_app`.

   In the app, the card is the same run card as before, on the call the ask
   names (else the latest call under way with the same tool and arguments).
   The agent's own prompt before a run the runtime asks about is answered
   allow-once at once, with no card and no mark on its row, so nothing is
   asked twice. Always this session on a run card switches to Auto and
   tells the runtime "auto".

   Manual holds edits in the runtime too, so it means the same whatever the
   agent's own settings allow (Cursor's global "Allow always", a personal
   Claude setup, a folder rule). The app adds `edits: true` to
   `endeavor/set_policy` in Manual (Claude's `default` mode); an older
   runtime ignores it and still holds runs. With it, the runtime holds every
   call `endeavor_mcp::changes_notebook` names (`edit_cell`, `edit_cells`,
   `add_cell`, `delete_cell`, `move_cell`, `fold_cell`, `new_notebook`)
   whatever the run policy, so an edit still asks after runs were allowed
   for the session; `asks_first` is the one rule, used by the runtime and the
   app. Reads never wait. The app shows a held edit as the edit card the
   agent's own prompt would get, and lets the agent's own prompt for it
   through at once. Denied, the change isn't made, even an edit that was to
   run after, and the call fails with `not_approved` ("The user chose not to
   make this change."). Always this session remembers the same rule as for
   the agent's own prompts (the same tool), and the app answers later held
   calls it matches at once. In this folder isn't offered on runtime cards:
   the agent's folder rule only answers the agent's own prompt.
   Stopping the turn denies what still waits; a card whose call the agent
   gave up on goes, with a note. Claude Code's MCP client gives up on a POST
   whose response hasn't begun within 60 seconds ("The operation timed
   out."; the larger of 60 s, the server's `timeout` or `MCP_TOOL_TIMEOUT`,
   and `MCP_TIMEOUT`), so a held call's response begins at once as an event
   stream that says every 15 seconds that the call is still waiting (a
   progress notification when the call asked for progress, else an SSE
   comment), and ends with the reply. It also aborts an MCP call with no
   answer for five minutes, so the app starts it with
   `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT=0`. Cursor's own timeout is untested.

   A runtime started by an older Endeavor (it outlives the app; see
   [remote-sessions.md](remote-sessions.md)) can't hold runs. Each runtime
   reports the build it came from (`build` in its event stream, from the
   helper's `--build`, the same version `remote::version()` computes), and
   the app compares it with its own. On another build, or none, each session
   on that host gets a note to restart Julia, and the host's listener answers
   the agent's calls that runtime can't carry out safely. It checks every
   request on a connection, also after one it passed through (Claude Code
   sends `GET /mcp` first and reuses the connection). The rules are one
   function, `older_runtime::refusal`; today there is one: in Ask to run, a
   call that runs code fails with `older_runtime`. In Manual nothing is
   refused: an older runtime can't hold edits, so the app leaves the agent's
   own prompt to ask.
4. **Give the agent the skills through the notebook MCP server.** Done, as a
   guide tool (`crates/endeavor-mcp/src/guide.rs`). The server's MCP
   `instructions` are three sentences: what the tools are for, and to call
   `notebook_guide` once before the first notebook call. With no arguments
   the tool returns the three skills (`pluto-session`, `pluto-workflow`,
   `pluto-semantics`, about 15 KB) without their front matter; links to
   reference files become topics, which the tool returns when called with
   `topic`. The text is the skill files themselves, built into the helper
   with `include_str!`, so `plugin/skills` stays the one source.

   Why a tool and not the instructions: the skills are about 44 KB with
   their references, too long for instructions, which clients may cut short
   or drop; a tool call works on any client that calls tools, and the
   references load only when needed, as skills do. The tool's own
   description also says to call it first, for agents that ignore
   instructions. Cursor's MCP `instructions` never reached the model, and
   tool descriptions load lazily, so calling the guide first depends on the
   model choosing to (see [cursor-agent.md](cursor-agent.md)). A
   notebook-tool error whose kind means the agent used a tool wrong appends
   a line pointing at `notebook_guide`, for a caller without the plugin.

   Claude doesn't get it twice: the app sends `X-Endeavor-Skills: plugin`
   with each session's MCP requests (`Tools::mcp_server`), and the server
   then leaves out both the instructions and the tool. An agent that doesn't
   load the plugin should leave the header out (work item 5's table).
5. **One table of per-agent facts** in `src/agent.rs`: how to install and
   start it, how to check sign-in, its session options, its modes, and the
   name shown in the app. Run one ACP connection per agent.
6. **Name the agent in the app from that table.** Replace the fixed "Claude"
   strings. The page script already names the agent through one `AGENT`
   constant (`frontend/src/prompt.ts`); set it from the table.
7. **A per-agent saved model**, and never re-apply a model to an agent where
   that changes the user's own default.
8. **Replay that doesn't depend on block boundaries.** Split a joined user
   message on the "[Endeavor]" and `<attached …>` / `<quote …>` markers.
9. **An agent choice on the new-session screen**, and each agent's sign-in on
   the sign-in screen.
10. **A sidebar that doesn't wait for any agent.** Done for Claude:
    `sessions.json` keeps each session's agent, place, title and last
    activity (`src/records.rs`; user names stay in `titles.json`, archiving
    in `archived.json`), and the sidebar draws from it at launch.
    `Records::merge` takes one agent's listing of a folder (or of a server's
    whole agent folder) and touches only that agent's sessions; sessions an
    agent made outside Endeavor aren't added, and a failed listing changes
    nothing. Claude starts at launch alongside Julia, except during
    first-launch setup. A session opened before its host's Julia is up waits
    with "Starting Julia…". Still to do with a second agent: starting it
    when one of its sessions is opened, and showing the agent on rows.

After that, each agent needs its own handling of whatever its answers to the
questions below turn up, such as Cursor's plan request.

## What to find out about each agent

Answer each question for Codex and Gemini the way the Cursor spike did.
The answers decide how much of the work above each agent needs, and which
agent is cheapest to add.

1. **How to start it.** Does it speak ACP itself, or through an adapter?
   What does it answer to `initialize`: protocol version, `loadSession`,
   session listing, MCP over HTTP, images, embedded resources?
2. **Sign-in.** Does it use its CLI's own login, and how can Endeavor check
   whether the user is signed in?
3. **Tool calls.** What title and `rawInput` does a notebook MCP call get?
   Does `rawOutput` carry the tool's real result?
4. **Before each tool call.** Does it ask permission, and for which calls?
   What options does the request offer, and does any of them write the user's
   global config? Does it run any hook for MCP calls in ACP mode?
5. **Skills and instructions.** Can a session be given Endeavor's skills
   without touching the user's own setup: a plugin or extension folder,
   a skills folder in the session folder, MCP `instructions`? Does it always
   load the user's global skills or instructions, and can that be turned off?
6. **Modes and plans.** What modes does it offer? How does plan mode end, and
   how is a plan approved?
7. **Models.** Is there a model option, and an effort option? Does choosing
   one change the user's global default?
8. **Turns.** Does streaming, thinking, cancel and steering work? What does a
   turn end with when it's stopped?
9. **Past sessions.** Does `session/load` replay messages as they were sent,
   with images and separate text blocks? Are there session titles?
10. **Leftovers.** What does a session leave on the user's machine: files,
    folders, background processes?

What is known already:

- **Gemini CLI** speaks ACP itself. Installed here at `/opt/homebrew/bin/gemini`.
- **Codex** is answered in [codex-agent.md](codex-agent.md). Its adapter is
  `@agentclientprotocol/codex-acp`; the older `@zed-industries/codex-acp`
  is deprecated.

## How to investigate

Follow the Cursor spike's method:

- **Use a script, not the app.** Write a small ACP client that starts the
  agent against a real Endeavor runtime. Pass the runtime's bridge as the MCP
  server, with the same headers `src/agent.rs` passes to Claude Code. Set the
  session folder with `endeavor/set_session_folder` and make a notebook with
  `endeavor/new_notebook`.
- **Run the notebook loop:** read a cell, edit it, run it, answer a Point
  comment. Then check the notebook file.
- **Record every message** from the agent, so the questions above can be
  answered from the log.
- **Don't change the user's setup.** Don't pass a model flag or pick a model,
  and don't answer a permission option that writes global config. Note any
  files or processes the agent leaves behind.
- **Write the findings** in a note per agent, like
  [cursor-agent.md](cursor-agent.md).

## Later: continue a session with another agent

ACP can't move a session between agents: each agent keeps its own history,
and `session/load` and forking only work within one agent. Endeavor can
hand a session over instead, as "Continue with…" in the session menu:

1. Start a session with the new agent in the same folder and notebook.
2. Send it the conversation so far ahead of the next message: the messages,
   replies, which cells changed and ran, and quotes. Summarise older turns
   when the whole history won't fit.
3. Show both halves as one thread, divided by a line such as "Continued with
   Cursor · 14:02", and remember that the two sessions belong together.

The notebook carries the real state (its code, outputs and file), and the new
agent reads it with the same tools, so the work itself carries over. What
doesn't: the old agent's own reasoning, the full detail of its tool calls,
images in old messages, and its approvals and mode. Switching back starts
another session; the first one can still be reopened, but doesn't know what
happened after the handoff. This needs the work above and a second agent.

## Permission rules

Decided 2026-10-01. Each rule has one owner, chosen by how long it lasts, so
nothing has to be kept in sync:

- **Always this session** belongs to Endeavor. Endeavor answers the request
  with the agent's allow-once option and remembers a rule in memory for that
  session; later matching requests get allow-once at once, with no card. The
  agent only ever sees allow-once, so this works the same for every agent and
  writes nothing to its config. For Endeavor's own run prompts it switches the
  session to Auto instead. What counts as the same request: the same file for
  edits, the same site for fetches, and for commands the prefix the agent's
  own suggested rule names.
- **Always in this folder** belongs to the agent. It is offered only on the
  agent's own prompts, only when the request carries an allow-always option,
  and only for agents whose lasting rules stay inside the session folder.
  Claude Code writes them to `.claude/settings.local.json`, which Endeavor's
  session options load (`settingSources` always includes `local`). Cursor's
  "always" changes the user's global config, so Cursor gets only "This
  session". Endeavor keeps no copy; it reads the agent's settings to list the
  folder's rules ("Allowed in this folder" in the session menu, each with
  Remove).

A call that a folder rule allows never reaches Endeavor, so its row has no
"allowed" mark. The per-agent table (work item 5) records whether an agent's
lasting rules stay in the folder and whether Endeavor can read them.

A folder allow rule for a notebook run, or in Manual for a notebook edit,
only answers Claude's own prompt: the runtime still holds the call, and its
card offers no "In this folder".

## Endeavor's copy of the transcript

The agent's copy of a session is still the one that matters. Endeavor also
keeps a display-only copy of each session's transcript
(`src/transcript_copy.rs`), so the history can show before the agent (and the
host's Julia) have loaded the session, including when a session's server is
offline. The look is in [ui-spec.md](ui-spec.md), Layout and Chat.

- **Where.** `transcripts/<session id>.json` in Endeavor's support folder,
  saved when a turn ends and when the session closes, with long tool input
  and output cut short. Deleting a session deletes it. A session saved
  before the copy existed has none, and opens on its summary.
- **At once, read-only.** Opening a session shows Endeavor's copy right
  away, read-only, with a faint "Loading…" line (or the usual Julia, server
  or Claude wait line). A message can still be typed; it waits and sends
  once the agent has loaded the session.
- **The swap.** The agent's replay gathers aside, then replaces Endeavor's
  copy. The replay carries no message ids, so it's matched against
  Endeavor's copy by order and text (a tool call also by its id). The view
  keeps its place: the message at the top of the view stays where it was.
  - If every message matches, nothing moves.
  - If the replay only adds messages at the end — the session was
    continued outside Endeavor, such as with `claude --resume` — the view
    stays where it was, even at the bottom, and a floating button, "N new
    messages ↓", appears above the composer.
  - If earlier messages differ — the session was compacted, or rewound,
    outside Endeavor — the thread updates around the view's place, and the
    button reads "Jump to latest ↓" instead.
- **Not only after a replay.** The same floating button appears in any
  transcript scrolled up from its end, such as while Claude is still
  streaming a reply.
- **Display only.** Endeavor's copy is never sent to the agent. The cost is
  disk space in Endeavor's own folder, and a second place the conversation
  is stored on this Mac.
- **Not built:** step 2 of "Continue a session with another agent" reading
  this copy for the conversation so far.

## Decisions for later

- Should Claude move to skills through the MCP server too, or keep the plugin?
  Dropping the `X-Endeavor-Skills` header and the plugin's skills would try it.
- Should an agent with many models (Cursor has about 40) offer all of them,
  or a short list?
