// Asking Claude from inside the notebook (docs/ui-spec.md, "The prompt"). One
// key, ⌘E: with text selected in a cell's code or output it asks about the
// selection (as Reply's pill does; ⌘J is a quiet alias); otherwise, with the
// cursor in a cell, about that cell (an empty cell: what to write in it). The
// ✦ Claude button beside Pluto's "+" asks for a new cell there. All of them
// open the same prompt (askbox.ts), placed as Point's bar is (place.ts), and
// floating over the page, so Pluto's cell list is never touched. Esc or a
// click elsewhere closes it and keeps the words as a draft for that cell or
// selection; opening it there again brings them back, selected.

import { byUser, on, send } from "./bridge";
import { type AskBox, type Icon, askBox } from "./askbox";
import { onRedraw } from "./redraw";
import { cellCode } from "./reveal";
import { modHeld, shortcut } from "./keys";
import { barPlace, type Rect } from "./place";
import { cellName } from "./cellname";
import { pickSource, sendQuote } from "./quote";
import { type Found, hidePill, initReply, rangeRects, selectedInCell } from "./reply";
import { context } from "./state";

const GAP = 8;

const css = () => `
  /* Beside Pluto's "+" in the gap above a cell (and below the last one): faint
     while the cell is hovered, like Pluto's own buttons, and full on the "+". */
  pluto-cell > .endeavor-add-agent {
    position: absolute; left: 14px; z-index: 20;
    height: 18px; padding: 0 7px; border-radius: 9px; border: 1px solid var(--e-pill-edge);
    background: var(--e-pill-bg); color: var(--e-text-tag); font: 11px system-ui, sans-serif; cursor: pointer;
    opacity: 0; transition: opacity 0.1s;
  }
  pluto-cell > .endeavor-add-agent.before { top: calc(-0.5 * var(--pluto-cell-spacing, 17px) - 9px); }
  pluto-cell > .endeavor-add-agent.after { bottom: calc(-0.5 * var(--pluto-cell-spacing, 17px) - 9px); }
  pluto-cell:hover > .endeavor-add-agent { opacity: 0.35; }
  pluto-cell > button.add_cell:hover + .endeavor-add-agent, pluto-cell > .endeavor-add-agent:hover { opacity: 1; }
  pluto-cell > .endeavor-add-agent:hover { color: var(--e-prompt-hover); border-color: var(--e-accent); }
  /* What the prompt asks about: the cell's code (or the output a selection is in). */
  .endeavor-asked { outline: 1.5px solid var(--e-focus-ring) !important; outline-offset: 0; }
  /* Where a new cell will go. */
  #endeavor-ask-line { position: fixed; z-index: 999; height: 2px; border-radius: 1px; background: var(--e-focus-ring); pointer-events: none; }
  /* The empty-cell hint names the shortcut. */
  pluto-input .cm-placeholder { font-size: 0; }
  pluto-input .cm-placeholder::after { content: "Type code, or ${shortcut("E")} to ask ${context.agent}"; font-size: 13px; }
`;

/** What a prompt is about: a cell (or, empty, what to write in it), a new cell before or after one, or a selection. */
export type Target = { kind: "cell" | "before" | "after"; cell: HTMLElement } | { kind: "selection"; found: Found };

type Open = {
  target: Target;
  box: AskBox;
  draft: string;
  marked: HTMLElement | null;
  line: HTMLElement | null;
  /** Opened below what it asks about: it grows down from there. */
  below: boolean;
};

let open: Open | null = null;
const drafts = new Map<string, string>();

const isEmpty = (cell: HTMLElement) => !cellCode(cell).trim();
const cellOf = (t: Target) => (t.kind === "selection" ? t.found.cell : t.cell);
const previous = (cell: HTMLElement) => {
  let prev = cell.previousElementSibling;
  while (prev && prev.tagName !== "PLUTO-CELL") prev = prev.previousElementSibling;
  return prev as HTMLElement | null;
};

/** The draft's key: one per cell, per gap, per selection. */
function draftKey(t: Target): string {
  if (t.kind !== "selection") return `${t.kind}:${t.cell.id}`;
  return `selection:${t.found.cell.id}:${pickSource(t.found.pick)}:${t.found.quote}`;
}

