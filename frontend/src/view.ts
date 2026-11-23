import { initAnnotate, pointState } from "./annotate";
import { initAskBox } from "./askbox";
import { findButton, initFind } from "./find";
import { setHost, type ToApp } from "./bridge";
import { cellName } from "./cellname";
import { drawBox, mayDraw } from "./crop";
import { mac, modHeld } from "./keys";
import type { Pick } from "./quote";
import { initNarrow } from "./narrow";
import { EMBER, PLUTO, initSpace } from "./space";
import { statusModel, type NotebookLike } from "./status";
import { initTheme } from "./theme";

export type Host = {
  canSend: boolean;
  engine?: Engine;
  send(text: string): Promise<void>;
  openLink?(url: string): Promise<void>;
  crop?(png: string, crop: string): Promise<void>;
};

export type Engine = "pluto" | "ember";

const format = (engine: Engine) =>
  "A quote's uri, notebook://" + engine + "/{notebook_id}/cell/{cell_id}, names its cell (it is not a fetchable URL); `lines` are " +
  "line numbers in the cell's code, counting from 1. `part` says what was picked when it isn't code: `output` (the cell's " +
  "output), `figure` (the cell's output, a picture) or `box` (a box the user drew over the notebook; `uri` lists the cells " +
  "under it). A `lines` quote holds the lines the user picked, as they were then; the others hold no code or output. Read " +
  "what the cells hold now with the notebook MCP tools (read_cell, view_cell_output). A box's " +
  "`crop` names a picture of what was in it: see it with view_cell_output, with that `crop` and one of the box's cells. Only what could be drawn is in it, and when nothing could be, there's none.";

export const quoteNote = (engine: Engine = "pluto") =>
  "[Endeavor] The user picked parts of the notebook in this message, each as a <quote> followed by their comment on it. " + format(engine);

const LEAD = "\u2193";
const LEAD_NOTE = " A \u2193 just before this note only marks where the quotes start; it isn't part of the user's words.";

const uri = (notebook: string, cell: string, engine: Engine) => `notebook://${engine}/${notebook}/cell/${cell}`;

export const MESSAGE_MAX = 8000;

export function chatSafe(text: string): string {
  return text
    .replace(/\r\n?/g, "\n")
    .replace(/[\u00ad\u180e\u200b\u2060\ufeff]/g, "")
    .replace(/(?:[ \t]*\n){4,}/g, "\n\n\n")
    .replace(/[ \t]{65,}/g, (run) => run.slice(0, 64));
}

export type Quoted = { picks: Pick[]; comment: string; crops?: (string | undefined)[] };

export function chatMessage(notebook: string, quoted: Quoted[], engine: Engine = "pluto"): string | null {
  const shortened = quoted.some((q) =>
    q.picks.some((p) => p.part === "lines" && chatSafe(p.text) !== p.text.replace(/\r\n?/g, "\n")),
  );
  const note = shortened
    ? `${quoteNote(engine)} Runs of more than 3 line breaks or 64 spaces in a quote were shortened to fit Claude's message box, and invisible characters left out.`
    : quoteNote(engine);
  const message = chatSafe(
    `${LEAD}\n\n${note}${LEAD_NOTE}\n\n` +
      quoted.map(({ picks, comment, crops }) => quoteText(notebook, picks, comment, engine, crops)).join("\n\n"),
  );
  return message.length <= MESSAGE_MAX ? message : null;
}

export function quoteBlock(notebook: string, pick: Pick, engine: Engine = "pluto", crop?: string): string {
  if (pick.part === "box") return `<quote part="box" uri="${pick.cells.map((c) => uri(notebook, c, engine)).join(" ")}"${crop ? ` crop="${crop}"` : ""}/>`;
  const at = `cell="${cellName(pick.code).replace(/"/g, "'")}"`;
  const where = uri(notebook, pick.cell, engine);
  switch (pick.part) {
    case "cell":
      return `<quote ${at} uri="${where}"/>`;
    case "lines":
      return `<quote ${at} lines="${pick.lines[0]}-${pick.lines[1]}" uri="${where}">\n${pick.text}\n</quote>`;
    case "output":
      return `<quote ${at} part="output" uri="${where}"/>`;
    case "figure":
      return `<quote ${at} part="figure" uri="${where}"/>`;
  }
}

