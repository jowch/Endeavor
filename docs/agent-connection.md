# How the app runs an agent

The app's side of ACP: how each agent is installed, started, connected, and
translated into the one shape the rest of the app knows (Claude's). The
per-agent findings behind it are in [codex-agent.md](codex-agent.md),
[antigravity-agent.md](antigravity-agent.md) and
[other-agents.md](other-agents.md).

## The connection

Each agent gets one ACP connection, run on its own thread (`agent::run`). One
connection hosts any number of sessions, and each session has at most one
running turn. The UI talks to it over two channels: `Command`s in and
`AgentEvent`s out. Message queues live in the UI, so they stay editable and
the same for every agent; the connection only runs turns. Commands sent
before the connection is up wait in its channel.

- **A permission request goes to the UI with its responder, and the loop
  returns at once.** Awaiting the user's click inside the loop would stall
  the connection's dispatch, and with it every other session's turn.
- The loop keeps taking commands while turns run, so Cancel and SendNow can
  act on them. It also watches the adapter's process, so a gone adapter is
  noticed at once, not at the next command.
- Sessions are opened with `NoticeCapabilities`. Without them the agent
  writes its notices ("Auto mode unavailable: …") into its reply.
- The user's setup is read again for each new session, so a Settings change
  applies to the next one.
- The UI sends `Prompt` only while a session is idle. One that arrives
  mid-turn is handed back to the queue, not dropped.
- `SendNow` joins the running turn (`_session/steering`) with
  `promptRequired` set: if the turn has already ended, the adapter hands the
  message back instead of starting a turn the app doesn't track. An agent
  that can't steer gets its turn stopped, and the message leads the queue
  after it ends.
- A config change's reply carries the options as they were then. Its mode
  can be older than a mode update already applied, since a reply can arrive
  after a notification the agent sent later, so only updates change the mode.
- A failed listing of past sessions is an error, not an empty folder.
  `complete` means the agent had no further page, so a session left out isn't
  there.
- claude-agent-acp marks the failures it can tell apart (Claude's API failing
  after Claude Code's retries, a usage limit) with an `errorKind`, and gives
  the error's text as the message. For an API failure the agent has also
  written the error as its reply.

## What the app relies on about each agent

`AgentFacts` holds what can differ between agents.

- `install`: an npm package run on the app's own Node (the resources folder
  pins it with a package.json and package-lock.json, installed with `npm ci`
  into `<installed>-<version>`), or a program in a pinned zip, where `zip` is
  None on a platform it isn't pinned for, so the agent shows as "Not available
  yet". Installs go beside the target and are renamed into place, so a
  partial install is never used. A new app version can pin a new adapter,
  which installs when the agent next starts.
- `env`: set on the adapter's process. Claude's has
  `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT=0`, because a run waits in the runtime
  for the user's answer for as long as that takes. Codex's has `CODEX_CONFIG`,
  which overrides the user's own setup for every session of the adapter: the
  user's own install of Endeavor's plugin is turned off there, as in Claude's
  sessions. The user's own `CODEX_CONFIG` keys stay (a gateway's
  `model_provider`, say), with Endeavor's merged over them.
- `private_temp`: the process gets a temp folder of its own, emptied before
  each start. Antigravity's server unpacks itself into the temp folder on
  every start and leaves that behind when it's ended, and a killed server
  can't clean up after itself.
- `plugin`: the agent loads Endeavor's skills as a Claude Code plugin, so the
  runtime leaves out its own guide to them. An agent without it gets
  `session_intro` ahead of its first message instead. Codex's says what the
  notebook is and where its tools are; without it Codex made a Jupyter
  notebook with its shell when asked to make a new notebook.
- `folder_rules`: where the agent keeps "don't ask again" rules inside the
  session folder (Claude Code: `.claude/settings.local.json`, since the app's
  session options load `local` settings). Endeavor then offers "In this
  folder" and lists the rules. None for agents whose lasting rules change the
  user's own config (Codex, Cursor).
- `on_servers`: its sessions can run on a server. The agent always runs on
  this computer, so on a server its own file and shell tools must be off.
  Codex's and Antigravity's can't be turned off per session, and would act on
  this computer.
- `asks_every_write`: it asks before every notebook write whatever
  Endeavor's mode, with no mode that leaves this to Endeavor. The app answers
  those prompts allow-once and leaves the decision to the runtime's gate,
  which holds what the session's mode asks about.
