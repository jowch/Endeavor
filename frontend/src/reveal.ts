// Chips in the chat point back into the notebook: clicking a cell chip scrolls
// to its cells and outlines them briefly, and a chip's popover asks for a
// cell's code now, to say whether it changed since the message was sent. A
// link in an agent's reply to a cell that isn't here says so instead.

import { on, send } from "./bridge";

const css = `
  pluto-cell.endeavor-flash { outline: 2px solid var(--e-accent); outline-offset: 4px; border-radius: 4px;
    transition: outline-color 0.3s; }
  pluto-cell.endeavor-flash.fading { outline-color: transparent; }
  .endeavor-gone { position: fixed; top: 12px; left: 50%; transform: translateX(-50%); z-index: 1000;
    padding: 6px 12px; border-radius: 6px; font: 13px system-ui, sans-serif;
    background: var(--e-bg-raised, #26262A); color: var(--e-text-primary, #F0EFEC);
    border: 1px solid var(--e-border, #2A2A2E); transition: opacity 0.3s; }
  .endeavor-gone.fading { opacity: 0; }
`;

/** Says briefly, at the top of the page, that a linked cell isn't in this notebook. */
function sayGone() {
  document.querySelector(".endeavor-gone")?.remove();
  const note = document.createElement("div");
  note.className = "endeavor-gone";
  note.setAttribute("role", "status");
  note.textContent = "That cell isn't in this notebook.";
  document.body.append(note);
  setTimeout(() => note.classList.add("fading"), 2200);
  setTimeout(() => note.remove(), 2600);
}

/** A cell's code: from its editor when there is one (exact line breaks). */
export function cellCode(cell: Element | null): string {
  const content = cell?.querySelector("pluto-input .cm-content") as any;
  const view = content?.cmTile?.root?.view;
  if (view) return view.state.doc.toString();
  const lines = content ? [...content.querySelectorAll(".cm-line")].map((l: Element) => l.textContent ?? "") : [];
  return lines.join("\n");
}

function reveal(ids: string[]) {
  const cells = ids.map((id) => document.getElementById(id)).filter((c): c is HTMLElement => !!c);
  if (!cells.length) return sayGone();
  cells[0].scrollIntoView({ block: "center", behavior: "smooth" });
  for (const cell of cells) {
    cell.classList.remove("fading");
    cell.classList.add("endeavor-flash");
  }
  setTimeout(() => cells.forEach((c) => c.classList.add("fading")), 900);
  setTimeout(() => cells.forEach((c) => c.classList.remove("endeavor-flash", "fading")), 1300);
}

export function initReveal(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("reveal", (msg) => reveal(msg.cells));
  on("code", (msg) => {
    const cell = document.getElementById(msg.cell);
    send({ type: "code", cell: msg.cell, code: cell ? cellCode(cell) : null });
  });
}
