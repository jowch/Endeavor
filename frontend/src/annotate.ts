// Annotation mode (design doc §4.2), "Point" in the app. In annotation mode,
// clicks pick cells instead of editing them, and a drag draws a box over part
// of the notebook; Pluto's own selection is picked up on entry. A comment on
// the picked cells or the box is sent to the agent (keyed by cell UUID; a box
// also goes as a picture the app takes) with the chat box's keys: Enter sends
// (queued if the agent is busy), Cmd+Enter sends now, Shift+Enter is a
// newline. Only Cmd+Shift+K toggles the mode (Cmd+K alone asks about a cell):
// Esc is reserved for stopping the agent, so a stray Esc never does two things.

import { byUser, on, send } from "./bridge";
import { cellCode } from "./reveal";

const css = `
  body.annotating pluto-cell { cursor: crosshair; }
  /* docs/ui-spec.md, "Pointing overlay": 25% dim, 1px accent edge, plain-text hint
     pill, dashed hover and solid picked outlines, dashed box for a drawn region. */
  body.annotating pluto-cell:hover { outline: 1.5px dashed #FF7A40; outline-offset: 4px; }
  body.annotating pluto-cell.annotate-picked { outline: 1.5px solid #CC3F00; outline-offset: 4px; }
  body.annotating.annotate-drawing pluto-cell:hover { outline: none; }
  body.annotating.annotate-drawing, body.annotating.annotate-drawing * { cursor: crosshair !important; user-select: none; }
  #annotate-frame { position: fixed; inset: 0; pointer-events: none; z-index: 9999;
    background: rgba(0, 0, 0, 0.25); box-shadow: inset 0 0 0 1px #CC3F00; display: none; }
  #annotate-box { position: absolute; z-index: 9999; pointer-events: none; display: none;
    border: 1.5px dashed #CC3F00; border-radius: 2px; }
  body.annotating #annotate-box.shown { display: block; }
  #annotate-hint { position: fixed; top: 12px; left: 50%; transform: translateX(-50%); z-index: 10000;
    display: none; align-items: center; gap: 6px; padding: 4px 12px; border-radius: 999px;
    background: rgba(28, 28, 30, 0.85); color: #ccc; font: 12px system-ui; white-space: nowrap; }
  #annotate-hint .done { color: #fff; cursor: pointer; }
  #annotate-hint .done:hover { text-decoration: underline; }
  #annotate-bar { position: fixed; left: 50%; bottom: 16px; transform: translateX(-50%);
    z-index: 10000; display: none; flex-direction: row; align-items: flex-end; gap: 8px;
    width: min(640px, 90vw); padding: 6px; border-radius: 10px;
    background: rgba(28, 28, 30, 0.72); color: #ddd; font: 13px system-ui;
    backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px);
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.4); }
  body.annotating #annotate-bar, body.annotating #annotate-frame, body.annotating #annotate-hint { display: flex; }
  #annotate-bar textarea { flex: 1; resize: none; height: 28px; max-height: 120px; padding: 5px 8px; border-radius: 6px;
    border: 1px solid #555; background: rgba(0, 0, 0, 0.35); color: #eee; font: 13px/18px system-ui; box-sizing: border-box; }
  #annotate-bar .status { flex: none; align-self: center; color: #aaa; font-size: 12px; white-space: nowrap; }
  #annotate-bar button { height: 28px; padding: 0 10px; border-radius: 6px; border: 0; cursor: pointer;
    background: #3a3a3c; color: #eee; font: 13px system-ui; }
  #annotate-bar button.primary { background: #CC3F00; color: #fff; }
  #annotate-bar button:disabled { opacity: 0.4; cursor: default; }
  /* While the app takes the box's picture, the notebook shows as it is. */
  body.annotating.annotate-shooting #annotate-frame, body.annotating.annotate-shooting #annotate-box,
  body.annotating.annotate-shooting #annotate-hint, body.annotating.annotate-shooting #annotate-bar { display: none; }
  body.annotating.annotate-shooting pluto-cell, body.annotating.annotate-shooting pluto-cell:hover { outline: none; }
`;

/** A drag shorter than this (in CSS pixels) is a click. */
const DRAG = 4;

/** A box in page coordinates (scrolls with the notebook). */
type Box = { left: number; top: number; right: number; bottom: number };

