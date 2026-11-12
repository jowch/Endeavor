// Endeavor's script for the notebook view in Claude's desktop app (EndeavorMCP's
// views/notebook.html, which vendors the built dist/view.js): the Endeavor look
// and Point on Pluto's page, without the app. The view passes a host: Point's
// ↩ passes the picks and the comment to Claude's chat as the user's message
// (MCP Apps' ui/message), with any that ⌘↩ added first: ⌘↩ keeps them here, in
// the view, so they don't depend on what the host does with a second message.
// Claude's desktop app puts the message in its message box for the user to send
// (checked 2026-11-11). Context for later turns (ui/update-model-context) isn't
// used: the desktop app's Code tab gave it to Claude on no turn at all. Pictures of
// figures and boxes aren't taken here (the app takes them natively), so
// those picks name their cells and Claude reads the output with its tools.
// Unlike the app, Point ends once ↩'s message is on its way: the user sends it from Claude's
// box, where the view's keys don't reach. The view fills Claude's panel: Pluto's header, footer and
// floating tabs give way to one slim toolbar (the file, what Pluto is doing, Docs, Point). It also
// closes a completion list nobody uses and follows the cells Claude adds.

import { initAnnotate, pointState } from "./annotate";
import { initAskBox } from "./askbox";
import { setHost, type ToApp } from "./bridge";
import { cellName } from "./cellname";
import { mac, modHeld } from "./keys";
import type { Pick } from "./quote";
import { statusModel, type NotebookLike } from "./status";
import { initTheme } from "./theme";

/** What the view gives the page: Claude's chat. */
export type Host = {
  /** Whether the host takes messages from the view (`send`); without it there is no Point. */
  canSend: boolean;
  /** Pass `text` to the chat as the user's message (the host may wait for the user to send it). */
  send(text: string): Promise<void>;
  /** Open `url` in the user's browser (MCP Apps' ui/open-link; Claude's desktop app asks the user first). Without it, links stay as they are. */
  openLink?(url: string): Promise<void>;
};

/** How the quotes read. Keep in step with the app's attach::quote_note and quote_block (src/attach.rs). */
const FORMAT =
  "A quote's uri, notebook://pluto/{notebook_id}/cell/{cell_id}, names its cell (it is not a fetchable URL); `lines` are " +
  "line numbers in the cell's code, counting from 1. `part` says what was quoted when it isn't code: `output` (text from " +
  "the cell's output), `figure` (the cell's output, a picture) or `box` (a box the user drew over the notebook; `uri` lists " +
  "the cells under it). No picture comes with a figure or box here: read current code and outputs with the notebook MCP tools.";

/** What Claude is told once in a message that quotes the notebook. */
export const QUOTE_NOTE =
  "[Endeavor] The user quoted parts of the notebook in this message, each as a <quote> followed by their comment on it. " + FORMAT;

// Claude's desktop app adds the message at the end of its box, straight after anything the user typed,
// and trims its leading whitespace (probed 2026-11-12), so a visible mark starts it and its newlines
// survive. A dash read to Claude as the user's own text cut off; an arrow pointing down, with a word
// in the note, doesn't.
const LEAD = "\u2193";
const LEAD_NOTE = " A \u2193 just before this note only marks where the quotes start; it isn't part of the user's words.";

const uri = (notebook: string, cell: string) => `notebook://pluto/${notebook}/cell/${cell}`;

/** One pick as a `<quote>` (the app's attach::quote_block). */
export function quoteBlock(notebook: string, pick: Pick): string {
  if (pick.part === "box") return `<quote part="box" uri="${pick.cells.map((c) => uri(notebook, c)).join(" ")}"/>`;
  const at = `cell="${cellName(pick.code).replace(/"/g, "'")}"`;
  const where = uri(notebook, pick.cell);
  switch (pick.part) {
    case "cell":
      return `<quote ${at} uri="${where}">\n${pick.code}\n</quote>`;
    case "lines":
      return `<quote ${at} lines="${pick.lines[0]}-${pick.lines[1]}" uri="${where}">\n${pick.text}\n</quote>`;
    case "output":
      return `<quote ${at} part="output" uri="${where}">\n${pick.text}\n</quote>`;
    case "figure":
      return `<quote ${at} part="figure" uri="${where}"/>`;
  }
}