/** The top line, placeholder and quote for a target. */
function describe(t: Target): { icon: Icon; name: string; detail: string; placeholder: string; quote?: { text: string; code: boolean } } {
  if (t.kind === "selection") {
    const { pick, quote } = t.found;
    const [name, detail] = pickSource(pick).split(" · ");
    if (pick.part === "lines") {
      const one = pick.lines[0] === pick.lines[1];
      return { icon: "lines", name, detail, placeholder: `Ask ${context.agent} about ${one ? "this line" : "these lines"}`, quote: { text: quote, code: true } };
    }
    return { icon: "text", name, detail, placeholder: `Ask ${context.agent} about this text`, quote: { text: quote, code: false } };
  }
  const { cell } = t;
  const prev = previous(cell);
  const after = (c: HTMLElement) => `after ${cellName(cellCode(c))}`;
  if (t.kind === "before") return { icon: "add", name: "new cell", detail: prev ? after(prev) : `before ${cellName(cellCode(cell))}`, placeholder: `Ask ${context.agent} to write a cell here` };
  if (t.kind === "after") return { icon: "add", name: "new cell", detail: after(cell), placeholder: `Ask ${context.agent} to write a cell here` };
  if (isEmpty(cell)) return { icon: "add", name: "new cell", detail: prev ? after(prev) : "here", placeholder: `Ask ${context.agent} what to write here` };
  return { icon: "cell", name: cellName(cellCode(cell)), detail: "whole cell", placeholder: `Ask ${context.agent} about this cell` };
}

/** Where what the prompt asks about is, in the viewport. */
function pickRect(o: Open): Rect {
  const t = o.target;
  if (t.kind === "selection") {
    const rects = rangeRects(t.found.range);
    return {
      left: Math.min(...rects.map((r) => r.left)),
      top: Math.min(...rects.map((r) => r.top)),
      right: Math.max(...rects.map((r) => r.right)),
      bottom: Math.max(...rects.map((r) => r.bottom)),
    };
  }
  if (t.kind === "cell") return (t.cell.querySelector("pluto-input") ?? t.cell).getBoundingClientRect();
  // A new cell: halfway across the gap above the cell, or below the last one.
  const r = t.cell.getBoundingClientRect();
  const spacing = parseFloat(getComputedStyle(t.cell).getPropertyValue("--pluto-cell-spacing")) || 17;
  const y = t.kind === "before" ? r.top - spacing / 2 : r.bottom + spacing / 2;
  return { left: r.left, top: y - 1, right: r.right, bottom: y + 1 };
}

function place(o: Open) {
  const pick = pickRect(o);
  if (o.line) Object.assign(o.line.style, { left: `${pick.left}px`, top: `${pick.top}px`, width: `${pick.right - pick.left}px` });
  const root = o.box.root;
  const at = barPlace(pick, window.innerWidth, window.innerHeight, root.offsetHeight);
  // Placed below, longer words grow it down rather than flipping it above.
  if (o.below && pick.bottom >= 0 && pick.bottom <= window.innerHeight) at.top = pick.bottom + GAP;
  Object.assign(root.style, { left: `${at.left}px`, top: `${at.top}px`, width: `${at.width}px` });
}

/** Close the prompt, keeping its words as the draft. `refocus`: back to the cell's editor. */
export function closeAsk(refocus: boolean): void {
  if (!open) return;
  const o = open;
  open = null;
  const words = o.box.text.value;
  if (words.trim()) drafts.set(o.draft, words);
  else drafts.delete(o.draft);
  o.box.root.remove();
  o.line?.remove();
  o.marked?.classList.remove("endeavor-asked");
  if (o.target.kind === "selection") {
    // Leave the selection as it was.
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(o.target.found.range);
  } else if (refocus && o.target.kind === "cell") o.target.cell.querySelector<HTMLElement>("pluto-input .cm-content")?.focus();
}

function sendAsk(o: Open, add: boolean, e: Event) {
  if (!byUser(e)) return;
  const comment = o.box.text.value.trim();
  const t = o.target;
  if (t.kind === "selection") {
    sendQuote([t.found.pick], comment, add);
  } else {
    if (!comment) return;
    const notebook = new URLSearchParams(location.search).get("id");
    const where = t.kind !== "cell" ? t.kind : isEmpty(t.cell) ? "fill" : "about";
    send({ type: "prompt", notebook, cell: t.cell.id, code: cellCode(t.cell), where, text: comment, add });
  }
  o.box.text.value = "";
  closeAsk(t.kind === "cell");
  if (t.kind === "selection" && add) window.getSelection()?.removeAllRanges();
}

