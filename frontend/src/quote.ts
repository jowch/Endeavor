// Quotes from the notebook page: what Point picks and what Reply quotes from a
// selection go to the app as one "quote" message (src/annotate.rs turns it
// into attach::Quote). A figure or a box goes with a picture the app takes of
// the viewport, one at a time: the page brings it into view, hides its own
// overlay, and waits for the app's "shot".

import { on, send } from "./bridge";

/** A box in page coordinates (scrolls with the notebook). */
export type Box = { left: number; top: number; right: number; bottom: number };

/** One quote, as the app parses it (`pick_quote` in src/annotate.rs). */
export type Pick =
  | { part: "cell"; cell: string; code: string }
  | { part: "lines"; cell: string; code: string; lines: [number, number]; text: string }
  | { part: "output"; cell: string; code: string; text: string }
  | { part: "figure"; cell: string; code: string; shot?: number }
  | { part: "box"; cells: string[]; shot?: number };

/** While the app takes a picture, the notebook shows as it is. */
export const SHOOTING = "endeavor-shooting";

const waiting = new Map<number, () => void>();
let nextShot = 1;

const nextFrame = () => new Promise<void>((done) => requestAnimationFrame(() => done()));

/** Take a picture of `b` (page coordinates); resolves with its id once the app has it. */
export async function shoot(b: Box): Promise<number> {
  const top = b.top - window.scrollY;
  const bottom = b.bottom - window.scrollY;
  if (top < 0 || bottom > window.innerHeight) window.scrollBy(0, top < 0 || bottom - top > window.innerHeight ? top - 8 : bottom - window.innerHeight + 8);
  document.body.classList.add(SHOOTING);
  await nextFrame();
  await nextFrame();
  const x = Math.max(0, b.left - window.scrollX);
  const y = Math.max(0, b.top - window.scrollY);
  const rect = {
    x,
    y,
    width: Math.min(window.innerWidth, b.right - window.scrollX) - x,
    height: Math.min(window.innerHeight, b.bottom - window.scrollY) - y,
  };
  const id = nextShot++;
  // The app may never answer (no picture taken): go on without it.
  await new Promise<void>((done) => {
    waiting.set(id, done);
    setTimeout(done, 3000);
    send({ type: "shoot", id, rect });
  });
  waiting.delete(id);
  document.body.classList.remove(SHOOTING);
  return id;
}

/** What a cell defines (`rates = …` → `rates`), else "cell", as the app names it (`defined_name`). */
export function cellName(code: string): string {
  const line = code.split("\n").find((l) => l.trim()) ?? "";
  if (!line.includes("=")) return "cell";
  const lhs = line.split("=")[0].trim().replace(/^function /, "").replace(/^const /, "");
  return lhs.match(/^[\p{L}\p{N}_!]+/u)?.[0] ?? "cell";
}

/** Where a pick is from: "rates · lines 3–5", "plot_fit · figure", "Box · 2 cells". */
export function pickSource(pick: Pick): string {
  if (pick.part === "box") return `Box · ${pick.cells.length} cell${pick.cells.length === 1 ? "" : "s"}`;
  const part =
    pick.part === "lines" ? (pick.lines[0] === pick.lines[1] ? `line ${pick.lines[0]}` : `lines ${pick.lines[0]}–${pick.lines[1]}`) : pick.part;
  return `${cellName(pick.code)} · ${part}`;
}

/** Send picks and the comment: now as a message, or (`add`) into the composer's. */
export function sendQuote(picks: Pick[], comment: string, add: boolean): void {
  const notebook = new URLSearchParams(location.search).get("id");
  send({ type: "quote", notebook, picks, comment, add });
}

