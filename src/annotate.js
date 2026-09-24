// Annotation mode, injected into the Pluto page by Endeavor (design doc §4.2).
// In annotation mode, clicks pick cells instead of editing them; Pluto's own selection
// is picked up on entry. Cmd+Enter sends a comment on the picked cells to the
// agent (via window.ipc, keyed by cell UUID); "Queue comment" batches several.
(() => {
  if (window.__annotate) return;

  const css = `
    body.annotating pluto-cell { cursor: crosshair; }
    body.annotating pluto-cell:hover { outline: 2px dashed #c8a040; outline-offset: 4px; }
    body.annotating pluto-cell.annotate-picked { outline: 2px solid #c8a040; outline-offset: 4px; }
    pluto-cell[data-annotate-count]::after {
      content: "✎ " attr(data-annotate-count); position: absolute; right: -2.6rem; top: 0;
      font: 600 11px system-ui; color: #c8a040;
    }
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

  const picked = new Set();
  const on = () => document.body.classList.contains("annotating");
  const cells = () => [...document.querySelectorAll("pluto-cell")];
  const post = (msg) => window.ipc && window.ipc.postMessage(JSON.stringify(msg));

  function init() {
    const style = document.createElement("style");
    style.textContent = css;
    const frame = document.createElement("div");
    frame.id = "annotate-frame";
    const bar = document.createElement("div");
    bar.id = "annotate-bar";
    bar.innerHTML = `
      <div class="row"><span class="status"></span></div>
      <textarea placeholder="Comment for Claude on the selected cells…"></textarea>
      <div class="row"><span class="status">⌘↩ send · Esc exit</span>
        <button class="exit">Done</button><button class="add">Queue comment</button>
        <button class="primary send">Send to Claude</button></div>`;
    document.head.append(style);
    document.body.append(frame, bar);

    const status = bar.querySelector(".status");
    const text = bar.querySelector("textarea");
    const add = bar.querySelector(".add");

    function refresh() {
      status.textContent = picked.size
        ? `${picked.size} cell${picked.size > 1 ? "s" : ""} selected`
        : "Click cells to select them";
      add.disabled = picked.size === 0;
      for (const c of cells()) c.classList.toggle("annotate-picked", picked.has(c.id));
    }

    function set(enable) {
      if (enable === on()) return;
      document.body.classList.toggle("annotating", enable);
      picked.clear();
      if (enable) {
        for (const c of document.querySelectorAll("pluto-cell.selected")) picked.add(c.id);
        text.focus();
      }
      refresh();
      post({ type: "mode", on: enable });
    }

    // Queue whatever is picked, then ask Endeavor to send everything queued.
    function send() {
      queue();
      post({ type: "send" });
    }

    function queue() {
      if (!picked.size) return;
      const ids = cells().map((c) => c.id).filter((id) => picked.has(id)); // notebook order
      const notebook = new URLSearchParams(location.search).get("id");
      post({ type: "annotation", notebook, cells: ids, comment: text.value.trim() });
      for (const c of cells())
        if (picked.has(c.id)) c.dataset.annotateCount = String(Number(c.dataset.annotateCount || 0) + 1);
      text.value = "";
      picked.clear();
      refresh();
    }

    // Capture phase so Pluto/CodeMirror never see clicks meant for picking.
    const swallow = (e) => {
      if (!on() || bar.contains(e.target)) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.type !== "click") return;
      const cell = e.target.closest("pluto-cell");
      if (!cell) return;
      picked.has(cell.id) ? picked.delete(cell.id) : picked.add(cell.id);
      refresh();
    };
    for (const type of ["pointerdown", "mousedown", "click"]) document.addEventListener(type, swallow, true);

    document.addEventListener(
      "keydown",
      (e) => {
        if (e.key.toLowerCase() === "g" && e.metaKey && e.shiftKey) return (e.preventDefault(), set(!on()));
        if (!on()) return;
        if (e.key === "Escape") return (e.preventDefault(), set(false));
        if (e.key === "Enter" && e.metaKey) return (e.preventDefault(), send());
      },
      true
    );
    add.onclick = queue;
    bar.querySelector(".send").onclick = send;
    bar.querySelector(".exit").onclick = () => set(false);

    window.__annotate = {
      set,
      clearBadges() {
        for (const c of document.querySelectorAll("pluto-cell[data-annotate-count]")) delete c.dataset.annotateCount;
      },
    };
  }

  document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
})();
