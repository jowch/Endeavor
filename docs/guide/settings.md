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
  again.

**Codex**, by OpenAI, works in sessions on your Mac. It uses your ChatGPT
sign-in, the same one as the `codex` command. When you're not signed in, its
row has **Sign in**, which opens ChatGPT's sign-in page in your browser, and
**Check again**, for after you sign in with `codex login` in a terminal.

**Antigravity**, by Google, works in sessions on your computer, on Windows
for now. It uses your Google account. When you're not signed in, its row has
**Sign in**, which opens Google's sign-in page in your browser. Unlike
Claude in Endeavor, Antigravity can read any file on your computer without
asking; its commands still ask first. On a Mac or Linux it is listed as
**Not available yet**.

Cursor is listed as **Not available yet**.

## Notebooks

| Setting | Default | What it does |
|---|---|---|
| **Stop idle notebooks after** | 48 hours | A notebook nobody has used for this long stops, even with Endeavor open, to free memory. **Start** brings it back. The choices are 12 hours, 24 hours, 48 hours, 1 week and Never. |
| **Keep notebooks running after Endeavor quits** | Off | Notebooks on your Mac carry on while Endeavor is closed, and it reconnects when you open it. Idle ones still stop. Notebooks on servers always keep running. |
| **Run notebook code without asking** | Off | Claude runs the cells it writes without asking first. It applies to new sessions. To change one session, use the mode under the message box. See [Modes and approvals](./modes-and-approvals.md). |

**Julia** sets which Julia runs your notebooks on your Mac. Click
**Settings** next to it to choose:

- **Endeavor's Julia** (the default): the Julia on your PATH if it's 1.11 or
  newer; otherwise Endeavor downloads its own Julia 1.12.6 the first time you
  open a Julia notebook. On Windows, Endeavor installs it with juliaup,
  Julia's installer, and leaves your other juliaup versions alone.
- **Another Julia on this Mac**: a Julia you installed yourself. Click
  **Choose…** (or **Change…**) and pick the file named `julia`, in a `bin`
  folder. It must be Julia 1.11 or newer.

The change takes effect when Julia restarts. Servers and clusters set their
own Julia. See [Servers](./servers.md).

R and Python notebooks are listed as **Not available yet**.

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
- **Pluto notebook look**: **Endeavor** (the default) puts Pluto's controls in
  the notebook's header, in Endeavor's colours. **Pluto classic** shows
  Pluto's own page, as it looks outside Endeavor. You can also switch in the
  notebook's **⋮** menu. Exported notebooks always use Pluto's standard look.

## Troubleshooting

**Restart Julia**, **Repair Julia**, **Log files** and **Report a problem**.
See [Troubleshooting](./troubleshooting.md).

## About

The version of Endeavor, and of the Claude Code adapter it uses to talk to
Claude. Endeavor can't check for a newer version or update itself yet, so
its row shows only the version you have. To get a newer one, build it again
from the source code (see [Overview](./overview.md)).