/** The quotes of one ↩ or ⌘↩: each pick, then the comment after the last. */
export function quoteText(notebook: string, picks: Pick[], comment: string): string {
  const blocks = picks.map((p) => quoteBlock(notebook, p));
  return comment ? `${blocks.join("\n\n")}\n${comment}` : blocks.join("\n\n");
}

/** The toolbar's height, without Claude's inset above it. */
const BAR = 36;

// The view fills Claude's panel edge to edge, as an app does, not a website in a box: Pluto's own
// website chrome (its header with the file box and Move, the share menu, the footer with Settings and
// FAQ, the floating Live docs and Status tabs) gives way to one slim toolbar: the file's name, what
// Pluto is doing, Docs and Point. Pluto's header stays only as the band it turns into when the
// notebook needs the user (Safe preview's Run notebook code, Restart, Julia stopped, Reconnecting),
// under the toolbar. (Claude's desktop app reports no safe-area insets in its panel, 2026-11-12.)
const chrome = `
  :root { --endeavor-bar: ${BAR}px; }
  body { padding-top: var(--endeavor-bar); box-sizing: border-box; }
  footer { display: none !important; }
  pluto-editor > header#pluto-nav { display: none !important; }
  pluto-editor:is(.process_waiting_for_permission, .process_dead, .nbpkg_restart_required, .nbpkg_restart_recommended, .disconnected) > header#pluto-nav {
    display: block !important; position: sticky; top: var(--endeavor-bar); z-index: 60; }
  pluto-editor > header#pluto-nav nav#at_the_top > :not(#process_status) { display: none !important; }
  pluto-editor > header#pluto-nav nav#at_the_top { justify-content: center; }
  pluto-editor > main { padding-top: 16px; }
  /* What Pluto keeps at the top as the page scrolls (its "Save all changes" button) and where a jump
     to a cell lands go under the toolbar, not behind it. */
  pluto-editor main > preamble { top: calc(var(--endeavor-bar) + 5px) !important; }
  html { scroll-padding-top: calc(var(--endeavor-bar) + 8px); }
  #endeavor-bar { position: fixed; top: 0; left: 0; right: 0; z-index: 9997; box-sizing: border-box; height: var(--endeavor-bar);
    display: flex; align-items: center; gap: 10px; padding: 0 10px 0 14px;
    background: var(--e-bg-page); border-bottom: 1px solid var(--e-divider); color: var(--e-text-secondary);
    font: 12.5px system-ui, -apple-system, "Segoe UI", sans-serif; }
  #endeavor-bar .file { font-weight: 600; color: var(--e-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  #endeavor-bar .doing { color: var(--e-text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 0 1 auto; }
  #endeavor-bar .grow { flex: 1; }
  #endeavor-bar button { flex: none; display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 8px; border: none; border-radius: 6px;
    background: none; color: var(--e-text-secondary); font: inherit; cursor: pointer; }
  #endeavor-bar button:hover { color: var(--e-text-primary); background: var(--e-button-hover); }
  #endeavor-bar button.on { color: var(--e-text-primary); background: var(--e-bg-raised); }
  #endeavor-bar .key { color: var(--e-text-faint); }
  /* Live docs: Pluto's own panel, as a sheet down the right side under the toolbar, opened from Docs.
     Not display: none when shut: Pluto alerts "window too small to show docs" whenever it opens a panel
     it finds undisplayed, and it hides the panel below 500px wide. Its Status tab isn't offered here. */
  #helpbox-wrapper { display: block !important; }
  pluto-helpbox.hidden, pluto-helpbox.helpbox-process { display: none !important; }
  pluto-helpbox { position: fixed !important; top: var(--endeavor-bar) !important; bottom: 0 !important;
    right: 0 !important; height: auto !important; width: min(380px, 100vw) !important;
    border-radius: 0 !important; border-left: 1px solid var(--e-divider); box-shadow: -8px 0 24px var(--e-shadow-popover) !important; }
  /* Its own tabs and close button: Docs in the toolbar opens and closes it. */
  pluto-helpbox > header { display: none !important; }
  /* A narrow panel: the name before the shortcut, and the sheet over the whole width. */
  @media (max-width: 420px) {
    #endeavor-bar .key { display: none; }
    pluto-helpbox { border-left: none; }
  }
  /* Pluto's floating safe-preview badge is hidden, as in the app: its band and each cell's
     "Code not executed in Safe preview" already say it. */
  .outline-frame-actions-container.safe-preview { display: none !important; }
`;

