# Design gaps

Open gaps in the app's design: things that are missing, or built without a
design and still needing one. [ui-spec.md](ui-spec.md) is the design for what
is done. **Improvised**: built without a design, works but needs a proper
look. **Missing**: doesn't exist yet.

_Listed 2026-10-03._

## Whole-app screens

- [ ] App updates — missing. The app can't update itself yet. About's
  update row for a new Endeavor (Restart) is drawn but can't show until it
  can.

## Sidebar

- [ ] "Show N more / Show fewer", the status line and the settings gear —
  improvised.

## Chat transcript

- [ ] Pin a message — deferred until wanted. As in Claude desktop, pinning a
  message would insert a chapter title where it was pinned and a bookmark
  mark in the chat's left margin; unpinning removes both. Copy and the time
  under each message are built.

## Cards above the composer

- [ ] A message Claude didn't answer while offline stays as its bubble,
  marked "Not answered yet". Board GQueue puts it at the head of the queue,
  marked "waits"; that part of the board isn't built.

## Notebook pane

- [ ] Point's drawn box goes as a picture of that part of the notebook only
  on macOS. On Linux and Windows it goes as its cells — missing.
- [ ] Find in the notebook shows no "3 of 7" count. WebKit's public find
  gives no count, so it needs our own search in the page — missing.
- [ ] Undoing Claude's edits — missing. Today the user can only change a cell back by hand or ask Claude to. Whether ⌘Z in a cell brings back the code from before Claude's edit is untested, since the edit arrives from the runtime, not from typing. Since 2026-10-02 a denied run keeps the edit unrun, which makes undo more useful. Open questions (user, 2026-10-03):
  - **Edits that depend on each other.** Undoing one cell may break cells Claude changed to use it, so undo may have to cover a set of edits, such as a turn's, not one cell.
  - **Linking edits.** Can Endeavor record which edits belong together? The runtime already keeps each cell's code from before the agent's edit (`before`), and the end-of-turn changed-cells card lists a turn's cells, so that card is a likely place for the button.
  - **What the button does.** Put the earlier code back itself, or send Claude a message that the user wants those edits reverted, leaving Claude to work out the dependencies. The second felt odd but may be simpler.
  - Earlier exploration: [ui-spec.md](ui-spec.md) lists per-change and per-cell undo as explored and left out of the final states.

## Accessibility

- [ ] The composer's text box shows its caret but no focus ring. GPUI 0.3.6
  has no public "does a descendant have keyboard focus" query, and sharing
  its internal `FocusHandle` with the box risks registering it twice as a
  Tab stop.
- [ ] Accessible names on text-only buttons. A button's visible text alone
  isn't an accessible name in GPUI; the new-session chips read as empty
  until given `.aria_label()`. The sweep covered icon-only controls; other
  text-only buttons may have the same problem.
