# Other agents: Cursor, Codex and Gemini

Endeavor runs one agent today: Claude Code, through the
`@agentclientprotocol/claude-agent-acp` adapter. The sign-in screen lists
Cursor, Codex and Gemini as "Not available yet".

This note covers two things:

- the work that makes Endeavor agent-neutral, which any second agent needs;
- what has to be found out about each agent before choosing which one to add.

Cursor was tested in a spike on 2026-09-27; its findings are in
[cursor-agent.md](cursor-agent.md). Codex and Gemini haven't been tested yet.

## Where Endeavor depends on Claude Code

Each of these works only because of how Claude Code or its adapter behaves.

- **Recognising notebook tool calls.** A call is a notebook tool when its title
  starts with Claude's `mcp__notebook__` (`celldiff::notebook_tool`,
  `src/celldiff.rs`). Cell diffs, run summaries, tool details and the pane
  following a new notebook all rely on it (`session.rs`, `runs.rs`,
  `details.rs`, `transcript.rs`). Cursor titles its calls differently.
- **Tool results.** Diffs and run summaries read the tool's JSON result from
  the ACP `rawOutput` (`celldiff.rs`, `session.rs`). Cursor sends only
  `{"success": true}` there.
- **Skills.** The Pluto skills in `plugin/` reach the model as a Claude Code
  plugin, set in the session options (`session_options`, `src/agent.rs`).
- **Asking before a run.** "Ask to run" depends on a Claude Code `PreToolUse`
  hook (`plugin/hooks/hooks.json`) that calls `endeavor hook-pretool`
  (`src/gate.rs`). The hook answers "ask" for calls that run code, and that
  reaches the app as a permission request. Cursor runs no hooks for MCP calls
  in ACP mode.
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

1. **Recognise tools by name.** Use `rawInput`'s tool name, or match the
   server name in any form, as well as Claude's title. Needed by every agent
   whose titles differ from Claude's.
2. **Get tool results from the runtime.** The runtime keeps each notebook
   call's result, keyed so the app can look it up when the agent's result is
   missing or only says "success". This touches `endeavor-remote` and the
   app. It isn't needed for an agent that sends real results.
3. **Ask before a run without a hook.** Move the run check into the runtime,
   or into the app's handling of permission requests, so it doesn't need the
   agent to run Claude hooks. The runtime already knows which calls run code
   (`gate::runs_code`). How this works depends on what the agent does before
   a tool call; see the questions below.
4. **Give the agent the skills through the notebook MCP server.** Either as
   the server's MCP `instructions`, or as a guide tool the model is told to
   call first. Keep the plugin for Claude unless the MCP route proves as good.
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
10. **A sidebar that doesn't wait for any agent.** Today the sidebar's past
    sessions come from Claude's `session/list`, and Claude only starts once
    This Mac's Julia is ready, so the list waits for both. Instead, Endeavor
    keeps its own record of each session (its agent, place, title and last
    activity; `sessions.json` already has the place) and draws the sidebar
    from it at launch. Each agent's listing then updates the record: new
    titles and times, and sessions deleted elsewhere (sessions an agent made
    outside Endeavor aren't added). An
    agent that can't list sessions adds none. The default agent starts at
    launch, alongside Julia, and others start when one of their sessions is
    opened or started. A session needs its agent and its host's Julia only
    when it's opened. With more than one agent in use, rows show which agent
    each session belongs to. Worth doing first, even with Claude alone.

    Built for Claude: `sessions.json` keeps each session's agent, place,
    title and last activity (`src/records.rs`; user names stay in
    `titles.json`, archiving in `archived.json`), and the sidebar draws from
    it at launch. `Records::merge` takes one agent's listing of a folder (or
    of a server's whole agent folder) and touches only that agent's
    sessions; a failed listing changes nothing. Claude starts at launch
    alongside Julia, except during first-launch setup. A session opened
    before its host's Julia is up waits with "Starting Julia…". Still to do
    with a second agent: starting it when one of its sessions is opened, and
    showing the agent on rows.

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
- **Codex** needs an adapter, `@zed-industries/codex-acp`, which isn't
  installed. The Codex CLI is, at `/opt/homebrew/bin/codex`. It has a skills
  folder of its own (`~/.codex/skills`), so it might load Endeavor's skills
  directly; that is untested.

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

## Decisions for later

- Should Claude move to skills through the MCP server too, or keep the plugin?
- Under an agent without hooks, is its own permission prompt an acceptable
  way to ask before a run?
- Should an agent with many models (Cursor has about 40) offer all of them,
  or a short list?
