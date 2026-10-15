---
title: Install Endeavor
description: Download Endeavor for a Mac or Windows, or build it yourself on Linux.
sidebar:
  order: 15
---

Endeavor has test builds for Macs with Apple silicon and for Windows 10 or
11 (x64). They are rebuilt from the newest working version of the app, and
they can't update themselves yet: to get a newer one, download it again.

Before you start, check [What you need](./overview.md#what-you-need).

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
[build Endeavor from its source code](https://github.com/jowch/Endeavor#build-it).
It runs under X11.

Next: [Get started](./getting-started.md).
