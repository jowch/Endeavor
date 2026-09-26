"use strict";
(() => {
  // src/bridge.ts
  var handlers = {};
  var nonce = typeof __ENDEAVOR_NONCE__ === "string" ? __ENDEAVOR_NONCE__ : "";
  var handler = window.webkit?.messageHandlers?.ipc;
  var post = handler ? handler.postMessage.bind(handler) : (body) => window.ipc?.postMessage(body);
  function send(msg) {
    post(JSON.stringify(nonce ? { ...msg, nonce } : msg));
  }
  var byUser = (e) => e.isTrusted || !nonce;
  function on(type, handler2) {
    (handlers[type] ??= []).push(handler2);
  }
  window.__endeavor = {
    receive(msg) {
      handlers[msg.type]?.forEach((handler2) => handler2(msg));
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
    function refresh2() {
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
      refresh2();
      send({ type: "mode", on: enable });
    }
    function sendComment(now) {
      if (!picked.size) return;
      const ids = cells().map((c) => c.id).filter((id) => picked.has(id));
      const notebook = new URLSearchParams(location.search).get("id");
      send({ type: "annotation", notebook, cells: ids, comment: text.value.trim(), now });
      text.value = "";
      picked.clear();
      refresh2();
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
      refresh2();
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
          if (byUser(e)) sendComment(e.metaKey);
        }
      },
      true
    );
    sendButton.onclick = (e) => byUser(e) && sendComment(false);
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
  /* Code the agent changed stays in view until it runs, even in a folded cell. */
  pluto-cell[data-endeavor="unrun"][data-author="agent"] > pluto-input { display: block !important; opacity: 1 !important; }
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

  // src/diff.ts
  var css4 = `
  .cm-line.endeavor-add { position: relative; z-index: 0; }
  .cm-line.endeavor-add::before {
    content: ""; position: absolute; z-index: -1; pointer-events: none;
    top: 0; bottom: 0; right: 0; left: calc(-1 * var(--indented, 0px));
    background: rgba(108, 199, 132, 0.12);
  }
  .cm-line.endeavor-add::after {
    content: "+"; position: absolute; top: 0; pointer-events: none;
    left: calc(-1 * var(--indented, 0px) - 13px); color: #6CC784; text-indent: 0;
  }
  .endeavor-add-ch { background: rgba(108, 199, 132, 0.28); border-radius: 2px; }
  .endeavor-del { position: relative; background: rgba(224, 122, 122, 0.12); color: #E07A7A; white-space: pre; }
  .endeavor-del::before { content: "\u2212"; position: absolute; left: -13px; }
  .endeavor-del-ch { background: rgba(224, 122, 122, 0.28); border-radius: 2px; }
`;
  function lineDiff(before, after) {
    const a = before === "" ? [] : before.split("\n");
    const b = after.split("\n");
    const n = a.length, m = b.length;
    if (n * m > 25e4) return [];
    const lcs = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
    for (let i2 = n - 1; i2 >= 0; i2--)
      for (let j2 = m - 1; j2 >= 0; j2--)
        lcs[i2][j2] = a[i2] === b[j2] ? lcs[i2 + 1][j2 + 1] + 1 : Math.max(lcs[i2 + 1][j2], lcs[i2][j2 + 1]);
    const hunks = [];
    let i = 0, j = 0, open2 = null;
    const hunk = () => open2 ??= (hunks.push({ at: j, removed: [], added: [] }), hunks[hunks.length - 1]);
    while (i < n || j < m) {
      if (i < n && j < m && a[i] === b[j]) {
        open2 = null;
        i++, j++;
      } else if (j < m && (i >= n || lcs[i][j + 1] >= lcs[i + 1][j])) {
        hunk().added.push(b[j++]);
      } else {
        hunk().removed.push(a[i++]);
      }
    }
    return hunks;
  }
  function changedSpan(a, b) {
    let start = 0;
    while (start < a.length && start < b.length && a[start] === b[start]) start++;
    let endA = a.length, endB = b.length;
    while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) endA--, endB--;
    return [start, endA, endB];
  }
  var cm = null;
  function classes(view) {
    if (cm) return cm;
    const EditorView = view.constructor;
    const StateEffect = EditorView.scrollIntoView(0).constructor;
    const Compartment = [...view.state.config?.compartments?.keys() ?? []][0]?.constructor;
    if (!Compartment) return null;
    for (const source of view.state.facet(EditorView.decorations)) {
      const set = typeof source === "function" ? source(view) : source;
      let c = set?.iter?.().value?.constructor;
      while (c && typeof c.line !== "function") c = Object.getPrototypeOf(c);
      if (c)
        return cm = {
          Decoration: c,
          Compartment,
          appendConfig: StateEffect.appendConfig,
          decorations: EditorView.decorations,
          extender: view.state.constructor.transactionExtender
        };
    }
    return null;
  }
  function viewOf(cell) {
    return cell.querySelector("pluto-input .cm-content")?.cmTile?.root?.view ?? null;
  }
  function removedLine(text, span) {
    return {
      text,
      toDOM() {
        const el = document.createElement("div");
        el.className = "endeavor-del";
        if (span) {
          const mark = document.createElement("span");
          mark.className = "endeavor-del-ch";
          mark.textContent = text.slice(span[0], span[1]);
          el.append(text.slice(0, span[0]), mark, text.slice(span[1]));
        } else {
          el.textContent = text || " ";
        }
        return el;
      },
      eq(other) {
        return other.text === text;
      },
      compare(other) {
        return other === this || other.text === text;
      },
      updateDOM() {
        return false;
      },
      estimatedHeight: -1,
      lineBreaks: 0,
      ignoreEvent() {
        return true;
      },
      coordsAt() {
        return null;
      },
      destroy() {
      }
    };
  }
  function decorate(doc, before) {
    const { Decoration } = cm;
    const ranges = [];
    for (const h of lineDiff(before, doc.toString())) {
      const paired = h.removed.length === h.added.length;
      const spans = h.added.map((line, k) => paired ? changedSpan(h.removed[k], line) : null);
      const at = h.at < doc.lines ? doc.line(h.at + 1).from : doc.length;
      h.removed.forEach((line, k) => {
        const span = spans[k];
        const widget = removedLine(line, span ? [span[0], span[1]] : null);
        ranges.push(Decoration.widget({ widget, block: true, side: h.at < doc.lines ? -1 : 1 }).range(at));
      });
      h.added.forEach((_, k) => {
        const line = doc.line(h.at + k + 1);
        ranges.push(Decoration.line({ class: "endeavor-add" }).range(line.from));
        const span = spans[k];
        if (span && span[2] > span[0]) ranges.push(Decoration.mark({ class: "endeavor-add-ch" }).range(line.from + span[0], line.from + span[2]));
      });
    }
    return Decoration.set(ranges, true);
  }
  var befores = /* @__PURE__ */ new Map();
  var installed = /* @__PURE__ */ new WeakMap();
  var extension = (doc, before) => before === void 0 ? [] : cm.decorations.of(decorate(doc, before));
  function refresh() {
    for (const cell of document.querySelectorAll("pluto-cell")) {
      const view = viewOf(cell);
      if (!view || !classes(view)) continue;
      const id = cell.id;
      const before = befores.get(id);
      let entry = installed.get(view);
      if (!entry) {
        if (before === void 0) continue;
        entry = { compartment: new cm.Compartment(), shown: before };
        installed.set(view, entry);
        const { compartment } = entry;
        const follow = cm.extender.of(
          (tr) => tr.docChanged && befores.has(id) ? { effects: compartment.reconfigure(extension(tr.newDoc, befores.get(id))) } : null
        );
        view.dispatch({ effects: cm.appendConfig.of([compartment.of(extension(view.state.doc, before)), follow]) });
      } else if (entry.shown !== before) {
        entry.shown = before;
        view.dispatch({ effects: entry.compartment.reconfigure(extension(view.state.doc, before)) });
      }
    }
  }
  function initDiffs() {
    const style = document.createElement("style");
    style.textContent = css4;
    document.head.append(style);
    on("cells", (msg) => {
      befores.clear();
      for (const c of msg.cells) if (typeof c.before === "string") befores.set(c.cell_id, c.before);
      refresh();
    });
    onRedraw(refresh);
  }

  // src/prompt.ts
  var AGENT = "Claude";
  var css5 = `
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
  pluto-input .cm-placeholder::after { content: "Type code, or \u2318K to ask ${AGENT}"; font-size: 13px; }
`;
  var open = null;
  function close(refocus) {
    if (!open) return;
    const { box, cell } = open;
    open = null;
    box.remove();
    if (refocus) cell.querySelector("pluto-input .cm-content")?.focus();
  }
  function place(box, cell, where) {
    const rect = cell.getBoundingClientRect();
    box.style.left = `${rect.left + window.scrollX}px`;
    const top = where === "before" ? rect.top + window.scrollY - 6 - 70 : rect.bottom + window.scrollY + 6;
    box.style.top = `${Math.max(top, 0)}px`;
    box.style.width = `${Math.max(rect.width, 280)}px`;
  }
  function isEmpty(cell) {
    return !(cell.querySelector("pluto-input .cm-content")?.textContent ?? "").trim();
  }
  function openPrompt(cell, where) {
    close(false);
    const box = document.createElement("div");
    box.id = "endeavor-prompt";
    box.dataset.endeavorUi = "";
    const asking = where !== "cell" ? `Ask ${AGENT} to write a cell here` : isEmpty(cell) ? `Ask ${AGENT} what to write here` : `Ask ${AGENT} about this cell`;
    box.innerHTML = `<textarea rows="1" spellcheck="false" autocorrect="off" autocapitalize="off"></textarea><div class="hint">\u21B5 send \xB7 esc cancel</div>`;
    const text = box.querySelector("textarea");
    text.placeholder = asking;
    text.addEventListener("input", () => {
      text.style.height = "auto";
      text.style.height = `${text.scrollHeight}px`;
    });
    text.addEventListener(
      "keydown",
      (e) => {
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
      true
    );
    document.body.append(box);
    place(box, cell, where);
    open = { box, cell, where };
    requestAnimationFrame(() => text.focus());
  }
  function initPrompt() {
    const style = document.createElement("style");
    style.textContent = css5;
    document.head.append(style);
    window.addEventListener(
      "keydown",
      (e) => {
        if (!(e.metaKey && e.key.toLowerCase() === "k") || e.shiftKey) return;
        const cell = document.activeElement?.closest("pluto-cell");
        if (!cell) return;
        e.preventDefault();
        e.stopPropagation();
        openPrompt(cell, "cell");
      },
      true
    );
    document.addEventListener("mousedown", (e) => {
      if (open && !open.box.contains(e.target)) close(false);
    });
    window.addEventListener("resize", () => open && place(open.box, open.cell, open.where));
    onRedraw(() => {
      const cells = [...document.querySelectorAll("pluto-cell")];
      cells.forEach((cell, i) => {
        const places = i === cells.length - 1 ? ["before", "after"] : ["before"];
        for (const where of places) {
          if (cell.querySelector(`:scope > .endeavor-add-agent.${where}`)) continue;
          const add = cell.querySelector(`:scope > button.add_cell.${where}`);
          if (!add) continue;
          const button = document.createElement("button");
          button.className = `endeavor-add-agent ${where}`;
          button.dataset.endeavorUi = "";
          button.textContent = `\u2726 ${AGENT}`;
          button.title = `Ask ${AGENT} to write a cell here`;
          button.onclick = () => openPrompt(cell, where);
          add.after(button);
        }
      });
      for (const stale of document.querySelectorAll("pluto-cell:not(:last-of-type) > .endeavor-add-agent.after")) stale.remove();
    });
  }

  // src/errors.ts
  var AGENT2 = "Claude";
  var css6 = `
  .fix-with-ai { display: none !important; }
  .endeavor-ask { display: flex; gap: 8px; margin: 8px 0; }
  .endeavor-ask button { font: 12px system-ui; padding: 3px 10px; border-radius: 4px; cursor: pointer;
    background: transparent; color: #E08A5E; border: 1px solid #CC3F00; }
  .endeavor-ask button.explain { color: #BDBDBD; border-color: #3A3A40; }
`;
  function decorate2() {
    for (const error of document.querySelectorAll("pluto-cell jlerror")) {
      if (error.querySelector(".endeavor-ask")) continue;
      const cell = error.closest("pluto-cell");
      if (!cell) continue;
      const row = document.createElement("div");
      row.className = "endeavor-ask";
      row.innerHTML = `<button class="fix">Fix with ${AGENT2}</button><button class="explain">Explain</button>`;
      const ask = (kind) => {
        const text = (error.querySelector("header")?.textContent ?? error.textContent ?? "").trim().slice(0, 2e3);
        const notebook = new URLSearchParams(location.search).get("id");
        send({ type: "ask", kind, notebook, cell: cell.id, error: text });
      };
      row.querySelector(".fix").onclick = (e) => byUser(e) && ask("fix");
      row.querySelector(".explain").onclick = (e) => byUser(e) && ask("explain");
      error.append(row);
    }
  }
  function initErrors() {
    const style = document.createElement("style");
    style.textContent = css6;
    document.head.append(style);
    onRedraw(decorate2);
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
    initDiffs();
    initPrompt();
    initErrors();
    initRail();
    watchRedraws();
    send({ type: "ready" });
  }
  document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
})();
