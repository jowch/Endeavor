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
- [x] Where it runs: This Mac, servers and clusters as separate kinds (they're set up differently and choosing one asks different questions); "Add server…" and "Add cluster…" end their sections; each host has a gear on hover that opens its settings (for a cluster: connection, scheduler, account, how to get Julia, default resources, idle stop). Picking a cluster adds a resources chip that overrides the defaults for that session (partition, CPUs, memory, time limit, presets, paste an `salloc` line). Starting shows the step log only, no progress bar — missing. Built: This Mac, Servers and Clusters in the menu with connection state, the server and cluster dialogs (Test connection lists a cluster's partitions), connecting on pick with an in-app folder browser, the resources chip with its popover, the starting step log (on a cluster: submitted job, waiting for a node with Slurm's reason, Cancel), and the per-host list in Settings ("Where notebooks run": each host's state, such as "Running · job 15 ends 22:31" or "Queued · job 16", with Stop after a confirmation, Connect or Check, and its gear) plus a dot on hosts running Julia in the Where menu
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
- [x] Window minimum size — missing (the window can shrink with no floor). Built: 862 × 600, every column at its minimum (sidebar 180, chat 320, notebook 360); the sidebar and chat shrink to their minimums before the notebook does
- [ ] Julia picker's prompt text ("Use this julia") in Settings — improvised
- [ ] Offline: no network shows as a generic setup failure; losing it mid-session has no state — missing

## Sidebar

- [ ] Session row states: active, working dot, needs-approval dot — improvised
- [x] Row actions: inline rename, hover ×, "Delete?" confirm — improvised. The hover × is too easy to hit by accident; replace it with a ⋮ button that opens a context menu, also on right-click, like Claude desktop. The menu is where session actions live as they're added; candidates: Rename, Pin, Reveal folder in Finder, Open notebook, Resume in Claude Code, Export transcript, Delete… (last, separated)
- [ ] "Show N more / Show fewer", folder headings, status line ("Claude connected."), settings gear — improvised
- [x] Long sidebar status lines overflow into the chat pane and can cover the settings gear — bug. Fixed: the line truncates, and hovering shows the full text wrapped
- [ ] Empty sidebar (no sessions yet) — missing
- [x] Session ⋮ menu keyboard shortcuts: while the menu is open, single keys pick an item (R Rename, P Pin, F Reveal folder, A Archive, ⌫ Delete…), shown on the right of each row — missing. Built with C for Close; P waits for Pin
- [x] Archive: an Archive item in the session ⋮ menu and a sidebar view filter (Active / All, including archived; archived rows dimmed) — missing

## Chat transcript

- [ ] Markdown in agent replies: code blocks, tables, headings, lists — improvised (component library defaults)
- [ ] "▸ Thinking" blocks — improvised
- [ ] Non-notebook tool calls (shell, Read, Write, ToolSearch) and their expanded input/output view — improvised (raw JSON)
- [x] Shell tool lines print the whole command (multi-line scripts) in the transcript; spec says one collapsed line each (grey verb, mono name, `›` to expand), like "Ran `python3 -c …`" — improvised
- [x] A run of tool calls between two agent messages folds into one line ("Used 5 tools ›": list_folder, read_file, run_shell…) that expands to the individual lines; the transcript should read as the agent's replies, as with folding long user messages. The expanded tool view should be tighter too (less padding, smaller input/output panels). Reference (Claude desktop, 2026-09-26): the folded line summarises by kind in plain words ("Ran 2 commands, finished a background command ›"; counts with failures in red, "22 background commands completed, 1 failed"); expanded, it becomes one bordered list with a row per call showing its description ("Added the tool-folding item to design gaps ›"); a row expands in place to the input in a code block, then the output as plain mono text under it — missing. Built: "Read 2 files, listed a folder, ran a command ›" with "1 failed" in red; notebook calls in plain words too ("edited 2 cells, ran a cell"); the thinking between calls folds with them; a lone call stays its own line; a run still going shows its latest call under the line and folds when the next agent message comes; approval cards stay above the composer, and the answer shows on the call's row inside the run ("· allowed", "· ran without asking", "denied"; "1 denied" in the folded line) instead of a note that splits it
- [ ] Transcript notes: "Turn ended", end-of-turn run warnings, and "Allowed: …" / "Denied: …" for a prompt with no call in the transcript (plain words, "Allowed: edit a cell") — improvised
- [ ] Failure states: turn failed, agent disconnected, Julia crashed and notebooks reopened, "Open a copy" for a session still open in the Claude Code CLI — improvised
- [ ] Opening a past session: loading and replay — improvised
- [x] Messages sent from the notebook ("✎ 1 cell: …") and the "📎 error message / selected text" attachment chip — improvised. Built: Fix with Claude, Explain, ⌘K, the selection chip and Point send the user's words with their chips (`rates`, "error in `bad`", "selection · 2 lines", "3 cells"); no "From the notebook" line
- [x] Attachments across the chat: how cells, selected text, error messages, picked regions (and later files and images from "+") look in the user bubble, the composer before sending, and queued messages; what clicking one does (jump to the cell, show the attached text) — missing. Built: 22px chips (icon, mono names, a red icon for errors); in the box above the text with × and a hover preview (code, text, image thumbnail, file size); above the sent bubble without ×; first on a queued message's one line. A cell chip scrolls the notebook to its cells and outlines them for a second; a selection, error or file chip opens what was sent, with "Show in notebook" and "The cell has changed since." when it has. A reopened session gets all its chips back as sent, from the prompt blocks its history keeps (each notebook attachment is one tagged `<attached>` block with the cells' code as sent; a region's picture is the image after it), popovers and "The cell has changed since." included. A region chip (Point's drawn box) shows its picture on hover and scrolls to its cells on click
- [ ] Working indicator: spec says orange asterisk + "Adding `residuals` · 12s"; app shows a rocket + "Working · 24s" — improvised
- [x] Long content: tool output and long messages have no max height or scroll (only the plan card does) — missing. Built: a call's input and output scroll past 160px (agent replies are always shown in full); the wheel goes back to the transcript at a panel's end
- [x] Long user messages: fold after about 10 lines with the last lines fading out and "Show more" ("Show less" once open), so the transcript shows mostly Claude's replies. The user wrote it and rarely needs to reread it all

