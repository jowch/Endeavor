import { on, type CellState } from "./bridge";
import { redrawRail } from "./rail";
import { onRedraw } from "./redraw";

export const outranksTrafficLight = "pluto-editor:not(.___):not(.____)";
const bar = `${outranksTrafficLight} pluto-cell[data-endeavor-bar]`;
const css = `
  pluto-cell { position: relative; }
  pluto-cell[data-endeavor="unrun"] > pluto-output { opacity: 0.4; }
  ${bar} > pluto-trafficlight {
    background: var(--e-accent); border-left-color: var(--e-accent); background-clip: padding-box;
  }
  ${bar}.queued > pluto-trafficlight, ${bar}.running > pluto-trafficlight {
    background: var(--e-stripe-tint); border-left-color: var(--e-stripe-tint);
  }
  ${bar}.queued > pluto-trafficlight::after {
    background: repeating-linear-gradient(-45deg, transparent, transparent 8px, var(--e-accent) 8px, var(--e-accent) 16px);
    background-clip: padding-box; opacity: 0.99; background-size: 4px var(--patternHeight);
  }
  ${bar}.running > pluto-trafficlight::after {
    background: repeating-linear-gradient(-45deg, var(--e-accent), var(--e-accent) 8px, var(--e-stripe-tint) 8px, var(--e-stripe-tint) 16px);
    background-clip: content-box; opacity: 0.99; background-size: 4px var(--patternHeight);
  }
`;

const START_WAIT = 3000;

let states = new Map<string, CellState>();
let asked: string[] = [];
const approved = new Map<string, { since: number; started: boolean }>();

function touched(cell: HTMLElement): boolean {
  const state = states.get(cell.id);
  if (state?.unrun && state.author === "agent") return true;
  if (asked.includes(cell.id)) return true;
  const run = approved.get(cell.id);
  if (!run) return false;
  if (cell.classList.contains("queued") || cell.classList.contains("running")) run.started = true;
  else if (run.started || Date.now() - run.since > START_WAIT) approved.delete(cell.id);
  return approved.has(cell.id);
}

let watching = 0;
function watchApproved() {
  if (watching || !approved.size) return;
  watching = window.setInterval(() => {
    apply();
    redrawRail();
    if (!approved.size) {
      clearInterval(watching);
      watching = 0;
    }
  }, 250);
}

function apply() {
  for (const cell of document.querySelectorAll<HTMLElement>("pluto-cell")) {
    const state = states.get(cell.id);
    setAttr(cell, "data-endeavor", state?.unrun ? "unrun" : null);
    setAttr(cell, "data-author", state?.author ?? null);
    setAttr(cell, "data-endeavor-bar", touched(cell) ? "claude" : null);
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
  on("context", (msg) => {
    const now = msg.ask_cells ?? [];
    for (const id of asked) if (!now.includes(id)) approved.set(id, { since: Date.now(), started: false });
    for (const id of now) approved.delete(id);
    asked = now;
    apply();
    redrawRail();
    watchApproved();
  });
  onRedraw(apply);
}
