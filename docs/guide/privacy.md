---
title: Privacy and what stays on your computer
description: What Endeavor sends to Anthropic, what runs where, and what it keeps on your Mac.
sidebar:
  order: 120
---

Endeavor and Claude Code run on your Mac. Your notebooks run on your Mac or on
a server you choose. What you and Claude say to each other goes to Anthropic,
the company that makes Claude. Apart from the downloads and checks listed
under [Other connections Endeavor makes](#other-connections-endeavor-makes),
Endeavor sends nothing anywhere.

## What goes to Anthropic

Claude runs on Anthropic's servers. Endeavor talks to it through Claude Code,
which it installs and runs on your Mac. Everything in the conversation goes
to Anthropic. That includes:

- the messages you write;
- pictures you attach, and pictures of plots or boxes you pick with Point;
- the text of a reply, a cell or a result you quote with Reply or ⌘E;
- the code and the full error of a cell when you click **✦ Fix with
  Claude** or **Explain**;
- a short note with your messages saying which notebook you're looking at, and
  which cells you changed since Claude last saw them;
- everything Claude reads with its tools: the notebook's code and results,
  files in the session's folder, and the output of commands it runs.

Claude Code also reads a `CLAUDE.md` file and Claude Code settings in the
session's folder when there are any, so their contents go to Anthropic too.

Endeavor doesn't send your notebook or your folder to Claude by itself. A
file you attach from elsewhere is copied into the session's folder, and Claude
is told its name. Its contents go to Anthropic only if Claude reads it.
Claude usually does, since that's why you attached it.

So treat anything in the session's folder, and anything a notebook can load,
as something Claude may read. Keep data you must not share with Anthropic
out of the folder. How Anthropic handles the conversation depends on your
Claude account. See [Anthropic's privacy policy](https://www.anthropic.com/legal/privacy).

## What runs where

| Part | Where it runs |
|---|---|
| Endeavor | your Mac |
| Claude Code, which talks to Claude | your Mac |
| Claude | Anthropic's servers |
| Julia and your notebook | your Mac, or the server or cluster you chose |

Nothing Claude-related is installed on a server, and your Claude sign-in is
never sent to a server. Endeavor reaches a server only through SSH. See
[Servers](./servers.md).

## Your Claude sign-in

Claude Code handles your sign-in. You type your password only in your web
browser, on claude.ai or console.anthropic.com. Endeavor never sees it and
doesn't store your sign-in itself.

**Use my Claude Code setup** in Settings is off by default. While it's off,
Endeavor's sessions don't load your own Claude Code settings or the tools you
connected to Claude Code.

## Other connections Endeavor makes

- **On first launch**, it downloads Node.js from nodejs.org and Claude Code
  from the npm package registry. **The first time you open a Julia
  notebook** (or when you click **Install** in Settings), it downloads Julia
  from julialang.org, through juliaup if you use it. On Windows, Julia always
  comes through juliaup, which Endeavor installs from the Microsoft Store, or
  from install.julialang.org if the Store doesn't work, when you don't have
  it.
- **Julia** downloads the packages your notebooks use from Julia's package
  servers.
- **To check that you're online**, it opens a connection to Anthropic's API
  server and closes it again without sending anything.
- **Web pages** that Claude fetches or searches for, after you allow it with
  a "Fetch a web page?" or "Search the web?" card, are fetched from your
  Mac.
- **Servers and clusters** you add are reached over SSH. A server downloads
  Julia and packages itself when it needs them.
- **Links you click**, such as **Report**, open in your browser.

Endeavor itself has no analytics, no usage tracking and no crash reporting,
and it doesn't check for updates. Claude Code may contact Anthropic for its
own purposes, such as error reports. Endeavor doesn't change Claude Code's
settings for that.

## The connection to your notebook

Endeavor's window and Claude Code talk to Julia over a network port that only
programs on the same computer can reach. A random secret, a token, guards that
port, so other people signed in to the same computer or server can't use it.
The token is stored in a file only your user account can read: on your Mac
in `~/Library/Application Support/endeavor/runtime/`, on a server in
`~/.cache/endeavor/`. On a server, Julia's port stays on the server and your
Mac reaches it through the SSH connection. Endeavor opens no port on a
server to the network.

## What Endeavor keeps on your Mac

In `~/Library/Application Support/endeavor/`:

- your settings, and the list of your sessions with their folders and
  notebooks;
- `transcripts/`: a copy of each session's chat, so it shows at once when you
  open the session. Endeavor never sends this copy to Claude. Deleting the
  session deletes it.
- `hosts/`: one folder per server, which Claude Code works in for that
  server's sessions;
- the programs it installed, and Julia's packages.

The log is in `~/Library/Logs/Endeavor/`.

Claude Code keeps each conversation's history so a session can continue
later. **Delete…** on a session asks Claude Code to delete the conversation,
and removes Endeavor's copy of it. Rules you saved with **In this folder**
stay in the folder. Your notebooks and the files in the session's folder stay where they
are until you delete them yourself.
