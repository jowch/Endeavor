// Annotation mode (design doc §4.2). In annotation mode, clicks pick cells
// instead of editing them; Pluto's own selection is picked up on entry. A
// comment on the picked cells is sent to the agent (keyed by cell UUID) with
// the chat box's keys: Enter sends (queued if the agent is busy), Cmd+Enter
// sends now, Shift+Enter is a newline. Only Cmd+Shift+E toggles the mode: Esc is
// reserved for stopping the agent, so a stray Esc never does two things.

import { on, send } from "./bridge";

const css = `
  body.annotating pluto-cell { cursor: crosshair; }
  body.annotating pluto-cell:hover { outline: 2px dashed #c8a040; outline-offset: 4px; }
  body.annotating pluto-cell.annotate-picked { outline: 2px solid #c8a040; outline-offset: 4px; }
  #annotate-frame { position: fixed; inset: 0; pointer-events: none; z-index: 9999;
    box-shadow: inset 0 0 0 3px #c8a040; display: none; }
  #annotate-bar { position: fixed; left: 50%; bottom: 16px; transform: translateX(-50%);
    z-index: 10000; display: none; flex-direction: column; gap: 8px;
    width: min(640px, 90vw); padding: 12px; border-radius: 12px;
    background: rgba(28, 28, 30, 0.72); color: #ddd; font: 13px system-ui;
    backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px);
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.4); }
  body.annotating #annotate-bar, body.annotating #annotate-frame { display: flex; }
  #annotate-bar textarea { resize: vertical; min-height: 3.5em; padding: 6px 8px; border-radius: 6px;
    border: 1px solid #555; background: rgba(0, 0, 0, 0.35); color: #eee; font: 13px system-ui; }
  #annotate-bar .row { display: flex; gap: 8px; align-items: center; }
  #annotate-bar .status { flex: 1; color: #aaa; }
  #annotate-bar button { padding: 4px 10px; border-radius: 6px; border: 0; cursor: pointer;
    background: #3a3a3c; color: #eee; font: 13px system-ui; }
  #annotate-bar button.primary { background: #8a6d1f; }
  #annotate-bar button:disabled { opacity: 0.4; cursor: default; }
`;

export function initAnnotate(): void {
  const picked = new Set<string>();
  const active = () => document.body.classList.contains("annotating");
  const cells = () => [...document.querySelectorAll<HTMLElement>("pluto-cell")];

  const style = document.createElement("style");
  style.textContent = css;
  const frame = document.createElement("div");
  frame.id = "annotate-frame";
  const bar = document.createElement("div");
  bar.id = "annotate-bar";
  bar.innerHTML = `
    <div class="row"><span class="status"></span></div>
    <textarea placeholder="Comment for Claude on the selected cells…"></textarea>
    <div class="row"><span class="status">↩ send · ⌘↩ send now · ⇧↩ newline · ⌘⇧E exit</span>
      <button class="exit">Done</button><button class="primary send">Send</button></div>`;
  document.head.append(style);
  document.body.append(frame, bar);

  const status = bar.querySelector<HTMLElement>(".status")!;
  const text = bar.querySelector<HTMLTextAreaElement>("textarea")!;
  const sendButton = bar.querySelector<HTMLButtonElement>(".send")!;

  function refresh() {
    status.textContent = picked.size ? `${picked.size} cell${picked.size > 1 ? "s" : ""} selected` : "Click cells to select them";
    sendButton.disabled = picked.size === 0;
    for (const c of cells()) c.classList.toggle("annotate-picked", picked.has(c.id));
  }

  function set(enable: boolean) {
    if (enable === active()) return;
    document.body.classList.toggle("annotating", enable);
    picked.clear();
    if (enable) {
      for (const c of document.querySelectorAll<HTMLElement>("pluto-cell.selected")) picked.add(c.id);
      text.focus();
    }
    refresh();
    send({ type: "mode", on: enable });
  }

  // Stays in annotation mode afterwards, ready for the next comment.
  function sendComment(now: boolean) {
    if (!picked.size) return;
    const ids = cells().map((c) => c.id).filter((id) => picked.has(id)); // notebook order
    const notebook = new URLSearchParams(location.search).get("id");
    send({ type: "annotation", notebook, cells: ids, comment: text.value.trim(), now });
    text.value = "";
    picked.clear();
    refresh();
  }

  // Capture phase so Pluto/CodeMirror never see clicks meant for picking.
  const swallow = (e: Event) => {
    const target = e.target as Element;
    if (!active() || bar.contains(target)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.type !== "click") return;
    const cell = target.closest<HTMLElement>("pluto-cell");
    if (!cell) return;
    picked.has(cell.id) ? picked.delete(cell.id) : picked.add(cell.id);
    refresh();
  };
  for (const type of ["pointerdown", "mousedown", "click"]) document.addEventListener(type, swallow, true);

  // Window capture runs before Pluto's own shortcuts (e.g. Shift+Enter runs a cell).
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.key.toLowerCase() === "e" && e.metaKey && e.shiftKey) {
        e.preventDefault();
        return set(!active());
      }
      if (!active() || !bar.contains(e.target as Node)) return;
      // Typing a comment must never trigger notebook shortcuts; defaults (newline) still apply.
      e.stopPropagation();
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        sendComment(e.metaKey);
      }
    },
    true,
  );
  sendButton.onclick = () => sendComment(false);
  bar.querySelector<HTMLButtonElement>(".exit")!.onclick = () => set(false);

  on("annotate", (msg) => set(msg.on));
}
