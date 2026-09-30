// The built page script in a fake Pluto page: the app turns annotation mode on
// through the bridge, a cell is picked, and the comment reaches the app.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("annotation round trip through the bridge", async () => {
  const dom = new JSDOM(
    `<body><pluto-cell id="11111111-1111-1111-1111-111111111111"></pluto-cell>` +
      `<pluto-cell id="22222222-2222-2222-2222-222222222222"></pluto-cell></body>`,
    { url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801", runScripts: "outside-only" },
  );
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  // The script sets itself up on DOMContentLoaded, as in the app's webview.
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));

  window.__endeavor.receive({ type: "annotate", on: true });
  assert.ok(window.document.body.classList.contains("annotating"));
  assert.deepEqual(sent.at(-1), { type: "mode", on: true });

  const cell = window.document.getElementById("22222222-2222-2222-2222-222222222222");
  cell.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  assert.ok(cell.classList.contains("annotate-picked"));

  window.document.querySelector("#annotate-bar textarea").value = "why is this slow?";
  window.document.querySelector("#annotate-bar .send").click();
  assert.deepEqual(sent.at(-1), {
    type: "quote",
    notebook: "0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    picks: [{ part: "cell", cell: "22222222-2222-2222-2222-222222222222", code: "" }],
    comment: "why is this slow?",
    add: false,
  });

  window.__endeavor.receive({ type: "annotate", on: false });
  assert.ok(!window.document.body.classList.contains("annotating"));
});

test("a drawn box sends its cells and where it is, with the overlay hidden for the picture", async () => {
  const [A, B] = ["11111111-1111-1111-1111-111111111111", "22222222-2222-2222-2222-222222222222"];
  const dom = new JSDOM(`<body><pluto-cell id="${A}"></pluto-cell><pluto-cell id="${B}"></pluto-cell></body>`, {
    url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
    pretendToBeVisual: true,
  });
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const at = (top, bottom) => () => ({ left: 0, right: 600, top, bottom, width: 600, height: bottom - top });
  window.document.getElementById(A).getBoundingClientRect = at(0, 100);
  window.document.getElementById(B).getBoundingClientRect = at(120, 400);

  window.__endeavor.receive({ type: "annotate", on: true });
  assert.equal(window.document.querySelector("#annotate-hint").textContent, "Click a cell or drag a box·Done");
  const mouse = (type, x, y) => window.document.getElementById(B).dispatchEvent(new window.MouseEvent(type, { bubbles: true, clientX: x, clientY: y }));
  mouse("mousedown", 50, 150);
  mouse("mousemove", 250, 200);
  mouse("mouseup", 250, 300);
  mouse("click", 250, 300);
  assert.ok(window.document.querySelector("#annotate-box").classList.contains("shown"));
  assert.ok(window.document.getElementById(B).classList.contains("annotate-picked"), "the click that ends a drag doesn't unpick");
  assert.ok(!window.document.getElementById(A).classList.contains("annotate-picked"));

  window.document.querySelector("#annotate-bar textarea").value = "what's this bump?";
  window.document.querySelector("#annotate-bar .send").click();
  assert.ok(window.document.body.classList.contains("endeavor-shooting"));
  await new Promise((done) => setTimeout(done, 100));
  assert.deepEqual(sent.at(-1), { type: "shoot", id: 1, rect: { x: 50, y: 150, width: 200, height: 150 } });
  window.__endeavor.receive({ type: "shot", id: 1 });
  await new Promise((done) => setTimeout(done, 0));
  assert.ok(!window.document.body.classList.contains("endeavor-shooting"));
  assert.deepEqual(sent.at(-1), {
    type: "quote",
    notebook: "0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    picks: [{ part: "box", cells: [B], shot: 1 }],
    comment: "what's this bump?",
    add: false,
  });
  assert.ok(!window.document.querySelector("#annotate-box").classList.contains("shown"));
});
