// Pluto error boxes get Fix with Claude / Explain above the trace, a status line
// after a click, "Fails because …" on cells that fail only because a cell above
// failed, and a folded trace.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

const NB = "0f381e2e-b8ca-11f1-b549-49cf0ce82801";
const FIT = "33333333-3333-3333-3333-333333333333";
const HALF = "44444444-4444-4444-4444-444444444444";
const tick = () => new Promise((r) => setTimeout(r, 0));

async function page(html, editorState) {
  const dom = new JSDOM(`<body>${html}</body>`, { url: `http://localhost/edit?id=${NB}`, runScripts: "outside-only" });
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  if (editorState) window.editor_state = editorState;
  // With a pluto-editor, state.ts reads Pluto's state every frame; no frames here.
  window.requestAnimationFrame = () => 0;
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const context = (extra) => window.__endeavor.receive({ type: "context", host: "This Mac", asking: false, readonly: false, ...extra });
  return { window, doc: window.document, sent, context };
}

const errorBox = (header, trace = true) =>
  `<pluto-output><jlerror><div class="error-header"><secret-h1>Error message</secret-h1></div><header>${header}</header>` +
  (trace ? `<section class="stacktrace-waiting-to-view"><button>Show stack trace</button></section>` : "") +
  `</jlerror></pluto-output>`;

const actions = (doc, id) => doc.querySelector(`[id="${id}"] .endeavor-ask`);
/** An element's text as read: its parts are laid out apart (flex gaps). */
const words = (el) => (el ? [...el.childNodes].map((n) => n.textContent.trim()).filter(Boolean).join(" ") : null);

test("Fix with Claude and Explain sit under the message, send the error, and turn into a status line", async () => {
  const msg = "UndefVarError: `p0` not defined in `Main`\nSuggestion: check for spelling errors or missing imports.";
  const stacktrace = [{ call: "top-level scope", file: `/n/fit.jl#==#${FIT}`, line: 1 }];
  const { doc, sent, context } = await page(`<pluto-cell id="${FIT}"></pluto-cell>`, {
    notebook: {
      cell_results: { [FIT]: { errored: true, output: { body: { msg, stacktrace } } } },
      cell_dependencies: { [FIT]: { upstream_cells_map: { p0: [] }, downstream_cells_map: { fit: [] } } },
    },
  });
  doc.getElementById(FIT).innerHTML = errorBox("UndefVarError: p0 not defined in Main");
  await tick();

  const error = doc.querySelector("jlerror");
  // Message, actions, the folded trace's line, then Pluto's own (hidden) trace.
  assert.deepEqual(
    [...error.children].map((c) => c.className || c.tagName.toLowerCase()),
    ["error-header", "header", "endeavor-ask", "endeavor-trace", "stacktrace-waiting-to-view"],
  );
  assert.deepEqual([...error.querySelectorAll(".endeavor-ask button")].map((b) => b.textContent), ["✦Fix with Claude", "Explain"]);
  assert.match(words(actions(doc, FIT)), /or (⌘E|Ctrl\+E) to ask something else$/);
  assert.equal(words(error.querySelector(".endeavor-trace")), "Stack trace · 1 step ›");
  assert.equal(error.getAttribute("data-endeavor-trace"), "closed");

  error.querySelector(".endeavor-ask .fix").click();
  assert.deepEqual(sent.at(-1), {
    type: "ask",
    kind: "fix",
    notebook: NB,
    cell: FIT,
    code: "",
    error: `${msg}\nStacktrace:\n [1] top-level scope @ cell ${FIT}, line 1`,
  });
  // At once, before the app answers: the buttons are gone, so no second send.
  assert.equal(words(actions(doc, FIT)), "✦ Claude is fixing this · Show in chat ›");
  assert.equal(doc.querySelector(".endeavor-ask .fix"), null);

  // The app says Claude is on it; Show in chat asks the app to show the message.
  context({ working: true, error_asks: [{ cell: FIT, kind: "fix", queued: false }] });
  doc.querySelector(".endeavor-ask a.show").click();
  assert.deepEqual(sent.at(-1), { type: "error_ask_show", cell: FIT });
  assert.equal(sent.filter((m) => m.type === "ask").length, 1);

  // The turn ends with the error still there: the buttons come back.
  context({ working: false, error_asks: [] });
  assert.deepEqual([...error.querySelectorAll(".endeavor-ask button")].map((b) => b.textContent), ["✦Fix with Claude", "Explain"]);

  // Redraws don't add a second row.
  doc.body.append(doc.createElement("div"));
  await tick();
  assert.equal(doc.querySelectorAll(".endeavor-ask").length, 1);
});

