# Codex as a second agent

Findings from a headless test on 2026-10-04. Codex can run Endeavor's
notebook loop through an ACP adapter, with the notebook MCP server passed in
`session/new` the way Endeavor passes it to Claude Code. Two things need
Codex-specific work before it is usable. Codex doesn't wait for a run the
runtime holds for long, and it asks for notebook edits through its own
permission prompt, which carries only a tool call id.

The adapter Endeavor should use is `@agentclientprotocol/codex-acp`, not
`@zed-industries/codex-acp`. The Zed package is deprecated: its npm page
says so, and its repository's README says development moved to
`agentclientprotocol/codex-acp`. Its 0.16.0 build carries an old Codex
engine that can't read today's model list. This note covers both, because
the brief named the Zed package. Where they differ, "Zed 0.16.0" and
"ACP 2.1.1" name them.

## How it was tested

- **Versions.** Codex CLI 0.160.0 (`~/.local/bin/codex`), signed in with
  ChatGPT. Zed adapter 0.16.0. ACP adapter 2.1.1, which bundles
  `@openai/codex` 0.159.3. The runtime was `endeavor serve` from EndeavorMCP,
  build `0.1.0-6379a838715a96d2`, with a private state folder, session folder
  and depot.
- **Client.** A Python ACP client started each adapter with the session
  folder as its working folder and logged every message both ways. It passed
  the runtime as an HTTP MCP server named `notebook`, with `Authorization`
  and `X-Endeavor-Session` and without `X-Endeavor-Skills`, so Codex got the
  guide tool. It followed `/endeavor/events` as the app does, set each
  session's policy with `endeavor/set_policy`, and answered held runs with
  `endeavor/answer_run`.
- **Permissions.** The client answered every prompt with its allow-once
  option, and declined plan approval. It never chose an option that saves a
  rule.
- **Model turns.** 11 in total: 3 with Zed 0.16.0, 8 with ACP 2.1.1. The
  model was each adapter's default; no model was picked except in one
  no-turn test of the model option.
- **Scripts.** In the session scratchpad, `codex-study/`: `acp.py` (client),
  `s1_init.py` (initialize and `session/new`), `s2_turns.py` (the turns),
  `s3_load.py` (`session/list` and `session/load`), `s5_options.py` (mode,
  model and effort options). The logs are the `*.jsonl` files beside them.

## Answers

### 1. Starting it

**Zed 0.16.0.** `npx @zed-industries/codex-acp`. A Node wrapper runs a
188 MB native binary from a platform package. The binary has Codex built in
(codex-core `rust-v0.137.0`), so it ignores the installed `codex`. That engine
fails to read the current model list (`unknown variant 'max'`, logged on
every start) and falls back to a built-in list that ends at `gpt-5.5`. The
wrapper doesn't pass SIGTERM on: stopping it left the native binary running,
still holding its MCP connection to the runtime.

