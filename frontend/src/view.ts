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

import { initAnnotate } from "./annotate";
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
`;

/** Start the look and Point on this page, which shows notebook `notebook`. */
let started = false;

export function start(notebook: string, host: Host): void {
  // Once: a later call (a notebook's own script, say) can't take Point's way into the chat.
  if (started) return;
  started = true;
  // What ⌘↩ added since the last ↩.
  const added: string[] = [];
  let toPage: ((msg: object) => void) | null = null;
  const sent = document.createElement("div");
  sent.id = "endeavor-sent";
  let hide = 0;
  const say = (text: string) => {
    sent.textContent = text;
    sent.classList.add("shown");
    clearTimeout(hide);
    hide = window.setTimeout(() => sent.classList.remove("shown"), 4000);
  };
  setHost((msg: ToApp) => {
    if (msg.type === "shoot") return setTimeout(() => toPage?.({ type: "shot", id: msg.id }));
    if (msg.type !== "quote" || !msg.picks.length) return;
    const text = quoteText(notebook, msg.picks, msg.comment);
    // ⌘↩ adds to the message the next ↩ passes, kept here.
    if (msg.add) {
      added.push(text);
      return say(`${added.length} added. ${mac ? "↩" : "Enter"} passes ${added.length === 1 ? "it" : "them"} to the chat with the next pick.`);
    }
    const earlier = added.splice(0);
    host.send(`${QUOTE_NOTE}\n\n${[...earlier, text].join("\n\n")}`).then(
      () => {
        toPage?.({ type: "quote_sent", ok: true });
        say("Passed to the chat");
      },
      (e) => {
        // What ⌘↩ added stays here; Point gets its picks and comment back, for ↩ to send again.
        added.unshift(...earlier);
        toPage?.({ type: "quote_sent", ok: false });
        say(`Couldn't reach Claude's chat${e instanceof Error && e.message ? `: ${e.message}` : ""}. Your ${msg.picks.length === 1 ? "pick is" : "picks are"} back: ${mac ? "↩" : "Enter"} sends again.`);
      },
    );
  });
  initTheme();
  const receive = (msg: object) => window.__endeavor?.receive(msg as never);
  toPage = receive;
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
  const point = (on: boolean) => receive({ type: "annotate", on });
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

declare global {
  interface Window {
    EndeavorView?: { start: typeof start };
  }
}

window.EndeavorView = { start };
