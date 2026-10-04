---
title: Troubleshooting
description: What to do when Julia, a notebook, a server or Claude doesn't work.
order: 110
---

Most problems have a button where they show up. This page explains each one,
and what to try when the button doesn't help. Settings has a
**Troubleshooting** page with the same tools. Try them in the order below.

## A notebook won't start or keeps failing

1. **Restart Julia.** In Settings, under **Troubleshooting**, click
   **Restart** next to **Restart Julia**. It stops your notebooks on this Mac
   and starts them again. Their files are already saved.
2. **Repair Julia.** If restarting doesn't help, click **Repair…** next to
   **Repair Julia**, then **Repair**. Endeavor clears what Julia saved and
   compiled, then starts it again. It takes a few minutes. Your notebooks,
   packages and settings stay. Endeavor also offers **Repair** by itself when
   Julia on your Mac fails to start and **Retry** fails twice.
3. **Report it.** See [Report a problem](#report-a-problem).

## Julia stopped unexpectedly

If Julia stops while a notebook runs, the notebook pane says "Julia stopped
unexpectedly". The notebook file is saved, but its results are gone until the
cells run again. When Endeavor knows which cell was running, it names it.
Very large data can make Julia run out of memory.

Click **Restart Julia** to open the notebook again and run it. If Julia stops
again during that run, the notebook opens in
[safe preview](./safe-preview.md) instead, so you can look at that cell
before you run it again. **Show log** shows Julia's own log.

## "Julia is from an older Endeavor"

Julia can keep running after Endeavor quits: always on a server, and on your
Mac when **Keep notebooks running after Endeavor quits** is on. After you
install a newer Endeavor, that Julia is still the one the older version
started. Each session that uses it then says:

"Julia on lab-server is from an older Endeavor. Restart Julia to get the
latest changes. Until then, Ask to run doesn't let Claude run code."

Until you restart it, Claude can still read and edit the notebook. In **Ask
to run**, it can't run code, because that Julia can't ask you first. To
restart it:

- On your Mac, use **Restart Julia** in Settings, under **Troubleshooting**.
- On a server, click **Stop** next to the server in Settings, under **Where
  notebooks run**. Then click **Start on lab-server** in the notebook pane.

When Endeavor can't connect to the older Julia at all, the notebook pane
shows a **Restart Julia on lab-server** button instead (**Restart Julia** on
your Mac). It asks first, since notebooks
open there stop. Their files are saved.

## A package won't install

When a package a notebook uses fails to install, the notebook's header shows
**Package failed**. Click it to open **Status**, which names the package and
the cells that can't run without it. From there:

- **✦ Fix with Claude** asks Claude to find out why and fix it.
- **↻ Restart notebook** starts the notebook again.
- **Log for** the package shows Julia's package log.

A package name with a typo, or a package that isn't in Julia's registry, also
shows here.

## Can't reach a server

When Endeavor can't reach a server, the session says, for example, "Can't
reach lab-server. Endeavor keeps trying." It reconnects by itself. Click **Try
now** to try at once. Meanwhile:

- the notebook on the server keeps running, but it's read-only in Endeavor;
- messages you write wait, and send once the server is back.

If it doesn't come back:

- If the server needs your university's or company's VPN, check that the VPN
  is on.
- Check that `ssh` to the server works in Terminal.
- Open the server's settings with its gear and click **Test connection**.
  Endeavor says in plain words what went wrong, for example that the server
  refused the sign-in, or that the connection timed out.

If the server's identity (its host key) changed, Endeavor doesn't connect,
and tells you the `ssh-keygen -R` command to run once you know why it
changed.

## Sign-in problems

If sign-in doesn't finish, Endeavor shows **Sign-in didn't finish** with the
reason, a **Try again** button and **Details**.

- **"The browser page closed before sign-in finished."** Click **Try
  again**, and finish signing in in the browser.
- **"This Claude account has no plan that includes Claude Code."** A free
  Claude account can't be used. Choose a plan at claude.ai and try again, or
  click **Use a Console account**.
- **"This Console account can't use Claude yet."** Ask whoever manages the
  account, or add credit at console.anthropic.com.

When your sign-in runs out later, a card says **Sign in to Claude again**.
Click **Sign in**, or **Use another account**. Your notebooks keep working
meanwhile, and a message you sent is kept and goes after you sign in.

## Claude doesn't answer

- **"Claude couldn't answer"**, with a reason such as "Anthropic's servers are
  busy right now." Your message is kept. Click **Try again**.
- **"You've reached your Claude usage limit."** The message says when it
  resets. Messages you send meanwhile wait and go then.
- **"Claude stopped before finishing"**: the reply was cut short. Click
  **Continue**.
- **"Claude stopped unexpectedly. Restarting it…"**: Endeavor restarts Claude
  by itself, and your sessions load again where they were.
- **"Claude isn't running"**: Claude stopped twice in a minute, so Endeavor
  stopped restarting it. Click **Restart Claude**. If it keeps stopping,
  click **Show logs**, and [report a problem](#report-a-problem).

## Log files

Endeavor writes a log of what it does. In Settings, under
**Troubleshooting**, click **Show in Finder** next to **Log files**. The log
is `~/Library/Logs/Endeavor/endeavor.log`, and the one from the time before
is `endeavor.old.log` next to it. Julia's own log on your Mac is
`~/Library/Application Support/endeavor/runtime/runtime.log`.

## Report a problem

In Settings, under **Troubleshooting**, click **Report** next to **Report a
problem**. It opens a new issue on GitHub, where you can describe what happened. Attach the log
files. Read them first: they can contain file names and output from Julia.
