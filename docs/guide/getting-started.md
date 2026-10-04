---
title: Get started
description: Open Endeavor for the first time, sign in to Claude, and run your first analysis.
sidebar:
  order: 20
---

This page takes you from opening Endeavor for the first time to a working
notebook with a plot in it.

## Let Endeavor set itself up

Open Endeavor. The first time, a setup window shows four steps:

1. **Julia**
2. **Pluto and its packages**
3. **Claude agent**
4. **Connecting to Claude**

A progress line under the turtle shows the current step, for example
"Setting up Julia · 1 of 4". The first two steps download several hundred
megabytes, so they can take a few minutes. [What Endeavor installs](./overview.md#what-endeavor-installs-on-first-launch)
lists every download.

If a step fails, the window shows which one, with the reason. Click **Retry**
to try that step again, or **Show logs** to see what happened. If you're
offline, setup pauses under **No internet connection** and goes on by itself
when you're back online.

## Sign in to Claude

Endeavor uses your own Claude account to answer you. During setup, under
**Sign in to finish setting up**, it first asks you to choose an assistant.
Claude is the only one available today. Click **Continue**.

1. Under **Sign in to Claude**, pick the kind of account you have:
   - **A Claude plan**: Pro, Max, Team or Enterprise, where you chat with
     Claude at claude.ai.
   - **An Anthropic Console account**: you or your lab pay for what you use,
     at console.anthropic.com.

   If you're not sure, and you chat with Claude at claude.ai, choose the
   first.
2. Click **Sign in**. Endeavor opens the sign-in page in your web browser.
3. Sign in there. Your password stays in the browser. Endeavor never sees it.
4. Come back to Endeavor. The screen moves on by itself.

If the browser page didn't open, click **Open it again**. If sign-in fails,
see [Sign-in problems](./troubleshooting.md#sign-in-problems).

## Start your first session

After sign-in, Endeavor shows **Start a session**. A session is one
conversation with Claude and the notebook it works in.

Under **Try one of these, or ask in your own words** are four examples. Two
need no data. Two use a file of your own.

- "Simulate 1,000 coin flips and plot how often heads comes up"
- "Load a CSV file I'll attach and show me what's in it"
- "Fit an exponential decay to measurements I'll attach, and plot the fit"
- "Show me the basics of Julia with a small worked example"

The chips above the message box say where the session works:

- **Where** the notebook runs. **This Mac** is your computer. You can also
  pick a server or a cluster you have added.
- **The folder** the session works in. A new session on your Mac starts in
  the last folder you used, or in `~/Documents/Endeavor` the first time.
- **The notebook**. **New notebook** lets Claude create one when there is code
  to run. You can also pick a notebook that is already in the folder.

To start:

1. Click **Simulate 1,000 coin flips and plot how often heads comes up**. The
   example goes into the message box.
2. Press Return to send it.
3. Claude asks before each change to the notebook, starting with creating
   it. A card above the message box shows the question, and the code when
   there is some. Press Return, or click the orange button, to let it go
   ahead.
4. Watch the notebook on the right. The cells and the plot appear as Claude
   works.

New sessions start in the **Manual** mode, where Claude asks before every
change to the notebook. Once you're comfortable, you can let it do more on
its own. See [Modes and approvals](./modes-and-approvals.md).

## Ask about what you see

When the first answer is done, ask a follow-up in your own words. For
example: "Explain the first cell line by line", or "Use 10,000 flips and add
a line at one half". You can also point at a cell or a plot and ask about
just that part. See [Point and Reply](./sessions.md#ask-about-one-part-of-the-notebook).

To use your own data, see [Your files](./your-files.md).

## Where your work is saved

The notebook is a `.jl` file in the session's folder. Pluto saves it as it
changes, so there is no Save button to remember. Your sessions are listed in
the sidebar on the left. Click one to come back to it later, even after you
quit Endeavor.
