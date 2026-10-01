# Endeavor UI spec — main interface

Handoff for implementing the redesigned chat + notebook interface.

- **Target states (source of truth for look):** https://claude.ai/artifact/Y4NunVg55n9zNtL2oz7bUV — nine 1440×900 boards. Read exact values from the boards' `.dc.html` sources; do not port the markup.
- **Exploration history (why decisions were made):** https://claude.ai/artifact/Ws1rn2Dyuo9Q2cqQBZD2VA
- Both canvases are private until shared from their Share menu.

## Principles

1. **You co-author the notebook.** Hands-on users and delegate-everything users both feel at home; the notebook invites direct editing.
2. **Visual weight follows urgency.**
   | Tier | What | Treatment |
   |---|---|---|
   | Needs you now | Run / plan approvals | The only heavy element: orange edge + soft ring, filled orange primary |
   | Look when you like | Agent-edited, unrun cells | Gutter stripe only — no badges, no header text |
   | Where you read | Chat messages | Brightest text, 14px |
   | Ambient | Sidebar, composer, controls | Smaller, grey, no emphasis |
3. **Native by default, CSS where needed, a thin adapter for the rest.** Every line injected into the notebook is maintained twice (Pluto, later Jupyter).
4. **Agent-agnostic.** Every "Claude" label comes from the ACP agent's name.

## Tokens

Replace the inline `rgb()` literals in `src/main.rs`, `src/session.rs`, `src/splash.rs` with one theme module.