// Point, from the toolbar, and what it says once ↩ has passed the picks on.
const css = `
  #annotate-hint { top: calc(var(--endeavor-bar) + 8px) !important; }
  #endeavor-sent { position: fixed; left: 50%; top: calc(var(--endeavor-bar) + 8px); transform: translateX(-50%); z-index: 10001; display: none;
    max-width: calc(100vw - 32px); box-sizing: border-box; text-align: center;
    padding: 4px 12px; border-radius: 12px; background: var(--e-hint-bg); color: var(--e-hint-text); font: 12px system-ui; }
  #endeavor-sent.shown { display: block; }
`;

/** What Pluto is doing, in words, for the toolbar: nothing when it's idle. What needs the user (Safe preview, Restart, Julia stopped, Reconnecting) is Pluto's own band. */
export function doing(nb: NotebookLike | undefined): string {
  if (!nb?.cell_order) return "";
  const model = statusModel(nb);
  if (model.busy) return model.busy;
  if (model.saveFailed) return "Couldn't save";
  return "";
}

/** The file's name, without its folder. */
export const fileName = (path: string | undefined): string => (path ? path.split(/[\\/]/).pop() || "Notebook" : "Notebook");

const book = `<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><path d="M2.5 3.2c1.8-.6 3.7-.4 5.5.8 1.8-1.2 3.7-1.4 5.5-.8v9.6c-1.8-.6-3.7-.4-5.5.8-1.8-1.2-3.7-1.4-5.5-.8z"/><path d="M8 4v9.6"/></svg>`;

/** The toolbar: the file's name, what Pluto is doing, Docs, and Point when the chat takes messages. */
function initBar(withPoint: boolean, point: (on: boolean) => void): void {
  const style = document.createElement("style");
  style.id = "endeavor-chrome";
  style.textContent = chrome;
  document.head.append(style);
  const bar = document.createElement("div");
  bar.id = "endeavor-bar";
  bar.innerHTML =
    `<span class="file">Notebook</span><span class="doing">Loading…</span><span class="grow"></span>` +
    `<button type="button" class="docs" title="Live docs: what the code under the cursor does">${book}<span>Docs</span></button>` +
    (withPoint
      ? `<button type="button" id="endeavor-point" title="Point at a cell, its code or its output, and comment on it for Claude"><span>Point</span><span class="key">${mac ? "⌘⇧E" : "Ctrl+Shift+E"}</span></button>`
      : "");
  document.body.prepend(bar);
  const file = bar.querySelector<HTMLElement>(".file")!;
  const words = bar.querySelector<HTMLElement>(".doing")!;
  const docs = bar.querySelector<HTMLButtonElement>(".docs")!;
  const pointButton = bar.querySelector<HTMLButtonElement>("#endeavor-point");
  const docsOpen = () => !!document.querySelector("pluto-helpbox.helpbox-docs:not(.hidden)");
  docs.addEventListener("click", (e) => {
    if (!e.isTrusted) return;
    window.dispatchEvent(new CustomEvent("open_bottom_right_panel", { detail: docsOpen() ? null : "docs" }));
    // The search box takes the cursor once Pluto has drawn the panel (it does so on its next render).
    if (!docsOpen()) setTimeout(() => document.querySelector<HTMLInputElement>("#live-docs-search")?.focus(), 50);
  });
  pointButton?.addEventListener("click", (e) => e.isTrusted && point(!document.body.classList.contains("annotating")));
  let last: unknown = null;
  const tick = () => {
    const nb = (window as any).editor_state?.notebook as (NotebookLike & { path?: string }) | undefined;
    if (nb && nb !== last) {
      last = nb;
      file.textContent = fileName(nb.path);
      file.title = file.textContent;
      words.textContent = doing(nb);
    }
    docs.classList.toggle("on", docsOpen());
    pointButton?.classList.toggle("on", document.body.classList.contains("annotating"));
  };
  // Pluto redraws the page whenever its state changes (a cell runs, a panel opens): follow its
  // redraws, at most every 250 ms, rather than polling a page that is sitting still.
  let soon = 0;
  new MutationObserver(() => {
    if (!soon) soon = window.setTimeout(() => ((soon = 0), tick()), 250);
  }).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
  tick();
  keepCursorBelowBar();
}

