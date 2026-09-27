# Remote sessions over SSH

Design for running notebooks on a remote machine (a lab server, a cloud VM, or
an HPC cluster) from the Endeavor app on macOS, Linux, or Windows. Plain
servers (the process launcher) are built on macOS; clusters (Slurm) and the
Linux and Windows clients are not yet. Settled work moves to
[roadmap.md](roadmap.md) once scheduled.

_Drafted 2026-09-26_

## Summary

The app and Claude stay on the user's computer. Julia (Pluto plus
EndeavorRuntime) runs on the remote machine and keeps running when the user
disconnects. The app reaches it through one plain `ssh` process whose
stdin/stdout carry all traffic to a small helper program on the remote. There
is no port forwarding, no SSH connection sharing, and one sign-in per connect.
The helper starts the runtime in one of two ways: as a detached process on a
plain server, or inside a Slurm job on a cluster.

## Decisions

- **One client per runtime.** Two computers attached to the same remote
  runtime at once is not supported. A new connection takes over and the old
  one is told it was replaced (a laptop that died without disconnecting must
  not lock the user out).
- **Notebooks keep running after a disconnect.** Pluto is slow to start a
  notebook, and users run long computations. This replaces the common manual
  setup of a Pluto server in tmux.
- **Clients on macOS, Linux, and Windows.** Remote hosts are Linux, and macOS
  where it comes up. A Windows remote host is out of scope.
- **Claude runs locally.** Credentials never leave the user's computer, and
  nothing Claude-related is installed on the remote.

## What carries over from the local design

Today the app starts `julia boot.jl <pluto_port> <mcp_port>` and talks to it
only through two loopback ports plus Julia's stdio (`src/runtime.rs`,
`runtime/boot.jl`). Most of that works unchanged when the ports lead to a
remote machine:

- The webview and Claude still connect to `127.0.0.1:<port>`.
- The bridge's `Host` check looks at the host name only, not the port
  (`Server.jl`, `_loopback_host`), so a local port that differs from the remote
  one is accepted.
- Pluto's `?secret=` and the bridge's bearer token still protect both ports
  from other users on a shared machine.
- Crash reopen, safe preview, and restart go through `/events` and
  `open_notebook`, not local files.

