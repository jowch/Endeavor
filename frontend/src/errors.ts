// Pluto's error boxes (docs/ui-spec.md, "Errors"); Pluto's own "Fix with AI" is
// off in the runtime. Under the message, above the trace: "✦ Fix with Claude"
// and "Explain", which ask the agent about that cell's error through the same
// queue as the chat box, and a hint that ⌘E asks something else. After a
// click the buttons become one line until Claude's turn ends: "Claude is
// fixing this · Show in chat", or "Fix queued · … · Cancel" while it waits;
// the app says which (the context's `error_asks`). A cell that fails only
// because a cell above failed (Pluto's own test: an UndefVarError for a name
// a failed cell defines) gets no buttons, only "Fails because `fit` failed ·
// Show". The trace is folded to one line with its length.

import { byUser, on, send, type ErrorAsk } from "./bridge";
import { shortcut } from "./keys";
import { onRedraw } from "./redraw";
import { cellCode } from "./reveal";
import { context } from "./state";

/** At most this much of the error goes with the message. */
const MAX_ERROR = 2000;
/** A click shows its status line at once; the app's word replaces it, or it goes after this long. */
const PENDING = 3000;

const look = `html[data-endeavor-look="endeavor"]`;
// The grey edge is the only bar such a cell gets: Pluto's red one goes back to
// its normal look. The extra :not() outranks Pluto's `pluto-editor:not(.___)`.
const upstreamBar = `${look} pluto-editor:not(.___):not(.____) pluto-cell.errored[data-endeavor-error="upstream"]:not([data-endeavor-bar])`;
const css = `
  ${upstreamBar} > pluto-trafficlight { background: var(--normal-cell-color); border-left-color: var(--normal-cell-color); }
  ${upstreamBar}.selected > pluto-trafficlight { background: var(--selected-cell-color); border-left-color: var(--selected-cell-color); }
  ${upstreamBar}.code_differs > pluto-trafficlight { background: var(--code-differs-cell-color); border-left-color: var(--code-differs-cell-color); }
  ${look} pluto-cell jlerror { border: 0; border-left: 3px solid var(--e-danger); border-radius: 0 6px 6px 0;
    background: var(--e-danger-bg); padding: 8px 12px 10px; margin: 2px 0 0; }
  ${look} pluto-cell jlerror > .error-header { display: none; }
  ${look} pluto-cell jlerror > header { border-left: 0; background: transparent; padding: 0; color: var(--e-danger); }
  ${look} pluto-cell jlerror > header > p:first-child { font-weight: normal; }
  ${look} pluto-cell jlerror > header > p:not(:first-child) { color: var(--e-text-secondary); font: 12.5px/18px system-ui, sans-serif; }
  ${look} pluto-cell jlerror[data-endeavor-error="upstream"] { border-left-color: var(--e-control-edge); background: transparent; }
  ${look} pluto-cell jlerror > section { border-block-start: 0; margin-block-start: 6px; padding-block-start: 0; }
  pluto-cell jlerror[data-endeavor-error] > section .stacktrace-header { display: none; }
  pluto-cell jlerror[data-endeavor-error] > section.stacktrace-waiting-to-view,
  pluto-cell jlerror[data-endeavor-trace="closed"] > section { display: none; }
  pluto-cell jlerror[data-endeavor-error="upstream"] > header { display: none; }

  .endeavor-ask { display: flex; align-items: center; gap: 6px; margin: 6px 0 2px; font: 12px/24px system-ui, sans-serif;
    color: var(--e-text-secondary); flex-wrap: wrap; }
  .endeavor-ask button.pill { height: 24px; display: inline-flex; align-items: center; gap: 5px; padding: 0 10px;
    border-radius: 12px; border: 1px solid var(--e-pill-edge); background: var(--e-pill-bg); color: var(--e-text-secondary);
    font: 12px/16px system-ui, sans-serif; white-space: nowrap; cursor: pointer; }
  .endeavor-ask button.pill:hover { background: var(--e-button-hover); }
  .endeavor-ask button.fix { border-color: var(--e-accent); color: var(--e-prompt-hover); }
  .endeavor-ask .hint { margin-left: 6px; font-size: 11.5px; color: var(--e-text-faint); }
  .endeavor-ask .spark { color: var(--e-prompt-hover); }
  .endeavor-ask .dot { color: var(--e-text-faint); }
  .endeavor-ask a, .endeavor-upstream a { color: var(--e-accent-text); cursor: pointer; text-decoration: none; }
  .endeavor-ask a.cancel { color: var(--e-text-secondary); text-decoration: underline; text-underline-offset: 2px; }
  .endeavor-ask svg { width: 12px; height: 12px; color: var(--e-text-muted); }
  .endeavor-trace { display: flex; align-items: center; gap: 4px; margin-top: 4px; font: 12px/17px system-ui, sans-serif;
    color: var(--e-text-muted); cursor: pointer; user-select: none; }
  .endeavor-trace:hover { color: var(--e-text-secondary); }
  .endeavor-upstream .message { font-family: var(--julia-mono-font-stack, JuliaMono, monospace); font-size: 12px; line-height: 18px;
    color: var(--e-text-secondary); white-space: pre-wrap; }
  .endeavor-upstream .why { display: flex; align-items: center; gap: 6px; margin-top: 6px; font: 12px/17px system-ui, sans-serif;
    color: var(--e-text-muted); }
  .endeavor-upstream code { font-size: 11.5px; color: var(--e-text-secondary); background: none; padding: 0; }
`;