const css = `
  .endeavor-field { position: relative; display: flex; align-items: flex-end; gap: 6px; min-height: 34px; box-sizing: border-box;
    padding: 4px 5px 4px 9px; border-radius: 6px; border: 1px solid var(--e-control-edge); background: var(--e-bg-page); }
  .endeavor-field:focus-within { border-color: var(--e-accent-text); }
  .endeavor-field textarea { flex: 1; min-width: 0; resize: none; border: 0; outline: none; background: transparent; padding: 1px 0;
    height: 21px; max-height: 120px; color: var(--e-text-primary); font: 14px/21px system-ui, sans-serif; }
  .endeavor-field .options { flex: none; width: 24px; height: 24px; border: 0; border-radius: 5px; cursor: pointer;
    display: flex; align-items: center; justify-content: center; background: var(--e-bg-raised); color: var(--e-text-primary); font: 13px system-ui; }
  .endeavor-field .options:hover, .endeavor-field .options[aria-expanded="true"] { background: var(--e-menu-hover); }
  .endeavor-field [role="menu"] { position: absolute; right: -6px; top: calc(100% + 6px); z-index: 5; width: 210px; box-sizing: border-box;
    display: flex; flex-direction: column; padding: 4px; border-radius: 10px; border: 1px solid var(--e-popover-edge);
    background: var(--e-popover-bg); box-shadow: 0 12px 32px var(--e-shadow-popover); font: 13px system-ui, sans-serif; }
  .endeavor-field [role="menu"][hidden] { display: none; }
  .endeavor-field [role="menuitem"] { height: 28px; display: flex; align-items: center; gap: 8px; padding: 0 8px; border: 0; border-radius: 6px;
    background: transparent; color: var(--e-text-primary); font: inherit; cursor: pointer; text-align: left; }
  .endeavor-field [role="menuitem"]:hover { background: var(--e-menu-hover); }
  .endeavor-field [role="menuitem"] .key { margin-left: auto; padding-left: 12px; font-size: 12px; color: var(--e-text-faint); }
`;

/** The comment field Reply's prompt and Point's bar share: ↩ sends, ⌘↩ adds
 * to the message (`done(add)`), ⇧↩ is a newline; the button at its end opens
 * a menu with the same two for people who don't know the keys. Typing here
 * never reaches the notebook's shortcuts. */
export function quoteField(placeholder: string, sendLabel: string, done: (add: boolean, e: Event) => void): { root: HTMLElement; text: HTMLTextAreaElement } {
  const root = document.createElement("div");
  root.className = "endeavor-field";
  root.innerHTML =
    `<textarea rows="1" spellcheck="false" autocorrect="off" autocapitalize="off"></textarea>` +
    `<button class="options" aria-label="Send options" aria-haspopup="menu" aria-expanded="false">↑</button>` +
    `<div role="menu" hidden><button role="menuitem" data-add="false"><span></span><span class="key">↩</span></button>` +
    `<button role="menuitem" data-add="true"><span>Add to message</span><span class="key">⌘↩</span></button></div>`;
  const text = root.querySelector("textarea")!;
  const options = root.querySelector<HTMLButtonElement>(".options")!;
  const menu = root.querySelector<HTMLElement>("[role=menu]")!;
  text.placeholder = placeholder;
  menu.querySelector("[data-add=false] span")!.textContent = sendLabel;
  const showMenu = (shown: boolean) => {
    menu.hidden = !shown;
    options.setAttribute("aria-expanded", String(shown));
  };
  text.addEventListener("input", () => {
    text.style.height = "21px";
    text.style.height = `${Math.min(text.scrollHeight, 120)}px`;
  });
  text.addEventListener(
    "keydown",
    (e) => {
      e.stopPropagation();
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        showMenu(false);
        done(e.metaKey, e);
      }
    },
    true,
  );
  // Keep the keyboard in the field.
  options.onmousedown = (e) => e.preventDefault();
  options.onclick = () => showMenu(menu.hidden);
  for (const item of menu.querySelectorAll<HTMLButtonElement>("[role=menuitem]")) {
    item.onmousedown = (e) => e.preventDefault();
    item.onclick = (e) => {
      showMenu(false);
      done(item.dataset.add === "true", e);
    };
  }
  return { root, text };
}

export function initQuote(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("shot", (msg) => waiting.get(msg.id)?.());
}
