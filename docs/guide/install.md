---
title: Install Endeavor
description: Download Endeavor for a Mac or Windows, or build it yourself on Linux.
sidebar:
  order: 15
---

Endeavor has test builds for Macs with Apple silicon and for Windows 10 or
11 (x64). They are rebuilt from the newest working version of the app. Both
update themselves: once a day Endeavor checks for a newer test build and
fetches it, and it installs the next time you restart or quit Endeavor. On
Windows the installer runs again, without its windows.

Before you start, check [What you need](./overview.md#what-you-need).

<!-- agent-install: start. A copy of Endeavor's site/agent-install.md; edit it there and run site/scripts/agent-install.mjs. -->
<details>
<summary><strong>Installing with an agent?</strong> The commands for Claude Code, Codex and Antigravity.</summary>

To use Endeavor from an agent you already run, install the EndeavorMCP
plugin into it. The plugin gives the agent Endeavor's notebook tools: live
Julia and R notebooks that it edits and runs while you watch in a browser.
The Endeavor desktop app is a separate download that a person installs:
[Install Endeavor](https://jowch.github.io/Endeavor/install/).

**Claude Code:**

```sh
claude plugin marketplace add jowch/EndeavorMCP
claude plugin install endeavor@endeavor
```

**Codex:**

```sh
codex plugin marketplace add jowch/EndeavorMCP
codex plugin add endeavor@endeavor
```

**Antigravity** (the `agy` command line; tested less than the other two).
Give the URL of the `antigravity-plugin` folder, as here. The repository's own
URL installs the Claude Code plugin over this one, and the notebook tools
are then missing.

```sh
agy plugin install https://github.com/jowch/EndeavorMCP/tree/main/antigravity-plugin
```

Codex and Antigravity don't tell the plugin the project folder, so notebook
paths must be absolute. Under `codex exec`, every tool that changes something
fails until the plugin's server is approved in `~/.codex/config.toml`
([how](https://github.com/jowch/EndeavorMCP#install)).

On Windows, Antigravity needs Git for Windows' `bin` folder on the PATH,
Codex isn't supported, and the Claude Code plugin hasn't been tried yet.

**Check that it worked.** `claude plugin list` (or `codex plugin list`) shows
`endeavor@endeavor` enabled. The tools load when a session starts, so ask the
user to start a new session in the folder for their notebooks. There, the
`endeavor` server's `session_status` tool answers without starting anything.
Its first start downloads the `endeavor` program (about 5 MB); if the tools
don't appear, reconnect the server (`/mcp` in Claude Code).

**Update an old plugin.** Claude Code: `claude plugin marketplace update
endeavor`, then `claude plugin update endeavor@endeavor`. Codex: `codex plugin
marketplace upgrade endeavor`. Antigravity: run the install line again. Then
start a new session. A notebook runtime that is already running keeps its old
build until it stops ([Update and remove](https://github.com/jowch/EndeavorMCP#update-and-remove)).

More detail for each agent is in
[EndeavorMCP's README](https://github.com/jowch/EndeavorMCP#install). The same
text for agents is at <https://jowch.github.io/Endeavor/llms.txt>.

</details>
<!-- agent-install: end -->

## On a Mac

1. Download [Endeavor for Mac](https://github.com/jowch/Endeavor/releases/download/nightly/Endeavor-macos-arm64.dmg).
2. Open `Endeavor-macos-arm64.dmg`. A window opens with Endeavor and your
   Applications folder.
3. Drag **Endeavor** onto **Applications**.
4. Open Endeavor from your Applications folder.

If macOS says it can't check Endeavor for malicious software, open
**System Settings → Privacy & Security**, scroll down and click
**Open Anyway** next to the line about Endeavor. You only need to do this
once.

The download is for Apple silicon Macs (M1 and later). On an Intel Mac,
[build Endeavor from its source code](https://github.com/jowch/Endeavor#build-it).

To uninstall, drag Endeavor from Applications to the Trash. Your sessions,
settings and the programs Endeavor set up stay in
`~/Library/Application Support/endeavor/`.

## On Windows

1. Download [Endeavor for Windows](https://github.com/jowch/Endeavor/releases/download/nightly/Endeavor-windows-x86_64-setup.exe).
2. Run `Endeavor-windows-x86_64-setup.exe`. It installs for you only and
   doesn't ask for an administrator.
3. The installer isn't signed yet, so Windows may say "Windows protected
   your PC". Click **More info**, then **Run anyway**.
4. Click **Install**, then **Finish**. Endeavor opens and sets itself up.

The Windows build has rough edges:

- It can't use servers or clusters yet. Notebooks run on your computer.
- Signing in to Claude for the first time hasn't been tried on Windows yet.
- Claude Code on Windows has needed [Git for Windows](https://git-scm.com/downloads/win).
  Install it first if you don't have it.

To uninstall, use **Settings → Apps**. Your sessions and settings stay in
`%LOCALAPPDATA%\Endeavor`, and Julia stays in juliaup.

## On Linux

There is no Linux download. You can
[build Endeavor from its source code](https://github.com/jowch/Endeavor#build-it),
after installing the system packages it needs
([the list for Ubuntu and Debian](https://github.com/jowch/Endeavor/blob/main/docs/linux.md#system-packages-ubuntu-2604)).
It runs under X11. Under Wayland it should run through XWayland, which hasn't
been tested. A Linux build doesn't update itself.

Next: [Get started](./getting-started.md).
