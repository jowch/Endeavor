// Point in a fake Pluto page: what the pointer picks, lines dragged over code,
// boxes, and the quote message the app gets.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { buildSync } from "esbuild";

const NB = "0f381e2e-b8ca-11f1-b549-49cf0ce82801";
const [A, B] = ["11111111-1111-1111-1111-111111111111", "22222222-2222-2222-2222-222222222222"];
const CODE = "rates = map(1:1000) do _\n  fit(rows)\nend";

async function page() {
  const dom = new JSDOM(
    `<body><pluto-cell id="${A}"><pluto-output><div class="markdown"><p>Each resample draws rows.</p></div><pre>0.42</pre></pluto-output>` +
      `<pluto-input><div class="cm-content">${CODE.split("\n").map((l) => `<div class="cm-line">${l}</div>`).join("")}</div></pluto-input></pluto-cell>` +
      `<pluto-cell id="${B}"><pluto-output><div><svg><g><path/></g></svg></div></pluto-output></pluto-cell></body>`,
    { url: `http://localhost/edit?id=${NB}`, runScripts: "outside-only", pretendToBeVisual: true },
  );
  const { window } = dom;
  Object.defineProperty(window.navigator, "platform", { value: "MacIntel" });
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const $ = (selector) => window.document.querySelector(selector);
  const mouse = (el, type, x, y, init = {}) => el.dispatchEvent(new window.MouseEvent(type, { bubbles: true, clientX: x, clientY: y, ...init }));
  const click = (el, init) => mouse(el, "click", 0, 0, init);
  const comment = (text, meta = false) => {
    $("#annotate-bar textarea").value = text;
    $("#annotate-bar textarea").dispatchEvent(new window.KeyboardEvent("keydown", { key: "Enter", metaKey: meta, bubbles: true }));
  };
  window.__endeavor.receive({ type: "annotate", on: true });
  return { window, $, sent, mouse, click, comment };
}

test("a click picks the smallest thing under the pointer; Shift adds", async () => {
  const { window, $, sent, click, comment } = await page();
  assert.deepEqual(sent.at(-1), { type: "mode", on: true });
  assert.equal($("#annotate-bar .status").textContent, "Click something to pick it");
  click($("pluto-output p"));
  assert.equal($("#annotate-bar .status").textContent, "Output of rates");
  assert.ok($("pluto-output p").classList.contains("annotate-picked"));
  click($(".cm-line"));
  assert.equal($("#annotate-bar .status").textContent, "Lines 1–3 of rates", "a click on code picks the code block");
  assert.ok(!$("pluto-output p").classList.contains("annotate-picked"), "a plain click replaces the pick");
  click(window.document.getElementById(A), { shiftKey: true });
  assert.equal($("#annotate-bar .status").textContent, "2 picks");
  comment("Is sampling with replacement right here?");
  assert.deepEqual(sent.at(-1), {
    type: "quote",
    notebook: NB,
    picks: [
      { part: "lines", cell: A, code: CODE, lines: [1, 3], text: CODE },
      { part: "cell", cell: A, code: CODE },
    ],
    comment: "Is sampling with replacement right here?",
    add: false,
  });
  assert.ok(window.document.body.classList.contains("annotating"), "Point stays on for the next pick");
  assert.equal($("#annotate-bar .status").textContent, "Click something to pick it");
});

test("a figure goes with its picture, taken while the overlay hides", async () => {
  const { window, $, sent, click, comment } = await page();
  click($("path"));
  assert.ok($("svg").classList.contains("annotate-picked"), "the whole figure, not a shape in it");
  assert.equal($("#annotate-bar .status").textContent, "Figure in cell");
  comment("Why does the fit miss these early points?", true);
  await new Promise((done) => setTimeout(done, 100));
  assert.ok(window.document.body.classList.contains("endeavor-shooting"));
  assert.equal(sent.at(-1).type, "shoot");
  window.__endeavor.receive({ type: "shot", id: sent.at(-1).id });
  await new Promise((done) => setTimeout(done, 0));
  assert.ok(!window.document.body.classList.contains("endeavor-shooting"));
  assert.deepEqual(sent.at(-1), {
    type: "quote",
    notebook: NB,
    picks: [{ part: "figure", cell: B, code: "", shot: sent.at(-2).id }],
    comment: "Why does the fit miss these early points?",
    add: true,
  });
});

