import { byUser, on, send } from "./bridge";
import { context, current, every, notebookId, onNotebook, report, setDrawerSource, type Drawer } from "./state";
import { onMarimo } from "./engine";
import { redrawRail } from "./rail";
import type { NotebookLike, StatusModel } from "./status";

const HEADER = 36;
const SIDE_WIDTH = 500;
const SIDE_FROM = 1300;

const css = `
  #endeavor-drawer { display: none; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer] #endeavor-drawer { display: flex; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer] body { padding-bottom: var(--endeavor-drawer-pad); }
  html[data-endeavor-engine="marimo"][data-endeavor-drawer="packages"] #endeavor-drawer { display: flex;
    left: var(--endeavor-docs-left, 0px); bottom: var(--endeavor-docs-bottom, 0px); }
  html[data-endeavor-engine="marimo"][data-endeavor-drawer="packages"] #App,
  html[data-endeavor-engine="marimo"][data-endeavor-drawer="packages"] [data-testid=chrome-body] { padding-bottom: var(--endeavor-drawer-pad); }
  #endeavor-drawer { position: fixed; left: 0; right: 0; bottom: 0; height: var(--endeavor-drawer-h); z-index: 70;
    flex-direction: column; background: var(--e-bg-page); border-top: 1px solid var(--e-border);
    font: 12.5px/1.45 system-ui, -apple-system, sans-serif; color: var(--e-text-secondary); }
  #endeavor-drawer .grip { position: absolute; top: -4px; left: 0; right: 0; height: 8px; cursor: ns-resize; }
  #endeavor-drawer .grip::after { content: ""; position: absolute; left: calc(50% - 14px); top: 1px; width: 28px; height: 3px;
    border-radius: 2px; background: var(--e-control-edge); }
  #endeavor-drawer > header { flex: none; height: ${HEADER}px; display: flex; align-items: center; gap: 4px; padding: 0 12px; }
  #endeavor-drawer > header button { display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 8px; border: none;
    border-radius: 5px; background: none; color: var(--e-text-muted); font: inherit; cursor: pointer; }
  #endeavor-drawer > header button:hover { color: var(--e-text-primary); }
  #endeavor-drawer > header button.active { background: var(--e-bg-raised); color: var(--e-text-primary); }
  #endeavor-drawer > header .grow { flex: 1; }
  #endeavor-drawer > header .close { font-size: 15px; padding: 0 6px; }
  #endeavor-drawer .packages { flex: 1; overflow-y: auto; padding: 2px 16px 12px; }
  #endeavor-drawer[data-tab="docs"] .packages { display: none; }
  #endeavor-drawer .headline { font-size: 14px; font-weight: 600; color: var(--e-text-primary); margin: 4px 0 6px; }
  #endeavor-drawer .headline.failed { color: var(--e-diff-del); }
  #endeavor-drawer .progress { height: 3px; border-radius: 2px; background: var(--e-bg-raised); margin: 0 0 8px; overflow: hidden; }
  #endeavor-drawer .progress > i { display: block; height: 100%; background: var(--e-accent); }
  #endeavor-drawer .progress.unknown > i { width: 30%; animation: endeavor-sweep 1.4s ease-in-out infinite; }
  @keyframes endeavor-sweep { from { transform: translateX(-100%); } to { transform: translateX(340%); } }
  @media (prefers-reduced-motion: reduce) { #endeavor-drawer .progress.unknown > i { animation: none; width: 100%; opacity: 0.4; } }
  #endeavor-drawer .host { color: var(--e-text-dim); font-size: 11.5px; margin-bottom: 4px; }
  #endeavor-drawer .summary { color: var(--e-text-dim); font-size: 11.5px; margin-bottom: 8px; }
  #endeavor-drawer .row { display: flex; align-items: center; gap: 8px; padding: 2px 0 2px 4px; min-height: 22px; }
  #endeavor-drawer .row .name { font: 12px JuliaMono, ui-monospace, monospace; color: var(--e-text-code); }
  #endeavor-drawer .row.waiting .name { color: var(--e-text-dim); }
  #endeavor-drawer .row .note { color: var(--e-text-waiting); font-size: 11px; }
  #endeavor-drawer .row .right { margin-left: auto; color: var(--e-text-muted); font-size: 11.5px; }
  #endeavor-drawer .row .right.time { color: var(--e-text-waiting); font-size: 10.5px; }
  #endeavor-drawer .row.failed .right { color: var(--e-diff-del); }
  #endeavor-drawer .mark { width: 12px; flex: none; display: flex; justify-content: center; font-size: 11px; color: var(--e-text-muted); }
  #endeavor-drawer .mark.failed { color: var(--e-diff-del); }
  #endeavor-drawer .mark.waiting::before { content: ""; width: 4px; height: 4px; border-radius: 2px; background: var(--e-dot-waiting); }
  #endeavor-drawer .mark.busy::before { content: ""; width: 7px; height: 7px; border-radius: 50%; border: 1.5px solid var(--e-accent);
    border-right-color: transparent; animation: endeavor-spin 0.9s linear infinite; }
  @media (prefers-reduced-motion: reduce) { #endeavor-drawer .mark.busy::before { animation: none; } }
  @keyframes endeavor-spin { to { transform: rotate(360deg); } }
  #endeavor-drawer .failure { margin: 8px 0 8px 4px; padding: 10px 12px; border: 1px solid var(--e-danger-edge);
    border-radius: 8px; background: var(--e-danger-bg); color: var(--e-text-code); }
  #endeavor-drawer .failure code { font: 11.5px JuliaMono, ui-monospace, monospace; }
  #endeavor-drawer .failure .actions { display: flex; gap: 6px; margin-top: 8px; }
  #endeavor-drawer .failure button, #endeavor-drawer .showlog { display: flex; align-items: center; gap: 5px; padding: 3px 9px;
    border-radius: 5px; border: 1px solid var(--e-control-edge); background: var(--e-bg-card); color: var(--e-text-code); font: inherit; cursor: pointer; }
  #endeavor-drawer .failure button.fix { color: var(--e-accent-text); border-color: var(--e-accent); }
  #endeavor-drawer .failure button.plain, #endeavor-drawer .showlog { border-color: transparent; background: none; color: var(--e-text-muted); }
  #endeavor-drawer .foot { display: flex; justify-content: flex-end; margin-top: 10px; }
  #endeavor-drawer pre.log { margin: 6px 0 0; padding: 8px 10px; max-height: 260px; overflow: auto; border-radius: 6px;
    background: var(--e-bg-card); color: var(--e-text-secondary); font: 11px/1.45 JuliaMono, ui-monospace, monospace; white-space: pre-wrap; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer="docs"] #helpbox-wrapper {
    display: block !important; position: fixed; left: 0; right: 0; bottom: 0; top: auto; width: auto;
    height: calc(var(--endeavor-drawer-h) - ${HEADER}px); z-index: 71; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer][data-endeavor-drawer-side] body { padding-bottom: 0; padding-right: ${SIDE_WIDTH}px; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer-side] #endeavor-drawer { left: auto; top: 0; width: ${SIDE_WIDTH}px; height: auto;
    border-top: none; border-left: 1px solid var(--e-border); }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer-side] #endeavor-drawer .grip { display: none; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer][data-endeavor-drawer-side] pluto-editor.fullscreen main > preamble #saveall-container { transform: none !important; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer="docs"][data-endeavor-drawer-side] #helpbox-wrapper {
    left: auto; top: ${HEADER}px; width: ${SIDE_WIDTH}px; height: auto; }
  html[data-endeavor-look="endeavor"] pluto-helpbox { position: static; width: 100%; height: 100%; right: auto;
    border-radius: 0; box-shadow: none; background: var(--e-bg-page); }
  html[data-endeavor-look="endeavor"] pluto-helpbox > header { display: none; }
  html[data-endeavor-look="endeavor"] pluto-helpbox .live-docs-searchbox { margin: 2px 16px 8px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox .live-docs-searchbox input { border: 1px solid var(--e-control-edge); border-radius: 6px;
    font-size: 12.5px; padding: 5px 8px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox > section { padding: 0 16px 12px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox > section h1 { font-size: 13px; margin: 4px 0 8px; }
  @media print {
    #endeavor-drawer, #endeavor-safe, #endeavor-rail, #helpbox-wrapper { display: none !important; }
    html[data-endeavor-drawer] body { padding-bottom: 0; padding-right: 0; }
  }
`;

