# Cursor as a second agent

Findings from a spike on 2026-09-27, parked for now. Cursor can run
Endeavor's notebook loop. Before adding it, Endeavor needs to stop depending
on Claude-only behavior in two places: tool results and skills.

## How it was tested

- **Version.** Cursor CLI `cursor-agent` 2026.09.26, signed in.
- **ACP mode.** It has an ACP mode that its help doesn't list,
  `cursor-agent acp`. It answers initialize with:
  - protocol version 1;
  - `loadSession`, and `sessionCapabilities.list`;
  - MCP over HTTP and SSE;
  - image prompts;
  - one auth method, `cursor_login`, which isn't needed when the CLI is
    already signed in.
- **Setup.** A Python ACP client started `cursor-agent acp` against a real
  Endeavor runtime.
  - The client passed the runtime's bridge as the same SSE MCP server
    `agent.rs` passes to Claude Code, headers included.
  - It set the session folder with `endeavor/set_session_folder` and created
    the notebook with `endeavor/new_notebook`.
- **Result.** Cursor read cells, edited them, ran them, and answered a Point
  cell comment. The Point prompt had exactly the blocks `prompt_blocks`
  builds. The notebook file came out right.
- **In the app.** The spike also got the app itself to connect to Cursor. The
  switch is `ENDEAVOR_AGENT=cursor`, which starts `cursor-agent acp` in place
  of the npm adapter and checks sign-in with `cursor-agent status`. That
  change was not kept.

## What works

- **Sign-in.** It uses the CLI's own login; no `authenticate` call is needed.
- **Streamed replies.** They arrive as `agent_message_chunk`, and thinking as
  `agent_thought_chunk`.
- **Stop.** `session/cancel` ends the turn with `cancelled`.
- **Images.** Image prompts work.
- **Attached files.** Embedded text resources work, even though Cursor
  reports `embeddedContext: false`: it appends them to the message.
  - A `resource_link` makes Cursor read the file with its own tool.
- **Modes.** They are `agent`, `plan` and `ask`, offered as modes and as a
  `mode` config option.
  - The mode id `plan` already maps to the runtime's plan policy.
  - The `--mode` flag does nothing under `acp`.
- **Past sessions.** `session/list` filters by folder. `session/load` replays
  text and tool calls.
- **Titles.** Session titles arrive as `session_info_update`.

## What needs work

In order of importance:

1. **Tool results are never sent.**
   - A finished MCP call's `rawOutput` is only `{"success": true}` or
     `{"rejected": true}`.
   - The call starts titled "MCP: tool". An update renames it "pluto:
     edit_cell" and sets `rawInput` to `{providerIdentifier, toolName, args}`.
   - Endeavor recognises pluto calls by Claude's `mcp__pluto__<tool>` title in
     `celldiff.rs`, `runs.rs`, `details.rs` and `session.rs`
     (`on_tool_update`). So under Cursor there are no cell diffs, no run
     summaries and no tool details. The pane also doesn't follow a notebook
     the agent opens.
   - **Fix.** Identify tools by `rawInput.toolName` as well as by the title.
     Get results from the runtime, not the agent: the runtime can keep each
     call's result for the app to look up.
   - Tool-call ids contain a newline.
2. **Skills and hooks don't load.**
   - `--plugin-dir` loads Endeavor's Claude-format `plugin/` as is in print
     mode (`-p`). The `acp` command never passes it on.
   - Linking the plugin into `~/.cursor/plugins/local/` doesn't load it either.
   - Neither Claude `PreToolUse` hooks nor Cursor `beforeMCPExecution` hooks
     fire for MCP calls under `acp`.
   - Skills do load from `.cursor/skills`, `.claude/skills` or
     `.agents/skills` in the session folder.
   - Skills also load from `--workspace <dir>`, but that makes Cursor ignore
     the project's own AGENTS.md for every session in the process.
   - Cursor always loads the user's global `~/.claude/skills` and
     `~/.cursor/skills-cursor`. Endeavor has no way to keep the user's personal
     setup off, as it can for Claude.
   - **Preferred fix.** Deliver the skills through the pluto MCP server, which
     works for any agent. That could be MCP `instructions`, or a guide tool
     that the model is told to call first. Whether Cursor passes
     `instructions` to the model is untested.
   - **Also worth doing.** Ask Cursor to honour `--plugin-dir` under `acp`.
3. **Every notebook call asks for permission, reads included.**
   - The card offers Allow once, Allow always and Reject. Its title is like
     "pluto-read_cell: read_cell", and its content is the arguments as JSON.
   - Reject works.
   - Allow always likely writes the user's global Cursor config, so Endeavor
     should never answer it for the user.
   - Endeavor could answer Allow once to read-only pluto tools itself.
4. **Plan approval is missing.**
   - In plan mode Cursor sends a standard `plan` update. It then sends its own
     request, `cursor/create_plan`, with the plan's name, overview and
     markdown. Endeavor answers unknown requests with an error, so the plan is
     never approved.
   - The CLI also has `cursor/ask_question`, `cursor/update_todos`,
     `cursor/task`, `cursor/generate_image`, `cursor/canvas` and
     `cursor/list_available_models`.
5. **Choosing a model changes the user's global Cursor default.**
   - The only config options are `mode` and `model`. There are about 40
     models, some from providers other than Anthropic.
   - Effort is part of each model's id, as in
     `claude-opus-5-5[context=300k,effort=medium,fast=false]`. There is no
     separate effort option.
   - Choosing a model through ACP or `--model` rewrites
     `~/.cursor/cli-config.json`.
   - Endeavor re-applies the last-picked model to every new session, so it
     would keep overwriting that default. Keep a separate saved model per
     agent, or don't re-apply it for Cursor.
6. **Smaller gaps.**
   - **Send now.** There is no steering: `_session/steering` returns "Method
     not found". Endeavor's fallback, which cancels the turn and re-queues the
     message, still applies.
   - **Reopened messages.** On replay Cursor joins a message's text blocks
     into one `user_message_chunk` and drops images. Endeavor drops chunks
     that start with "[Endeavor]", so a reopened message with a chip loses the
     user's words. Replay should split the chunk on the "[Endeavor]" and
     `<attached …>` markers.
   - **Claude named in the UI.** "Claude connected." and "Fix with Claude" are
     hard-coded.
   - **Commands.** `available_commands_update`, about 30 commands including
     personal skills, is stored but not shown.
   - **Leftovers on the user's machine.** Each session leaves a folder in
     `~/.cursor/acp-sessions/`. The CLI also starts a `cursor-agent
     worker-server` process that keeps running after the CLI exits.

## Order of work when it resumes

1. **Agent-neutral groundwork, useful for Claude too.**
   - Identify tools by name, not title.
   - Get tool results from the runtime.
   - Deliver skills through the MCP server.
   - Put per-agent facts in one table in `agent.rs`: launch command, sign-in
     check, session `_meta`, and the name shown in the UI.
   - Run one ACP connection per agent.
2. **An agent picker on the new-session panel.**
3. **Cursor-specific handling.**
   - Auto-approve read-only pluto tools.
   - Show `cursor/create_plan` as a plan-approval card.
   - A model picker that warns it changes the Cursor CLI default.
   - Split joined user messages on replay.

## Testing note

Test this headless with an ACP client script against a runtime. Don't drive
the app's folder picker: the macOS open panel doesn't take synthetic
keystrokes reliably. Don't pass `--model` or change the model through ACP on
a real machine, because that changes the user's Cursor default.
