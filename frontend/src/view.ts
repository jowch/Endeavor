// Endeavor's script for the notebook view in Claude's desktop app (EndeavorMCP's
// views/notebook.html, which vendors the built dist/view.js): the Endeavor look
// and Point on Pluto's page, without the app. The view passes a host: Point's
// ↩ sends the picks and the comment as the user's message in Claude's chat,
// and ⌘↩ gives them to Claude as context (MCP Apps' ui/update-model-context),
// which the host may keep for every later turn, not only the next one. Pictures of
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
  /** Whether it takes context from the view (`context`); without it ⌘↩ says to use ↩. */
  canAdd: boolean;
  /** Send `text` as the user's message now. */
  send(text: string): Promise<void>;
  /** Give Claude `text` as context, replacing what was given before; the host may keep it for later turns. */
  context(text: string): Promise<void>;
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

/** What Claude is told with quotes given as context (⌘↩): true whenever Claude reads it, on any later turn. */
export const CONTEXT_NOTE =
  "[Endeavor] At some point in this conversation the user pointed at these parts of the notebook in the notebook view, " +
  "each as a <quote> followed by their comment on it. A later message of theirs may refer to them; they are not a request " +
  "on their own, and the notebook may have changed since. " + FORMAT;

/** At most this many ⌘↩ quotes are kept as context: the host gives no sign that the user's own message took them. */
const KEPT = 10;

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
  // ⌘↩ quotes since the last ↩, given to Claude as context.
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
  // Point clears its comment as it sends: when the chat doesn't take it, it goes back in the box.
  const giveBack = (comment: string, why: string) => {
    const box = document.querySelector<HTMLTextAreaElement>("#annotate-bar textarea");
    if (box && !box.value.trim() && comment) {
      box.value = comment;
      box.dispatchEvent(new Event("input"));
    }
    say(comment ? `${why} Your comment is back in the box.` : why);
  };
  const failed = (comment: string) => (e: unknown) => giveBack(comment, `Couldn't reach Claude's chat${e instanceof Error && e.message ? `: ${e.message}` : ""}.`);
  setHost((msg: ToApp) => {
    if (msg.type === "shoot") return setTimeout(() => toPage?.({ type: "shot", id: msg.id }));
    if (msg.type !== "quote" || !msg.picks.length) return;
    const text = quoteText(notebook, msg.picks, msg.comment);
    if (msg.add) {
      if (!host.canAdd) return giveBack(msg.comment, `This chat can't take context from the notebook: press ${mac ? "↩" : "Enter"} to send.`);
      added.push(text);
      added.splice(0, Math.max(0, added.length - KEPT));
      host.context(`${CONTEXT_NOTE}\n\n${added.join("\n\n")}`).then(
        () => say("Given to Claude as context"),
        (e) => {
          added.pop();
          failed(msg.comment)(e);
        },
      );
      return;
    }
    // What ⌘↩ added goes with this message, then no longer.
    const earlier = added.splice(0);
    host.send(`${QUOTE_NOTE}\n\n${[...earlier, text].join("\n\n")}`).then(
      () => {
        say("Sent to Claude");
        if (earlier.length) host.context("").catch(() => {});
      },
      (e) => {
        added.unshift(...earlier);
        failed(msg.comment)(e);
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
  initAnnotate();

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
