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

  // src/keys.ts
  var mac = /Mac/.test(navigator.platform);
  function modHeld(e) {
    return mac ? e.metaKey : e.ctrlKey;
  }
  function shortcut(key) {
    return mac ? `\u2318${key}` : `Ctrl+${key}`;
  }

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

  // src/place.ts
  function pillPlace(lines, viewportHeight) {
    const PILL = 30;
    const last2 = lines[lines.length - 1];
    const below = last2.bottom + 6;
    const top = below + PILL <= viewportHeight ? below : lines[0].top - 6 - PILL;
    return { left: Math.max(last2.right - 28, 4), top };
  }
  function barPlace(pick2, width, height2, barHeight) {
    const GAP = 8;
    const EDGE = 16;
    const barWidth = Math.max(Math.min(380, width - 32), 0);
    const left = Math.min(Math.max(pick2.left, EDGE), width - EDGE - barWidth);
    const fitsBelow = pick2.bottom + GAP + barHeight <= height2 - GAP;
    const fitsAbove = pick2.top - GAP - barHeight >= GAP;
    const lowest = height2 - EDGE - barHeight;
    let top;
    if (pick2.bottom < 0 || fitsBelow) top = pick2.bottom + GAP;
    else if (pick2.top > height2 || fitsAbove) top = pick2.top - GAP - barHeight;
    else top = lowest;
    return { left: Math.max(left, EDGE), top: Math.min(Math.max(top, GAP), lowest), width: barWidth };
  }

  // src/quote.ts
  var SHOOTING = "endeavor-shooting";
  var waiting = /* @__PURE__ */ new Map();
  var nextShot = 1;
  var nextFrame = () => new Promise((done) => requestAnimationFrame(() => done()));
  async function shoot(b) {
    const top = b.top - window.scrollY;
    const bottom = b.bottom - window.scrollY;
    if (top < 0 || bottom > window.innerHeight) window.scrollBy(0, top < 0 || bottom - top > window.innerHeight ? top - 8 : bottom - window.innerHeight + 8);
    document.body.classList.add(SHOOTING);
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
    const id = nextShot++;
    await new Promise((done) => {
      waiting.set(id, done);
      setTimeout(done, 3e3);
      send({ type: "shoot", id, rect });
    });
    waiting.delete(id);
    document.body.classList.remove(SHOOTING);
    return id;
  }
  function cellName(code) {
    const line = code.split("\n").find((l) => l.trim()) ?? "";
    if (!line.includes("=")) return "cell";
    const lhs = line.split("=")[0].trim().replace(/^function /, "").replace(/^const /, "");
    return lhs.match(/^[\p{L}\p{N}_!]+/u)?.[0] ?? "cell";
  }
  function pickSource(pick2) {
    if (pick2.part === "box") return `Box \xB7 ${pick2.cells.length} cell${pick2.cells.length === 1 ? "" : "s"}`;
    const part = pick2.part === "lines" ? pick2.lines[0] === pick2.lines[1] ? `line ${pick2.lines[0]}` : `lines ${pick2.lines[0]}\u2013${pick2.lines[1]}` : pick2.part;
    return `${cellName(pick2.code)} \xB7 ${part}`;
  }
  function sendQuote(picks, comment, add) {
    const notebook = new URLSearchParams(location.search).get("id");
    send({ type: "quote", notebook, picks, comment, add });
  }
  var css2 = `
  .endeavor-field { position: relative; display: flex; align-items: flex-end; gap: 6px; min-height: 34px; box-sizing: border-box;
    padding: 4px 5px 4px 9px; border-radius: 6px; border: 1px solid var(--e-control-edge); background: var(--e-bg-page); }
  .endeavor-field:focus-within { border-color: var(--e-accent-text); }
  .endeavor-field textarea { flex: 1; min-width: 0; resize: none; border: 0; outline: none; background: transparent; padding: 1px 0;
    height: 21px; max-height: 120px; color: var(--e-text-primary); font: 14px/21px system-ui, sans-serif; }
  .endeavor-field .options { flex: none; width: 24px; height: 24px; border: 0; border-radius: 5px; cursor: pointer;
    display: flex; align-items: center; justify-content: center; background: var(--e-bg-raised); color: var(--e-text-primary); font: 13px system-ui; }
  .endeavor-field .options:hover, .endeavor-field .options[aria-expanded="true"] { background: var(--e-menu-hover); }
  .endeavor-field [role="menu"] { position: absolute; right: -6px; top: calc(100% + 6px); z-index: 5; width: 210px; box-sizing: border-box;
    display: flex; flex-direction: column; padding: 4px; border-radius: 10px; border: 1px solid var(--e-popover-edge);
    background: var(--e-popover-bg); box-shadow: 0 12px 32px var(--e-shadow-popover); font: 13px system-ui, sans-serif; }
  .endeavor-field [role="menu"][hidden] { display: none; }
  .endeavor-field [role="menuitem"] { height: 28px; display: flex; align-items: center; gap: 8px; padding: 0 8px; border: 0; border-radius: 6px;
    background: transparent; color: var(--e-text-primary); font: inherit; cursor: pointer; text-align: left; }
  .endeavor-field [role="menuitem"]:hover { background: var(--e-menu-hover); }
  .endeavor-field [role="menuitem"] .key { margin-left: auto; padding-left: 12px; font-size: 12px; color: var(--e-text-faint); }
`;
  function quoteField(placeholder, sendLabel, done) {
    const root = document.createElement("div");
    root.className = "endeavor-field";
    root.innerHTML = `<textarea rows="1" spellcheck="false" autocorrect="off" autocapitalize="off"></textarea><button class="options" aria-label="Send options" aria-haspopup="menu" aria-expanded="false">\u2191</button><div role="menu" hidden><button role="menuitem" data-add="false"><span></span><span class="key">\u21A9</span></button><button role="menuitem" data-add="true"><span>Add to message</span><span class="key">${shortcut("\u21A9")}</span></button></div>`;
    const text = root.querySelector("textarea");
    const options = root.querySelector(".options");
    const menu = root.querySelector("[role=menu]");
    text.placeholder = placeholder;
    menu.querySelector("[data-add=false] span").textContent = sendLabel;
    const showMenu = (shown) => {
      menu.hidden = !shown;
      options.setAttribute("aria-expanded", String(shown));
    };
    text.addEventListener("input", () => {
      text.style.height = "21px";
      text.style.height = `${Math.min(text.scrollHeight, 120)}px`;
    });
    text.addEventListener(
      "keydown",
      (e) => {
        e.stopPropagation();
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          showMenu(false);
          done(modHeld(e), e);
        }
      },
      true
    );
    options.onmousedown = (e) => e.preventDefault();
    options.onclick = () => showMenu(menu.hidden);
    for (const item of menu.querySelectorAll("[role=menuitem]")) {
      item.onmousedown = (e) => e.preventDefault();
      item.onclick = (e) => {
        showMenu(false);
        done(item.dataset.add === "true", e);
      };
    }
    return { root, text };
  }
  function initQuote() {
    const style2 = document.createElement("style");
    style2.textContent = css2;
    document.head.append(style2);
    on("shot", (msg) => waiting.get(msg.id)?.());
  }

  // src/reveal.ts
  var css3 = `
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
    style2.textContent = css3;
    document.head.append(style2);
    on("reveal", (msg) => reveal(msg.cells));
    on("code", (msg) => {
      const cell = document.getElementById(msg.cell);
      send({ type: "code", cell: msg.cell, code: cell ? cellCode(cell) : null });
    });
  }

  // src/annotate.ts
  var css4 = `
  body.annotating pluto-cell, body.annotating pluto-cell * { cursor: crosshair !important; }
  /* docs/ui-spec.md, "Pointing overlay": 25% dim, 1px accent edge, plain-text hint
     pill, dashed hover and solid picked outlines, dashed box for a drawn region. */
  body.annotating .annotate-hover { outline: 1.5px dashed var(--e-hover-edge); outline-offset: 3px; }
  body.annotating .annotate-picked { outline: 1.5px solid var(--e-accent); outline-offset: 3px; }
  body.annotating.annotate-drawing .annotate-hover { outline: none; }
  body.annotating.annotate-drawing, body.annotating.annotate-drawing * { user-select: none; }
  body.annotating pluto-input .cm-content { counter-reset: endeavor-line; padding-left: 2.6em !important; }
  body.annotating pluto-input .cm-line { counter-increment: endeavor-line; position: relative; }
  body.annotating pluto-input .cm-line::before { content: counter(endeavor-line); position: absolute; left: -2.6em; width: 2em;
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
  var DRAG = 4;
  var FIGURE = "img, svg, canvas, video";
  var PARAGRAPH = "p, li, h1, h2, h3, h4, h5, h6, pre, blockquote, table";
  function targetAt(el) {
    const cell = el?.closest("pluto-cell");
    if (!el || !cell) return null;
    const input = el.closest("pluto-input");
    if (input) return { part: "code", cell, el: input };
    const output = el.closest("pluto-output");
    if (!output) return { part: "cell", cell, el: cell };
    let figure = el.closest(FIGURE);
    while (figure?.parentElement?.closest(FIGURE) && output.contains(figure.parentElement.closest(FIGURE))) figure = figure.parentElement.closest(FIGURE);
    if (figure && output.contains(figure)) return { part: "figure", cell, el: figure };
    const paragraph = el.closest(PARAGRAPH);
    if (paragraph && output.contains(paragraph)) return { part: "output", cell, el: paragraph };
    return { part: output.querySelector(FIGURE) ? "figure" : "output", cell, el: output };
  }
  var pageBox = (r) => ({
    left: r.left + window.scrollX,
    top: r.top + window.scrollY,
    right: r.right + window.scrollX,
    bottom: r.bottom + window.scrollY
  });
  var cmLines = (cell) => [...cell.querySelectorAll("pluto-input .cm-line")];
  function pickStatus(picks) {
    if (picks.length > 1) return `${picks.length} picks`;
    const pick2 = picks[0];
    if (!pick2) return "Click something to pick it";
    if (pick2.part === "box") return `Box over ${pick2.cells.length} cell${pick2.cells.length === 1 ? "" : "s"}`;
    const name = cellName(pick2.code);
    switch (pick2.part) {
      case "cell":
        return `Cell ${name}`;
      case "lines":
        return pick2.lines[0] === pick2.lines[1] ? `Line ${pick2.lines[0]} of ${name}` : `Lines ${pick2.lines[0]}\u2013${pick2.lines[1]} of ${name}`;
      case "output":
        return `Output of ${name}`;
      case "figure":
        return `Figure in ${name}`;
    }
  }
  function samePick(a, b) {
    if (a.box || b.box) return a === b;
    if (a.pick.part === "lines" && b.pick.part === "lines") return a.pick.cell === b.pick.cell && a.pick.lines.join() === b.pick.lines.join();
    return !!a.el && a.el === b.el;
  }
  var state = null;
  function pointState() {
    const picks = state?.picks ?? [];
    const cells = picks.flatMap((p) => p.pick.part === "box" ? p.pick.cells : [p.pick.cell]);
    return {
      picked: [...new Set(cells)],
      picks: picks.map((p) => pickSource(p.pick)),
      box: picks.some((p) => p.box),
      status: state?.status() ?? "",
      comment: state?.comment() ?? ""
    };
  }
  function initAnnotate() {
    const picks = [];
    let hover = null;
    const active = () => document.body.classList.contains("annotating");
    const cells = () => [...document.querySelectorAll("pluto-cell")];
    const style2 = document.createElement("style");
    style2.textContent = css4;
    const frame2 = document.createElement("div");
    frame2.id = "annotate-frame";
    const box = document.createElement("div");
    box.id = "annotate-box";
    const tag = document.createElement("div");
    tag.id = "annotate-tag";
    const hint = document.createElement("div");
    hint.id = "annotate-hint";
    hint.innerHTML = `<span>Click to pick \xB7 drag over code lines \xB7 drag elsewhere for a box</span><span>\xB7</span><span class="done" role="button">Done</span>`;
    const bar = document.createElement("div");
    bar.id = "annotate-bar";
    bar.innerHTML = `<div class="head"><span class="status"></span><span class="keys">\u21A9 send \xB7 ${shortcut("\u21A9")} add to message</span></div>`;
    const status = bar.querySelector(".status");
    const field = quoteField("Comment for Claude\u2026", "Send", (add, e) => byUser(e) && sendComment(add));
    const text = field.text;
    bar.append(field.root);
    document.head.append(style2);
    document.body.append(frame2, box, tag, hint, bar);
    state = { picks, status: () => active() ? status.textContent ?? "" : "", comment: () => text.value };
    function drawBox(b) {
      box.classList.toggle("shown", !!b);
      if (!b) return;
      Object.assign(box.style, { left: `${b.left}px`, top: `${b.top}px`, width: `${b.right - b.left}px`, height: `${b.bottom - b.top}px` });
    }
    function lastRect() {
      const last2 = picks.at(-1);
      if (!last2) return null;
      if (last2.box) {
        const b = last2.box;
        return { left: b.left - window.scrollX, top: b.top - window.scrollY, right: b.right - window.scrollX, bottom: b.bottom - window.scrollY };
      }
      const shown = last2.lines?.length ? last2.lines : last2.el ? [last2.el] : [];
      if (!shown.length) return null;
      const [first, end] = [shown[0].getBoundingClientRect(), shown[shown.length - 1].getBoundingClientRect()];
      return { left: first.left, top: first.top, right: Math.max(first.right, end.right), bottom: end.bottom };
    }
    function place3() {
      const rect = lastRect();
      bar.classList.toggle("placed", !!rect);
      if (!rect) {
        bar.removeAttribute("style");
        return;
      }
      const at = barPlace(rect, window.innerWidth, window.innerHeight, bar.offsetHeight);
      Object.assign(bar.style, { left: `${at.left}px`, top: `${at.top}px`, width: `${at.width}px` });
    }
    function showHover(target) {
      hover?.el.classList.remove("annotate-hover");
      hover = target;
      tag.classList.toggle("shown", !!target);
      if (!target) return;
      target.el.classList.add("annotate-hover");
      tag.textContent = { cell: "Whole cell", code: "Code", output: "Text", figure: "Figure" }[target.part];
      const r = target.el.getBoundingClientRect();
      Object.assign(tag.style, { left: `${r.left + window.scrollX}px`, top: `${r.top + window.scrollY - 22}px` });
    }
    function refresh2() {
      for (const el of document.querySelectorAll(".annotate-picked")) el.classList.remove("annotate-picked");
      for (const el of document.querySelectorAll(".annotate-line")) el.classList.remove("annotate-line");
      for (const p of picks) {
        p.el?.classList.add("annotate-picked");
        for (const line of p.lines ?? []) line.classList.add("annotate-line");
      }
      drawBox([...picks].reverse().find((p) => p.box)?.box ?? null);
      status.textContent = pickStatus(picks.map((p) => p.pick));
      place3();
    }
    function choose(next, add) {
      const at = picks.findIndex((p) => samePick(p, next));
      if (add) at >= 0 ? picks.splice(at, 1) : picks.push(next);
      else picks.splice(0, picks.length, ...at >= 0 && picks.length === 1 ? [] : [next]);
      refresh2();
      text.focus();
    }
    function targetPick(target) {
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
    function linesPick(cell, from, to) {
      const [first, last2] = [Math.min(from, to), Math.max(from, to)];
      const code = cellCode(cell);
      const text2 = code.split("\n").slice(first - 1, last2).join("\n");
      return { pick: { part: "lines", cell: cell.id, code, lines: [first, last2], text: text2 }, lines: cmLines(cell).slice(first - 1, last2) };
    }
    function set(enable) {
      if (enable === active()) return;
      document.body.classList.toggle("annotating", enable);
      document.body.classList.remove("annotate-drawing");
      picks.length = 0;
      drag = null;
      showHover(null);
      if (enable) {
        for (const c of document.querySelectorAll("pluto-cell.selected")) picks.push(targetPick({ part: "cell", cell: c, el: c }));
        text.focus();
      }
      refresh2();
      send({ type: "mode", on: enable });
    }
    async function sendComment(add) {
      if (!picks.length) return;
      const sending = picks.splice(0, picks.length);
      const comment = text.value.trim();
      text.value = "";
      refresh2();
      const out = [];
      for (const p of sending) {
        if (p.pick.part === "box" || p.pick.part === "figure") {
          const b = p.box ?? (p.el ? pageBox(p.el.getBoundingClientRect()) : null);
          out.push(b ? { ...p.pick, shot: await shoot(b) } : p.pick);
        } else out.push(p.pick);
      }
      sendQuote(out, comment, add);
    }
    let drag = null;
    let justDragged = false;
    const dragBox = (e) => {
      const [x, y] = [e.clientX + window.scrollX, e.clientY + window.scrollY];
      return { left: Math.min(drag.x, x), top: Math.min(drag.y, y), right: Math.max(drag.x, x), bottom: Math.max(drag.y, y) };
    };
    const lineAt = (el, cell) => {
      const line = el?.closest(".cm-line");
      return line && cell.contains(line) ? cmLines(cell).indexOf(line) + 1 : 0;
    };
    const ours = (target) => bar.contains(target) || hint.contains(target);
    const swallow = (e) => {
      const target = e.target;
      if (!active() || ours(target)) return;
      if (e.type !== "pointerdown") e.preventDefault();
      e.stopPropagation();
      const m = e;
      if (e.type === "mousedown" && m.button === 0) {
        const cell = target.closest("pluto-cell");
        const line = cell && !m.altKey ? lineAt(target, cell) : 0;
        drag = { x: m.clientX + window.scrollX, y: m.clientY + window.scrollY, moved: false, add: m.shiftKey, lines: line ? { cell, from: line, to: line } : void 0 };
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
        if (!drag) return showHover(ours(e.target) ? null : targetAt(e.target));
        if (!drag.moved && Math.hypot(e.clientX + window.scrollX - drag.x, e.clientY + window.scrollY - drag.y) < DRAG) return;
        drag.moved = true;
        e.preventDefault();
        showHover(null);
        if (drag.lines) {
          const line = lineAt(e.target, drag.lines.cell);
          if (line) drag.lines.to = line;
          const shown = linesPick(drag.lines.cell, drag.lines.from, drag.lines.to);
          for (const el of document.querySelectorAll(".annotate-line")) el.classList.remove("annotate-line");
          for (const el of shown.lines ?? []) el.classList.add("annotate-line");
          return;
        }
        document.body.classList.add("annotate-drawing");
        drawBox(dragBox(e));
      },
      true
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
        setTimeout(() => justDragged = false, 0);
        if (done.lines) return choose(linesPick(done.lines.cell, done.lines.from, done.lines.to), done.add);
        const under = cells().filter((c) => {
          const r = pageBox(c.getBoundingClientRect());
          return drawn.left < r.right && r.left < drawn.right && drawn.top < r.bottom && r.top < drawn.bottom;
        });
        choose({ pick: { part: "box", cells: under.map((c) => c.id) }, box: drawn }, done.add);
      },
      true
    );
    window.addEventListener("scroll", () => active() && place3(), true);
    window.addEventListener("resize", () => active() && place3());
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
      true
    );
    hint.querySelector(".done").onclick = () => set(false);
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
  var css5 = `
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
    style2.textContent = css5;
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
  var css6 = `
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
      const state2 = states.get(cell.id);
      setAttr(cell, "data-endeavor", state2?.unrun ? "unrun" : null);
      setAttr(cell, "data-author", state2?.author ?? null);
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
    style2.textContent = css6;
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
      const point = pointState();
      send({
        type: "debug",
        point: annotating,
        picked: point.picked,
        picks: point.picks,
        box: point.box,
        point_status: point.status,
        comment: point.comment,
        drawer: drawer2 === "docs" || drawer2 === "status" ? drawer2 : null,
        callout: !!document.querySelector("#endeavor-safe.shown"),
        reply: document.querySelector("#endeavor-reply") ? "prompt" : document.querySelector("#endeavor-reply-pill") ? "pill" : null,
        // Recorded by the debug build's own script, which wraps `alert`.
        alerts: window.__endeavorAlerts ?? null
      });
    });
  }

  // src/diff.ts
  var css7 = `
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
  .endeavor-del { position: relative; background: var(--e-diff-del-tint); color: var(--e-diff-del); white-space: break-spaces; word-break: break-word; overflow-wrap: anywhere; }
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
    style2.textContent = css7;
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
  function cellName2(nb, id) {
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
      let state2;
      if (pkgPhase === "failed" && (failedSet.has(name) || failedSet.size === 0 && busy.has(name))) state2 = "failed";
      else if (busy.has(name) && pkgPhase === "busy") state2 = precompiled.has(name) ? "ready" : precompiling ? "precompiling" : "installing";
      else if (version != null) state2 = "ready";
      else if (pkgPhase === "done") state2 = Object.keys(installed2).length ? "failed" : "ready";
      else state2 = "waiting";
      const notFound = state2 === "failed" && version == null && pkgPhase === "done";
      return { name, state: state2, detail: state2 === "ready" ? detail : notFound ? "not found" : "" };
    });
    const deps = log.added.filter((n) => !direct.includes(n));
    const depsRow = deps.length ? { count: deps.length, precompiled: deps.filter((n) => precompiled.has(n)).length, failed: deps.filter((n) => failedSet.has(n)).length } : null;
    const cells = nb.cell_order.map((id) => {
      const r = nb.cell_results[id] ?? {};
      const state2 = r.running ? "running" : r.queued ? "waiting" : r.errored ? "failed" : r.runtime != null ? "done" : "waiting";
      const time = (state2 === "done" || state2 === "failed") && r.runtime != null ? prettyTime(r.runtime) : null;
      return { id, name: cellName2(nb, id), state: state2, time };
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
  var context = { host: "This Mac", asking: false, crash: null };
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
      context.crash = msg.crash ?? null;
      if (last && model) listeners.forEach((l) => l(last, model));
    });
    lastSent = "";
  }

  // src/drawer.ts
  var HEADER = 36;
  var css8 = `
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
  var markFor = (state2) => state2 === "done" || state2 === "ready" ? `<span class="mark">\u2713</span>` : state2 === "failed" ? `<span class="mark failed">\u2715</span>` : state2 === "waiting" ? `<span class="mark waiting"></span>` : `<span class="mark busy"></span>`;
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
    style2.textContent = css8;
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
  var css9 = `
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
  /* The empty-cell hint names the shortcut. */
  pluto-input .cm-placeholder { font-size: 0; }
  pluto-input .cm-placeholder::after { content: "Type code, or ${shortcut("E")} to ask ${AGENT}"; font-size: 13px; }
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
    box.style.width = `${Math.min(Math.max(rect.width, 320), 640)}px`;
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
          const kind = where !== "cell" ? where : isEmpty(cell) ? "fill" : "about";
          send({ type: "prompt", notebook, cell: cell.id, code: cellCode(cell), where: kind, text: comment, now: modHeld(e) });
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
    const style2 = document.createElement("style");
    style2.textContent = css9;
    document.head.append(style2);
    window.addEventListener(
      "keydown",
      (e) => {
        if (!(modHeld(e) && e.key.toLowerCase() === "e") || e.shiftKey) return;
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

  // src/reply.ts
  var css10 = `
  #endeavor-reply-pill { position: absolute; z-index: 1000; display: inline-flex; align-items: center; height: 30px; box-sizing: border-box;
    padding: 0 3px; border-radius: 8px; border: 1px solid var(--e-popover-edge); background: var(--e-popover-bg);
    box-shadow: 0 8px 24px var(--e-shadow-popover); font: 13px/18px system-ui, sans-serif; }
  #endeavor-reply-pill button { height: 24px; display: flex; align-items: center; gap: 6px; padding: 0 8px; border: 0; border-radius: 5px;
    background: transparent; color: var(--e-text-primary); font: inherit; cursor: pointer; }
  #endeavor-reply-pill button:hover { background: var(--e-menu-hover); }
  #endeavor-reply-pill .key { font-size: 11px; color: var(--e-text-faint); }
  #endeavor-reply { position: absolute; z-index: 1000; width: 380px; box-sizing: border-box; display: flex; flex-direction: column; gap: 6px;
    padding: 8px; border-radius: 10px; border: 1px solid var(--e-popover-edge); background: var(--e-popover-bg);
    box-shadow: 0 12px 32px var(--e-shadow-popover); font: 13px system-ui, sans-serif; color: var(--e-text-primary); }
  #endeavor-reply .quote { border-left: 2px solid var(--e-control-edge); padding-left: 8px; font-size: 12.5px; line-height: 17px;
    color: var(--e-text-faint); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  #endeavor-reply .source { font-size: 12px; color: var(--e-text-faint); }