## Cards above the composer

- [ ] Run-card variants: "Delete `x`?", "Run all N cells?", "Run code?" (no cell preview), "Let this notebook run? (Nothing runs yet.)", "Add a cell and run it?" (spec shows only "Run 3 cells?") — improvised
- [ ] The agent's own permission prompts (e.g. "Allow list_notebooks?" with Yes / Yes, don't ask again / No) — improvised
- [ ] Queued messages with ✎ edit and ✕ remove — improvised. Now one bordered line each, chips first and the words cut to fit; ✎ puts both back in the composer
- [ ] Plan card: spec shows numbered steps; the app renders the plan's markdown, headings included, under a "Ready to start?" heading the spec doesn't have — improvised

## Composer

- [x] Model and effort pickers — improvised. Built: the same menus on the new-session screen, from the choices the last session offered; a pick applies to the session and to the next ones
- [ ] Slash-command menu — improvised
- [ ] Context ring hover label ("N% context") — improvised
- [x] "+" button (attach / @ cell): its menu — missing. Built: "Add files or photos ⌘U" (the file picker, several at once; also drop or paste into the box) and "Slash commands". Images and text files go in the prompt itself (images up to 3.7 MB, text up to 250 KB), the same on This Mac and servers; other files are refused with a line saying why. Cells come only from Point, Fix with Claude, ⌘K and the selection chip. Typing @ lists the session folder's files and folders (fuzzy, four levels, hidden ones skipped; on a server through its helper) and puts the path in the text as one tinted piece that one backspace deletes; Claude gets the path relative to the folder and reads it itself. Notebooks are files here, so "Refer to another notebook" is this too
- [ ] Steering feedback (a message joining the running turn) — missing
- [ ] The new-session screen's composer doesn't match the chat composer. It needn't be identical, but it should mostly be the chat composer's refined design, with the new-session parts added (the Where / folder / notebook chips above it). Reference (Claude desktop, 2026-09-27): its new-session composer keeps the chat's elements (chips above; the input; "+", model and effort under it) and adds only what starting needs. Keeping the orange send button is optional — improvised. Built: one composer for both, the new-session screen adding only its chips above; the row under the box is the same in both ("+", Point, mode; model, effort, context ring), with Point greyed until a notebook is open and the ring greyed until there is usage, so nothing moves when a session starts. The box is one line in a chat and two on the new-session screen, growing to ten. The send button is orange only with something to send, grey when empty, and Stop (Esc) while Claude works with an empty box. The mode opens a menu (Manual, Ask to run, Auto, Plan, each with a line on what it allows, ✓ on the current one, 1–4 while open); ⇧⇥ still cycles, and the new-session screen's pick applies when the session starts

## Notebook pane

- [ ] Pluto's welcome page when no notebook is open, and its "Can't find a file here" page — missing (Pluto's own)
- [ ] Pluto's header: save box, "Run notebook code", Safe preview pill (spec says hide; undecided) — missing
- [ ] Safe preview in Endeavor's own terms — missing
- [ ] Live docs and Status panels (only recoloured) — missing
- [ ] ⌘K prompt box — improvised
- [ ] Selection chip and quote prompt ("✦ Ask Claude") — improvised
- [ ] Agent button between cells — improvised
- [ ] Fix with Claude / Explain at the bottom of error cards — improvised
- [ ] Pointing mode's comment bar under the picked cell — improvised. Built: the hint pill "Click a cell or drag a box · Done" and the drawn box (dashed; its cells are picked and the bar sits under it). The box goes as a picture of that part of the notebook, taken by WebKit's own snapshot of the web view (macOS; on Linux it goes as its cells for now)
- [ ] Folded cell shown open until it runs — improvised

## Accessibility

- [ ] VoiceOver labels for icon-only controls (×, ✦, point, gear) — missing
- [ ] Keyboard focus styles across sidebar, cards and composer — missing

## Spec'd but not built yet

- [ ] Session menu (title ⌄)
- [ ] End-of-turn changed-cells card
- [ ] Copy / pin / time under each message
- [ ] Translucent headers with blur
