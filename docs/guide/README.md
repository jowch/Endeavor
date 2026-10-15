---
title: About this folder
description: Conventions for writing the Endeavor user guide.
draft: true
---

This folder is the Endeavor user guide. To read it, start with
[What Endeavor is](./overview.md). The pages, in menu order:

1. [What Endeavor is](./overview.md)
2. [Install Endeavor](./install.md)
3. [Get started](./getting-started.md)
4. [Sessions, the chat and the notebook](./sessions.md)
5. [Modes and approvals](./modes-and-approvals.md)
6. [Safe preview](./safe-preview.md)
7. [Your files](./your-files.md)
8. [Servers](./servers.md)
9. [Clusters](./clusters.md)
10. [Working offline](./offline.md)
11. [Settings](./settings.md)
12. [Troubleshooting](./troubleshooting.md)
13. [Privacy and what stays on your computer](./privacy.md)

The rest of this page is for people who write the guide.

## How the site uses this folder

The documentation site in `site/` reads this folder when it builds, and
publishes every page here at <https://jowch.github.io/Endeavor/>, except the
pages for contributors. Those pages (this one and
[images/SHOTS.md](images/SHOTS.md)) have `draft: true` in their front matter,
and the site leaves out any page marked that way. Keep that marker on them.
How to run the site locally is in `docs/development.md`.

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
  roadmap) or to source files. Those are for people working on
  the app, and they change without notice.
- Link to [EndeavorMCP's README](https://github.com/jowch/EndeavorMCP#readme)
  for the notebook tools used without the app, rather than repeating it.
- Pages must also read well on GitHub, so use only standard Markdown. No
  site-only syntax, such as Starlight's `:::note` asides or MDX components.

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
