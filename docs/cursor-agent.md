# Cursor as a second agent

Findings from a spike on 2026-09-27 and a live test in the app on
2026-10-02. Cursor is parked. It can run Endeavor's notebook loop. The
agent-neutral groundwork it needed is done (tool calls recognised by name,
tool results from the runtime, skills through the notebook MCP server, runs
held by the runtime; see [other-agents.md](other-agents.md)). What is left
below is Cursor-specific.

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

1. **Skills reach Cursor only if it calls the guide.**
   - Endeavor's skills come from the notebook MCP server's `notebook_guide`
     tool. Cursor's MCP `instructions` never reach the model, and tool
     descriptions load lazily, so calling the guide first depends on the
     model choosing to (see the live test below).
   - `--plugin-dir` loads Endeavor's Claude-format `plugin/` as is in print
     mode (`-p`). The `acp` command never passes it on. Linking the plugin
     into `~/.cursor/plugins/local/` doesn't load it either. Worth asking
     Cursor to honour `--plugin-dir` under `acp`.
   - Neither Claude `PreToolUse` hooks nor Cursor `beforeMCPExecution` hooks
     fire for MCP calls under `acp`.
   - Skills do load from `.cursor/skills`, `.claude/skills` or
     `.agents/skills` in the session folder.
   - Skills also load from `--workspace <dir>`, but that makes Cursor ignore
     the project's own AGENTS.md for every session in the process.
   - Cursor always loads the user's global `~/.claude/skills` and
     `~/.cursor/skills-cursor`. Endeavor has no way to keep the user's personal
     setup off, as it can for Claude.
2. **Every notebook call asks for permission, reads included.**
   - The card offers Allow once, Allow always and Reject. Its title is like
     "notebook-read_cell: read_cell", and its content is the arguments as
     JSON.
   - Reject works.
   - Allow always likely writes the user's global Cursor config, so Endeavor
     should never answer it for the user.
   - Endeavor could answer Allow once to read-only notebook tools itself.
3. **Plan approval is missing.**
   - In plan mode Cursor sends a standard `plan` update. It then sends its own
     request, `cursor/create_plan`, with the plan's name, overview and
     markdown. Endeavor answers unknown requests with an error, so the plan is
     never approved.
   - The CLI also has `cursor/ask_question`, `cursor/update_todos`,
     `cursor/task`, `cursor/generate_image`, `cursor/canvas` and
     `cursor/list_available_models`.
4. **Choosing a model changes the user's global Cursor default.**
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
5. **Smaller gaps.**
   - **Send now.** There is no steering: `_session/steering` returns "Method
     not found". Endeavor's fallback, which cancels the turn and re-queues the
     message, still applies.
   - **Reopened messages.** On replay Cursor joins a message's text blocks
     into one `user_message_chunk` and drops images. Endeavor drops chunks
     that start with "[Endeavor]", so a reopened message with a chip loses the
     user's words. Replay should split the chunk on the "[Endeavor]" and
     `<attached …>` markers.
   - **Commands.** `available_commands_update` lists about 30 commands,
     including personal skills.
   - **Leftovers on the user's machine.** Each session leaves a folder in
     `~/.cursor/acp-sessions/` and one in `~/.cursor/projects/`. The CLI also
     starts a `cursor-agent worker-server` process that keeps running after
     the CLI exits.

## Order of work when it resumes

The agent-neutral work it waited on is done: per-agent facts in one table in
`agent.rs`, one ACP connection per agent, a per-agent saved model, the agent
named in the UI from that table, and an agent choice on the new-session
screen ([other-agents.md](other-agents.md) items 5 to 7 and 9). What is left:

1. **Replay that splits joined messages** ([other-agents.md](other-agents.md)
   item 8), needed only for Cursor.
2. **Cursor-specific handling.**
   - Auto-approve read-only notebook tools.
   - Show `cursor/create_plan` as a plan-approval card.
   - A model picker that warns it changes the Cursor CLI default, and no
     re-applying the last model to a new Cursor session.

## Testing note

Test this headless with an ACP client script against a runtime. Don't drive
the app's folder picker: the macOS open panel doesn't take synthetic
keystrokes reliably. Don't pass `--model` or change the model through ACP on
a real machine, because that changes the user's Cursor default.

## 2026-10-02: a second live test

Findings from a live session in the app, with the agent-neutral groundwork
partly in place.

- Same CLI build, `2026.09.26-dd393fe`. `initialize`'s answer is unchanged.
  Tool-call ids no longer carry a newline.
- Model `composer-2.5` tested. Setting it through `session/set_config_option`
  rewrote both `~/.cursor/cli-config.json` and `acp-config.json` (the user's
  global default) — confirmed, as expected.
- MCP `instructions` don't reach the model, and tool descriptions load
  lazily. In one fresh session the guide was called second; in another it
  was never called, and that session went on to write a two-expression cell
  and hit the error. Because of this, a tool-misuse error points an agent
  without the plugin at `notebook_guide`.
- The notebook call sequence: a placeholder `tool_call` ("MCP: tool", empty
  `rawInput`), a `tool_call_update` ("notebook: `<tool>`", `rawInput`
  `{providerIdentifier, toolName, args}`), `in_progress`, a
  `request_permission` ("notebook-`<tool>`: `<tool>`", no `rawInput` at
  all), then `completed`. `rawOutput` is always `{"success": true}`, even
  when the tool call itself failed.
- Every notebook call asks for permission, reads included (Allow once,
  Allow always, Reject). Cursor's own Find, Read and grep don't ask.
- No hooks fire. At the time, the runtime's "ask" policy let runs through;
  only Cursor's own prompt stopped them, and that depends on the user's
  global Cursor approval settings. The runtime now holds runs itself
  ([other-agents.md](other-agents.md), work item 3); no test of that with
  Cursor is recorded.
- Plan mode: a `plan` update, a "Create Plan" tool call with `rawInput`
  `{"_toolName": "createPlan", ...}`, then Cursor's own request,
  `cursor/create_plan`, carrying `{toolCallId, name, overview, plan,
  todos}`. Endeavor answers unknown requests with an error (`-32601`);
  Cursor saved the plan to `~/.cursor/plans/<name>.plan.md` and ended the
  turn anyway. Its own bundle suggests the expected reply is
  `{"outcome": {"outcome": "accepted"}}` or `{"outcome": {"outcome":
  "rejected", "reason"}}` — untested. Plan mode by itself doesn't stop
  notebook tool calls; the runtime's own plan policy has to.
- It loads the user's global `~/.cursor/skills-cursor` skills.
- Each session leaves a folder in `~/.cursor/acp-sessions/` and one in
  `~/.cursor/projects/`.
