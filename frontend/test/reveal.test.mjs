// A chip in the chat asks the page to show its cell, or for the cell's code now.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("reveal outlines the cells; code answers with a cell's code", async () => {
  const C = "44444444-4444-4444-4444-444444444444";
  const dom = new JSDOM(
    `<body><pluto-cell id="${C}"><pluto-input><div class="cm-content">` +
      `<div class="cm-line">rates = 1</div><div class="cm-line">rates + 1</div></div></pluto-input></pluto-cell></body>`,
    { url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801", runScripts: "outside-only" },
  );
  const { window } = dom;
  window.HTMLElement.prototype.scrollIntoView = () => {};
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));

  window.__endeavor.receive({ type: "reveal", cells: [C, "missing"] });
  assert.ok(window.document.getElementById(C).classList.contains("endeavor-flash"));

  window.__endeavor.receive({ type: "code", cell: C });
  assert.deepEqual(sent.at(-1), { type: "code", cell: C, code: "rates = 1\nrates + 1" });
  window.__endeavor.receive({ type: "code", cell: "missing" });
  assert.deepEqual(sent.at(-1), { type: "code", cell: "missing", code: null });
});
