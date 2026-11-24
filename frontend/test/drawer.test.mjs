import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

async function page(notebook) {
  const dom = new JSDOM(`<body><pluto-editor></pluto-editor></body>`, {
    url: "http://localhost/ember/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
  });
  const { window } = dom;
  let frames = [];
  window.requestAnimationFrame = (callback) => frames.push(callback);
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
  window.document.documentElement.dataset.endeavorLook = "endeavor";
  window.editor_state = { notebook, connected: true };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  window.__endeavor.receive({ type: "context", host: "This Mac", agent: "Claude", asking: false });
  const settle = async () => {
    for (let i = 0; i < 4; i++) {
      await new Promise((done) => setTimeout(done, 300));
      const due = frames;
      frames = [];
      due.forEach((callback) => callback());
    }
  };
  await settle();
  return { window, sent, settle, text: () => window.document.querySelector("#endeavor-drawer .packages").textContent };
}

const rNotebook = (library, deseq) => ({
  process_status: "ready",
  status_tree: null,
  nbpkg: null,
  ember: {
    packages: {
      snapshot: "2026-10-01",
      r_version: "4.4.1",
      library,
      rows: [
        { name: "DESeq2", version: "1.46.0", direct: true, status: deseq },
        { name: "ggplot2", version: "3.5.1", direct: true, status: "installed" },
        { name: "S4Vectors", version: "0.44.0", direct: false, status: "installed" },
      ],
    },
  },
  cell_order: ["a"],
  cell_inputs: { a: { code: "library(DESeq2)" } },
  cell_results: {},
});

test("an R notebook's first install opens Packages by itself and tells the app it's installing; it closes once done", async () => {
  const p = await page(rNotebook({ status: "installing", progress: { done: 12, total: 40 } }, "installing"));
  assert.equal(p.window.document.documentElement.dataset.endeavorDrawer, "packages");
  assert.match(p.text(), /^Installing packages · 12 of 40/);
  assert.match(p.text(), /R 4\.4\.1 · CRAN snapshot 2026-10-01 · This Mac/);
  assert.match(p.text(), /DESeq2installing/);
  const state = p.sent.filter((m) => m.type === "state").at(-1);
  assert.equal(state.installing, true);
  assert.equal(state.busy, "Installing packages · 12 of 40");
  assert.equal(state.drawer, "packages");

  p.window.editor_state = { notebook: rNotebook({ status: "ready" }, "installed"), connected: true };
  await p.settle();
  assert.equal(p.window.document.documentElement.dataset.endeavorDrawer, undefined);
  p.window.__endeavor.receive({ type: "drawer", tab: "packages" });
  await p.settle();
  assert.match(p.text(), /^Ready/);
  assert.match(p.text(), /2 packages and 1 dependency, all ready/);
  assert.match(p.text(), /DESeq21\.46\.0/);
  assert.doesNotMatch(p.text(), /Start Julia|Cells/);
});

test("each new install opens Packages again, but one the user closed stays closed until that install ends", async () => {
  const p = await page(rNotebook({ status: "installing" }, "installing"));
  const drawer = () => p.window.document.documentElement.dataset.endeavorDrawer;
  const install = async (status, deseq) => {
    p.window.editor_state = { notebook: rNotebook({ status }, deseq), connected: true };
    await p.settle();
  };
  assert.equal(drawer(), "packages");
  assert.ok(p.window.document.querySelector("#endeavor-drawer .progress.unknown"), "with no count from Ember, the bar runs instead of sitting at 0");
  assert.match(p.text(), /^Installing packages(?! ·)/);
  p.window.__endeavor.receive({ type: "drawer", tab: null });
  await install("installing", "installing");
  assert.equal(drawer(), undefined, "closed by the user during this install");
  await install("ready", "installed");
  await install("installing", "not_installed");
  assert.equal(drawer(), "packages", "a later install opens it again");
});
