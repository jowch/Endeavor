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

  // src/redraw.ts
  var hooks = [];
  var queued = false;
  function onRedraw(hook) {
    hooks.push(hook);
    hook();
  }
  function watchRedraws() {
    new MutationObserver((records) => {
      const ours = (r) => r.type === "attributes" && !!r.attributeName?.startsWith("data-") || r.target instanceof Element && !!r.target.closest("[data-endeavor-ui]");
      if (queued || records.every(ours)) return;
      queued = true;
      queueMicrotask(() => {
        queued = false;
        hooks.forEach((hook) => hook());
      });
    }).observe(document.body, { childList: true, subtree: true });
  }

  // src/rail.ts
  var css2 = `
  #endeavor-rail { position: fixed; right: 4px; top: 10px; bottom: 10px; width: 3px; z-index: 50; pointer-events: none; }
  #endeavor-rail a { position: absolute; left: 0; right: 0; min-height: 4px; border-radius: 2px;
    background: #CC3F00; pointer-events: auto; cursor: pointer; }
  #endeavor-rail a.user { background: #9A9A9A; }
`;
  var rail;
  function draw() {
    const marked = [...document.querySelectorAll('pluto-cell[data-endeavor="unrun"], pluto-cell.code_differs')];
    const total = Math.max(document.documentElement.scrollHeight, 1);
    rail.replaceChildren(
      ...marked.map((cell) => {
        const mark = document.createElement("a");
        const top = cell.getBoundingClientRect().top + window.scrollY;
        mark.style.top = `${100 * top / total}%`;
        mark.style.height = `${100 * cell.offsetHeight / total}%`;
        if (cell.dataset.author === "user" || cell.classList.contains("code_differs")) mark.className = "user";
        mark.dataset.cell = cell.id;
        mark.onclick = (e) => {
          e.preventDefault();
          cell.scrollIntoView({ block: "center", behavior: "smooth" });
        };
        return mark;
      })
    );
  }
  function initRail() {
    const style = document.createElement("style");
    style.textContent = css2;
    rail = document.createElement("div");
    rail.id = "endeavor-rail";
    rail.dataset.endeavorUi = "";
    document.head.append(style);
    document.body.append(rail);
    onRedraw(draw);
    window.addEventListener("resize", draw);
  }
  var redrawRail = () => rail && draw();

  // src/cells.ts
  var css3 = `
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
    style.textContent = css3;
    document.head.append(style);
    on("cells", (msg) => {
      states = new Map(msg.cells.map((c) => [c.cell_id, c]));
      apply();
      redrawRail();
    });
    onRedraw(apply);
  }

  // src/errors.ts
  var AGENT = "Claude";
  var css4 = `
  .fix-with-ai { display: none !important; }
  .endeavor-ask { display: flex; gap: 8px; margin: 8px 0; }
  .endeavor-ask button { font: 12px system-ui; padding: 3px 10px; border-radius: 4px; cursor: pointer;
    background: transparent; color: #E08A5E; border: 1px solid #CC3F00; }
  .endeavor-ask button.explain { color: #BDBDBD; border-color: #3A3A40; }
`;
  function decorate() {
    for (const error of document.querySelectorAll("pluto-cell jlerror")) {
      if (error.querySelector(".endeavor-ask")) continue;
      const cell = error.closest("pluto-cell");
      if (!cell) continue;
      const row = document.createElement("div");
      row.className = "endeavor-ask";
      row.innerHTML = `<button class="fix">Fix with ${AGENT}</button><button class="explain">Explain</button>`;
      const ask = (kind) => {
        const text = (error.querySelector("header")?.textContent ?? error.textContent ?? "").trim().slice(0, 2e3);
        const notebook = new URLSearchParams(location.search).get("id");
        send({ type: "ask", kind, notebook, cell: cell.id, error: text });
      };
      row.querySelector(".fix").onclick = () => ask("fix");
      row.querySelector(".explain").onclick = () => ask("explain");
      const header = error.querySelector(".error-header");
      header ? header.after(row) : error.prepend(row);
    }
  }
  function initErrors() {
    const style = document.createElement("style");
    style.textContent = css4;
    document.head.append(style);
    onRedraw(decorate);
  }

  // src/theme.ts
  var endeavorDark = `
@media (prefers-color-scheme: dark) {
  :root {
    --main-bg-color: #151517;
    --header-bg-color: #151517;
    --footer-bg-color: #151517;
    --rule-color: rgba(255, 255, 255, 0.08);
    --code-background: #1B1B1E;
    --normal-cell-color: rgba(100, 100, 100, 0.18);
    --dark-normal-cell-color: rgba(100, 100, 100, 0.3);
    --code-differs-cell-color: #9A9A9A;
    --selected-cell-color: rgba(143, 170, 216, 0.45);
    --pluto-output-color: #BDBDBD;
    --pluto-output-h-color: #E0E0E0;
    --pluto-output-bg-color: #151517;
    --pluto-runarea-bg-color: #1C1C1F;
    --pluto-logs-bg-color: #1C1C1F;
    --overlay-button-bg: #1C1C1F;
    --input-context-menu-bg-color: #1C1C1F;
    --input-context-menu-border-color: #2A2A2E;
    --cm-selection-background: rgba(143, 170, 216, 0.3);
    --cm-color-editor-text: #D4D4D4;
    --cm-color-keyword: #D98BB5;
    --cm-color-control-operator: #D98BB5;
    --cm-color-literal: #CFA47A;
    --cm-color-symbol: #CFA47A;
    --cm-color-string: #A3C48C;
    --cm-color-function: #9AB6E6;
    --cm-color-builtin: #9AB6E6;
    --cm-color-comment: #5E5E5E;
    --cm-color-line-numbers: #555555;
    /* Live docs */
    --helpbox-bg-color: #1C1C1F;
    --helpbox-header-bg-color: #26262A;
    --helpbox-header-tab-bg-color: #26262A;
    --helpbox-header-color: #ECECEC;
    --helpbox-text-color: #D4D4D4;
    --helpbox-search-bg-color: #151517;
    --helpbox-search-border-color: #3A3A40;
    --helpbox-box-shadow-color: rgba(0, 0, 0, 0.3);
    --docs-binding-bg: #26262A;
  }
}
footer { display: none !important; }
`;
  function initTheme() {
    const style = document.createElement("style");
    style.id = "endeavor-theme";
    document.head.append(style);
    on("theme", (msg) => {
      style.textContent = msg.name === "endeavor" ? endeavorDark : "";
    });
  }

  // src/main.ts
  function init() {
    initTheme();
    initAnnotate();
    initCells();
    initErrors();
    initRail();
    watchRedraws();
    send({ type: "ready" });
  }
  document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
})();
