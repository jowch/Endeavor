// The drawer at the bottom of the notebook (Endeavor look): Live docs and
// Status share it, and it resizes from its top edge. The header's book and
// pulse buttons open it on a tab (`drawer` messages); it reports its tab back
// in the `state` summary. Live docs is Pluto's own panel, moved into the
// drawer by CSS (it follows the cursor as in Pluto). Status is ours, from
// status.ts: the current step in bold, the steps under it, then packages,
// dependencies and cells, with the raw Pkg log behind Show log. On a first run
// that installs packages Status opens by itself, and closes when the run ends
// unless the user opened it.

import { byUser, on, send } from "./bridge";
import { current, every, notebookId, onNotebook, report, setDrawerSource, type Drawer } from "./state";
import type { NotebookLike, StatusModel } from "./status";

const HEADER = 36;

const css = `
  #endeavor-drawer { display: none; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer] #endeavor-drawer { display: flex; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer] body { padding-bottom: var(--endeavor-drawer-h); }
  #endeavor-drawer { position: fixed; left: 0; right: 0; bottom: 0; height: var(--endeavor-drawer-h); z-index: 70;
    flex-direction: column; background: #151517; border-top: 1px solid #2A2A2E;
    font: 12.5px/1.45 system-ui, -apple-system, sans-serif; color: #BDBDBD; }
  #endeavor-drawer .grip { position: absolute; top: -4px; left: 0; right: 0; height: 8px; cursor: ns-resize; }
  #endeavor-drawer .grip::after { content: ""; position: absolute; left: calc(50% - 14px); top: 1px; width: 28px; height: 3px;
    border-radius: 2px; background: #3A3A40; }
  #endeavor-drawer > header { flex: none; height: ${HEADER}px; display: flex; align-items: center; gap: 4px; padding: 0 12px; }
  #endeavor-drawer > header button { display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 8px; border: none;
    border-radius: 5px; background: none; color: #8C8C8C; font: inherit; cursor: pointer; }
  #endeavor-drawer > header button:hover { color: #ECECEC; }
  #endeavor-drawer > header button.active { background: #26262A; color: #ECECEC; }
  #endeavor-drawer > header .grow { flex: 1; }
  #endeavor-drawer > header .close { font-size: 15px; padding: 0 6px; }
  #endeavor-drawer .status { flex: 1; overflow-y: auto; padding: 2px 16px 12px; }
  #endeavor-drawer[data-tab="docs"] .status { display: none; }
  #endeavor-drawer .headline { font-size: 14px; font-weight: 600; color: #ECECEC; margin: 4px 0 8px; }
  #endeavor-drawer .headline.failed { color: #E07A7A; }
  #endeavor-drawer .steps { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  #endeavor-drawer .steps .step { display: flex; align-items: center; gap: 6px; white-space: nowrap; color: #8C8C8C; }
  #endeavor-drawer .steps .step.busy { color: #ECECEC; }
  #endeavor-drawer .steps .step.failed { color: #E07A7A; }
  #endeavor-drawer .steps .step.waiting { color: #5E5E5E; }
  #endeavor-drawer .steps .line { flex: 1; height: 1px; background: #2A2A2E; min-width: 12px; }
  #endeavor-drawer .host { color: #7A7A7A; font-size: 11.5px; margin-bottom: 10px; }
  #endeavor-drawer .group { margin-top: 6px; }
  #endeavor-drawer .group > summary { display: flex; align-items: center; gap: 6px; list-style: none; cursor: pointer;
    color: #BDBDBD; padding: 3px 0; }
  #endeavor-drawer .group > summary::-webkit-details-marker { display: none; }
  #endeavor-drawer .group > summary::before { content: "›"; width: 10px; color: #7A7A7A; transition: transform 0.1s; }
  #endeavor-drawer .group[open] > summary::before { transform: rotate(90deg); }
  #endeavor-drawer .group > summary .right { margin-left: auto; color: #7A7A7A; font-size: 11.5px; }
  #endeavor-drawer .row { display: flex; align-items: center; gap: 8px; padding: 2px 0 2px 16px; min-height: 22px; }
  #endeavor-drawer .row .name { font: 12px JuliaMono, ui-monospace, monospace; color: #D4D4D4; }
  #endeavor-drawer .row.waiting .name { color: #7A7A7A; }
  #endeavor-drawer .row .note { color: #5E5E5E; font-size: 11px; }
  #endeavor-drawer .row .right { margin-left: auto; color: #8C8C8C; font-size: 11.5px; }
  #endeavor-drawer .row .right.time { color: #5E5E5E; font-size: 10.5px; }
  #endeavor-drawer .row.failed .right { color: #E07A7A; }
  #endeavor-drawer .mark { width: 12px; flex: none; display: flex; justify-content: center; font-size: 11px; color: #8C8C8C; }
  #endeavor-drawer .mark.failed { color: #E07A7A; }
  #endeavor-drawer .mark.waiting::before { content: ""; width: 4px; height: 4px; border-radius: 2px; background: #4A4A4E; }
  #endeavor-drawer .mark.busy::before { content: ""; width: 7px; height: 7px; border-radius: 50%; border: 1.5px solid #CC3F00;
    border-right-color: transparent; animation: endeavor-spin 0.9s linear infinite; }
  @media (prefers-reduced-motion: reduce) { #endeavor-drawer .mark.busy::before { animation: none; } }
  @keyframes endeavor-spin { to { transform: rotate(360deg); } }
  #endeavor-drawer .failure { margin: 8px 0 8px 16px; padding: 10px 12px; border: 1px solid rgba(224, 122, 122, 0.4);
    border-radius: 8px; background: rgba(224, 122, 122, 0.06); color: #D4D4D4; }
  #endeavor-drawer .failure code { font: 11.5px JuliaMono, ui-monospace, monospace; }
  #endeavor-drawer .failure .actions { display: flex; gap: 6px; margin-top: 8px; }
  #endeavor-drawer .failure button, #endeavor-drawer .showlog { display: flex; align-items: center; gap: 5px; padding: 3px 9px;
    border-radius: 5px; border: 1px solid #3A3A40; background: #1C1C1F; color: #D4D4D4; font: inherit; cursor: pointer; }
  #endeavor-drawer .failure button.fix { color: #E08A5E; border-color: #CC3F00; }
  #endeavor-drawer .failure button.plain, #endeavor-drawer .showlog { border-color: transparent; background: none; color: #8C8C8C; }
  #endeavor-drawer .foot { display: flex; justify-content: flex-end; margin-top: 10px; }
  #endeavor-drawer pre.log { margin: 6px 0 0; padding: 8px 10px; max-height: 260px; overflow: auto; border-radius: 6px;
    background: #1C1C1F; color: #BDBDBD; font: 11px/1.45 JuliaMono, ui-monospace, monospace; white-space: pre-wrap; }
  /* Live docs: Pluto's panel, filling the drawer under its tabs. */
  html[data-endeavor-look="endeavor"][data-endeavor-drawer="docs"] #helpbox-wrapper {
    display: block !important; position: fixed; left: 0; right: 0; bottom: 0; top: auto;
    height: calc(var(--endeavor-drawer-h) - ${HEADER}px); z-index: 71; }
  html[data-endeavor-look="endeavor"] pluto-helpbox { position: static; width: 100%; height: 100%; right: auto;
    border-radius: 0; box-shadow: none; background: #151517; }
  html[data-endeavor-look="endeavor"] pluto-helpbox > header { display: none; }
  html[data-endeavor-look="endeavor"] pluto-helpbox .live-docs-searchbox { margin: 2px 16px 8px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox .live-docs-searchbox input { border: 1px solid #3A3A40; border-radius: 6px;
    font-size: 12.5px; padding: 5px 8px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox > section { padding: 0 16px 12px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox > section h1 { font-size: 13px; margin: 4px 0 8px; }
  @media print {
    #endeavor-drawer, #endeavor-safe, #endeavor-rail, #helpbox-wrapper { display: none !important; }
    html[data-endeavor-drawer] body { padding-bottom: 0; }
  }
`;