/**
 * Where the cursor moves in a cell, it stays in sight below the toolbar. CodeMirror scrolls the cursor
 * into view by the window's top edge and ignores scroll-padding, so a jump up (⌘↑, Page Up, a long
 * cell's first line) left it under the toolbar (headless, 2026-11-12).
 */
function keepCursorBelowBar(): void {
  let queued = false;
  document.addEventListener("selectionchange", () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const selection = window.getSelection();
      if (!selection?.rangeCount || !selection.isCollapsed || !(selection.anchorNode as Element | null)?.parentElement?.closest?.(".cm-content")) return;
      const range = selection.getRangeAt(0);
      const rect = range.getClientRects()[0] ?? (selection.anchorNode as Element).parentElement!.getBoundingClientRect();
      const bar = document.getElementById("endeavor-bar")?.getBoundingClientRect().bottom ?? 0;
      if (rect.bottom > 0 && rect.top < bar + 4) window.scrollBy(0, rect.top - bar - 8);
    });
  });
}

/** Start the look and Point on this page, which shows notebook `notebook`. */
let started = false;

export function start(notebook: string, host: Host): void {
  // Once: a later call (a notebook's own script, say) can't take Point's way into the chat.
  if (started) return;
  started = true;
  // What ⌘↩ added since the last ↩.
  const added: string[] = [];
  const receive = (msg: object) => window.__endeavor?.receive(msg as never);
  const sent = document.createElement("div");
  sent.id = "endeavor-sent";
  let hide = 0;
  // `ms`: how long it shows (0: until the next).
  const say = (text: string, ms = 4000) => {
    sent.textContent = text;
    sent.classList.add("shown");
    clearTimeout(hide);
    if (ms) hide = window.setTimeout(() => sent.classList.remove("shown"), ms);
  };
  const point = (on: boolean) => receive({ type: "annotate", on });
  setHost((msg: ToApp) => {
    if (msg.type !== "quote" || !msg.picks.length) return;
    const text = quoteText(notebook, msg.picks, msg.comment);
    // ⌘↩ adds to the message the next ↩ passes, kept here.
    if (msg.add) {
      added.push(text);
      return say(`${added.length} added. ${mac ? "↩" : "Enter"} passes ${added.length === 1 ? "it" : "them"} to the chat with the next pick.`);
    }
    const earlier = added.splice(0);
    // Claude's desktop app takes a few seconds to show it in its box (2–7 s, 2026-11-12): say it's on its way.
    say("Sending to Claude's chat…", 0);
    host.send(`${LEAD}\n\n${QUOTE_NOTE}${LEAD_NOTE}\n\n${[...earlier, text].join("\n\n")}`).then(
      () => {
        if (msg.ref !== undefined) receive({ type: "quote_sent", ref: msg.ref, ok: true });
        say("On its way to Claude's message box. Send it from there.", 8000);
        // The user sends it from Claude's box, where the view's keys (Escape, ⌘⇧E) don't reach, so Point
        // ends once the view has lost the keyboard, unless something was picked or typed since.
        // While the keyboard stays here, Point stays on for the next pick, and Escape ends it.
        const end = () => {
          const now = pointState();
          if (!now.picks.length && !now.comment.trim()) point(false);
        };
        if (!document.hasFocus()) end();
        else window.addEventListener("blur", end, { once: true });
      },
      (e) => {
        // What ⌘↩ added stays here; Point gets its picks and comment back, for ↩ to send again.
        added.unshift(...earlier);
        if (msg.ref !== undefined) receive({ type: "quote_sent", ref: msg.ref, ok: false });
        say(`Couldn't reach Claude's chat${e instanceof Error && e.message ? `: ${e.message}` : ""}. Your ${msg.picks.length === 1 ? "pick is" : "picks are"} back: ${mac ? "↩" : "Enter"} sends again.`);
      },
    );
  });
  initTheme();
  closeStrayCompletions();
  initLinks(host.openLink);
  followNewCells();
  receive({ type: "theme", name: "view" });
  initBar(host.canSend, point);
  followClaudesPalette();
  // A chat that doesn't take messages from the view gets the look and the toolbar, without Point.
  if (!host.canSend) return;
  initAskBox();
  initAnnotate(true);

  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  document.body.append(sent);
  // ⌘⇧E (Ctrl+Shift+E) toggles Point, as in the app's menu.
  window.addEventListener(
    "keydown",
    (e) => {
      if (!e.isTrusted || !modHeld(e) || !e.shiftKey || e.key.toLowerCase() !== "e" || (mac && e.ctrlKey)) return;
      e.preventDefault();
      e.stopPropagation();
      point(!document.body.classList.contains("annotating"));
    },
    true,
  );
}

