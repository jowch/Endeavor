# Remote sessions over SSH

Design for running notebooks on a remote machine (a lab server, a cloud VM, or
an HPC cluster) from the Endeavor app on macOS, Linux, or Windows. Plain
servers (the process launcher) and clusters (Slurm) are built. A Linux client
reaches servers too ([linux.md](linux.md)); a Windows client can't connect
yet ([windows.md](windows.md)).

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

## Loopback on both ends

The webview and Claude connect to `127.0.0.1:<port>` whether the runtime is
local or remote:

- The runtime has one port ([one-port.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/one-port.md)): the core answers
  `/mcp` and `/endeavor/…` itself and passes every other path through to
  Pluto's private port.
- The core's `Host` check looks at the host name only, not the port, so a
  local port that differs from the remote one is accepted.
- The runtime's token protects its port from other users on a shared
  machine: as a bearer header everywhere, or for Pluto's page as a cookie the
  web view gets from a `?token=` link. Pluto's own secret never leaves the core.
- Crash reopen, safe preview, and restart go through `/endeavor/events` and
  `open_notebook`, not local files.

Both ends stay on loopback, so the security rule in
[pluto-agent-design-doc.md](pluto-agent-design-doc.md) §6 ("never let the
app's proxying extend beyond loopback") still holds. The runtime chooses its
own port (bind port 0), since the app only sees the local machine; the
helper makes the bridge token on the host, so it never has to travel in
ssh's environment, which plain `ssh` drops; and the runtime outlives the
connection, so stopping it is an explicit command.

## Architecture

```
app (local)                        remote login host / server        runtime host
───────────                        ──────────────────────────        ────────────
webview ─┐                                                            core :port ── Pluto (private)
Claude  ─┼─ local listener ── ssh stdio ── endeavor helper ────           ── Julia's bridge (private)
app     ─┘  127.0.0.1:<port>   (one ssh)   (relays, attaches)         (loopback)
```

**Connect.** The app runs `ssh <host> <bootstrap>`, where the bootstrap is a
short `sh` script. If the matching helper version is already in
`~/.cache/endeavor/<version>/`, the script starts it. If not, it reads the
helper and `runtime/` as a tar stream on the same stdin, unpacks them, and
then starts it. Installing and connecting share one SSH session, so a Duo or
other two-factor prompt appears once.

**Helper.** `endeavor` is a small static Rust binary built for Linux
x86_64 and aarch64 and for macOS. It is not written in Julia, because it runs
on every connect and Julia is slow to start, and it does not rely on `socat`
or Python being installed. On the Mac itself the app runs its own binary as
the helper (`endeavor --helper connect …`; the helper is a library the app
links), so the local helper can't be missing or from another build; the
separate `endeavor` binary (on macOS, the app's `endeavor-helper` target) is
what servers are sent. It:

1. Says hello with the machine's name and home folder, and from then on
   answers file requests itself, without Julia: list a folder (folders and
   `.jl` files), find the Pluto notebooks under a folder, and the first cells
   of a notebook (`crates/wire`'s `files` and `notebooks`). The new-session
   screen browses a server with these before any runtime exists. The one
   write is a file attached in the chat: `Place` picks its name in the
   session folder's `data/` (or finds the same contents there by size and
   SHA-256), and `Write` sends it in 1 MB pieces to a hidden `.part` file,
   renamed into place after the last piece. Paths stay inside the session's
   folder, links resolved; parts the app never finished are deleted when it
   goes. Hello's `uploads` tells a helper with these from an older one.
2. When the app sends `StartRuntime` (a session needs Julia), takes an
   exclusive lock on the runtime's state file (one client), then reads it. If
   the runtime is alive, it connects to its port. If not, it starts one (see
   Launchers). Only this step takes a runtime over from another client, so
   browsing a server never does.
3. Multiplexes connections (HTTP, event streams, Pluto's WebSocket) between
   the runtime's port and the SSH stdio channel.

A runtime that dies, fails to start, or is stopped by the app leaves the
helper connected, so starting it again needs no new SSH sign-in.

**Local listener.** The app listens on one local loopback port per host and
feeds each connection into the channel. The webview (`/?token=…`), the
agent's MCP config (`/mcp`, `agent.rs`), and the `/endeavor/events` watcher
use `127.0.0.1` URLs on it. Each host
has its own listener for the whole launch, so a session's MCP URL (and the
token, kept in the host's state folder) survive reconnects and restarts. Each
server is one EndeavorMCP `client::Session` (`src/remote.rs`): it signs in,
installs the helper, starts or attaches to Julia, and gets a dropped
connection back by itself, for up to 10 minutes, starting Julia again if it
ran. ssh's prompts come through its `Asker`. Its listener refuses code runs
on a runtime too old to ask before one, in the app's words
(`client::Messages::no_run_gate`, `older_runtime`); This Mac's listener
(`src/runtime.rs`) does the same. The app keeps one connection per host
(`src/connection.rs`): its status (connecting, browsing, starting, ready,
died, replaced, failed) and what it follows of the runtime. A server's comes
from its session's state, read as it changes (`changes`). A drop that gets
back to the same Julia within 2 s isn't shown at all (`LOST_GRACE`): the
session usually has it back in well under a second, and Pluto's page
reconnects by itself. A longer drop shows "Can't reach" with the page kept
read-only, and once the session is back on the same Julia (same pid and node),
however long it took, the page and its notebooks carry on: nothing is reopened
or reloaded, since that would cut across the page's own reconnect. The
app's Stop, Cancel and Restart force the stop, so a start the session resumed
by itself after a drop is cancelled too. The session is kept for the whole launch unless the
server's connection settings change, since its listener's port is in the MCP
URL of the agents there; one that gave up is asked to try again. On quit, servers detach; their idle stop
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
stop each switch on this one value instead of scattered checks. Each host
entry has its own state folder: `~/.cache/endeavor/state/` for a server,
`~/.cache/endeavor/cluster-<id>/` for a cluster, so one machine can be both a
server entry and a cluster entry without the two sharing a runtime. The app
sends the launcher and the folder's name in the bootstrap's preamble.

```
<state folder>/runtime.json   (mode 0600)
{ "launcher": "process", "node": "lab-server", "pid": 81234, "job": "",
  "started": null, "port": 40211, "token": "…" }
{ "launcher": "slurm", "job": "4812731", "node": "node042", "pid": 5120,
  "started": null, "port": 40211, "token": "…" }
```

**Process (plain server).** The helper starts the runtime with `setsid`/`nohup`:
`endeavor core` ([runtime-core.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/runtime-core.md); on This Mac, where
the app is the helper, `endeavor --helper core`), which starts
`julia boot.jl` as its child in the same process group, serves the runtime's
one port and writes `runtime.json` (its own pid and that port; Pluto's port
and secret stay between the core and Julia, in `julia.json`). A runtime from
a build before one port per runtime (`runtime.json` without `port`) is not
attached to: the helper says Julia was started by an older Endeavor and asks
for a restart, and Stop ends it by its pid.
The runtime runs until the user stops it from the app's per-host list. The
app's server runtimes don't exit when idle (`exit_idle` off, as before the
move to `client::Session`); each notebook still stops after the server's idle
stop, so a forgotten runtime holds no running notebooks. The state file
records the node name. If the host name rotates between machines, the helper
reports that the runtime is on another node and does not start a second one.

**Slurm (cluster).** HPC centers do not want long-lived notebook servers on
login nodes (see Prior art). The helper on the login node
(`connect --launcher slurm`, `crates/endeavor-mcp/src/slurm.rs`) only
submits, waits and relays; Julia runs in a batch job.

- **Settings.** A cluster entry has an SSH host, how to get Julia, an optional
  account, default resources (partition, CPUs, memory, time limit), where to
  keep Julia packages, and the idle stop. Test connection checks that `sinfo`
  and `sbatch` exist and lists the partitions with each one's time limit and
  node size, which fill the partition select. The new-session screen has a
  resources chip ("8 CPUs · 32 GB · 8 h") whose popover sets one session's
  job: presets (Small 2 CPUs · 8 GB · 2 h, Medium 8 · 32 · 8 h, Large 32 ·
  128 · 24 h), partition, CPUs, memory and time limit, all kept within the
  partition's limits (a line under the steppers names any limit a value sits
  at), or a pasted `salloc` line. From a pasted line Endeavor
  reads `-p/--partition`, `-c/--cpus-per-task`, `--mem`, `-t/--time`,
  `-A/--account` and `--gres`, drops interactive-only flags (`--pty`), and
  passes the other flags to `sbatch` as they are. Each session's resources are
  saved with it (`resources.json`), for the job that runs it after a reopen.
  Whatever was saved, the job asks for no more than its partition's largest
  node has (`Cluster::job`), so a cluster added without Test connection, still
  at Medium, fits once its partitions are known.
- **Submit.** On `StartRuntime { job }` the helper finds Julia on the login
  node (the shared filesystem makes it the compute node's too), writes
  `job.sh` and runs `sbatch --parsable --job-name=endeavor` with the
  resources, the account and `--output` to the state folder's `runtime.log`.
  The script is `endeavor node-start`, which becomes the core on the
  compute node; it starts Julia on private ports free there and writes `runtime.json`
  with the node and `SLURM_JOB_ID`. `job.json` records the
  job until its runtime is up, so a reconnect waits for the same job instead
  of submitting another. Packages go to `$SCRATCH/endeavor/depot` when the
  cluster sets `$SCRATCH` (home quotas are small, and Pluto's per-notebook
  package handling is slow on some cluster filesystems), else
  `~/.cache/endeavor/depot`, unless the cluster entry names a folder.
- **Queued.** The helper polls `squeue` every 2 seconds and tells the app the
  job's state and reason. The starting pane shows "Submitted job N (8 CPUs ·
  32 GB · 8 h)", then "Waiting for a node" with the time waited and the
  reason in plain words ("other jobs are ahead in the queue", "waiting for a
  node with enough free CPUs and memory"), and a Cancel link, which runs
  `scancel`. Leaving the app leaves the job queued.
- **Relay.** Once `runtime.json` names the job, the login helper starts
  `endeavor relay` on the job's node and passes the app's streams
  through its stdin and stdout; the runtime stays on the node's loopback.
  It tries `srun --jobid=<job> --overlap --unbuffered` first: it needs no SSH
  between nodes and works wherever the user can run job steps (Slurm 20.11
  and later). If that fails (older Slurm, steps not allowed), it uses
  `ssh <node>`, which works on clusters with `pam_slurm_adopt`. The starting
  log says which route it took and why. Without `--unbuffered`, `srun` holds
  a step's output until a newline, which stalls the binary frames.
- **Reconnect** works from any login node: `squeue` finds the job by the ID in
  `runtime.json`. The state folder's lock records the host with the pid, since
  a pid means nothing on another login node.
- **Time limit.** The notebook header shows the job's end ("Job ends 18:40",
  from `squeue`'s time left) and turns orange in the last 15 minutes; then
  each session on the cluster gets a notice in the chat that the notebook file
  is already saved and Start Julia runs it in a new job. The idle stop still
  applies inside the job.
- **Stop and end.** Stop asks the runtime to shut down through the relay,
  then runs `scancel`. When the job ends on its own, the app says why, from
  `sacct` (or `squeue`, or the job's log): "Julia on lab-cluster stopped. Its
  Slurm job reached its time limit." Likewise for preemption, cancellation,
  running out of memory, a failed node.

If Slurm is present (`sinfo` exists) on a host set up as a plain server,
starting a session there asks first: "This looks like a cluster's login node.
Clusters stop long-running programs here. Add it as a cluster instead?", with
Add as cluster (a cluster entry from the same SSH host) or Start anyway (once
per launch).

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
  <server>?" with the command and folder. It runs in the session's folder
  unless Claude gives another; the app tells the runtime each session's
  folder when the host is ready.
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
- **Pluto's secret** stays in the core, which adds it to what it passes on
  to Pluto ([one-port.md](https://github.com/jowch/EndeavorMCP/blob/main/docs/one-port.md)). The app and the page never see it.

## SSH client

Use the system `ssh` binary (macOS, Linux, and Windows 10 or later all ship
one). The user's `~/.ssh/config`, keys, agent, and ProxyJump then work as
they do in a terminal. Because the design uses one plain stdio session, it
does not need ControlMaster, which Windows OpenSSH lacks. On Windows, pass
`-o ControlMaster=no -o ControlPath=none` so a user's config written for
macOS or Linux doesn't break the connection.

Password and two-factor prompts have no terminal to appear in. On macOS and
Linux, use `SSH_ASKPASS` with `SSH_ASKPASS_REQUIRE=force` (OpenSSH 8.4 or
later) to show them in the app. On Windows this works too: Windows 10's
OpenSSH shows a password, a key's passphrase and a host-key question through
the app with `SSH_ASKPASS_REQUIRE=force` (tried against a test server). A
two-factor prompt such as Duo hasn't been tried on Windows. If it fails there,
either drive `ssh` through a pseudo-terminal and relay the prompt, or embed
`russh`. `russh` gives full control over
prompts but has to reimplement config parsing, the agent, and ProxyJump, and
has no Kerberos sign-in, which some clusters use.

While its askpass waits for the user, ssh reads nothing from the server, so
it can't see the server give up on the sign-in (sshd's `LoginGraceTime`). The
app, run as the askpass, watches ssh's TCP connection (`src/askpass_watch.rs`:
`/proc` on Linux, `lsof` on macOS, the TCP table on Windows). Once the server
has closed it, the askpass exits without an answer, ssh fails, and the app
takes the prompt down and says the server stopped waiting. Through a
ProxyJump or ProxyCommand ssh holds no TCP socket of its own, so nothing is
watched and the prompt stays up as before.

On a reconnect, what happens next depends on ssh's own error. After a
password prompt, ssh says the connection ended, which the reconnect retries,
so the next try asks again. After a key's passphrase prompt, ssh says
"Permission denied", which isn't retried, so the line stops and waits for
Reconnect (EndeavorMCP #64).

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
  work where the data is, but Node, the adapter and a Claude sign-in would
  all sit on what is often a shared university server.
- **Pluto server local, notebook workers remote.** Pluto's Malt workers can't
  run on another machine, so the server and the notebook must share a host.

## Open questions

- Does Windows OpenSSH show a Duo or other two-factor prompt through the
  askpass? Passwords, passphrases and host keys work on Windows 10; test a
  two-factor account before choosing between system `ssh` and `russh`.

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
