---
title: Settings
description: Every setting in Endeavor, what it does, and its default.
sidebar:
  order: 100
---

To open Settings, press ⌘, or click the gear at the bottom of the sidebar.
Type in **Search settings** to find a setting by name. Press Esc to close
Settings.

## Assistants

**New sessions use** shows the assistants new sessions can use, and whether
you're signed in to each. The one picked is the one the new-session screen
starts with. Click **Settings** next to Claude for these:

- **Use my Claude Code setup**. Off by default. When on, new sessions also
  load your own Claude Code settings and the tools you connected to it. Leave
  it off if you only use Claude in Endeavor.
- **Signed in as**, with your plan and organisation.
- **Plan and usage**, with **Open claude.ai**, which shows your usage.
- **Sign out**. Claude stops answering in every session until you sign in
  again. Endeavor uses the same sign-in as the `claude` command, so this also
  signs out `claude` in your terminal. Signing in here signs it in too.

**Codex**, by OpenAI, works in sessions on your Mac. It uses your ChatGPT
sign-in, the same one as the `codex` command. When you're not signed in, its
row has **Sign in**, which opens ChatGPT's sign-in page in your browser, and
**Check again**, for after you sign in with `codex login` in a terminal.

**Antigravity**, by Google, works in sessions on your computer. It uses your
Google account. When you're not signed in, its row has **Sign in**, which
opens Google's sign-in page in your browser. Unlike Claude in Endeavor,
Antigravity can read any file on your computer without asking; its commands
still ask first.

Cursor is listed as **Not available yet**.

## Notebooks

| Setting | Default | What it does |
|---|---|---|
| **Stop idle notebooks after** | 48 hours | A notebook nobody has used for this long stops, even with Endeavor open, to free memory. **Start** brings it back. The choices are 12 hours, 24 hours, 48 hours, 1 week and Never. |
| **Keep notebooks running after Endeavor quits** | Off | Notebooks on your Mac carry on while Endeavor is closed, and it reconnects when you open it. Idle ones still stop. Notebooks on servers always keep running. |
| **Run notebook code without asking** | Off | Claude runs the cells it writes without asking first. It applies to new sessions. To change one session, use the mode under the message box. See [Modes and approvals](./modes-and-approvals.md). |

**Julia** sets which Julia runs your notebooks on your Mac. Click
**Settings** next to it to choose:

- **Endeavor's Julia** (the default): Julia 1.12.6, the version Endeavor is
  tested with. Endeavor installs it the first time you open a Julia notebook,
  or when you click **Install**. If you use juliaup, Julia's installer,
  Endeavor adds 1.12.6 to it rather than downloading a second Julia, and
  leaves your other juliaup versions alone. On Windows it always comes through
  juliaup, which Endeavor installs if you don't have it. A Julia on your PATH
  isn't used unless you choose it below. **Remove…** deletes Endeavor's Julia
  (Julia on your Mac stops first if it's running); it's installed again the
  next time you open a Julia notebook. If you added 1.12.6 to juliaup
  yourself, Endeavor leaves it there.
- **Another Julia on this Mac**: a Julia you installed yourself. Click
  **Choose…** (or **Change…**) and pick the file named `julia`, in a `bin`
  folder. It must be Julia 1.11 or newer.

The change takes effect when Julia restarts. Servers and clusters set their
own Julia. See [Servers](./servers.md).

**R** sets which R runs your R notebooks on your Mac or Linux computer.
Click **Settings** next to it to choose:

- **Find R by itself** (the default): the R your login shell finds. On a
  Mac, Endeavor's own R 4.6.1 comes first once it's installed, and when a
  Mac has no R at all, the first R notebook offers **Install R** (about
  165 MB, from CRAN). **Install** and **Remove…** under **Endeavor's R** do
  the same ahead of time, or undo it; notebooks on your Mac stop first if
  they're running. On Linux, Endeavor doesn't install R:
  [rig](https://github.com/r-lib/rig) installs one without admin rights.
- **Another R on this computer**: an R you installed yourself. Click
  **Choose…** (or **Change…**) and pick the file named `Rscript`, in a `bin`
  folder.

A change takes effect when notebooks restart: **Restart** appears under the
choice while notebooks run. Servers and clusters set their own R. R notebooks
don't run on Windows yet, so there the row says **Not available yet**, as
Python's does everywhere.

## Where notebooks run

This lists your Mac, your servers and your clusters, with what each is doing.
Each has at most one button: **Stop** to stop Julia there, **Cancel job** for
a cluster job still in the queue, or **Try now** for one Endeavor can't
reach. The gear next to a server or cluster opens its settings. **Add
server…** and **Add cluster…** add new ones. See [Servers](./servers.md) and
[Clusters](./clusters.md).

## Appearance

- **Light or dark**: **Dark** (the default), **Light**, or **Match macOS**,
  which follows your Mac's setting. The whole window changes, notebook
  included.
- **Notebook look**: **Endeavor** (the default) puts the notebook's controls
  in its header, in Endeavor's colours. **Classic** shows Pluto's or Ember's
  own page, as it looks outside Endeavor. You can also switch in the
  notebook's **⋮** menu. Exported notebooks always use their standard look.

## Troubleshooting

**Restart Julia**, **Repair Julia**, **Log files** and **Report a problem**.
See [Troubleshooting](./troubleshooting.md).

## About

The version of Endeavor, and of the Claude Code adapter it uses to talk to
Claude. The Mac and Windows test builds check for a newer test build once a day, and
**Check now** checks straight away. A newer one downloads in the background;
**Restart** installs it, and so does quitting Endeavor. While an agent is
working, Restart waits until it's done. When Endeavor can't replace
itself, it offers **Download** instead: on a Mac when it isn't in your
Applications folder, on Windows when it wasn't put there by the installer. Other copies, such as one built from the source code, show only
their version (see [Overview](./overview.md)).