**ACP 2.1.1.** `npx @agentclientprotocol/codex-acp`. A Node program that
starts one `codex app-server` child for the whole adapter process, from its
bundled `@openai/codex`. `CODEX_PATH` points it at another `codex`; with
`~/.local/bin/codex` (0.160.0) `session/new` and the options below worked.
It exits when its stdin closes and leaves no process behind. Its README
lists the environment variables it reads: `CODEX_CONFIG` (JSON merged into
each session's config), `INITIAL_AGENT_MODE`, `CODEX_PATH`,
`MODEL_PROVIDER`, `NO_BROWSER`, `APP_SERVER_LOGS`. The installed packages
take 347 MB.

`initialize`, trimmed:

```json
// ACP 2.1.1
{"protocolVersion": 1,
 "agentInfo": {"name": "@agentclientprotocol/codex-acp", "title": "Codex", "version": "2.1.1"},
 "agentCapabilities": {"loadSession": true,
   "promptCapabilities": {"embeddedContext": true, "image": true},
   "sessionCapabilities": {"resume": {}, "list": {}, "close": {}, "delete": {}, "fork": {},
                           "additionalDirectories": {}, "subagents": {}},
   "mcpCapabilities": {"acp": false, "http": true, "sse": false}, "auth": {"logout": {}}},
 "authMethods": [{"id": "api-key"}, {"id": "chat-gpt", "name": "ChatGPT"}],
 "_meta": {"steering": {"supported": true}}}

// Zed 0.16.0
{"protocolVersion": 1,
 "agentCapabilities": {"loadSession": true,
   "promptCapabilities": {"image": true, "audio": false, "embeddedContext": true},
   "mcpCapabilities": {"http": true, "sse": false, "acp": false},
   "sessionCapabilities": {"list": {}, "resume": {}, "close": {}}, "auth": {"logout": {}}},
 "authMethods": [{"id": "chatgpt"}, {"id": "codex-api-key"}, {"id": "openai-api-key"}]}
```

Both take MCP over HTTP only. Endeavor already passes HTTP.

### 2. Sign-in

Codex uses the CLI's own login in `~/.codex/auth.json`. No `authenticate`
call was needed. Two read-only ways to check it:

- `codex login status` prints `Logged in using ChatGPT` and exits 0.
- ACP 2.1.1 sends `_auth/status_update` right after `initialize`, and again
  on every change:
  `{"authStatus": {"kind": "account", "label": "ChatGPT Plus", "account":
  {"email": "…", "plan": "plus"}}}`. Its source sends `kind: "none"` when
  signed out.

Signed out, both adapters refuse `session/new`, `session/load` and
`session/prompt` with ACP's auth-required error (source only; signing out
was off limits). The `chat-gpt` method of `authenticate` starts Codex's
browser login. `codex login` does the same from a terminal.

### 3. Notebook tool calls

Both adapters put the call in `rawInput` as `{server, tool, arguments}`,
which `celldiff::name_notebook_call` already recognises. Neither sends a
tool-use id with the MCP call, so the runtime records `call_id: null` and
`endeavor/tool_result` has to match by tool and arguments.

**ACP 2.1.1**, live and in replay alike:

```json
{"sessionUpdate": "tool_call", "toolCallId": "exec-b829600f-…", "kind": "execute",
 "title": "mcp.notebook.execute_cell", "status": "in_progress",
 "rawInput": {"server": "notebook", "tool": "execute_cell",
              "arguments": {"notebook_id": "9dd537ce-…", "cell_id": "a2ae2d0c-…"}}}
{"sessionUpdate": "tool_call_update", "toolCallId": "exec-b829600f-…", "status": "completed",
 "rawOutput": {"result": {"content": [{"type": "text", "text": "{\"affected_cells\":[…],\"applied\":true,…}"}]}}}
```

