# Design gaps

Surfaces and components the app has that [ui-spec.md](ui-spec.md) doesn't design
yet. **Improvised**: built without a design, works but needs a proper look.
**Missing**: doesn't exist yet.

_Listed 2026-09-25._

## Whole-app screens

- [x] First launch: the turtle walks in and looks around; one progress line and a thin bar; on failure it tucks its head in, lists the steps, Retry and "Show logs"
- [ ] Logo and app icon — missing
- [ ] Sign-in panel: subscription / console buttons, waiting state, errors — improvised
- [ ] Settings screen: Claude setup, custom Julia path, run without asking, appearance, notebook theme, troubleshooting — improvised
- [x] New-session screen: working folder, Choose…, recent folders, optional first message, Start session — improvised; needs a rework. Direction chosen (2026-09-26): "Start a session" at the top with the few most recent sessions to resume; chips above the composer for where it runs, the folder and the notebook; the first message starts the session. The notebook pane is always there but shows no Pluto until the session starts: a quiet empty state for a new notebook, or a static safe preview of the chosen notebook's first cells. Julia may start early on this Mac; remote hosts connect only on send
- [ ] Where it runs: This Mac, servers and clusters as separate kinds (they're set up differently and choosing one asks different questions); "Add server…" and "Add cluster…" end their sections; each host has a gear on hover that opens its settings (for a cluster: connection, scheduler, account, how to get Julia, default resources, idle stop). Picking a cluster adds a resources chip that overrides the defaults for that session (partition, CPUs, memory, time limit, presets, paste an `salloc` line). Starting shows the step log only, no progress bar — missing. Built: This Mac, Servers and Clusters in the menu with connection state, the server and cluster dialogs (Test connection lists a cluster's partitions), connecting on pick with an in-app folder browser, the resources chip with its popover, the starting step log (on a cluster: submitted job, waiting for a node with Slurm's reason, Cancel); the per-host list is still missing
- [x] Folder picking for a new session: an in-app picker (search, recent folders, Browse… opening at ~/Documents/Endeavor) instead of going straight to the macOS open panel — missing
- [x] One notebook per session (decided 2026-09-26): no switcher; Claude reads other notebooks as plain files; a session started from an existing notebook keeps it, one started from "New notebook" may create exactly one, and opening another offers a new session instead; the notebook's ⋯ has Reveal in Finder, Open in a new session…, Stop notebook — missing. Built: the app binds each session to its notebook in the runtime (again after a Julia restart); until Claude creates the notebook the pane shows the resting turtle and the folder it goes in, never Pluto's start page; Stop leaves "`file` is stopped." with Start, which reopens it as it was (safe preview or running). The ⋯ menu shows over the notebook by cutting a hole in the web view
- [x] Opening notebooks: the notebook chip lists only notebooks inside the chosen folder (subfolders by relative path) plus "New notebook"; existing notebooks open in safe preview. Starting shows a slowly walking turtle, the current step with elapsed time, and a short step log — missing. Built, on This Mac and on servers (the step log shows while a host connects or starts Julia; a host that died or was taken over shows that with Start or Reconnect)
- [x] Several sessions on one notebook: the tools warn an agent when another session changed the notebook in the last two minutes, refuse the same cell until it's read again, and refuse a run while a cell it depends on (Pluto's dependency graph) has another session's unread changes; edits that don't conflict go through. Edits aren't queued. Two people sharing one notebook file is out of scope (file permissions cover it) — built
- [x] Notebook lifetime, idle stop: notebooks stop after an idle period with no tool calls on them, edits or runs through Pluto, or running cells (Settings → "Stop idle notebooks after": 12 hours, 24 hours, 48 hours by default, 1 week, Never); the agent's `keep_notebook_alive` tool keeps one running when the user asks; the ⋮ menu can stop them now. The idle stop applies everywhere, even with the app open, since notebooks can hold a lot of memory and get forgotten. Built: the runtime checks every 5 minutes, and the pane says "`file` stopped after N hours idle." with Start
- [x] Notebook lifetime, the rest: a cluster job's time limit also caps a notebook, and the cluster may end the job sooner (partition maximum, preemption), so show the job's real end time, warn before it, and bound the resources chip's time limit by the partition's maximum. Local notebooks quit with the app by default; a setting keeps them running in a background process the app reconnects to, like a remote host. Built: the notebook header shows "Job ends 18:40" from `squeue`'s time left, the chat warns 15 minutes before, a job that ends says why (time limit, preemption, cancelled, out of memory, from `sacct`), and the chip's time limit stops at the partition's maximum; Settings → "Keep notebooks running after Endeavor quits"
- [ ] Endeavor light mode — missing (planned for later)
- [ ] About window and update notices (adapter updates only show in the status line) — missing
- [ ] Menu bar: Endeavor (Settings…, Quit), Edit, View — improvised; Window and Help menus — missing
- [ ] Appearance "Light": native chrome stays dark while the notebook turns Pluto-light — improvised (mismatched until light mode exists)
- [ ] Window minimum size — missing (the window can shrink with no floor)
- [ ] Julia picker's prompt text ("Use this julia") in Settings — improvised
- [ ] Offline: no network shows as a generic setup failure; losing it mid-session has no state — missing

## Sidebar