const book = `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.1"><path d="M1 2.2h3.3c.9 0 1.7.7 1.7 1.6v6.4c0-.7-.6-1.2-1.3-1.2H1zM11 2.2H7.7c-.9 0-1.7.7-1.7 1.6v6.4c0-.7.6-1.2 1.3-1.2H11z"/></svg>`;
const pulse = `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.1"><path d="M.5 6.5h2.5l1.5-4 2.5 7 1.5-3h3"/></svg>`;

let drawer: HTMLElement;
let body: HTMLElement;
let tab: Drawer = null;
/** The drawer opened by itself for a first run; closes again when the run ends. */
let auto = false;
/** Already opened by itself for this page (once per notebook load). */
let autoDone = false;
let showLog = false;
/** Groups the user folded or unfolded, by name; otherwise open while busy or failed. */
const folded = new Map<string, boolean>();

const escape = (s: string) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

function height(): number {
  try {
    const saved = Number(localStorage.getItem("endeavor-drawer-h"));
    if (saved > 80) return saved;
  } catch {}
  return Math.round(window.innerHeight * 0.42);
}

function setHeight(h: number) {
  const clamped = Math.max(120, Math.min(h, window.innerHeight - 80));
  document.documentElement.style.setProperty("--endeavor-drawer-h", `${clamped}px`);
  return clamped;
}