**Colour.** Dark and light, from Settings → Appearance (Dark, Light, or Match macOS, which follows the Mac's setting while the app runs). The whole window changes, notebook included. Neutrals carry a faint cool tint. `src/theme.rs` holds every value; the frontend's copies are CSS variables in `frontend/src/theme.ts`. The setup window's night sky stays dark as a band at the top in both.

| Token | Dark | Light | Use |
|---|---|---|---|
| `bg_sidebar` | `#111113` | `#F3F3F5` | Sessions sidebar |
| `bg_page` | `#151517` | `#FCFCFD` | Chat, notebook, headers |
| `bg_card` | `#1C1C1F` | `#F4F4F6` | Cards, inputs |
| `bg_raised` | `#26262A` | `#EAEAED` | User bubble, secondary buttons |
| `row_active` | `#1E1E21` | `#E6E6EA` | Active sidebar row |
| `bg_urgent` | `#1F1712` | `#FCF1EB` | Approval and plan cards |
| `bg_tag` | `#222225` | `#ECECEF` | Tags, inline code |
| `border` | `#2A2A2E` | `#E1E1E6` | Card outlines (decorative) |
| `divider` | `#1F1F22` | `#E6E6EA` | Column dividers |
| `composer_bg` | `#1C1C1F` | `#FFFFFF` | Message box; light adds a faint shadow (0 1px 3px) |
| `composer_edge` / focused | `#3A3A40` / `#55555C` | `#D6D6DC` / `#8A8A92` | Message box outline |
| `control_edge` | `#3A3A40` | `#8A8A92` | Outlined buttons, text fields (3:1 in light) |
| `popover_bg` / `popover_edge` | `#26262A` / `#3A3A40` | `#FFFFFF` / `#D9D9DF` | Menus, popovers, dialogs, with a soft shadow |
| `scrim` | `rgba(8,8,10,.62)` | `rgba(24,24,30,.32)` | Behind Settings and dialogs |
| `text_primary` | `#ECECEC` | `#1B1B1F` | Chat body |
| `text_secondary` | `#BDBDBD` | `#45454C` | Cell names, model/effort |
| `text_muted` | `#8C8C8C` | `#5C5C64` | Sidebar meta, status line |
| `text_row` | `#BDBDBD` | `#5C5C64` | Sidebar session rows |
| `text_faint` | `#858585` | `#66666E` | Tool lines, timestamps |
| `text_section` | `#888888` | `#6B6B73` | Section heads |
| `accent` | `#CC3F00` | `#CC3F00` | Filled primary (white text, 4.9:1), unrun stripe, busy dot |
| `accent_text` | `#E08A5E` | `#B23600` | Orange text and icons |
| `focus_ring` | `#E08A5E` | `#CC3F00` | Keyboard focus: a 2 px ring 2 px outside a button, with a gap; Outlined buttons, rows and fields recolour their own outline instead |
| `diff_add` / `diff_del` | `#6CC784` / `#E07A7A` | `#1C7038` / `#B42A36` | ± counts, gutter signs; line tints ~12% |
| `danger` | = `diff_del` | = `diff_del` | Errors |

Every light text colour passes WCAG AA on the surfaces it sits on.

**Type** — Geist (UI), Geist Mono (cell names, paths, counts). Sizes: chat 14px/1.6; tool lines 13px; sidebar 12.5px; section heads 11.5px; toolbar 12px. The notebook keeps Pluto's own fonts (Vollkorn, Alegreya Sans, JuliaMono).

**Space** — 16px inset from every column edge (headers, transcript, composer, sidebar rows); 10px inside cards; 8px between related items, 16px between blocks. Column headers 44px.

**Radius** — cards, composer, bubbles 8px; buttons, menus 4–5px; chips, inline code 3px; gutter bars 2px; circles stay round. *(Open: Pluto's 4px everywhere in the chat panel.)*

## Layout

- Three columns: sidebar · chat · notebook. **All resizable** by dragging dividers; sidebar within min/max and collapsible (⌘B). No full-width toolbar and no "focus" mode — full-width notebook comes from collapsing panes.
- **Per-column 44px headers**, dividers aligned: sidebar = traffic lights + sidebar toggle; chat = session title ⌄ (session menu: the sidebar row's ⋮ items for the open session; its Rename edits the title in place) + folder tag; notebook = filename. No horizontal rules — headers are translucent with backdrop blur and a ~20px fade so content scrolls softly under them. When the sidebar is collapsed, traffic lights + toggle move to the start of the chat header.
- **The sidebar's past sessions show at launch**, from Endeavor's own record of its sessions, without waiting for Julia or Claude. Claude starts at launch alongside Julia; its listing then updates titles and times, adds sessions started elsewhere (such as the Claude Code CLI) in the listed folders, and drops ones deleted elsewhere.
- Webview sits above GPUI, so the notebook header's blur needs the webview to extend under the header (theme CSS adds top padding to Pluto's page).
- Risk: confirm GPUI can do backdrop blur; fallback is the gradient fade alone.

## Chat

- **Transcript is top-anchored**: messages start at the top and fill down; stick-to-bottom only once it overflows, and only while the user is at the bottom.
- **User message**: right-aligned bubble, `bg.raised`, 8px radius, max ~300px.
- **Tool calls**: collapsed by default, one line each — grey verb, light mono cell name, ± counts, `›` to expand (expanded shows the diff).
- **End of turn**: a card listing cells changed this turn (cell icon, name, `new` tag, ±, `›`); a row click scrolls the notebook to the cell and outlines it. Each cell is one row with its net ± over the turn; a deleted cell gets a `deleted` tag and no `›`; a cell left as it was, or added and deleted again, isn't listed; no card when no cell changed. Under the message: copy, pin, relative time.
- **Mid-turn**: working indicator (orange asterisk + "Adding `residuals` · 12s"). The agent's plan checklist (✓ done struck through, ◐ current bright, ○ upcoming grey; "Progress · 1 of 3", collapsible) is **pinned above the composer** while the turn runs, then folds back into the transcript.
- **Approval card** (above composer): "Run 3 cells?", cell list, "also re-runs N that depend on them"; Deny · Always this session · **Run**. Keys: ⏎ run, ⌘⏎ always, Esc deny.
- **Plan card** (plan mode): numbered steps; Keep planning · Start in Auto · **Start**.
- **"You edited `x`"** lines record user actions in the notebook (the agent receives them too).
- **Reply** (boards: QQuote, QCompose): selecting text in a reply, or in a cell's code, output or rendered Markdown, shows a "Reply ⌘J" pill 6px under the selection's end (above it with no room below); it replaces the notebook's old "✦ Ask Claude" chip. The pill or ⌘J opens a small prompt at the selection: the quote on one faint line with a left rule (in a cell, also the cell and lines, "rates · lines 3–5"), a "Reply to Claude" field, and a button at its end whose menu has **Send reply ↩** and **Add to message ⌘↩**. ↩ sends the quote and the reply now as their own message (queued while Claude works); ⌘↩ adds them to the composer as a card and says "Added" briefly; Esc closes the prompt.
- **Sent quotes** show in the message's bubble before its words: a quote of a reply as a blockquote, a notebook quote as a chip (a click shows the cell) and its lines or thumbnail; each followed by its comment, with a hairline between quotes. Claude gets a reply quote as `>` lines ending `> — Claude's reply, 14:02`, then the comment; a notebook quote as `<quote cell="rates" lines="3-5" uri="notebook://pluto/…">…</quote>` (or `part="output"`, `part="figure"` with the PNG as the next block, `part="box"`), then the comment.

## Composer

- One line, 38px, `bg.card`, 1px `border`, 8px radius, 10px side padding. Placeholder "Type / for commands"; a faint ↵ glyph instead of a send button (Enter sends). While running: placeholder "Queue a message, or ⌘⏎ to steer" and the glyph becomes a small stop button (Esc).
- **Quote cards** stack above the box, in the order added, under "N quotes go out with this message · Clear": each has its source ("Claude's reply · 14:02", "rates · lines 3–5", "plot_fit · figure"), the excerpt (one line, or up to three numbered code lines) or a thumbnail, the comment, "show in notebook" for a notebook quote, and ×. From four cards on, each takes one line (source and comment). A message can go with cards and no text. In the composer ⌘⏎ still means send now / steer.
- Toolbar **below** the box, 24px buttons, 12px text, centre level with the sidebar settings gear: `+` (attach / @ cell) · point (⌘⇧E, orange when active) · mode — then model · effort · context ring.
- **Modes** (⇧⇥ cycles), mirroring Claude Code:
  | Mode | Notebook edits | Runs / deletes |
  |---|---|---|
  | Plan | none (read-only) | none; ends with a plan to approve |
  | Ask to run (default) | land live | ask first |
  | Auto | land live | no asking |

## Notebook (Pluto)

- Real Pluto 1.0.3 frontend, in the app's appearance: the web view's own appearance is set to the resolved light or dark, and the Endeavor look maps Pluto's colour variables to the tokens in both.
- **Theme CSS** (per notebook type): override Pluto's colour variables to the tokens; hide Pluto's header, footer and Live Docs/Status panel (via CSS, **not** `?disable_ui`, which makes it read-only); hide the "…" menu's "Ask AI" item. Keep Pluto's own cell anatomy, run tab, eye, "+".
- **Cell states** via `data-endeavor` + `data-author` attributes, styled by the theme CSS:
  - Edited, not run: striped 4px gutter bar (`accent` for agent, `you.stripe` for user); stale output at 40% opacity. No text.
  - Changed lines: **unified diff in the CodeMirror gutter** — line number + ± sign, line tint, changed-character highlight, removed lines shown above additions (bundle decorations).
  - A 3px overview rail on the notebook's right edge marks changed/unrun cells, including off-screen; click jumps.
- **Between cells**: Pluto's "+" unchanged; an agent button beside it on hover ("Ask <agent> to write a cell here").
- **Empty cell**: hint "Type code, or ⌘E to ask <agent>"; ⌘E turns it into a prompt; Esc returns to typing.
- **Errors**: Pluto's error card gains **Fix with <agent>** (outlined, not filled) and **Explain**, replacing Pluto's "Fix with AI".
- **Pointing overlay** (⌘⇧E; board QPoint): over the notebook only — 25% dim, solid 1px `accent` inset edge (no glow), plain-text hint pill "Click to pick · drag over code lines · drag elsewhere for a box · Done". Hover picks the smallest thing under the pointer, outlined dashed with a small tag: a figure or output ("Figure", "Text"), a code block ("Code"), a Markdown paragraph ("Text"); the cell's edge picks the whole cell. A drag that starts on code picks whole lines; a drag anywhere else, or any ⌥-drag, draws a dashed box. Shift adds to the pick; each pick is its own quote. Line numbers show in code while Point is on; picked lines are tinted with orange numbers.
- **Point's comment bar**: the status line ("Lines 4–7 of plot_fit", "Figure in plot_fit", "Box over 2 cells", "3 picks") and "↩ send · ⌘↩ add to message" above the same field and menu as Reply's prompt. It opens 8px under the pick (for lines, the last picked line), left edge on the pick's; with no room below, 8px above; with no room either way, pinned to the pane's bottom. It follows scroll and waits at the pane's edge once the pick scrolls out. Width: the pick's, at least 360px, at most the pane minus 32px. ↩ sends the picks with the comment on the last; ⌘↩ adds them to the composer as cards. Point stays on either way.

## Build map

| Layer | Pieces |
|---|---|
| Native GPUI | theme module, sidebar, headers, chat entries, cards, composer + toolbar, pane resizing, keyboard shortcuts |
| Pluto theme CSS | variable overrides, hidden chrome, `data-endeavor` state styles, header padding |
| Adapter JS (per notebook type, ~50 lines) | `cellAt(x,y)`, `cellRect(id)`, `scrollTo(id)`, `focusedCell()`, `mark(id, state)` |
| Injected bundle (notebook-agnostic where possible) | gutter diffs (CodeMirror decorations), pointing overlay, ⌘E cell prompt, "+"/agent button, overview rail, user-edit reporting |
| Rust↔JS channel | today: two JS→Rust message types (`src/annotate.js`), `evaluate_script` fire-and-forget (`src/main.rs:676`). Needs a general two-way message channel. |

## Data wiring

| UI | Source |
|---|---|
| Mode switch, model, effort | ACP session modes / models — currently dropped (`src/session.rs:390` `_ => {}`; `session/new` response keeps only the id) |
| Context ring | ACP usage updates — dropped |
| "/" commands | ACP available commands — dropped |
| Progress checklist | ACP `plan` updates — already received and rendered |
| Changed-cells summary, diffs | `src/celldiff.rs`; keep per-cell before-text; aggregate per turn |
| Unrun stripe, stale output | Pluto's per-cell "code differs from last run" state, read by the bundle |
| "also re-runs N" | PlutoMCP `get_cell_dependents` |
| "You edited `x`" | bundle observes user edits/runs/moves → app → agent context |
| Agent names | ACP agent info |
| Cell labels | defined symbol (`find_symbol_definitions`) or first line — cells have only UUIDs |

## Failure states

Boards XTurn, XStopped, XOpen. Three tiers. **Waiting** (Endeavor fixes it by itself): a muted line above the composer, no red or orange, at most Try now. **Failed** (it needs the user): one card where the thing would have been, with a title saying what didn't happen, a plain reason, what is kept, one primary button, and the raw error under Details, closed; a danger-coloured warning icon. **Recovered**: one quiet note in the transcript. The sidebar's status line holds app-wide states only (offline, signed out, Restarting Claude… with a spinner, Claude isn't running, Julia on <computer> stopped), with a red dot only when something needs the user.

- **A turn Claude couldn't answer** (servers busy, other server errors): "Claude couldn't answer", the reason ("Anthropic's servers are busy right now."), "Your message is kept.", **Try again** sends the same message again without a second bubble, and the card goes. No automatic retry. If Claude can't be reached at all, that's offline (waiting).
- **Usage limit**: waiting. "You've reached your Claude usage limit. It resets in 1 h 12 min, at 3:00 PM, and your message sends then." The time comes from Claude Code's message ("resets 3pm (America/Los_Angeles)", "resets Oct 3, 3:30pm (…)"); the line counts down by the minute, by the second in the last minute, and at zero the held message goes. Without a time: "It resets later", with Try now. The message shows "Not answered yet"; new ones queue.
- **A reply that stops partway** (the connection dropped mid-reply, the length limit, too many steps): what came stays; "Claude stopped before finishing" with the reason and **Continue**, which sends "Continue from where you stopped." as a new message. "You stopped Claude" stays a quiet note.
- **Claude's process stops**: it restarts by itself ("Claude stopped unexpectedly. Restarting it…", with a spinner); sessions load again where they were and messages queue meanwhile. Only a session whose reply was cut off gets "Claude restarted. This reply was cut off." with Continue. A second stop within a minute leaves it stopped: "Claude isn't running" above the composer of whichever session is open, with **Restart Claude**, Show logs (the app's log, in Finder) and Details (the error and the log's last lines).
- **A session opened before its Julia is up** (a past session clicked, or a first message sent, at launch): waiting. "Starting Julia…" with a spinner above the composer, then it opens, and a message sent meanwhile goes.
- **Julia stops by itself**: the notebook pane says "Julia stopped unexpectedly": the file is saved, its outputs are gone until the cells run again, and, when known, which cell was running ("Very large data can run out of memory."). **Restart Julia**, Show log (This Mac's runtime log); the header chip says "Julia stopped". Restart reopens the notebook and runs it again; if Julia stops again during that run, the notebook opens in safe preview, and the callout says "Julia stopped again while running this notebook" and names the cell. A notebook's own Julia stopping is noticed whether or not its page is open and whether or not a cell was running, and a notebook tool call waiting on that run fails with the page's words ("Julia stopped unexpectedly while running `rates`. The notebook file is saved; its outputs are gone until the cells run again.").
- **A session that won't open**, in place of its transcript: open in the Claude Code CLI, "This session is open in Claude Code" with **Open a copy** and Try again (a message sent from the composer opens the copy first); otherwise "Couldn't open this session" with a plain reason when it can be told (history unreadable, folder gone, Claude not running), "The notebook and its file are fine." when true, **Try again** and Details.
- **One-off failures** (export, rename, move, run, restart, stop, new notebook, sign out): a notice under the control used, with its title ("Couldn't export fit_decay.html"), the reason, a way on (Export to…, Choose another name, Try again), Details and ×. It stays until closed or tried again. Run notebook (inside the page) and the pane's New notebook hang it at the top right of the pane; Sign out at the top of the window, over Settings.

## Shortcuts

Enter send · ⌘⏎ steer running turn · ⌘J reply to the selection (in its prompt and Point's bar, ⏎ send, ⌘⏎ add to message) · Esc stop / deny · ⇧⇥ cycle mode · ⌘B sidebar · ⌘⇧E pointing · ⌘E ask about focused/empty cell · ⌘N new session.

## Open decisions

- Chat-panel radius: 8px scale (boards) vs Pluto's 4px everywhere.
- Undo for agent edits (per change / per cell) — explored, not in the final states. PlutoMCP has no undo; it would restore the app's saved before-text.

## Suggested phases

1. Theme module + native restyle (sidebar, headers, chat, composer, cards, resizing).
2. Pluto theme CSS + adapter + `data-endeavor` states + overview rail.
3. Bundle: gutter diffs, pointing overlay restyle, ⌘E cell prompt, "+"/agent button, user-edit reporting.
4. ACP wiring: modes, models/effort, usage ring, slash commands, plan pinning.

Verify each phase by running the app (`cargo run`) and comparing against the matching board.