export function quoteText(notebook: string, picks: Pick[], comment: string, engine: Engine = "pluto", crops: (string | undefined)[] = []): string {
  const blocks = picks.map((p, i) => quoteBlock(notebook, p, engine, crops[i]));
  return comment ? `${blocks.join("\n\n")}\n${comment}` : blocks.join("\n\n");
}

const DRAWER = 40;

const BAR = 32;

const chrome = `
  :root { --endeavor-bar: ${BAR}px; }
  body { padding-top: var(--endeavor-bar); box-sizing: border-box; }
  footer { display: none !important; }
  pluto-editor > header#pluto-nav { display: none !important; }
  pluto-editor:is(.process_waiting_for_permission, .process_dead, .nbpkg_restart_required, .nbpkg_restart_recommended, .disconnected) > header#pluto-nav {
    display: block !important; position: sticky; top: var(--endeavor-bar); z-index: 60; }
  pluto-editor > header#pluto-nav nav#at_the_top > :not(#process_status) { display: none !important; }
  pluto-editor > header#pluto-nav nav#at_the_top { justify-content: center; }
  pluto-editor main > preamble { top: calc(var(--endeavor-bar) + 5px) !important; }
  html { scroll-padding-top: calc(var(--endeavor-bar) + 8px); }
  #endeavor-bar { position: fixed; top: 0; left: 0; right: 0; z-index: 9997; box-sizing: border-box; height: var(--endeavor-bar);
    display: flex; align-items: center; gap: 10px; padding: 0 10px 0 14px;
    background: var(--e-bg-page); border-bottom: 1px solid var(--e-divider); color: var(--e-text-secondary);
    font: 12.5px system-ui, -apple-system, "Segoe UI", sans-serif; }
  #endeavor-bar .file { font-weight: 600; color: var(--e-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  #endeavor-bar .doing { color: var(--e-text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 0 1 auto; }
  #endeavor-bar .grow { flex: 1; }
  #endeavor-bar button { flex: none; display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 8px; border: none; border-radius: 6px;
    background: none; color: var(--e-text-secondary); font: inherit; cursor: pointer; }
  #endeavor-bar button:hover { color: var(--e-text-primary); background: var(--e-button-hover); }
  #endeavor-bar button.on { color: var(--e-text-primary); background: var(--e-bg-raised); }
  #endeavor-bar .key { color: var(--e-text-faint); }
  #helpbox-wrapper { display: block !important; }
  pluto-helpbox.hidden { display: none !important; }
  ${PLUTO} pluto-helpbox, ${EMBER} #helpbox-wrapper.open { position: fixed !important; top: var(--endeavor-bar) !important; bottom: 0 !important;
    left: auto !important; right: 0 !important; height: auto !important; width: min(380px, 100vw) !important;
    border-radius: 0 !important; border: none !important; border-left: 1px solid var(--e-divider) !important;
    box-shadow: -8px 0 24px var(--e-shadow-popover) !important; }
  ${EMBER} pluto-helpbox { position: static !important; width: auto !important; height: 100% !important; border: none !important; box-shadow: none !important; }
  ${EMBER} .ember-scrim { display: none !important; }
  pluto-helpbox > header { display: none !important; }
  @media (max-width: 759px) {
    ${PLUTO} pluto-helpbox, ${EMBER} #helpbox-wrapper.open { top: auto !important; left: 0 !important; width: auto !important; height: ${DRAWER}vh !important;
      border-left: none !important; border-top: 1px solid var(--e-divider) !important; box-shadow: 0 -8px 24px var(--e-shadow-popover) !important; }
    ${EMBER} #helpbox-wrapper.open { animation-name: ember-slide-up !important; }
    body:has(pluto-helpbox:not(.hidden)) pluto-editor:not(.disable_ui) > main { padding-bottom: calc(${DRAWER}vh + 24px) !important; }
  }
  @media (max-width: 420px) {
    #endeavor-bar .key, #endeavor-bar .panel .word, #endeavor-find-button .word { display: none; }
  }
  .outline-frame-actions-container.safe-preview { display: none !important; }
`;

