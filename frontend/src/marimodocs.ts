import { on, send } from "./bridge";
import { capDrawer, drawerTab, openDrawer, refitDrawer, resizesDrawer } from "./drawer";

const DOCS_OPTION = "[data-testid=chrome-sidebar] [role=option][data-key=documentation]";
const PANEL = "[data-testid=helper]";
const ICON_COLUMN = "[data-testid=chrome-sidebar]";
const TOP_BUTTONS = "[data-testid=notebook-menu-dropdown]";
const BOTTOM_BUTTONS = "[data-testid=command-palette-button]";
const CLEAR_OF_BUTTONS = 8;
const RUN_STRIP_ROOM = 96;
const BOTTOM_BUTTONS_INSET = 20;
const SETTLE_MS = 5000;
const MOUNT_MS = 15000;
const POLL_MS = 50;

const css = `
  html[data-endeavor-drawer="docs"] ${PANEL} {
    position: fixed !important; left: var(--endeavor-docs-left, 0px); right: 0; bottom: var(--endeavor-docs-bottom, 0px); top: auto; width: auto !important;
    height: var(--endeavor-drawer-h, 42vh); flex: none !important; z-index: 40;
    border-top: 1px solid var(--slate-6, #ddd); background: var(--background, #fff); }
  html[data-endeavor-drawer="docs"] [data-panel-resize-handle-id] { display: none; }
  html[data-endeavor-drawer="docs"] #App, html[data-endeavor-drawer="docs"] [data-testid=chrome-body] {
    padding-bottom: var(--endeavor-drawer-pad, 42vh); }
  #endeavor-docs-grip { display: none; }
  html[data-endeavor-drawer="docs"] #endeavor-docs-grip { display: block; position: fixed; z-index: 41; height: 8px; cursor: ns-resize;
    left: var(--endeavor-docs-left, 0px); right: 0; bottom: calc(var(--endeavor-docs-bottom, 0px) + var(--endeavor-drawer-h, 42vh) - 4px); }
  #endeavor-docs-grip::after { content: ""; position: absolute; left: calc(50% - 14px); top: 1px; width: 28px; height: 3px;
    border-radius: 2px; background: var(--e-control-edge); }
  @media print { html[data-endeavor-drawer="docs"] :is(${PANEL}, #endeavor-docs-grip) { display: none !important; } }
`;

export function showsDocs(panelSize: string | null, title: string): boolean {
  return Number(panelSize) > 0 && title.trim().toLowerCase() === "documentation";
}

function docsShown(): boolean {
  const panel = document.querySelector<HTMLElement>(PANEL);
  return !!panel && showsDocs(panel.getAttribute("data-panel-size"), panel.querySelector(".uppercase")?.textContent ?? "");
}

export async function setDocs(want: boolean, shown: () => boolean, press: () => boolean, wait: (ms: number) => Promise<void>): Promise<boolean> {
  for (let waited = 0; shown() !== want && !press(); waited += POLL_MS) {
    if (waited >= MOUNT_MS) return false;
    await wait(POLL_MS);
  }
  for (let waited = 0; waited < SETTLE_MS && shown() !== want; waited += POLL_MS) await wait(POLL_MS);
  return shown() === want;
}

export function pressLikeAMouse(target: HTMLElement): void {
  const box = target.getBoundingClientRect();
  const at = { bubbles: true, cancelable: true, composed: true, view: window, button: 0, detail: 1, clientX: box.x + box.width / 2, clientY: box.y + box.height / 2, pointerId: 1, pointerType: "mouse", isPrimary: true };
  target.dispatchEvent(new PointerEvent("pointerdown", { ...at, buttons: 1 }));
  target.dispatchEvent(new MouseEvent("mousedown", { ...at, buttons: 1 }));
  target.dispatchEvent(new PointerEvent("pointerup", at));
  target.dispatchEvent(new MouseEvent("mouseup", at));
  target.dispatchEvent(new MouseEvent("click", at));
}

let reported: boolean | null = null;
let moving = 0;

export function besideIconColumn(): { left: number; bottom: number } {
  const column = document.querySelector(ICON_COLUMN)?.getBoundingClientRect();
  if (!column) return { left: 0, bottom: 0 };
  return { left: Math.max(0, Math.round(column.right)), bottom: Math.max(0, Math.round(window.innerHeight - column.bottom)) };
}

export function tallestBelowButtons(innerHeight: number, footer: number, topButtonsBottom: number, bottomButtonsHeight: number): number {
  return innerHeight - footer - (topButtonsBottom + CLEAR_OF_BUTTONS + RUN_STRIP_ROOM + CLEAR_OF_BUTTONS + bottomButtonsHeight + BOTTOM_BUTTONS_INSET);
}

function tallestDrawer(): number {
  const top = document.querySelector(TOP_BUTTONS)?.closest(".absolute")?.getBoundingClientRect();
  const bottom = document.querySelector(BOTTOM_BUTTONS)?.closest(".absolute")?.getBoundingClientRect();
  if (!top || !bottom) return Infinity;
  return tallestBelowButtons(window.innerHeight, besideIconColumn().bottom, top.bottom, bottom.height);
}

export function sidebarDragging(): boolean {
  return !!document.querySelector(`${ICON_COLUMN} [role=option][inert]`);
}

function endSidebarDrag(): void {
  const target = document.activeElement ?? document.body;
  for (const type of ["keydown", "keyup"]) target.dispatchEvent(new KeyboardEvent(type, { key: "Escape", code: "Escape", keyCode: 27, bubbles: true, cancelable: true }));
}

function follow() {
  if (sidebarDragging()) endSidebarDrag();
  if (moving) return;
  const { left, bottom } = besideIconColumn();
  const root = document.documentElement.style;
  if (root.getPropertyValue("--endeavor-docs-left") !== `${left}px`) root.setProperty("--endeavor-docs-left", `${left}px`);
  if (root.getPropertyValue("--endeavor-docs-bottom") !== `${bottom}px`) root.setProperty("--endeavor-docs-bottom", `${bottom}px`);
  if (drawerTab()) refitDrawer();
  const open = docsShown();
  if (open && drawerTab() !== "docs") openDrawer("docs");
  else if (!open && drawerTab() === "docs") openDrawer(null);
  if (open === reported) return;
  reported = open;
  send({ type: "docs", open });
}

function addGrip(): void {
  const grip = document.createElement("div");
  grip.id = "endeavor-docs-grip";
  grip.dataset.endeavorUi = "";
  document.body.append(grip);
  resizesDrawer(grip, () => document.querySelector<HTMLElement>(PANEL));
}

export function initMarimoDocs(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  addGrip();
  capDrawer(tallestDrawer);
  const press = () => {
    const option = document.querySelector<HTMLElement>(DOCS_OPTION);
    if (option) pressLikeAMouse(option);
    return !!option;
  };
  const wait = (ms: number) => new Promise<void>((done) => setTimeout(done, ms));
  on("drawer", (msg) => {
    moving++;
    void setDocs(msg.tab === "docs", docsShown, press, wait).finally(() => {
      moving--;
      follow();
    });
  });
  new MutationObserver(follow).observe(document.body, { attributes: true, attributeFilter: ["data-panel-size", "inert"], childList: true, subtree: true, characterData: true });
  follow();
}