Both ends stay on loopback, so the security rule in
[pluto-agent-design-doc.md](pluto-agent-design-doc.md) §6 ("never let the
app's proxying extend beyond loopback") still holds. Its v1 non-goal "not
supporting remote Pluto servers" would be lifted.

What must change:

- **Lifetime.** Julia exits when its stdin closes (`boot.jl`). Remote runtimes
  must outlive the connection, so stopping becomes an explicit command.
- **Token delivery.** The token is passed in `ENDEAVOR_TOKEN`
  (`runtime.rs`), and plain `ssh` drops environment variables.
- **The `folder` stdin command** moves to the bridge's `/call` endpoint.
- **Ports** are chosen by the runtime (bind port 0), not by the app's
  `free_ports`, which only sees the local machine.
- **Paths** become host plus path (see Notebook identity).

## Architecture

```
app (local)                        remote login host / server        runtime host
───────────                        ──────────────────────────        ────────────
webview ─┐                                                            Pluto  :p1
Claude  ─┼─ local listener ── ssh stdio ── endeavor-remote helper ──── bridge :p2
app     ─┘  127.0.0.1:<port>   (one ssh)   (relays, attaches)         (loopback)
```

**Connect.** The app runs `ssh <host> <bootstrap>`, where the bootstrap is a
short `sh` script. If the matching helper version is already in
`~/.cache/endeavor/<version>/`, the script starts it. If not, it reads the
helper and `runtime/` as a tar stream on the same stdin, unpacks them, and
then starts it. Installing and connecting share one SSH session, so a Duo or
other two-factor prompt appears once.

**Helper.** `endeavor-remote` is a small static Rust binary built for Linux
x86_64 and aarch64 and for macOS. It is not written in Julia, because it runs
on every connect and Julia is slow to start, and it does not rely on `socat`
or Python being installed. It:

1. Says hello with the machine's name and home folder, and from then on
   answers file requests itself, without Julia: list a folder (folders and
   `.jl` files), find the Pluto notebooks under a folder, and the first cells
   of a notebook (`crates/wire`'s `files` and `notebooks`). The new-session
   screen browses a server with these before any runtime exists.
2. When the app sends `StartRuntime` (a session needs Julia), takes an
   exclusive lock on the runtime's state file (one client), then reads it. If
   the runtime is alive, it connects to its ports. If not, it starts one (see
   Launchers). Only this step takes a runtime over from another client, so
   browsing a server never does.
3. Multiplexes HTTP and SSE connections between the runtime and the SSH stdio
   channel.

A runtime that dies, fails to start, or is stopped by the app leaves the
helper connected, so starting it again needs no new SSH sign-in.

**Local listener.** The app listens on local loopback ports and feeds each
connection into the channel. The webview, the agent's MCP config
(`agent.rs`), and the `/events` watcher keep using `127.0.0.1` URLs. Each host
has its own listener for the whole launch, so a session's MCP URL (and the
token, kept in the host's state folder) survive reconnects and restarts. The
app keeps one connection per host (`src/connection.rs`): its status
(connecting, browsing, starting, ready, died, replaced, failed), its askpass,
and what it follows of the runtime. On quit, servers detach; their idle stop
(the server's own setting, else Settings') covers forgotten notebooks.

**Local sessions use the same path.** The app can run the helper as a child
process without SSH, so local and remote share one transport, with local as
the case that skips SSH. Whether local runtimes should also survive the app
quitting is a user setting (decided 2026-09-26): by default local notebooks
quit with the app; with the setting on, the local runtime keeps running and the
app reconnects to it on the next launch, as with a remote host. Either way,
notebooks stop after 48 hours idle (a setting), even while the app is open.

## Launchers

The state file records how the runtime was launched. Connect, reconnect, and
stop each switch on this one value instead of scattered checks.

```
~/.cache/endeavor/runtime.json   (mode 0600)
{ "launcher": "process", "node": "labbox3", "pid": 81234,
  "pluto_port": 40211, "mcp_port": 40212, "token": "…" }
{ "launcher": "slurm", "job": "4812731", "node": "n2cn0216",
  "pluto_port": 40211, "mcp_port": 40212, "token": "…" }
```

**Process (plain server).** The helper starts `boot.jl` with `setsid`/`nohup`.
The runtime runs until the user stops it from the app's per-host list, or
until a long idle timeout (days, with no notebooks open and nothing running)
so forgotten runtimes don't pile up on shared machines. The state file
records the node name. If the host name rotates between machines, the helper
reports that the runtime is on another node and does not start a second one.

**Slurm (cluster).** HPC centers do not want long-lived notebook servers on
login nodes (see Prior art). The helper on the login node only relays, which
is light and short-lived. The runtime starts with `sbatch` using settings the
user chooses per host. A good first version is letting users paste the
`salloc` line they already use, since novices rarely know partition names.

- The login-node helper reaches the runtime with an inner
  `ssh <compute node> endeavor-remote …` inside the cluster. There is still one
  outer sign-in, and the runtime stays on the compute node's loopback. Compute
  nodes usually accept SSH only while the user has a job there
  (`pam_slurm_adopt`), which holds here.
- Reconnect works from any login node, because `squeue` finds the job by ID.
- Persistence lasts until the job's time limit. The app shows "waiting for a
  node" while the job is queued and warns before the time limit, noting that
  Pluto has already saved the notebook file.
- Put the Julia depot on scratch and not in home. Home quotas are small, and
  Pluto's per-notebook package handling is very slow on some cluster
  filesystems.

If Slurm is present (`sinfo` exists) and the host isn't set to cluster mode,
the app warns before starting a process runtime on what is probably a login
node.

Build the process launcher first and Slurm second, but keep both in the state
file format from the start.

## Claude's tools on a remote session

Claude Code's built-in Bash, Read, and Write run on the user's computer in the
session's local working folder. With a remote notebook they would look at the
wrong machine. In remote sessions:

- Turn off the built-in Bash, Read, Write, Edit, MultiEdit, Glob, Grep and
  NotebookEdit (`disallowedTools` in the session's options).
- The runtime's MCP adds tools that run where it runs: `list_folder`,
  `read_file` and `run_shell`. It lists them only on MCP connections that
  carry `X-Endeavor-Host: <server name>`, which the app sends for server
  sessions, and refuses them otherwise. `run_shell` goes through the same
  execution gate as running cells; its card says "Run a command on
  <server>?" with the command and folder.
- The first message tells Claude which server and folder it works in.

The skills already route notebook work through MCP and tell the agent not to
scan the filesystem (`plugin/skills/pluto-session/SKILL.md`). This makes that
enforced rather than advisory. Claude Code's session history and CLAUDE.md
stay tied to a local folder: `hosts/<server id>/` in the app's data
directory, one per server. The app records each session's host and remote
folder in `sessions.json`.

## Notebook identity and files

- A notebook is identified by host plus path everywhere the app stores paths:
  `recent.json`, `sessions.json`, `notebooks.json`, crash reopen. Entries saved
  before servers were plain paths and read as This Mac's. The `is_dir` filter
  on recents only applies to This Mac's entries.
- The native folder picker can't browse a remote disk. The folder chip's
  Browse… opens an in-app browser instead (breadcrumbs, up, open, "Choose
  this folder"), fed by the helper's file requests, as are the notebook chip
  and the static preview.

## Secrets

- **Remote.** The runtime's token lives in the 0600 state file and in the
  runtime's memory. The helper runs as the user, reads it, and adds it to
  requests. The app does not store a long-lived secret, so there is no need
  for Keychain, Secret Service, or Windows Credential Manager.
- **Local.** The app's local listener keeps a per-launch token as today,
  because other users on the same computer can reach local loopback.
- **Pluto's secret** is fetched from the bridge after each connect instead of
  parsed from a `READY` line.

## SSH client

Use the system `ssh` binary (macOS, Linux, and Windows 10 or later all ship
one). The user's `~/.ssh/config`, keys, agent, and ProxyJump then work as
they do in a terminal. Because the design uses one plain stdio session, it
does not need ControlMaster, which Windows OpenSSH lacks. On Windows, pass
`-o ControlMaster=no -o ControlPath=none` so a user's config written for
macOS or Linux doesn't break the connection.

Password and two-factor prompts have no terminal to appear in. On macOS and
Linux, use `SSH_ASKPASS` with `SSH_ASKPASS_REQUIRE=force` (OpenSSH 8.4 or
later) to show them in the app. Windows askpass support has been unreliable
across versions. If it fails, either drive `ssh` through a pseudo-terminal
and relay the prompt, or embed `russh`. `russh` gives full control over
prompts but has to reimplement config parsing, the agent, and ProxyJump, and
has no Kerberos sign-in, which some clusters use.

## Rejected alternatives

- **An sshfs-style mount** so Claude's local tools see remote files. On macOS
  it needs macFUSE or FUSE-T (an extra install and a permissions prompt). It
  stalls rather than fails when the connection drops. Bash would still run
  locally against network files. Every file would have two paths (Pluto's
  remote one and the mount's local one). And raw writes to the notebook `.jl`
  would be overwritten by Pluto's next save. It also does nothing for
  persistence.
- **Port forwarding with ControlMaster** (`ssh -O forward` after the runtime
  reports its ports). Not available on Windows OpenSSH.
- **Claude on the remote** (ACP over SSH stdio). Bash and CLAUDE.md would
  work where the data is, but Node, the adapter, the `endeavor` binary for the
  pre-tool hook, and a Claude sign-in would all sit on what is often a shared
  university server.
- **Pluto server local, notebook workers remote.** Pluto's Malt workers can't
  run on another machine, so the server and the notebook must share a host.

## Open questions

- Does current Windows 11 OpenSSH honor `SSH_ASKPASS_REQUIRE=force` with a
  Duo account? Test before choosing between system `ssh` and `russh`.
- Slurm settings: pasted `salloc` line, a small form, or both?
- Idle timeout length for process runtimes, and where the user sees and
  changes it.
- Should local runtimes also survive the app quitting?

## Prior art

Research from 2026-09-26, gathered by subagents. Not every page was opened
directly.

- People run Pluto on clusters by hand: `Pluto.run(launch_browser=false)` on
  the remote plus an SSH tunnel, with a second tunnel to the compute node.
  There's a handful of write-ups and no packaged setup.
  - Pluto FAQ: https://plutojl.org/en/docs/faq/
  - Paderborn PC2 guide:
    https://upb-pc2.atlassian.net/wiki/spaces/PC2DOK/pages/1903837/Pluto.jl+on+Noctua
  - Julia Discourse:
    https://discourse.julialang.org/t/how-to-run-julia-notebooks-on-cluster-computers/75277
  - Slow notebook creation on cluster filesystems:
    https://github.com/JuliaPluto/Pluto.jl/issues/1341
  - Malt has no remote workers: https://github.com/JuliaPluto/Malt.jl
  - `jupyter-pluto-proxy`, archived 2024:
    https://github.com/IllumiChat/jupyter-pluto-proxy
- HPC centers put notebook and editor servers on compute nodes, through Open
  OnDemand or an interactive job plus a tunnel.
  - NIH Biowulf: https://hpc.nih.gov/apps/jupyter.html
  - NCAR on VS Code and login-node abuse:
    https://ncar-hpc-docs.readthedocs.io/en/latest/environment-and-software/vscode/
  - NERSC, VS Code through ProxyJump to a compute node:
    https://docs.nersc.gov/connect/vscode/
  - `pam_slurm_adopt`: https://slurm.schedmd.com/pam_slurm_adopt.html
- Login nodes limit and kill long processes, and often rotate users across
  many machines.
  - Utah CHPC Arbiter2:
    https://www.chpc.utah.edu/documentation/policies/2.1GeneralHPCClusterPolicies.php
  - Virginia Tech ARC: https://www.docs.arc.vt.edu/usage/abuse.html
  - ETH Euler, 50 login nodes: https://docs.hpc.ethz.ch/hardware/login_nodes/
  - UF, tmux sessions lost on reboot:
    https://docs.rc.ufl.edu/access/persistent_sessions/
- VS Code Remote-SSH installs a server under `~/.vscode-server` and stops it
  when the client leaves: https://code.visualstudio.com/docs/remote/faq
- Windows OpenSSH has no ControlMaster
  (https://github.com/PowerShell/Win32-OpenSSH/issues/1328), and askpass
  broke between versions
  (https://github.com/PowerShell/Win32-OpenSSH/issues/2115).