- `on_demand`: it starts when one of its sessions first needs it, not at
  launch, so its sessions say when they wait for it to install, connect or be
  signed in. Claude starts at launch and has its own sign-in screens.
- `sign_in`: `claude auth` (signin.rs), `codex login status` and
  `codex-acp login` (codex.rs), or ACP's own `authenticate`, checked by the
  agent's sign-in file (antigravity.rs). `codex-acp login` runs `CODEX_PATH`,
  else a `codex` on PATH, which a new user hasn't got, so the app points it at
  the `codex` the adapter bundles, in its own process group so Cancel stops it
  and what it started.

## Claude Code's session options

`session_options` builds them in layers. Endeavor's own plugin (its skills
and guards) is always loaded, and so are the project's settings and
CLAUDE.md from the working folder. The user's own setup (user settings and
CLAUDE.md, skills, plugins, hooks, MCP servers) loads unless they turned it
off in Settings; off, strict MCP config also leaves out the project's
`.mcp.json`. The agent and the user's MCP servers always run on this
computer, as with the plugin in Claude's desktop app.

- `settings` is Claude Code's flag settings, which outrank user and project
  settings. Sending it replaces what the adapter would have put there from a
  `CLAUDE_MODEL_CONFIG` variable (a gateway's model list), which the app's
  sessions don't use.
- The user's own Endeavor (the `endeavor@endeavor` plugin from EndeavorMCP's
  marketplace, or a server added by hand as `endeavor`) is off: it would add a
  second set of notebook tools outside the app's cards. The app's own plugin
  is `endeavor@inline`.
- The skills point at reference files beside them, outside the working
  folder, which Claude Code would ask about, so they get a Read rule. When
  the plugin folder's path goes through a symlink, Claude Code reads without
  asking only with a rule for each form of the path, so both go in.
- A permission rule writes an absolute path as `//` and the path with forward
  slashes. Claude Code compares a Windows path in POSIX form, so `C:\Users\me`
  becomes `//c/Users/me`. Rules are gitignore-style patterns, so a folder name
  with `[`, `*` or `?` would still ask, and a UNC path isn't handled; the
  app's folder is never on one in practice.
- `READ_ONLY_TOOLS` run without asking in every mode, as Claude Code's own
  reads do in its default mode: Claude Code prompts for any MCP tool without
  an allow rule, `readOnlyHint` or not (as of 2.1.280). Every other notebook
  and host tool still asks in Manual. `ASKS_ITSELF` are the tools the app asks
  about itself (`restart_ask`), so Claude Code's own prompt would be a second
  question.
- On a server, `LOCAL_TOOLS` are off; the runtime's host tools stand in for
  them.
- Each session's tool calls carry its key, so the runtime applies its policy,
  and say they are the app's, so a runtime the plugin started and shares
  answers them as the app's.

## Dialects

Claude's sessions are the shape the app is written for. Another agent's are
translated at the ACP boundary (`codex::Dialect`, `antigravity::Dialect`), so
the rest of the app has no per-agent branches. Both keep the agent in one
mode, the one where it asks before notebook writes (Codex's
`workspace-write`, Antigravity's `default`), and make Endeavor's modes out of
the runtime's gate: Manual and Ask to run differ only in what the runtime
holds. The app sees them as the mode ids it knows: `default` (Manual), `auto`
(Ask to run and Auto), and `plan` where the agent has one (Codex's plan
collaboration mode). Leaving Codex's plan by approving it goes on in Ask to
run, as Claude's Start does.

Antigravity's permission requests lose "Allow Always": Antigravity keeps that
answer as a rule of its own, which the app can't show or remove, and which
would hold in Manual too.

## Ending the process tree on Windows

On Unix the ACP library starts the adapter as its own process group and kills
the group. Windows has no groups, and the library ends only the process it
started, so an adapter's own children (Claude Code, `codex app-server`,
Antigravity's second process) were left running. There the adapter runs under
the app instead: `endeavor --agent-job <app pid> <program> <args…>` puts
itself in a job that ends every process in it when its last handle closes,
then starts the adapter inside it (`agent_job.rs`).

- The job handle is held until the process exits; closing it sooner would end
  this process before it could exit with the adapter's code. Nothing inherits
  the handle, so it closes however the process exits.
- A process that asks to leave the job may (`BREAKAWAY_OK`), as one that
  leaves its process group may on Unix.
- Windows 8 and later nest jobs, so this works inside the app's own job.
- Without a job the agent still works; it only leaves its processes behind
  when it's stopped.
- It also waits on the app, so the agent ends when the app is gone.
