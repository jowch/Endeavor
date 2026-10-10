---
title: Servers
description: Run your notebook on a lab server or another computer you reach over SSH.
sidebar:
  order: 70
---

A session can run its notebook on another computer, such as a lab server
with more memory or with your data on it. Endeavor connects to it with SSH,
the standard way to sign in to another computer from a terminal.

Only Julia and the notebook run on the server. Endeavor and Claude stay on
your Mac, and nothing Claude-related is installed on the server.

To run notebooks as jobs on a shared cluster, see [Clusters](./clusters.md)
instead.

## What you need

- You can already sign in to the server from Terminal on your Mac, for
  example with `ssh lab-server`. Endeavor uses the same `ssh` program, so
  your `~/.ssh/config`, your keys and any jump hosts work the same way.
- The server runs Linux (on Intel or ARM processors), or macOS on the same
  kind of processor as your Mac.

If the server asks for a password or a two-factor code, Endeavor shows the
prompt in a dialog. If an answer doesn't work, the dialog says so and asks
again. A server gives you only so long to answer (two minutes, unless its
administrator changed it). If you take longer, the dialog closes and Endeavor
says the server stopped waiting. Connect again to sign in.

## Add a server

1. Open the **Where** chip on the **Start a session** screen, and click
   **Add server…**. You can also add one in Settings, under **Where notebooks
   run**.
2. Fill in the fields:
   - **SSH host**: an alias from your `~/.ssh/config`, or `user@host`. Add
     `:port` if the server doesn't use port 22. Aliases from your
     `~/.ssh/config` are listed under the field. Click one to use it.
   - **Name**: what Endeavor calls the server. Leave it empty to use the SSH
     host.
   - **How to get Julia**: see [Julia on the server](#julia-on-the-server).
     You can leave it empty.
   - **Stop idle notebooks after**: how long a notebook nobody uses keeps
     running. The default follows Settings.
3. Click **Test connection**. Endeavor connects, installs what it needs,
   finds Julia and starts it once. Each step shows as it happens. When all is
   well, it says the server "works with Endeavor".
4. Click **Add**.

To change a server later, click the gear next to it in the **Where** chip or
in Settings. **Remove server…** makes Endeavor forget it. Anything still
running there keeps running.

## Start a session on the server

On the **Start a session** screen, pick the server in the **Where** chip.
Then pick a folder on the server with the folder chip. **Browse…** shows the
server's folders. The notebook and the files the session makes are saved in
that folder on the server.

## What Endeavor installs on the server

Everything goes in `~/.cache/endeavor/` in your home folder on the server:

- a small program, `endeavor`, that relays between your Mac and Julia, with
  the files Julia needs to run the notebook, in a folder named after
  Endeavor's version;
- the notebook's state, in `state/`;
- Julia's packages, in `depot/`;
- Julia itself, only if Endeavor has to download it.

Endeavor installs over the same SSH connection it uses to connect, so you
sign in once. Nothing needs administrator rights.

## Julia on the server

**How to get Julia** takes one of three things:

- nothing: Endeavor uses the `julia` it finds when you sign in. If there is
  none, or it's older than 1.11, Endeavor downloads its own Julia 1.12.6 into
  `~/.cache/endeavor/`;
- the path to a `julia` program, such as `/opt/julia-1.12/bin/julia`;
- a shell line that makes `julia` available, such as `module load julia`.

Endeavor needs Julia 1.11 or newer.

## What Claude can do on a server

In a server session, Claude's own tools for your Mac's files and commands
are switched off, because they would look at the wrong computer. Instead,
Claude can list folders, read files and run commands on the server. In
**Manual** and **Ask to run**, each command asks first, with a card such as "Run a command on lab-server?" that
shows the command and the folder it runs in.

## Notebooks keep running

A notebook on a server keeps running when you quit
Endeavor or lose the connection. When you come back, Endeavor reconnects to
it. A long calculation goes on while your laptop is closed.

A notebook stops when:

- you stop it: **Stop notebook** in its **⋮** menu, or **Stop** next to the
  server in Settings, under **Where notebooks run**;
- nobody has used it for the time set in **Stop idle notebooks after**, 48
  hours unless you changed it.

The notebook's file stays on the server either way. **Start** runs it again.

Only one computer can be connected to a server's Julia at a time. If you
connect from another Mac, that one takes over, and the first says "Another
connection took over Julia on lab-server." with a **Reconnect** button.

## Servers that are clusters

If the server is a cluster's login node, Endeavor notices when you start a
session and asks: "This looks like a cluster's login node. Clusters stop
long-running programs here. Add it as a cluster instead?". Choose **Add as
cluster** to run the notebook in a cluster job. See [Clusters](./clusters.md).

## When the server can't be reached

See [Can't reach a server](./troubleshooting.md#cant-reach-a-server).
