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
  assert.deepEqual(marks.map((m) => [m.dataset.cell, m.dataset.state]), [[A, "claude"], [B, "ran"], [C, "ran"]], "one mark per cell, by its run state");
  let scrolled = null;
  cell(A).scrollIntoView = () => (scrolled = A);
  marks[0].dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
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
  assert.deepEqual([...doc.querySelectorAll("#endeavor-rail a")].map((m) => m.dataset.state), ["ran", "ran", "ran"]);
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
  assert.deepEqual([...doc.querySelectorAll("#endeavor-rail a")].map((m) => [m.dataset.cell, m.dataset.state]), [[A, "claude"], [B, "pending"], [C, "pending"]]);

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

test("the rail's states: running, not run yet, failed and ran, with Ember's own rail colour and not-run marks", async () => {
  const { window, doc } = await page();
  const cell = (id) => doc.getElementById(id);
  const states = () => [...doc.querySelectorAll("#endeavor-rail a")].map((m) => m.dataset.state);
  const redraw = async () => {
    doc.body.append(doc.createElement("div"));
    await new Promise((r) => setTimeout(r, 0));
  };
  cell(A).classList.add("running");
  cell(B).classList.add("queued");
  cell(C).classList.add("errored");
  await redraw();
  assert.deepEqual(states(), ["running", "pending", "failed"]);
  cell(A).classList.remove("running");
  cell(B).classList.replace("queued", "not_run_yet");
  cell(C).classList.add("code_differs");
  await redraw();
  assert.deepEqual(states(), ["ran", "pending", "pending"], "an edited cell with an old error is waiting to run, as Ember's rail says");
  cell(B).classList.remove("not_run_yet");
  cell(B).setAttribute("data-rail", "due");
  cell(C).classList.remove("code_differs", "errored");
  cell(C).classList.add("stale");
  await redraw();
  assert.deepEqual(states(), ["ran", "pending", "pending"]);
  window.__endeavor.receive(cellsMsg([{ cell_id: A, unrun: true, author: "agent" }, { cell_id: B }, { cell_id: C }]));
  await redraw();
  assert.equal(states()[0], "claude", "Claude's edit, not run");
  cell(A).classList.add("running");
  await redraw();
  assert.equal(states()[0], "running", "Claude's cell pulses while it runs, like any other");
});

test("the rail shows every cell as not run yet in safe preview, whatever its saved state", async () => {
  const { doc } = await page();
  const states = () => [...doc.querySelectorAll("#endeavor-rail a")].map((m) => m.dataset.state);
  const editor = doc.querySelector("pluto-editor") ?? doc.body.appendChild(doc.createElement("pluto-editor"));
  doc.getElementById(C).classList.add("errored");
  editor.classList.add("process_waiting_for_permission");
  doc.body.append(doc.createElement("div"));
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(states(), ["pending", "pending", "pending"]);
  editor.classList.remove("process_waiting_for_permission");
  doc.body.append(doc.createElement("div"));
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(states(), ["ran", "ran", "failed"]);
});

test("the rail sits left of a drawer beside the cells, and ends above one below them", async () => {
  const { window, doc } = await page();
  const root = doc.documentElement;
  Object.defineProperty(root, "clientWidth", { value: 1300, configurable: true });
  Object.defineProperty(window, "innerWidth", { value: 1300, configurable: true });
  Object.defineProperty(window, "innerHeight", { value: 900, configurable: true });
  const drawer = doc.getElementById("endeavor-drawer");
  Object.defineProperty(drawer, "offsetHeight", { value: 1, configurable: true });
  const rail = doc.getElementById("endeavor-rail");
  const at = async (rect) => {
    drawer.getBoundingClientRect = () => ({ ...rect, right: rect.left + rect.width, bottom: rect.top + rect.height, x: rect.left, y: rect.top });
    root.dataset.endeavorDrawer = "packages";
    window.dispatchEvent(new window.Event("resize"));
    await new Promise((r) => setTimeout(r, 0));
    return [rail.style.left, rail.style.top, rail.style.height];
  };
  assert.deepEqual(await at({ left: 800, top: 0, width: 500, height: 900 }), [`${800 - 2 - 14}px`, "10px", "880px"], "beside: 2px clear of the drawer's edge, full height");
  assert.deepEqual(await at({ left: 0, top: 600, width: 1300, height: 300 }), [`${1300 - 12 - 14}px`, "10px", "580px"], "below: at the scroll bar, ending 10px above the drawer");
});

test("the rail's tip names a cell that ran, and adds the state only when it's out of the ordinary", async () => {
  const { window, doc } = await page();
  const rail = doc.getElementById("endeavor-rail");
  const tip = doc.getElementById("endeavor-rail-tip");
  const hoverFirst = async () => {
    doc.body.append(doc.createElement("div"));
    await new Promise((r) => setTimeout(r, 0));
    rail.dispatchEvent(new window.MouseEvent("pointermove", { clientY: 1, bubbles: true }));
    return [tip.textContent, tip.dataset.state, tip.querySelector(".state")?.textContent ?? null];
  };
  assert.deepEqual(await hoverFirst(), ["cell", "ran", null]);
  doc.getElementById(A).classList.add("errored");
  assert.deepEqual(await hoverFirst(), ["cellerror", "failed", "error"]);
});
