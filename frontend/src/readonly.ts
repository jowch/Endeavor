import { on } from "./bridge";

const CLASS = "endeavor-readonly";
let readonly = false;

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

holdReconnects();

export function initReadonly(): void {
  style();
  window.addEventListener("keydown", blockKeys, true);
  on("context", (msg) => setReadonly(msg.readonly));
}