/** A link the view hands to the host: a web or mail address, not a jump within the page. */
export function outsideLink(href: string | null): string | null {
  if (!href) return null;
  return /^(https?:|mailto:)/i.test(href.trim()) ? href.trim() : null;
}

/**
 * Links in cells and Live docs open in the user's browser through the host. The view's frame may not
 * open windows (Claude's desktop app gives it no allow-popups), so a click on one did nothing
 * (2026-11-12). Any other link that would load a page (href="#", a relative path, file:) instead
 * replaced the view with a blank one, as the frame's page is markup with no address of its own; a jump
 * within the page ("#cell") scrolls there. In the bubble phase, after the page's own handlers: a click
 * one of them handled (Pluto's Restart link, a widget's own) is left alone, and so is javascript:.
 * Only the user's own clicks open anything: a script's made-up click opens nothing.
 */
export function initLinks(open: ((url: string) => Promise<void>) | undefined): void {
  document.addEventListener("click", (e) => {
    if (e.defaultPrevented || e.button !== 0) return;
    const link = (e.target as Element | null)?.closest?.("a[href]");
    if (!link) return;
    const href = (link.getAttribute("href") ?? "").trim();
    if (/^javascript:/i.test(href)) return;
    e.preventDefault();
    if (/^#./.test(href)) {
      document.getElementById(decodeURIComponent(href.slice(1)))?.scrollIntoView({ behavior: "smooth", block: "start" });
      return;
    }
    const url = outsideLink(href);
    if (url && open && e.isTrusted) open(url).catch(() => {});
  });
}

