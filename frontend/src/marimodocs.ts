import { on, send } from "./bridge";

const DOCS_OPTION = "[data-testid=chrome-sidebar] [role=option][data-key=documentation]";
const PANEL = "[data-testid=helper]";
const ICON_COLUMN = "[data-testid=chrome-sidebar]";
const TRIES = 4;
const SETTLE_MS = 1200;
const POLL_MS = 50;

const css = `
  html[data-endeavor-drawer="docs"] ${PANEL} {
    position: fixed !important; left: var(--endeavor-docs-left, 0px); right: 0; bottom: var(--endeavor-docs-bottom, 0px); top: auto; width: auto !important;
    height: 42vh; flex: none !important; z-index: 40;
    border-top: 1px solid var(--slate-6, #ddd); background: var(--background, #fff); }
  html[data-endeavor-drawer="docs"] [data-panel-resize-handle-id] { display: none; }
  html[data-endeavor-drawer="docs"] #App, html[data-endeavor-drawer="docs"] [data-testid=chrome-body] {
    padding-bottom: 42vh; }
  @media print { html[data-endeavor-drawer="docs"] ${PANEL} { display: none !important; } }
`;

export function showsDocs(panelSize: string | null, title: string): boolean {
  return Number(panelSize) > 0 && title.trim().toLowerCase() === "documentation";
}

function docsShown(): boolean {
  const panel = document.querySelector<HTMLElement>(PANEL);
  return !!panel && showsDocs(panel.getAttribute("data-panel-size"), panel.querySelector(".uppercase")?.textContent ?? "");
}

export async function setDocs(want: boolean, shown: () => boolean, press: () => void, wait: (ms: number) => Promise<void>, tries = TRIES): Promise<boolean> {
  for (let i = 0; i < tries && shown() !== want; i++) {
    press();
    for (let waited = 0; waited < SETTLE_MS && shown() !== want; waited += POLL_MS) await wait(POLL_MS);
  }
  return shown() === want;
}

let reported: boolean | null = null;

export function besideIconColumn(): { left: number; bottom: number } {
  const column = document.querySelector(ICON_COLUMN)?.getBoundingClientRect();
  if (!column) return { left: 0, bottom: 0 };
  return { left: Math.max(0, Math.round(column.right)), bottom: Math.max(0, Math.round(window.innerHeight - column.bottom)) };
}

function follow() {
  const open = docsShown();
  if (open) {
    document.documentElement.dataset.endeavorDrawer = "docs";
    const { left, bottom } = besideIconColumn();
    document.documentElement.style.setProperty("--endeavor-docs-left", `${left}px`);
    document.documentElement.style.setProperty("--endeavor-docs-bottom", `${bottom}px`);
  }
  else if (document.documentElement.dataset.endeavorDrawer === "docs") delete document.documentElement.dataset.endeavorDrawer;
  if (open === reported) return;
  reported = open;
  send({ type: "docs", open });
}

export function initMarimoDocs(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  const press = () => document.querySelector<HTMLElement>(DOCS_OPTION)?.click();
  const wait = (ms: number) => new Promise<void>((done) => setTimeout(done, ms));
  on("drawer", (msg) => void setDocs(msg.tab === "docs", docsShown, press, wait).then(follow));
  new MutationObserver(follow).observe(document.body, { attributes: true, attributeFilter: ["data-panel-size"], childList: true, subtree: true, characterData: true });
  follow();
}
