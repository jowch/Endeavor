import { on, send } from "./bridge";
import { changedCells } from "./runguard";
import { statusModel, type NotebookLike, type StatusModel } from "./status";

export type Drawer = "docs" | "packages" | null;

export interface Context {
  host: string;
  agent: string;
  asking: boolean;
  crash: { title: string; body: string } | null;
  notebook: string;
}

type Listener = (nb: NotebookLike, model: StatusModel) => void;

const listeners: Listener[] = [];
let last: NotebookLike | null = null;
let model: StatusModel | null = null;
let lastSent = "";
export const context: Context = { host: "This Mac", agent: "Claude", asking: false, crash: null, notebook: "" };
let drawerOf: () => Drawer = () => null;

export function onNotebook(listener: Listener): void {
  listeners.push(listener);
  if (last && model) listener(last, model);
}

export function current(): { nb: NotebookLike; model: StatusModel } | null {
  return last && model ? { nb: last, model } : null;
}

export const notebookId = (): string => new URLSearchParams(location.search).get("id") ?? context.notebook;

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
    installing: model.install !== null,
    restart: model.restart,
    save_failed: model.saveFailed,
    package_failed: model.failure?.name ?? null,
    dead: last.process_status === "no_process",
    connected: editor?.connected !== false,
    unsaved: editor ? changedCells(editor).length > 0 : false,
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

const TICK_MS = 250;

function frame() {
  ticks.forEach((t) => t());
  setTimeout(() => requestAnimationFrame(frame), TICK_MS);
}

export function initState(): void {
  every(250, tick);
  if (document.querySelector("pluto-editor")) requestAnimationFrame(frame);
  on("context", (msg) => {
    context.host = msg.host;
    context.agent = msg.agent ?? "Claude";
    context.asking = msg.asking;
    context.crash = msg.crash ?? null;
    context.notebook = msg.notebook ?? "";
    if (last && model) listeners.forEach((l) => l(last!, model!));
  });
  lastSent = "";
}