test("a drag that starts on code picks lines; ⌥ or anywhere else draws a box", async () => {
  const { window, $, sent, mouse, comment } = await page();
  const lines = window.document.querySelectorAll(".cm-line");
  mouse(lines[1], "mousedown", 40, 30);
  mouse(lines[2], "mousemove", 40, 50);
  assert.ok(lines[2].classList.contains("annotate-line") && !lines[0].classList.contains("annotate-line"));
  mouse(lines[2], "mouseup", 40, 50);
  mouse(lines[2], "click", 40, 50);
  assert.equal($("#annotate-bar .status").textContent, "Lines 2–3 of rates", "the click that ends a drag doesn't pick again");
  comment("");
  assert.deepEqual(sent.at(-1).picks, [{ part: "lines", cell: A, code: CODE, lines: [2, 3], text: "  fit(rows)\nend" }]);

  const at = (top, bottom) => () => ({ left: 0, right: 600, top, bottom, width: 600, height: bottom - top });
  window.document.getElementById(A).getBoundingClientRect = at(0, 100);
  window.document.getElementById(B).getBoundingClientRect = at(120, 400);
  mouse(lines[0], "mousedown", 50, 10, { altKey: true });
  mouse(lines[0], "mousemove", 250, 200);
  mouse(lines[0], "mouseup", 250, 300);
  mouse(lines[0], "click", 250, 300);
  assert.ok($("#annotate-box").classList.contains("shown"));
  assert.equal($("#annotate-bar .status").textContent, "Box over 2 cells");
  comment("what's this bump?");
  await new Promise((done) => setTimeout(done, 100));
  assert.deepEqual(sent.at(-1), { type: "shoot", id: sent.at(-1).id, rect: { x: 50, y: 10, width: 200, height: 290 } });
  window.__endeavor.receive({ type: "shot", id: sent.at(-1).id });
  await new Promise((done) => setTimeout(done, 0));
  assert.deepEqual(sent.at(-1).picks, [{ part: "box", cells: [A, B], shot: sent.at(-2).id }]);
  assert.ok(!$("#annotate-box").classList.contains("shown"));

  const output = $("pluto-output pre");
  mouse(output, "mousedown", 10, 10);
  mouse(output, "mousemove", 30, 30);
  mouse(output, "mouseup", 30, 30);
  assert.equal($("#annotate-bar .status").textContent, "Box over 1 cell", "a drag from an output draws a box");
});

test("line numbers show while Point is on, and Esc turns it off", async () => {
  const { window, $ } = await page();
  const css = [...window.document.querySelectorAll("style")].map((s) => s.textContent).join("");
  assert.match(css, /body\.annotating pluto-input \.cm-line::before \{ content: counter\(endeavor-line\)/);
  window.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape" }));
  assert.ok(!window.document.body.classList.contains("annotating"));
  assert.equal($("#annotate-hint").textContent, "Click to pick · drag over code lines · drag elsewhere for a box·Done");
});

const { outputFiles } = buildSync({ entryPoints: [new URL("../src/place.ts", import.meta.url).pathname], bundle: true, format: "esm", write: false });
const place = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString("base64")}`);

test("the comment bar opens under the pick, else above it, else pinned to the bottom", () => {
  const [width, height, bar] = [900, 700, 70];
  const pick = (top, bottom, left = 100, right = 700) => ({ left, top, right, bottom });
  assert.deepEqual(place.barPlace(pick(100, 300), width, height, bar), { left: 100, top: 308, width: 380 });
  assert.deepEqual(place.barPlace(pick(500, 640), width, height, bar), { left: 100, top: 422, width: 380 }, "no room below: 8 px above");
  assert.deepEqual(place.barPlace(pick(40, 680), width, height, bar), { left: 100, top: 614, width: 380 }, "no room either way: the pane's bottom");
  assert.deepEqual(place.barPlace(pick(100, 120, 10, 60), width, height, bar), { left: 16, top: 128, width: 380 }, "380 wide, inside the pane");
  assert.deepEqual(place.barPlace(pick(100, 120, 0, 2000), 300, height, bar).width, 268, "at most the pane minus 32");
  assert.deepEqual(place.barPlace(pick(-400, -100), width, height, bar).top, 8, "a pick scrolled up: the bar waits at the top");
  assert.deepEqual(place.barPlace(pick(900, 1200), width, height, bar).top, 614, "a pick scrolled down: the bar waits at the bottom");
  assert.deepEqual(place.barPlace(pick(100, 120, 700, 800), width, height, bar).left, 504, "kept inside the pane's right edge");
});
