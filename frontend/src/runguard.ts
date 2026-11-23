import { byUser, on, send } from "./bridge";
import { modHeld } from "./keys";
import { cellCode } from "./reveal";
import { context, notebookId } from "./state";
import { definedNames } from "./status";

export type Waiting = { id: string; name?: string | null };

export type Dependencies = Record<string, { downstream_cells_map?: Record<string, string[]> } | undefined>;

export function reach(start: string[], deps: Dependencies): string[] {
  const seen = new Set<string>();
  const todo = [...start];
  while (todo.length) {
    const id = todo.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    for (const next of Object.values(deps[id]?.downstream_cells_map ?? {}).flat()) todo.push(next);
  }
  return [...seen];
}

const css = `
  #endeavor-runguard { position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%); z-index: 210;
    width: min(440px, calc(100vw - 32px)); box-sizing: border-box; padding: 14px 16px; border-radius: 10px;
    border: 1px solid var(--e-dialog-edge); background: var(--e-dialog-bg); box-shadow: 0 16px 48px var(--e-dialog-shadow);
    font: 13px/1.5 system-ui, -apple-system, sans-serif; color: var(--e-text-secondary); }
  #endeavor-runguard b { display: block; margin-bottom: 4px; font-weight: 600; font-size: 13.5px; color: var(--e-text-primary); }
  #endeavor-runguard code { font: 600 12.5px JuliaMono, ui-monospace, monospace; color: var(--e-text-primary); }
  #endeavor-runguard button.show code { color: inherit; }
  #endeavor-runguard .buttons { display: flex; align-items: center; gap: 8px; margin-top: 12px; }
  #endeavor-runguard .gap { flex: 1; }
  #endeavor-runguard button { padding: 4px 12px; border-radius: 5px; border: 1px solid var(--e-control-edge); background: var(--e-bg-raised);
    color: var(--e-text-primary); font: 12.5px system-ui, sans-serif; cursor: pointer; }
  #endeavor-runguard button.show { border-color: transparent; background: none; color: var(--e-accent-text); padding: 4px 6px; }
  #endeavor-runguard button.primary { background: var(--e-accent); border-color: var(--e-accent); color: #fff; }
  #endeavor-runguard .hint { color: var(--e-text-dim); font-size: 11.5px; margin-left: -2px; }
  pluto-cell.endeavor-guard-show { outline: 2px solid var(--e-accent); outline-offset: 4px; border-radius: 4px; }
`;

let waiting: Waiting[] = [];
let replaying = false;
let shown: { el: HTMLElement; hit: Waiting[]; replay: () => void } | null = null;

const escape = (s: string) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

type EditorState = {
  notebook?: {
    process_status?: string;
    cell_order?: string[];
    cell_inputs?: Record<string, { code?: string } | undefined>;
    cell_results?: Record<string, { queued?: boolean; running?: boolean; last_run_timestamp?: number } | undefined>;
    cell_dependencies?: Dependencies;
  };
  cell_inputs_local?: Record<string, { code?: string } | undefined>;
  selected_cells?: string[];
};

const editor = (): EditorState | undefined => (window as any).editor_state;

export function changedCells(st: EditorState): string[] {
  const local = st.cell_inputs_local ?? {};
  const remote = st.notebook?.cell_inputs ?? {};
  return (st.notebook?.cell_order ?? []).filter((id) => local[id] != null && remote[id]?.code !== local[id]?.code);
}

function differs(st: EditorState, cell: HTMLElement): boolean {
  const text = st.cell_inputs_local?.[cell.id]?.code ?? cellCode(cell);
  return text !== st.notebook?.cell_inputs?.[cell.id]?.code;
}

export function runStart(e: Event, st: EditorState): { start: string[]; changes: boolean } | null {
  const target = e.target instanceof Element ? e.target : null;
  const selected = st.selected_cells ?? [];
  if (e.type === "click") {
    const button = target?.closest("pluto-runarea.run button.runcell");
    const cell = button?.closest<HTMLElement>("pluto-cell");
    if (!cell) return null;
    return { start: selected.includes(cell.id) ? selected : [cell.id], changes: false };
  }
  if (e.type !== "keydown") return null;
  const k = e as KeyboardEvent;
  const inEditor = target?.closest("pluto-input") ? target.closest<HTMLElement>("pluto-cell") : null;
  if (k.key?.toLowerCase() === "s" && modHeld(k) && !k.shiftKey && !k.altKey) {
    const changed = changedCells(st);
    return changed.length ? { start: changed, changes: true } : null;
  }
  if (k.key !== "Enter") return null;
  if (k.shiftKey && !k.metaKey && !k.ctrlKey && !k.altKey) {
    const start = [...new Set([...(inEditor ? [inEditor.id] : []), ...selected])];
    return start.length ? { start, changes: false } : null;
  }
  if ((k.metaKey || k.ctrlKey) && !k.shiftKey && !k.altKey && inEditor) {
    return differs(st, inEditor) ? { start: [inEditor.id], changes: true } : null;
  }
  return null;
}

