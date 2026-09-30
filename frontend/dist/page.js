"use strict";
(() => {
  // src/bridge.ts
  var handlers = {};
  var nonce = typeof __ENDEAVOR_NONCE__ === "string" ? __ENDEAVOR_NONCE__ : "";
  var handler = window.webkit?.messageHandlers?.ipc;
  var post = handler ? handler.postMessage.bind(handler) : (body2) => window.ipc?.postMessage(body2);
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

  // src/actions.ts
  var css = `
  #endeavor-sheet { position: fixed; inset: 0; z-index: 200; display: flex; align-items: center; justify-content: center;
    background: var(--e-backdrop); font: 13px/1.5 system-ui, -apple-system, sans-serif; color: var(--e-text-code); }
  #endeavor-sheet .card { width: min(460px, calc(100vw - 32px)); max-height: calc(100vh - 64px); overflow: auto; padding: 16px 18px;
    border-radius: 10px; border: 1px solid var(--e-dialog-edge); background: var(--e-dialog-bg); box-shadow: 0 16px 48px var(--e-dialog-shadow); }
  #endeavor-sheet h2 { margin: 0 0 10px; font-size: 14px; font-weight: 600; color: var(--e-text-primary); }
  #endeavor-sheet .keys { display: grid; grid-template-columns: auto 1fr; gap: 4px 16px; }
  #endeavor-sheet .keys kbd { all: unset; font: 12.5px system-ui, -apple-system, sans-serif; color: var(--e-text-primary); white-space: nowrap; }
  #endeavor-sheet .keys .head { grid-column: 1 / -1; margin-top: 8px; color: var(--e-text-dim); font-size: 11.5px; }
  #endeavor-sheet p { margin: 10px 0 0; color: var(--e-text-muted); font-size: 12px; }
  #endeavor-sheet textarea { width: 100%; box-sizing: border-box; min-height: 90px; padding: 8px; border-radius: 6px;
    border: 1px solid var(--e-control-edge); background: var(--e-bg-page); color: var(--e-text-primary); font: inherit; resize: vertical; }
  #endeavor-sheet input.email { width: 100%; box-sizing: border-box; margin-top: 8px; padding: 6px 8px; border-radius: 6px;
    border: 1px solid var(--e-control-edge); background: var(--e-bg-page); color: var(--e-text-primary); font: inherit; }
  #endeavor-sheet p.said { white-space: pre-wrap; color: var(--e-text-secondary); }
  #endeavor-sheet button:disabled { opacity: 0.5; cursor: default; }
  #endeavor-sheet .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
  #endeavor-sheet button { padding: 4px 12px; border-radius: 5px; border: 1px solid var(--e-control-edge); background: var(--e-bg-raised); color: var(--e-text-primary);
    font: 12.5px system-ui, sans-serif; cursor: pointer; }
  #endeavor-sheet button.primary { background: var(--e-accent); border-color: var(--e-accent); color: #fff; }
`;
  var mac = /Mac/.test(navigator.platform);
  var cmd = mac ? "\u2318" : "Ctrl";
  var alt = mac ? "\u2325" : "Alt";
  var shortcuts = [
    ["\u21E7 Enter", "Run cell"],
    [`${cmd} Enter`, "Run cell and add a cell below"],
    [`${cmd} S`, "Submit all changes"],
    ["Delete or Backspace", "Delete an empty cell"],
    ["PageUp or fn \u2191", "Jump to the cell above"],
    ["PageDown or fn \u2193", "Jump to the cell below"],
    [`${mac ? "\u2303" : "Ctrl"} click`, "Jump to definition"],
    [`${alt} \u2191 / ${alt} \u2193`, "Move line or cell up / down"],
    [`${mac ? "\u2303" : "Ctrl"} /`, "Toggle comment"],
    [`${mac ? "\u2303" : "Ctrl"} M`, "Toggle Markdown"],
    [`${mac ? "\u2325\u2318" : "Ctrl Shift"} [ / ]`, "Fold / unfold code"],
    [`${mac ? "\u2303" : "Ctrl"} Q`, "Interrupt the notebook"],
    "Select cells by dragging a box from the space between them, then:",
    [`${cmd} C / ${cmd} X / ${cmd} V`, "Copy / cut / paste the selected cells"],
    "Endeavor",
    [`${cmd} K`, "Ask Claude about the cell"],
    [`${cmd} \u21E7 K`, "Point: pick cells or draw a box to ask about"]
  ];
  var escape = (s) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
  function sheet(html) {
    document.getElementById("endeavor-sheet")?.remove();
    const el = document.createElement("div");
    el.id = "endeavor-sheet";
    el.dataset.endeavorUi = "";
    el.innerHTML = `<div class="card">${html}</div>`;
    el.onclick = (e) => e.target === el && el.remove();
    el.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        el.remove();
      }
    });
    document.body.append(el);
    return el;
  }
  function showShortcuts() {
    const rows = shortcuts.map((s) => typeof s === "string" ? `<div class="head">${escape(s)}</div>` : `<kbd>${escape(s[0])}</kbd><span>${escape(s[1])}</span>`).join("");
    const el = sheet(`<h2>Keyboard shortcuts</h2><div class="keys">${rows}</div><p>The notebook file saves every time you run a cell.</p><div class="buttons"><button class="primary done">Done</button></div>`);
    const done = el.querySelector(".done");
    done.onclick = () => el.remove();
    done.focus();
  }
  var FEEDBACK_WAIT_MS = 2e4;
  function submitFeedback(opinion, email, waitMs = FEEDBACK_WAIT_MS) {
    const form = document.querySelector("form#feedback");
    const field = form?.querySelector("#opinion");
    if (!form || !field) return Promise.resolve(null);
    const w = window;
    const { alert, prompt } = w;
    return new Promise((resolve) => {
      const done = (message) => {
        clearTimeout(timer);
        w.alert = alert;
        resolve(message);
      };
      const timer = setTimeout(() => done(null), waitMs);
      w.alert = (message) => done(String(message ?? ""));
      w.prompt = () => email;
      field.value = opinion;
      try {
        form.requestSubmit();
      } finally {
        w.prompt = prompt;
      }
    });
  }
  function feedbackOutcome(message) {
    if (message === null) {
      return { sent: false, title: "No answer from Pluto's feedback form", body: "It may not have been sent. Check your internet connection and try again." };
    }
    const sent = message.startsWith("Submitted");
    return { sent, title: sent ? "Sent to Pluto's developers" : "Pluto couldn't send it", body: message.trim() };
  }
  function showFeedback() {
    const el = sheet(
      `<h2>Feedback for Pluto's developers</h2><textarea placeholder="What would you tell the people who make Pluto?"></textarea><input class="email" type="email" placeholder="Email, if you'd like a reply (optional)"><p>This goes to the Pluto.jl team, not to Endeavor.</p><div class="buttons"><button class="cancel">Cancel</button><button class="primary send" disabled>Send</button></div>`
    );
    const text = el.querySelector("textarea");
    const email = el.querySelector("input.email");
    const send2 = el.querySelector(".send");
    text.focus();
    text.oninput = () => send2.disabled = text.value.trim().length < 4;
    el.querySelector(".cancel").onclick = () => el.remove();
    send2.onclick = async () => {
      const card = el.querySelector(".card");
      card.innerHTML = `<h2>Sending\u2026</h2>`;
      const outcome = feedbackOutcome(await submitFeedback(text.value.trim(), email.value.trim()));
      card.innerHTML = `<h2>${escape(outcome.title)}</h2><p class="said">${escape(outcome.body)}</p><div class="buttons"><button class="primary">Done</button></div>`;
      const done = card.querySelector("button");
      done.onclick = () => el.remove();
      done.focus();
    };
  }
  function initActions() {
    const style2 = document.createElement("style");
    style2.textContent = css;
    document.head.append(style2);
    on("action", (msg) => {
      const w = window;
      if (msg.name === "present") w.present?.();
      else if (msg.name === "record") w.editor_state_set?.({ recording_waiting_to_start: true });
      else if (msg.name === "frontmatter") window.dispatchEvent(new CustomEvent("open pluto frontmatter"));
      else if (msg.name === "shortcuts") showShortcuts();
      else if (msg.name === "feedback") showFeedback();
    });
    window.addEventListener(
      "keydown",
      (e) => {
        if (e.key === "F1" || e.key === "?" && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          e.stopImmediatePropagation();
          showShortcuts();
        }
      },
      true
    );
  }

  // src/reveal.ts
  var css2 = `
  pluto-cell.endeavor-flash { outline: 2px solid var(--e-accent); outline-offset: 4px; border-radius: 4px;
    transition: outline-color 0.3s; }
  pluto-cell.endeavor-flash.fading { outline-color: transparent; }
`;
  function cellCode(cell) {
    const content = cell?.querySelector("pluto-input .cm-content");
    const view = content?.cmTile?.root?.view;
    if (view) return view.state.doc.toString();
    const lines = content ? [...content.querySelectorAll(".cm-line")].map((l) => l.textContent ?? "") : [];
    return lines.join("\n");
  }
  function reveal(ids) {
    const cells = ids.map((id) => document.getElementById(id)).filter((c) => !!c);
    if (!cells.length) return;
    cells[0].scrollIntoView({ block: "center", behavior: "smooth" });
    for (const cell of cells) {
      cell.classList.remove("fading");
      cell.classList.add("endeavor-flash");
    }
    setTimeout(() => cells.forEach((c) => c.classList.add("fading")), 900);
    setTimeout(() => cells.forEach((c) => c.classList.remove("endeavor-flash", "fading")), 1300);
  }
  function initReveal() {
    const style2 = document.createElement("style");
    style2.textContent = css2;
    document.head.append(style2);
    on("reveal", (msg) => reveal(msg.cells));
    on("code", (msg) => {
      const cell = document.getElementById(msg.cell);
      send({ type: "code", cell: msg.cell, code: cell ? cellCode(cell) : null });
    });
  }

  // src/annotate.ts
  var css3 = `
  body.annotating pluto-cell { cursor: crosshair; }
  /* docs/ui-spec.md, "Pointing overlay": 25% dim, 1px accent edge, plain-text hint
     pill, dashed hover and solid picked outlines, dashed box for a drawn region. */
  body.annotating pluto-cell:hover { outline: 1.5px dashed var(--e-hover-edge); outline-offset: 4px; }
  body.annotating pluto-cell.annotate-picked { outline: 1.5px solid var(--e-accent); outline-offset: 4px; }
  body.annotating.annotate-drawing pluto-cell:hover { outline: none; }
  body.annotating.annotate-drawing, body.annotating.annotate-drawing * { cursor: crosshair !important; user-select: none; }
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
    z-index: 10000; display: none; flex-direction: row; align-items: flex-end; gap: 8px;
    width: min(640px, 90vw); padding: 6px; border-radius: 10px; border: 1px solid transparent;
    background: var(--e-bar-bg); color: var(--e-text-primary); font: 13px system-ui;
    backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px);
    box-shadow: 0 8px 30px var(--e-bar-shadow); }
  body.annotating #annotate-bar, body.annotating #annotate-frame, body.annotating #annotate-hint { display: flex; }
  @media (prefers-color-scheme: light) {
    #annotate-hint, #annotate-bar { border-color: var(--e-popover-edge); }
    #annotate-hint { box-shadow: 0 12px 32px var(--e-shadow-popover); }
  }
  #annotate-bar textarea { flex: 1; resize: none; height: 28px; max-height: 120px; padding: 5px 8px; border-radius: 6px;
    border: 1px solid var(--e-field-edge); background: var(--e-field-bg); color: var(--e-text-primary); font: 13px/18px system-ui; box-sizing: border-box; }
  #annotate-bar .status { flex: none; align-self: center; color: var(--e-text-secondary); font-size: 12px; white-space: nowrap; }
  #annotate-bar button { height: 28px; padding: 0 10px; border-radius: 6px; border: 0; cursor: pointer;
    background: var(--e-annotate-btn-bg); color: var(--e-text-primary); font: 13px system-ui; }
  #annotate-bar button.primary { background: var(--e-accent); color: #fff; }
  #annotate-bar button:disabled { opacity: 0.4; cursor: default; }
  /* While the app takes the box's picture, the notebook shows as it is. */
  body.annotating.annotate-shooting #annotate-frame, body.annotating.annotate-shooting #annotate-box,
  body.annotating.annotate-shooting #annotate-hint, body.annotating.annotate-shooting #annotate-bar { display: none; }
  body.annotating.annotate-shooting pluto-cell, body.annotating.annotate-shooting pluto-cell:hover { outline: none; }
`;
  var DRAG = 4;
  function initAnnotate() {
    const picked = /* @__PURE__ */ new Set();
    let region = null;
    const active = () => document.body.classList.contains("annotating");
    const cells = () => [...document.querySelectorAll("pluto-cell")];
    const style2 = document.createElement("style");
    style2.textContent = css3;
    const frame2 = document.createElement("div");
    frame2.id = "annotate-frame";
    const box = document.createElement("div");
    box.id = "annotate-box";
    const hint = document.createElement("div");
    hint.id = "annotate-hint";
    hint.innerHTML = `<span>Click a cell or drag a box</span><span>\xB7</span><span class="done" role="button">Done</span>`;
    const bar = document.createElement("div");
    bar.id = "annotate-bar";
    bar.innerHTML = `<span class="status"></span><textarea rows="1" autocorrect="off" autocapitalize="off" spellcheck="false" placeholder="Comment for Claude\u2026" title="\u21A9 send \xB7 \u2318\u21A9 send now \xB7 \u21E7\u21A9 newline \xB7 Esc or \u2318\u21E7K exit"></textarea><button class="primary send">Send</button>`;
    document.head.append(style2);
    document.body.append(frame2, box, hint, bar);
    const status = bar.querySelector(".status");
    const text = bar.querySelector("textarea");
    const sendButton = bar.querySelector(".send");
    const fit = () => {
      text.style.height = "28px";
      text.style.height = `${Math.min(text.scrollHeight + 2, 120)}px`;
    };
    text.addEventListener("input", fit);
    const pageBox = (r) => ({
      left: r.left + window.scrollX,
      top: r.top + window.scrollY,
      right: r.right + window.scrollX,
      bottom: r.bottom + window.scrollY
    });
    const overlaps = (a, b) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
    function place3() {
      const last2 = cells().filter((c) => picked.has(c.id)).at(-1);
      const under = region ?? (last2 ? pageBox(last2.getBoundingClientRect()) : null);
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
        width: `${width}px`
      });
    }
    function drawBox(b) {
      box.classList.toggle("shown", !!b);
      if (!b) return;
      Object.assign(box.style, {
        left: `${b.left}px`,
        top: `${b.top}px`,
        width: `${b.right - b.left}px`,
        height: `${b.bottom - b.top}px`
      });
    }
    function refresh2() {
      place3();
      drawBox(region);
      const n = picked.size;
      const cellsText = `${n} cell${n === 1 ? "" : "s"}`;
      status.textContent = region ? `Box over ${cellsText}` : n ? `${cellsText} selected` : "Click cells to select them";
      sendButton.disabled = !region && n === 0;
      for (const c of cells()) c.classList.toggle("annotate-picked", picked.has(c.id));
    }
    function set(enable) {
      if (enable === active()) return;
      document.body.classList.toggle("annotating", enable);
      document.body.classList.remove("annotate-drawing", "annotate-shooting");
      picked.clear();
      region = null;
      drag = null;
      if (enable) {
        for (const c of document.querySelectorAll("pluto-cell.selected")) picked.add(c.id);
        text.focus();
      }
      refresh2();
      send({ type: "mode", on: enable });
    }
    const nextFrame = () => new Promise((done) => requestAnimationFrame(() => done()));
    async function sendRegion(b, ids, codes, notebook, comment, now) {
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
        height: Math.min(window.innerHeight, b.bottom - window.scrollY) - y
      };
      send({ type: "region", notebook, cells: ids, codes, comment, now, rect });
      setTimeout(() => document.body.classList.remove("annotate-shooting"), 3e3);
    }
    function sendComment(now) {
      if (!picked.size && !region) return;
      const ids = cells().map((c) => c.id).filter((id) => picked.has(id));
      const notebook = new URLSearchParams(location.search).get("id");
      const codes = ids.map((id) => cellCode(document.getElementById(id)));
      const comment = text.value.trim();
      if (region) void sendRegion(region, ids, codes, notebook, comment, now);
      else send({ type: "annotation", notebook, cells: ids, codes, comment, now });
      text.value = "";
      fit();
      picked.clear();
      region = null;
      refresh2();
    }
    let drag = null;
    let justDrew = false;
    const dragBox = (e) => {
      const [x, y] = [e.clientX + window.scrollX, e.clientY + window.scrollY];
      return { left: Math.min(drag.x, x), top: Math.min(drag.y, y), right: Math.max(drag.x, x), bottom: Math.max(drag.y, y) };
    };
    const swallow = (e) => {
      const target = e.target;
      if (!active() || bar.contains(target) || hint.contains(target)) return;
      if (e.type !== "pointerdown") e.preventDefault();
      e.stopPropagation();
      if (e.type === "mousedown" && e.button === 0) {
        const m = e;
        drag = { x: m.clientX + window.scrollX, y: m.clientY + window.scrollY, drawing: false };
      }
      if (e.type !== "click") return;
      if (justDrew) {
        justDrew = false;
        return;
      }
      const cell = target.closest("pluto-cell");
      if (!cell) return;
      region = null;
      picked.has(cell.id) ? picked.delete(cell.id) : picked.add(cell.id);
      refresh2();
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
      true
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
        setTimeout(() => justDrew = false, 0);
        region = drawn;
        picked.clear();
        for (const c of cells()) if (overlaps(drawn, pageBox(c.getBoundingClientRect()))) picked.add(c.id);
        refresh2();
        text.focus();
      },
      true
    );
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
    hint.querySelector(".done").onclick = () => set(false);
    on("annotate", (msg) => set(msg.on));
    on("shot", () => document.body.classList.remove("annotate-shooting"));
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
  var css4 = `
  #endeavor-rail { position: fixed; right: 4px; top: 10px; bottom: 10px; width: 3px; z-index: 50; pointer-events: none; }
  #endeavor-rail a { position: absolute; left: 0; right: 0; min-height: 4px; border-radius: 2px;
    background: var(--e-accent); pointer-events: auto; cursor: pointer; }
  #endeavor-rail a.user { background: var(--e-you-stripe); }
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
    const style2 = document.createElement("style");
    style2.textContent = css4;
    rail = document.createElement("div");
    rail.id = "endeavor-rail";
    rail.dataset.endeavorUi = "";
    document.head.append(style2);
    document.body.append(rail);
    onRedraw(draw);
    window.addEventListener("resize", draw);
  }
  var redrawRail = () => rail && draw();

  // src/cells.ts
  var css5 = `
  pluto-cell { position: relative; }
  pluto-cell[data-endeavor="unrun"]::before, pluto-cell.code_differs::before {
    content: ""; position: absolute; left: -8px; top: 0; bottom: 0; width: 4px;
    border-radius: 2px; pointer-events: none;
    background: repeating-linear-gradient(-45deg, var(--e-accent) 0 3px, var(--e-stripe-tint) 3px 6px);
  }
  pluto-cell[data-endeavor="unrun"][data-author="user"]::before, pluto-cell.code_differs::before {
    background: repeating-linear-gradient(-45deg, var(--e-you-stripe) 0 3px, var(--e-you-stripe-tint) 3px 6px);
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
    const style2 = document.createElement("style");
    style2.textContent = css5;
    document.head.append(style2);
    on("cells", (msg) => {
      states = new Map(msg.cells.map((c) => [c.cell_id, c]));
      apply();
      redrawRail();
    });
    onRedraw(apply);
  }

  // src/debug.ts
  function initDebug() {
    on("debug", () => {
      const annotating = document.body.classList.contains("annotating");
      const drawer2 = document.documentElement.dataset.endeavorDrawer;
      send({
        type: "debug",
        point: annotating,
        picked: [...document.querySelectorAll("pluto-cell.annotate-picked")].map((c) => c.id),
        box: !!document.querySelector("#annotate-box.shown"),
        point_status: annotating ? document.querySelector("#annotate-bar .status")?.textContent ?? "" : "",
        comment: document.querySelector("#annotate-bar textarea")?.value ?? "",
        drawer: drawer2 === "docs" || drawer2 === "status" ? drawer2 : null,
        callout: !!document.querySelector("#endeavor-safe.shown"),
        // Recorded by the debug build's own script, which wraps `alert`.
        alerts: window.__endeavorAlerts ?? null
      });
    });
  }

  // src/diff.ts
  var css6 = `
  .cm-line.endeavor-add { position: relative; z-index: 0; }
  .cm-line.endeavor-add::before {
    content: ""; position: absolute; z-index: -1; pointer-events: none;
    top: 0; bottom: 0; right: 0; left: calc(-1 * var(--indented, 0px));
    background: var(--e-diff-add-tint);
  }
  .cm-line.endeavor-add::after {
    content: "+"; position: absolute; top: 0; pointer-events: none;
    left: calc(-1 * var(--indented, 0px) - 13px); color: var(--e-diff-add); text-indent: 0;
  }
  .endeavor-add-ch { background: var(--e-diff-add-ch); border-radius: 2px; }
  .endeavor-del { position: relative; background: var(--e-diff-del-tint); color: var(--e-diff-del); white-space: pre; }
  .endeavor-del::before { content: "\u2212"; position: absolute; left: -13px; }
  .endeavor-del-ch { background: var(--e-diff-del-ch); border-radius: 2px; }
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
        const follow2 = cm.extender.of(
          (tr) => tr.docChanged && befores.has(id) ? { effects: compartment.reconfigure(extension(tr.newDoc, befores.get(id))) } : null
        );
        view.dispatch({ effects: cm.appendConfig.of([compartment.of(extension(view.state.doc, before)), follow2]) });
      } else if (entry.shown !== before) {
        entry.shown = before;
        view.dispatch({ effects: entry.compartment.reconfigure(extension(view.state.doc, before)) });
      }
    }
  }
  function initDiffs() {
    const style2 = document.createElement("style");
    style2.textContent = css6;
    document.head.append(style2);
    on("cells", (msg) => {
      befores.clear();
      for (const c of msg.cells) if (typeof c.before === "string") befores.set(c.cell_id, c.before);
      refresh();
    });
    onRedraw(refresh);
  }

  // src/status.ts
  function phaseOf(entry) {
    if (!entry) return "waiting";
    if (entry.success === false) return "failed";
    if (entry.finished_at != null) return "done";
    if (entry.started_at != null) return "busy";
    return "waiting";
  }
  var ansi = /\x1b\[[0-9;]*m/g;
  function parsePkgLog(log) {
    const precompiled = [];
    const failed = [];
    const added = [];
    let inManifest = false;
    for (const raw of log.replace(ansi, "").split("\n")) {
      const line = raw.trimEnd();
      const mark = line.match(/^\s*(?:[\d.]+\s*ms)?\s*([✓✗])\s+([\w.]+)/);
      if (mark) (mark[1] === "\u2713" ? precompiled : failed).push(mark[2]);
      if (/^\s*Updating\s+`.*Manifest\.toml`/.test(line)) {
        inManifest = true;
        continue;
      }
      const entry = line.match(/\[[0-9a-f]{8}\]\s+(\+)?\s*([\w.]+)/);
      if (!entry) inManifest = false;
      else if (inManifest && entry[1]) added.push(entry[2]);
    }
    return { precompiled, failed, added };
  }
  function importedPackages(codes) {
    const names = [];
    for (const code of codes) {
      for (const line of code.split("\n")) {
        const m = line.match(/^\s*(?:using|import)\s+([^#]+)/);
        if (!m) continue;
        const list = m[1].split(":")[0];
        for (const part of list.split(",")) {
          const name = part.trim().split(/[.\s]/)[0];
          if (/^[A-Za-z_]\w*$/.test(name) && !["Base", "Core", "Main"].includes(name) && !names.includes(name)) names.push(name);
        }
      }
    }
    return names;
  }
  function prettyTime(ns) {
    if (ns < 1e3) return `${Math.round(ns)} ns`;
    if (ns < 1e6) return `${Math.round(ns / 1e3)} \xB5s`;
    if (ns < 1e9) return `${Math.round(ns / 1e6)} ms`;
    const s = ns / 1e9;
    if (s < 60) return `${s < 10 ? s.toFixed(1) : Math.round(s)} s`;
    return `${Math.floor(s / 60)} min ${Math.round(s % 60)} s`;
  }
  function cellName(nb, id) {
    const defined = Object.keys(nb.cell_dependencies?.[id]?.downstream_cells_map ?? {});
    if (defined.length) return defined.slice(0, 2).join(", ") + (defined.length > 2 ? ", \u2026" : "");
    const first = (nb.cell_inputs[id]?.code ?? "").split("\n").find((l) => l.trim()) ?? "";
    const line = first.trim();
    return line.length > 28 ? `${line.slice(0, 27)}\u2026` : line;
  }
  function statusModel(nb) {
    const tree = nb.status_tree?.subtasks ?? {};
    const pkgTask = tree.pkg;
    const pkgPhase = phaseOf(pkgTask);
    const runTask = tree.run;
    const workspace = phaseOf(tree.workspace);
    const running = nb.process_status === "ready" || nb.process_status === "starting";
    const steps = [
      { name: "Start Julia", phase: tree.workspace ? workspace : running ? "done" : "waiting" },
      { name: "Packages", phase: pkgTask ? pkgPhase : running ? "done" : "waiting" },
      { name: "Run cells", phase: phaseOf(runTask) }
    ];
    const nbpkg = nb.nbpkg ?? {};
    const installed2 = nbpkg.installed_versions ?? {};
    const busy = new Set(nbpkg.busy_packages ?? []);
    const log = parsePkgLog(nbpkg.terminal_outputs?.nbpkg_sync ?? "");
    const precompiled = new Set(log.precompiled);
    const failedSet = new Set(log.failed);
    const precompiling = phaseOf(pkgTask?.subtasks?.precompile) === "busy";
    const codes = nb.cell_order.map((id) => nb.cell_inputs[id]?.code ?? "");
    const direct = importedPackages(codes);
    for (const name of [...Object.keys(installed2), ...busy].sort()) {
      if (!name.startsWith("__internal") && name !== "nbpkg_sync" && !direct.includes(name)) direct.push(name);
    }
    const packages = direct.map((name) => {
      const version = installed2[name];
      const detail = version === "stdlib" ? "standard library" : version ?? "";
      let state;
      if (pkgPhase === "failed" && (failedSet.has(name) || failedSet.size === 0 && busy.has(name))) state = "failed";
      else if (busy.has(name) && pkgPhase === "busy") state = precompiled.has(name) ? "ready" : precompiling ? "precompiling" : "installing";
      else if (version != null) state = "ready";
      else if (pkgPhase === "done") state = Object.keys(installed2).length ? "failed" : "ready";
      else state = "waiting";
      const notFound = state === "failed" && version == null && pkgPhase === "done";
      return { name, state, detail: state === "ready" ? detail : notFound ? "not found" : "" };
    });
    const deps = log.added.filter((n) => !direct.includes(n));
    const depsRow = deps.length ? { count: deps.length, precompiled: deps.filter((n) => precompiled.has(n)).length, failed: deps.filter((n) => failedSet.has(n)).length } : null;
    const cells = nb.cell_order.map((id) => {
      const r = nb.cell_results[id] ?? {};
      const state = r.running ? "running" : r.queued ? "waiting" : r.errored ? "failed" : r.runtime != null ? "done" : "waiting";
      const time = (state === "done" || state === "failed") && r.runtime != null ? prettyTime(r.runtime) : null;
      return { id, name: cellName(nb, id), state, time };
    });
    const failedPkg = packages.find((p) => p.state === "failed");
    const failure = failedPkg ? { name: failedPkg.name, cells: cells.filter((c) => c.state === "failed").map((c) => c.name) } : null;
    if (failedPkg && steps[1].phase === "done") steps[1].phase = "failed";
    const readyPkgs = packages.filter((p) => p.state === "ready").length;
    const evaluate = runTask?.subtasks?.evaluate?.subtasks ?? {};
    const runTotal = Object.keys(evaluate).length;
    const runDone = Object.values(evaluate).filter((e) => e.finished_at != null).length;
    let busyText = null;
    if (steps[0].phase === "busy") busyText = "Starting Julia";
    else if (pkgPhase === "busy") busyText = packages.length ? `Installing packages \xB7 ${readyPkgs} of ${packages.length}` : "Installing packages";
    else if (steps[2].phase === "busy") busyText = runTotal ? `Running ${runDone} of ${runTotal}` : "Running";
    const restart = nbpkg.restart_required_msg ? "required" : nbpkg.restart_recommended_msg ? "recommended" : null;
    let headline;
    if (nb.process_status === "waiting_for_permission") headline = "Safe preview \xB7 nothing has run";
    else if (failure) headline = `Package failed \xB7 ${failure.name}`;
    else if (busyText?.startsWith("Running")) headline = `Running cells \xB7 ${runDone} of ${runTotal}`;
    else if (busyText) headline = busyText;
    else if (restart === "required") headline = "Restart needed";
    else if (nb.process_status === "no_process" || nb.process_status === "waiting_to_restart") headline = "Julia stopped";
    else headline = "Ready";
    return {
      headline,
      steps,
      packages,
      deps: depsRow,
      cells,
      failure,
      busy: busyText,
      saveFailed: tree.saving?.success === false,
      restart
    };
  }

  // src/state.ts
  var listeners = [];
  var last = null;
  var model = null;
  var lastSent = "";
  var context = { host: "This Mac", asking: false };
  var drawerOf = () => null;
  function onNotebook(listener) {
    listeners.push(listener);
    if (last && model) listener(last, model);
  }
  function current() {
    return last && model ? { nb: last, model } : null;
  }
  var notebookId = () => new URLSearchParams(location.search).get("id") ?? "";
  function setDrawerSource(source) {
    drawerOf = source;
  }
  function report() {
    const editor = window.editor_state;
    if (!last || !model) return;
    const msg = {
      type: "state",
      notebook: notebookId(),
      safe: last.process_status === "waiting_for_permission",
      busy: model.busy,
      restart: model.restart,
      save_failed: model.saveFailed,
      package_failed: model.failure?.name ?? null,
      dead: last.process_status === "no_process",
      connected: editor?.connected !== false,
      drawer: drawerOf()
    };
    const json = JSON.stringify(msg);
    if (json === lastSent) return;
    lastSent = json;
    send(msg);
  }
  function tick() {
    const nb = window.editor_state?.notebook;
    if (!nb?.cell_order || nb === last) return report();
    last = nb;
    model = statusModel(nb);
    for (const listener of listeners) listener(nb, model);
    report();
  }
  var ticks = [];
  function every(ms, hook) {
    let lastRun = 0;
    ticks.push(() => {
      const now = performance.now();
      if (now - lastRun >= ms) {
        lastRun = now;
        hook();
      }
    });
  }
  function frame() {
    ticks.forEach((t) => t());
    requestAnimationFrame(frame);
  }
  function initState() {
    every(250, tick);
    if (document.querySelector("pluto-editor")) requestAnimationFrame(frame);
    on("context", (msg) => {
      context.host = msg.host;
      context.asking = msg.asking;
      if (last && model) listeners.forEach((l) => l(last, model));
    });
    lastSent = "";
  }

  // src/drawer.ts
  var HEADER = 36;
  var css7 = `
  #endeavor-drawer { display: none; }
  html[data-endeavor-look="endeavor"][data-endeavor-drawer] #endeavor-drawer { display: flex; }
  /* Separate from --endeavor-drawer-h (the drawer's own height, which also
     sizes Pluto's Live docs panel): while dragging the grip, only the
     drawer's height changes every frame; the body's padding -- and so the
     whole notebook's layout -- only catches up once the drag ends (see
     setHeight's settle parameter below). */
  html[data-endeavor-look="endeavor"][data-endeavor-drawer] body { padding-bottom: var(--endeavor-drawer-pad); }
  #endeavor-drawer { position: fixed; left: 0; right: 0; bottom: 0; height: var(--endeavor-drawer-h); z-index: 70;
    flex-direction: column; background: var(--e-bg-page); border-top: 1px solid var(--e-border);
    font: 12.5px/1.45 system-ui, -apple-system, sans-serif; color: var(--e-text-secondary); }
  #endeavor-drawer .grip { position: absolute; top: -4px; left: 0; right: 0; height: 8px; cursor: ns-resize; }
  #endeavor-drawer .grip::after { content: ""; position: absolute; left: calc(50% - 14px); top: 1px; width: 28px; height: 3px;
    border-radius: 2px; background: var(--e-control-edge); }
  #endeavor-drawer > header { flex: none; height: ${HEADER}px; display: flex; align-items: center; gap: 4px; padding: 0 12px; }
  #endeavor-drawer > header button { display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 8px; border: none;
    border-radius: 5px; background: none; color: var(--e-text-muted); font: inherit; cursor: pointer; }
  #endeavor-drawer > header button:hover { color: var(--e-text-primary); }
  #endeavor-drawer > header button.active { background: var(--e-bg-raised); color: var(--e-text-primary); }
  #endeavor-drawer > header .grow { flex: 1; }
  #endeavor-drawer > header .close { font-size: 15px; padding: 0 6px; }
  #endeavor-drawer .status { flex: 1; overflow-y: auto; padding: 2px 16px 12px; }
  #endeavor-drawer[data-tab="docs"] .status { display: none; }
  #endeavor-drawer .headline { font-size: 14px; font-weight: 600; color: var(--e-text-primary); margin: 4px 0 8px; }
  #endeavor-drawer .headline.failed { color: var(--e-diff-del); }
  #endeavor-drawer .steps { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  #endeavor-drawer .steps .step { display: flex; align-items: center; gap: 6px; white-space: nowrap; color: var(--e-text-muted); }
  #endeavor-drawer .steps .step.busy { color: var(--e-text-primary); }
  #endeavor-drawer .steps .step.failed { color: var(--e-diff-del); }
  #endeavor-drawer .steps .step.waiting { color: var(--e-text-waiting); }
  #endeavor-drawer .steps .line { flex: 1; height: 1px; background: var(--e-border); min-width: 12px; }
  #endeavor-drawer .host { color: var(--e-text-dim); font-size: 11.5px; margin-bottom: 10px; }
  #endeavor-drawer .group { margin-top: 6px; }
  #endeavor-drawer .group > summary { display: flex; align-items: center; gap: 6px; list-style: none; cursor: pointer;
    color: var(--e-text-secondary); padding: 3px 0; }
  #endeavor-drawer .group > summary::-webkit-details-marker { display: none; }
  #endeavor-drawer .group > summary::before { content: "\u203A"; width: 10px; color: var(--e-text-dim); transition: transform 0.1s; }
  #endeavor-drawer .group[open] > summary::before { transform: rotate(90deg); }
  #endeavor-drawer .group > summary .right { margin-left: auto; color: var(--e-text-dim); font-size: 11.5px; }
  #endeavor-drawer .row { display: flex; align-items: center; gap: 8px; padding: 2px 0 2px 16px; min-height: 22px; }
  #endeavor-drawer .row .name { font: 12px JuliaMono, ui-monospace, monospace; color: var(--e-text-code); }
  #endeavor-drawer .row.waiting .name { color: var(--e-text-dim); }
  #endeavor-drawer .row .note { color: var(--e-text-waiting); font-size: 11px; }
  #endeavor-drawer .row .right { margin-left: auto; color: var(--e-text-muted); font-size: 11.5px; }
  #endeavor-drawer .row .right.time { color: var(--e-text-waiting); font-size: 10.5px; }
  #endeavor-drawer .row.failed .right { color: var(--e-diff-del); }
  #endeavor-drawer .mark { width: 12px; flex: none; display: flex; justify-content: center; font-size: 11px; color: var(--e-text-muted); }
  #endeavor-drawer .mark.failed { color: var(--e-diff-del); }
  #endeavor-drawer .mark.waiting::before { content: ""; width: 4px; height: 4px; border-radius: 2px; background: var(--e-dot-waiting); }
  #endeavor-drawer .mark.busy::before { content: ""; width: 7px; height: 7px; border-radius: 50%; border: 1.5px solid var(--e-accent);
    border-right-color: transparent; animation: endeavor-spin 0.9s linear infinite; }
  @media (prefers-reduced-motion: reduce) { #endeavor-drawer .mark.busy::before { animation: none; } }
  @keyframes endeavor-spin { to { transform: rotate(360deg); } }
  #endeavor-drawer .failure { margin: 8px 0 8px 16px; padding: 10px 12px; border: 1px solid var(--e-danger-edge);
    border-radius: 8px; background: var(--e-danger-bg); color: var(--e-text-code); }
  #endeavor-drawer .failure code { font: 11.5px JuliaMono, ui-monospace, monospace; }
  #endeavor-drawer .failure .actions { display: flex; gap: 6px; margin-top: 8px; }
  #endeavor-drawer .failure button, #endeavor-drawer .showlog { display: flex; align-items: center; gap: 5px; padding: 3px 9px;
    border-radius: 5px; border: 1px solid var(--e-control-edge); background: var(--e-bg-card); color: var(--e-text-code); font: inherit; cursor: pointer; }
  #endeavor-drawer .failure button.fix { color: var(--e-accent-text); border-color: var(--e-accent); }
  #endeavor-drawer .failure button.plain, #endeavor-drawer .showlog { border-color: transparent; background: none; color: var(--e-text-muted); }
  #endeavor-drawer .foot { display: flex; justify-content: flex-end; margin-top: 10px; }
  #endeavor-drawer pre.log { margin: 6px 0 0; padding: 8px 10px; max-height: 260px; overflow: auto; border-radius: 6px;
    background: var(--e-bg-card); color: var(--e-text-secondary); font: 11px/1.45 JuliaMono, ui-monospace, monospace; white-space: pre-wrap; }
  /* Live docs: Pluto's panel, filling the drawer under its tabs. */
  html[data-endeavor-look="endeavor"][data-endeavor-drawer="docs"] #helpbox-wrapper {
    display: block !important; position: fixed; left: 0; right: 0; bottom: 0; top: auto;
    height: calc(var(--endeavor-drawer-h) - ${HEADER}px); z-index: 71; }
  html[data-endeavor-look="endeavor"] pluto-helpbox { position: static; width: 100%; height: 100%; right: auto;
    border-radius: 0; box-shadow: none; background: var(--e-bg-page); }
  html[data-endeavor-look="endeavor"] pluto-helpbox > header { display: none; }
  html[data-endeavor-look="endeavor"] pluto-helpbox .live-docs-searchbox { margin: 2px 16px 8px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox .live-docs-searchbox input { border: 1px solid var(--e-control-edge); border-radius: 6px;
    font-size: 12.5px; padding: 5px 8px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox > section { padding: 0 16px 12px; }
  html[data-endeavor-look="endeavor"] pluto-helpbox > section h1 { font-size: 13px; margin: 4px 0 8px; }
  @media print {
    #endeavor-drawer, #endeavor-safe, #endeavor-rail, #helpbox-wrapper { display: none !important; }
    html[data-endeavor-drawer] body { padding-bottom: 0; }
  }
`;
  var book = `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.1"><path d="M1 2.2h3.3c.9 0 1.7.7 1.7 1.6v6.4c0-.7-.6-1.2-1.3-1.2H1zM11 2.2H7.7c-.9 0-1.7.7-1.7 1.6v6.4c0-.7.6-1.2 1.3-1.2H11z"/></svg>`;
  var pulse = `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.1"><path d="M.5 6.5h2.5l1.5-4 2.5 7 1.5-3h3"/></svg>`;
  var drawer;
  var body;
  var tab = null;
  var auto = false;
  var autoDone = false;
  var showLog = false;
  var folded = /* @__PURE__ */ new Map();
  var escape2 = (s) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
  function height() {
    try {
      const saved = Number(localStorage.getItem("endeavor-drawer-h"));
      if (saved > 80) return saved;
    } catch {
    }
    return Math.round(window.innerHeight * 0.42);
  }
  function setHeight(h, settle = true) {
    const clamped = Math.max(120, Math.min(h, window.innerHeight - 80));
    const root = document.documentElement.style;
    root.setProperty("--endeavor-drawer-h", `${clamped}px`);
    if (settle) root.setProperty("--endeavor-drawer-pad", `${clamped}px`);
    return clamped;
  }
  var echoing = false;
  function openDrawer(next, byItself = false) {
    tab = next;
    auto = byItself;
    if (next) document.documentElement.dataset.endeavorDrawer = next;
    else delete document.documentElement.dataset.endeavorDrawer;
    drawer.dataset.tab = next ?? "";
    for (const b of drawer.querySelectorAll("header [data-tab]")) b.classList.toggle("active", b.dataset.tab === next);
    echoing = true;
    window.dispatchEvent(new CustomEvent("open_bottom_right_panel", { detail: next === "docs" ? "docs" : null }));
    echoing = false;
    render();
    report();
  }
  function pick(next) {
    openDrawer(next);
    if (next === "docs") setTimeout(() => document.querySelector("#live-docs-search")?.focus(), 50);
  }
  var markFor = (state) => state === "done" || state === "ready" ? `<span class="mark">\u2713</span>` : state === "failed" ? `<span class="mark failed">\u2715</span>` : state === "waiting" ? `<span class="mark waiting"></span>` : `<span class="mark busy"></span>`;
  function group(name, busy, summary, right, rows) {
    const open2 = folded.get(name) ?? busy;
    return `<details class="group" data-group="${name}"${open2 ? " open" : ""}><summary>${summary}<span class="right">${right}</span></summary>${rows}</details>`;
  }
  function statusHtml(nb, m) {
    const steps = m.steps.map((s, i) => `${i ? `<span class="line"></span>` : ""}<span class="step ${s.phase}">${markFor(s.phase)}${s.name}</span>`).join("");
    const version = String(nb.julia_version ?? "").replace(/^v/, "");
    const julia = version ? `Julia ${escape2(version)} \xB7 ` : "";
    const host = `<div class="host">${julia}${escape2(hostName)}</div>`;
    const pkgBusy = m.steps[1].phase === "busy";
    const readyCount = m.packages.filter((p) => p.state === "ready").length;
    const stateText = { waiting: "waiting", installing: "installing", precompiling: "precompiling", ready: "", failed: "failed" };
    const pkgRows = m.packages.map((p) => {
      const note = p.detail === "standard library" ? `<span class="note">standard library</span>` : "";
      const right = p.state === "ready" ? p.detail === "standard library" ? "built in" : escape2(p.detail) : stateText[p.state];
      return `<div class="row ${p.state}">${markFor(p.state)}<span class="name">${escape2(p.name)}</span>${note}<span class="right">${right}</span></div>`;
    }).join("");
    let deps = "";
    if (m.deps) {
      const d = m.deps;
      const right = d.failed ? `${d.failed} failed` : pkgBusy ? `${d.precompiled} precompiled` : "ready";
      deps = `<div class="row">${markFor(d.failed ? "failed" : pkgBusy ? "busy" : "done")}<span>and ${d.count} dependenc${d.count === 1 ? "y" : "ies"}</span><span class="right">${right}</span></div>`;
    }
    let failure = "";
    if (m.failure) {
      const f = m.failure;
      const blocked = f.cells.length ? ` Cells that use it can't run: <code>${f.cells.map(escape2).join("</code>, <code>")}</code>.` : "";
      failure = `<div class="failure"><code>${escape2(f.name)}</code> ${m.packages.find((p) => p.name === f.name)?.detail === "not found" ? "isn't a package Pkg can find: a typo, or not in the registry." : "couldn't be installed or precompiled."}${blocked}<div class="actions"><button class="fix">\u2726 Fix with Claude</button><button class="restart">\u21BB Restart notebook</button><button class="plain pkglog">Log for ${escape2(f.name)}</button></div></div>`;
    }
    const packages = m.packages.length ? group("packages", pkgBusy || !!m.failure, "Packages", m.packages.length ? `${readyCount} of ${m.packages.length} ready` : "", pkgRows + failure + deps) : failure;
    const running = m.steps[2].phase === "busy";
    const done = m.cells.filter((c) => c.state === "done").length;
    const cellRows = m.cells.map((c) => {
      const right = c.state === "running" ? "running" : c.time ? c.time : c.state === "waiting" ? "waiting" : "";
      return `<div class="row ${c.state}">${markFor(c.state)}<span class="name">${escape2(c.name || "(empty)")}</span><span class="right${c.time ? " time" : ""}">${right}</span></div>`;
    }).join("");
    const cells = group("cells", running, "Cells", `${done} of ${m.cells.length} \xB7 notebook order`, cellRows);
    const log = (nb.nbpkg?.terminal_outputs?.nbpkg_sync ?? "").replace(/\x1b\[[0-9;]*m/g, "");
    const logBlock = showLog ? `<pre class="log">${escape2(log || "Nothing from Pkg yet.")}</pre>` : "";
    const foot = `<div class="foot"><button class="showlog">\u2261 ${showLog ? "Hide log" : "Show log"}</button></div>${logBlock}`;
    return `<div class="headline${m.failure ? " failed" : ""}">${escape2(m.headline)}</div><div class="steps">${steps}</div>${host}${packages}${cells}${foot}`;
  }
  var hostName = "This Mac";
  var lastHtml = "";
  function render() {
    const now = current();
    if (!now || tab !== "status") return;
    const html = statusHtml(now.nb, now.model);
    if (html === lastHtml) return;
    lastHtml = html;
    body.innerHTML = html;
    for (const d of body.querySelectorAll("details.group")) {
      d.addEventListener("toggle", () => folded.set(d.dataset.group, d.open));
    }
    const failure = now.model.failure;
    body.querySelector(".showlog").onclick = () => {
      showLog = !showLog;
      render();
    };
    if (failure) {
      const pkgLog = () => now.nb.nbpkg?.terminal_outputs?.[failure.name] ?? now.nb.nbpkg?.terminal_outputs?.nbpkg_sync ?? "";
      body.querySelector(".failure .fix").onclick = (e) => byUser(e) && send({ type: "fix_package", notebook: notebookId(), name: failure.name, log: pkgLog().replace(/\x1b\[[0-9;]*m/g, "").slice(-6e3) });
      body.querySelector(".failure .restart").onclick = (e) => byUser(e) && send({ type: "restart", notebook: notebookId() });
      body.querySelector(".failure .pkglog").onclick = () => {
        showLog = true;
        render();
        body.querySelector("pre.log")?.scrollIntoView({ block: "nearest" });
      };
    }
  }
  function follow(_, m) {
    const installing = m.steps[1].phase === "busy" && m.packages.some((p) => p.state === "installing" || p.state === "precompiling");
    if (installing && !autoDone && tab === null && document.documentElement.dataset.endeavorLook === "endeavor") {
      autoDone = true;
      openDrawer("status", true);
    } else if (auto && tab === "status" && !m.busy && !m.failure && m.steps[2].phase !== "waiting") {
      openDrawer(null);
    }
    render();
  }
  function initDrawer() {
    const style2 = document.createElement("style");
    style2.textContent = css7;
    document.head.append(style2);
    setHeight(height());
    drawer = document.createElement("div");
    drawer.id = "endeavor-drawer";
    drawer.dataset.endeavorUi = "";
    drawer.innerHTML = `<div class="grip"></div><header><button data-tab="docs">${book}Live docs</button><button data-tab="status">${pulse}Status</button><span class="grow"></span><button class="close" title="Close">\xD7</button></header><section class="status"></section>`;
    body = drawer.querySelector(".status");
    document.body.append(drawer);
    for (const b of drawer.querySelectorAll("header [data-tab]")) b.onclick = () => pick(b.dataset.tab);
    drawer.querySelector(".close").onclick = () => openDrawer(null);
    const grip = drawer.querySelector(".grip");
    grip.onpointerdown = (e) => {
      grip.setPointerCapture(e.pointerId);
      let pending = null;
      let frame2 = 0;
      const move = (m) => {
        pending = window.innerHeight - m.clientY;
        if (frame2) return;
        frame2 = requestAnimationFrame(() => {
          frame2 = 0;
          if (pending !== null) setHeight(pending, false);
        });
      };
      const up = () => {
        grip.removeEventListener("pointermove", move);
        if (frame2) cancelAnimationFrame(frame2);
        const h = setHeight(pending ?? drawer.offsetHeight);
        try {
          localStorage.setItem("endeavor-drawer-h", String(h));
        } catch {
        }
      };
      grip.addEventListener("pointermove", move);
      grip.addEventListener("pointerup", up, { once: true });
    };
    window.addEventListener("open_bottom_right_panel", (e) => {
      if (!echoing && e.detail === "docs" && tab !== "docs") openDrawer("docs");
    });
    setDrawerSource(() => tab);
    on("drawer", (msg) => pick(msg.tab));
    on("context", (msg) => {
      hostName = msg.host;
      lastHtml = "";
      render();
    });
    onNotebook(follow);
    every(1e3, () => current()?.model.busy && render());
  }

  // src/prompt.ts
  var AGENT = "Claude";
  var css8 = `
  /* Beside Pluto's "+" in the gap above a cell (and below the last one): faint
     while the cell is hovered, like Pluto's own buttons, and full on the "+". */
  pluto-cell > .endeavor-add-agent {
    position: absolute; left: 14px; z-index: 20;
    height: 18px; padding: 0 7px; border-radius: 9px; border: 1px solid var(--e-pill-edge);
    background: var(--e-pill-bg); color: var(--e-text-tag); font: 11px system-ui, sans-serif; cursor: pointer;
    opacity: 0; transition: opacity 0.1s;
  }
  pluto-cell > .endeavor-add-agent.before { top: calc(-0.5 * var(--pluto-cell-spacing, 17px) - 9px); }
  pluto-cell > .endeavor-add-agent.after { bottom: calc(-0.5 * var(--pluto-cell-spacing, 17px) - 9px); }
  pluto-cell:hover > .endeavor-add-agent { opacity: 0.35; }
  pluto-cell > button.add_cell:hover + .endeavor-add-agent, pluto-cell > .endeavor-add-agent:hover { opacity: 1; }
  pluto-cell > .endeavor-add-agent:hover { color: var(--e-prompt-hover); border-color: var(--e-accent); }
  #endeavor-prompt {
    position: absolute; z-index: 1000; display: flex; flex-direction: column; gap: 6px;
    padding: 8px 10px; border-radius: 8px; border: 1px solid var(--e-accent); background: var(--e-dialog-bg);
    box-shadow: 0 6px 24px var(--e-shadow-popover); font: 13px system-ui, sans-serif; color: var(--e-text-primary);
  }
  #endeavor-prompt textarea {
    resize: none; border: none; outline: none; background: transparent; color: inherit;
    font: inherit; min-height: 20px;
  }
  #endeavor-prompt .hint { color: var(--e-text-dim); font-size: 11px; }
  #endeavor-prompt .quote { color: var(--e-text-tag); font: 12px ui-monospace, monospace; white-space: pre-wrap;
    border-left: 2px solid var(--e-accent); padding-left: 8px; max-height: 5.5em; overflow: hidden; }
  #endeavor-ask-selection {
    position: absolute; z-index: 1000; height: 22px; padding: 0 9px; border-radius: 11px;
    border: 1px solid var(--e-accent); background: var(--e-pill-bg); color: var(--e-prompt-hover); font: 12px system-ui, sans-serif; cursor: pointer;
  }
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
  function place(box, cell, where, selection) {
    const rect = selection?.rect ?? cell.getBoundingClientRect();
    box.style.left = `${rect.left + window.scrollX}px`;
    const top = where === "before" ? rect.top + window.scrollY - 6 - 70 : rect.bottom + window.scrollY + 6;
    box.style.top = `${Math.max(top, 0)}px`;
    box.style.width = `${Math.min(Math.max(rect.width, 320), 640)}px`;
  }
  function isEmpty(cell) {
    return !(cell.querySelector("pluto-input .cm-content")?.textContent ?? "").trim();
  }
  function openPrompt(cell, where, selection) {
    close(false);
    const box = document.createElement("div");
    box.id = "endeavor-prompt";
    box.dataset.endeavorUi = "";
    const asking = selection ? `Ask ${AGENT} about the selection` : where !== "cell" ? `Ask ${AGENT} to write a cell here` : isEmpty(cell) ? `Ask ${AGENT} what to write here` : `Ask ${AGENT} about this cell`;
    box.innerHTML = `<div class="quote" hidden></div><textarea rows="1" spellcheck="false" autocorrect="off" autocapitalize="off"></textarea><div class="hint">\u21B5 send \xB7 esc cancel</div>`;
    if (selection) {
      const quote = box.querySelector(".quote");
      quote.hidden = false;
      quote.textContent = selection.quote.length > 160 ? selection.quote.slice(0, 160) + "\u2026" : selection.quote;
    }
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
          const kind = where !== "cell" ? where : isEmpty(cell) ? "fill" : "about";
          send({ type: "prompt", notebook, cell: cell.id, code: cellCode(cell), where: kind, text: comment, now: e.metaKey, quote: selection?.quote });
          close(true);
        }
      },
      true
    );
    document.body.append(box);
    place(box, cell, where, selection);
    open = { box, cell, where, selection };
    requestAnimationFrame(() => text.focus());
  }
  function initPrompt() {
    const style2 = document.createElement("style");
    style2.textContent = css8;
    document.head.append(style2);
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
    window.addEventListener("resize", () => open && place(open.box, open.cell, open.where, open.selection));
    initSelectionChip();
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
  function selectedInCell() {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || !selection.rangeCount) return null;
    const range = selection.getRangeAt(0);
    const node = range.commonAncestorContainer;
    const element = node instanceof Element ? node : node.parentElement;
    const cell = element?.closest("pluto-cell");
    if (!cell || element?.closest("[data-endeavor-ui]")) return null;
    const view = element?.closest(".cm-content")?.cmTile?.root?.view;
    const main = view?.state.selection.main;
    const quote = (main && !main.empty ? view.state.sliceDoc(main.from, main.to) : selection.toString()).trim();
    return quote ? { cell, quote, rect: range.getBoundingClientRect() } : null;
  }
  function initSelectionChip() {
    let chip = null;
    const hide = () => {
      chip?.remove();
      chip = null;
    };
    document.addEventListener("mouseup", (e) => {
      if (chip?.contains(e.target) || document.body.classList.contains("annotating")) return;
      setTimeout(() => {
        hide();
        const found = selectedInCell();
        if (!found) return;
        chip = document.createElement("button");
        chip.id = "endeavor-ask-selection";
        chip.dataset.endeavorUi = "";
        chip.textContent = `\u2726 Ask ${AGENT}`;
        chip.style.left = `${found.rect.left + window.scrollX}px`;
        chip.style.top = `${found.rect.bottom + window.scrollY + 6}px`;
        chip.onmousedown = (event) => event.preventDefault();
        chip.onclick = (event) => {
          if (!byUser(event)) return;
          hide();
          openPrompt(found.cell, "cell", { rect: found.rect, quote: found.quote });
        };
        document.body.append(chip);
      });
    });
    document.addEventListener("keydown", hide, true);
  }

  // src/readonly.ts
  var CLASS = "endeavor-readonly";
  var readonly = false;
  function holdReconnects() {
    const Real = window.WebSocket;
    const Held = function(url, protocols) {
      return new Real(readonly ? "ws://127.0.0.1:9/" : url, protocols);
    };
    Held.prototype = Real.prototype;
    Object.assign(Held, { CONNECTING: Real.CONNECTING, OPEN: Real.OPEN, CLOSING: Real.CLOSING, CLOSED: Real.CLOSED });
    window.WebSocket = Held;
  }
  function style() {
    const css11 = document.createElement("style");
    css11.textContent = `
    body.${CLASS} pluto-notebook { opacity: 0.85; }
    body.${CLASS} pluto-notebook, body.${CLASS} pluto-notebook * { pointer-events: none !important; }
  `;
    document.head.append(css11);
  }
  function blockKeys(e) {
    if (!readonly) return;
    const inNotebook = e.target instanceof Element && e.target.closest("pluto-notebook");
    const copying = (e.metaKey || e.ctrlKey) && (e.key === "c" || e.key === "a");
    if (inNotebook && !copying) {
      e.preventDefault();
      e.stopImmediatePropagation();
    }
  }
  function setReadonly(on2) {
    readonly = on2;
    document.body?.classList.toggle(CLASS, on2);
    if (on2 && document.activeElement instanceof HTMLElement && document.activeElement.closest("pluto-notebook")) {
      document.activeElement.blur();
    }
  }
  holdReconnects();
  function initReadonly() {
    style();
    window.addEventListener("keydown", blockKeys, true);
    on("context", (msg) => setReadonly(msg.readonly));
  }

  // src/errors.ts
  var AGENT2 = "Claude";
  var css9 = `
  .endeavor-ask { display: flex; gap: 8px; margin: 8px 0; }
  .endeavor-ask button { font: 12px system-ui; padding: 3px 10px; border-radius: 4px; cursor: pointer;
    background: transparent; color: var(--e-accent-text); border: 1px solid var(--e-accent); }
  .endeavor-ask button.explain { color: var(--e-text-secondary); border-color: var(--e-control-edge); }
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
        send({ type: "ask", kind, notebook, cell: cell.id, code: cellCode(cell), error: text });
      };
      row.querySelector(".fix").onclick = (e) => byUser(e) && ask("fix");
      row.querySelector(".explain").onclick = (e) => byUser(e) && ask("explain");
      error.append(row);
    }
  }
  function initErrors() {
    const style2 = document.createElement("style");
    style2.textContent = css9;
    document.head.append(style2);
    onRedraw(decorate2);
  }

  // src/safe.ts
  var css10 = `
  #endeavor-safe { display: none; }
  html[data-endeavor-look="endeavor"] #endeavor-safe.shown { display: flex; }
  #endeavor-safe { gap: 10px; align-items: flex-start; margin: 0 0 20px 0; padding: 12px 14px;
    border: 1px solid var(--e-border); border-radius: 8px; background: var(--e-bg-card);
    font: 13px/1.5 system-ui, -apple-system, sans-serif; color: var(--e-text-secondary); }
  #endeavor-safe svg { flex: none; margin-top: 3px; color: var(--e-accent-text); }
  #endeavor-safe .text { flex: 1; }
  #endeavor-safe b { display: block; font-weight: 600; color: var(--e-text-primary); font-size: 13.5px; }
  #endeavor-safe .asking { color: var(--e-accent-text); margin-top: 4px; }
  #endeavor-safe .asking::before { content: ""; display: inline-block; width: 6px; height: 6px; border-radius: 3px;
    background: var(--e-accent); margin: 0 7px 1px 0; }
  #endeavor-safe button { flex: none; display: flex; align-items: center; gap: 6px; padding: 4px 10px; border-radius: 5px;
    border: 1px solid var(--e-control-edge); background: var(--e-bg-raised); color: var(--e-text-primary); font: 12.5px system-ui, sans-serif; cursor: pointer; }
  #endeavor-safe button:hover { background: var(--e-button-hover); }
  #endeavor-safe button svg { margin: 0; }
`;
  var shield = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M7 1.2 12 3v3.6c0 3-2.2 5.2-5 6.2-2.8-1-5-3.2-5-6.2V3z"/></svg>`;
  var play = `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" style="stroke: var(--e-accent-text)" stroke-width="1.3"><path d="M2.5 1.5v7l6-3.5z"/></svg>`;
  var callout;
  function render2() {
    const now = current();
    const safe = now?.nb.process_status === "waiting_for_permission";
    callout.classList.toggle("shown", !!safe);
    if (!safe) return;
    const asking = context.asking ? `<div class="asking">Claude is asking to run it. Answer in the chat, or here.</div>` : "";
    const html = `${shield}<div class="text"><b>Safe preview</b>You're reading and editing this file without running any code.${asking}</div><button class="run">${play}Run notebook</button>`;
    if (callout.innerHTML !== html) {
      callout.innerHTML = html;
      callout.querySelector(".run").onclick = (e) => byUser(e) && send({ type: "run_notebook", notebook: notebookId() });
    }
  }
  function place2() {
    const notebook = document.querySelector("main pluto-notebook");
    if (notebook && callout.nextElementSibling !== notebook) notebook.before(callout);
  }
  function initSafe() {
    const style2 = document.createElement("style");
    style2.textContent = css10;
    document.head.append(style2);
    callout = document.createElement("div");
    callout.id = "endeavor-safe";
    callout.dataset.endeavorUi = "";
    onNotebook(render2);
    onRedraw(place2);
  }

  // src/theme.ts
  var tokens = `
:root {
  --e-bg-sidebar: #111113;
  --e-bg-page: #151517;
  --e-bg-card: #1C1C1F;
  --e-bg-raised: #26262A;
  --e-bg-sunken: #18181A;
  --e-bg-tag: #222225;
  --e-text-tag: #9A9A9A;
  --e-border: #2A2A2E;
  --e-control-edge: #3A3A40;
  --e-divider: #1F1F22;
  --e-text-primary: #ECECEC;
  --e-text-secondary: #BDBDBD;
  --e-text-muted: #8C8C8C;
  --e-text-faint: #858585;
  --e-text-section: #888888;
  --e-accent: #CC3F00;
  --e-accent-text: #E08A5E;
  --e-focus-ring: #E08A5E;
  --e-diff-add: #6CC784;
  --e-diff-del: #E07A7A;
  --e-diff-add-tint: rgba(108, 199, 132, 0.12);
  --e-diff-del-tint: rgba(224, 122, 122, 0.12);
  --e-popover-bg: #26262A;
  --e-popover-edge: #3A3A40;
  --e-menu-hover: #313136;
  --e-shadow-popover: rgba(0, 0, 0, 0.45);
  --e-button-hover: #2E2E33;
  --e-dialog-bg: #1C1C1F;
  --e-dialog-edge: #2A2A2E;
  --e-dialog-shadow: rgba(0, 0, 0, 0.5);

  /* Colours used by one or a few modules that don't match one of the tokens
     above closely enough to reuse it without shifting the dark look. */
  --e-text-dim: #7A7A7A;
  --e-text-waiting: #5E5E5E;
  --e-text-code: #D4D4D4;
  --e-dot-waiting: #4A4A4E;
  --e-danger-edge: rgba(224, 122, 122, 0.4);
  --e-danger-bg: rgba(224, 122, 122, 0.06);
  --e-diff-add-ch: rgba(108, 199, 132, 0.28);
  --e-diff-del-ch: rgba(224, 122, 122, 0.28);
  --e-you-stripe: #9A9A9A;
  --e-you-stripe-tint: rgba(154, 154, 154, 0.25);
  --e-stripe-tint: rgba(204, 63, 0, 0.3);
  --e-hover-edge: #FF7A40;
  --e-dim: rgba(0, 0, 0, 0.25);
  --e-hint-bg: rgba(28, 28, 30, 0.85);
  --e-hint-text: #ccc;
  --e-bar-bg: rgba(28, 28, 30, 0.72);
  --e-bar-shadow: rgba(0, 0, 0, 0.4);
  --e-field-bg: rgba(0, 0, 0, 0.35);
  --e-field-edge: #555555;
  --e-annotate-btn-bg: #3a3a3c;
  --e-pill-bg: #1C1C1E;
  --e-pill-edge: #333333;
  --e-prompt-hover: #FF9A6B;
  --e-backdrop: rgba(0, 0, 0, 0.45);
}
@media (prefers-color-scheme: light) {
  :root {
    --e-bg-sidebar: #F3F3F5;
    --e-bg-page: #FCFCFD;
    --e-bg-card: #F4F4F6;
    --e-bg-raised: #EAEAED;
    --e-bg-sunken: #F8F8FA;
    --e-bg-tag: #ECECEF;
    --e-text-tag: #5C5C64;
    --e-border: #E1E1E6;
    --e-control-edge: #8A8A92;
    --e-divider: #E6E6EA;
    --e-text-primary: #1B1B1F;
    --e-text-secondary: #45454C;
    --e-text-muted: #5C5C64;
    --e-text-faint: #66666E;
    --e-text-section: #6B6B73;
    --e-accent: #CC3F00;
    --e-accent-text: #B23600;
    --e-focus-ring: #CC3F00;
    --e-diff-add: #1C7038;
    --e-diff-del: #B42A36;
    --e-diff-add-tint: rgba(28, 140, 70, 0.12);
    --e-diff-del-tint: rgba(200, 40, 60, 0.1);
    --e-popover-bg: #FFFFFF;
    --e-popover-edge: #D9D9DF;
    --e-menu-hover: #EEEEF1;
    --e-shadow-popover: rgba(20, 20, 30, 0.14);
    --e-button-hover: #E2E2E6;
    --e-dialog-bg: #FFFFFF;
    --e-dialog-edge: #D9D9DF;
    --e-dialog-shadow: rgba(20, 20, 30, 0.16);

    --e-text-dim: #66666E;
    --e-text-waiting: #707078;
    --e-text-code: #26262B;
    --e-dot-waiting: #A6A6AE;
    --e-danger-edge: rgba(180, 42, 54, 0.35);
    --e-danger-bg: rgba(180, 42, 54, 0.05);
    --e-diff-add-ch: rgba(28, 140, 70, 0.24);
    --e-diff-del-ch: rgba(200, 40, 60, 0.2);
    --e-you-stripe: #8A8A92;
    --e-you-stripe-tint: rgba(138, 138, 146, 0.3);
    --e-stripe-tint: rgba(204, 63, 0, 0.25);
    --e-hover-edge: #CC3F00;
    --e-dim: rgba(20, 20, 30, 0.1);
    --e-hint-bg: rgba(255, 255, 255, 0.94);
    --e-hint-text: #45454C;
    --e-bar-bg: rgba(255, 255, 255, 0.94);
    --e-bar-shadow: rgba(20, 20, 30, 0.14);
    --e-field-bg: rgba(20, 20, 30, 0.06);
    --e-field-edge: #8A8A92;
    --e-annotate-btn-bg: #EAEAED;
    --e-pill-bg: #F4F4F6;
    --e-pill-edge: #D2D2D8;
    --e-prompt-hover: #B23600;
    --e-backdrop: rgba(20, 20, 30, 0.35);
  }
}
`;
  var endeavorLook = `
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
    --cm-color-comment: #858585;
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
    /* Frontmatter and Pluto's other dialogs */
    --export-bg-color: #1C1C1F;
    --export-color: #D4D4D4;
    --frontmatter-button-bg-color: #26262A;
    --frontmatter-input-bg-color: #151517;
    --frontmatter-input-border-color: #3A3A40;
    /* A cell's run time: faint text, no chip */
    --pluto-runarea-bg-color: transparent;
    --pluto-runarea-span-color: #858585;
  }
  .pluto-modal { border: 1px solid #2A2A2E; border-radius: 8px !important; box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5) !important; }
  .pluto-modal-dark h1 { color: #ECECEC; font-weight: 600; }
  body.presentation nav#slide_controls { gap: 4px; padding: 4px; margin: 12px; border-radius: 8px;
    background: #1C1C1F; border: 1px solid #2A2A2E; }
  nav#slide_controls > button { border-radius: 5px; opacity: 0.8; }
  nav#slide_controls > button:hover { background: #26262A; opacity: 1; }
}
@media (prefers-color-scheme: light) {
  :root {
    --main-bg-color: #FCFCFD;
    --header-bg-color: #FCFCFD;
    --footer-bg-color: #FCFCFD;
    --rule-color: rgba(20, 20, 30, 0.08);
    --code-background: #F4F4F6;
    --normal-cell-color: rgba(120, 120, 130, 0.2);
    --dark-normal-cell-color: rgba(120, 120, 130, 0.34);
    --code-differs-cell-color: #8A8A92;
    --selected-cell-color: rgba(64, 112, 196, 0.22);
    --pluto-output-color: #45454C;
    --pluto-output-h-color: #1B1B1F;
    --pluto-output-bg-color: #FCFCFD;
    --pluto-runarea-bg-color: #F4F4F6;
    --pluto-logs-bg-color: #F4F4F6;
    --overlay-button-bg: #F4F4F6;
    --input-context-menu-bg-color: #FFFFFF;
    --input-context-menu-border-color: #D9D9DF;
    --cm-selection-background: rgba(64, 112, 196, 0.18);
    --cm-color-editor-text: #26262B;
    --cm-color-keyword: #A2366F;
    --cm-color-control-operator: #A2366F;
    --cm-color-literal: #955A12;
    --cm-color-symbol: #955A12;
    --cm-color-string: #3F7A22;
    --cm-color-function: #2F5FA6;
    --cm-color-builtin: #2F5FA6;
    --cm-color-comment: #6B6B73;
    --cm-color-line-numbers: #6E6E76;
    /* Live docs */
    --helpbox-bg-color: #F4F4F6;
    --helpbox-header-bg-color: #EAEAED;
    --helpbox-header-tab-bg-color: #EAEAED;
    --helpbox-header-color: #1B1B1F;
    --helpbox-text-color: #26262B;
    --helpbox-search-bg-color: #FCFCFD;
    --helpbox-search-border-color: #8A8A92;
    --helpbox-box-shadow-color: rgba(20, 20, 30, 0.12);
    --docs-binding-bg: #EAEAED;
    /* Frontmatter and Pluto's other dialogs */
    --export-bg-color: #FFFFFF;
    --export-color: #26262B;
    --frontmatter-button-bg-color: #EAEAED;
    --frontmatter-input-bg-color: #FCFCFD;
    --frontmatter-input-border-color: #8A8A92;
    /* A cell's run time: faint text, no chip */
    --pluto-runarea-bg-color: transparent;
    --pluto-runarea-span-color: #6B6B73;
  }
  .pluto-modal { border: 1px solid #D9D9DF; border-radius: 8px !important; box-shadow: 0 12px 40px rgba(20, 20, 30, 0.16) !important; }
  .pluto-modal h1 { color: #1B1B1F; font-weight: 600; }
  body.presentation nav#slide_controls { gap: 4px; padding: 4px; margin: 12px; border-radius: 8px;
    background: #FFFFFF; border: 1px solid #D9D9DF; }
  nav#slide_controls > button { border-radius: 5px; opacity: 0.8; }
  nav#slide_controls > button:hover { background: #EAEAED; opacity: 1; }
}
header#pluto-nav, footer { display: none !important; }
/* Not display: none \u2014 Pluto alerts "window too small to show docs" whenever it opens a panel it finds undisplayed. */
html:not([data-endeavor-drawer="docs"]) #helpbox-wrapper { visibility: hidden !important; pointer-events: none !important;
  position: fixed !important; width: 0 !important; height: 0 !important; overflow: hidden !important; }
.outline-frame.safe-preview, .outline-frame-actions-container.safe-preview { display: none !important; }
pluto-output.rich_output:has(> .safe-preview-output) { display: none !important; }
pluto-editor > main { padding-top: 16px; }
/* Left-aligned, not centred (Pluto classic stays centred): a floated
   PlutoUI TableOfContents sits at the right, and centring put it over the
   notebook rather than beside it. 731px keeps the same reading width Pluto
   centred at; 48px on the left clears our striped edit-gutter bar
   (cells.ts, 8px + 4px past the cell's own left edge) with room to spare.
   Pluto's own rule sizes main at width: 100%, so the 48px margin has to come
   out of that width too (a plain margin-left would push it 48px past the
   pane's right edge instead); the 16px on the right is a small gap, the same
   floor as the chat column's own narrow-width margin (main.rs, CHAT_MIN).
   The overview rail (rail.ts) is fixed to the right of the viewport, not the
   notebook, so it isn't affected either way. */
pluto-editor main { margin-left: 48px !important; margin-right: auto !important; width: calc(100% - 48px - 16px) !important; max-width: 731px !important; }
pluto-runarea > span { font-size: 10px; }
/* The web view draws over native views, so the notebook header can't blur
   what's under it (docs/design-gaps.md, "Translucent headers with blur").
   Instead, the top of the page fades into the page background, so a cell
   scrolled toward the top softens instead of cutting off hard. Fixed to the
   viewport, not the scroller, and never over the first cell at rest: the
   16px top padding above leaves exactly this fade's height clear.
   pointer-events: none keeps clicks and selection reaching the page under it. */
body::before {
  content: "";
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  height: 16px;
  background: linear-gradient(to bottom, var(--main-bg-color), transparent);
  pointer-events: none;
  z-index: 50;
}
`;
  var classic = `
nav#at_the_top > pluto-filepicker, nav#at_the_top > div.desktop_picker_group { display: none !important; }
`;
  var both = `
footer form#feedback { display: none !important; }
`;
  var juliaMono = [
    { file: "Regular", weight: 400, style: "normal" },
    { file: "Bold", weight: 700, style: "normal" },
    { file: "RegularItalic", weight: 400, style: "italic" }
  ].map(
    ({ file, weight, style: style2 }) => `
@font-face {
  font-family: JuliaMono;
  src: url("endeavor://localhost/fonts/JuliaMono-${file}.ttf") format("truetype");
  font-display: swap;
  font-weight: ${weight};
  font-style: ${style2};
}`
  ).join("");
  function initTheme() {
    const tokenSheet = document.createElement("style");
    tokenSheet.id = "endeavor-tokens";
    tokenSheet.textContent = tokens;
    document.head.append(tokenSheet);
    const fonts = document.createElement("style");
    fonts.id = "endeavor-fonts";
    fonts.textContent = juliaMono;
    document.head.append(fonts);
    const style2 = document.createElement("style");
    style2.id = "endeavor-theme";
    document.head.append(style2);
    on("theme", (msg) => {
      style2.textContent = both + (msg.name === "endeavor" ? endeavorLook : classic);
      document.documentElement.dataset.endeavorLook = msg.name;
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
    initReveal();
    initActions();
    initDrawer();
    initSafe();
    initState();
    initReadonly();
    initDebug();
    watchRedraws();
    send({ type: "ready" });
  }
  document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
})();
