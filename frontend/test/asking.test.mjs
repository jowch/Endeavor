// The chat's card asking to run cells marks them in the notebook, and the marks survive redraws.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("asked-about cells, the cells that re-run after them, and the cells it needs that never ran are marked", async () => {
  const A = "11111111-1111-1111-1111-111111111111",
    B = "22222222-2222-2222-2222-222222222222",
    C = "33333333-3333-3333-3333-333333333333",
    D = "44444444-4444-4444-4444-444444444444";
  const dom = new JSDOM(
    `<body><pluto-notebook><pluto-cell id="${A}"></pluto-cell><pluto-cell id="${B}"></pluto-cell><pluto-cell id="${C}"></pluto-cell><pluto-cell id="${D}"></pluto-cell></pluto-notebook></body>`,
    {
      url: "http://localhost/edit?id=x",
      runScripts: "outside-only",
    },
  );
  const { window } = dom;
  window.ipc = { postMessage: () => {} };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const mark = (id) => window.document.getElementById(id).getAttribute("data-endeavor-ask");

  window.__endeavor.receive({ type: "context", host: "This Mac", asking: false, readonly: false, ask_cells: [A], rerun_cells: [B], needed_ids: [D] });
  assert.deepEqual([mark(A), mark(B), mark(C), mark(D)], ["asks", "reruns", null, "needed"]);

  // Pluto redraws a cell: the mark comes back.
  const fresh = window.document.createElement("pluto-cell");
  fresh.id = A;
  window.document.getElementById(A).replaceWith(fresh);
  await new Promise((done) => setTimeout(done, 0));
  assert.equal(mark(A), "asks");

  // The card is answered: the marks go.
  window.__endeavor.receive({ type: "context", host: "This Mac", asking: false, readonly: false, ask_cells: [], rerun_cells: [], needed_ids: [] });
  assert.deepEqual([mark(A), mark(B), mark(C), mark(D)], [null, null, null, null]);
});
