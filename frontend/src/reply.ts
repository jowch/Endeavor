// Reply on text selected in the notebook: a selection in one cell's code,
// output or rendered Markdown offers a "Reply ⌘J" pill under it, and the pill
// or ⌘J opens a small prompt with the quote. ↩ sends the quote and the reply
// now, ⌘↩ adds them to the chat's message (quote.ts), Esc closes the prompt
// and leaves the selection as it was.

import { byUser } from "./bridge";
import { pillPlace } from "./place";
import { type Pick, pickSource, quoteField, sendQuote } from "./quote";
import { cellCode } from "./reveal";
import { modHeld, shortcut } from "./keys";

const css = `
  #endeavor-reply-pill { position: absolute; z-index: 1000; display: inline-flex; align-items: center; height: 30px; box-sizing: border-box;
    padding: 0 3px; border-radius: 8px; border: 1px solid var(--e-popover-edge); background: var(--e-popover-bg);
    box-shadow: 0 8px 24px var(--e-shadow-popover); font: 13px/18px system-ui, sans-serif; }
  #endeavor-reply-pill button { height: 24px; display: flex; align-items: center; gap: 6px; padding: 0 8px; border: 0; border-radius: 5px;
    background: transparent; color: var(--e-text-primary); font: inherit; cursor: pointer; }
  #endeavor-reply-pill button:hover { background: var(--e-menu-hover); }
  #endeavor-reply-pill .key { font-size: 11px; color: var(--e-text-faint); }
  #endeavor-reply { position: absolute; z-index: 1000; width: 380px; box-sizing: border-box; display: flex; flex-direction: column; gap: 6px;
    padding: 8px; border-radius: 10px; border: 1px solid var(--e-popover-edge); background: var(--e-popover-bg);
    box-shadow: 0 12px 32px var(--e-shadow-popover); font: 13px system-ui, sans-serif; color: var(--e-text-primary); }
  #endeavor-reply .quote { border-left: 2px solid var(--e-control-edge); padding-left: 8px; font-size: 12.5px; line-height: 17px;
    color: var(--e-text-faint); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  #endeavor-reply .source { font-size: 12px; color: var(--e-text-faint); }
`;

/** Text selected in one cell, as a pick, and where it is on the page. */
type Found = { cell: HTMLElement; pick: Pick; quote: string; rects: DOMRect[] };

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
  const rects = [...range.getClientRects()];
  if (!rects.length) rects.push(range.getBoundingClientRect());
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
    return { cell, pick: { part: "lines", cell: cell.id, code, lines: [first, last], text: quote }, quote, rects };
  }
  if (!element?.closest("pluto-output")) return null;
  const quote = selection.toString().trim();
  return quote ? { cell, pick: { part: "output", cell: cell.id, code, text: quote }, quote, rects } : null;
}

/** The pill's place for a selection, in page coordinates. */
function pillAt(found: Found): { left: number; top: number } {
  const at = pillPlace(found.rects, window.innerHeight);
  return { left: at.left + window.scrollX, top: at.top + window.scrollY };
}

export function initReply(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  let pill: HTMLElement | null = null;
  let prompt: HTMLElement | null = null;
  const hidePill = () => {
    pill?.remove();
    pill = null;
  };
  const close = () => {
    prompt?.remove();
    prompt = null;
  };

  function open(found: Found) {
    hidePill();
    close();
    const box = document.createElement("div");
    box.id = "endeavor-reply";
    box.dataset.endeavorUi = "";
    box.setAttribute("role", "dialog");
    box.setAttribute("aria-label", "Reply");
    const quote = document.createElement("div");
    quote.className = "quote";
    quote.textContent = found.quote.split(/\s+/).join(" ").trim();
    const source = document.createElement("div");
    source.className = "source";
    source.textContent = pickSource(found.pick);
    const field = quoteField("Reply to Claude", "Send reply", (add, e) => {
      if (!byUser(e)) return;
      sendQuote([found.pick], field.text.value.trim(), add);
      if (add) window.getSelection()?.removeAllRanges();
      close();
    });
    // Capture, like the field's own keys, which stop the event there.
    field.text.addEventListener(
      "keydown",
      (e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          close();
        }
      },
      true,
    );
    box.append(quote, source, field.root);
    document.body.append(box);
    const place = pillAt(found);
    box.style.left = `${Math.min(place.left - 12, window.scrollX + window.innerWidth - 388)}px`;
    box.style.top = `${place.top}px`;
    prompt = box;
    requestAnimationFrame(() => field.text.focus());
  }

  document.addEventListener("mouseup", (e) => {
    const target = e.target as Node;
    if (pill?.contains(target) || prompt?.contains(target) || document.body.classList.contains("annotating")) return;
    // After the browser has settled the selection.
    setTimeout(() => {
      hidePill();
      const found = selectedInCell();
      if (!found) return;
      pill = document.createElement("div");
      pill.id = "endeavor-reply-pill";
      pill.dataset.endeavorUi = "";
      pill.innerHTML = `<button aria-label="Reply">Reply <span class="key">${shortcut("J")}</span></button>`;
      const place = pillAt(found);
      pill.style.left = `${place.left}px`;
      pill.style.top = `${place.top}px`;
      // Keep the selection: pressing the pill would otherwise clear it first.
      pill.onmousedown = (event) => event.preventDefault();
      pill.querySelector("button")!.onclick = (event) => byUser(event) && open(found);
      document.body.append(pill);
    });
  });
  document.addEventListener("mousedown", (e) => {
    if (prompt && !prompt.contains(e.target as Node)) close();
  });
  // Window capture: before CodeMirror and Pluto see ⌘J.
  window.addEventListener(
    "keydown",
    (e) => {
      if (modHeld(e) && !e.shiftKey && e.key.toLowerCase() === "j") {
        const found = selectedInCell();
        if (!found || document.body.classList.contains("annotating")) return;
        e.preventDefault();
        e.stopPropagation();
        return open(found);
      }
      if (!prompt?.contains(e.target as Node)) hidePill();
    },
    true,
  );
}