test("while Claude works, a click queues, with Cancel", async () => {
  const { doc, sent, context } = await page(`<pluto-cell id="${FIT}"></pluto-cell>`);
  doc.getElementById(FIT).innerHTML = errorBox("UndefVarError: `lsq` not defined", false);
  await tick();
  // Without Pluto's state the header's text goes; with no trace there's no trace line.
  assert.equal(doc.querySelector(".endeavor-trace"), null);
  context({ working: true, error_asks: [] });
  doc.querySelector(".endeavor-ask .explain").click();
  assert.equal(sent.at(-1).error, "UndefVarError: `lsq` not defined");
  assert.equal(sent.at(-1).kind, "explain");
  assert.equal(words(actions(doc, FIT)), "Explain queued · sends after Claude’s current turn · Cancel");
  context({ working: true, error_asks: [{ cell: FIT, kind: "explain", queued: true }] });
  doc.querySelector(".endeavor-ask a.cancel").click();
  assert.deepEqual(sent.at(-1), { type: "error_ask_cancel", cell: FIT });
  context({ working: true, error_asks: [] });
  assert.equal(doc.querySelectorAll(".endeavor-ask button").length, 2);
});

test("a cell that fails because a cell above failed gets no buttons, only what failed", async () => {
  const notebook = {
    cell_results: {
      [FIT]: { errored: true, output: { body: { msg: "UndefVarError: `p0` not defined", stacktrace: [] } } },
      [HALF]: { errored: true, output: { body: { msg: 'UndefVarError: `fit` not defined in `Main.var"workspace#3"`', stacktrace: [] } } },
    },
    cell_dependencies: {
      [FIT]: { upstream_cells_map: { p0: [] }, downstream_cells_map: { fit: [HALF] } },
      [HALF]: { upstream_cells_map: { fit: [FIT] }, downstream_cells_map: { half_life: [] } },
    },
  };
  const { doc } = await page(`<pluto-cell id="${FIT}"></pluto-cell><pluto-cell id="${HALF}"></pluto-cell>`, { notebook });
  doc.getElementById(FIT).innerHTML = errorBox("UndefVarError: p0 not defined");
  doc.getElementById(HALF).innerHTML = errorBox("<p><em>Another cell defining fit contains errors.</em></p>", false);
  await tick();

  assert.ok(actions(doc, FIT), "the cell where the error starts keeps its buttons");
  const half = doc.getElementById(HALF).querySelector("jlerror");
  assert.equal(half.getAttribute("data-endeavor-error"), "upstream");
  assert.equal(actions(doc, HALF), null);
  assert.equal(words(half.querySelector(".endeavor-upstream .message")), "UndefVarError: fit not defined in Main");
  assert.equal(words(half.querySelector(".endeavor-upstream .why")), "Fails because fit failed · Show ›");
  let shown = null;
  doc.getElementById(FIT).scrollIntoView = () => (shown = FIT);
  half.querySelector(".endeavor-upstream a").click();
  assert.equal(shown, FIT);
});

