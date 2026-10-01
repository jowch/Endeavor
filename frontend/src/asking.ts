// The chat's card asks to run cells (docs/ui-spec.md, "Approval card"): the
// asked-about cells get an orange line "Claude asks to run this." and the
// accent stripe; the cells that would re-run after them "Re-runs after it"
// in grey; and the cells it needs that never ran "Runs first: it hasn't run
// yet", also in grey. The page tells the app whether an asked-about cell is
// on screen, so the card offers "Show in notebook" only when it isn't.

import { on, send } from "./bridge";
import { onRedraw } from "./redraw";

const css = `
  pluto-cell[data-endeavor-ask] { margin-top: 26px; }
  pluto-cell[data-endeavor-ask]::after {
    position: absolute; left: 0; top: -22px; font: 12px/18px var(--sans-serif-font-stack, system-ui); pointer-events: none;
  }
  pluto-cell[data-endeavor-ask="asks"]::after { content: "\\25CF  Claude asks to run this."; color: var(--e-accent-text); }
  pluto-cell[data-endeavor-ask="reruns"]::after { content: "Re-runs after it"; color: var(--e-text-muted); }
  pluto-cell[data-endeavor-ask="needed"]::after { content: "Runs first: it hasn't run yet"; color: var(--e-text-muted); }
  pluto-cell[data-endeavor-ask]::before {
    content: ""; position: absolute; left: -8px; top: 0; bottom: 0; width: 4px;
    border-radius: 2px; pointer-events: none;
    background: repeating-linear-gradient(-45deg, var(--e-accent) 0 3px, var(--e-stripe-tint) 3px 6px);
  }
`;

let asked: string[] = [];
let rerun: string[] = [];
let needed: string[] = [];
let observer: IntersectionObserver | null = null;
const onScreen = new Set<string>();
let lastSent: boolean | null = null;

function apply() {
  for (const cell of document.querySelectorAll<HTMLElement>("pluto-cell")) {
    const mark = asked.includes(cell.id) ? "asks" : rerun.includes(cell.id) ? "reruns" : needed.includes(cell.id) ? "needed" : null;
    if (mark === null) {
      if (cell.hasAttribute("data-endeavor-ask")) cell.removeAttribute("data-endeavor-ask");
    } else if (cell.getAttribute("data-endeavor-ask") !== mark) {
      cell.setAttribute("data-endeavor-ask", mark);
    }
  }
}

function report() {
  if (!asked.some((id) => document.getElementById(id))) return;
  const visible = asked.some((id) => onScreen.has(id));
  if (visible === lastSent) return;
  lastSent = visible;
  send({ type: "asked_visible", visible });
}

function watch() {
  observer?.disconnect();
  onScreen.clear();
  lastSent = null;
  if (typeof IntersectionObserver === "undefined") return;
  observer = new IntersectionObserver((entries) => {
    for (const e of entries) (e.isIntersecting ? onScreen.add(e.target.id) : onScreen.delete(e.target.id));
    report();
  });
  for (const id of asked) {
    const cell = document.getElementById(id);
    if (cell) observer.observe(cell);
  }
}

export function initAsking(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("context", (msg) => {
    const cells = msg.ask_cells ?? [];
    const changed = cells.join() !== asked.join();
    asked = cells;
    rerun = (msg.rerun_cells ?? []).filter((id) => !cells.includes(id));
    needed = (msg.needed_ids ?? []).filter((id) => !cells.includes(id) && !rerun.includes(id));
    apply();
    if (changed) watch();
  });
  onRedraw(apply);
}
