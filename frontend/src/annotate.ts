// Point (design doc §4.2, "annotation mode" in the code). While it's on, the
// pointer picks the smallest thing under it instead of editing: a figure or
// an output, a code block, a Markdown paragraph, or the whole cell from its
// edge. A drag that starts on code picks whole lines, a drag anywhere else
// (or any ⌥-drag) draws a box, and Shift adds to the pick. Pluto's own
// selected cells are picked on entry. The comment bar opens by the pick
// (place.ts) and works like Reply's prompt (quote.ts): ↩ sends the picks and
// the comment now, ⌘↩ adds them to the chat's message, and Point stays on for
// the next pick. Only ⌘⇧E toggles Point (⌘E alone asks about a cell).

import { byUser, on, send } from "./bridge";
import { barPlace } from "./place";
import { type Box, type Pick, SHOOTING, cellName, pickSource, quoteField, sendQuote, shoot } from "./quote";
import { cellCode } from "./reveal";
import { modHeld, shortcut } from "./keys";

const css = `
  /* Only the hovered elements: a rule over every element in every cell restyles the whole notebook on entry. */
  body.annotating pluto-cell:hover, body.annotating pluto-cell :hover { cursor: crosshair !important; }
  /* docs/ui-spec.md, "Pointing overlay": 25% dim, 1px accent edge, plain-text hint
     pill, dashed hover and solid picked outlines, dashed box for a drawn region. */
  body.annotating .annotate-hover { outline: 1.5px dashed var(--e-hover-edge); outline-offset: 3px; }
  body.annotating .annotate-picked { outline: 1.5px solid var(--e-accent); outline-offset: 3px; }
  body.annotating.annotate-drawing .annotate-hover { outline: none; }
  body.annotating.annotate-drawing, body.annotating.annotate-drawing * { user-select: none; }
  body.annotate-numbers pluto-input .cm-content { counter-reset: endeavor-line; padding-left: 2.6em !important; }
  body.annotate-numbers pluto-input .cm-line { counter-increment: endeavor-line; position: relative; }
  body.annotate-numbers pluto-input .cm-line::before { content: counter(endeavor-line); position: absolute; left: -2.6em; width: 2em;
    text-align: right; color: var(--e-text-faint); font-size: 0.85em; }
  body.annotating pluto-input .cm-line.annotate-line { background: rgba(204, 63, 0, 0.12); }
  body.annotating pluto-input .cm-line.annotate-line::before { color: var(--e-accent-text); }
  #annotate-tag { position: absolute; z-index: 9999; display: none; pointer-events: none; padding: 0 6px; border-radius: 4px;
    background: var(--e-accent); color: #fff; font: 11px/18px system-ui; }
  body.annotating #annotate-tag.shown { display: block; }
  #annotate-frame { position: fixed; inset: 0; pointer-events: none; z-index: 9999;
    background: var(--e-dim); box-shadow: inset 0 0 0 1px var(--e-accent); display: none; }
  #annotate-box { position: absolute; z-index: 9999; pointer-events: none; display: none;
    border: 1.5px dashed var(--e-accent); border-radius: 2px; }
  body.annotating #annotate-box.shown { display: block; }
  #annotate-hint { position: fixed; top: 12px; left: 50%; transform: translateX(-50%); z-index: 10000;
    display: none; align-items: center; gap: 6px; padding: 4px 12px; border-radius: 999px; border: 1px solid transparent;
    background: var(--e-hint-bg); color: var(--e-hint-text); font: 12px system-ui; white-space: nowrap; }
  #annotate-hint .done { color: var(--e-text-primary); cursor: pointer; }
  #annotate-hint .done:hover { text-decoration: underline; }
  #annotate-bar { position: fixed; left: 50%; bottom: 16px; transform: translateX(-50%);
    z-index: 10000; display: none; flex-direction: column; gap: 6px; box-sizing: border-box;
    width: min(640px, calc(100vw - 32px)); padding: 8px; border-radius: 10px; border: 1px solid transparent;
    background: var(--e-bar-bg); color: var(--e-text-primary); font: 13px system-ui;
    backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px);
    box-shadow: 0 8px 30px var(--e-bar-shadow); }
  #annotate-bar.placed { transform: none; bottom: auto; }
  body.annotating #annotate-bar, body.annotating #annotate-frame, body.annotating #annotate-hint { display: flex; }
  @media (prefers-color-scheme: light) {
    #annotate-hint, #annotate-bar { border-color: var(--e-popover-edge); }
    #annotate-hint { box-shadow: 0 12px 32px var(--e-shadow-popover); }
  }
  #annotate-bar .head { display: flex; align-items: center; gap: 8px; font-size: 12px; }
  #annotate-bar .status { flex: 1; min-width: 0; color: var(--e-text-secondary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  #annotate-bar .keys { flex: none; color: var(--e-text-faint); }
  /* While the app takes a picture, the notebook shows as it is. */
  body.annotating.${SHOOTING} #annotate-frame, body.annotating.${SHOOTING} #annotate-box, body.annotating.${SHOOTING} #annotate-tag,
  body.annotating.${SHOOTING} #annotate-hint, body.annotating.${SHOOTING} #annotate-bar { display: none; }
  body.annotating.${SHOOTING} .annotate-picked, body.annotating.${SHOOTING} .annotate-hover { outline: none; }
  body.annotating.${SHOOTING} pluto-input .cm-line.annotate-line { background: none; }
`;