function replayOf(e: Event): () => void {
  const target = e.target as HTMLElement;
  if (e.type === "click") return () => target.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  const k = e as KeyboardEvent;
  const init = { key: k.key, code: k.code, shiftKey: k.shiftKey, ctrlKey: k.ctrlKey, metaKey: k.metaKey, altKey: k.altKey, bubbles: true, cancelable: true };
  return () => {
    target.focus?.();
    target.dispatchEvent(new KeyboardEvent("keydown", init));
  };
}

const nameOf = (w: Waiting, deps: Dependencies): string => w.name || definedNames(deps[w.id]?.downstream_cells_map)[0] || "a cell";

function names(list: string[]): string {
  const code = list.map((n) => `<code>${escape(n)}</code>`);
  return code.length <= 1 ? code.join("") : `${code.slice(0, -1).join(", ")} and ${code[code.length - 1]}`;
}

function outline(on: boolean) {
  for (const cell of document.querySelectorAll("pluto-cell.endeavor-guard-show")) cell.classList.remove("endeavor-guard-show");
  if (!on || !shown) return;
  const cells = shown.hit.map((w) => document.getElementById(w.id)).filter((c): c is HTMLElement => !!c);
  cells[0]?.scrollIntoView?.({ block: "center", behavior: "smooth" });
  for (const cell of cells) cell.classList.add("endeavor-guard-show");
}

function close() {
  outline(false);
  shown?.el.remove();
  shown = null;
}

function cancel() {
  close();
}

function runAnyway() {
  if (!shown) return;
  const { hit, replay } = shown;
  close();
  const ids = hit.map((w) => w.id);
  const results = () => editor()?.notebook?.cell_results ?? {};
  const before = Object.fromEntries(ids.map((id) => [id, results()[id]?.last_run_timestamp]));
  replaying = true;
  try {
    replay();
  } finally {
    replaying = false;
  }
  const started = Date.now();
  const answer = () => {
    const now = results();
    const under_way = ids.every((id) => now[id]?.queued || now[id]?.running || now[id]?.last_run_timestamp !== before[id]);
    if (under_way || Date.now() - started > 3000) {
      send({ type: "run_anyway", notebook: notebookId(), cells: ids.map((id) => ({ id, last_run: before[id] ?? 0 })) });
    } else setTimeout(answer, 50);
  };
  answer();
}

function show(hit: Waiting[], changes: boolean, replay: () => void, deps: Dependencies) {
  close();
  const list = hit.map((w) => nameOf(w, deps));
  const one = list.length === 1;
  const el = document.createElement("div");
  el.id = "endeavor-runguard";
  el.dataset.endeavorUi = "";
  el.setAttribute("role", "dialog");
  el.setAttribute("aria-label", `${context.agent} is waiting for your answer`);
  const what = changes ? "your changes" : "this";
  el.innerHTML =
    `<b>${context.agent} is waiting for your answer on ${names(list)}.</b>` +
    `Running ${what} now also runs ${one ? "it" : "them"}.` +
    `<div class="buttons"><button class="show">Show ${one ? names(list) : "them"}</button><span class="gap"></span>` +
    `<button class="cancel">Cancel</button><span class="hint">esc</span><button class="primary run">Run anyway</button></div>`;
  el.querySelector<HTMLButtonElement>(".show")!.onclick = () => outline(true);
  el.querySelector<HTMLButtonElement>(".cancel")!.onclick = cancel;
  el.querySelector<HTMLButtonElement>(".run")!.onclick = (e) => byUser(e) && runAnyway();
  document.body.append(el);
  shown = { el, hit, replay };
  el.querySelector<HTMLButtonElement>(".run")!.focus();
}

const inSafePreviewWhereSavingRunsNothing = (st: EditorState) => st.notebook?.process_status === "waiting_for_permission";

function guard(e: Event) {
  if (replaying || !waiting.length || !byUser(e)) return;
  const st = editor();
  if (!st?.notebook || inSafePreviewWhereSavingRunsNothing(st)) return;
  const run = runStart(e, st);
  if (!run) return;
  const deps = st.notebook.cell_dependencies ?? {};
  const reached = new Set(reach(run.start, deps));
  const hit = waiting.filter((w) => reached.has(w.id));
  if (!hit.length) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  show(hit, run.changes, replayOf(e), deps);
}

function onKey(e: KeyboardEvent) {
  if (shown && byUser(e) && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
    if (e.key === "Escape" || e.key === "Enter") {
      e.preventDefault();
      e.stopImmediatePropagation();
      return e.key === "Escape" ? cancel() : runAnyway();
    }
  }
  guard(e);
}

export function initRunGuard(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("context", (msg) => {
    waiting = msg.waiting_runs ?? [];
  });
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("click", guard, true);
}