// Claude's palette with our accent (Jonathan's pick, 2026-11-12): in Claude's panel, the page's
// backgrounds, text greys and borders are Claude's own, from the style variables the host sends
// (MCP Apps' hostContext.styles.variables, which the view puts on <html>; Claude's desktop app sends
// 76, each a light-dark() pair, so they follow the appearance as Pluto's colours do). Our accent,
// the code's colours and the diff colours stay Endeavor's. Only while the host sent them: without,
// the Endeavor look stays as it is.
export const claudePalette = `
html.claude-palette {
  /* Pages, cards, raised controls */
  --e-bg-page: var(--color-background-primary);
  --e-bg-sidebar: var(--color-background-secondary);
  --e-bg-card: var(--color-background-secondary);
  --e-bg-sunken: var(--color-background-secondary);
  --e-bg-raised: var(--color-background-tertiary);
  --e-bg-tag: var(--color-background-tertiary);
  --e-button-hover: var(--color-background-tertiary);
  --e-menu-hover: var(--color-background-tertiary);
  --e-popover-bg: var(--color-background-primary);
  --e-dialog-bg: var(--color-background-primary);
  /* Text */
  --e-text-primary: var(--color-text-primary);
  --e-text-secondary: var(--color-text-secondary);
  --e-text-muted: var(--color-text-secondary);
  --e-text-section: var(--color-text-secondary);
  --e-text-tag: var(--color-text-secondary);
  --e-text-faint: var(--color-text-tertiary);
  --e-text-dim: var(--color-text-tertiary);
  --e-hint-text: var(--color-text-secondary);
  /* Edges */
  --e-border: var(--color-border-tertiary);
  --e-divider: var(--color-border-tertiary);
  --e-popover-edge: var(--color-border-secondary);
  --e-dialog-edge: var(--color-border-secondary);
  --e-control-edge: var(--color-border-primary);
  /* Pluto's own colours that the Endeavor look sets */
  --main-bg-color: var(--color-background-primary);
  --header-bg-color: var(--color-background-primary);
  --footer-bg-color: var(--color-background-primary);
  --pluto-output-bg-color: var(--color-background-primary);
  --pluto-output-color: var(--color-text-secondary);
  --pluto-output-h-color: var(--color-text-primary);
  --rule-color: var(--color-border-tertiary);
  --code-background: var(--color-background-secondary);
  --pluto-logs-bg-color: var(--color-background-secondary);
  --overlay-button-bg: var(--color-background-secondary);
  --input-context-menu-bg-color: var(--color-background-secondary);
  --input-context-menu-border-color: var(--color-border-tertiary);
  --helpbox-bg-color: var(--color-background-secondary);
  --helpbox-header-bg-color: var(--color-background-tertiary);
  --helpbox-header-tab-bg-color: var(--color-background-tertiary);
  --helpbox-header-color: var(--color-text-primary);
  --helpbox-text-color: var(--color-text-primary);
  --helpbox-search-bg-color: var(--color-background-primary);
  --helpbox-search-border-color: var(--color-border-primary);
  --docs-binding-bg: var(--color-background-tertiary);
  --export-bg-color: var(--color-background-secondary);
  --export-color: var(--color-text-primary);
  --frontmatter-button-bg-color: var(--color-background-tertiary);
  --frontmatter-input-bg-color: var(--color-background-primary);
  --frontmatter-input-border-color: var(--color-border-primary);
  --pluto-runarea-span-color: var(--color-text-tertiary);
}
html.claude-palette .pluto-modal { border-color: var(--color-border-secondary); }
`;

/** Which of Claude's variables the palette needs; without all of them it stays off. */
const PALETTE_NEEDS = ["--color-background-primary", "--color-background-secondary", "--color-background-tertiary", "--color-text-primary",
  "--color-text-secondary", "--color-text-tertiary", "--color-border-primary", "--color-border-secondary", "--color-border-tertiary"];

/** Claude's palette is on while the host's variables are on <html> (the view sets them, and sets them again when Claude's change). */
function followClaudesPalette(): void {
  const style = document.createElement("style");
  style.id = "endeavor-claude-palette";
  style.textContent = claudePalette;
  document.head.append(style);
  const root = document.documentElement;
  const sync = () => root.classList.toggle("claude-palette", PALETTE_NEEDS.every((v) => root.style.getPropertyValue(v).trim() !== ""));
  new MutationObserver(sync).observe(root, { attributes: true, attributeFilter: ["style"] });
  sync();
}

/** How long a code completion list may stay open with no key or click from the user. */
const STRAY_MS = 5000;

/**
 * Close Pluto's code completion list when the user isn't using it: open while the view doesn't have
 * the keyboard (the user is in Claude's chat), or for STRAY_MS with no key, click or pointer movement since it opened.
 * Claude's edits land in cells while a cursor sits in one, and a list could stay up over the notebook
 * for as long as Claude worked (2026-11-12). Escape to its editor closes it as the user's Escape would;
 * not while Point is on, whose own Escape ends Point (a list then closes once Point ends).
 */
