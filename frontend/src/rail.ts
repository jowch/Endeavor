// The overview rail (docs/ui-spec.md): a 3px strip on the notebook's right edge
// with a mark per unrun cell, placed by the cell's position in the notebook, so
// off-screen changes show too. Clicking a mark scrolls to its cell. Reads the
// marks cells.ts sets.

import { onRedraw } from "./redraw";

const css = `
  #endeavor-rail { position: fixed; right: 4px; top: 10px; bottom: 10px; width: 3px; z-index: 50; pointer-events: none; }
  #endeavor-rail a { position: absolute; left: 0; right: 0; min-height: 4px; border-radius: 2px;
    background: #CC3F00; pointer-events: auto; cursor: pointer; }
  #endeavor-rail a.user { background: #9A9A9A; }
`;

let rail: HTMLElement;

function draw() {
  const marked = [...document.querySelectorAll<HTMLElement>('pluto-cell[data-endeavor="unrun"], pluto-cell.code_differs')];
  const total = Math.max(document.documentElement.scrollHeight, 1);
  rail.replaceChildren(
    ...marked.map((cell) => {
      const mark = document.createElement("a");
      const top = cell.getBoundingClientRect().top + window.scrollY;
      mark.style.top = `${(100 * top) / total}%`;
      mark.style.height = `${(100 * cell.offsetHeight) / total}%`;
      if (cell.dataset.author === "user" || cell.classList.contains("code_differs")) mark.className = "user";
      mark.dataset.cell = cell.id;
      mark.onclick = (e) => {
        e.preventDefault();
        cell.scrollIntoView({ block: "center", behavior: "smooth" });
      };
      return mark;
    }),
  );
}

export function initRail(): void {
  const style = document.createElement("style");
  style.textContent = css;
  rail = document.createElement("div");
  rail.id = "endeavor-rail";
  rail.dataset.endeavorUi = "";
  document.head.append(style);
  document.body.append(rail);
  onRedraw(draw);
  window.addEventListener("resize", draw);
}

export const redrawRail = () => rail && draw();