const book = `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.1"><path d="M1 2.2h3.3c.9 0 1.7.7 1.7 1.6v6.4c0-.7-.6-1.2-1.3-1.2H1zM11 2.2H7.7c-.9 0-1.7.7-1.7 1.6v6.4c0-.7.6-1.2 1.3-1.2H11z"/></svg>`;
const box = `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linejoin="round"><path d="M6 1 10.5 3.3v5.4L6 11 1.5 8.7V3.3zM1.5 3.3 6 5.6l4.5-2.3M6 5.6V11"/></svg>`;

let drawer: HTMLElement;
let body: HTMLElement;
let tab: Drawer = null;
let auto = false;
let autoDone = false;
let failureSeen: string | null = null;
let showLog = false;
let modelOf = (): StatusModel | null => current()?.model ?? null;

const escape = (s: string) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

function height(): number {
  try {
    const saved = Number(localStorage.getItem("endeavor-drawer-h"));
    if (saved > 80) return saved;
  } catch {}
  return Math.round(window.innerHeight * 0.42);
}

let cap = (): number => Infinity;
let dragging = false;

export function capDrawer(tallest: () => number): void {
  cap = tallest;
}

export function refitDrawer(): void {
  if (dragging) return;
  const fitted = Math.max(120, Math.min(height(), window.innerHeight - 80, cap()));
  if (document.documentElement.style.getPropertyValue("--endeavor-drawer-h") !== `${fitted}px`) setHeight(fitted);
}