const css = `
  #annotate-hint { top: calc(var(--endeavor-bar) + 8px) !important; }
  #endeavor-sent { position: fixed; left: 50%; top: calc(var(--endeavor-bar) + 8px); transform: translateX(-50%); z-index: 10001; display: none;
    max-width: calc(100vw - 32px); box-sizing: border-box; text-align: center;
    padding: 4px 12px; border-radius: 12px; background: var(--e-hint-bg); color: var(--e-hint-text); font: 12px system-ui; }
  #endeavor-sent.shown { display: block; }
`;

export function doing(nb: NotebookLike | undefined): string {
  if (!nb?.cell_order) return "";
  const model = statusModel(nb);
  if (model.busy) return model.busy;
  if (model.saveFailed) return "Couldn't save";
  return "";
}

export const fileName = (path: string | undefined): string => (path ? path.split(/[\\/]/).pop() || "Notebook" : "Notebook");

const book = `<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><path d="M2.5 3.2c1.8-.6 3.7-.4 5.5.8 1.8-1.2 3.7-1.4 5.5-.8v9.6c-1.8-.6-3.7-.4-5.5.8-1.8-1.2-3.7-1.4-5.5-.8z"/><path d="M8 4v9.6"/></svg>`;

const pulse = `<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M1.5 8.5h3l1.8-4.5 3.4 8 1.8-3.5h3"/></svg>`;

const PANELS: [tab: string, word: string, icon: string, title: string][] = [
  ["docs", "Docs", book, "Live docs: what the code under the cursor does"],
  ["process", "Status", pulse, "Status: what the notebook is doing, its packages and how long each step took"],
];

function initBar(withPoint: boolean, point: (on: boolean) => void): void {
  const style = document.createElement("style");
  style.id = "endeavor-chrome";
  style.textContent = chrome;
  document.head.append(style);
  const bar = document.createElement("div");
  bar.id = "endeavor-bar";
  bar.innerHTML =
    `<span class="file">Notebook</span><span class="doing">Loading…</span><span class="grow"></span>` +
    findButton() +
    PANELS.map(([tab, word, icon, title]) => `<button type="button" class="panel ${tab}" title="${title}" aria-label="${word}">${icon}<span class="word">${word}</span></button>`).join("") +
    (withPoint
      ? `<button type="button" id="endeavor-point" title="Point at a cell, its code or its output, and comment on it for Claude"><span>Point</span><span class="key">${mac ? "⌘⇧E" : "Ctrl+Shift+E"}</span></button>`
      : "");
  document.body.prepend(bar);
  const file = bar.querySelector<HTMLElement>(".file")!;
  const words = bar.querySelector<HTMLElement>(".doing")!;
  const pointButton = bar.querySelector<HTMLButtonElement>("#endeavor-point");
  const panelOpen = (tab: string) => !!document.querySelector(`pluto-helpbox.helpbox-${tab}:not(.hidden)`);
  const panels = PANELS.map(([tab]) => [tab, bar.querySelector<HTMLButtonElement>(`.panel.${tab}`)!] as const);
  for (const [tab, button] of panels)
    button.addEventListener("click", (e) => {
      if (!e.isTrusted) return;
      const open = panelOpen(tab);
      window.dispatchEvent(new CustomEvent("open_bottom_right_panel", { detail: open ? null : tab }));
      if (!open && tab === "docs") setTimeout(() => document.querySelector<HTMLInputElement>("#live-docs-search")?.focus(), 50);
    });
  pointButton?.addEventListener("click", (e) => e.isTrusted && point(!document.body.classList.contains("annotating")));
  initFind(bar.querySelector<HTMLButtonElement>("#endeavor-find-button")!);
  let last: unknown = null;
  const tick = () => {
    const nb = (window as any).editor_state?.notebook as (NotebookLike & { path?: string }) | undefined;
    if (nb && nb !== last) {
      last = nb;
      file.textContent = fileName(nb.path);
      file.title = file.textContent;
      words.textContent = doing(nb);
    }
    for (const [tab, button] of panels) button.classList.toggle("on", panelOpen(tab));
    pointButton?.classList.toggle("on", document.body.classList.contains("annotating"));
  };
  let soon = 0;
  new MutationObserver(() => {
    if (!soon) soon = window.setTimeout(() => ((soon = 0), tick()), 250);
  }).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
  tick();
  keepCursorBelowBar();
}

