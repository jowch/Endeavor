// Find in the notebook, for the view in Claude's desktop app. Desktop's own ⌘F searches Desktop's page
// and not the view's frame (Mac audit, 2026-11-12: 0/0 for text on screen), so the view has its own: a
// Find button in the toolbar, and ⌘F (Ctrl+F) with the cursor in the view, which the view takes before
// Desktop's bar opens (Mac, 2026-11-13). It searches the text on the page: the cells' code and their
// outputs, as shown. A folded cell's code and the lines of a long cell that CodeMirror hasn't drawn are
// left out, as a browser's find leaves them.

import { mac, modHeld } from "./keys";

/** Where a search looks: the notebook's cells (Pluto's and Ember's page), not the view's own chrome. */
const ROOT = "pluto-notebook";
/** A match never runs across two of these: a code line, a paragraph, a table cell, a cell. */
const BLOCK = ".cm-line, p, li, td, th, h1, h2, h3, h4, h5, h6, pre, div, pluto-output, pluto-cell";

/** The matches of `query` in the text under `root`, case-insensitive, in page order. */
export function findMatches(root: Element, query: string): Range[] {
  const needle = query.toLowerCase();
  if (!needle) return [];
  const doc = root.ownerDocument;
  // What isn't drawn (a folded cell's code, a closed details, display: none) isn't found. Asked once per element.
  const shown = new Map<Element, boolean>();
  const isShown = (el: Element) => {
    let yes = shown.get(el);
    if (yes === undefined) shown.set(el, (yes = (el as any).checkVisibility?.() ?? true));
    return yes;
  };
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT, {
    acceptNode: (node) => {
      if (node.nodeType === 1) return (node as Element).tagName === "BR" ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
      const parent = node.parentElement;
      if (!parent || !node.nodeValue) return NodeFilter.FILTER_REJECT;
      if (parent.closest("script, style, template, [hidden], [aria-hidden='true']") || !isShown(parent)) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  // The text, with a line break wherever the block changes or a <br> breaks the line, and which node each stretch came from.
  const pieces: { node: Text; at: number }[] = [];
  let text = "";
  let block: Element | null = null;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.nodeType === 1) {
      text += "\n";
      block = null;
      continue;
    }
    const here = node.parentElement!.closest(BLOCK);
    if (here !== block) {
      text += "\n";
      block = here;
    }
    pieces.push({ node: node as Text, at: text.length });
    text += node.nodeValue!;
  }
  const haystack = text.toLowerCase();
  const point = (offset: number, end: boolean): [Text, number] => {
    // The last piece starting at or before `offset` (for an end, before it).
    let lo = 0;
    let hi = pieces.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (end ? pieces[mid].at < offset : pieces[mid].at <= offset) lo = mid;
      else hi = mid - 1;
    }
    return [pieces[lo].node, offset - pieces[lo].at];
  };
  const found: Range[] = [];
  for (let i = haystack.indexOf(needle); i >= 0; i = haystack.indexOf(needle, i + needle.length)) {
    const range = doc.createRange();
    range.setStart(...point(i, false));
    range.setEnd(...point(i + needle.length, true));
    found.push(range);
  }
  return found;
}

const glass = `<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 14 14"/></svg>`;

/** The toolbar's Find button. */
export const findButton = () =>
  `<button type="button" id="endeavor-find-button" title="Find in the notebook (${mac ? "⌘F" : "Ctrl+F"})" aria-label="Find">${glass}<span class="word">Find</span></button>`;

const css = `
  #endeavor-find { position: fixed; top: calc(var(--endeavor-bar) + 6px); right: 10px; z-index: 9998; display: none; align-items: center; gap: 4px;
    padding: 4px 4px 4px 8px; border-radius: 8px; border: 1px solid var(--e-divider); background: var(--e-bg-page);
    box-shadow: 0 4px 16px var(--e-shadow-popover); color: var(--e-text-secondary); font: 12.5px system-ui, -apple-system, "Segoe UI", sans-serif; }
  #endeavor-find.open { display: flex; }
  #endeavor-find input { width: min(220px, 50vw); border: none; outline: none; background: none; color: var(--e-text-primary); font: inherit; }
  #endeavor-find .count { min-width: 44px; text-align: right; color: var(--e-text-muted); white-space: nowrap; }
  #endeavor-find button { display: flex; align-items: center; justify-content: center; width: 24px; height: 24px; padding: 0; border: none; border-radius: 6px;
    background: none; color: var(--e-text-secondary); font: inherit; cursor: pointer; }
  #endeavor-find button:hover { color: var(--e-text-primary); background: var(--e-button-hover); }
  ::highlight(endeavor-find) { background-color: rgba(255, 196, 0, 0.35); }
  ::highlight(endeavor-find-current) { background-color: rgba(255, 150, 0, 0.75); color: black; }
`;

type Highlights = { set(name: string, value: unknown): void; delete(name: string): void };
const highlights = (): Highlights | undefined => (globalThis as any).CSS?.highlights;
const Highlight = (): (new (...ranges: Range[]) => unknown) | undefined => (globalThis as any).Highlight;

