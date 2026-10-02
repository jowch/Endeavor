// The one prompt for asking Claude from the notebook (docs/ui-spec.md, "The
// prompt"): ⌘E on a cell, the ✦ Claude button between cells, Reply on a
// selection, and Point's bar all build it here. A top line says what goes with
// the message and the keys; under it an optional faint quote, then the field
// with a send button (grey while empty, orange with words) and a ⌄ menu with
// the same two ways to send. ↩ sends now (queued while Claude works), ⌘↩
// (Ctrl+Enter off macOS) adds to the composer's message, ⇧↩ is a new line.
// Typing here never reaches the notebook's shortcuts.

import { on } from "./bridge";
import { mac, modHeld } from "./keys";

const svg = (body: string, size = 12) =>
  `<svg width="${size}" height="${size}" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

/** What goes with the message, as the top line's icon shows it. */
export const ICONS = {
  cell: svg(`<rect x="2.5" y="3" width="11" height="10" rx="1.5"></rect><path d="M5 6.5h6M5 9.5h4"></path>`),
  lines: svg(`<path d="M2 4.5h6M2 8h5M2 11.5h6"></path><path d="M11.5 3v10M10 3h3M10 13h3"></path>`),
  text: svg(`<path d="M3 4.5h10M3 8h10M3 11.5h6"></path>`),
  add: svg(`<path d="M8 3.5v9M3.5 8h9"></path>`),
  // Point's own glyph (the board's Pointer icon, Glyph::Pointer in new_session.rs).
  point: svg(`<path d="M3.3 2 3.3 13.3 6.4 10.4 8.8 14.7 10.7 13.7 8.4 9.5 12.7 9.5Z"></path>`),
  none: "",
} as const;
export type Icon = keyof typeof ICONS;

const UP = svg(`<path d="M8 13V3.5M4 7.5l4-4 4 4"></path>`, 14);
const DOWN = svg(`<path d="M4.5 6.25 8 9.75l3.5-3.5"></path>`);
const QUOTE = svg(`<path d="M3 4.5h3.75v3.75H5c0 1.6-.6 2.6-2 3.25"></path><path d="M9.25 4.5H13v3.75h-1.75c0 1.6-.6 2.6-2 3.25"></path>`, 14);

/** The keys as this platform writes them: "↩" and "⌘↩", or "Enter" and "Ctrl+Enter". */
export const KEYS = mac ? { send: "↩", add: "⌘↩" } : { send: "Enter", add: "Ctrl+Enter" };

/** While Claude works, ↩ queues: the root element carries this flag (from the app's "context"). */
const WORKING = "endeavorWorking";

const css = `
  .endeavor-prompt { box-sizing: border-box; display: flex; flex-direction: column; gap: 6px; font: 13px/18px system-ui, sans-serif;
    color: var(--e-text-primary); text-align: left; }
  .endeavor-prompt.popover { position: fixed; z-index: 1000; padding: 7px 8px 8px; border-radius: 10px; border: 1px solid var(--e-popover-edge);
    background: var(--e-popover-bg); box-shadow: 0 12px 32px var(--e-shadow-popover); }
  .endeavor-prompt .head { display: flex; align-items: center; gap: 8px; padding: 0 2px 0 4px; font-size: 12px; line-height: 17px;
    color: var(--e-text-faint); white-space: nowrap; }
  .endeavor-prompt .what { flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px; overflow: hidden; text-overflow: ellipsis; }
  .endeavor-prompt .what svg { flex: none; }
  .endeavor-prompt .what .name { font-family: JuliaMono, ui-monospace, monospace; font-size: 11.5px; color: var(--e-text-secondary); }
  .endeavor-prompt .keys { flex: none; font-size: 11px; }
  .endeavor-prompt .quote { margin: 0 4px; border-left: 2px solid var(--e-control-edge); padding-left: 8px; font-size: 11.5px; line-height: 17px;
    color: var(--e-text-faint); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .endeavor-prompt .quote.code { font-family: JuliaMono, ui-monospace, monospace; }
  .endeavor-prompt .note { padding: 0 4px; font-size: 12px; color: var(--e-text-faint); }
  .endeavor-field { position: relative; display: flex; align-items: flex-end; gap: 4px; min-height: 34px; box-sizing: border-box;
    padding: 4px 4px 4px 9px; border-radius: 6px; border: 1px solid var(--e-control-edge); background: var(--e-bg-page); }
  .endeavor-field:focus-within { border-color: var(--e-focus-ring); }
  .endeavor-field textarea { flex: 1; min-width: 0; resize: none; border: 0; outline: none; background: transparent; padding: 1.5px 0;
    height: 21px; max-height: 126px; overflow-y: auto; color: var(--e-text-primary); font: 14px/21px system-ui, sans-serif; }
  .endeavor-field textarea::placeholder { color: var(--e-text-muted); }
  .endeavor-field .send { flex: none; width: 24px; height: 24px; border: 0; padding: 0; border-radius: 50%; cursor: pointer;
    display: flex; align-items: center; justify-content: center; background: var(--e-bg-raised); color: var(--e-text-faint); }
  .endeavor-field .send.ready { background: var(--e-accent); color: #fff; }
  .endeavor-field .options { flex: none; width: 18px; height: 24px; border: 0; padding: 0; border-radius: 5px; cursor: pointer;
    display: flex; align-items: center; justify-content: center; background: transparent; color: var(--e-text-muted); }
  .endeavor-field .options:hover, .endeavor-field .options[aria-expanded="true"] { background: var(--e-control-edge); color: var(--e-text-primary); }
  .endeavor-field [role="menu"] { position: absolute; right: -6px; top: calc(100% + 6px); z-index: 5; width: ${mac ? 216 : 236}px; box-sizing: border-box;
    display: flex; flex-direction: column; padding: 4px; border-radius: 10px; border: 1px solid var(--e-popover-edge);
    background: var(--e-popover-bg); box-shadow: 0 12px 32px var(--e-shadow-popover); font: 13px system-ui, sans-serif; }
  .endeavor-field [role="menu"][hidden] { display: none; }
  .endeavor-field [role="menuitem"] { height: 28px; flex: none; display: flex; align-items: center; gap: 8px; padding: 0 8px; border: 0; border-radius: 6px;
    background: transparent; color: var(--e-text-primary); font: inherit; cursor: pointer; text-align: left; }
  .endeavor-field [role="menuitem"]:hover { background: var(--e-menu-hover); }
  .endeavor-field [role="menuitem"] svg { color: var(--e-text-muted); }
  .endeavor-field [role="menuitem"] .key { margin-left: auto; padding-left: 12px; font-size: 12px; color: var(--e-text-faint); white-space: nowrap; }
  html:not([data-endeavor-working]) .endeavor-prompt .working, html[data-endeavor-working] .endeavor-prompt .idle { display: none; }
`;

export type AskBox = {
  root: HTMLElement;
  text: HTMLTextAreaElement;
  /** The top line's "what goes with the message" part. */
  what: HTMLElement;
  setWhat(icon: Icon, name: string, detail: string): void;
};

/** Build a prompt. `done(add, e)` is ↩ or Send now (`add` false), ⌘↩ or Add to message (true). */
export function askBox(o: { label: string; placeholder: string; quote?: { text: string; code: boolean }; done: (add: boolean, e: Event) => void }): AskBox {
  const root = document.createElement("div");
  root.className = "endeavor-prompt";
  root.dataset.endeavorUi = "";
  root.setAttribute("role", "dialog");
  root.setAttribute("aria-label", o.label);
  root.innerHTML =
    `<div class="head"><span class="what"></span><span class="keys"><span class="idle">${KEYS.send} send</span><span class="working">${KEYS.send} queue</span> · ${KEYS.add} add to message</span></div>` +
    `<div class="endeavor-field"><textarea rows="1" spellcheck="false" autocorrect="off" autocapitalize="off"></textarea>` +
    `<button class="send" aria-label="Send">${UP}</button>` +
    `<button class="options" aria-label="Send options" aria-haspopup="menu" aria-expanded="false">${DOWN}</button>` +
    `<div role="menu" hidden><button role="menuitem" data-add="false">${UP}<span class="idle">Send now</span><span class="working">Send after this turn</span><span class="key">${KEYS.send}</span></button>` +
    `<button role="menuitem" data-add="true">${QUOTE}<span>Add to message</span><span class="key">${KEYS.add}</span></button></div></div>` +
    `<div class="note working">Claude is working. This goes after its turn.</div>`;
  const what = root.querySelector<HTMLElement>(".what")!;
  const field = root.querySelector<HTMLElement>(".endeavor-field")!;
  const text = root.querySelector("textarea")!;
  const sendButton = root.querySelector<HTMLButtonElement>(".send")!;
  const options = root.querySelector<HTMLButtonElement>(".options")!;
  const menu = root.querySelector<HTMLElement>("[role=menu]")!;
  text.placeholder = o.placeholder;
  text.setAttribute("aria-label", o.placeholder);
  if (o.quote) {
    const quote = document.createElement("div");
    quote.className = o.quote.code ? "quote code" : "quote";
    quote.textContent = o.quote.text.split(/\s+/).join(" ").trim();
    field.before(quote);
  }
  const showMenu = (shown: boolean) => {
    menu.hidden = !shown;
    options.setAttribute("aria-expanded", String(shown));
  };
  // Up to six lines, then it scrolls; the buttons stay at the bottom right.
  const fit = () => {
    sendButton.classList.toggle("ready", !!text.value.trim());
    text.style.height = "21px";
    text.style.height = `${Math.min(text.scrollHeight, 126)}px`;
  };
  text.addEventListener("input", fit);
  text.addEventListener(
    "keydown",
    (e) => {
      e.stopPropagation();
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        showMenu(false);
        o.done(modHeld(e), e);
      }
    },
    true,
  );
  // Keep the keyboard in the field.
  for (const button of root.querySelectorAll<HTMLButtonElement>("button")) button.onmousedown = (e) => e.preventDefault();
  options.onclick = () => showMenu(menu.hidden);
  sendButton.onclick = (e) => o.done(false, e);
  for (const item of menu.querySelectorAll<HTMLButtonElement>("[role=menuitem]")) {
    item.onclick = (e) => {
      showMenu(false);
      o.done(item.dataset.add === "true", e);
    };
  }
  const setWhat = (icon: Icon, name: string, detail: string) => {
    what.innerHTML = ICONS[icon];
    if (name) {
      const span = document.createElement("span");
      span.className = "name";
      span.textContent = name;
      what.append(span, Object.assign(document.createElement("span"), { textContent: "·" }));
    }
    what.append(Object.assign(document.createElement("span"), { className: "status", textContent: detail }));
  };
  return { root, text, what, setWhat };
}

export function initAskBox(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("context", (msg) => {
    if (msg.working) document.documentElement.dataset[WORKING] = "";
    else delete document.documentElement.dataset[WORKING];
  });
}
