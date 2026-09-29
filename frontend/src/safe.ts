// Safe preview in the Endeavor look: one callout at the top of the notebook
// instead of Pluto's per-cell "Code not executed" labels and its header pill.
// Its Run notebook is the only one: it asks the app, which answers the agent's
// pending "Let this notebook run?" card if there is one (the same state), or
// lets the notebook run itself. Editing stays allowed, as in Pluto.

import { byUser, send } from "./bridge";
import { onRedraw } from "./redraw";
import { context, current, notebookId, onNotebook } from "./state";

const css = `
  #endeavor-safe { display: none; }
  html[data-endeavor-look="endeavor"] #endeavor-safe.shown { display: flex; }
  #endeavor-safe { gap: 10px; align-items: flex-start; margin: 0 0 20px 0; padding: 12px 14px;
    border: 1px solid var(--e-border); border-radius: 8px; background: var(--e-bg-card);
    font: 13px/1.5 system-ui, -apple-system, sans-serif; color: var(--e-text-secondary); }
  #endeavor-safe svg { flex: none; margin-top: 3px; color: var(--e-accent-text); }
  #endeavor-safe .text { flex: 1; }
  #endeavor-safe b { display: block; font-weight: 600; color: var(--e-text-primary); font-size: 13.5px; }
  #endeavor-safe .asking { color: var(--e-accent-text); margin-top: 4px; }
  #endeavor-safe .asking::before { content: ""; display: inline-block; width: 6px; height: 6px; border-radius: 3px;
    background: var(--e-accent); margin: 0 7px 1px 0; }
  #endeavor-safe button { flex: none; display: flex; align-items: center; gap: 6px; padding: 4px 10px; border-radius: 5px;
    border: 1px solid var(--e-control-edge); background: var(--e-bg-raised); color: var(--e-text-primary); font: 12.5px system-ui, sans-serif; cursor: pointer; }
  #endeavor-safe button:hover { background: var(--e-button-hover); }
  #endeavor-safe button svg { margin: 0; }
`;

const shield = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M7 1.2 12 3v3.6c0 3-2.2 5.2-5 6.2-2.8-1-5-3.2-5-6.2V3z"/></svg>`;
const play = `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" style="stroke: var(--e-accent-text)" stroke-width="1.3"><path d="M2.5 1.5v7l6-3.5z"/></svg>`;

let callout: HTMLElement;

function render() {
  const now = current();
  const safe = now?.nb.process_status === "waiting_for_permission";
  callout.classList.toggle("shown", !!safe);
  if (!safe) return;
  const asking = context.asking ? `<div class="asking">Claude is asking to run it. Answer in the chat, or here.</div>` : "";
  const html =
    `${shield}<div class="text"><b>Safe preview</b>` +
    `You're reading and editing this file without running any code.${asking}</div>` +
    `<button class="run">${play}Run notebook</button>`;
  if (callout.innerHTML !== html) {
    callout.innerHTML = html;
    callout.querySelector<HTMLButtonElement>(".run")!.onclick = (e) => byUser(e) && send({ type: "run_notebook", notebook: notebookId() });
  }
}

/** At the top of the notebook, above the first cell; Pluto redraws main, so it's put back. */
function place() {
  const notebook = document.querySelector("main pluto-notebook");
  if (notebook && callout.nextElementSibling !== notebook) notebook.before(callout);
}


export function initSafe(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  callout = document.createElement("div");
  callout.id = "endeavor-safe";
  callout.dataset.endeavorUi = "";
  onNotebook(render);
  onRedraw(place);
}
