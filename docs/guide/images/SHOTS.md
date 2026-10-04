---
title: Screenshot list
description: The screenshots the user guide needs, and what must be on screen in each.
draft: true
---

The guide references these images before they exist. Take each one from a
release build (`scripts/bundle.sh`), so there are no debug-only marks, and
save it here under the file name given.

For every shot:

- Window size 1440×900 points, saved at 2× (2880×1800 pixels). Crop only
  where the entry says so.
- Light appearance (Settings, **Appearance**, **Light**), unless the entry
  says otherwise.
- Notebook look **Endeavor**.
- A clean sidebar: a few sessions with plain titles, such as "Coin flips" and
  "Decay fit", in a folder named `Endeavor`. No personal names, paths or
  e-mail addresses anywhere on screen.
- Model and effort left at their defaults.

## main-window.png

Used on [What Endeavor is](../overview.md) and [Sessions, the chat and the
notebook](../sessions.md).

- A This Mac session titled "Coin flips", in **Ask to run**.
- The chat: the user's message "Simulate 1,000 coin flips and plot how often
  heads comes up", Claude's reply with two or three collapsed step lines, and
  the end-of-turn changed-cells card with two cells, one tagged **new**.
- A second, short user message "Use 10,000 flips", and Claude's reply to it
  in which it edited `flips` and has not run it yet. No card waiting.
- The notebook: three cells (`flips`, a counting cell, a plot of the share of
  heads). The `flips` cell has the orange bar, its changed line marked in the
  margin, and its old output faded.
- Sidebar open, with the "Coin flips" row selected and one other session row
  showing the orange "new reply" dot.
- Full window, no crop.

## sign-in.png

Used on [Get started](../getting-started.md).

- The first-launch setup window, at the **Sign in to Claude** step: the two
  choices **A Claude plan** and **An Anthropic Console account**, each with
  its **Sign in** button, the "Not sure?" hint and the lock line.
- Taken on a fresh user account, or after **Sign out**, so no account is
  shown.
- The setup window's sky band is dark in both appearances. Take it in Light.
- Crop to the setup window's content.

## start-session.png

Used on [Get started](../getting-started.md).

- The **Start a session** screen with no past sessions, so the four example
  prompts show under **Try one of these, or ask in your own words**.
- Chips above the message box: **This Mac**, the folder `Endeavor`, **New
  notebook**, and the mode **Manual**.
- The notebook pane on the right shows "Your notebook will appear here".
- Sidebar open and empty apart from **+ New session**.
- Full window, no crop.

## approval-card.png

Used on [Modes and approvals](../modes-and-approvals.md).

- A This Mac session in **Ask to run**, titled "Decay fit".
- Card waiting above the empty message box: "Edit `fit` and run it?", with a
  diff of two or three lines (one removed, two added), the line "Also re-runs
  2 cells that depend on it.", and the buttons **Deny**, **Always this
  session** and **Edit and run** with their key hints.
- In the notebook, the `fit` cell is marked "Claude asks to run this.", and
  the two cells after it "Re-runs after it".
- Crop to the chat column and the notebook column; leave out the sidebar.

## safe-preview.png

Used on [Safe preview](../safe-preview.md).

- A session opened on an existing notebook from disk, such as `decay.jl`
  with four or five cells and no outputs.
- The notebook header shows the **Safe preview** label.
- The box at the top of the notebook: "Safe preview", "You're reading and
  editing this file without running any code." and **Run notebook**.
- No card in the chat.
- Crop to the notebook column.

## point.png

Used on [Sessions, the chat and the notebook](../sessions.md).

- The "Coin flips" session with its plot.
- Point is on: the notebook is dimmed, the hint pill shows at the top, and
  the plot is picked (outlined, tagged "Figure").
- The comment bar under the plot reads "Figure in plot" (or the plot cell's
  name) and holds the typed question "Why is the line not exactly at 0.5?".
- Crop to the notebook column.

## cluster-resources.png

Used on [Clusters](../clusters.md).

- The **Start a session** screen with a cluster picked in the **Where** chip
  (named, for example, "hpc").
- The resources popover open: the presets **Small**, **Medium** and **Large**
  with **Medium** selected, a partition picked, and CPUs, memory and time
  limit showing 8, 32 GB and 8 h.
- Crop to the popover and the chips under it.
