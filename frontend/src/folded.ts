// Folded cells Claude changes (docs/ui-spec.md, "Folded cells"): a cell whose
// code is folded and that Claude added or changed shows its code until it has
// run once without an error, with a tag at the code's top-right, "Folded ·
// shows until it runs" ("… runs without error" after a failed run). A clean
// run folds it again over 200 ms (at once with Reduce motion). The user's
// choice wins: the tag's "Fold now" (on hover) or Pluto's eye in the margin
// folds it at once and it stays folded until Claude edits it again.

import { on, type CellState } from "./bridge";
import { onRedraw } from "./redraw";

const css = `
  pluto-cell[data-endeavor-fold] > pluto-input { display: block !important; opacity: 1 !important; }
  pluto-cell[data-endeavor-fold="closing"] > pluto-input { overflow: hidden; transition: height 200ms ease; }
  pluto-cell[data-endeavor-fold="closing"] > .endeavor-fold-tag,
  pluto-cell[data-endeavor-fold] > pluto-input > .preview_hidden_code_info { display: none !important; }
  @media (any-pointer: fine) {
    pluto-cell[data-endeavor-fold] > pluto-shoulder > button.foldcode { opacity: 0.6; }
  }
  pluto-cell > .endeavor-fold-tag {
    position: absolute; right: 6px; z-index: 31; display: flex; align-items: center; gap: 5px;
    height: 20px; padding: 0 7px; border-radius: 4px; border: 0; cursor: pointer;
    background: var(--e-bg-tag); color: var(--e-text-tag); font: 11px/20px system-ui, sans-serif; user-select: none;
  }
  .endeavor-fold-tag svg { width: 12px; height: 12px; flex: none; }
  .endeavor-fold-tag .now { display: none; }
  .endeavor-fold-tag:hover { color: var(--e-text-primary); }
  .endeavor-fold-tag:hover .label { display: none; }
  .endeavor-fold-tag:hover .now { display: inline; }
`;

const EYE_OFF = `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" aria-hidden="true"><path d="M2 2l12 12"/><path d="M6.6 3.6A6.6 6.6 0 0 1 8 3.5c3.5 0 6 4.5 6 4.5a11 11 0 0 1-1.7 2.2M10.4 11.6A5.4 5.4 0 0 1 8 12.5C4.5 12.5 2 8 2 8a11 11 0 0 1 2.5-2.9"/><path d="M6.6 6.6a2 2 0 0 0 2.8 2.8"/></svg>`;

/** Shown open: a run that failed since Claude's edit says so in the tag. */
const open = new Map<string, { failed: boolean }>();
/** Folded by the user while still unrun: the code version they folded. */
const dismissed = new Map<string, string | undefined>();
let states = new Map<string, CellState>();

const reduced = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

function update() {
  for (const cell of document.querySelectorAll<HTMLElement>("pluto-cell")) {
    const id = cell.id;
    const state = states.get(id);
    const edited = !!state?.unrun && state.author === "agent";
    if (edited && dismissed.has(id) && dismissed.get(id) !== state?.version) dismissed.delete(id);
    if (!edited && !open.has(id)) dismissed.delete(id);
    if (cell.getAttribute("data-endeavor-fold") === "closing") continue;
    const folded = cell.classList.contains("code_folded");
    if (edited && folded && !dismissed.has(id)) {
      const kept = open.get(id);
      if (kept) kept.failed = false;
      else open.set(id, { failed: false });
    } else if (open.has(id) && (!folded || dismissed.has(id))) {
      open.delete(id);
    } else if (open.has(id) && state && !edited && !state.running) {
      if (state.errored) open.get(id)!.failed = true;
      else {
        open.delete(id);
        close(cell);
        continue;
      }
    }
    show(cell);
  }
}

function show(cell: HTMLElement) {
  const kept = open.get(cell.id);
  const input = cell.querySelector<HTMLElement>(":scope > pluto-input");
  let tag = cell.querySelector<HTMLElement>(":scope > .endeavor-fold-tag");
  if (!kept) {
    if (cell.hasAttribute("data-endeavor-fold")) cell.removeAttribute("data-endeavor-fold");
    tag?.remove();
    return;
  }
  if (cell.getAttribute("data-endeavor-fold") !== "open") cell.setAttribute("data-endeavor-fold", "open");
  if (!input) return;
  const label = kept.failed ? "Folded · shows until it runs without error" : "Folded · shows until it runs";
  if (!tag) {
    // Not a <button>: Pluto hides its own buttons in the editor until hover.
    tag = document.createElement("div");
    tag.setAttribute("role", "button");
    tag.tabIndex = 0;
    tag.className = "endeavor-fold-tag";
    tag.dataset.endeavorUi = "";
    tag.innerHTML = `${EYE_OFF}<span class="label"></span><span class="now">Fold now</span>`;
    tag.onclick = (e) => {
      e.preventDefault();
      e.stopPropagation();
      foldNow(cell.id);
    };
    tag.onkeydown = (e) => {
      if (e.key !== "Enter" && e.key !== " ") return;
      e.preventDefault();
      foldNow(cell.id);
    };
    cell.append(tag);
  }
  // On the cell, not in Pluto's editor element: at the code's top-right.
  const top = `${input.offsetTop + 5}px`;
  if (tag.style.top !== top) tag.style.top = top;
  const span = tag.querySelector<HTMLElement>(".label")!;
  if (span.textContent !== label) span.textContent = label;
  tag.title = label;
}

/** Fold a clean-run cell: its code closes over 200 ms, and the cells below move up. */
function close(cell: HTMLElement) {
  const input = cell.querySelector<HTMLElement>(":scope > pluto-input");
  if (!input || reduced()) return show(cell);
  const height = input.offsetHeight;
  cell.setAttribute("data-endeavor-fold", "closing");
  input.style.height = `${height}px`;
  void input.offsetHeight;
  input.style.height = "0px";
  setTimeout(() => {
    input.style.removeProperty("height");
    if (cell.getAttribute("data-endeavor-fold") === "closing") cell.removeAttribute("data-endeavor-fold");
    cell.querySelector(":scope > .endeavor-fold-tag")?.remove();
  }, 200);
}

function foldNow(id: string) {
  if (!open.delete(id)) return;
  dismissed.set(id, states.get(id)?.version);
  const cell = document.getElementById(id);
  if (cell) show(cell);
}

export function initFolded(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("cells", (msg) => {
    states = new Map(msg.cells.map((c) => [c.cell_id, c]));
    update();
  });
  // Pluto's eye on a cell shown open folds it, as "Fold now" does.
  document.addEventListener(
    "click",
    (e) => {
      const eye = (e.target as Element | null)?.closest?.("pluto-shoulder > button.foldcode");
      const cell = eye?.closest("pluto-cell");
      if (!cell || cell.getAttribute("data-endeavor-fold") !== "open") return;
      e.preventDefault();
      e.stopPropagation();
      foldNow(cell.id);
    },
    true,
  );
  onRedraw(update);
}