- [ ] Session row states: active, working dot, needs-approval dot — improvised
- [x] Row actions: inline rename, hover ×, "Delete?" confirm — improvised. The hover × is too easy to hit by accident; replace it with a ⋮ button that opens a context menu, also on right-click, like Claude desktop. The menu is where session actions live as they're added; candidates: Rename, Pin, Reveal folder in Finder, Open notebook, Resume in Claude Code, Export transcript, Delete… (last, separated)
- [ ] "Show N more / Show fewer", folder headings, status line ("Claude connected."), settings gear — improvised
- [ ] Long sidebar status lines overflow into the chat pane and can cover the settings gear — bug
- [ ] Empty sidebar (no sessions yet) — missing
- [x] Session ⋮ menu keyboard shortcuts: while the menu is open, single keys pick an item (R Rename, P Pin, F Reveal folder, A Archive, ⌫ Delete…), shown on the right of each row — missing. Built with C for Close; P waits for Pin
- [x] Archive: an Archive item in the session ⋮ menu and a sidebar view filter (Active / All, including archived; archived rows dimmed) — missing

## Chat transcript

- [ ] Markdown in agent replies: code blocks, tables, headings, lists — improvised (component library defaults)
- [ ] "▸ Thinking" blocks — improvised
- [ ] Non-notebook tool calls (shell, Read, Write, ToolSearch) and their expanded input/output view — improvised (raw JSON)
- [x] Shell tool lines print the whole command (multi-line scripts) in the transcript; spec says one collapsed line each (grey verb, mono name, `›` to expand), like "Ran `python3 -c …`" — improvised
- [ ] A run of tool calls between two agent messages folds into one line ("Used 5 tools ›": list_folder, read_file, run_shell…) that expands to the individual lines; the transcript should read as the agent's replies, as with folding long user messages. The expanded tool view should be tighter too (less padding, smaller input/output panels). Reference (Claude desktop, 2026-09-26): the folded line summarises by kind in plain words ("Ran 2 commands, finished a background command ›"; counts with failures in red, "22 background commands completed, 1 failed"); expanded, it becomes one bordered list with a row per call showing its description ("Added the tool-folding item to design gaps ›"); a row expands in place to the input in a code block, then the output as plain mono text under it — missing
- [ ] Transcript notes: "Allowed: …", "Denied: …", "Turn ended", end-of-turn run warnings — improvised
- [ ] Failure states: turn failed, agent disconnected, Julia crashed and notebooks reopened, "Open a copy" for a session still open in the Claude Code CLI — improvised
- [ ] Opening a past session: loading and replay — improvised
- [ ] Messages sent from the notebook ("✎ 1 cell: …") and the "📎 error message / selected text" attachment chip — improvised
- [ ] Attachments across the chat: how cells, selected text, error messages, picked regions (and later files and images from "+") look in the user bubble, the composer before sending, and queued messages; what clicking one does (jump to the cell, show the attached text) — missing
- [ ] Working indicator: spec says orange asterisk + "Adding `residuals` · 12s"; app shows a rocket + "Working · 24s" — improvised
- [ ] Long content: tool output and long messages have no max height or scroll (only the plan card does) — missing
- [x] Long user messages: fold after about 10 lines with the last lines fading out and "Show more" ("Show less" once open), so the transcript shows mostly Claude's replies. The user wrote it and rarely needs to reread it all

## Cards above the composer

- [ ] Run-card variants: "Delete `x`?", "Run all N cells?", "Run code?" (no cell preview), "Let this notebook run? (Nothing runs yet.)", "Add a cell and run it?" (spec shows only "Run 3 cells?") — improvised
- [ ] The agent's own permission prompts (e.g. "Allow list_notebooks?" with Yes / Yes, don't ask again / No) — improvised
- [ ] Queued messages with ✎ edit and ✕ remove — improvised
- [ ] Plan card: spec shows numbered steps; the app renders the plan's markdown, headings included, under a "Ready to start?" heading the spec doesn't have — improvised

## Composer

- [ ] Model and effort pickers — improvised
- [ ] Slash-command menu — improvised
- [ ] Context ring hover label ("N% context") — improvised
- [ ] "+" button (attach / @ cell): its menu — missing
- [ ] Steering feedback (a message joining the running turn) — missing

## Notebook pane

- [ ] Pluto's welcome page when no notebook is open, and its "Can't find a file here" page — missing (Pluto's own)
- [ ] Pluto's header: save box, "Run notebook code", Safe preview pill (spec says hide; undecided) — missing
- [ ] Safe preview in Endeavor's own terms — missing
- [ ] Live docs and Status panels (only recoloured) — missing
- [ ] ⌘K prompt box — improvised
- [ ] Selection chip and quote prompt ("✦ Ask Claude") — improvised
- [ ] Agent button between cells — improvised
- [ ] Fix with Claude / Explain at the bottom of error cards — improvised
- [ ] Pointing mode's comment bar under the picked cell — improvised (spec's hint pill and drawn-box region not built)
- [ ] Folded cell shown open until it runs — improvised

## Accessibility

- [ ] VoiceOver labels for icon-only controls (×, ✦, point, gear) — missing
- [ ] Keyboard focus styles across sidebar, cards and composer — missing

## Spec'd but not built yet

- [ ] Session menu (title ⌄)
- [ ] End-of-turn changed-cells card
- [ ] Copy / pin / time under each message
- [ ] Translucent headers with blur