const CLOCK = `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" aria-hidden="true"><circle cx="8" cy="8" r="6"/><path d="M8 5v3.2l2 1.3"/></svg>`;

type Frame = { call?: string; file?: string; line?: number };
type Result = { errored?: boolean; output?: { body?: { msg?: string; stacktrace?: Frame[] } | unknown } };
type Notebook = {
  cell_results?: Record<string, Result>;
  cell_dependencies?: Record<string, { upstream_cells_map?: Record<string, string[]>; downstream_cells_map?: Record<string, string[]> }>;
};

let asks: ErrorAsk[] = [];
let working = false;
const pending = new Map<string, ErrorAsk & { at: number }>();
const traceOpen = new Set<string>();

const notebook = (): Notebook | undefined => (window as any).editor_state?.notebook;

function body(nb: Notebook | undefined, id: string): { msg?: string; stacktrace?: Frame[] } | undefined {
  const b = nb?.cell_results?.[id]?.output?.body;
  return b && typeof b === "object" ? (b as { msg?: string; stacktrace?: Frame[] }) : undefined;
}

/** The failed cells this one fails because of, by the name it uses from each (Pluto's get_erred_upstreams). */
export function failedUpstreams(nb: Notebook | undefined, id: string, seen: string[] = []): Record<string, string> {
  let found: Record<string, string> = {};
  if (!nb?.cell_results?.[id]?.errored) return found;
  const uses = nb.cell_dependencies?.[id]?.upstream_cells_map ?? {};
  for (const [name, cells] of Object.entries(uses)) {
    if (seen.includes(name)) continue;
    seen.push(name);
    for (const up of cells) {
      const deeper = failedUpstreams(nb, up, seen);
      found = { ...found, ...deeper };
      if (Object.keys(deeper).length === 0 && nb.cell_results?.[up]?.errored && up !== id) found[name] = up;
    }
  }
  return found;
}

/** The cells a cell fails because of, if it fails only because they did; else null. */
function upstreamCause(nb: Notebook | undefined, id: string): Record<string, string> | null {
  const msg = body(nb, id)?.msg ?? "";
  const sym = msg.match(/^UndefVarError: (.*?) not defined/)?.[1]?.replaceAll("`", "");
  if (!sym) return null;
  const fromAbove = Object.values(nb?.cell_dependencies ?? {}).some((d) => Object.keys(d.downstream_cells_map ?? {}).includes(sym));
  const failed = failedUpstreams(nb, id);
  return fromAbove && Object.keys(failed).length ? failed : null;
}

const plainMessage = (msg: string) => msg.split("\n")[0].replaceAll("`", "").replace(/Main\.var"workspace#\d+"/g, "Main");

