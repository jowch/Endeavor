// The channel between the notebook page and the app, both directions.
// Page → app: `send`, over wry's `window.ipc` (parsed by src/annotate.rs).
// App → page: the app evaluates `window.__endeavor.receive(<json>)`; `on`
// registers a handler per message type.

/** Messages the page sends the app. */
export type ToApp =
  | { type: "mode"; on: boolean }
  | { type: "annotation"; notebook: string | null; cells: string[]; comment: string; now: boolean };

/** Messages the app sends the page. */
export type ToPage = { type: "annotate"; on: boolean };

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
    __endeavor?: { receive(msg: ToPage): void };
  }
}

type Handlers = { [T in ToPage["type"]]?: (msg: Extract<ToPage, { type: T }>) => void };
const handlers: Handlers = {};

export function send(msg: ToApp): void {
  window.ipc?.postMessage(JSON.stringify(msg));
}

export function on<T extends ToPage["type"]>(type: T, handler: (msg: Extract<ToPage, { type: T }>) => void): void {
  (handlers as Record<string, unknown>)[type] = handler;
}

window.__endeavor = {
  receive(msg) {
    const handler = handlers[msg.type] as ((m: ToPage) => void) | undefined;
    handler?.(msg);
  },
};
