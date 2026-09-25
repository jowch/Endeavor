// Pluto error boxes get Fix with Claude / Explain, which ask the app about that cell.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("error boxes get Fix with Claude and Explain", async () => {
  const C = "33333333-3333-3333-3333-333333333333";
  const dom = new JSDOM(`<body><pluto-cell id="${C}"></pluto-cell></body>`, {
    url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
  });
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));

  // Pluto renders an error into the cell later.
  window.document.getElementById(C).innerHTML =
    `<pluto-output><jlerror><div class="error-header"><secret-h1>Error message</secret-h1></div>` +
    `<header>UndefVarError: \`lsq\` not defined</header></jlerror></pluto-output>`;
  await new Promise((r) => setTimeout(r, 0));

  const buttons = [...window.document.querySelectorAll(".endeavor-ask button")].map((b) => b.textContent);
  assert.deepEqual(buttons, ["Fix with Claude", "Explain"]);
  window.document.querySelector(".endeavor-ask .fix").click();
  assert.deepEqual(sent.at(-1), {
    type: "ask",
    kind: "fix",
    notebook: "0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    cell: C,
    error: "UndefVarError: `lsq` not defined",
  });
  // Redraws don't add a second row.
  window.document.body.append(window.document.createElement("div"));
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(window.document.querySelectorAll(".endeavor-ask").length, 1);
});
