// "Fix with Claude" and "Explain" on Pluto's error boxes (docs/ui-spec.md,
// Errors), replacing Pluto's own "Fix with AI". A click asks the agent about
// that cell's error, through the same queue as the chat box.

import { send } from "./bridge";
import { onRedraw } from "./redraw";

// ponytail: "Claude" until the agent's name reaches the page (ACP agent info).
const AGENT = "Claude";

const css = `
  .fix-with-ai { display: none !important; }
  .endeavor-ask { display: flex; gap: 8px; margin: 8px 0; }
  .endeavor-ask button { font: 12px system-ui; padding: 3px 10px; border-radius: 4px; cursor: pointer;
    background: transparent; color: #E08A5E; border: 1px solid #CC3F00; }
  .endeavor-ask button.explain { color: #BDBDBD; border-color: #3A3A40; }
`;

function decorate() {
  for (const error of document.querySelectorAll<HTMLElement>("pluto-cell jlerror")) {
    if (error.querySelector(".endeavor-ask")) continue;
    const cell = error.closest("pluto-cell");
    if (!cell) continue;
    const row = document.createElement("div");
    row.className = "endeavor-ask";
    row.innerHTML = `<button class="fix">Fix with ${AGENT}</button><button class="explain">Explain</button>`;
    const ask = (kind: "fix" | "explain") => {
      const text = (error.querySelector("header")?.textContent ?? error.textContent ?? "").trim().slice(0, 2000);
      const notebook = new URLSearchParams(location.search).get("id");
      send({ type: "ask", kind, notebook, cell: cell.id, error: text });
    };
    row.querySelector<HTMLButtonElement>(".fix")!.onclick = () => ask("fix");
    row.querySelector<HTMLButtonElement>(".explain")!.onclick = () => ask("explain");
    const header = error.querySelector(".error-header");
    header ? header.after(row) : error.prepend(row);
  }
}

export function initErrors(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  onRedraw(decorate);
}