export function openDrawer(next: Drawer, byItself = false): void {
  tab = next;
  auto = byItself;
  if (next) document.documentElement.dataset.endeavorDrawer = next;
  else delete document.documentElement.dataset.endeavorDrawer;
  drawer.dataset.tab = next ?? "";
  for (const b of drawer.querySelectorAll<HTMLElement>("header [data-tab]")) b.classList.toggle("active", b.dataset.tab === next);
  // Pluto's panel shows its Live docs tab only while ours is on it.
  window.dispatchEvent(new CustomEvent("open_bottom_right_panel", { detail: next === "docs" ? "docs" : null }));
  if (next === "docs") setTimeout(() => document.querySelector<HTMLInputElement>("#live-docs-search")?.focus(), 50);
  render();
  report();
}

const markFor = (state: string) =>
  state === "done" || state === "ready" ? `<span class="mark">✓</span>` : state === "failed" ? `<span class="mark failed">✕</span>` : state === "waiting" ? `<span class="mark waiting"></span>` : `<span class="mark busy"></span>`;

function group(name: string, busy: boolean, summary: string, right: string, rows: string): string {
  const open = folded.get(name) ?? busy;
  return `<details class="group" data-group="${name}"${open ? " open" : ""}><summary>${summary}<span class="right">${right}</span></summary>${rows}</details>`;
}

