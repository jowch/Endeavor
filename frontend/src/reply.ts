// Reply on text selected in the notebook: a selection in one cell's code,
// output or rendered Markdown offers a "Reply ⌘E" pill under it; the pill (or
// ⌘E, or ⌘J) opens the prompt with the quote (prompt.ts).

import { byUser } from "./bridge";
import { pillPlace } from "./place";
import { type Pick } from "./quote";
import { cellCode } from "./reveal";
import { shortcut } from "./keys";

const css = `
  #endeavor-reply-pill { position: absolute; z-index: 1000; display: inline-flex; align-items: center; height: 30px; box-sizing: border-box;
    padding: 0 3px; border-radius: 8px; border: 1px solid var(--e-popover-edge); background: var(--e-popover-bg);
    box-shadow: 0 8px 24px var(--e-shadow-popover); font: 13px/18px system-ui, sans-serif; }
  #endeavor-reply-pill button { height: 24px; display: flex; align-items: center; gap: 6px; padding: 0 8px; border: 0; border-radius: 5px;
    background: transparent; color: var(--e-text-primary); font: inherit; cursor: pointer; }
  #endeavor-reply-pill button:hover { background: var(--e-menu-hover); }
  #endeavor-reply-pill .key { font-size: 11px; color: var(--e-text-faint); }
`;

/** Text selected in one cell, as a pick, and the range it covers. */
export type Found = { cell: HTMLElement; pick: Pick; quote: string; range: Range };

/** A line's number in a cell's editor, counting from 1 (from the DOM when there's no editor to ask). */
function lineOf(node: Node, content: Element): number {
  const view = (content as any).cmTile?.root?.view;
  if (view) return view.state.doc.lineAt(view.posAtDOM(node)).number;
  const line = (node instanceof Element ? node : node.parentElement)?.closest(".cm-line");
  return [...content.querySelectorAll(".cm-line")].indexOf(line as Element) + 1;
}

/** The text selected inside one cell: code (with its lines), or output text. */
export function selectedInCell(): Found | null {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || !selection.rangeCount) return null;
  const range = selection.getRangeAt(0);
  const node = range.commonAncestorContainer;
  const element = node instanceof Element ? node : node.parentElement;
  const cell = element?.closest<HTMLElement>("pluto-cell");
  if (!cell || element?.closest("[data-endeavor-ui]")) return null;
  const code = cellCode(cell);
  const content = element?.closest(".cm-content");
  if (content) {
    const view = (content as any).cmTile?.root?.view;
    const main = view?.state.selection.main;
    let [first, last] = [lineOf(range.startContainer, content), lineOf(range.endContainer, content)];
    // A selection that ends at a line's start (a triple-click) doesn't take that line.
    if (main && !main.empty) [first, last] = [view.state.doc.lineAt(main.from).number, view.state.doc.lineAt(Math.max(main.from, main.to - 1)).number];
    if (first < 1 || last < first) return null;
    // Without the editor to ask, the whole lines: the page's text has no line breaks.
    const quote = main && !main.empty ? view.state.sliceDoc(main.from, main.to) : code.split("\n").slice(first - 1, last).join("\n");
    if (!quote.trim()) return null;
    return { cell, pick: { part: "lines", cell: cell.id, code, lines: [first, last], text: quote }, quote, range: range.cloneRange() };
  }
  if (!element?.closest("pluto-output")) return null;
  const quote = selection.toString().trim();
  return quote ? { cell, pick: { part: "output", cell: cell.id, code, text: quote }, quote, range: range.cloneRange() } : null;
}

let pill: HTMLElement | null = null;

export function hidePill(): void {
  pill?.remove();
  pill = null;
}

/** The selection's line boxes, in the viewport. */
export function rangeRects(range: Range): DOMRect[] {
  const rects = [...range.getClientRects()];
  return rects.length ? rects : [range.getBoundingClientRect()];
}

/** Offer the pill for each new selection; `open` opens the prompt for it. */
export function initReply(open: (found: Found) => void): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  document.addEventListener("mouseup", (e) => {
    const target = e.target as Element;
    if (pill?.contains(target) || target.closest?.("[data-endeavor-ui]") || document.body.classList.contains("annotating")) return;
    // After the browser has settled the selection.
    setTimeout(() => {
      hidePill();
      const found = selectedInCell();
      if (!found) return;
      pill = document.createElement("div");
      pill.id = "endeavor-reply-pill";
      pill.dataset.endeavorUi = "";
      pill.innerHTML = `<button aria-label="Reply">Reply <span class="key">${shortcut("E")}</span></button>`;
      const at = pillPlace(rangeRects(found.range), window.innerHeight);
      pill.style.left = `${at.left + window.scrollX}px`;
      pill.style.top = `${at.top + window.scrollY}px`;
      // Keep the selection: pressing the pill would otherwise clear it first.
      pill.onmousedown = (event) => event.preventDefault();
      pill.querySelector("button")!.onclick = (event) => byUser(event) && open(found);
      document.body.append(pill);
    });
  });
  window.addEventListener("keydown", () => hidePill(), true);
}
