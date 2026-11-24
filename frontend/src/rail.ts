import { cellName } from "./cellname";
import { dom, onMarimo } from "./engine";
import { onRedraw } from "./redraw";
import { context } from "./state";

export type RunState = "ran" | "pending" | "running" | "failed" | "claude";

const WIDTH = 6;
const OVERLAY_GAP = 12;
const CLASSIC_GAP = 2;
const MIN_MARK = 2;

const css = `
  #endeavor-rail { position: fixed; width: ${WIDTH + 8}px; z-index: 50; cursor: pointer; touch-action: none; }
  #endeavor-rail a { position: absolute; left: 4px; width: ${WIDTH}px; border-radius: 2px; min-height: ${MIN_MARK}px;
    background: var(--e-you-stripe); cursor: pointer; }
  #endeavor-rail a[data-state="pending"] { opacity: 0.28; }
  #endeavor-rail a[data-state="running"] { background: var(--e-accent); animation: endeavor-rail-pulse 1s ease-in-out infinite alternate; }
  #endeavor-rail a[data-state="claude"] { background: var(--e-accent); }
  #endeavor-rail a[data-state="failed"] { background: var(--e-danger); }
  @keyframes endeavor-rail-pulse { from { opacity: 1; } to { opacity: 0.35; } }
  @media (prefers-reduced-motion: reduce) { #endeavor-rail a[data-state="running"] { animation: none; } }
  #endeavor-rail .view { position: absolute; left: 0; right: 0; border-radius: 4px; pointer-events: none;
    background: color-mix(in srgb, var(--e-text-primary) 7%, transparent); border: 1px solid color-mix(in srgb, var(--e-text-primary) 16%, transparent); }
  #endeavor-rail-tip { position: fixed; z-index: 51; transform: translateY(-50%); pointer-events: none; white-space: nowrap;
    padding: 3px 8px; border-radius: 6px; background: var(--e-popover-bg); color: var(--e-text-primary);
    border: 1px solid var(--e-popover-edge); box-shadow: 0 4px 12px var(--e-shadow-popover); font: 11.5px system-ui, -apple-system, sans-serif; }
  #endeavor-rail-tip[hidden] { display: none; }
  @media print { #endeavor-rail, #endeavor-rail-tip { display: none !important; } }
`;

const inSafePreview = () => document.querySelector("pluto-editor.process_waiting_for_permission") !== null;

export function runState(cell: Element): RunState {
  const has = (name: string) => cell.classList.contains(name);
  if (inSafePreview()) return "pending";
  if (onMarimo()) {
    const status = cell.querySelector("[data-testid=cell-status][data-status], [data-status]")?.getAttribute("data-status");
    if (status === "running") return "running";
    if (cell.hasAttribute("data-endeavor-bar")) return "claude";
    if (status === "queued" || has("needs-run") || has("stale") || has("stopped")) return "pending";
    if (has("has-error")) return "failed";
    return "ran";
  }
  if (has("running")) return "running";
  if (cell.hasAttribute("data-endeavor-bar")) return "claude";
  const rail = cell.getAttribute("data-rail");
  if (has("queued") || has("code_differs") || has("not_run_yet") || has("stale") || rail === "due" || cell.getAttribute("data-endeavor") === "unrun") return "pending";
  if (has("errored")) return "failed";
  return "ran";
}

const stateWords = (state: RunState): string =>
  ({ ran: "ran", pending: "not run yet", running: "running", failed: "failed", claude: `${context.agent} edited it, not run` })[state] ?? "";

let rail: HTMLElement;
let view: HTMLElement;
let tip: HTMLElement;
let cells: HTMLElement[] = [];

function scroller(): HTMLElement {
  const root = document.scrollingElement as HTMLElement | null;
  if (!onMarimo()) return root ?? document.documentElement;
  let el = document.querySelector(dom().cell)?.parentElement ?? null;
  while (el && el !== document.body) {
    const overflow = getComputedStyle(el).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && el.scrollHeight > el.clientHeight) return el;
    el = el.parentElement;
  }
  return root ?? document.documentElement;
}

function frame(s: HTMLElement): { left: number; top: number; height: number; contentRight: number; classic: boolean } {
  if (s === document.scrollingElement || s === document.documentElement) {
    const width = document.documentElement.clientWidth;
    return { left: 0, top: 0, height: window.innerHeight, contentRight: width, classic: window.innerWidth - width > 0 };
  }
  const r = s.getBoundingClientRect();
  return { left: r.left, top: r.top, height: s.clientHeight, contentRight: r.left + s.clientWidth, classic: s.offsetWidth - s.clientWidth > 0 };
}

function contentTop(cell: HTMLElement, s: HTMLElement): number {
  const outer = s === document.scrollingElement || s === document.documentElement ? 0 : s.getBoundingClientRect().top;
  return cell.getBoundingClientRect().top - outer + s.scrollTop;
}

const PAGE_CONTROLS_ABOVE = "[data-testid=chrome-controls-top-right], main > preamble #saveall-container";
const PAGE_CONTROLS_BELOW = "[data-testid=chrome-controls-bottom-right]";
const CLEAR_OF_CONTROLS = 8;

function besideRail(selector: string, left: number): DOMRect[] {
  return [...document.querySelectorAll(selector)]
    .map((el) => el.getBoundingClientRect())
    .filter((r) => r.height > 0 && r.right > left - CLEAR_OF_CONTROLS);
}

function shownDrawer(): DOMRect | null {
  const drawer = document.querySelector<HTMLElement>("html[data-endeavor-drawer] #endeavor-drawer, html[data-endeavor-drawer=\"docs\"] [data-testid=helper]");
  return drawer && drawer.offsetHeight ? drawer.getBoundingClientRect() : null;
}

