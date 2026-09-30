// Reply on a selection in the notebook: the pill, the prompt, and what the app gets.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { buildSync } from "esbuild";

const NB = "0f381e2e-b8ca-11f1-b549-49cf0ce82801";
const CELL = "11111111-1111-1111-1111-111111111111";

async function page() {
  const dom = new JSDOM(
    `<body><pluto-cell id="${CELL}"><pluto-output><p>The middle 95% of the rates.</p></pluto-output>` +
      `<pluto-input><div class="cm-content"><div class="cm-line">rates = map(1:1000) do _</div>` +
      `<div class="cm-line">  rows = rand(1:n, n)</div><div class="cm-line">  fit(rows)</div><div class="cm-line">end</div></div></pluto-input></pluto-cell></body>`,
    { url: `http://localhost/edit?id=${NB}`, runScripts: "outside-only", pretendToBeVisual: true },
  );
  const { window } = dom;
  // jsdom lays nothing out: every selection is one line box.
  window.Range.prototype.getClientRects = () => [{ left: 10, top: 20, right: 110, bottom: 38 }];
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const select = (start, startOffset, end, endOffset) => {
    const range = window.document.createRange();
    range.setStart(start, startOffset);
    range.setEnd(end, endOffset);
    window.getSelection().removeAllRanges();
    window.getSelection().addRange(range);
    window.document.body.dispatchEvent(new window.MouseEvent("mouseup", { bubbles: true }));
    return new Promise((done) => setTimeout(done, 10));
  };
  const enter = (meta) =>
    window.document.querySelector("#endeavor-reply textarea").dispatchEvent(new window.KeyboardEvent("keydown", { key: "Enter", metaKey: meta, bubbles: true }));
  return { window, document: window.document, sent, select, enter };
}

test("a selection in code offers Reply, which sends the lines and the reply", async () => {
  const { document, sent, select, enter } = await page();
  const lines = document.querySelectorAll(".cm-line");
  await select(lines[1].firstChild, 2, lines[2].firstChild, 11);
  const pill = document.querySelector("#endeavor-reply-pill");
  assert.equal(pill?.textContent, "Reply ⌘J");
  pill.querySelector("button").click();
  assert.ok(!document.querySelector("#endeavor-reply-pill"), "the pill gives way to the prompt");
  assert.equal(document.querySelector("#endeavor-reply .source").textContent, "rates · lines 2–3");
  assert.equal(document.querySelector("#endeavor-reply .quote").textContent, "rows = rand(1:n, n) fit(rows)");
  document.querySelector("#endeavor-reply textarea").value = "Should this sample with replacement?";
  enter(false);
  assert.deepEqual(sent.at(-1), {
    type: "quote",
    notebook: NB,
    picks: [{ part: "lines", cell: CELL, code: "rates = map(1:1000) do _\n  rows = rand(1:n, n)\n  fit(rows)\nend", lines: [2, 3], text: "  rows = rand(1:n, n)\n  fit(rows)" }],
    comment: "Should this sample with replacement?",
    add: false,
  });
  assert.ok(!document.querySelector("#endeavor-reply"), "sending closes the prompt");
});

test("⌘J on output text opens the prompt; ⌘↩ adds the quote to the message", async () => {
  const { window, document, sent, select, enter } = await page();
  const text = document.querySelector("pluto-output p").firstChild;
  await select(text, 4, text, 14);
  window.dispatchEvent(new window.KeyboardEvent("keydown", { key: "j", metaKey: true }));
  assert.equal(document.querySelector("#endeavor-reply .source").textContent, "rates · output");
  document.querySelector("#endeavor-reply textarea").value = "why 95?";
  enter(true);
  assert.deepEqual(sent.at(-1), {
    type: "quote",
    notebook: NB,
    picks: [{ part: "output", cell: CELL, code: "rates = map(1:1000) do _\n  rows = rand(1:n, n)\n  fit(rows)\nend", text: "middle 95%" }],
    comment: "why 95?",
    add: true,
  });
  assert.equal(document.querySelector("#endeavor-reply").textContent, "Added to the message");
  assert.equal(window.getSelection().rangeCount, 0, "the selection clears");
});

test("the prompt's menu sends or adds; Esc closes it", async () => {
  const { window, document, sent, select } = await page();
  const text = document.querySelector("pluto-output p").firstChild;
  await select(text, 4, text, 14);
  document.querySelector("#endeavor-reply-pill button").click();
  const menu = document.querySelector("#endeavor-reply [role=menu]");
  assert.ok(menu.hidden);
  document.querySelector("#endeavor-reply .options").click();
  assert.deepEqual([...menu.querySelectorAll("[role=menuitem]")].map((r) => r.textContent), ["Send reply↩", "Add to message⌘↩"]);
  menu.querySelector("[data-add=true]").click();
  assert.equal(sent.at(-1).add, true);

  await select(text, 4, text, 14);
  document.querySelector("#endeavor-reply-pill button").click();
  document.querySelector("#endeavor-reply textarea").dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  assert.ok(!document.querySelector("#endeavor-reply"));
  assert.equal(window.getSelection().toString(), "middle 95%", "Esc keeps the selection");
});

const { outputFiles } = buildSync({ entryPoints: [new URL("../src/place.ts", import.meta.url).pathname], bundle: true, format: "esm", write: false });
const place = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString("base64")}`);

test("the pill sits under the selection's end, or above its start with no room below", () => {
  const lines = [
    { left: 100, top: 200, right: 400, bottom: 220 },
    { left: 20, top: 220, right: 180, bottom: 240 },
  ];
  assert.deepEqual(place.pillPlace(lines, 800), { left: 152, top: 246 });
  assert.deepEqual(place.pillPlace(lines, 270), { left: 152, top: 164 }, "36 px below doesn't fit in 270");
  assert.deepEqual(place.pillPlace([{ left: 0, top: 10, right: 20, bottom: 30 }], 800), { left: 4, top: 36 }, "kept off the left edge");
});