/** A drag shorter than this (in CSS pixels) is a click. */
const DRAG = 4;

/** What the pointer is over: which part of which cell, and the element that shows it. */
export type Target = { part: "cell" | "code" | "output" | "figure"; cell: HTMLElement; el: HTMLElement };

const FIGURE = "img, svg, canvas, video";
const PARAGRAPH = "p, li, h1, h2, h3, h4, h5, h6, pre, blockquote, table";

/** The smallest thing Point can pick under `el`: a figure, a Markdown
 * paragraph or an output; the code block; else (the cell's edge) the cell. */
export function targetAt(el: Element | null): Target | null {
  const cell = el?.closest<HTMLElement>("pluto-cell");
  if (!el || !cell) return null;
  const input = el.closest<HTMLElement>("pluto-input");
  if (input) return { part: "code", cell, el: input };
  const output = el.closest<HTMLElement>("pluto-output");
  if (!output) return { part: "cell", cell, el: cell };
  // The outermost figure element, not a shape inside an SVG.
  let figure = el.closest<HTMLElement>(FIGURE);
  while (figure?.parentElement?.closest(FIGURE) && output.contains(figure.parentElement.closest(FIGURE))) figure = figure.parentElement.closest<HTMLElement>(FIGURE);
  if (figure && output.contains(figure)) return { part: "figure", cell, el: figure };
  const paragraph = el.closest<HTMLElement>(PARAGRAPH);
  if (paragraph && output.contains(paragraph)) return { part: "output", cell, el: paragraph };
  return { part: output.querySelector(FIGURE) ? "figure" : "output", cell, el: output };
}

/** A picked thing: the quote it becomes, and what shows it (to outline it
 * and place the bar by it). A figure or box takes its picture on sending. */
type Picked = { pick: Pick; el?: HTMLElement; lines?: HTMLElement[]; box?: Box };

const pageBox = (r: DOMRect): Box => ({
  left: r.left + window.scrollX,
  top: r.top + window.scrollY,
  right: r.right + window.scrollX,
  bottom: r.bottom + window.scrollY,
});

/** A cell's code lines as the page shows them. */
const cmLines = (cell: Element) => [...cell.querySelectorAll<HTMLElement>("pluto-input .cm-line")];

/** What the bar says is picked: "Figure in plot_fit", "Lines 4–7 of plot_fit", "3 picks". */
export function pickStatus(picks: Pick[]): string {
  if (picks.length > 1) return `${picks.length} picks`;
  const pick = picks[0];
  if (!pick) return "Click something to pick it";
  if (pick.part === "box") return `Box over ${pick.cells.length} cell${pick.cells.length === 1 ? "" : "s"}`;
  const name = cellName(pick.code);
  switch (pick.part) {
    case "cell":
      return `Cell ${name}`;
    case "lines":
      return pick.lines[0] === pick.lines[1] ? `Line ${pick.lines[0]} of ${name}` : `Lines ${pick.lines[0]}–${pick.lines[1]} of ${name}`;
    case "output":
      return `Output of ${name}`;
    case "figure":
      return `Figure in ${name}`;
  }
}

