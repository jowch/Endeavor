// Asking the agent from inside the notebook (docs/ui-spec.md, "Between cells" and
// "Empty cell"): ⌘K in a cell opens a prompt about that cell (an empty cell: what
// to write in it); the agent button beside Pluto's "+" asks for a new cell there.
// Enter sends, Esc returns to the editor. The prompt floats over the page, so
// Pluto's cell list is never touched.

import { byUser, send } from "./bridge";
import { onRedraw } from "./redraw";

const AGENT = "Claude";

const css = `
  /* Beside Pluto's "+" in the gap above a cell (and below the last one): faint
     while the cell is hovered, like Pluto's own buttons, and full on the "+". */
  pluto-cell > .endeavor-add-agent {
    position: absolute; left: 14px; z-index: 20;
    height: 18px; padding: 0 7px; border-radius: 9px; border: 1px solid #333;
    background: #1C1C1E; color: #9A9A9A; font: 11px system-ui, sans-serif; cursor: pointer;
    opacity: 0; transition: opacity 0.1s;
  }
  pluto-cell > .endeavor-add-agent.before { top: calc(-0.5 * var(--pluto-cell-spacing, 17px) - 9px); }
  pluto-cell > .endeavor-add-agent.after { bottom: calc(-0.5 * var(--pluto-cell-spacing, 17px) - 9px); }
  pluto-cell:hover > .endeavor-add-agent { opacity: 0.35; }
  pluto-cell > button.add_cell:hover + .endeavor-add-agent, pluto-cell > .endeavor-add-agent:hover { opacity: 1; }
  pluto-cell > .endeavor-add-agent:hover { color: #FF9A6B; border-color: #CC3F00; }
  #endeavor-prompt {
    position: absolute; z-index: 1000; display: flex; flex-direction: column; gap: 6px;
    padding: 8px 10px; border-radius: 8px; border: 1px solid #CC3F00; background: #1C1C1E;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.4); font: 13px system-ui, sans-serif; color: #E6E6E6;
  }
  #endeavor-prompt textarea {
    resize: none; border: none; outline: none; background: transparent; color: inherit;
    font: inherit; min-height: 20px;
  }
  #endeavor-prompt .hint { color: #7A7A7A; font-size: 11px; }
  /* The empty-cell hint names the shortcut. */
  pluto-input .cm-placeholder { font-size: 0; }
  pluto-input .cm-placeholder::after { content: "Type code, or ⌘K to ask ${AGENT}"; font-size: 13px; }
`;

type Where = "cell" | "before" | "after";
let open: { box: HTMLElement; cell: HTMLElement; where: Where } | null = null;

function close(refocus: boolean) {
  if (!open) return;
  const { box, cell } = open;
  open = null;
  box.remove();
  if (refocus) cell.querySelector<HTMLElement>("pluto-input .cm-content")?.focus();
}

function place(box: HTMLElement, cell: HTMLElement, where: Where) {
  const rect = cell.getBoundingClientRect();
  box.style.left = `${rect.left + window.scrollX}px`;
  // A new cell before this one: the prompt sits in the gap above it.
  const top = where === "before" ? rect.top + window.scrollY - 6 - 70 : rect.bottom + window.scrollY + 6;
  box.style.top = `${Math.max(top, 0)}px`;
  box.style.width = `${Math.max(rect.width, 280)}px`;
}

function isEmpty(cell: HTMLElement) {
  return !(cell.querySelector("pluto-input .cm-content")?.textContent ?? "").trim();
}

export function openPrompt(cell: HTMLElement, where: Where) {
  close(false);
  const box = document.createElement("div");
  box.id = "endeavor-prompt";
  box.dataset.endeavorUi = "";
  const asking = where !== "cell" ? `Ask ${AGENT} to write a cell here` : isEmpty(cell) ? `Ask ${AGENT} what to write here` : `Ask ${AGENT} about this cell`;
  box.innerHTML = `<textarea rows="1" spellcheck="false" autocorrect="off" autocapitalize="off"></textarea><div class="hint">↵ send · esc cancel</div>`;
  const text = box.querySelector("textarea")!;
  text.placeholder = asking;
  text.addEventListener("input", () => {
    text.style.height = "auto";
    text.style.height = `${text.scrollHeight}px`;
  });
  text.addEventListener(
    "keydown",
    (e) => {
      // Typing here must never reach Pluto's shortcuts.
      e.stopPropagation();
      if (e.key === "Escape") {
        e.preventDefault();
        close(true);
      } else if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const comment = text.value.trim();
        if (!comment || !byUser(e)) return;
        const notebook = new URLSearchParams(location.search).get("id");
        send({ type: "prompt", notebook, cell: cell.id, where: where !== "cell" ? where : isEmpty(cell) ? "fill" : "about", text: comment, now: e.metaKey });
        close(true);
      }
    },
    true,
  );
  document.body.append(box);
  place(box, cell, where);
  open = { box, cell, where };
  // After the key event that opened it: focusing during ⌘K's keydown doesn't stick.
  requestAnimationFrame(() => text.focus());
}

export function initPrompt(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);

  // Window capture: before CodeMirror and Pluto handle the key.
  window.addEventListener(
    "keydown",
    (e) => {
      if (!(e.metaKey && e.key.toLowerCase() === "k") || e.shiftKey) return;
      const cell = (document.activeElement as Element | null)?.closest<HTMLElement>("pluto-cell");
      if (!cell) return;
      e.preventDefault();
      e.stopPropagation();
      openPrompt(cell, "cell");
    },
    true,
  );
  document.addEventListener("mousedown", (e) => {
    if (open && !open.box.contains(e.target as Node)) close(false);
  });
  window.addEventListener("resize", () => open && place(open.box, open.cell, open.where));

  // The agent button beside Pluto's "+": above each cell, and below the last.
  // Re-added when Pluto redraws.
  onRedraw(() => {
    const cells = [...document.querySelectorAll<HTMLElement>("pluto-cell")];
    cells.forEach((cell, i) => {
      const places: Array<"before" | "after"> = i === cells.length - 1 ? ["before", "after"] : ["before"];
      for (const where of places) {
        if (cell.querySelector(`:scope > .endeavor-add-agent.${where}`)) continue;
        const add = cell.querySelector(`:scope > button.add_cell.${where}`);
        if (!add) continue;
        const button = document.createElement("button");
        button.className = `endeavor-add-agent ${where}`;
        button.dataset.endeavorUi = "";
        button.textContent = `✦ ${AGENT}`;
        button.title = `Ask ${AGENT} to write a cell here`;
        button.onclick = () => openPrompt(cell, where);
        add.after(button);
      }
    });
    // A cell that stopped being last keeps a stale bottom button.
    for (const stale of document.querySelectorAll("pluto-cell:not(:last-of-type) > .endeavor-add-agent.after")) stale.remove();
  });
}
