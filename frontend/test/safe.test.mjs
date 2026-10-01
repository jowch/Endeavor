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