The result is in `rawOutput.result`, with no `content`. `celldiff::tool_json`
doesn't read that shape, so today each finished call would cost a
`tool_result` lookup, and a replay (where the app doesn't look up) would show
no result.

**Zed 0.16.0**, live:

```json
{"sessionUpdate": "tool_call", "toolCallId": "call_f4I4In7x…", "title": "Tool: notebook/read_cell",
 "status": "in_progress",
 "rawInput": {"server": "notebook", "tool": "read_cell", "arguments": {"notebook_id": "b05fe4ee-…", "cell_id": "b9d630b4-…"}}}
{"sessionUpdate": "tool_call_update", "toolCallId": "call_f4I4In7x…", "status": "completed",
 "content": [{"type": "content", "content": {"type": "text", "text": "{\"cell_id\":\"b9d630b4-…\",\"code\":\"x = 1 + 1\",\"output\":\"2\",…}"}}],
 "rawOutput": {"content": [{"type": "text", "text": "…same…"}], "isError": false}}
```

No `kind`. The result is in both `content` and `rawOutput`. Replay differs
from live: see question 9.

The transcript for the requested sequence (`list_notebooks`, `new_notebook`,
`add_cell`, `execute_cell`, `read_cell`) is in `zed-turns.jsonl` and
`acp2-turns.jsonl`. Both models called `notebook_guide` first, read the new
notebook's empty cell, added `x = 1 + 1`, ran it and read back `2`. The
notebook files came out right.

### 4. Permission

**Zed 0.16.0.** A session folder Codex doesn't trust starts in its
`read-only` mode. Notebook reads (`list_notebooks`, `read_cell`,
`notebook_guide`) ran without asking. Every notebook write asked
(`new_notebook`, `add_cell`, `execute_cell`):

```json
{"toolCall": {"toolCallId": "call_UBeTFlEP…", "status": "pending", "title": "Approve MCP tool call",
   "content": [{"type": "content", "content": {"type": "text",
     "text": "Allow the notebook MCP server to run tool \"new_notebook\"?\n\nServer: notebook\n\n<description>\n\nArguments:\n{}"}}],
   "rawInput": {"server_name": "notebook", "id": "mcp_tool_call_approval_call_UBeTFlEP…",
     "request": {"mode": "form", "_meta": {"codex_approval_kind": "mcp_tool_call",
       "persist": ["session", "always"], "tool_description": "…", "tool_params": {}}}}},
 "options": [{"optionId": "approved", "kind": "allow_once", "name": "Allow"},
             {"optionId": "approved-for-session", "kind": "allow_always", "name": "Allow for this session"},
             {"optionId": "approved-always", "kind": "allow_always", "name": "Allow and don't ask again"},
             {"optionId": "cancel", "kind": "reject_once", "name": "Cancel"}]}
```

The tool name is only in the message text. The `toolCallId` is the call's
own id, so the tool and its arguments can come from the `tool_call` that came
before. A shell write asked with `approved` ("Yes, proceed"),
`approved-execpolicy-amendment` ("Yes, and don't ask again for commands that
start with …", `allow_always`) and `abort` (`reject_once`). Its source shows
that switching to its `auto` or `full-access` mode writes
`trust_level = "trusted"` for the folder into `~/.codex/config.toml`. That
wasn't tried, because it edits the user's global config.

**ACP 2.1.1.** Four modes, set per session with no file written
(`config.toml`'s hash was the same before and after):

| Mode id | Approval | Sandbox | Notebook writes |
|---|---|---|---|
| `read-only` | on-request, user | read-only | ask the user |
| `workspace-write` | on-request, user | session folder | ask the user |
| `agent` (default, "Auto review") | on-request, AI reviewer | session folder | Codex's reviewer decides |
| `agent-full-access` | never | none | never ask |

In `agent`, the default, nothing reached the client. Each notebook write
went to Codex's own AI reviewer, shown as a "Guardian Review" tool call
(`kind: "think"`), which approved it:
`"Status: Approved\nAction: MCP new_notebook on notebook\nRisk: low"`.

In `workspace-write`, notebook writes asked, and reads didn't:

```json
{"toolCall": {"toolCallId": "exec-ad7c0cc5-…", "kind": "execute", "status": "pending"},
 "_meta": {"is_mcp_tool_approval": true},
 "options": [{"optionId": "allow_once", "kind": "allow_once", "name": "Allow"},
             {"optionId": "allow_session", "kind": "allow_always", "name": "Allow for this session"},
             {"optionId": "allow_always", "kind": "allow_always", "name": "Always allow"},
             {"optionId": "cancel", "kind": "reject_once", "name": "Cancel"}]}
```

There is no title and no `rawInput`, only the id of the `tool_call` that
came just before. A shell write in `read-only` asked with title "Run
command", `rawInput {command, cwd}`, and options `allow_once`,
`accept_execpolicy_amendment` (`allow_always`) and `cancel`.

Which options save a rule outside the session: "Always allow" for an MCP
tool and the command-prefix option for shell. Codex saves those itself, in
the user's Codex config or rules (the adapter passes the choice on). Neither
was chosen. Endeavor should only ever answer allow-once, as its permission
rules already say, and offer no "In this folder" for Codex.

Hooks weren't tested.

### 5. A run the runtime holds

With the session's policy at "ask", the runtime held `execute_cell` and
showed it in `asks`, as designed. Codex then didn't wait for the answer.

**Zed 0.16.0.** Its MCP client gave up after 120 seconds, although the
runtime sent a progress event every 15 seconds:

```json
{"toolCallId": "call_l57CglhA…", "status": "failed",
 "rawOutput": "tool call error: tool call failed for `notebook/execute_cell`\n\nCaused by:\n    timed out awaiting tools/call after 120s"}
```

It sent no MCP cancel and kept the connection open, so the ask stayed held.
The model read the cell and called `execute_cell` again, which made a second
ask. The test allowed the first ask at 150 seconds and the cell ran, after
Codex had already reported it failed. The second ask stayed until the
adapter was killed. The adapter gives client MCP servers no
`tool_timeout_sec`, so the 120 seconds can't be raised per session.

**ACP 2.1.1.** Codex 0.159 calls MCP tools from a script tool (`exec`):
the model writes `await tools.mcp__notebook__execute_cell({…})`. A script
that runs longer than 30 seconds hands control back to the model
(`"Script running with cell ID 6\nWall time 31.0 seconds"`). The model then
polls with `wait` (10 seconds each). After about two minutes it ended the turn
with `end_turn` and said "execute_cell is still pending, so the runtime result
isn't confirmed yet." The ACP tool call stayed `in_progress`, with no
timeout and no failure, and the ask stayed held.

**Cancel.** `session/cancel` ended the turn with `stopReason: "cancelled"`.
Codex sent no MCP cancel and kept the connection open, so the ask stayed
held. Allowed 5 seconds later, the cell ran and Codex sent the call's
`completed` update with the real result after the turn had ended. Endeavor
already denies held asks when the user stops a turn. A turn that ends on its
own while a run waits is new: see "Changes", item 7.

### 6. Skills and instructions

Without `X-Endeavor-Skills`, both models called `notebook_guide` first in
every fresh session that did notebook work, unprompted. Asked afterwards,
the ACP 2.1.1 model quoted the runtime's MCP `instructions` verbatim, so they
reach the model.

Codex also loads the user's own setup, and neither adapter can turn that
off per session. The ACP 2.1.1 model listed these skills: `imagegen`,
`openai-docs`, `skill-creator`, `skill-installer`, plus plugin skills from
`plugin-management`, `sites` and `work-pets`. It reads AGENTS.md files the
usual Codex way; the session folder had none. ACP 2.1.1 sends
`developer_instructions: null` in every session, so there is no
per-session instruction channel. `CODEX_CONFIG` applies to every session of
the adapter process; whether its `developer_instructions` key reaches the
model wasn't tested.

### 7. Modes, models and effort

`session/new` config options:

```json
// ACP 2.1.1
{"id": "mode", "currentValue": "agent", "options": ["read-only", "workspace-write", "agent", "agent-full-access"]}
{"id": "collaboration_mode", "currentValue": "default", "options": ["default", "plan"]}
{"id": "model", "currentValue": "gpt-6.1-sol", "options": ["gpt-6.1-sol", "gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5"]}
{"id": "reasoning_effort", "currentValue": "low", "options": ["low", "medium", "high", "xhigh", "max", "ultra"]}
{"id": "fast-mode", "currentValue": "off", "options": ["off", "on"]}

// Zed 0.16.0
{"id": "mode", "currentValue": "read-only", "options": ["read-only", "auto", "full-access"]}
{"id": "model", "currentValue": "gpt-5.5", "options": ["gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex", "gpt-5.2"]}
{"id": "reasoning_effort", "currentValue": "medium", "options": ["low", "medium", "high", "xhigh"]}
```

ACP 2.1.1 also returns ACP `models`, one entry per model and effort, such as
`gpt-6.1-sol[medium]`. Setting `model`, `reasoning_effort`, `mode` and
`fast-mode` on a session (with the CLI's own `codex`) changed only that
session; `config.toml` didn't change. So Endeavor can re-apply a saved model
and effort to each new Codex session, as it does for Claude.

**Plan mode** (ACP 2.1.1). `collaboration_mode: plan`. The plan arrived as
message text, then as a permission request:

```json
{"toolCall": {"toolCallId": "plan-review:01a10908-…-plan", "kind": "switch_mode", "status": "pending",
   "title": "Implement this plan?", "rawInput": {"plan": "1. Open `notebook_tool_test.jl` …\n2. …\n3. …\n"}},
 "options": [{"optionId": "implement_plan", "kind": "allow_once", "name": "Yes, implement this plan"},
             {"optionId": "revise_plan", "kind": "reject_once", "name": "No, and tell Codex what to do differently"}]}
```

Declining ended the turn with `rawOutput: "User kept the session in plan
mode."` In plan mode Codex read files with its own tools and made no
notebook call. Zed 0.16.0 has no plan mode.

### 8. Turns

- **Streaming.** `agent_message_chunk` in both.
- **Thinking.** ACP 2.1.1 streams reasoning summaries as
  `agent_thought_chunk`. Zed 0.16.0 sent none.
- **Usage.** Both send `usage_update {used, size}` (size 258,400). ACP 2.1.1
  also puts token counts in the `session/prompt` reply.
- **Stop.** `cancelled` (ACP 2.1.1; not tried with Zed 0.16.0).
- **Steering.** ACP 2.1.1 takes `_session/steering`, the method Endeavor's
  Send now already uses, and answers `{"outcome": "injected"}`. The model
  answered the steered message in the same turn. Zed 0.16.0 answers
  "Method not found".
- **Titles.** ACP 2.1.1 sends `session_info_update` with the first message as
  a title, then a generated one ("Test notebook tools"). It also sends
  `session_info_update` with only `_meta.codex.threadStatus` (`active`,
  `idle`, or `waitingOnApproval`).
- **Commands.** ACP 2.1.1 lists 16 commands, the user's skills among them
  (`$imagegen`, `$skill-creator`, …). Zed 0.16.0 lists 6.

### 9. Past sessions

Both have `session/list` (filtered by `cwd`, with titles) and
`session/load`. Both list every Codex session in the folder, whichever client
made it, because they share `~/.codex/sessions`.

**ACP 2.1.1 replays faithfully.** A message's text blocks come back as
separate `user_message_chunk`s (the "[Endeavor]" block and the user's words
stayed apart). Tool calls come back with the same title, `rawInput` and
`rawOutput` as live. A steered message comes back as a user message in the
middle of the turn. A call that never finished (the held run) is missing.

**Zed 0.16.0 doesn't.** It joins a message's blocks with nothing between
them (`"…one sentence.Use the notebook MCP tools…"`). A notebook call comes
back titled with the bare tool name (`"read_cell"`), with only its arguments
as `rawInput` (no server), and with `rawOutput` as a string
(`"Wall time: 0.0097 seconds\nOutput:\n[{\"type\":\"text\",…}]"`). Calls that
failed come back `completed`.

Images in replay weren't tested.

### 10. Leftovers

In `~/.codex`, during the runs:

- a rollout file per session that had a turn, in `sessions/2026/10/04/`
  (7 files), and a line each in `session_index.jsonl`;
- `shell_snapshots/<session>.*.sh` (Zed 0.16.0);
- a new `thread_history_1.sqlite`, and writes to `state_5`, `logs_2`,
  `goals_1`, `memories_1` and `queue_1` sqlite files;
- `models_cache.json`, `cache/`, `skills/.system/` and `plugins/cache/`
  rewritten at start.

`config.toml` and `auth.json` didn't change. Processes: Zed 0.16.0's
native binary outlives its wrapper (above). ACP 2.1.1 left nothing running.
A `codex app-server --managed-daemon` was already running before the test;
neither adapter used it. In the session folder, the test left the notebooks
and `notes.txt` from the shell test. `endeavor serve` unpacked its runtime to
`~/.cache/endeavor/serve/0.1.0-f339f9a16b2a9e56/`.

## Changes Endeavor needs for Codex

In order. Items 1 to 6 are [other-agents.md](other-agents.md)'s work items
5 to 10, as Codex needs them.

1. **The per-agent table (item 5).** Codex's row:
   - Adapter `@agentclientprotocol/codex-acp`, pinned like Claude's. It
     brings its own `codex`; pass `CODEX_PATH` only if Endeavor decides to
     follow the user's CLI.
   - Sign-in from `codex login status`, or from `_auth/status_update` once the
     adapter runs.
   - `INITIAL_AGENT_MODE=workspace-write`, and the mode set again per session
     (item 8 below).
   - The notebook MCP server without `X-Endeavor-Skills`.
   - Stop the adapter by its process group. The Zed package showed a wrapper
     that doesn't pass signals on; the ACP adapter's `codex app-server` is a
     child process too.
2. **Name the agent in the app (item 6).** "Codex".
3. **A per-agent saved model (item 7).** Re-applying is safe for Codex: it
   writes nothing global. Effort values differ from Claude's (`low` to
   `ultra`), and there is a `fast-mode` option.
4. **Replay (item 8).** ACP 2.1.1 keeps blocks apart, so Codex needs nothing
   here. Cursor still does.
5. **Agent choice and sign-in screen (item 9).** Codex's sign-in is
   `codex login` in a terminal, or `authenticate` with `chat-gpt`.
6. **Starting Codex when one of its sessions opens (item 10).** `session/list`
   returns sessions the user's own Codex made in the same folder.
   `Records::merge` already leaves those out.
7. **Runs that outlive the turn.** Codex stops waiting for a held run after
   about two minutes (120 seconds with the old engine) and ends the turn with
   the call still open. It never cancels the MCP request. Treat a turn that
   ends while one of its runs waits like Stop: deny the ask, and say on the
   card that Codex stopped waiting. Otherwise the run can happen after the
   turn, and Codex learns of it only as a late `tool_call_update`.
8. **Modes.** Map Endeavor's modes onto Codex's, never using
   `agent-full-access`:
   - Manual: `workspace-write`, runtime policy "ask" with `edits: true`.
   - Ask to run: `workspace-write`, runtime "ask".
   - Auto: `workspace-write`, runtime "auto", and Endeavor answers Codex's
     notebook-tool prompts allow-once itself. Codex's `agent` mode would
     let its AI reviewer decide instead, which can refuse a run the user
     wanted.
   - Plan: `collaboration_mode: plan`, runtime "plan".
9. **Codex's permission prompt.** For a request with only a `toolCallId`
   (`_meta.is_mcp_tool_approval`), take the tool and arguments from the
   matching `tool_call`. The Cursor path takes them from a
   `tool_call_update`, so check that it covers a `tool_call` too. Answer only
   `allow_once`; never `allow_always` or `accept_execpolicy_amendment`, which
   save rules in the user's Codex setup.
10. **Tool results.** Read `rawOutput.result.content` (ACP 2.1.1) in
    `celldiff::tool_json`. Then live calls need no runtime lookup, and
    reopened sessions show results.
11. **Plan approval.** Show the `switch_mode` request titled "Implement
    this plan?" as the plan card, with the plan from `rawInput.plan`.
    `implement_plan` approves; `revise_plan` keeps planning.
12. **Smaller things.**
    - Ignore `session_info_update` that carries only `_meta.codex.threadStatus`.
    - If `agent` mode is ever used, show or hide its "Guardian Review" calls
      (`kind: "think"`).
    - The personal-setup switch can't apply: Codex always loads the user's
      skills, plugins and AGENTS.md.

## Not found out

- **Signed out.** Signing out was off limits; the behaviour above is from
  the source.
- **Hooks.** Not tested.
- **Saved rules.** Where "Always allow" and the command-prefix option write.
  They weren't chosen, because they change the user's setup.
- **The app's own runtime.** The test used `endeavor serve`. Its MCP
  `instructions` say the user works without the app; the app's runtime says
  otherwise. Run cards, held edits and replay in the real app need a live
  test.
- **Images** in prompts and replay.
- **`CODEX_CONFIG`** keys such as `developer_instructions`, or a per-server
  `tool_timeout_sec` for the notebook server.
- **Zed 0.16.0 cancel.**
