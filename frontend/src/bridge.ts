import type { Pick } from "./quote";

export type ToApp =
  | { type: "ready" }
  | { type: "out_of_step" }
  | { type: "mode"; on: boolean }
  | { type: "quote"; notebook: string | null; picks: Pick[]; comment: string; add: boolean; ref?: number }
  | { type: "shoot"; id: number; rect: { x: number; y: number; width: number; height: number } }
  | { type: "ask"; kind: "fix" | "explain"; notebook: string | null; cell: string; code: string; error: string }
  | { type: "prompt"; notebook: string | null; cell: string; code: string; where: "about" | "fill" | "before" | "after"; text: string; add: boolean }
  | { type: "code"; cell: string; code: string | null }
  | {
      type: "state";
      notebook: string;
      safe: boolean;
      busy: string | null;
      installing: boolean;
      restart: "required" | "recommended" | null;
      save_failed: boolean;
      package_failed: string | null;
      dead: boolean;
      connected: boolean;
      drawer: "docs" | "packages" | null;
    }
  | { type: "asked_visible"; visible: boolean }
  | { type: "docs"; open: boolean }
  | { type: "run_notebook"; notebook: string }
  | { type: "run_anyway"; notebook: string; cells: { id: string; last_run: number }[] }
  | { type: "answer_card"; notebook: string; card: number }
  | { type: "fix_package"; notebook: string; name: string; log: string }
  | { type: "restart"; notebook: string }
  | { type: "error_ask_show"; cell: string }
  | { type: "error_ask_cancel"; cell: string }
  | {
      type: "debug";
      point: boolean;
      picked: string[];
      picks: string[];
      box: boolean;
      point_status: string;
      comment: string;
      drawer: "docs" | "packages" | null;
      callout: boolean;
      reply: "pill" | "prompt" | null;
      prompt: { kind: string; about: string; text: string } | null;
      alerts: string[] | null;
      dialogs: string[];
    };

export type CellState = {
  cell_id: string;
  running: boolean;
  errored: boolean;
  unrun: boolean;
  author: "agent" | "user" | null;
  before?: string | null;
  version?: string;
};

export type ErrorAsk = { cell: string; kind: "fix" | "explain"; queued: boolean };

export type ToPage =
  | { type: "annotate"; on: boolean }
  | { type: "quote_sent"; ref: number; ok: boolean }
  | { type: "cells"; cells: CellState[] }
  | { type: "theme"; name: "endeavor" | "pluto" | "view" }
  | { type: "reveal"; cells: string[] }
  | { type: "code"; cell: string }
  | { type: "shot"; id: number }
  | { type: "drawer"; tab: "docs" | "packages" | null }
  | {
      type: "context";
      host: string;
      agent?: string;
      asking: boolean;
      readonly: boolean;
      crash?: { title: string; body: string } | null;
      ask_cells?: string[];
      rerun_cells?: string[];
      needed_ids?: string[];
      working?: boolean;
      error_asks?: ErrorAsk[];
      waiting_runs?: Array<{ id: string; name?: string | null }>;
      card?: number | null;
      notebook?: string | null;
    }
  | { type: "action"; name: "present" | "record" | "frontmatter" | "shortcuts" | "feedback" }
  | { type: "debug" };

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
    __endeavor?: { receive(msg: ToPage): void };
  }
}

const handlers: Record<string, Array<(msg: ToPage) => void>> = {};

declare const __ENDEAVOR_NONCE__: string;
const nonce = typeof __ENDEAVOR_NONCE__ === "string" ? __ENDEAVOR_NONCE__ : "";
const handler = (window as any).webkit?.messageHandlers?.ipc;
const post: (body: string) => void = handler ? handler.postMessage.bind(handler) : (body) => window.ipc?.postMessage(body);

let host: ((msg: ToApp) => void) | null = null;

export function setHost(handler: (msg: ToApp) => void): void {
  host = handler;
}

export function send(msg: ToApp): void {
  if (host) return host(msg);
  post(JSON.stringify(nonce ? { ...msg, nonce } : msg));
}

export const byUser = (e: Event): boolean => e.isTrusted || (!nonce && !host);

export function on<T extends ToPage["type"]>(type: T, handler: (msg: Extract<ToPage, { type: T }>) => void): void {
  (handlers[type] ??= []).push(handler as (msg: ToPage) => void);
}

window.__endeavor = {
  receive(msg) {
    handlers[msg.type]?.forEach((handler) => handler(msg));
  },
};
