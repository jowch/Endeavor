---
title: About this folder
description: Conventions for writing the Endeavor user guide.
draft: true
---

This folder is the Endeavor user guide. To read it, start with
[What Endeavor is](./overview.md). The pages, in menu order:

1. [What Endeavor is](./overview.md)
2. [Get started](./getting-started.md)
3. [Sessions, the chat and the notebook](./sessions.md)
4. [Modes and approvals](./modes-and-approvals.md)
5. [Safe preview](./safe-preview.md)
6. [Your files](./your-files.md)
7. [Servers](./servers.md)
8. [Clusters](./clusters.md)
9. [Working offline](./offline.md)
10. [Settings](./settings.md)
11. [Troubleshooting](./troubleshooting.md)
12. [Privacy and what stays on your computer](./privacy.md)

The rest of this page is for people who write the guide.

## How the site uses this folder

The Endeavor website fetches `docs/guide/**` from this repo when it builds,
and publishes every page here except the pages for contributors. Those pages (this one and
[images/SHOTS.md](images/SHOTS.md)) have `draft: true` in their front matter,
and the site must skip any page marked that way. Keep that marker on them.

## Who the guide is for

The main reader is a scientist taking on their own analysis who isn't
comfortable with code or notebooks yet. An experienced reader must find what
they need without wading through basics.

- Write in plain language. Explain a term the first time a page uses it.
- Use short, declarative sentences. Lead with what the reader can do.
- No metaphors and no marketing claims. Describe only what the app does
  today.
- Use the app's exact labels, taken from the code in `src/` and
  `frontend/src/`. Put them in bold: **Run notebook**.
- When a feature is missing or planned, say so in one line. Don't give dates.

## Page format

- Each page starts with YAML front matter:

  ```yaml
  ---
  title: Servers
  description: One sentence on what the page covers.
  sidebar:
    order: 70
  ---
  ```

  `sidebar.order` is an integer that sets the page's place in the site's
  menu. Leave gaps of 10 so a page can go between two others.
- No H1 in the body. The site shows the title. Start sections at `##`.
- Link to other guide pages with relative paths: `[Servers](./servers.md)`.
- Don't link to the internal docs in `docs/` (the UI spec, design notes,
  design gaps, roadmap) or to source files. Those are for people working on
  the app, and they change without notice.
- Link to [EndeavorMCP's README](https://github.com/jowch/EndeavorMCP#readme)
  for the notebook tools used without the app, rather than repeating it.
- Pages must also read well on GitHub, so use only standard Markdown.

## Images

- Images go in `images/` and are referenced relatively:
  `![Alt text that says what the picture shows](images/approval-card.png)`.
- [images/SHOTS.md](images/SHOTS.md) lists every screenshot the guide uses,
  with what must be on screen and where each one goes in the pages.
- Reference an image only once its file is in `images/`. The documentation
  site's build fails on a missing image. Until then, keep its line in
  SHOTS.md.
- Keep the number of screenshots small. Add one only where a reader would be
  lost without it.

## Keeping it true

When a change in the app renames a label or changes a behavior that a page
describes, update the page in the same change. Search this folder for the
old label.