export function initAnnotate(): void {
  const picked = new Set<string>();
  let region: Box | null = null;
  const active = () => document.body.classList.contains("annotating");
  const cells = () => [...document.querySelectorAll<HTMLElement>("pluto-cell")];

  const style = document.createElement("style");
  style.textContent = css;
  const frame = document.createElement("div");
  frame.id = "annotate-frame";
  const box = document.createElement("div");
  box.id = "annotate-box";
  const hint = document.createElement("div");
  hint.id = "annotate-hint";
  hint.innerHTML = `<span>Click a cell or drag a box</span><span>·</span><span class="done" role="button">Done</span>`;
  const bar = document.createElement("div");
  bar.id = "annotate-bar";
  bar.innerHTML = `<span class="status"></span><textarea rows="1" placeholder="Comment for Claude…" title="↩ send · ⌘↩ send now · ⇧↩ newline · Esc or ⌘⇧K exit"></textarea><button class="primary send">Send</button>`;
  document.head.append(style);
  document.body.append(frame, box, hint, bar);

  const status = bar.querySelector<HTMLElement>(".status")!;
  const text = bar.querySelector<HTMLTextAreaElement>("textarea")!;
  const sendButton = bar.querySelector<HTMLButtonElement>(".send")!;
  // One line until the comment needs more, so the bar covers as little of the notebook as it can.
  const fit = () => {
    text.style.height = "28px";
    text.style.height = `${Math.min(text.scrollHeight + 2, 120)}px`;
  };
  text.addEventListener("input", fit);

  const pageBox = (r: DOMRect): Box => ({
    left: r.left + window.scrollX,
    top: r.top + window.scrollY,
    right: r.right + window.scrollX,
    bottom: r.bottom + window.scrollY,
  });
  const overlaps = (a: Box, b: Box) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

  // The bar floats under what's picked (the box, or the last cell in notebook
  // order), next to what it's about; otherwise it waits at the bottom of the window.
  function place() {
    const last = cells().filter((c) => picked.has(c.id)).at(-1);
    const under = region ?? (last ? pageBox(last.getBoundingClientRect()) : null);
    if (!under) {
      bar.removeAttribute("style");
      return;
    }
    const width = Math.min(Math.max(under.right - under.left, 320), 640);
    Object.assign(bar.style, {
      position: "absolute",
      transform: "none",
      bottom: "auto",
      left: `${under.left}px`,
      top: `${under.bottom + 10}px`,
      width: `${width}px`,
    });
  }

  function drawBox(b: Box | null) {
    box.classList.toggle("shown", !!b);
    if (!b) return;
    Object.assign(box.style, {
      left: `${b.left}px`,
      top: `${b.top}px`,
      width: `${b.right - b.left}px`,
      height: `${b.bottom - b.top}px`,
    });
  }

  function refresh() {
    place();
    drawBox(region);
    const n = picked.size;
    const cellsText = `${n} cell${n === 1 ? "" : "s"}`;
    status.textContent = region ? `Box over ${cellsText}` : n ? `${cellsText} selected` : "Click cells to select them";
    sendButton.disabled = !region && n === 0;
    for (const c of cells()) c.classList.toggle("annotate-picked", picked.has(c.id));
  }

  function set(enable: boolean) {
    if (enable === active()) return;
    document.body.classList.toggle("annotating", enable);
    document.body.classList.remove("annotate-drawing", "annotate-shooting");
    picked.clear();
    region = null;
    drag = null;
    if (enable) {
      for (const c of document.querySelectorAll<HTMLElement>("pluto-cell.selected")) picked.add(c.id);
      text.focus();
    }
    refresh();
    send({ type: "mode", on: enable });
  }

  const nextFrame = () => new Promise<void>((done) => requestAnimationFrame(() => done()));

  // A box goes with a picture: bring it into view, hide the overlay, and let
  // the app take the picture before it shows again ("shot").
  async function sendRegion(b: Box, ids: string[], codes: string[], notebook: string | null, comment: string, now: boolean) {
    const top = b.top - window.scrollY;
    const bottom = b.bottom - window.scrollY;
    if (top < 0 || bottom > window.innerHeight) window.scrollBy(0, top < 0 || bottom - top > window.innerHeight ? top - 8 : bottom - window.innerHeight + 8);
    document.body.classList.add("annotate-shooting");
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
    send({ type: "region", notebook, cells: ids, codes, comment, now, rect });
    setTimeout(() => document.body.classList.remove("annotate-shooting"), 3000);
  }

  // Stays in annotation mode afterwards, ready for the next comment.
  function sendComment(now: boolean) {
    if (!picked.size && !region) return;
    const ids = cells().map((c) => c.id).filter((id) => picked.has(id)); // notebook order
    const notebook = new URLSearchParams(location.search).get("id");
    const codes = ids.map((id) => cellCode(document.getElementById(id)));
    const comment = text.value.trim();
    if (region) void sendRegion(region, ids, codes, notebook, comment, now);
    else send({ type: "annotation", notebook, cells: ids, codes, comment, now });
    text.value = "";
    fit();
    picked.clear();
    region = null;
    refresh();
  }

  // A press starts a drag; it becomes a box once it moves DRAG pixels.
  let drag: { x: number; y: number; drawing: boolean } | null = null;
  let justDrew = false;
  const dragBox = (e: MouseEvent): Box => {
    const [x, y] = [e.clientX + window.scrollX, e.clientY + window.scrollY];
    return { left: Math.min(drag!.x, x), top: Math.min(drag!.y, y), right: Math.max(drag!.x, x), bottom: Math.max(drag!.y, y) };
  };

  // Capture phase so Pluto/CodeMirror never see clicks meant for picking.
  const swallow = (e: Event) => {
    const target = e.target as Element;
    if (!active() || bar.contains(target) || hint.contains(target)) return;
    // Cancelling pointerdown would also cancel the mousedown/mousemove/mouseup
    // that drawing a box listens for.
    if (e.type !== "pointerdown") e.preventDefault();
    e.stopPropagation();
    if (e.type === "mousedown" && (e as MouseEvent).button === 0) {
      const m = e as MouseEvent;
      drag = { x: m.clientX + window.scrollX, y: m.clientY + window.scrollY, drawing: false };
    }
    if (e.type !== "click") return;
    if (justDrew) {
      justDrew = false;
      return;
    }
    const cell = target.closest<HTMLElement>("pluto-cell");
    if (!cell) return;
    region = null;
    picked.has(cell.id) ? picked.delete(cell.id) : picked.add(cell.id);
    refresh();
  };
  for (const type of ["pointerdown", "mousedown", "click"]) document.addEventListener(type, swallow, true);

  document.addEventListener(
    "mousemove",
    (e) => {
      if (!drag || !active()) return;
      if (!drag.drawing && Math.hypot(e.clientX + window.scrollX - drag.x, e.clientY + window.scrollY - drag.y) < DRAG) return;
      drag.drawing = true;
      document.body.classList.add("annotate-drawing");
      e.preventDefault();
      drawBox(dragBox(e));
    },
    true,
  );
  document.addEventListener(
    "mouseup",
    (e) => {
      if (!drag) return;
      const drawn = drag.drawing ? dragBox(e) : null;
      drag = null;
      document.body.classList.remove("annotate-drawing");
      if (!drawn || !active()) return;
      justDrew = true;
      setTimeout(() => (justDrew = false), 0);
      region = drawn;
      picked.clear();
      for (const c of cells()) if (overlaps(drawn, pageBox(c.getBoundingClientRect()))) picked.add(c.id);
      refresh();
      text.focus();
    },
    true,
  );

  // Window capture runs before Pluto's own shortcuts (e.g. Shift+Enter runs a cell).
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.key.toLowerCase() === "k" && e.metaKey && e.shiftKey) {
        e.preventDefault();
        return set(!active());
      }
      if (e.key === "Escape" && active()) {
        e.preventDefault();
        e.stopPropagation();
        return set(false);
      }
      if (!active() || !bar.contains(e.target as Node)) return;
      // Typing a comment must never trigger notebook shortcuts; defaults (newline) still apply.
      e.stopPropagation();
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        if (byUser(e)) sendComment(e.metaKey);
      }
    },
    true,
  );
  sendButton.onclick = (e) => byUser(e) && sendComment(false);
  hint.querySelector<HTMLElement>(".done")!.onclick = () => set(false);

  on("annotate", (msg) => set(msg.on));
  on("shot", () => document.body.classList.remove("annotate-shooting"));
}
