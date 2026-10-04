// Pluto's own state for the notebook: the Editor puts it on window.editor_state
// after every update (a new object each time, so a changed reference means a
// change). Features that follow it (the drawer, the safe-preview callout)
// register here, and the app's header gets a summary as `state` messages.

import { on, send } from "./bridge";
import { statusModel, type NotebookLike, type StatusModel } from "./status";

export type Drawer = "docs" | "status" | null;

export interface Context {
  /** Where the notebook runs ("This Mac", a server's name). */
  host: string;
  /** The session's agent ("Claude", "Codex"), for page text that names it. */
  agent: string;
  /** The agent is asking to run the notebook (the chat's card is up). */
  asking: boolean;
  /** Julia stopped again while the notebook ran after a restart: the safe-preview callout's words. */
  crash: { title: string; body: string } | null;
}

type Listener = (nb: NotebookLike, model: StatusModel) => void;

const listeners: Listener[] = [];
let last: NotebookLike | null = null;
let model: StatusModel | null = null;
let lastSent = "";
export const context: Context = { host: "This Mac", agent: "Claude", asking: false, crash: null };
let drawerOf: () => Drawer = () => null;

export function onNotebook(listener: Listener): void {
  listeners.push(listener);
  if (last && model) listener(last, model);
}

export function current(): { nb: NotebookLike; model: StatusModel } | null {
  return last && model ? { nb: last, model } : null;
}

export const notebookId = (): string => new URLSearchParams(location.search).get("id") ?? "";

/** The drawer reports its tab through this, so the header's buttons follow it. */
export function setDrawerSource(source: () => Drawer): void {
  drawerOf = source;
}

export function report(): void {
  const editor = (window as any).editor_state;
  if (!last || !model) return;
  const msg = {
    type: "state" as const,
    notebook: notebookId(),
    safe: last.process_status === "waiting_for_permission",
    busy: model.busy,
    restart: model.restart,
    save_failed: model.saveFailed,
    package_failed: model.failure?.name ?? null,
    dead: last.process_status === "no_process",
    connected: editor?.connected !== false,
    drawer: drawerOf(),
  };
  const json = JSON.stringify(msg);
  if (json === lastSent) return;
  lastSent = json;
  send(msg);
}

function tick() {
  const nb = (window as any).editor_state?.notebook as NotebookLike | undefined;
  if (!nb?.cell_order || nb === last) return report();
  last = nb;
  model = statusModel(nb);
  for (const listener of listeners) listener(nb, model);
  report();
}

const ticks: Array<() => void> = [];

/** Run `hook` about every `ms` while the page shows (animation frames; none in tests). */
export function every(ms: number, hook: () => void): void {
  let lastRun = 0;
  ticks.push(() => {
    const now = performance.now();
    if (now - lastRun >= ms) {
      lastRun = now;
      hook();
    }
  });
}

function frame() {
  ticks.forEach((t) => t());
  requestAnimationFrame(frame);
}

export function initState(): void {
  every(250, tick);
  // Only Pluto's editor page has state to follow (its HTML has the element from the start).
  if (document.querySelector("pluto-editor")) requestAnimationFrame(frame);
  on("context", (msg) => {
    context.host = msg.host;
    context.agent = msg.agent ?? "Claude";
    context.asking = msg.asking;
    context.crash = msg.crash ?? null;
    if (last && model) listeners.forEach((l) => l(last!, model!));
  });
  // A new page for another notebook reports afresh.
  lastSent = "";
}