function statusHtml(nb: NotebookLike, m: StatusModel): string {
  const steps = m.steps
    .map((s, i) => `${i ? `<span class="line"></span>` : ""}<span class="step ${s.phase}">${markFor(s.phase)}${s.name}</span>`)
    .join("");
  const version = String((nb as any).julia_version ?? "").replace(/^v/, "");
  const julia = version ? `Julia ${escape(version)} · ` : "";
  const host = `<div class="host">${julia}${escape(hostName)}</div>`;

  const pkgBusy = m.steps[1].phase === "busy";
  const readyCount = m.packages.filter((p) => p.state === "ready").length;
  const stateText = { waiting: "waiting", installing: "installing", precompiling: "precompiling", ready: "", failed: "failed" };
  const pkgRows = m.packages
    .map((p) => {
      const note = p.detail === "standard library" ? `<span class="note">standard library</span>` : "";
      const right = p.state === "ready" ? (p.detail === "standard library" ? "built in" : escape(p.detail)) : stateText[p.state];
      return `<div class="row ${p.state}">${markFor(p.state)}<span class="name">${escape(p.name)}</span>${note}<span class="right">${right}</span></div>`;
    })
    .join("");
  let deps = "";
  if (m.deps) {
    const d = m.deps;
    const right = d.failed ? `${d.failed} failed` : pkgBusy ? `${d.precompiled} precompiled` : "ready";
    deps = `<div class="row">${markFor(d.failed ? "failed" : pkgBusy ? "busy" : "done")}<span>and ${d.count} dependenc${d.count === 1 ? "y" : "ies"}</span><span class="right">${right}</span></div>`;
  }
  let failure = "";
  if (m.failure) {
    const f = m.failure;
    const blocked = f.cells.length ? ` Cells that use it can't run: <code>${f.cells.map(escape).join("</code>, <code>")}</code>.` : "";
    failure =
      `<div class="failure"><code>${escape(f.name)}</code> ${
        m.packages.find((p) => p.name === f.name)?.detail === "not found"
          ? "isn't a package Pkg can find: a typo, or not in the registry."
          : "couldn't be installed or precompiled."
      }${blocked}` +
      `<div class="actions"><button class="fix">✦ Fix with Claude</button><button class="restart">↻ Restart notebook</button>` +
      `<button class="plain pkglog">Log for ${escape(f.name)}</button></div></div>`;
  }
  const packages = m.packages.length
    ? group("packages", pkgBusy || !!m.failure, "Packages", m.packages.length ? `${readyCount} of ${m.packages.length} ready` : "", pkgRows + failure + deps)
    : failure;

  const running = m.steps[2].phase === "busy";
  const done = m.cells.filter((c) => c.state === "done").length;
  const cellRows = m.cells
    .map((c) => {
      const right = c.state === "running" ? "running" : c.time ? c.time : c.state === "waiting" ? "waiting" : "";
      return `<div class="row ${c.state}">${markFor(c.state)}<span class="name">${escape(c.name || "(empty)")}</span><span class="right${c.time ? " time" : ""}">${right}</span></div>`;
    })
    .join("");
  const cells = group("cells", running, "Cells", `${done} of ${m.cells.length} · notebook order`, cellRows);

  const log = (nb.nbpkg?.terminal_outputs?.nbpkg_sync ?? "").replace(/\x1b\[[0-9;]*m/g, "");
  const logBlock = showLog ? `<pre class="log">${escape(log || "Nothing from Pkg yet.")}</pre>` : "";
  const foot = `<div class="foot"><button class="showlog">≡ ${showLog ? "Hide log" : "Show log"}</button></div>${logBlock}`;

  return `<div class="headline${m.failure ? " failed" : ""}">${escape(m.headline)}</div><div class="steps">${steps}</div>${host}${packages}${cells}${foot}`;
}

let hostName = "This Mac";
let lastHtml = "";

function render() {
  const now = current();
  if (!now || tab !== "status") return;
  const html = statusHtml(now.nb, now.model);
  if (html === lastHtml) return;
  lastHtml = html;
  body.innerHTML = html;
  for (const d of body.querySelectorAll<HTMLDetailsElement>("details.group")) {
    d.addEventListener("toggle", () => folded.set(d.dataset.group!, d.open));
  }
  const failure = now.model.failure;
  body.querySelector<HTMLButtonElement>(".showlog")!.onclick = () => {
    showLog = !showLog;
    render();
  };
  if (failure) {
    const pkgLog = () => now.nb.nbpkg?.terminal_outputs?.[failure.name] ?? now.nb.nbpkg?.terminal_outputs?.nbpkg_sync ?? "";
    body.querySelector<HTMLButtonElement>(".failure .fix")!.onclick = (e) =>
      byUser(e) && send({ type: "fix_package", notebook: notebookId(), name: failure.name, log: pkgLog().replace(/\x1b\[[0-9;]*m/g, "").slice(-6000) });
    body.querySelector<HTMLButtonElement>(".failure .restart")!.onclick = (e) => byUser(e) && send({ type: "restart", notebook: notebookId() });
    body.querySelector<HTMLButtonElement>(".failure .pkglog")!.onclick = () => {
      showLog = true;
      render();
      body.querySelector("pre.log")?.scrollIntoView({ block: "nearest" });
    };
  }
}

/** Status opens by itself while a first run installs packages, and shuts after, unless the user opened it. */
function follow(_: NotebookLike, m: StatusModel) {
  const installing = m.steps[1].phase === "busy" && m.packages.some((p) => p.state === "installing" || p.state === "precompiling");
  if (installing && !autoDone && tab === null && document.documentElement.dataset.endeavorLook === "endeavor") {
    autoDone = true;
    openDrawer("status", true);
  } else if (auto && tab === "status" && !m.busy && !m.failure && m.steps[2].phase !== "waiting") {
    openDrawer(null);
  }
  render();
}

export function initDrawer(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  setHeight(height());

  drawer = document.createElement("div");
  drawer.id = "endeavor-drawer";
  drawer.dataset.endeavorUi = "";
  drawer.innerHTML =
    `<div class="grip"></div><header><button data-tab="docs">${book}Live docs</button><button data-tab="status">${pulse}Status</button>` +
    `<span class="grow"></span><button class="close" title="Close">×</button></header><section class="status"></section>`;
  body = drawer.querySelector(".status")!;
  document.body.append(drawer);

  for (const b of drawer.querySelectorAll<HTMLElement>("header [data-tab]")) b.onclick = () => openDrawer(b.dataset.tab as Drawer);
  drawer.querySelector<HTMLElement>(".close")!.onclick = () => openDrawer(null);

  const grip = drawer.querySelector<HTMLElement>(".grip")!;
  grip.onpointerdown = (e) => {
    grip.setPointerCapture(e.pointerId);
    const move = (m: PointerEvent) => setHeight(window.innerHeight - m.clientY);
    const up = () => {
      grip.removeEventListener("pointermove", move);
      try {
        localStorage.setItem("endeavor-drawer-h", String(drawer.offsetHeight));
      } catch {}
    };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up, { once: true });
  };

  setDrawerSource(() => tab);
  on("drawer", (msg) => openDrawer(msg.tab));
  on("context", (msg) => {
    hostName = msg.host;
    lastHtml = "";
    render();
  });
  onNotebook(follow);
  // Elapsed times and spinners while something runs.
  every(1000, () => current()?.model.busy && render());
}
