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

**New sessions use** shows **Claude** and whether you're signed in. Click
**Settings** next to Claude for these:

- **Use my Claude Code setup**. Off by default. When on, new sessions also
  load your own Claude Code settings and the tools you connected to it. Leave
  it off if you only use Claude in Endeavor.
- **Signed in as**, with your plan and organisation.
- **Plan and usage**, with **Open claude.ai**, which shows your usage.
- **Sign out**. Claude stops answering in every session until you sign in
  again.

Cursor, Codex and Gemini are listed as **Not available yet**.

## Notebooks

| Setting | Default | What it does |
|---|---|---|
| **Stop idle notebooks after** | 48 hours | A notebook nobody has used for this long stops, even with Endeavor open, to free memory. **Start** brings it back. The choices are 12 hours, 24 hours, 48 hours, 1 week and Never. |
| **Keep notebooks running after Endeavor quits** | Off | Notebooks on your Mac carry on while Endeavor is closed, and it reconnects when you open it. Idle ones still stop. Notebooks on servers always keep running. |
| **Run notebook code without asking** | Off | Claude runs the cells it writes without asking first. It applies to new sessions. To change one session, use the mode under the message box. See [Modes and approvals](./modes-and-approvals.md). |

**Julia** sets which Julia runs your notebooks on your Mac. Click
**Settings** next to it to choose:

- **Endeavor's Julia** (the default): the Julia that Endeavor installed and
  keeps up to date.
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
Claude. The app can't update itself yet, so its row says "Up to date" even
when a newer Endeavor exists.