function setHeight(h: number, settle = true) {
  const clamped = Math.max(120, Math.min(h, window.innerHeight - 80, cap()));
  const root = document.documentElement.style;
  root.setProperty("--endeavor-drawer-h", `${clamped}px`);
  if (settle) root.setProperty("--endeavor-drawer-pad", `${clamped}px`);
  return clamped;
}

export function resizesDrawer(grip: HTMLElement, resizing: () => HTMLElement | null): void {
  grip.addEventListener("pointerdown", (e) => {
    const resized = resizing();
    if (!resized) return;
    grip.setPointerCapture(e.pointerId);
    dragging = true;
    const below = window.innerHeight - resized.getBoundingClientRect().bottom;
    let pending: number | null = null;
    let frame = 0;
    const move = (m: PointerEvent) => {
      pending = window.innerHeight - below - m.clientY;
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        if (pending !== null) setHeight(pending, false);
      });
    };
    const ends = ["pointerup", "pointercancel", "lostpointercapture"];
    const end = () => {
      dragging = false;
      grip.removeEventListener("pointermove", move);
      for (const type of ends) grip.removeEventListener(type, end);
      if (frame) cancelAnimationFrame(frame);
      const h = setHeight(pending ?? resized.offsetHeight);
      try {
        localStorage.setItem("endeavor-drawer-h", String(h));
      } catch {}
    };
    grip.addEventListener("pointermove", move);
    for (const type of ends) grip.addEventListener(type, end);
  });
}

export const drawerAtSide = (width: number) => width >= SIDE_FROM && !onMarimo();

