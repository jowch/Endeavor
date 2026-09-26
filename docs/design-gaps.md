# Design gaps

Surfaces and components the app has that [ui-spec.md](ui-spec.md) doesn't design
yet. **Improvised**: built without a design, works but needs a proper look.
**Missing**: doesn't exist yet.

_Listed 2026-09-25._

## Whole-app screens

- [ ] First launch: splash, setup steps and progress bar, error and Retry, "Show logs" — improvised (🚀 stands in for the logo)
- [ ] Logo and app icon — missing
- [ ] Sign-in panel: subscription / console buttons, waiting state, errors — improvised
- [ ] Settings screen: Claude setup, custom Julia path, run without asking, appearance, notebook theme, troubleshooting — improvised
- [ ] New-session screen: working folder, Choose…, recent folders, optional first message, Start session — improvised
- [ ] Endeavor light mode — missing (planned for later)
- [ ] About window and update notices (adapter updates only show in the status line) — missing

## Sidebar

- [ ] Session row states: active, working dot, needs-approval dot — improvised
- [ ] Row actions: inline rename, hover ×, "Delete?" confirm — improvised
- [ ] "Show N more / Show fewer", folder headings, status line ("Claude connected."), settings gear — improvised
- [ ] Empty sidebar (no sessions yet) — missing

## Chat transcript

- [ ] Markdown in agent replies: code blocks, tables, headings, lists — improvised (component library defaults)
- [ ] "▸ Thinking" blocks — improvised
- [ ] Non-notebook tool calls (shell, Read, Write, ToolSearch) and their expanded input/output view — improvised (raw JSON)
- [ ] Transcript notes: "Allowed: …", "Denied: …", "Turn ended", end-of-turn run warnings — improvised
- [ ] Failure states: turn failed, agent disconnected, Julia crashed and notebooks reopened, "Open a copy" for a session still open in the Claude Code CLI — improvised
- [ ] Opening a past session: loading and replay — improvised
- [ ] Messages sent from the notebook ("✎ 1 cell: …") and the "📎 error message / selected text" attachment chip — improvised
- [ ] Working indicator: spec says orange asterisk + "Adding `residuals` · 12s"; app shows a rocket + "Working · 24s" — improvised

## Cards above the composer

- [ ] Run-card variants: "Delete `x`?", "Run all N cells?", "Let this notebook run? (Nothing runs yet.)", "Add a cell and run it?" (spec shows only "Run 3 cells?") — improvised
- [ ] The agent's own permission prompts (e.g. "Allow list_notebooks?" with Yes / Yes, don't ask again / No) — improvised
- [ ] Queued messages with ✎ edit and ✕ remove — improvised
- [ ] Plan card: spec shows numbered steps; the app renders the plan's markdown, headings included — improvised

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

## Spec'd but not built yet

- [ ] Session menu (title ⌄)
- [ ] End-of-turn changed-cells card
- [ ] Copy / pin / time under each message
- [ ] Translucent headers with blur
