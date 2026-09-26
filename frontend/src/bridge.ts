// The channel between the notebook page and the app, both directions.
// Page → app: `send`, over wry's `window.ipc` (parsed by src/annotate.rs).
// App → page: the app evaluates `window.__endeavor.receive(<json>)`; `on`
// registers a handler per message type.

/** Messages the page sends the app. */
export type ToApp =
  | { type: "ready" }
  | { type: "mode"; on: boolean }
  | { type: "annotation"; notebook: string | null; cells: string[]; comment: string; now: boolean }
  // Fix with Claude / Explain on a cell's error.
  | { type: "ask"; kind: "fix" | "explain"; notebook: string | null; cell: string; error: string }
  // ⌘K on a cell / the agent button between cells: about this cell, fill this
  // empty cell, or add a new cell after it. `now` (⌘⏎) joins a running turn.
  | { type: "prompt"; notebook: string | null; cell: string; where: "about" | "fill" | "after"; text: string; now: boolean };

/** One cell's state, from the runtime's events (see runtime Events.jl). */
export type CellState = {
  cell_id: string;
  running: boolean;
  errored: boolean;
  unrun: boolean;
  author: "agent" | "user" | null;
  /** An unrun cell's code before the agent's edits ("" if the agent added it). */
  before?: string | null;
};

/** Messages the app sends the page. */
export type ToPage =
  | { type: "annotate"; on: boolean }
  // The shown notebook's cells, whenever they change and after `ready`.
  | { type: "cells"; cells: CellState[] }
  // The notebook theme (Settings), after `ready` and on change.
  | { type: "theme"; name: "endeavor" | "pluto" };

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
    __endeavor?: { receive(msg: ToPage): void };
  }
}

const handlers: Record<string, Array<(msg: ToPage) => void>> = {};

// The page also runs notebook output JS, which could post messages that become
// prompts. So the app substitutes a per-launch secret for __ENDEAVOR_NONCE__ and
// drops messages without it. This is an initialization script, so it runs before
// any page script: the secret and the captured postMessage stay in this closure,
// out of reach of code that later patches window.ipc or webkit's handler.
declare const __ENDEAVOR_NONCE__: string;
const nonce = typeof __ENDEAVOR_NONCE__ === "string" ? __ENDEAVOR_NONCE__ : "";
const handler = (window as any).webkit?.messageHandlers?.ipc;
const post: (body: string) => void = handler ? handler.postMessage.bind(handler) : (body) => window.ipc?.postMessage(body);

export function send(msg: ToApp): void {
  post(JSON.stringify(nonce ? { ...msg, nonce } : msg));
}

/** Only the user's own clicks and keys may send prompts, not events a page
 * script synthesizes. (Tests run without a secret and can't make trusted events.) */
export const byUser = (e: Event): boolean => e.isTrusted || !nonce;

/** Several features may listen to the same message (e.g. "cells"). */
export function on<T extends ToPage["type"]>(type: T, handler: (msg: Extract<ToPage, { type: T }>) => void): void {
  (handlers[type] ??= []).push(handler as (msg: ToPage) => void);
}

window.__endeavor = {
  receive(msg) {
    handlers[msg.type]?.forEach((handler) => handler(msg));
  },
};
