---
title: Your files
description: Give Claude your own data, and find the notebooks and files a session makes.
sidebar:
  order: 60
---

Every session works in one folder. Your notebook is saved there, and files
you give Claude are copied there, so the notebook's code can load them.

## Choose the session's folder

Pick the folder with the folder chip on the **Start a session** screen. It
lists recent folders, and **Browse…** opens any other. A new session on your
Mac starts in the last folder you used there, or in `~/Documents/Endeavor`
the first time. On a server, the folder is on the server. See
[Servers](./servers.md).

Use one folder per project. Put the project's data in it, and start the
project's sessions there. The sidebar groups sessions by folder.

## Where notebooks are saved

Claude creates the notebook in the session's folder when there is code to
run, and gives it a name. It's a plain Julia file ending in `.jl`, which
Pluto can also open outside Endeavor. Pluto saves it as it changes.

To rename or move it, use **Rename…** or **Move to…** in the notebook's
**⋮** menu. Endeavor keeps track of the new place. If you move or rename it
outside Endeavor, the notebook pane says "Can't find" with the file's name.
Click **Locate file…** to show Endeavor where it went.

## Give Claude a file

There are three ways:

- Type `@` in the message box and pick a file that's already in the
  session's folder. Claude reads it from there. Nothing is copied.
- Click **+** under the message box and choose **Add files or photos**
  (⌘U), then pick one or more files from anywhere.
- Drag files onto the message box, or paste them.

Each file shows as a chip above the message box. Write what you want done
with it, and send.

What happens to the file depends on what it is:

- **A file from outside the session's folder** is copied into a `data`
  folder inside it when you send the message. Claude is told where the copy
  is, and reads it from there. The original stays where it was. If a file
  with the same name and the same contents is already there, Endeavor uses
  that one. If the name is taken by a different file, the copy gets a number,
  such as `decay (2).csv`. Files can be up to 2 GB.
- **A picture** (PNG, JPEG, GIF or WebP, up to about 3.7 MB) goes into the
  message itself, so Claude can look at it. It isn't copied into the folder.
  To analyze a picture's data with code, put the file in the session's
  folder and name it with `@`.

On a server, the copy goes into the `data` folder of the session's folder on
the server.

## Try it

Start a session and pick the example "Load a CSV file I'll attach and show
me what's in it". Attach a CSV file of your own with **+**, and send. Claude
writes cells that load the copy in `data` and show what's in it.