/** Open the prompt about `t`, with its draft if it has one. */
export function openAsk(t: Target): void {
  closeAsk(false);
  hidePill();
  const about = describe(t);
  const box = askBox({ label: `Ask ${context.agent}`, placeholder: about.placeholder, quote: about.quote, done: (add, e) => open && sendAsk(open, add, e) });
  box.root.classList.add("popover");
  box.root.id = "endeavor-ask";
  box.root.dataset.kind = t.kind;
  box.setWhat(about.icon, about.name, about.detail);
  box.root.addEventListener(
    "keydown",
    (e) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      closeAsk(true);
    },
    true,
  );
  const key = draftKey(t);
  const draft = drafts.get(key);
  if (draft) box.text.value = draft;
  let marked: HTMLElement | null = null;
  let line: HTMLElement | null = null;
  if (t.kind === "before" || t.kind === "after") {
    line = document.createElement("div");
    line.id = "endeavor-ask-line";
    line.dataset.endeavorUi = "";
    document.body.append(line);
  } else {
    const inOutput = t.kind === "selection" && t.found.pick.part === "output";
    marked = cellOf(t).querySelector<HTMLElement>(inOutput ? "pluto-output" : "pluto-input");
    marked?.classList.add("endeavor-asked");
  }
  document.body.append(box.root);
  // Sized once it is in the page: a draft may need several lines.
  box.text.dispatchEvent(new Event("input"));
  const o: Open = { target: t, box, draft: key, marked, line, below: true };
  const pick = pickRect(o);
  o.below = barPlace(pick, window.innerWidth, window.innerHeight, box.root.offsetHeight).top === pick.bottom + GAP;
  place(o);
  open = o;
  box.text.addEventListener("input", () => open === o && place(o));
  // After the key event that opened it: focusing during ⌘E's keydown doesn't stick.
  requestAnimationFrame(() => {
    box.text.focus();
    if (draft) box.text.select();
  });
}

/** The open prompt, for the state dump (debug.ts). */
export function askState(): { kind: string; about: string; text: string } | null {
  return open && { kind: open.target.kind, about: open.box.what.textContent ?? "", text: open.box.text.value };
}

/** ⌘E (and ⌘J) in the page: a selection in a cell → about the selection; the cursor in a cell → about the cell; else nothing. */
function onKey(e: KeyboardEvent) {
  const key = e.key.toLowerCase();
  if (!modHeld(e) || e.shiftKey || e.altKey || (key !== "e" && key !== "j")) return;
  if (document.body.classList.contains("annotating")) return;
  if (open?.box.root.contains(e.target as Node)) {
    if (key !== "e") return;
    e.preventDefault();
    e.stopPropagation();
    return closeAsk(true);
  }
  const found = selectedInCell();
  const cell = (document.activeElement as Element | null)?.closest<HTMLElement>("pluto-cell");
  if (!found && (key === "j" || !cell)) return;
  e.preventDefault();
  e.stopPropagation();
  openAsk(found ? { kind: "selection", found } : { kind: "cell", cell: cell! });
}

export function initPrompt(): void {
  const style = document.createElement("style");
  style.textContent = css();
  document.head.append(style);
  // The empty-cell hint is baked into this style tag, so it needs redoing
  // once the session's actual agent reaches the page.
  on("context", () => (style.textContent = css()));
  initReply((found) => openAsk({ kind: "selection", found }));

  // Window capture: before CodeMirror and Pluto handle the key.
  window.addEventListener("keydown", onKey, true);
  document.addEventListener("mousedown", (e) => {
    if (open && !open.box.root.contains(e.target as Node)) closeAsk(false);
  });
  window.addEventListener("resize", () => open && place(open));
  window.addEventListener("scroll", () => open && place(open), true);

  // The agent button beside Pluto's "+": above each cell, and below the last.
  // Re-added when Pluto redraws.
  onRedraw(() => {
    const cells = [...document.querySelectorAll<HTMLElement>("pluto-cell")];
    cells.forEach((cell, i) => {
      const places: Array<"before" | "after"> = i === cells.length - 1 ? ["before", "after"] : ["before"];
      for (const where of places) {
        if (cell.querySelector(`:scope > .endeavor-add-agent.${where}`)) continue;
        const add = cell.querySelector(`:scope > button.add_cell.${where}`);
        if (!add) continue;
        const button = document.createElement("button");
        button.className = `endeavor-add-agent ${where}`;
        button.dataset.endeavorUi = "";
        button.textContent = `✦ ${context.agent}`;
        button.title = `Ask ${context.agent} to write a cell here`;
        button.onclick = () => openAsk({ kind: where, cell });
        add.after(button);
      }
    });
    // A cell that stopped being last keeps a stale bottom button.
    for (const stale of document.querySelectorAll("pluto-cell:not(:last-of-type) > .endeavor-add-agent.after")) stale.remove();
    // A deleted cell takes its drafts (and its open prompt) with it.
    const ids = new Set(cells.map((c) => c.id));
    for (const key of drafts.keys()) if (!ids.has(key.split(":")[1])) drafts.delete(key);
    if (open && !ids.has(cellOf(open.target).id)) closeAsk(false);
  });
}
