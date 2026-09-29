// "Fix with Claude" and "Explain" on Pluto's error boxes (docs/ui-spec.md,
// Errors); Pluto's own "Fix with AI" is off in the runtime. A click asks the
// agent about that cell's error, through the same queue as the chat box.

import { byUser, send } from "./bridge";
import { onRedraw } from "./redraw";
import { cellCode } from "./reveal";

// ponytail: "Claude" until the agent's name reaches the page (ACP agent info).
const AGENT = "Claude";

const css = `
  .endeavor-ask { display: flex; gap: 8px; margin: 8px 0; }
  .endeavor-ask button { font: 12px system-ui; padding: 3px 10px; border-radius: 4px; cursor: pointer;
    background: transparent; color: var(--e-accent-text); border: 1px solid var(--e-accent); }
  .endeavor-ask button.explain { color: var(--e-text-secondary); border-color: var(--e-control-edge); }
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
      send({ type: "ask", kind, notebook, cell: cell.id, code: cellCode(cell), error: text });
    };
    row.querySelector<HTMLButtonElement>(".fix")!.onclick = (e) => byUser(e) && ask("fix");
    row.querySelector<HTMLButtonElement>(".explain")!.onclick = (e) => byUser(e) && ask("explain");
    // At the bottom of the error card, after the message and trace.
    error.append(row);
  }
}

export function initErrors(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  onRedraw(decorate);
}
