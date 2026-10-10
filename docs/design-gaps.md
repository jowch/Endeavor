# Design gaps

Open gaps in the app's design: things that are missing, or built without a
design and still needing one. [ui-spec.md](ui-spec.md) is the design for what
is done. **Improvised**: built without a design, works but needs a proper
look. **Missing**: doesn't exist yet.

_Listed 2026-10-03._

## Whole-app screens

- [ ] App updates — missing. The app can't update itself or check for a
  newer version yet, so Check now and "up to date" are hidden. About's
  update row for a new Endeavor (Restart) is drawn but can't show until it
  can.

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
- [ ] Undoing Claude's edits — missing. Today the user can only change a cell back by hand or ask Claude to. In Pluto 1.0.3's own page (tried 2026-10-10 in Chromium, with an edit made the runtime's way), Ctrl+Z in a cell brings back the code from before Claude's edit: Pluto applies the runtime's edit as an ordinary editor change, so it joins the cell's undo history. It comes back only into the editor, unrun until the user runs the cell, and only if the page was open when the edit came; a page opened later has no such history. A cell Claude deletes can't come back this way: Ctrl+Z restores no deleted cell, and Pluto's own "Cell deleted (Undo)" bar shows only for a delete made in the page. Whether ⌘Z reaches the page in the app is what hasn't been tried: the macOS Edit menu has no Undo or Redo item (`main.rs`), so it depends on the web view getting the key. Since 2026-10-02 a denied run keeps the edit unrun, which makes undo more useful. Open questions (user, 2026-10-03):
  - **Edits that depend on each other.** Undoing one cell may break cells Claude changed to use it, so undo may have to cover a set of edits, such as a turn's, not one cell.
  - **Linking edits.** Can Endeavor record which edits belong together? The runtime already keeps each cell's code from before the agent's edit (`before`), and the end-of-turn changed-cells card lists a turn's cells, so that card is a likely place for the button.
  - **What the button does.** Put the earlier code back itself, or send Claude a message that the user wants those edits reverted, leaving Claude to work out the dependencies. The second felt odd but may be simpler.
  - Earlier exploration: [ui-spec.md](ui-spec.md) lists per-change and per-cell undo as explored and left out of the final states.

## Accessibility

- [ ] Plain text makes no accessibility node in GPUI, so a screen reader
  hears the buttons but not the words around them: the sign-in card's
  heading and explanation, setup's error, a message's text. Clickable text
  has a role and a name since
  [#53](https://github.com/jowch/Endeavor/issues/53).
- [ ] Most clickable text isn't a Tab stop, so it can be reached with a
  screen reader but not with the keyboard alone (the sign-in card's buttons,
  the chip menus' rows).
- [ ] On Windows, a sidebar folder heading and Settings' dropdowns say
  whether they're open, which makes AccessKit offer a screen reader Expand
  instead of Invoke. GPUI ignores Expand, so activating them from a screen
  reader may do nothing (inferred from AccessKit's code, not yet tried with
  Narrator or NVDA).
