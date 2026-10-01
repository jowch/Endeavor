// The ⌘E prompt in a fake Pluto page: which prompt the key opens, what the app
// gets for ↩ and ⌘↩, drafts, and where the prompt sits.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

const NB = "0f381e2e-b8ca-11f1-b549-49cf0ce82801";
const [A, B] = ["11111111-1111-1111-1111-111111111111", "22222222-2222-2222-2222-222222222222"];
const CODE = "rates = map(1:1000) do _\n  fit(rows)\nend";

const cell = (id, code, top) =>
  `<pluto-cell id="${id}" data-top="${top}"><pluto-output><p>The middle 95% of the rates.</p></pluto-output>` +
  `<pluto-input><div class="cm-content" tabindex="0">${code
    .split("\n")
    .filter((l) => l)
    .map((l) => `<div class="cm-line">${l}</div>`)
    .join("")}</div></pluto-input></pluto-cell>`;

async function page(platform = "MacIntel") {
  const dom = new JSDOM(`<body>${cell(A, CODE, 100)}${cell(B, "", 400)}<div id="outside" tabindex="0"></div></body>`, {
    url: `http://localhost/edit?id=${NB}`,
    runScripts: "outside-only",
    pretendToBeVisual: true,
  });
  const { window } = dom;
  Object.defineProperty(window.navigator, "platform", { value: platform });
  Object.defineProperty(window, "innerWidth", { value: 900 });
  Object.defineProperty(window, "innerHeight", { value: 700 });
  // jsdom lays nothing out: each cell's code box is 80 px tall at its data-top, the prompt is 70 px.
  window.HTMLElement.prototype.getBoundingClientRect = function () {
    const top = Number(this.closest("pluto-cell")?.dataset.top ?? 0);
    return { left: 100, top, right: 700, bottom: top + 80, width: 600, height: 80 };
  };
  Object.defineProperty(window.HTMLElement.prototype, "offsetHeight", { get: () => 70 });
  window.Range.prototype.getClientRects = () => [{ left: 120, top: 120, right: 300, bottom: 138 }];
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const $ = (selector) => window.document.querySelector(selector);
  const frame = () => new Promise((done) => window.requestAnimationFrame(() => done()));
  const key = (k, init = {}) => {
    const e = new window.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init });
    (window.document.activeElement ?? window.document.body).dispatchEvent(e);
    return e.defaultPrevented;
  };
  const mod = platform === "MacIntel" ? { metaKey: true } : { ctrlKey: true };
  const inCell = (id) => window.document.getElementById(id).querySelector(".cm-content").focus();
  const type = (words) => {
    $("#endeavor-ask textarea").value = words;
    $("#endeavor-ask textarea").dispatchEvent(new window.Event("input"));
  };
  return { window, $, sent, frame, key, mod, inCell, type };
}

test("⌘E routes: a selection asks about it, the cursor in a cell about the cell, elsewhere nothing", async () => {
  const { window, $, key, mod, inCell, frame } = await page();
  $("#outside").focus();
  assert.equal(key("e", mod), false, "not in a cell, nothing selected: the key goes on");
  assert.equal($("#endeavor-ask"), null);

  inCell(A);
  assert.equal(key("e", mod), true);
  assert.equal($("#endeavor-ask").dataset.kind, "cell");
  assert.equal($("#endeavor-ask .what").textContent, "rates·whole cell");
  assert.equal($("#endeavor-ask textarea").placeholder, "Ask Claude about this cell");
  assert.ok(window.document.getElementById(A).querySelector("pluto-input").classList.contains("endeavor-asked"), "the cell is outlined");
  await frame();
  key("Escape");
  assert.equal($(".endeavor-asked"), null, "the outline goes with the prompt");

  const text = window.document.querySelector(`[id="${A}"] pluto-output p`).firstChild;
  const range = window.document.createRange();
  range.setStart(text, 4);
  range.setEnd(text, 14);
  inCell(A);
  window.getSelection().removeAllRanges();
  window.getSelection().addRange(range);
  key("e", mod);
  assert.equal($("#endeavor-ask").dataset.kind, "selection", "a selection wins over the cursor");
  assert.equal($("#endeavor-ask .what").textContent, "rates·output");
  assert.equal($("#endeavor-ask .quote").textContent, "middle 95%");
});

test("⌘J is Reply only: with nothing selected it does nothing in a cell", async () => {
  const { $, key, mod, inCell } = await page();
  inCell(A);
  assert.equal(key("j", mod), false);
  assert.equal($("#endeavor-ask"), null);
});

