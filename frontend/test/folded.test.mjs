import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

const A = "11111111-1111-1111-1111-111111111111";
const tick = () => new Promise((r) => setTimeout(r, 0));

async function page({ reduceMotion = true } = {}) {
  const dom = new JSDOM(
    `<body><pluto-editor><pluto-cell id="${A}" class="code_folded"><pluto-shoulder><button class="foldcode"><span></span></button></pluto-shoulder>` +
      `<pluto-trafficlight></pluto-trafficlight><pluto-output></pluto-output><pluto-input></pluto-input></pluto-cell></pluto-editor></body>`,
    { url: "http://localhost/edit?id=x", runScripts: "outside-only" },
  );
  const { window } = dom;
  window.ipc = { postMessage: () => {} };
  window.requestAnimationFrame = () => 0;
  window.matchMedia = (q) => ({ matches: reduceMotion && q.includes("reduce"), addEventListener() {} });
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const cells = (state) => window.__endeavor.receive({ type: "cells", cells: [{ cell_id: A, running: false, errored: false, unrun: false, author: "agent", version: "v1", ...state }] });
  const cell = window.document.getElementById(A);
  const tag = () => cell.querySelector(":scope > .endeavor-fold-tag");
  return { window, cell, cells, tag, open: () => cell.getAttribute("data-endeavor-fold") };
}

test("Claude's change to a folded cell shows until it runs cleanly, then folds", async () => {
  const { cell, cells, tag, open } = await page();
  cells({ unrun: true });
  assert.equal(open(), "open");
  assert.equal(tag().querySelector(".label").textContent, "Folded · shows until it runs");
  assert.equal(tag().querySelector(".now").textContent, "Fold now");

  cells({ unrun: true, running: true });
  assert.equal(open(), "open");
  cells({ errored: true });
  assert.equal(open(), "open");
  assert.equal(tag().querySelector(".label").textContent, "Folded · shows until it runs without error");
  cells({});
  assert.equal(open(), null);
  assert.equal(tag(), null);
  cell.classList.remove("code_folded");
  cells({ unrun: true, version: "v2" });
  assert.equal(open(), null);
});

test("a clean run folds the code over 200 ms", async () => {
  const { cell, cells, open } = await page({ reduceMotion: false });
  cells({ unrun: true });
  cells({});
  assert.equal(open(), "closing");
  assert.equal(cell.querySelector("pluto-input").style.height, "0px");
  await new Promise((r) => setTimeout(r, 250));
  assert.equal(open(), null);
  assert.equal(cell.querySelector("pluto-input").style.height, "");
});

test("Fold now, or Pluto's eye, folds it at once until Claude edits it again", async () => {
  const { window, cell, cells, tag, open } = await page();
  cells({ unrun: true });
  tag().click();
  assert.equal(open(), null);
  cells({ unrun: true });
  window.document.body.append(window.document.createElement("div"));
  await tick();
  assert.equal(open(), null);
  cells({ unrun: true, version: "v2" });
  assert.equal(open(), "open");
  let pluto = false;
  cell.querySelector("button.foldcode").addEventListener("click", () => (pluto = true));
  cell.querySelector("button.foldcode").click();
  assert.equal(open(), null);
  assert.equal(pluto, false);
  cell.querySelector("button.foldcode").click();
  assert.equal(pluto, true);
});
