// The safe-preview callout at the top of the notebook (src/safe.ts): its own
// words, or, for a notebook Julia stopped under twice, why it opened that way.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("after a second stop the callout says why the notebook is open without running", async () => {
  const dom = new JSDOM(`<body><pluto-editor><main><pluto-notebook></pluto-notebook></main></pluto-editor></body>`, {
    url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
  });
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  // Frames run when the test says, so nothing is left running after it.
  let frames = [];
  window.requestAnimationFrame = (callback) => frames.push(callback);
  const frame = async () => {
    await new Promise((done) => setTimeout(done, 300));
    const due = frames;
    frames = [];
    due.forEach((callback) => callback());
  };
  window.editor_state = {
    notebook: { process_status: "waiting_for_permission", cell_order: [], cell_inputs: {}, cell_results: {}, cell_dependencies: {}, status_tree: null, nbpkg: null },
  };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  window.document.documentElement.dataset.endeavorLook = "endeavor";
  const callout = () => window.document.getElementById("endeavor-safe");
  for (let i = 0; i < 5 && !callout()?.classList.contains("shown"); i++) await frame();
  assert.ok(callout().classList.contains("shown"));
  assert.equal(callout().querySelector("b").textContent, "Safe preview");

  window.__endeavor.receive({
    type: "context",
    host: "This Mac",
    asking: false,
    readonly: false,
    crash: { title: "Julia stopped again while running this notebook", body: "So it's open without running. `rates` was running both times; check it, then run the notebook." },
  });
  assert.equal(callout().querySelector("b").textContent, "Julia stopped again while running this notebook");
  assert.equal(callout().querySelector("code").textContent, "rates");
  assert.match(callout().querySelector(".text").textContent, /was running both times; check it, then run the notebook\./);

  callout().querySelector(".run").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  assert.deepEqual(sent.filter((m) => m.type === "run_notebook").map((m) => m.notebook), ["0f381e2e-b8ca-11f1-b549-49cf0ce82801"]);
});

test("on Ember's page the callout says what running would install or restart, as Ember's banner does", async () => {
  const dom = new JSDOM(`<body><pluto-editor><main><pluto-notebook></pluto-notebook></main></pluto-editor></body>`, {
    url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
  });
  const { window } = dom;
  window.ipc = { postMessage: () => {} };
  let frames = [];
  window.requestAnimationFrame = (callback) => frames.push(callback);
  const frame = async () => {
    await new Promise((done) => setTimeout(done, 300));
    const due = frames;
    frames = [];
    due.forEach((callback) => callback());
  };
  const notebook = (plan) => ({ process_status: "waiting_for_permission", cell_order: [], cell_inputs: {}, cell_results: {}, cell_dependencies: {}, status_tree: null, nbpkg: null, ember: { plan } });
  window.editor_state = { notebook: notebook({ install: 3, restart: [] }) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  window.document.documentElement.dataset.endeavorLook = "endeavor";
  const text = () => window.document.querySelector("#endeavor-safe .text")?.textContent ?? "";
  for (let i = 0; i < 5 && !text().includes("installs"); i++) await frame();
  assert.match(text(), /without running any code\. Running starts R and installs 3 packages\.$/);

  window.editor_state = { notebook: notebook({ install: 1, restart: [] }) };
  for (let i = 0; i < 5 && !text().endsWith("1 package."); i++) await frame();
  assert.match(text(), /installs 1 package\.$/);

  window.editor_state = { notebook: notebook({ install: 0, restart: ["dplyr", "ggplot2"] }) };
  for (let i = 0; i < 5 && !text().includes("restart"); i++) await frame();
  assert.match(text(), /Running starts R; it will then restart to use a new version of: dplyr, ggplot2\.$/);

  window.editor_state = { notebook: notebook({ install: 0, restart: [] }) };
  for (let i = 0; i < 5 && text().includes("Running starts"); i++) await frame();
  assert.equal(text(), "Safe previewYou're reading and editing this file without running any code.", "nothing to install or restart: Endeavor's words only");
});