function keepCursorBelowBar(): void {
  let queued = false;
  document.addEventListener("selectionchange", () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const selection = window.getSelection();
      if (!selection?.rangeCount || !selection.isCollapsed || !(selection.anchorNode as Element | null)?.parentElement?.closest?.(".cm-content")) return;
      const range = selection.getRangeAt(0);
      const rect = range.getClientRects()[0] ?? (selection.anchorNode as Element).parentElement!.getBoundingClientRect();
      const bar = document.getElementById("endeavor-bar")?.getBoundingClientRect().bottom ?? 0;
      if (rect.bottom > 0 && rect.top < bar + 4) window.scrollBy(0, rect.top - bar - 8);
    });
  });
}

let started = false;

export function start(notebook: string, host: Host): void {
  if (started) return;
  started = true;
  const engine: Engine = host.engine === "ember" ? "ember" : "pluto";
  const receive = (msg: object) => window.__endeavor?.receive(msg as never);
  const sent = document.createElement("div");
  sent.id = "endeavor-sent";
  let hide = 0;
  const say = (text: string, ms = 4000) => {
    sent.textContent = text;
    sent.classList.add("shown");
    clearTimeout(hide);
    if (ms) hide = window.setTimeout(() => sent.classList.remove("shown"), ms);
  };
  const point = (on: boolean) => receive({ type: "annotate", on });
  type Drawn = Quoted & { pngs: Promise<string | null>[] };
  const ids = (picks: Pick[]) =>
    picks.map((p) =>
      p.part === "box" && p.rect && host.crop && mayDraw(p.rect)
        ? [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("")
        : undefined,
    );
  const draw = (q: Quoted): Drawn => ({
    ...q,
    pngs: q.picks.map((p, i) => (q.crops?.[i] && p.part === "box" && p.rect ? drawBox(p.rect).catch(() => null) : Promise.resolve(null))),
  });
  const keep = (q: Drawn) =>
    q.pngs.forEach((png, i) => {
      const id = q.crops?.[i];
      if (id) png.then((png) => (png ? host.crop?.(png, id) : undefined)).catch(() => {});
    });
  const added: Drawn[] = [];
  setHost((msg: ToApp) => {
    if (msg.type !== "quote" || !msg.picks.length) return;
    if (msg.add) {
      added.push(draw({ picks: msg.picks, comment: msg.comment, crops: ids(msg.picks) }));
      return say(`${added.length} added. ${mac ? "↩" : "Enter"} passes ${added.length === 1 ? "it" : "them"} to the chat with the next pick.`);
    }
    const enter = mac ? "↩" : "Enter";
    const max = `${MESSAGE_MAX.toLocaleString("en-US")} characters`;
    const now: Quoted = { picks: msg.picks, comment: msg.comment, crops: ids(msg.picks) };
    const giveBack = () => msg.ref !== undefined && receive({ type: "quote_sent", ref: msg.ref, ok: false });
    let message = chatMessage(notebook, [...added, now], engine);
    let held = false;
    if (message === null) {
      if (!added.length || chatMessage(notebook, [now], engine) === null) {
        giveBack();
        return say(`Too long for Claude's message box (${max}). Shorten your comment or pick less, then press ${enter}.`, 8000);
      }
      message = chatMessage(notebook, added, engine);
      giveBack();
      if (message === null) {
        added.length = 0;
        return say(`Too long for Claude's message box (${max}) with what ${mac ? "⌘↩" : "Ctrl+Enter"} added, so those picks were let go. Press ${enter} to send this one.`, 8000);
      }
      held = true;
    }
    const earlier = added.splice(0);
    say("Sending to Claude's chat…", 0);
    const sending = host.send(message);
    const going = held ? earlier : [...earlier, draw(now)];
    sending.then(
      () => {
        going.forEach(keep);
        if (held) return say(`Sent what ${mac ? "⌘↩" : "Ctrl+Enter"} added: together they were too long for Claude's message box. Send it from there, then press ${enter} to send this one.`, 8000);
        if (msg.ref !== undefined) receive({ type: "quote_sent", ref: msg.ref, ok: true });
        say("On its way to Claude's message box. Send it from there.", 8000);
        const end = () => {
          const left = pointState();
          if (!left.picks.length && !left.comment.trim()) point(false);
        };
        if (!document.hasFocus()) end();
        else window.addEventListener("blur", end, { once: true });
      },
      (e) => {
        added.unshift(...earlier);
        if (!held) giveBack();
        say(`Couldn't reach Claude's chat${e instanceof Error && e.message ? `: ${e.message}` : ""}. Your ${msg.picks.length === 1 ? "pick is" : "picks are"} back: ${mac ? "↩" : "Enter"} sends again.`);
      },
    );
  });
  initTheme("web");
  initNarrow();
  initSpace();
  closeStrayCompletions();
  initLinks(host.openLink);
  followNewCells();
  receive({ type: "theme", name: "view" });
  initBar(host.canSend, point);
  followClaudesPalette();
  if (!host.canSend) return;
  initAskBox();
  initAnnotate(true);

  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  document.body.append(sent);
  window.addEventListener(
    "keydown",
    (e) => {
      if (!e.isTrusted || !modHeld(e) || !e.shiftKey || e.key.toLowerCase() !== "e" || (mac && e.ctrlKey)) return;
      e.preventDefault();
      e.stopPropagation();
      point(!document.body.classList.contains("annotating"));
    },
    true,
  );
}

export function outsideLink(href: string | null): string | null {
  if (!href) return null;
  return /^(https?:|mailto:)/i.test(href.trim()) ? href.trim() : null;
}

export function initLinks(open: ((url: string) => Promise<void>) | undefined): void {
  document.addEventListener("click", (e) => {
    if (e.defaultPrevented || e.button !== 0) return;
    const link = (e.target as Element | null)?.closest?.("a[href]");
    if (!link) return;
    const href = (link.getAttribute("href") ?? "").trim();
    if (/^javascript:/i.test(href)) return;
    e.preventDefault();
    if (/^#./.test(href)) {
      document.getElementById(decodeURIComponent(href.slice(1)))?.scrollIntoView({ behavior: "smooth", block: "start" });
      return;
    }
    const url = outsideLink(href);
    if (url && open && e.isTrusted) open(url).catch(() => {});
  });
}

export const claudePalette = `
html.claude-palette {
  --e-bg-page: var(--color-background-primary);
  --e-bg-sidebar: var(--color-background-secondary);
  --e-bg-card: var(--color-background-secondary);
  --e-bg-sunken: var(--color-background-secondary);
  --e-bg-raised: var(--color-background-tertiary);
  --e-bg-tag: var(--color-background-tertiary);
  --e-button-hover: var(--color-background-tertiary);
  --e-menu-hover: var(--color-background-tertiary);
  --e-popover-bg: var(--color-background-primary);
  --e-dialog-bg: var(--color-background-primary);
  --e-text-primary: var(--color-text-primary);
  --e-text-secondary: var(--color-text-secondary);
  --e-text-muted: var(--color-text-secondary);
  --e-text-section: var(--color-text-secondary);
  --e-text-tag: var(--color-text-secondary);
  --e-text-faint: var(--color-text-tertiary);
  --e-text-dim: var(--color-text-tertiary);
  --e-hint-text: var(--color-text-secondary);
  --e-border: var(--color-border-tertiary);
  --e-divider: var(--color-border-tertiary);
  --e-popover-edge: var(--color-border-secondary);
  --e-dialog-edge: var(--color-border-secondary);
  --e-control-edge: var(--color-border-primary);
  --main-bg-color: var(--color-background-primary);
  --header-bg-color: var(--color-background-primary);
  --footer-bg-color: var(--color-background-primary);
  --pluto-output-bg-color: var(--color-background-primary);
  --pluto-output-color: var(--color-text-secondary);
  --pluto-output-h-color: var(--color-text-primary);
  --rule-color: var(--color-border-tertiary);
  --code-background: var(--color-background-secondary);
  --pluto-logs-bg-color: var(--color-background-secondary);
  --overlay-button-bg: var(--color-background-secondary);
  --input-context-menu-bg-color: var(--color-background-secondary);
  --input-context-menu-border-color: var(--color-border-tertiary);
  --helpbox-bg-color: var(--color-background-secondary);
  --helpbox-header-bg-color: var(--color-background-tertiary);
  --helpbox-header-tab-bg-color: var(--color-background-tertiary);
  --helpbox-header-color: var(--color-text-primary);
  --helpbox-text-color: var(--color-text-primary);
  --helpbox-search-bg-color: var(--color-background-primary);
  --helpbox-search-border-color: var(--color-border-primary);
  --docs-binding-bg: var(--color-background-tertiary);
  --export-bg-color: var(--color-background-secondary);
  --export-color: var(--color-text-primary);
  --frontmatter-button-bg-color: var(--color-background-tertiary);
  --frontmatter-input-bg-color: var(--color-background-primary);
  --frontmatter-input-border-color: var(--color-border-primary);
  --pluto-runarea-span-color: var(--color-text-tertiary);
}
html.claude-palette .pluto-modal { border-color: var(--color-border-secondary); }
`;

const PALETTE_NEEDS = ["--color-background-primary", "--color-background-secondary", "--color-background-tertiary", "--color-text-primary",
  "--color-text-secondary", "--color-text-tertiary", "--color-border-primary", "--color-border-secondary", "--color-border-tertiary"];

function followClaudesPalette(): void {
  const style = document.createElement("style");
  style.id = "endeavor-claude-palette";
  style.textContent = claudePalette;
  document.head.append(style);
  const root = document.documentElement;
  const sync = () => root.classList.toggle("claude-palette", PALETTE_NEEDS.every((v) => root.style.getPropertyValue(v).trim() !== ""));
  new MutationObserver(sync).observe(root, { attributes: true, attributeFilter: ["style"] });
  sync();
}

const STRAY_MS = 5000;

function closeStrayCompletions(): void {
  let lastInput = 0;
  let timer = 0;
  for (const type of ["keydown", "pointerdown", "pointermove", "wheel"]) window.addEventListener(type, (e) => e.isTrusted && (lastInput = Date.now()), { capture: true, passive: true });
  const list = () => document.querySelector(".cm-tooltip-autocomplete");
  const close = () => {
    const open = list();
    if (!open || document.body.classList.contains("annotating")) return;
    open.closest(".cm-editor")?.querySelector(".cm-content")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", keyCode: 27, bubbles: true, cancelable: true }));
  };
  const check = (openedAt: number) => {
    timer = 0;
    if (!list()) return;
    const idle = Date.now() - Math.max(openedAt, lastInput);
    if (!document.hasFocus() || idle >= STRAY_MS) close();
    if (list()) timer = window.setTimeout(() => check(openedAt), Math.max(STRAY_MS - idle, 500));
  };
  new MutationObserver(() => {
    if (!timer && list()) check(Date.now());
  }).observe(document.body, { childList: true, subtree: true });
  window.addEventListener("blur", close);
}

const IDLE_MS = 2000;
const OWN_SCROLL_MS = 1000;

function followNewCells(): void {
  let lastInput = 0;
  for (const type of ["keydown", "pointerdown"]) window.addEventListener(type, (e) => e.isTrusted && (lastInput = Date.now()), true);
  let following = true;
  let ownScroll = 0;
  window.addEventListener(
    "scroll",
    () => {
      if (Date.now() - ownScroll < OWN_SCROLL_MS) return;
      following = window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 80;
    },
    { passive: true },
  );
  const known = new Set<string>();
  let settled = false;
  let settle = 0;
  const cells = () => [...document.querySelectorAll<HTMLElement>("pluto-cell")];
  const show = (cell: HTMLElement) => {
    const r = cell.getBoundingClientRect();
    if (r.top >= 0 && r.bottom <= window.innerHeight) return;
    ownScroll = Date.now();
    cell.scrollIntoView({ block: r.height > window.innerHeight ? "start" : "center", behavior: "smooth" });
  };
  new MutationObserver(() => {
    const added = cells().filter((c) => c.id && !known.has(c.id));
    if (!added.length) return;
    added.forEach((c) => known.add(c.id));
    if (!settled) {
      clearTimeout(settle);
      settle = window.setTimeout(() => (settled = true), 1500);
      return;
    }
    if (added.length > 3 || !following || Date.now() - lastInput < IDLE_MS) return;
    const cell = added[added.length - 1];
    const since = Date.now();
    show(cell);
    const grow = new ResizeObserver(() => {
      if (!following || lastInput > since || Date.now() - since > 15000 || !cell.isConnected) return grow.disconnect();
      show(cell);
    });
    grow.observe(cell);
  }).observe(document.body, { childList: true, subtree: true });
}

declare global {
  interface Window {
    EndeavorView?: { start: typeof start };
  }
}

window.EndeavorView = { start };
