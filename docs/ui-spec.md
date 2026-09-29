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
- **Per-column 44px headers**, dividers aligned: sidebar = traffic lights + sidebar toggle; chat = session title ⌄ (session menu) + folder tag; notebook = filename. No horizontal rules — headers are translucent with backdrop blur and a ~20px fade so content scrolls softly under them. When the sidebar is collapsed, traffic lights + toggle move to the start of the chat header.
- Webview sits above GPUI, so the notebook header's blur needs the webview to extend under the header (theme CSS adds top padding to Pluto's page).
- Risk: confirm GPUI can do backdrop blur; fallback is the gradient fade alone.

## Chat

- **Transcript is top-anchored**: messages start at the top and fill down; stick-to-bottom only once it overflows, and only while the user is at the bottom.
- **User message**: right-aligned bubble, `bg.raised`, 8px radius, max ~300px.
- **Tool calls**: collapsed by default, one line each — grey verb, light mono cell name, ± counts, `›` to expand (expanded shows the diff).
- **End of turn**: a card listing cells changed this turn (cell icon, name, `new` tag, ±, `›`); a row click scrolls the notebook to the cell and outlines it. Under the message: copy, pin, relative time.
- **Mid-turn**: working indicator (orange asterisk + "Adding `residuals` · 12s"). The agent's plan checklist (✓ done struck through, ◐ current bright, ○ upcoming grey; "Progress · 1 of 3", collapsible) is **pinned above the composer** while the turn runs, then folds back into the transcript.
- **Approval card** (above composer): "Run 3 cells?", cell list, "also re-runs N that depend on them"; Deny · Always this session · **Run**. Keys: ⏎ run, ⌘⏎ always, Esc deny.
- **Plan card** (plan mode): numbered steps; Keep planning · Start in Auto · **Start**.
- **"You edited `x`"** lines record user actions in the notebook (the agent receives them too).

## Composer

- One line, 38px, `bg.card`, 1px `border`, 8px radius, 10px side padding. Placeholder "Type / for commands"; a faint ↵ glyph instead of a send button (Enter sends). While running: placeholder "Queue a message, or ⌘⏎ to steer" and the glyph becomes a small stop button (Esc).
- Toolbar **below** the box, 24px buttons, 12px text, centre level with the sidebar settings gear: `+` (attach / @ cell) · point (⌘⇧K, orange when active) · mode — then model · effort · context ring.
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
- **Empty cell**: hint "Type code, or ⌘K to ask <agent>"; ⌘K turns it into a prompt; Esc returns to typing.
- **Errors**: Pluto's error card gains **Fix with <agent>** (outlined, not filled) and **Explain**, replacing Pluto's "Fix with AI".
- **Pointing overlay** (⌘⇧K): over the notebook only — 25% dim, solid 1px `accent` inset edge (no glow), plain-text hint pill "Click a cell or drag a box · Done", dashed hover outline, solid selection outline, dashed box for a drawn region, prompt anchored under the selection with a single **Send**. Sends immediately (no tray).

## Build map

| Layer | Pieces |
|---|---|
| Native GPUI | theme module, sidebar, headers, chat entries, cards, composer + toolbar, pane resizing, keyboard shortcuts |
| Pluto theme CSS | variable overrides, hidden chrome, `data-endeavor` state styles, header padding |
| Adapter JS (per notebook type, ~50 lines) | `cellAt(x,y)`, `cellRect(id)`, `scrollTo(id)`, `focusedCell()`, `mark(id, state)` |
| Injected bundle (notebook-agnostic where possible) | gutter diffs (CodeMirror decorations), pointing overlay, ⌘K cell prompt, "+"/agent button, overview rail, user-edit reporting |
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

## Shortcuts

Enter send · ⌘⏎ steer running turn · Esc stop / deny · ⇧⇥ cycle mode · ⌘B sidebar · ⌘⇧K pointing · ⌘K ask about focused/empty cell · ⌘N new session.

## Open decisions

- Chat-panel radius: 8px scale (boards) vs Pluto's 4px everywhere.
- Undo for agent edits (per change / per cell) — explored, not in the final states. PlutoMCP has no undo; it would restore the app's saved before-text.

## Suggested phases

1. Theme module + native restyle (sidebar, headers, chat, composer, cards, resizing).
2. Pluto theme CSS + adapter + `data-endeavor` states + overview rail.
3. Bundle: gutter diffs, pointing overlay restyle, ⌘K cell prompt, "+"/agent button, user-edit reporting.
4. ACP wiring: modes, models/effort, usage ring, slash commands, plan pinning.

Verify each phase by running the app (`cargo run`) and comparing against the matching board.
