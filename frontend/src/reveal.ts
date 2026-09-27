// Chips in the chat point back into the notebook: clicking a cell chip scrolls
// to its cells and outlines them briefly, and a chip's popover asks for a
// cell's code now, to say whether it changed since the message was sent.

import { on, send } from "./bridge";

const css = `
  pluto-cell.endeavor-flash { outline: 2px solid #CC3F00; outline-offset: 4px; border-radius: 4px;
    transition: outline-color 0.3s; }
  pluto-cell.endeavor-flash.fading { outline-color: transparent; }
`;

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
  if (!cells.length) return;
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