test("a cell that fails because a cell above failed has one bar: the grey edge, not Pluto's red", async () => {
  const notebook = {
    cell_results: {
      [FIT]: { errored: true, output: { body: { msg: "UndefVarError: `p0` not defined", stacktrace: [] } } },
      [HALF]: { errored: true, output: { body: { msg: "UndefVarError: `fit` not defined", stacktrace: [] } } },
    },
    cell_dependencies: {
      [FIT]: { upstream_cells_map: { p0: [] }, downstream_cells_map: { fit: [HALF] } },
      [HALF]: { upstream_cells_map: { fit: [FIT] }, downstream_cells_map: {} },
    },
  };
  const cell = (id) => `<pluto-cell id="${id}" class="errored"><pluto-trafficlight></pluto-trafficlight></pluto-cell>`;
  const { doc } = await page(`<pluto-editor><pluto-notebook>${cell(FIT)}${cell(HALF)}</pluto-notebook></pluto-editor>`, { notebook });
  doc.documentElement.setAttribute("data-endeavor-look", "endeavor");
  for (const id of [FIT, HALF]) doc.getElementById(id).insertAdjacentHTML("beforeend", errorBox("UndefVarError", false));
  await tick();

  /** The background our rules give a cell's bar, or "Pluto's" when none apply. */
  const bar = (id) => {
    const light = doc.getElementById(id).querySelector("pluto-trafficlight");
    let found = "Pluto's";
    for (const style of doc.head.querySelectorAll("style")) {
      for (const [, selectors, body] of style.textContent.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
        for (const selector of selectors.split(",").map((s) => s.trim())) {
          if (selector.endsWith("> pluto-trafficlight") && light.matches(selector)) found = body.match(/background: ([^;]+);/)?.[1] ?? found;
        }
      }
    }
    return found;
  };
  assert.equal(bar(FIT), "Pluto's", "the cell where the error starts keeps Pluto's red bar");
  assert.equal(bar(HALF), "var(--normal-cell-color)");
  doc.getElementById(HALF).classList.add("selected");
  assert.equal(bar(HALF), "var(--selected-cell-color)");

  doc.getElementById(HALF).setAttribute("data-endeavor-bar", "claude");
  assert.equal(bar(HALF), "var(--e-accent)", "a cell Claude touched keeps the accent bar");

  doc.getElementById(HALF).querySelector("pluto-output").remove();
  doc.getElementById(HALF).classList.remove("errored");
  await tick();
  assert.equal(doc.getElementById(HALF).getAttribute("data-endeavor-error"), null);
});

test("a long trace is folded to one line with its length, and opening it opens Pluto's trace", async () => {
  const stacktrace = [
    ...Array.from({ length: 11 }, (_, i) => ({ call: `f${i}`, file: "LsqFit/src/curve_fit.jl", line: i + 1 })),
    { call: "fit_decay", file: `/n/fit.jl#==#${FIT}`, line: 1 },
    { call: "top-level scope", file: `/n/fit.jl#==#${FIT}`, line: 1 },
  ];
  const { doc } = await page(`<pluto-cell id="${FIT}"></pluto-cell>`, {
    notebook: { cell_results: { [FIT]: { errored: true, output: { body: { msg: "MethodError: no method matching curve_fit", stacktrace } } } }, cell_dependencies: {} },
  });
  doc.getElementById(FIT).innerHTML = errorBox("MethodError: no method matching curve_fit");
  await tick();
  const error = doc.querySelector("jlerror");
  let opened = false;
  error.querySelector("section button").onclick = () => (opened = true);
  assert.equal(words(error.querySelector(".endeavor-trace")), "Stack trace · 13 steps ›");
  error.querySelector(".endeavor-trace").click();
  assert.equal(error.getAttribute("data-endeavor-trace"), "open");
  assert.equal(words(error.querySelector(".endeavor-trace")), "Stack trace · 2 steps in your notebook, 11 inside packages ⌄");
  assert.ok(opened, "Pluto's own Show stack trace was pressed");
});
