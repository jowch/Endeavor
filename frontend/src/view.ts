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
// box, where the view's keys don't reach. The view also tidies what Pluto shows there (its
// overlays at the bottom, a completion list nobody uses) and follows the cells Claude adds.

import { initAnnotate, pointState } from "./annotate";
import { initAskBox } from "./askbox";
import { setHost, type ToApp } from "./bridge";
import { cellName } from "./cellname";
import { mac, modHeld } from "./keys";
import type { Pick } from "./quote";
import { initTheme } from "./theme";

/** What the view gives the page: Claude's chat. */
export type Host = {
  /** Whether the host takes messages from the view (`send`); without it there is no Point. */
  canSend: boolean;
  /** Pass `text` to the chat as the user's message (the host may wait for the user to send it). */
  send(text: string): Promise<void>;
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

const css = `
  #endeavor-point { position: fixed; right: 16px; bottom: 16px; z-index: 9998; display: flex; align-items: center; gap: 6px;
    height: 28px; padding: 0 12px; border-radius: 999px; border: 1px solid var(--e-popover-edge);
    background: var(--e-popover-bg); color: var(--e-text-secondary); font: 12px system-ui, -apple-system, "Segoe UI", sans-serif;
    box-shadow: 0 4px 16px var(--e-shadow-popover); cursor: pointer; }
  #endeavor-point:hover { color: var(--e-text-primary); background: var(--e-button-hover); }
  #endeavor-point .key { color: var(--e-text-faint); }
  body.annotating #endeavor-point { display: none; }
  #endeavor-sent { position: fixed; right: 16px; bottom: 16px; z-index: 10001; display: none;
    padding: 4px 12px; border-radius: 999px; background: var(--e-hint-bg); color: var(--e-hint-text); font: 12px system-ui; }
  #endeavor-sent.shown { display: block; }
  /* Pluto's own overlays at the bottom, laid out round Point's button: the Live docs and Status tabs just above it,
     the safe-preview badge at the bottom left, and Settings and FAQ, at the end of the page, above all three. */
  pluto-helpbox { bottom: 52px !important; }
  .outline-frame-actions-container.safe-preview { left: 16px; bottom: 12px; }
  pluto-editor footer { padding-bottom: 104px; }
`;

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
    if (msg.type === "shoot") return setTimeout(() => receive({ type: "shot", id: msg.id }));
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
        // ends here, unless something was picked or typed since.
        const now = pointState();
        if (!now.picks.length && !now.comment.trim()) point(false);
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
  followNewCells();
  receive({ type: "theme", name: "view" });
  // A chat that doesn't take messages from the view gets the look only.
  if (!host.canSend) return;
  initAskBox();
  initAnnotate(true);

  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  const button = document.createElement("button");
  button.id = "endeavor-point";
  button.type = "button";
  button.title = "Point at a cell, its code or its output, and comment on it for Claude";
  button.innerHTML = `<span>Point</span><span class="key">${mac ? "⌘⇧E" : "Ctrl+Shift+E"}</span>`;
  button.addEventListener("click", (e) => e.isTrusted && point(true));
  document.body.append(button, sent);
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

/** How long a code completion list may stay open with no key or click from the user. */
const STRAY_MS = 5000;

/**
 * Close Pluto's code completion list when the user isn't using it: open while the view doesn't have
 * the keyboard (the user is in Claude's chat), or for STRAY_MS with no key or click since it opened.
 * Claude's edits land in cells while a cursor sits in one, and a list could stay up over the notebook
 * for as long as Claude worked (2026-11-12). Escape to its editor closes it as the user's Escape would;
 * not while Point is on, whose own Escape ends Point (a list then closes once Point ends).
 */
function closeStrayCompletions(): void {
  let lastInput = 0;
  let timer = 0;
  for (const type of ["keydown", "pointerdown"]) window.addEventListener(type, (e) => e.isTrusted && (lastInput = Date.now()), true);
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

/** How long since the user's last key, click or scroll before the view follows Claude's new cells. */
const IDLE_MS = 2000;

/**
 * Bring a cell Claude adds into view, as long as the user isn't busy in the notebook (no key, click
 * or scroll for IDLE_MS): they watch the notebook beside the chat, and otherwise the new cell landed
 * out of sight, or the page moved somewhere else (2026-11-12). A cell the user adds themselves comes
 * right after their click or key, so Pluto shows it as usual. While the page first draws its cells,
 * nothing is followed: Pluto adds them in bursts until it has drawn the whole notebook.
 */
function followNewCells(): void {
  let lastInput = 0;
  for (const type of ["keydown", "pointerdown", "wheel", "touchmove"]) window.addEventListener(type, (e) => e.isTrusted && (lastInput = Date.now()), { capture: true, passive: true });
  const known = new Set<string>();
  let settled = false;
  let settle = 0;
  const cells = () => [...document.querySelectorAll<HTMLElement>("pluto-cell")];
  const show = (cell: HTMLElement) => {
    const r = cell.getBoundingClientRect();
    if (r.top >= 0 && r.bottom <= window.innerHeight) return;
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
    if (added.length > 3 || Date.now() - lastInput < IDLE_MS) return;
    const cell = added[added.length - 1];
    const since = Date.now();
    show(cell);
    // Its output comes once it runs (a plot, say): keep it in view while it grows, for a while, if the user hasn't moved.
    const grow = new ResizeObserver(() => {
      if (lastInput > since || Date.now() - since > 15000 || !cell.isConnected) return grow.disconnect();
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