function placeSide() {
  if (drawerAtSide(window.innerWidth)) document.documentElement.dataset.endeavorDrawerSide = "";
  else delete document.documentElement.dataset.endeavorDrawerSide;
  redrawRail();
}

let echoing = false;

export const drawerTab = (): Drawer => tab;

export function openDrawer(next: Drawer, byItself = false): void {
  tab = next;
  auto = byItself;
  if (next) document.documentElement.dataset.endeavorDrawer = next;
  else delete document.documentElement.dataset.endeavorDrawer;
  drawer.dataset.tab = next ?? "";
  for (const b of drawer.querySelectorAll<HTMLElement>("header [data-tab]")) b.classList.toggle("active", b.dataset.tab === next);
  echoing = true;
  window.dispatchEvent(new CustomEvent("open_bottom_right_panel", { detail: next === "docs" ? "docs" : null }));
  echoing = false;
  render();
  report();
  redrawRail();
}

function pick(next: Drawer): void {
  openDrawer(next);
  if (next === "docs") setTimeout(() => document.querySelector<HTMLInputElement>("#live-docs-search")?.focus(), 50);
}

const markFor = (state: string) =>
  state === "done" || state === "ready" ? `<span class="mark">✓</span>` : state === "failed" ? `<span class="mark failed">✕</span>` : state === "waiting" ? `<span class="mark waiting"></span>` : `<span class="mark busy"></span>`;

function packagesHtml(m: StatusModel): string {
  const failed = !!m.failure;
  const headline = `<div class="headline${failed ? " failed" : ""}">${escape(m.headline)}</div>`;
  const progress = !m.install ? ""
    : m.install.total ? `<div class="progress"><i style="width:${Math.round((100 * m.install.done) / m.install.total)}%"></i></div>`
    : `<div class="progress unknown"><i></i></div>`;
  const host = `<div class="host">${m.runtime ? `${escape(m.runtime)} · ` : ""}${escape(hostName)}</div>`;
  const allReady = !m.install && !failed && m.packages.length > 0 && m.packages.every((p) => p.state === "ready") && !m.deps?.failed;
  const counted = `${m.packages.length} package${m.packages.length === 1 ? "" : "s"}${m.deps ? ` and ${m.deps.count} dependenc${m.deps.count === 1 ? "y" : "ies"}` : ""}`;
  const summary = allReady ? `<div class="summary">${counted}, all ready</div>` : m.packages.length ? "" : `<div class="summary">No packages</div>`;

  const stateText = { waiting: "waiting", installing: "installing", precompiling: "precompiling", ready: "", failed: "failed" };
  const rows = m.packages
    .map((p) => {
      const right = p.state === "ready" ? (p.detail === "standard library" ? "built in" : escape(p.detail)) : p.state === "failed" && p.detail ? escape(p.detail) : stateText[p.state];
      return `<div class="row ${p.state}">${markFor(p.state)}<span class="name">${escape(p.name)}</span><span class="right">${right}</span></div>`;
    })
    .join("");
  let deps = "";
  if (m.deps && !allReady) {
    const d = m.deps;
    const right = d.failed ? `${d.failed} failed` : m.install ? `${d.done} done` : "ready";
    deps = `<div class="row">${markFor(d.failed ? "failed" : m.install ? "busy" : "done")}<span>and ${d.count} dependenc${d.count === 1 ? "y" : "ies"}</span><span class="right">${right}</span></div>`;
  }
  let failure = "";
  if (m.failure) {
    const f = m.failure;
    const blocked = f.cells.length ? ` Cells that use it can't run: <code>${f.cells.map(escape).join("</code>, <code>")}</code>.` : "";
    failure =
      `<div class="failure"><code>${escape(f.name)}</code> ${escape(f.why)}${blocked}` +
      `<div class="actions"><button class="fix">✦ Fix with ${escape(context.agent)}</button><button class="restart">↻ Restart notebook</button>` +
      `<button class="plain pkglog">Log for ${escape(f.name)}</button></div></div>`;
  }
  const logBlock = showLog ? `<pre class="log">${escape(m.log || "Nothing from the installer yet.")}</pre>` : "";
  const foot = `<div class="foot"><button class="showlog">≡ ${showLog ? "Hide log" : "Show log"}</button></div>${logBlock}`;
  return headline + progress + host + summary + rows + failure + deps + foot;
}