`;
  function lineOf(node, content) {
    const view = content.cmTile?.root?.view;
    if (view) return view.state.doc.lineAt(view.posAtDOM(node)).number;
    const line = (node instanceof Element ? node : node.parentElement)?.closest(".cm-line");
    return [...content.querySelectorAll(".cm-line")].indexOf(line) + 1;
  }
  function selectedInCell() {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || !selection.rangeCount) return null;
    const range = selection.getRangeAt(0);
    const node = range.commonAncestorContainer;
    const element = node instanceof Element ? node : node.parentElement;
    const cell = element?.closest("pluto-cell");
    if (!cell || element?.closest("[data-endeavor-ui]")) return null;
    const rects = [...range.getClientRects()];
    if (!rects.length) rects.push(range.getBoundingClientRect());
    const code = cellCode(cell);
    const content = element?.closest(".cm-content");
    if (content) {
      const view = content.cmTile?.root?.view;
      const main = view?.state.selection.main;
      let [first, last2] = [lineOf(range.startContainer, content), lineOf(range.endContainer, content)];
      if (main && !main.empty) [first, last2] = [view.state.doc.lineAt(main.from).number, view.state.doc.lineAt(Math.max(main.from, main.to - 1)).number];
      if (first < 1 || last2 < first) return null;
      const quote2 = main && !main.empty ? view.state.sliceDoc(main.from, main.to) : code.split("\n").slice(first - 1, last2).join("\n");
      if (!quote2.trim()) return null;
      return { cell, pick: { part: "lines", cell: cell.id, code, lines: [first, last2], text: quote2 }, quote: quote2, rects };
    }
    if (!element?.closest("pluto-output")) return null;
    const quote = selection.toString().trim();
    return quote ? { cell, pick: { part: "output", cell: cell.id, code, text: quote }, quote, rects } : null;
  }
  function pillAt(found) {
    const at = pillPlace(found.rects, window.innerHeight);
    return { left: at.left + window.scrollX, top: at.top + window.scrollY };
  }
  function initReply() {
    const style2 = document.createElement("style");
    style2.textContent = css10;
    document.head.append(style2);
    let pill = null;
    let prompt = null;
    const hidePill = () => {
      pill?.remove();
      pill = null;
    };
    const close2 = () => {
      prompt?.remove();
      prompt = null;
    };
    function open2(found) {
      hidePill();
      close2();
      const box = document.createElement("div");
      box.id = "endeavor-reply";
      box.dataset.endeavorUi = "";
      box.setAttribute("role", "dialog");
      box.setAttribute("aria-label", "Reply");
      const quote = document.createElement("div");
      quote.className = "quote";
      quote.textContent = found.quote.split(/\s+/).join(" ").trim();
      const source = document.createElement("div");
      source.className = "source";
      source.textContent = pickSource(found.pick);
      const field = quoteField("Reply to Claude", "Send reply", (add, e) => {
        if (!byUser(e)) return;
        sendQuote([found.pick], field.text.value.trim(), add);
        if (add) window.getSelection()?.removeAllRanges();
        close2();
      });
      field.text.addEventListener(
        "keydown",
        (e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            close2();
          }
        },
        true
      );
      box.append(quote, source, field.root);
      document.body.append(box);
      const place3 = pillAt(found);
      box.style.left = `${Math.min(place3.left - 12, window.scrollX + window.innerWidth - 388)}px`;
      box.style.top = `${place3.top}px`;
      prompt = box;
      requestAnimationFrame(() => field.text.focus());
    }
    document.addEventListener("mouseup", (e) => {
      const target = e.target;
      if (pill?.contains(target) || prompt?.contains(target) || document.body.classList.contains("annotating")) return;
      setTimeout(() => {
        hidePill();
        const found = selectedInCell();
        if (!found) return;
        pill = document.createElement("div");
        pill.id = "endeavor-reply-pill";
        pill.dataset.endeavorUi = "";
        pill.innerHTML = `<button aria-label="Reply">Reply <span class="key">${shortcut("J")}</span></button>`;
        const place3 = pillAt(found);
        pill.style.left = `${place3.left}px`;
        pill.style.top = `${place3.top}px`;
        pill.onmousedown = (event) => event.preventDefault();
        pill.querySelector("button").onclick = (event) => byUser(event) && open2(found);
        document.body.append(pill);
      });
    });
    document.addEventListener("mousedown", (e) => {
      if (prompt && !prompt.contains(e.target)) close2();
    });
    window.addEventListener(
      "keydown",
      (e) => {
        if (modHeld(e) && !e.shiftKey && e.key.toLowerCase() === "j") {
          const found = selectedInCell();
          if (!found || document.body.classList.contains("annotating")) return;
          e.preventDefault();
          e.stopPropagation();
          return open2(found);
        }
        if (!prompt?.contains(e.target)) hidePill();
      },
      true
    );
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
    const css13 = document.createElement("style");
    css13.textContent = `
    body.${CLASS} pluto-notebook { opacity: 0.85; }
    body.${CLASS} pluto-notebook, body.${CLASS} pluto-notebook * { pointer-events: none !important; }
  `;
    document.head.append(css13);
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
  var css11 = `
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
    style2.textContent = css11;
    document.head.append(style2);
    onRedraw(decorate2);
  }

  // src/safe.ts
  var css12 = `
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
  #endeavor-safe code { font: 12px JuliaMono, ui-monospace, monospace; padding: 0 3px; border-radius: 3px;
    background: var(--e-bg-tag); color: var(--e-text-primary); }
`;
  var escape3 = (s) => s.replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
  var withCode = (s) => escape3(s).replace(/`([^`]*)`/g, "<code>$1</code>");
  var shield = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M7 1.2 12 3v3.6c0 3-2.2 5.2-5 6.2-2.8-1-5-3.2-5-6.2V3z"/></svg>`;
  var play = `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" style="stroke: var(--e-accent-text)" stroke-width="1.3"><path d="M2.5 1.5v7l6-3.5z"/></svg>`;
  var callout;
  function render2() {
    const now = current();
    const safe = now?.nb.process_status === "waiting_for_permission";
    callout.classList.toggle("shown", !!safe);
    if (!safe) return;
    const asking = context.asking ? `<div class="asking">Claude is asking to run it. Answer in the chat, or here.</div>` : "";
    const crash = context.crash;
    const title = crash ? escape3(crash.title) : "Safe preview";
    const body2 = crash ? withCode(crash.body) : "You're reading and editing this file without running any code.";
    const html = `${shield}<div class="text"><b>${title}</b>${body2}${asking}</div><button class="run">${play}Run notebook</button>`;
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
    style2.textContent = css12;
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
  --e-text-tag: #9A9997;
  --e-border: #2A2A2E;
  --e-control-edge: #3A3A40;
  --e-divider: #1F1F22;
  --e-text-primary: #F0EFEC;
  --e-text-secondary: #BDBCBA;
  --e-text-muted: #8C8B8A;
  --e-text-faint: #858483;
  --e-text-section: #888786;
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
  --e-text-dim: #7A7978;
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
    --pluto-output-color: #BDBCBA;
    --pluto-output-h-color: #E0DFDC;
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
    --helpbox-header-color: #F0EFEC;
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
    --pluto-runarea-span-color: #858483;
  }
  .pluto-modal { border: 1px solid #2A2A2E; border-radius: 8px !important; box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5) !important; }
  .pluto-modal-dark h1 { color: #F0EFEC; font-weight: 600; }
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
    initQuote();
    initAnnotate();
    initCells();
    initDiffs();
    initPrompt();
    initReply();
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
