// Reply on a selection in the notebook: the pill, the prompt, and what the app gets.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { buildSync } from "esbuild";

const NB = "0f381e2e-b8ca-11f1-b549-49cf0ce82801";
const CELL = "11111111-1111-1111-1111-111111111111";

async function page(platform = "MacIntel") {
  const dom = new JSDOM(
    `<body><pluto-cell id="${CELL}"><pluto-output><p>The middle 95% of the rates.</p></pluto-output>` +
      `<pluto-input><div class="cm-content"><div class="cm-line">rates = map(1:1000) do _</div>` +
      `<div class="cm-line">  rows = rand(1:n, n)</div><div class="cm-line">  fit(rows)</div><div class="cm-line">end</div></div></pluto-input></pluto-cell></body>`,
    { url: `http://localhost/edit?id=${NB}`, runScripts: "outside-only", pretendToBeVisual: true },
  );
  const { window } = dom;
  Object.defineProperty(window.navigator, "platform", { value: platform });
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
    window.document.querySelector("#endeavor-ask textarea").dispatchEvent(new window.KeyboardEvent("keydown", { key: "Enter", metaKey: meta, bubbles: true }));
  return { window, document: window.document, sent, select, enter };
}

test("a selection in code offers Reply, which sends the lines and the reply", async () => {
  const { document, sent, select, enter } = await page();
  const lines = document.querySelectorAll(".cm-line");
  await select(lines[1].firstChild, 2, lines[2].firstChild, 11);
  const pill = document.querySelector("#endeavor-reply-pill");
  assert.equal(pill?.textContent, "Reply ⌘E");
  pill.querySelector("button").click();
  assert.ok(!document.querySelector("#endeavor-reply-pill"), "the pill gives way to the prompt");
  assert.equal(document.querySelector("#endeavor-ask .what").textContent, "rates·lines 2–3");
  assert.equal(document.querySelector("#endeavor-ask .quote").textContent, "rows = rand(1:n, n) fit(rows)");
  assert.equal(document.querySelector("#endeavor-ask textarea").placeholder, "Ask Claude about these lines");
  document.querySelector("#endeavor-ask textarea").value = "Should this sample with replacement?";
  enter(false);
  assert.deepEqual(sent.at(-1), {
    type: "quote",
    notebook: NB,
    picks: [{ part: "lines", cell: CELL, code: "rates = map(1:1000) do _\n  rows = rand(1:n, n)\n  fit(rows)\nend", lines: [2, 3], text: "  rows = rand(1:n, n)\n  fit(rows)" }],
    comment: "Should this sample with replacement?",
    add: false,
  });
  assert.ok(!document.querySelector("#endeavor-ask"), "sending closes the prompt");
});

for (const key of ["e", "j"]) {
test(`⌘${key.toUpperCase()} on output text opens the prompt; ⌘↩ adds the quote to the message`, async () => {
  const { window, document, sent, select, enter } = await page();
  const text = document.querySelector("pluto-output p").firstChild;
  await select(text, 4, text, 14);
  window.dispatchEvent(new window.KeyboardEvent("keydown", { key, metaKey: true }));
  assert.equal(document.querySelector("#endeavor-ask .what").textContent, "rates·output");
  document.querySelector("#endeavor-ask textarea").value = "why 95?";
  enter(true);
  assert.deepEqual(sent.at(-1), {
    type: "quote",
    notebook: NB,
    picks: [{ part: "output", cell: CELL, code: "rates = map(1:1000) do _\n  rows = rand(1:n, n)\n  fit(rows)\nend", text: "middle 95%" }],
    comment: "why 95?",
    add: true,
  });
  assert.equal(document.querySelector("#endeavor-ask"), null, "the prompt closes; the card above the composer shows it was added");
  assert.equal(window.getSelection().rangeCount, 0, "the selection clears");
});
}

test("off macOS, Ctrl+E opens the prompt and its keys are spelled out", async () => {
  const { window, document, select } = await page("Linux x86_64");
  const text = document.querySelector("pluto-output p").firstChild;
  await select(text, 4, text, 14);
  assert.equal(document.querySelector("#endeavor-reply-pill").textContent, "Reply Ctrl+E");
  window.dispatchEvent(new window.KeyboardEvent("keydown", { key: "e", metaKey: true }));
  assert.equal(document.querySelector("#endeavor-ask"), null, "⌘ does nothing off macOS");
  window.dispatchEvent(new window.KeyboardEvent("keydown", { key: "e", ctrlKey: true }));
  assert.equal(document.querySelector("#endeavor-ask .keys .idle").parentElement.textContent, "Enter sendEnter queue · Ctrl+Enter add to message");
  assert.equal(document.querySelector("#endeavor-ask [data-add=true] .key").textContent, "Ctrl+Enter");
  assert.equal(document.querySelector("#endeavor-ask [data-add=false] .key").textContent, "Enter");
});

test("the prompt's menu sends or adds; Esc closes it", async () => {
  const { window, document, sent, select } = await page();
  const text = document.querySelector("pluto-output p").firstChild;
  await select(text, 4, text, 14);
  document.querySelector("#endeavor-reply-pill button").click();
  const menu = document.querySelector("#endeavor-ask [role=menu]");
  assert.ok(menu.hidden);
  document.querySelector("#endeavor-ask .options").click();
  assert.deepEqual([...menu.querySelectorAll("[role=menuitem]")].map((r) => r.textContent), ["Send nowSend after this turn↩", "Add to message⌘↩"]);
  menu.querySelector("[data-add=true]").click();
  assert.equal(sent.at(-1).add, true);

  await select(text, 4, text, 14);
  document.querySelector("#endeavor-reply-pill button").click();
  document.querySelector("#endeavor-ask textarea").dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  assert.ok(!document.querySelector("#endeavor-ask"));
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