function place(): { s: HTMLElement; height: number; total: number } {
  const s = scroller();
  const f = frame(s);
  const drawer = shownDrawer();
  const beside = !!drawer && drawer.left < f.contentRight && drawer.top <= f.top;
  const right = beside ? drawer!.left : f.contentRight;
  const gap = beside || f.classic ? CLASSIC_GAP : OVERLAY_GAP;
  const left = right - gap - (WIDTH + 8);
  const above = besideRail(PAGE_CONTROLS_ABOVE, left);
  const below = besideRail(PAGE_CONTROLS_BELOW, left);
  const floor = drawer && !beside ? Math.min(drawer.top, f.top + f.height) : f.top + f.height;
  const top = Math.max(f.top + 10, ...above.map((r) => r.bottom + CLEAR_OF_CONTROLS));
  const bottom = Math.min(floor - 10, ...below.map((r) => r.top - CLEAR_OF_CONTROLS));
  const height = Math.max(bottom - top, 0);
  rail.style.left = `${left}px`;
  rail.style.top = `${top}px`;
  rail.style.height = `${height}px`;
  return { s, height, total: Math.max(s.scrollHeight, 1) };
}

function draw() {
  cells = [...document.querySelectorAll<HTMLElement>(dom().cell)];
  rail.hidden = cells.length === 0;
  if (!cells.length) {
    rail.querySelectorAll("a").forEach((a) => a.remove());
    return;
  }
  const { s, height, total } = place();
  const marks = cells.map((cell) => {
    const mark = document.createElement("a");
    mark.dataset.cell = dom().id(cell);
    mark.dataset.state = runState(cell);
    mark.style.top = `${(contentTop(cell, s) / total) * height}px`;
    mark.style.height = `${Math.max(MIN_MARK, (cell.offsetHeight / total) * height - 1)}px`;
    return mark;
  });
  rail.replaceChildren(...marks, view);
  follow();
}

function follow() {
  const s = scroller();
  const f = frame(s);
  const height = rail.clientHeight || parseFloat(rail.style.height) || 0;
  const total = Math.max(s.scrollHeight, 1);
  view.style.top = `${(s.scrollTop / total) * height}px`;
  view.style.height = `${Math.min(height, (f.height / total) * height)}px`;
}

function markAt(y: number): HTMLElement | null {
  for (const mark of rail.querySelectorAll<HTMLElement>("a")) {
    const top = parseFloat(mark.style.top);
    if (y >= top - 2 && y <= top + parseFloat(mark.style.height) + 2) return mark;
  }
  return null;
}

function cellOf(mark: HTMLElement): HTMLElement | undefined {
  return cells.find((c) => dom().id(c) === mark.dataset.cell);
}

function codeOf(cell: HTMLElement): string {
  const input = cell.querySelector(dom().input);
  const lines = input?.querySelectorAll(".cm-line");
  return lines?.length ? [...lines].map((l) => l.textContent ?? "").join("\n") : (input?.textContent ?? "");
}

function showTip(mark: HTMLElement | null) {
  const cell = mark && cellOf(mark);
  if (!mark || !cell) {
    tip.hidden = true;
    return;
  }
  const r = mark.getBoundingClientRect();
  tip.textContent = `${cellName(codeOf(cell))} · ${stateWords(mark.dataset.state as RunState)}`;
  tip.hidden = false;
  tip.style.top = `${r.top + r.height / 2}px`;
  tip.style.left = "auto";
  tip.style.right = `${window.innerWidth - rail.getBoundingClientRect().left + 4}px`;
}

function scrollToFraction(y: number, smooth: boolean) {
  const s = scroller();
  const height = rail.clientHeight || 1;
  s.scrollTo({ top: (y / height) * s.scrollHeight - frame(s).height / 2, behavior: smooth ? "smooth" : "auto" });
}

function wire() {
  let dragging = false;
  let downY = 0;
  const y = (e: PointerEvent) => e.clientY - rail.getBoundingClientRect().top;
  rail.addEventListener("pointerdown", (e) => {
    dragging = true;
    downY = y(e);
    rail.setPointerCapture?.(e.pointerId);
  });
  rail.addEventListener("pointermove", (e) => {
    if (dragging && Math.abs(y(e) - downY) > 4) scrollToFraction(y(e), false);
    showTip(markAt(y(e)));
  });
  for (const end of ["pointerup", "pointercancel", "lostpointercapture"]) {
    rail.addEventListener(end, () => {
      dragging = false;
    });
  }
  rail.addEventListener("pointerleave", () => {
    if (!dragging) showTip(null);
  });
  rail.addEventListener("click", (e) => {
    e.preventDefault();
    if (Math.abs(y(e as PointerEvent) - downY) > 4) return;
    const target = (e.target as Element).closest<HTMLElement>("#endeavor-rail a");
    const cell = target && cellOf(target);
    if (cell) cell.scrollIntoView({ block: "center", behavior: "smooth" });
    else scrollToFraction(y(e as PointerEvent), true);
  });
}

export function initRail(): void {
  const style = document.createElement("style");
  style.textContent = css;
  rail = document.createElement("div");
  rail.id = "endeavor-rail";
  rail.dataset.endeavorUi = "";
  rail.setAttribute("aria-hidden", "true");
  view = document.createElement("div");
  view.className = "view";
  tip = document.createElement("div");
  tip.id = "endeavor-rail-tip";
  tip.dataset.endeavorUi = "";
  tip.hidden = true;
  document.head.append(style);
  document.body.append(rail, tip);
  wire();
  onRedraw(draw);
  window.addEventListener("resize", draw);
  document.addEventListener("scroll", follow, { capture: true, passive: true });
}

export const redrawRail = () => rail && draw();