/** The error as the agent gets it: the message and the whole trace, folded or not. */
function errorText(error: HTMLElement, id: string): string {
  const b = body(notebook(), id);
  if (!b?.msg) return (error.querySelector("header")?.textContent ?? error.textContent ?? "").trim().slice(0, MAX_ERROR);
  const frames = (b.stacktrace ?? []).map((f, i) => {
    const cell = f.file?.match(/#==#([0-9a-f-]{36})/)?.[1];
    const where = cell ? `cell ${cell}${f.line ? `, line ${f.line}` : ""}` : `${f.file ?? "?"}${f.line ? `:${f.line}` : ""}`;
    return ` [${i + 1}] ${f.call ?? "?"} @ ${where}`;
  });
  return [b.msg.trim(), ...(frames.length ? ["Stacktrace:", ...frames] : [])].join("\n").slice(0, MAX_ERROR);
}

function traceLabel(frames: Frame[], open: boolean): string {
  const shown = frames.filter((f) => !(f.file === "none" && f.call === "top-level scope"));
  const steps = (n: number) => `${n} step${n === 1 ? "" : "s"}`;
  const mine = shown.filter((f) => f.file?.includes("#==#")).length;
  const theirs = shown.length - mine;
  if (open && mine && theirs) return `Stack trace · ${steps(mine)} in your notebook, ${theirs} inside packages`;
  return `Stack trace · ${steps(shown.length)}`;
}

function statusOf(id: string): ErrorAsk | null {
  const told = asks.find((a) => a.cell === id);
  if (told) return told;
  const mine = pending.get(id);
  if (mine && Date.now() - mine.at < PENDING) return mine;
  pending.delete(id);
  return null;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function link(text: string, cls: string, act: () => void): HTMLAnchorElement {
  const a = el("a", cls, text);
  a.href = "#";
  a.onclick = (e) => {
    e.preventDefault();
    if (byUser(e)) act();
  };
  return a;
}

/** The actions row: the two buttons, or what became of a click. */
function fillActions(row: HTMLElement, cell: HTMLElement, error: HTMLElement) {
  const status = statusOf(cell.id);
  const key = status ? `${status.kind}:${status.queued}` : "buttons";
  if (row.dataset.key === key) return;
  row.dataset.key = key;
  row.replaceChildren();
  if (!status) {
    const ask = (kind: "fix" | "explain") => {
      if (statusOf(cell.id)) return;
      const notebookId = new URLSearchParams(location.search).get("id");
      send({ type: "ask", kind, notebook: notebookId, cell: cell.id, code: cellCode(cell), error: errorText(error, cell.id) });
      pending.set(cell.id, { cell: cell.id, kind, queued: working, at: Date.now() });
      fillActions(row, cell, error);
      setTimeout(decorate, PENDING + 50);
    };
    const fix = el("button", "pill fix");
    fix.append(el("span", "spark", "✦"), `Fix with ${context.agent}`);
    fix.onclick = (e) => byUser(e) && ask("fix");
    const explain = el("button", "pill explain", "Explain");
    explain.onclick = (e) => byUser(e) && ask("explain");
    row.append(fix, explain, el("span", "hint", `or ${shortcut("E")} to ask something else`));
  } else if (!status.queued) {
    row.append(
      el("span", "spark", "✦"),
      `${context.agent} is ${status.kind === "fix" ? "fixing" : "explaining"} this`,
      el("span", "dot", "·"),
      link("Show in chat ›", "show", () => send({ type: "error_ask_show", cell: cell.id })),
    );
  } else {
    const clock = el("span", "clock");
    clock.innerHTML = CLOCK;
    row.append(
      clock,
      `${status.kind === "fix" ? "Fix" : "Explain"} queued · sends after ${context.agent}’s current turn`,
      el("span", "dot", "·"),
      link("Cancel", "cancel", () => {
        pending.delete(cell.id);
        send({ type: "error_ask_cancel", cell: cell.id });
      }),
    );
  }
}

function place(after: Element, node: Element) {
  if (after.nextElementSibling !== node) after.after(node);
}

function decorateOwn(error: HTMLElement, cell: HTMLElement, nb: Notebook | undefined) {
  error.querySelector(":scope > .endeavor-upstream")?.remove();
  const header = error.querySelector(":scope > header");
  if (!header) return;
  let row = error.querySelector<HTMLElement>(":scope > .endeavor-ask");
  if (!row) {
    row = el("div", "endeavor-ask");
    row.dataset.endeavorUi = "";
  }
  place(header, row);
  fillActions(row, cell, error);

  const frames = body(nb, cell.id)?.stacktrace;
  const plutoTrace = error.querySelector(":scope > section");
  let toggle = error.querySelector<HTMLElement>(":scope > .endeavor-trace");
  if (!plutoTrace || (frames && frames.length === 0)) {
    toggle?.remove();
    return;
  }
  const open = traceOpen.has(cell.id);
  const want = open ? "open" : "closed";
  if (error.getAttribute("data-endeavor-trace") !== want) error.setAttribute("data-endeavor-trace", want);
  if (!toggle) {
    toggle = el("div", "endeavor-trace");
    toggle.dataset.endeavorUi = "";
    toggle.setAttribute("role", "button");
    toggle.onclick = () => {
      traceOpen.has(cell.id) ? traceOpen.delete(cell.id) : traceOpen.add(cell.id);
      decorate();
    };
  }
  place(row, toggle);
  const label = `${frames ? traceLabel(frames, open) : "Stack trace"} ${open ? "⌄" : "›"}`;
  if (toggle.textContent !== label) toggle.textContent = label;
  if (open) error.querySelector<HTMLButtonElement>(":scope > section.stacktrace-waiting-to-view button")?.click();
}

function decorateUpstream(error: HTMLElement, cell: HTMLElement, causes: Record<string, string>, nb: Notebook | undefined) {
  error.querySelector(":scope > .endeavor-ask")?.remove();
  error.querySelector(":scope > .endeavor-trace")?.remove();
  const header = error.querySelector(":scope > header");
  if (!header) return;
  const names = Object.keys(causes);
  const key = `${names.join()}|${body(nb, cell.id)?.msg ?? ""}`;
  let box = error.querySelector<HTMLElement>(":scope > .endeavor-upstream");
  if (!box) {
    box = el("div", "endeavor-upstream");
    box.dataset.endeavorUi = "";
  }
  place(header, box);
  if (box.dataset.key === key) return;
  box.dataset.key = key;
  const why = el("div", "why");
  why.append("Fails because ");
  names.forEach((name, i) => {
    if (i) why.append(" or ");
    why.append(el("code", "", name));
  });
  why.append(
    " failed",
    el("span", "dot", "·"),
    link("Show ›", "show", () => document.getElementById(causes[names[0]])?.scrollIntoView({ block: "center", behavior: "smooth" })),
  );
  box.replaceChildren(el("div", "message", plainMessage(body(nb, cell.id)?.msg ?? "")), why);
}

function decorate() {
  const nb = notebook();
  const decorated = new Set<HTMLElement>();
  for (const error of document.querySelectorAll<HTMLElement>("pluto-cell jlerror")) {
    const cell = error.closest<HTMLElement>("pluto-cell");
    if (!cell || error.closest("pluto-log-dot")) continue;
    const causes = upstreamCause(nb, cell.id);
    const kind = causes ? "upstream" : "own";
    for (const node of [error, cell]) if (node.getAttribute("data-endeavor-error") !== kind) node.setAttribute("data-endeavor-error", kind);
    decorated.add(cell);
    if (causes) decorateUpstream(error, cell, causes, nb);
    else decorateOwn(error, cell, nb);
  }
  for (const cell of document.querySelectorAll<HTMLElement>("pluto-cell[data-endeavor-error]")) {
    if (!decorated.has(cell)) cell.removeAttribute("data-endeavor-error");
  }
}

export function initErrors(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("context", (msg) => {
    working = !!msg.working;
    asks = msg.error_asks ?? [];
    for (const a of asks) pending.delete(a.cell);
    decorate();
  });
  onRedraw(decorate);
}
