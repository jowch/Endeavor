// Share and ⋮ items that act in the page, through Pluto's own functions:
// Present (window.present), Record (the Editor's recording state), Frontmatter
// (Pluto's own dialog). Also two sheets of our own: Pluto shows its keyboard
// shortcuts in a plain alert() and takes feedback in a small form in its
// footer, so the list and a feedback box are drawn here (both looks).

import { on } from "./bridge";

const css = `
  #endeavor-sheet { position: fixed; inset: 0; z-index: 200; display: flex; align-items: center; justify-content: center;
    background: rgba(0, 0, 0, 0.45); font: 13px/1.5 system-ui, -apple-system, sans-serif; color: #D4D4D4; }
  #endeavor-sheet .card { width: min(460px, calc(100vw - 32px)); max-height: calc(100vh - 64px); overflow: auto; padding: 16px 18px;
    border-radius: 10px; border: 1px solid #2A2A2E; background: #1C1C1F; box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5); }
  #endeavor-sheet h2 { margin: 0 0 10px; font-size: 14px; font-weight: 600; color: #ECECEC; }
  #endeavor-sheet .keys { display: grid; grid-template-columns: auto 1fr; gap: 4px 16px; }
  #endeavor-sheet .keys kbd { all: unset; font: 12.5px system-ui, -apple-system, sans-serif; color: #ECECEC; white-space: nowrap; }
  #endeavor-sheet .keys .head { grid-column: 1 / -1; margin-top: 8px; color: #7A7A7A; font-size: 11.5px; }
  #endeavor-sheet p { margin: 10px 0 0; color: #8C8C8C; font-size: 12px; }
  #endeavor-sheet textarea { width: 100%; box-sizing: border-box; min-height: 90px; padding: 8px; border-radius: 6px;
    border: 1px solid #3A3A40; background: #151517; color: #ECECEC; font: inherit; resize: vertical; }
  #endeavor-sheet input.email { width: 100%; box-sizing: border-box; margin-top: 8px; padding: 6px 8px; border-radius: 6px;
    border: 1px solid #3A3A40; background: #151517; color: #ECECEC; font: inherit; }
  #endeavor-sheet p.said { white-space: pre-wrap; color: #B4B4B4; }
  #endeavor-sheet button:disabled { opacity: 0.5; cursor: default; }
  #endeavor-sheet .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
  #endeavor-sheet button { padding: 4px 12px; border-radius: 5px; border: 1px solid #3A3A40; background: #26262A; color: #ECECEC;
    font: 12.5px system-ui, sans-serif; cursor: pointer; }
  #endeavor-sheet button.primary { background: #CC3F00; border-color: #CC3F00; color: #fff; }
`;

const mac = /Mac/.test(navigator.platform);
const cmd = mac ? "⌘" : "Ctrl";
const alt = mac ? "⌥" : "Alt";
const shortcuts: Array<[string, string] | string> = [
  ["⇧ Enter", "Run cell"],
  [`${cmd} Enter`, "Run cell and add a cell below"],
  [`${cmd} S`, "Submit all changes"],
  ["Delete or Backspace", "Delete an empty cell"],
  ["PageUp or fn ↑", "Jump to the cell above"],
  ["PageDown or fn ↓", "Jump to the cell below"],
  [`${mac ? "⌃" : "Ctrl"} click`, "Jump to definition"],
  [`${alt} ↑ / ${alt} ↓`, "Move line or cell up / down"],
  [`${mac ? "⌃" : "Ctrl"} /`, "Toggle comment"],
  [`${mac ? "⌃" : "Ctrl"} M`, "Toggle Markdown"],
  [`${mac ? "⌥⌘" : "Ctrl Shift"} [ / ]`, "Fold / unfold code"],
  [`${mac ? "⌃" : "Ctrl"} Q`, "Interrupt the notebook"],
  "Select cells by dragging a box from the space between them, then:",
  [`${cmd} C / ${cmd} X / ${cmd} V`, "Copy / cut / paste the selected cells"],
  "Endeavor",
  [`${cmd} K`, "Ask Claude about the cell"],
  [`${cmd} ⇧ K`, "Point: pick cells or draw a box to ask about"],
];

const escape = (s: string) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);

function sheet(html: string): HTMLElement {
  document.getElementById("endeavor-sheet")?.remove();
  const el = document.createElement("div");
  el.id = "endeavor-sheet";
  el.dataset.endeavorUi = "";
  el.innerHTML = `<div class="card">${html}</div>`;
  el.onclick = (e) => e.target === el && el.remove();
  el.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      el.remove();
    }
  });
  document.body.append(el);
  return el;
}