/** The Find box under the toolbar, opened by `button` and by ⌘F (Ctrl+F). */
export function initFind(button: HTMLButtonElement): { open(): void } {
  const style = document.createElement("style");
  style.id = "endeavor-find-style";
  style.textContent = css;
  document.head.append(style);
  const box = document.createElement("div");
  box.id = "endeavor-find";
  box.setAttribute("role", "search");
  box.innerHTML =
    `<input type="text" placeholder="Find in the notebook" aria-label="Find in the notebook" spellcheck="false">` +
    `<span class="count" aria-live="polite"></span>` +
    `<button type="button" class="previous" title="Previous (⇧↩)" aria-label="Previous match">↑</button>` +
    `<button type="button" class="next" title="Next (↩)" aria-label="Next match">↓</button>` +
    `<button type="button" class="close" title="Close (Esc)" aria-label="Close">✕</button>`;
  document.body.append(box);
  const input = box.querySelector("input")!;
  const count = box.querySelector<HTMLElement>(".count")!;
  let matches: Range[] = [];
  let current = -1;

  const paint = () => {
    const h = highlights();
    const H = Highlight();
    if (!h || !H) return;
    h.set("endeavor-find", new H(...matches));
    if (current >= 0 && matches[current]) h.set("endeavor-find-current", new H(matches[current]));
    else h.delete("endeavor-find-current");
  };
  const show = () => {
    const range = matches[current];
    count.textContent = !input.value ? "" : matches.length ? `${current + 1} of ${matches.length}` : "No matches";
    paint();
    if (!range) return;
    // Into sight: first inside any scrolling box around it (a long output, a wide table), then in the
    // page, below the toolbar and this box.
    for (let el = range.startContainer.parentElement; el && el !== document.body; el = el.parentElement) {
      const style = getComputedStyle(el);
      const r = range.getBoundingClientRect();
      const area = el.getBoundingClientRect();
      if (/auto|scroll/.test(style.overflowY) && el.scrollHeight > el.clientHeight && (r.top < area.top || r.bottom > area.bottom))
        el.scrollTop += r.top - area.top - el.clientHeight / 2;
      if (/auto|scroll/.test(style.overflowX) && el.scrollWidth > el.clientWidth && (r.left < area.left || r.right > area.right))
        el.scrollLeft += r.left - area.left - el.clientWidth / 2;
    }
    const rect = range.getBoundingClientRect();
    const top = (document.getElementById("endeavor-bar")?.getBoundingClientRect().bottom ?? 0) + box.offsetHeight + 16;
    if (rect.top < top || rect.bottom > window.innerHeight - 16) window.scrollBy(0, rect.top - (top + window.innerHeight) / 2);
  };
  const search = (keep: boolean) => {
    const root = document.querySelector(ROOT);
    const before = matches[current];
    matches = root ? findMatches(root, input.value) : [];
    // After the notebook changes, stay on the same match if it's still there; else the first.
    current = matches.length ? 0 : -1;
    if (keep && before) {
      const same = matches.findIndex((m) => m.compareBoundaryPoints(Range.START_TO_START, before) >= 0);
      if (same >= 0) current = same;
    }
    show();
  };
  const step = (by: number) => {
    if (!matches.length) return;
    current = (current + by + matches.length) % matches.length;
    show();
  };
  const isOpen = () => box.classList.contains("open");
  const open = () => {
    box.classList.add("open");
    button.classList.add("on");
    input.focus();
    input.select();
    if (input.value) search(true);
  };
  const close = () => {
    box.classList.remove("open");
    button.classList.remove("on");
    matches = [];
    current = -1;
    highlights()?.delete("endeavor-find");
    highlights()?.delete("endeavor-find-current");
  };

  let typing = 0;
  input.addEventListener("input", () => {
    clearTimeout(typing);
    typing = window.setTimeout(() => ((typing = 0), search(false)), 80);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      // ↩ straight after typing goes to the first match of what was typed, not the next of the old search.
      if (typing) {
        clearTimeout(typing);
        typing = 0;
        search(false);
      } else step(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
    // Pluto's own keys (⌘S, Shift+Enter in a cell) are for the cells, not the box.
    e.stopPropagation();
  });
  box.querySelector(".previous")!.addEventListener("click", () => step(-1));
  box.querySelector(".next")!.addEventListener("click", () => step(1));
  box.querySelector(".close")!.addEventListener("click", close);
  button.addEventListener("click", (e) => e.isTrusted && (isOpen() ? close() : open()));
  // ⌘F (Ctrl+F) and ⌘G, with the cursor in the view. Before CodeMirror's own keys, and before Desktop's find.
  window.addEventListener(
    "keydown",
    (e) => {
      if (!e.isTrusted || !modHeld(e) || e.altKey || (mac && e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "f" && !e.shiftKey) {
        e.preventDefault();
        e.stopPropagation();
        open();
      } else if (key === "g" && isOpen()) {
        e.preventDefault();
        e.stopPropagation();
        step(e.shiftKey ? -1 : 1);
      }
    },
    true,
  );
  // The notebook changes under an open box (a cell runs, Claude adds one): search again, at most every 300 ms.
  let soon = 0;
  new MutationObserver((records) => {
    if (!isOpen() || !input.value || records.every((r) => box.contains(r.target))) return;
    if (!soon) soon = window.setTimeout(() => ((soon = 0), search(true)), 300);
  }).observe(document.body, { childList: true, subtree: true, characterData: true });
  return { open };
}
