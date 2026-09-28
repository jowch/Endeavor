// While the notebook's server is out of reach, the page stays readable but
// takes no edits, and Pluto doesn't reconnect: the app loads the notebook
// afresh once the server is back, and a page reconnecting to a restarted
// Julia (a new secret) would only be refused with a "lost authentication" alert.

import { on } from "./bridge";

const CLASS = "endeavor-readonly";
let readonly = false;

/** Pluto's reconnects go nowhere while read-only. */
function holdReconnects(): void {
  const Real = window.WebSocket;
  const Held = function (this: unknown, url: string | URL, protocols?: string | string[]) {
    return new Real(readonly ? "ws://127.0.0.1:9/" : url, protocols);
  } as unknown as typeof WebSocket;
  Held.prototype = Real.prototype;
  Object.assign(Held, { CONNECTING: Real.CONNECTING, OPEN: Real.OPEN, CLOSING: Real.CLOSING, CLOSED: Real.CLOSED });
  window.WebSocket = Held;
}

function style(): void {
  const css = document.createElement("style");
  css.textContent = `
    body.${CLASS} pluto-notebook { opacity: 0.85; }
    body.${CLASS} pluto-notebook, body.${CLASS} pluto-notebook * { pointer-events: none !important; }
  `;
  document.head.append(css);
}

/** No typing into a cell that had focus when the server went away. */
function blockKeys(e: KeyboardEvent): void {
  if (!readonly) return;
  const inNotebook = e.target instanceof Element && e.target.closest("pluto-notebook");
  const copying = (e.metaKey || e.ctrlKey) && (e.key === "c" || e.key === "a");
  if (inNotebook && !copying) {
    e.preventDefault();
    e.stopImmediatePropagation();
  }
}

export function setReadonly(on: boolean): void {
  readonly = on;
  document.body?.classList.toggle(CLASS, on);
  if (on && document.activeElement instanceof HTMLElement && document.activeElement.closest("pluto-notebook")) {
    document.activeElement.blur();
  }
}

// At load, before Pluto opens its first connection.
holdReconnects();

export function initReadonly(): void {
  style();
  window.addEventListener("keydown", blockKeys, true);
  on("context", (msg) => setReadonly(msg.readonly));
}
