import { on, send } from "./bridge";

const DOCS_OPTION = "[data-testid=chrome-sidebar] [role=option][data-key=documentation]";
const PANEL = "[data-testid=helper]";
const TRIES = 4;
const SETTLE_MS = 1200;
const POLL_MS = 50;

const css = `
  html[data-endeavor-drawer="docs"] ${PANEL} {
    position: fixed !important; left: 0; right: 0; bottom: 0; top: auto; width: auto !important;
    height: var(--endeavor-drawer-h, 42vh); flex: none !important; z-index: 40;
    border-top: 1px solid var(--slate-6, #ddd); background: var(--background, #fff); }
  html[data-endeavor-drawer="docs"] [data-panel-resize-handle-id] { display: none; }
  html[data-endeavor-drawer="docs"] #App, html[data-endeavor-drawer="docs"] [data-testid=chrome-body] {
    padding-bottom: var(--endeavor-drawer-h, 42vh); }
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

function follow() {
  const open = docsShown();
  if (open) document.documentElement.dataset.endeavorDrawer = "docs";
  else if (document.documentElement.dataset.endeavorDrawer === "docs") delete document.documentElement.dataset.endeavorDrawer;
  if (open === reported) return;
  reported = open;
  send({ type: "docs", open });
}

export function initMarimoDocs(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  document.documentElement.style.setProperty("--endeavor-drawer-h", `${Math.round(window.innerHeight * 0.42)}px`);
  const press = () => document.querySelector<HTMLElement>(DOCS_OPTION)?.click();
  const wait = (ms: number) => new Promise<void>((done) => setTimeout(done, ms));
  on("drawer", (msg) => void setDocs(msg.tab === "docs", docsShown, press, wait).then(follow));
  new MutationObserver(follow).observe(document.body, { attributes: true, attributeFilter: ["data-panel-size"], childList: true, subtree: true, characterData: true });
  follow();
}
