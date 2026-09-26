// Cell states on Pluto's own cells (docs/ui-spec.md, "Cell states"): a cell the
// agent edited and nobody has run since gets data-endeavor="unrun" and a striped
// gutter bar; its output is dimmed as stale. data-author says who last changed
// the code. The user's own unsubmitted edits are Pluto's `code_differs` and get
// the grey stripe. Pluto redraws cells, so the attributes are re-applied.

import { on, type CellState } from "./bridge";
import { redrawRail } from "./rail";
import { onRedraw } from "./redraw";

const css = `
  pluto-cell { position: relative; }
  pluto-cell[data-endeavor="unrun"]::before, pluto-cell.code_differs::before {
    content: ""; position: absolute; left: -8px; top: 0; bottom: 0; width: 4px;
    border-radius: 2px; pointer-events: none;
    background: repeating-linear-gradient(-45deg, #CC3F00 0 3px, rgba(204, 63, 0, 0.3) 3px 6px);
  }
  pluto-cell[data-endeavor="unrun"][data-author="user"]::before, pluto-cell.code_differs::before {
    background: repeating-linear-gradient(-45deg, #9A9A9A 0 3px, rgba(154, 154, 154, 0.25) 3px 6px);
  }
  pluto-cell[data-endeavor="unrun"] > pluto-output { opacity: 0.4; }
  /* Code the agent changed stays in view until it runs, even in a folded cell. */
  pluto-cell[data-endeavor="unrun"][data-author="agent"] > pluto-input { display: block !important; opacity: 1 !important; }
`;

let states = new Map<string, CellState>();

function apply() {
  for (const cell of document.querySelectorAll<HTMLElement>("pluto-cell")) {
    const state = states.get(cell.id);
    setAttr(cell, "data-endeavor", state?.unrun ? "unrun" : null);
    setAttr(cell, "data-author", state?.author ?? null);
  }
}

function setAttr(el: HTMLElement, name: string, value: string | null) {
  if (value === null) {
    if (el.hasAttribute(name)) el.removeAttribute(name);
  } else if (el.getAttribute(name) !== value) {
    el.setAttribute(name, value);
  }
}

export function initCells(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("cells", (msg) => {
    states = new Map(msg.cells.map((c) => [c.cell_id, c]));
    apply();
    redrawRail();
  });
  onRedraw(apply);
}
