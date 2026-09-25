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
    type: "annotation",
    notebook: "0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    cells: ["22222222-2222-2222-2222-222222222222"],
    comment: "why is this slow?",
    now: false,
  });

  window.__endeavor.receive({ type: "annotate", on: false });
  assert.ok(!window.document.body.classList.contains("annotating"));
});