let hostName = "This Mac";
let lastHtml = "";

function render() {
  const model = modelOf();
  if (!model || tab !== "packages") return;
  const html = packagesHtml(model);
  if (html === lastHtml) return;
  lastHtml = html;
  body.innerHTML = html;
  const failure = model.failure;
  body.querySelector<HTMLButtonElement>(".showlog")!.onclick = () => {
    showLog = !showLog;
    render();
  };
  if (failure) {
    body.querySelector<HTMLButtonElement>(".failure .fix")!.onclick = (e) =>
      byUser(e) && send({ type: "fix_package", notebook: notebookId(), name: failure.name, log: failure.log.slice(-6000) });
    body.querySelector<HTMLButtonElement>(".failure .restart")!.onclick = (e) => byUser(e) && send({ type: "restart", notebook: notebookId() });
    body.querySelector<HTMLButtonElement>(".failure .pkglog")!.onclick = () => {
      showLog = true;
      render();
      body.querySelector("pre.log")?.scrollIntoView({ block: "nearest" });
    };
  }
}

function follow(_: NotebookLike | null, m: StatusModel) {
  const root = document.documentElement.dataset;
  const ours = root.endeavorLook === "endeavor" || root.endeavorEngine === "marimo";
  if (!m.install) autoDone = false;
  const newFailure = root.endeavorEngine === "marimo" && m.failure !== null && m.failure.name !== failureSeen;
  failureSeen = m.failure?.name ?? null;
  if (m.install && !autoDone && tab === null && ours) {
    autoDone = true;
    openDrawer("packages", true);
  } else if (newFailure && tab === null) {
    openDrawer("packages", true);
  } else if (auto && tab === "packages" && !m.install && !m.failure && !m.busy?.startsWith("Starting")) {
    openDrawer(null);
  }
  render();
}

export function showModel(source: () => StatusModel | null): (model: StatusModel) => void {
  modelOf = source;
  return (model) => follow(null, model);
}

export function initDrawer(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  setHeight(height());
  placeSide();
  window.addEventListener("resize", placeSide);

  drawer = document.createElement("div");
  drawer.id = "endeavor-drawer";
  drawer.dataset.endeavorUi = "";
  drawer.innerHTML =
    `<div class="grip"></div><header><button data-tab="docs">${book}Live docs</button><button data-tab="packages">${box}Packages</button>` +
    `<span class="grow"></span><button class="close" title="Close">×</button></header><section class="packages"></section>`;
  body = drawer.querySelector(".packages")!;
  if (document.documentElement.dataset.endeavorEngine === "marimo") drawer.querySelector('header [data-tab="docs"]')!.remove();
  document.body.append(drawer);

  for (const b of drawer.querySelectorAll<HTMLElement>("header [data-tab]")) b.onclick = () => pick(b.dataset.tab as Drawer);
  drawer.querySelector<HTMLElement>(".close")!.onclick = () => openDrawer(null);

  resizesDrawer(drawer.querySelector<HTMLElement>(".grip")!, () => drawer);

  window.addEventListener("open_bottom_right_panel", (e) => {
    if (!echoing && (e as CustomEvent).detail === "docs" && tab !== "docs") openDrawer("docs");
  });

  setDrawerSource(() => tab);
  on("drawer", (msg) => pick(msg.tab));
  on("context", (msg) => {
    hostName = msg.host;
    lastHtml = "";
    render();
  });
  onNotebook(follow);
  every(1000, () => current()?.model.busy && render());
}