function closeStrayCompletions(): void {
  let lastInput = 0;
  let timer = 0;
  // A pointer over the list, or a wheel turn, counts as using it.
  for (const type of ["keydown", "pointerdown", "pointermove", "wheel"]) window.addEventListener(type, (e) => e.isTrusted && (lastInput = Date.now()), { capture: true, passive: true });
  const list = () => document.querySelector(".cm-tooltip-autocomplete");
  const close = () => {
    const open = list();
    if (!open || document.body.classList.contains("annotating")) return;
    open.closest(".cm-editor")?.querySelector(".cm-content")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", keyCode: 27, bubbles: true, cancelable: true }));
  };
  // Checked again STRAY_MS after it opened, or after the user's last key or click.
  const check = (openedAt: number) => {
    timer = 0;
    if (!list()) return;
    const idle = Date.now() - Math.max(openedAt, lastInput);
    if (!document.hasFocus() || idle >= STRAY_MS) close();
    if (list()) timer = window.setTimeout(() => check(openedAt), Math.max(STRAY_MS - idle, 500));
  };
  new MutationObserver(() => {
    if (!timer && list()) check(Date.now());
  }).observe(document.body, { childList: true, subtree: true });
  window.addEventListener("blur", close);
}

/** How long since the user's last key or click before the view follows Claude's new cells. */
const IDLE_MS = 2000;
/** How long the view's own scroll to a cell takes, so its scroll events aren't taken for the user's. */
const OWN_SCROLL_MS = 1000;

/**
 * Bring a cell Claude adds into view: the user watches the notebook beside the chat, and otherwise the
 * new cell landed out of sight (2026-11-12). Only while the user follows along: once they scroll the
 * page somewhere else, it stays where they put it, until they scroll back to the end of the notebook
 * (as a chat follows its end). Not within IDLE_MS of a key or click either: a cell the user adds
 * themselves comes right after one, and Pluto shows it as usual. While the page first draws its
 * cells, nothing is followed: Pluto adds them in bursts until it has drawn the whole notebook.
 */
function followNewCells(): void {
  let lastInput = 0;
  for (const type of ["keydown", "pointerdown"]) window.addEventListener(type, (e) => e.isTrusted && (lastInput = Date.now()), true);
  let following = true;
  let ownScroll = 0;
  window.addEventListener(
    "scroll",
    () => {
      if (Date.now() - ownScroll < OWN_SCROLL_MS) return;
      following = window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 80;
    },
    { passive: true },
  );
  const known = new Set<string>();
  let settled = false;
  let settle = 0;
  const cells = () => [...document.querySelectorAll<HTMLElement>("pluto-cell")];
  const show = (cell: HTMLElement) => {
    const r = cell.getBoundingClientRect();
    if (r.top >= 0 && r.bottom <= window.innerHeight) return;
    ownScroll = Date.now();
    cell.scrollIntoView({ block: r.height > window.innerHeight ? "start" : "center", behavior: "smooth" });
  };
  new MutationObserver(() => {
    const added = cells().filter((c) => c.id && !known.has(c.id));
    if (!added.length) return;
    added.forEach((c) => known.add(c.id));
    if (!settled) {
      clearTimeout(settle);
      settle = window.setTimeout(() => (settled = true), 1500);
      return;
    }
    if (added.length > 3 || !following || Date.now() - lastInput < IDLE_MS) return;
    const cell = added[added.length - 1];
    const since = Date.now();
    show(cell);
    // Its output comes once it runs (a plot, say): keep it in view while it grows, for a while, while the user follows.
    const grow = new ResizeObserver(() => {
      if (!following || lastInput > since || Date.now() - since > 15000 || !cell.isConnected) return grow.disconnect();
      show(cell);
    });
    grow.observe(cell);
  }).observe(document.body, { childList: true, subtree: true });
}

declare global {
  interface Window {
    EndeavorView?: { start: typeof start };
  }
}

window.EndeavorView = { start };