export function showShortcuts(): void {
  const rows = shortcuts
    .map((s) => (typeof s === "string" ? `<div class="head">${escape(s)}</div>` : `<kbd>${escape(s[0])}</kbd><span>${escape(s[1])}</span>`))
    .join("");
  const el = sheet(`<h2>Keyboard shortcuts</h2><div class="keys">${rows}</div><p>The notebook file saves every time you run a cell.</p><div class="buttons"><button class="primary done">Done</button></div>`);
  const done = el.querySelector<HTMLButtonElement>(".done")!;
  done.onclick = () => el.remove();
  done.focus();
}

/** How long to wait for Pluto's feedback form to say how sending went (it gives up after 5 s once loaded). */
const FEEDBACK_WAIT_MS = 20_000;

/**
 * Hand `opinion` to Pluto's own (hidden) feedback form and resolve with what
 * Pluto says: the message it would alert(), or null if it says nothing in
 * `waitMs`. Its form asks for an email with prompt(), answered here with `email`.
 */
export function submitFeedback(opinion: string, email: string, waitMs = FEEDBACK_WAIT_MS): Promise<string | null> {
  const form = document.querySelector<HTMLFormElement>("form#feedback");
  const field = form?.querySelector<HTMLInputElement>("#opinion");
  if (!form || !field) return Promise.resolve(null);
  const w = window as any;
  const { alert, prompt } = w;
  return new Promise((resolve) => {
    const done = (message: string | null) => {
      clearTimeout(timer);
      w.alert = alert;
      resolve(message);
    };
    const timer = setTimeout(() => done(null), waitMs);
    w.alert = (message: unknown) => done(String(message ?? ""));
    w.prompt = () => email;
    field.value = opinion;
    try {
      form.requestSubmit();
    } finally {
      w.prompt = prompt;
    }
  });
}

/** What the sheet says about Pluto's answer: its own words, under a heading that says whether it went. */
export function feedbackOutcome(message: string | null): { sent: boolean; title: string; body: string } {
  if (message === null) {
    return { sent: false, title: "No answer from Pluto's feedback form", body: "It may not have been sent. Check your internet connection and try again." };
  }
  const sent = message.startsWith("Submitted");
  return { sent, title: sent ? "Sent to Pluto's developers" : "Pluto couldn't send it", body: message.trim() };
}

/** Pluto's Instant feedback, sent through its own form, which sends it to Pluto's developers. */
function showFeedback(): void {
  const el = sheet(
    `<h2>Feedback for Pluto's developers</h2><textarea placeholder="What would you tell the people who make Pluto?"></textarea>` +
      `<input class="email" type="email" placeholder="Email, if you'd like a reply (optional)">` +
      `<p>This goes to the Pluto.jl team, not to Endeavor.</p><div class="buttons"><button class="cancel">Cancel</button><button class="primary send" disabled>Send</button></div>`,
  );
  const text = el.querySelector("textarea")!;
  const email = el.querySelector<HTMLInputElement>("input.email")!;
  const send = el.querySelector<HTMLButtonElement>(".send")!;
  text.focus();
  // Pluto's form drops anything shorter.
  text.oninput = () => (send.disabled = text.value.trim().length < 4);
  el.querySelector<HTMLButtonElement>(".cancel")!.onclick = () => el.remove();
  send.onclick = async () => {
    const card = el.querySelector(".card")!;
    card.innerHTML = `<h2>Sending…</h2>`;
    const outcome = feedbackOutcome(await submitFeedback(text.value.trim(), email.value.trim()));
    card.innerHTML = `<h2>${escape(outcome.title)}</h2><p class="said">${escape(outcome.body)}</p><div class="buttons"><button class="primary">Done</button></div>`;
    const done = card.querySelector<HTMLButtonElement>("button")!;
    done.onclick = () => el.remove();
    done.focus();
  };
}

export function initActions(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("action", (msg) => {
    const w = window as any;
    if (msg.name === "present") w.present?.();
    else if (msg.name === "record") w.editor_state_set?.({ recording_waiting_to_start: true });
    else if (msg.name === "frontmatter") window.dispatchEvent(new CustomEvent("open pluto frontmatter"));
    else if (msg.name === "shortcuts") showShortcuts();
    else if (msg.name === "feedback") showFeedback();
  });
  // Pluto's own F1 / ⌘? list is a plain alert(); this one is laid out.
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.key === "F1" || (e.key === "?" && (e.metaKey || e.ctrlKey))) {
        e.preventDefault();
        e.stopImmediatePropagation();
        showShortcuts();
      }
    },
    true,
  );
}