test("↩ sends the question now; ⌘↩ adds it to the message", async () => {
  const { window, $, sent, key, mod, inCell, type, frame } = await page();
  inCell(A);
  key("e", mod);
  await frame();
  assert.ok(!$("#endeavor-ask .send").classList.contains("ready"), "the send button is grey while empty");
  key("Enter");
  assert.equal(sent.filter((m) => m.type === "prompt").length, 0, "nothing to send");
  type("Why resample the rows?");
  assert.ok($("#endeavor-ask .send").classList.contains("ready"), "and orange with words");
  key("Enter");
  assert.deepEqual(sent.at(-1), { type: "prompt", notebook: NB, cell: A, code: CODE, where: "about", text: "Why resample the rows?", add: false });
  assert.equal($("#endeavor-ask"), null);
  await frame();
  assert.equal(window.document.activeElement, window.document.getElementById(A).querySelector(".cm-content"), "the keyboard goes back to the cell");

  key("e", mod);
  await frame();
  type("and the units?");
  key("Enter", mod);
  assert.equal(sent.at(-1).add, true);
  assert.equal(sent.at(-1).where, "about");

  key("e", mod);
  await frame();
  type("from the menu");
  $("#endeavor-ask .options").click();
  $("#endeavor-ask [data-add=true]").click();
  assert.deepEqual([sent.at(-1).text, sent.at(-1).add], ["from the menu", true]);
});

test("an empty cell asks what to write; ✦ Claude asks for a new cell", async () => {
  const { window, $, sent, key, mod, inCell, type, frame } = await page();
  inCell(B);
  key("e", mod);
  assert.equal($("#endeavor-ask textarea").placeholder, "Ask Claude what to write here");
  assert.equal($("#endeavor-ask .what").textContent, "new cell·after rates");
  await frame();
  type("plot the rates");
  key("Enter");
  assert.equal(sent.at(-1).where, "fill");

  // Pluto's own "+" buttons, which the ✦ Claude button sits beside.
  for (const id of [A, B]) {
    const plus = window.document.createElement("button");
    plus.className = "add_cell before";
    window.document.getElementById(id).append(plus);
  }
  await frame();
  await new Promise((done) => setTimeout(done, 50));
  window.document.querySelector(`[id="${B}"] > .endeavor-add-agent.before`).click();
  assert.equal($("#endeavor-ask").dataset.kind, "before");
  assert.equal($("#endeavor-ask textarea").placeholder, "Ask Claude to write a cell here");
  assert.ok($("#endeavor-ask-line"), "a line shows where the new cell goes");
  assert.equal($(".endeavor-asked"), null, "instead of an outline");
});

test("Esc keeps the words as the cell's draft; ⌘E there brings them back, selected", async () => {
  const { window, $, key, mod, inCell, type, frame } = await page();
  inCell(A);
  key("e", mod);
  await frame();
  type("half a thought");
  key("Escape");
  assert.equal($("#endeavor-ask"), null);
  await frame();

  inCell(B);
  key("e", mod);
  assert.equal($("#endeavor-ask textarea").value, "", "another cell starts empty");
  window.document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }));
  assert.equal($("#endeavor-ask"), null, "a click outside closes it too");

  inCell(A);
  key("e", mod);
  await frame();
  const field = $("#endeavor-ask textarea");
  assert.equal(field.value, "half a thought");
  assert.deepEqual([field.selectionStart, field.selectionEnd], [0, "half a thought".length]);

  key("e", mod);
  assert.equal($("#endeavor-ask"), null, "⌘E in the open prompt closes it");
  inCell(A);
  key("e", mod);
  assert.equal($("#endeavor-ask textarea").value, "half a thought", "and keeps the draft");
});

test("the prompt opens 8 px under the cell, 380 wide, and above it near the pane's bottom", async () => {
  const { window, $, key, mod, inCell } = await page();
  inCell(A);
  key("e", mod);
  const style = $("#endeavor-ask").style;
  assert.deepEqual([style.left, style.top, style.width], ["100px", "188px", "380px"]);
  key("Escape");
  window.document.getElementById(A).dataset.top = "580";
  inCell(A);
  key("e", mod);
  assert.equal($("#endeavor-ask").style.top, "502px", "no room below: 8 px above");
});

test("while Claude works, ↩ queues", async () => {
  const { window, $, key, mod, inCell } = await page();
  window.__endeavor.receive({ type: "context", host: "This Mac", asking: false, readonly: false, working: true });
  assert.ok(window.document.documentElement.hasAttribute("data-endeavor-working"));
  inCell(A);
  key("e", mod);
  assert.ok($("#endeavor-ask .note.working"), "a line under the field says so");
  window.__endeavor.receive({ type: "context", host: "This Mac", asking: false, readonly: false, working: false });
  assert.ok(!window.document.documentElement.hasAttribute("data-endeavor-working"));
});

test("off macOS: Ctrl+E, and the keys spelled out", async () => {
  const { $, key, inCell } = await page("Linux x86_64");
  inCell(A);
  assert.equal(key("e", { metaKey: true }), false, "⌘ is not the key off macOS");
  key("e", { ctrlKey: true });
  assert.equal($("#endeavor-ask .keys").textContent, "Enter sendEnter queue · Ctrl+Enter add to message");
});