/** The same picks: the same element, the same lines of one cell, or boxes. */
function samePick(a: Picked, b: Picked): boolean {
  if (a.box || b.box) return a === b;
  if (a.pick.part === "lines" && b.pick.part === "lines") return a.pick.cell === b.pick.cell && a.pick.lines.join() === b.pick.lines.join();
  return !!a.el && a.el === b.el;
}

let state: { picks: Picked[]; status: () => string; comment: () => string } | null = null;

/** Point's state for the state dump (debug.ts). */
export function pointState(): { picked: string[]; picks: string[]; box: boolean; status: string; comment: string } {
  const picks = state?.picks ?? [];
  const cells = picks.flatMap((p) => (p.pick.part === "box" ? p.pick.cells : [p.pick.cell]));
  return {
    picked: [...new Set(cells)],
    picks: picks.map((p) => pickSource(p.pick)),
    box: picks.some((p) => p.box),
    status: state?.status() ?? "",
    comment: state?.comment() ?? "",
  };
}

export function initAnnotate(): void {
  const picks: Picked[] = [];
  let hover: Target | null = null;
  const active = () => document.body.classList.contains("annotating");
  const cells = () => [...document.querySelectorAll<HTMLElement>("pluto-cell")];

  const style = document.createElement("style");
  style.textContent = css;
  const frame = document.createElement("div");
  frame.id = "annotate-frame";
  const box = document.createElement("div");
  box.id = "annotate-box";
  const tag = document.createElement("div");
  tag.id = "annotate-tag";
  const hint = document.createElement("div");
  hint.id = "annotate-hint";
  hint.innerHTML = `<span>Click to pick · drag over code lines · drag elsewhere for a box</span><span>·</span><span class="done" role="button">Done</span>`;
  const bar = document.createElement("div");
  bar.id = "annotate-bar";
  bar.innerHTML = `<div class="head"><span class="status"></span><span class="keys">↩ send · ${shortcut("↩")} add to message</span></div>`;
  const status = bar.querySelector<HTMLElement>(".status")!;
  const field = quoteField("Comment for Claude…", "Send", (add, e) => byUser(e) && sendComment(add));
  const text = field.text;
  bar.append(field.root);
  document.head.append(style);
  document.body.append(frame, box, tag, hint, bar);
  state = { picks, status: () => (active() ? status.textContent ?? "" : ""), comment: () => text.value };

  function drawBox(b: Box | null) {
    box.classList.toggle("shown", !!b);
    if (!b) return;
    Object.assign(box.style, { left: `${b.left}px`, top: `${b.top}px`, width: `${b.right - b.left}px`, height: `${b.bottom - b.top}px` });
  }

  /** Where the last pick is in the viewport: a box, the picked lines, or its element. */
  function lastRect(): { left: number; top: number; right: number; bottom: number } | null {
    const last = picks.at(-1);
    if (!last) return null;
    if (last.box) {
      const b = last.box;
      return { left: b.left - window.scrollX, top: b.top - window.scrollY, right: b.right - window.scrollX, bottom: b.bottom - window.scrollY };
    }
    const shown = last.lines?.length ? last.lines : last.el ? [last.el] : [];
    if (!shown.length) return null;
    const [first, end] = [shown[0].getBoundingClientRect(), shown[shown.length - 1].getBoundingClientRect()];
    return { left: first.left, top: first.top, right: Math.max(first.right, end.right), bottom: end.bottom };
  }

  // The bar opens by the pick (place.ts) and follows it as the notebook scrolls.
  function place() {
    const rect = lastRect();
    bar.classList.toggle("placed", !!rect);
    if (!rect) {
      bar.removeAttribute("style");
      return;
    }
    const at = barPlace(rect, window.innerWidth, window.innerHeight, bar.offsetHeight);
    Object.assign(bar.style, { left: `${at.left}px`, top: `${at.top}px`, width: `${at.width}px` });
  }

  function showHover(target: Target | null) {
    hover?.el.classList.remove("annotate-hover");
    hover = target;
    tag.classList.toggle("shown", !!target);
    if (!target) return;
    target.el.classList.add("annotate-hover");
    tag.textContent = { cell: "Whole cell", code: "Code", output: "Text", figure: "Figure" }[target.part];
    const r = target.el.getBoundingClientRect();
    Object.assign(tag.style, { left: `${r.left + window.scrollX}px`, top: `${r.top + window.scrollY - 22}px` });
  }

  function refresh() {
    for (const el of document.querySelectorAll(".annotate-picked")) el.classList.remove("annotate-picked");
    for (const el of document.querySelectorAll(".annotate-line")) el.classList.remove("annotate-line");
    for (const p of picks) {
      p.el?.classList.add("annotate-picked");
      for (const line of p.lines ?? []) line.classList.add("annotate-line");
    }
    drawBox([...picks].reverse().find((p) => p.box)?.box ?? null);
    status.textContent = pickStatus(picks.map((p) => p.pick));
    place();
  }

  /** Pick `next`: with Shift, add it (or take it back out); else it's the only pick. */
  function choose(next: Picked, add: boolean) {
    const at = picks.findIndex((p) => samePick(p, next));
    if (add) at >= 0 ? picks.splice(at, 1) : picks.push(next);
    else picks.splice(0, picks.length, ...(at >= 0 && picks.length === 1 ? [] : [next]));
    refresh();
    text.focus();
  }

  function targetPick(target: Target): Picked {
    const { cell, el } = target;
    const code = cellCode(cell);
    switch (target.part) {
      case "cell":
        return { pick: { part: "cell", cell: cell.id, code }, el };
      case "code": {
        const lines = code.split("\n").length;
        return { pick: { part: "lines", cell: cell.id, code, lines: [1, lines], text: code }, el };
      }
      case "output":
        return { pick: { part: "output", cell: cell.id, code, text: (el.innerText ?? el.textContent ?? "").trim() }, el };
      case "figure":
        return { pick: { part: "figure", cell: cell.id, code }, el };
    }
  }

  function linesPick(cell: HTMLElement, from: number, to: number): Picked {
    const [first, last] = [Math.min(from, to), Math.max(from, to)];
    const code = cellCode(cell);
    const text = code.split("\n").slice(first - 1, last).join("\n");
    return { pick: { part: "lines", cell: cell.id, code, lines: [first, last], text }, lines: cmLines(cell).slice(first - 1, last) };
  }

  function set(enable: boolean) {
    if (enable === active()) return;
    document.body.classList.toggle("annotating", enable);
    document.body.classList.remove("annotate-drawing");
    picks.length = 0;
    drag = null;
    showHover(null);
    if (enable) {
      for (const c of document.querySelectorAll<HTMLElement>("pluto-cell.selected")) picks.push(targetPick({ part: "cell", cell: c, el: c }));
      text.focus();
    }
    refresh();
    send({ type: "mode", on: enable });
    // Numbering restyles every code line, so it follows a frame after Point shows or hides.
    requestAnimationFrame(() => setTimeout(() => document.body.classList.toggle("annotate-numbers", active())));
  }

  // Stays in Point afterwards, ready for the next pick. Pictures are taken
  // one at a time, each brought into view.
  async function sendComment(add: boolean) {
    if (!picks.length) return;
    const sending = picks.splice(0, picks.length);
    const comment = text.value.trim();
    text.value = "";
    refresh();
    const out: Pick[] = [];
    for (const p of sending) {
      if (p.pick.part === "box" || p.pick.part === "figure") {
        const b = p.box ?? (p.el ? pageBox(p.el.getBoundingClientRect()) : null);
        out.push(b ? { ...p.pick, shot: await shoot(b) } : p.pick);
      } else out.push(p.pick);
    }
    sendQuote(out, comment, add);
  }

  // A press starts a drag: over lines when it starts on code (and ⌥ isn't
  // held), else a box, once it moves DRAG pixels.
  let drag: { x: number; y: number; moved: boolean; lines?: { cell: HTMLElement; from: number; to: number }; add: boolean } | null = null;
  let justDragged = false;
  const dragBox = (e: MouseEvent): Box => {
    const [x, y] = [e.clientX + window.scrollX, e.clientY + window.scrollY];
    return { left: Math.min(drag!.x, x), top: Math.min(drag!.y, y), right: Math.max(drag!.x, x), bottom: Math.max(drag!.y, y) };
  };
  const lineAt = (el: Element | null, cell: HTMLElement) => {
    const line = el?.closest<HTMLElement>(".cm-line");
    return line && cell.contains(line) ? cmLines(cell).indexOf(line) + 1 : 0;
  };
  const ours = (target: Element) => bar.contains(target) || hint.contains(target);

  // Capture phase so Pluto/CodeMirror never see presses meant for picking.
  const swallow = (e: Event) => {
    const target = e.target as Element;
    if (!active() || ours(target)) return;
    // Cancelling pointerdown would also cancel the mousedown/mousemove/mouseup drags listen for.
    if (e.type !== "pointerdown") e.preventDefault();
    e.stopPropagation();
    const m = e as MouseEvent;
    if (e.type === "mousedown" && m.button === 0) {
      const cell = target.closest<HTMLElement>("pluto-cell");
      const line = cell && !m.altKey ? lineAt(target, cell) : 0;
      drag = { x: m.clientX + window.scrollX, y: m.clientY + window.scrollY, moved: false, add: m.shiftKey, lines: line ? { cell: cell!, from: line, to: line } : undefined };
    }
    if (e.type !== "click") return;
    if (justDragged) {
      justDragged = false;
      return;
    }
    const found = targetAt(target);
    if (found) choose(targetPick(found), m.shiftKey);
  };
  for (const type of ["pointerdown", "mousedown", "click"]) document.addEventListener(type, swallow, true);

  document.addEventListener(
    "mousemove",
    (e) => {
      if (!active()) return;
      if (!drag) return showHover(ours(e.target as Element) ? null : targetAt(e.target as Element));
      if (!drag.moved && Math.hypot(e.clientX + window.scrollX - drag.x, e.clientY + window.scrollY - drag.y) < DRAG) return;
      drag.moved = true;
      e.preventDefault();
      showHover(null);
      if (drag.lines) {
        const line = lineAt(e.target as Element, drag.lines.cell);
        if (line) drag.lines.to = line;
        const shown = linesPick(drag.lines.cell, drag.lines.from, drag.lines.to);
        for (const el of document.querySelectorAll(".annotate-line")) el.classList.remove("annotate-line");
        for (const el of shown.lines ?? []) el.classList.add("annotate-line");
        return;
      }
      document.body.classList.add("annotate-drawing");
      drawBox(dragBox(e));
    },
    true,
  );
  document.addEventListener(
    "mouseup",
    (e) => {
      if (!drag) return;
      const done = drag;
      const drawn = done.moved && !done.lines ? dragBox(e) : null;
      drag = null;
      document.body.classList.remove("annotate-drawing");
      if (!done.moved || !active()) return;
      justDragged = true;
      setTimeout(() => (justDragged = false), 0);
      if (done.lines) return choose(linesPick(done.lines.cell, done.lines.from, done.lines.to), done.add);
      const under = cells().filter((c) => {
        const r = pageBox(c.getBoundingClientRect());
        return drawn!.left < r.right && r.left < drawn!.right && drawn!.top < r.bottom && r.top < drawn!.bottom;
      });
      choose({ pick: { part: "box", cells: under.map((c) => c.id) }, box: drawn! }, done.add);
    },
    true,
  );
  window.addEventListener("scroll", () => active() && place(), true);
  window.addEventListener("resize", () => active() && place());

  // Window capture runs before Pluto's own shortcuts (e.g. Shift+Enter runs a cell).
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.key.toLowerCase() === "e" && modHeld(e) && e.shiftKey) {
        e.preventDefault();
        return set(!active());
      }
      if (e.key === "Escape" && active()) {
        e.preventDefault();
        e.stopPropagation();
        return set(false);
      }
    },
    true,
  );
  hint.querySelector<HTMLElement>(".done")!.onclick = () => set(false);

  on("annotate", (msg) => set(msg.on));
}
