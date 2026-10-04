---
title: Working offline
description: What still works without an internet connection, and what waits until you're back.
order: 90
---

After the first setup, only Claude needs the internet. Notebooks on your Mac
keep working offline.

## What works offline

- Notebooks on your Mac run as usual. You can edit cells, run them and see
  the results.
- You can read every session's chat and notebook.
- You can write messages to Claude. They wait and send in order when you're
  back.

The session shows "You're offline. The notebook still works. Claude will
continue when you're back."

## What waits

- **Claude.** It can't answer until you're back online. Messages you send
  queue above the message box, under "These send in order when you're back."
  A reply that was cut off when the connection dropped shows a **Continue**
  button.
- **Server and cluster notebooks.** Endeavor can't reach them, so their
  notebooks are read-only until it reconnects. They keep running on the
  server meanwhile.
- **First setup.** Endeavor needs the internet once, to download Julia,
  Node.js and Claude Code. If you're offline, setup pauses and goes on by
  itself when you're back.
- **New packages.** A notebook that needs a Julia package it hasn't
  installed yet can't download it.

Endeavor reconnects by itself. The status line at the bottom of the sidebar
says "Offline · reconnects by itself" until it does.
