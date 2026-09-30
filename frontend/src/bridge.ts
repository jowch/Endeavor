// The channel between the notebook page and the app, both directions.
// Page → app: `send`, over wry's `window.ipc` (parsed by src/annotate.rs).
// App → page: the app evaluates `window.__endeavor.receive(<json>)`; `on`
// registers a handler per message type.

import type { Pick } from "./quote";

/** Messages the page sends the app. */
export type ToApp =
  | { type: "ready" }
  | { type: "mode"; on: boolean }
  // What Point picked, or a selection Reply quotes, with the user's comment:
  // sent now, or (`add`) added to the composer's message.
  | { type: "quote"; notebook: string | null; picks: Pick[]; comment: string; add: boolean }
  // Take a picture of this part of the viewport (CSS pixels) for a quote; the app answers "shot".
  | { type: "shoot"; id: number; rect: { x: number; y: number; width: number; height: number } }
  // Fix with Claude / Explain on a cell's error.
  | { type: "ask"; kind: "fix" | "explain"; notebook: string | null; cell: string; code: string; error: string }
  // ⌘K on a cell / the agent button between cells: about this cell, fill this
  // empty cell, or add a new cell after it. `now` (⌘⏎) joins a running turn.
  | { type: "prompt"; notebook: string | null; cell: string; code: string; where: "about" | "fill" | "before" | "after"; text: string; now: boolean }
  // A cell's code now, answering the app's `code` (null: no such cell here).
  | { type: "code"; cell: string; code: string | null }
  // The shown notebook's state for the app's header, whenever it changes.
  | {
      type: "state";
      notebook: string;
      safe: boolean;
      busy: string | null;
      restart: "required" | "recommended" | null;
      save_failed: boolean;
      package_failed: string | null;
      dead: boolean;
      connected: boolean;
      drawer: "docs" | "status" | null;
    }
  // Run notebook in the safe-preview callout.
  | { type: "run_notebook"; notebook: string }
  // The Status failure box: Fix with Claude (with the package's log), Restart notebook.
  | { type: "fix_package"; notebook: string; name: string; log: string }
  | { type: "restart"; notebook: string }
  // Answering `debug`: Point, its picks and their cells, the drawer's tab, the
  // safe-preview callout, and the `alert`s shown (null outside debug builds).
  | {
      type: "debug";
      point: boolean;
      picked: string[];
      // Each pick's source, as the app will name it ("rates · lines 2–3").
      picks: string[];
      box: boolean;
      point_status: string;
      comment: string;
      drawer: "docs" | "status" | null;
      callout: boolean;
      // Reply on a selection: its pill or its prompt, if either shows.
      reply: "pill" | "prompt" | null;
      alerts: string[] | null;
    };

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
  | { type: "theme"; name: "endeavor" | "pluto" }
  // Scroll to these cells and outline them briefly (a chip in the chat was clicked).
  | { type: "reveal"; cells: string[] }
  // Ask for a cell's code now.
  | { type: "code"; cell: string }
  // The app took picture `id` (or couldn't): the page shows its overlay again.
  | { type: "shot"; id: number }
  // The header's Live docs / Status buttons: open that tab, or shut the drawer (null).
  | { type: "drawer"; tab: "docs" | "status" | null }
  // What the safe-preview callout and Status say: where the notebook runs, and
  // whether the agent is asking to run it (the chat's card is up). `readonly`:
  // its server is out of reach, so the page can be read but not changed.
  | { type: "context"; host: string; asking: boolean; readonly: boolean }
  // Share and ⋮ items that act in the page, with Pluto's own functions.
  | { type: "action"; name: "present" | "record" | "frontmatter" | "shortcuts" | "feedback" }
  // A debug build's state dump asks what the page shows.
  | { type: "debug" };

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
