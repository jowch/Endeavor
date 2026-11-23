import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

const A = "11111111-1111-1111-1111-111111111111",
  B = "22222222-2222-2222-2222-222222222222",
  C = "33333333-3333-3333-3333-333333333333";

async function page() {
  const cells = [A, B, C].map((id) => `<pluto-cell id="${id}"><pluto-trafficlight></pluto-trafficlight></pluto-cell>`).join("");
  const dom = new JSDOM(`<body><pluto-editor><pluto-notebook>${cells}</pluto-notebook></pluto-editor></body>`, {
    url: "http://localhost/edit?id=x",
    runScripts: "outside-only",
  });
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.requestAnimationFrame = () => 0;
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  return { window, doc: window.document, sent };
}

function barRules(doc) {
  const rules = [];
  for (const style of doc.head.querySelectorAll("style")) {
    for (const [, selectors, body] of style.textContent.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      for (const selector of selectors.split(",").map((s) => s.trim())) {
        if (selector.includes("pluto-trafficlight")) rules.push([selector, body.replace(/\s+/g, " ").trim()]);
      }
    }
  }
  return rules;
}

function bar(doc, id) {
  const light = doc.getElementById(id).querySelector("pluto-trafficlight");
  const look = {};
  for (const [selector, body] of barRules(doc)) {
    const after = selector.endsWith("::after");
    if (!light.matches(selector.replace(/::after$/, ""))) continue;
    const background = body.match(/background: ([^;]+);/)?.[1];
    if (background) look[after ? "pattern" : "bar"] = background;
  }
  return Object.keys(look).length ? look : "Pluto's";
}

const cellsMsg = (cells) => ({ type: "cells", cells: cells.map((c) => ({ running: false, errored: false, unrun: false, author: null, ...c })) });
const context = (ask_cells) => ({ type: "context", host: "This Mac", asking: false, readonly: false, ask_cells });

test("unrun and author marks follow the app and survive redraws", async () => {
  const { window, doc } = await page();
  const cell = (id) => doc.getElementById(id);
  window.__endeavor.receive(cellsMsg([{ cell_id: A, unrun: true, author: "agent" }, { cell_id: B, author: "user" }]));
  assert.equal(cell(A).getAttribute("data-endeavor"), "unrun");
  assert.equal(cell(A).getAttribute("data-author"), "agent");
  assert.equal(cell(B).getAttribute("data-endeavor"), null);
  assert.equal(cell(B).getAttribute("data-author"), "user");

  const marks = [...doc.querySelectorAll("#endeavor-rail a")];
  assert.deepEqual(marks.map((m) => [m.dataset.cell, m.className]), [[A, ""]]);
  let scrolled = null;
  cell(A).scrollIntoView = () => (scrolled = A);
  marks[0].click();
  assert.equal(scrolled, A);

  const fresh = doc.createElement("pluto-cell");
  fresh.id = A;
  cell(A).replaceWith(fresh);
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(fresh.getAttribute("data-endeavor"), "unrun");
  assert.equal(fresh.getAttribute("data-endeavor-bar"), "claude");

  window.__endeavor.receive(cellsMsg([{ cell_id: A, author: "agent" }]));
  assert.equal(cell(A).getAttribute("data-endeavor"), null);
  assert.equal(cell(A).getAttribute("data-endeavor-bar"), null);
  assert.equal(doc.querySelectorAll("#endeavor-rail a").length, 0);
});

test("one status bar: Claude's cells in orange, queued and running in orange, the rest Pluto's own", async () => {
  const { window, doc } = await page();
  const cell = (id) => doc.getElementById(id);
  const ORANGE = { bar: "var(--e-accent)" };
  const QUEUED = {
    bar: "var(--e-stripe-tint)",
    pattern: "repeating-linear-gradient(-45deg, transparent, transparent 8px, var(--e-accent) 8px, var(--e-accent) 16px)",
  };
  const RUNNING = {
    bar: "var(--e-stripe-tint)",
    pattern: "repeating-linear-gradient(-45deg, var(--e-accent), var(--e-accent) 8px, var(--e-stripe-tint) 8px, var(--e-stripe-tint) 16px)",
  };

  window.__endeavor.receive(cellsMsg([{ cell_id: A, unrun: true, author: "agent" }, { cell_id: B, unrun: true, author: "user" }, { cell_id: C }]));
  assert.deepEqual([bar(doc, A), bar(doc, B), bar(doc, C)], [ORANGE, "Pluto's", "Pluto's"]);

  cell(A).classList.add("selected", "errored", "code_folded");
  assert.deepEqual(bar(doc, A), ORANGE);
  cell(C).classList.add("code_differs");
  doc.body.append(doc.createElement("div"));
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(bar(doc, C), "Pluto's");
  assert.deepEqual([...doc.querySelectorAll("#endeavor-rail a")].map((m) => [m.dataset.cell, m.className]), [[A, ""], [B, "user"], [C, "user"]]);

  cell(A).classList.add("queued");
  assert.deepEqual(bar(doc, A), QUEUED);
  cell(A).classList.replace("queued", "running");
  assert.deepEqual(bar(doc, A), RUNNING);

  cell(A).classList.remove("running");
  window.__endeavor.receive(cellsMsg([{ cell_id: A, errored: true, author: "agent" }]));
  assert.equal(bar(doc, A), "Pluto's");

  window.__endeavor.receive(context([C]));
  assert.deepEqual(bar(doc, C), ORANGE);
  assert.equal(cell(C).getAttribute("data-endeavor-bar"), "claude");
  window.__endeavor.receive(context([]));
  assert.deepEqual(bar(doc, C), ORANGE);
  cell(C).classList.add("running");
  await new Promise((r) => setTimeout(r, 300));
  assert.deepEqual(bar(doc, C), RUNNING);
  cell(C).classList.remove("running");
  await new Promise((r) => setTimeout(r, 300));
  assert.equal(bar(doc, C), "Pluto's");
});
