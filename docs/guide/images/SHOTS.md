---
title: Screenshot list
description: The screenshots the user guide needs, and what must be on screen in each.
draft: true
---

All seven screenshots below are in this folder, and their lines are in the
pages listed under **Where it goes**. Each entry stays here as the spec for
a retake, with a **Taken** note on how the current picture differs from it.

The current set was taken on 3 October 2026 from a release build at 2×
(2880×1800 pixels for the full window), except `cluster-resources.png`,
which is still the earlier 1× picture. Retake it at 2× when a cluster is at
hand.

To retake a shot, take it from a release build (`scripts/bundle.sh`), so
there are no debug-only marks, and save it here under the same file name.
The documentation site's build fails on an image that is missing, so
don't delete a picture while a page still uses it.

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

**Taken.** The notebook has three cells: `using`, `flips` and the plot,
which has no name and shows as `let`. The first turn's changed-cells card
lists all three, two tagged **new**. In the second turn Claude edited both
`flips` and the plot cell, and its run was denied, so both have the orange
bar and the reply says the run was declined.

**Where it goes.** Each line sits where it says, with a blank line before
and after it:

- [What Endeavor is](../overview.md), after the first paragraph, before **The words this guide uses**:

  ```markdown
  ![The Endeavor window: the list of sessions on the left, the chat in the middle, and the notebook on the right](images/main-window.png)
  ```

- [Sessions, the chat and the notebook](../sessions.md), after the first paragraph, before **One notebook per session**:

  ```markdown
  ![The Endeavor window during a session: the chat shows Claude's reply and the changed-cells card, and the notebook shows a cell with an orange bar at its left edge](images/main-window.png)
  ```

**On screen:**

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

**Taken.** From a release build started with `HOME` set to an empty
folder, so Claude Code found no sign-in and setup wasn't finished; nobody
was signed out. Cropped to the middle 720 points of the window.

**Where it goes.** The line sits where it says, with a blank line before
and after it:

- [Get started](../getting-started.md), under **Sign in to Claude**, after the paragraph that ends "Click **Continue**.", before the numbered list:

  ```markdown
  ![The "Sign in to Claude" screen, with the two kinds of account to choose from](images/sign-in.png)
  ```

**On screen:**

- The first-launch setup window, at the **Sign in to Claude** step: the two
  choices **A Claude plan** and **An Anthropic Console account**, each with
  its **Sign in** button, the "Not sure?" hint and the lock line.
- Taken on a fresh user account, or after **Sign out**, so no account is
  shown.
- The setup window's sky band is dark in both appearances. Take it in Light.
- Crop to the setup window's content.

## start-session.png

**Taken.** As specified. The mode shows in the row under the message box,
not among the chips.

**Where it goes.** The line sits where it says, with a blank line before
and after it:

- [Get started](../getting-started.md), under **Start your first session**, after the first paragraph, before "Under **Try one of these, or ask in your own words**":

  ```markdown
  ![The "Start a session" screen, with four example prompts, the chips above the message box, and the empty notebook pane on the right](images/start-session.png)
  ```

**On screen:**

- The **Start a session** screen with no past sessions, so the four example
  prompts show under **Try one of these, or ask in your own words**.
- Chips above the message box: **This Mac**, the folder `Endeavor`, **New
  notebook**, and the mode **Manual**.
- The notebook pane on the right shows "Your notebook will appear here".
- Sidebar open and empty apart from **+ New session**.
- Full window, no crop.

## approval-card.png

**Taken.** The diff has one line removed and one added (a lower bound
added to `curve_fit`), and the card says "Also re-runs 2 cells that depend
on it." The chat column above the card shows the previous turn's reply.

**Where it goes.** The line sits where it says, with a blank line before
and after it:

- [Modes and approvals](../modes-and-approvals.md), first under **Answer a card**, before "A card asks one question":

  ```markdown
  ![An approval card above the message box asking "Edit `fit` and run it?", with the change shown as a diff and the Deny, Always this session and Edit and run buttons](images/approval-card.png)
  ```

**On screen:**

- A This Mac session in **Ask to run**, titled "Decay fit".
- Card waiting above the empty message box: "Edit `fit` and run it?", with a
  diff of two or three lines (one removed, two added), the line "Also re-runs
  2 cells that depend on it.", and the buttons **Deny**, **Always this
  session** and **Edit and run** with their key hints.
- In the notebook, the `fit` cell is marked "Claude asks to run this.", and
  the two cells after it "Re-runs after it".
- Crop to the chat column and the notebook column; leave out the sidebar.

## safe-preview.png

**Taken.** As specified: `decay.jl`, a copy of the "Decay fit" notebook, with six cells.

**Where it goes.** The line sits where it says, with a blank line before
and after it:

- [Safe preview](../safe-preview.md), under **What you see**, after the paragraph that ends "a **Run notebook** button.", before "You can edit cells in safe preview.":

  ```markdown
  ![A notebook in safe preview: the Safe preview label in the header, and the box at the top of the notebook with the Run notebook button](images/safe-preview.png)
  ```

**On screen:**

- A session opened on an existing notebook from disk, such as `decay.jl`
  with four or five cells and no outputs.
- The notebook header shows the **Safe preview** label.
- The box at the top of the notebook: "Safe preview", "You're reading and
  editing this file without running any code." and **Run notebook**.
- No card in the chat.
- Crop to the notebook column.

## point.png

**Taken.** After the second turn's edits ran, so the plot shows 10,000
flips. The bar reads "Figure in let", because the plot cell has no name.
The notebook column is narrower than in the earlier set, from a wider chat
column.

**Where it goes.** The line sits where it says, with a blank line before
and after it:

- [Sessions, the chat and the notebook](../sessions.md), at the end of **Ask about one part of the notebook**, after the paragraph that ends "keeps what you typed for next time.", before **When a cell fails**:

  ```markdown
  ![Point turned on: a plot is picked, and a bar under it holds the question for Claude](images/point.png)
  ```

**On screen:**

- The "Coin flips" session with its plot.
- Point is on: the notebook is dimmed, the hint pill shows at the top, and
  the plot is picked (outlined, tagged "Figure").
- The comment bar under the plot reads "Figure in plot" (or the plot cell's
  name) and holds the typed question "Why is the line not exactly at 0.5?".
- Crop to the notebook column.

## cluster-resources.png

**Taken.** The cluster "hpc" was a test VM. Its partitions were set in the
app's `hosts.json` (`standard`, 64 CPUs, 256 GB, 48 h) so that **Medium**
fits.

**Where it goes.** The line sits where it says, with a blank line before
and after it:

- [Clusters](../clusters.md), under **Choose the job's resources**, after the line that ends "Click it to set this session's job:", before the list of presets:

  ```markdown
  ![The resources popover for a cluster session, with the Small, Medium and Large presets, the partition, and CPUs, memory and time limit](images/cluster-resources.png)
  ```

**On screen:**

- The **Start a session** screen with a cluster picked in the **Where** chip
  (named, for example, "hpc").
- The resources popover open: the presets **Small**, **Medium** and **Large**
  with **Medium** selected, a partition picked, and CPUs, memory and time
  limit showing 8, 32 GB and 8 h.
- Crop to the popover and the chips under it.
