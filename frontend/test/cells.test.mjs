// Cell states from the app mark Pluto's cells, and survive Pluto redrawing them.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("unrun and author marks follow the app and survive redraws", async () => {
  const A = "11111111-1111-1111-1111-111111111111", B = "22222222-2222-2222-2222-222222222222";
  const dom = new JSDOM(`<body><pluto-notebook><pluto-cell id="${A}"></pluto-cell><pluto-cell id="${B}"></pluto-cell></pluto-notebook></body>`, {
    url: "http://localhost/edit?id=x",
    runScripts: "outside-only",
  });
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  assert.deepEqual(sent[0], { type: "ready" });

  const cell = (id) => window.document.getElementById(id);
  window.__endeavor.receive({
    type: "cells",
    cells: [
      { cell_id: A, running: false, errored: false, unrun: true, author: "agent" },
      { cell_id: B, running: false, errored: false, unrun: false, author: "user" },
    ],
  });
  assert.equal(cell(A).getAttribute("data-endeavor"), "unrun");
  assert.equal(cell(A).getAttribute("data-author"), "agent");
  assert.equal(cell(B).getAttribute("data-endeavor"), null);
  assert.equal(cell(B).getAttribute("data-author"), "user");

  // The overview rail has a mark for the unrun cell only; clicking scrolls to it.
  const marks = [...window.document.querySelectorAll("#endeavor-rail a")];
  assert.deepEqual(marks.map((m) => m.dataset.cell), [A]);
  let scrolled = null;
  cell(A).scrollIntoView = () => (scrolled = A);
  marks[0].click();
  assert.equal(scrolled, A);

  // Pluto replaces a cell's element: the mark comes back.
  const fresh = window.document.createElement("pluto-cell");
  fresh.id = A;
  cell(A).replaceWith(fresh);
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(fresh.getAttribute("data-endeavor"), "unrun");

  // Run it: the app sends the new state and the mark goes.
  window.__endeavor.receive({ type: "cells", cells: [{ cell_id: A, running: false, errored: false, unrun: false, author: "agent" }] });
  assert.equal(cell(A).getAttribute("data-endeavor"), null);
});
