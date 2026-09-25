"use strict";
(() => {
  // src/bridge.ts
  var handlers = {};
  function send(msg) {
    window.ipc?.postMessage(JSON.stringify(msg));
  }
  function on(type, handler) {
    handlers[type] = handler;
  }
  window.__endeavor = {
    receive(msg) {
      const handler = handlers[msg.type];
      handler?.(msg);
    }
  };

  // src/annotate.ts
  var css = `
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
  function initAnnotate() {
    const picked = /* @__PURE__ */ new Set();
    const active = () => document.body.classList.contains("annotating");
    const cells = () => [...document.querySelectorAll("pluto-cell")];
    const style = document.createElement("style");
    style.textContent = css;
    const frame = document.createElement("div");
    frame.id = "annotate-frame";
    const bar = document.createElement("div");
    bar.id = "annotate-bar";
    bar.innerHTML = `
    <div class="row"><span class="status"></span></div>
    <textarea placeholder="Comment for Claude on the selected cells\u2026"></textarea>
    <div class="row"><span class="status">\u21A9 send \xB7 \u2318\u21A9 send now \xB7 \u21E7\u21A9 newline \xB7 \u2318\u21E7E exit</span>
      <button class="exit">Done</button><button class="primary send">Send</button></div>`;
    document.head.append(style);
    document.body.append(frame, bar);
    const status = bar.querySelector(".status");
    const text = bar.querySelector("textarea");
    const sendButton = bar.querySelector(".send");
    function refresh() {
      status.textContent = picked.size ? `${picked.size} cell${picked.size > 1 ? "s" : ""} selected` : "Click cells to select them";
      sendButton.disabled = picked.size === 0;
      for (const c of cells()) c.classList.toggle("annotate-picked", picked.has(c.id));
    }
    function set(enable) {
      if (enable === active()) return;
      document.body.classList.toggle("annotating", enable);
      picked.clear();
      if (enable) {
        for (const c of document.querySelectorAll("pluto-cell.selected")) picked.add(c.id);
        text.focus();
      }
      refresh();
      send({ type: "mode", on: enable });
    }
    function sendComment(now) {
      if (!picked.size) return;
      const ids = cells().map((c) => c.id).filter((id) => picked.has(id));
      const notebook = new URLSearchParams(location.search).get("id");
      send({ type: "annotation", notebook, cells: ids, comment: text.value.trim(), now });
      text.value = "";
      picked.clear();
      refresh();
    }
    const swallow = (e) => {
      const target = e.target;
      if (!active() || bar.contains(target)) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.type !== "click") return;
      const cell = target.closest("pluto-cell");
      if (!cell) return;
      picked.has(cell.id) ? picked.delete(cell.id) : picked.add(cell.id);
      refresh();
    };
    for (const type of ["pointerdown", "mousedown", "click"]) document.addEventListener(type, swallow, true);
    window.addEventListener(
      "keydown",
      (e) => {
        if (e.key.toLowerCase() === "e" && e.metaKey && e.shiftKey) {
          e.preventDefault();
          return set(!active());
        }
        if (!active() || !bar.contains(e.target)) return;
        e.stopPropagation();
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          sendComment(e.metaKey);
        }
      },
      true
    );
    sendButton.onclick = () => sendComment(false);
    bar.querySelector(".exit").onclick = () => set(false);
    on("annotate", (msg) => set(msg.on));
  }

  // src/cells.ts
  var css2 = `
  pluto-cell { position: relative; }
  pluto-cell[data-endeavor="unrun"]::before, pluto-cell.code_differs::before {
    content: ""; position: absolute; left: -8px; top: 0; bottom: 0; width: 4px;
    border-radius: 2px; pointer-events: none;
    background: repeating-linear-gradient(-45deg, #CC3F00 0 3px, rgba(204, 63, 0, 0.3) 3px 6px);
  }
  pluto-cell[data-endeavor="unrun"][data-author="user"]::before, pluto-cell.code_differs::before {
    background: repeating-linear-gradient(-45deg, #9A9A9A 0 3px, rgba(154, 154, 154, 0.25) 3px 6px);
  }
  pluto-cell[data-endeavor="unrun"] > pluto-output { opacity: 0.4; }
`;
  var states = /* @__PURE__ */ new Map();
  function apply() {
    for (const cell of document.querySelectorAll("pluto-cell")) {
      const state = states.get(cell.id);
      setAttr(cell, "data-endeavor", state?.unrun ? "unrun" : null);
      setAttr(cell, "data-author", state?.author ?? null);
    }
  }
  function setAttr(el, name, value) {
    if (value === null) {
      if (el.hasAttribute(name)) el.removeAttribute(name);
    } else if (el.getAttribute(name) !== value) {
      el.setAttribute(name, value);
    }
  }
  function initCells() {
    const style = document.createElement("style");
    style.textContent = css2;
    document.head.append(style);
    on("cells", (msg) => {
      states = new Map(msg.cells.map((c) => [c.cell_id, c]));
      apply();
    });
    let queued = false;
    new MutationObserver((records) => {
      if (queued || records.every((r) => r.type === "attributes" && r.attributeName?.startsWith("data-"))) return;
      queued = true;
      queueMicrotask(() => (queued = false, apply()));
    }).observe(document.body, { childList: true, subtree: true });
  }

  // src/main.ts
  function init() {
    initAnnotate();
    initCells();
    send({ type: "ready" });
  }
  document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
})();
