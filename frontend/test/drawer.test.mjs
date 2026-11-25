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

test("the drawer sits at the right in a pane wide enough for the cells and 500px beside them, and at the bottom otherwise", async () => {
  const p = await page(rNotebook({ status: "ready" }, "installed"));
  const root = p.window.document.documentElement;
  const resize = (width) => {
    Object.defineProperty(p.window, "innerWidth", { value: width, configurable: true });
    p.window.dispatchEvent(new p.window.Event("resize"));
  };
  resize(840);
  assert.equal("endeavorDrawerSide" in root.dataset, false);
  resize(1300);
  assert.equal("endeavorDrawerSide" in root.dataset, true);
  resize(1299);
  assert.equal("endeavorDrawerSide" in root.dataset, false);
});

async function marimoPage() {
  const dom = new JSDOM(`<body><div id="App"></div></body>`, { url: "http://localhost/marimo/?file=growth.py", runScripts: "outside-only" });
  const { window } = dom;
  window.requestAnimationFrame = () => 0;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
  window.WebSocket = class extends window.EventTarget {
    static CLOSING = 2;
  };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  window.__endeavor.receive({ type: "context", host: "This Mac", agent: "Claude", asking: false, notebook: "0f381e2e-b8ca-11f1-b549-49cf0ce82801" });
  const settle = () => new Promise((done) => setTimeout(done, 50));
  const socket = new window.WebSocket("ws://localhost/marimo/ws?file=growth.py");
  const marimoSays = async (op, data) => {
    socket.dispatchEvent(new window.MessageEvent("message", { data: `{"op": "${op}", "data": ${JSON.stringify({ op, ...data })}}` }));
    await settle();
  };
  const environment = (install, failed = null) => ({
    python: ">=3.10", python_version: "3.12.8", package_date: "2026-10-05", install, failed,
    packages: [{ name: "boto3", version: install ? null : "1.43.108" }, { name: "marimo", version: "0.25.1" }],
  });
  const packages = async (install, failed) => {
    window.__endeavor.receive({ type: "packages", environment: environment(install, failed) });
    await settle();
  };
  return {
    window, sent, settle, packages, marimoSays,
    drawer: () => window.document.documentElement.dataset.endeavorDrawer,
    text: () => window.document.querySelector("#endeavor-drawer .packages").textContent,
    state: () => sent.filter((m) => m.type === "state").at(-1),
  };
}

test("a Python notebook's first install opens Packages by itself, tells the app, and closes once done", async () => {
  const p = await marimoPage();
  await p.packages({ step: "installing", packages: ["boto3"] });
  assert.equal(p.drawer(), "packages");
  assert.match(p.text(), /^Installing packages/);
  assert.match(p.text(), /Python 3\.12\.8 · uv · PyPI as of 2026-10-05 · This Mac/);
  assert.match(p.text(), /boto3installing/);
  assert.deepEqual(
    [p.state().notebook, p.state().busy, p.state().installing, p.state().drawer],
    ["0f381e2e-b8ca-11f1-b549-49cf0ce82801", "Installing packages", true, "packages"],
  );
  await p.packages(null);
  assert.equal(p.drawer(), undefined);
  assert.equal(p.state().drawer, null);
  assert.equal(p.state().installing, false);
  p.window.__endeavor.receive({ type: "drawer", tab: "packages" });
  await p.settle();
  assert.match(p.text(), /^Ready/);
  assert.match(p.text(), /2 packages, all ready/);
  assert.match(p.text(), /boto31\.43\.108/);
});

test("a Python notebook's failed install shows the failure box and the header's Package failed", async () => {
  const p = await marimoPage();
  const log = "  ╰─▶ Because endeavor-e2e-absent was not found in the package registry\nfailed to install endeavor_e2e_absent\n";
  await p.packages(null, { packages: ["endeavor_e2e_absent"], log });
  assert.equal(p.drawer(), "packages", "a failure opens Packages by itself, as an install does");
  assert.match(p.text(), /^Package failed · endeavor_e2e_absent/);
  assert.match(p.text(), /endeavor_e2e_absent couldn't be installed: it isn't in the package index/);
  assert.equal(p.state().package_failed, "endeavor_e2e_absent");
  p.window.document.querySelector("#endeavor-drawer .failure .fix").dispatchEvent(new p.window.MouseEvent("click", { bubbles: true }));
  const fix = p.sent.find((m) => m.type === "fix_package");
  assert.deepEqual([fix?.notebook, fix?.name], ["0f381e2e-b8ca-11f1-b549-49cf0ce82801", "endeavor_e2e_absent"]);
});

const operation = (status, packages, logs = {}, log_mode = "append") => ({
  operation_id: "2cc80a17", action: "install", source: "kernel", status, packages, logs, log_mode,
});

test("an install marimo runs itself, from its own popup or for Endeavor's card, shows in Packages and the header however short it is", async () => {
  const p = await marimoPage();
  await p.packages(null);
  const popup = p.window.document.createElement("div");
  popup.className = "flex flex-col gap-4 mb-5 fixed top-5 left-12";
  popup.innerHTML = "<div><span>Missing packages</span><button>Install</button></div>";
  p.window.document.body.append(popup);
  await p.marimoSays("environment-operation", operation({ kind: "running" }, { humanize: "running" }));
  assert.equal(p.drawer(), "packages");
  assert.match(p.text(), /^Installing packages/);
  assert.match(p.text(), /humanizeinstalling/);
  assert.deepEqual([p.state().installing, p.state().busy], [true, "Installing packages"]);
  popup.innerHTML = "<div><svg></svg><span>Installing packages</span></div>";
  await p.settle();
  assert.equal(popup.style.display, "none", "marimo's own progress popup would only repeat Packages");
  await p.marimoSays("environment-operation", operation({ kind: "succeeded" }, { humanize: "succeeded" }));
  assert.equal(p.state().installing, false, "the app asks for the new package list when this turns false");
  assert.equal(p.drawer(), undefined, "it closes by itself once done");
});

test("marimo's missing-packages popup stays, since its Install button is the user's own way in", async () => {
  const p = await marimoPage();
  const popup = p.window.document.createElement("div");
  popup.className = "fixed top-5 left-12";
  popup.innerHTML = "<div><span>Missing packages</span><button>Install</button></div>";
  p.window.document.body.append(popup);
  await p.settle();
  assert.equal(popup.style.display, "");
});

test("an install marimo runs itself that fails shows uv's reason, and stays until a later install of it succeeds", async () => {
  const p = await marimoPage();
  await p.packages(null);
  await p.marimoSays("environment-operation", operation({ kind: "running" }, { humanize: "running" }, { humanize: "Installing humanize...\n" }, "replace"));
  const uv = "× No solution found when resolving dependencies:\n  ╰─▶ Because the requested Python version (>=3.10) does not satisfy\n      Python>=3.11 and humanize==4.17.0 depends on Python>=3.11, we can\n      conclude that humanize==4.17.0 cannot be used.\nFailed to install humanize\n";
  await p.marimoSays("environment-operation", operation({ kind: "running" }, { humanize: "failed" }, { humanize: uv }));
  await p.marimoSays("environment-operation", operation({ kind: "failed", error: "Could not apply changes to humanize." }, { humanize: "failed" }));
  assert.equal(p.drawer(), "packages");
  assert.match(p.text(), /^Package failed · humanize/);
  assert.match(p.text(), /humanize couldn't be installed: the requested Python version \(>=3\.10\) does not satisfy Python>=3\.11 and humanize==4\.17\.0 depends on Python>=3\.11/);
  assert.equal(p.state().package_failed, "humanize");
  await p.packages(null);
  assert.equal(p.state().package_failed, "humanize", "the runtime didn't run this install, so its list doesn't clear it");
  await p.marimoSays("environment-operation", { ...operation({ kind: "succeeded" }, { humanize: "succeeded" }), operation_id: "later" });
  assert.equal(p.state().package_failed, null);
});

test("after marimo's own install has settled, a later install or failure the runtime reports still shows", async () => {
  const p = await marimoPage();
  await p.packages(null);
  await p.marimoSays("environment-operation", operation({ kind: "running" }, { humanize: "running" }));
  await p.marimoSays("environment-operation", operation({ kind: "failed", error: "Could not apply changes to humanize." }, { humanize: "failed" }));
  assert.equal(p.state().package_failed, "humanize");
  await p.packages({ step: "installing", packages: ["polars"] });
  assert.deepEqual([p.state().installing, p.state().busy, p.state().package_failed], [true, "Installing packages", null]);
  assert.match(p.text(), /polarsinstalling/);
  await p.packages(null, { packages: ["polars"], log: "  ╰─▶ Because polars was not found in the package registry\n" });
  assert.equal(p.state().package_failed, "polars, humanize", "the runtime's failure first, and marimo's earlier one still");
  assert.equal(p.drawer(), "packages");
  await p.marimoSays("environment-operation", { ...operation({ kind: "succeeded" }, { six: "succeeded" }), operation_id: "later" });
  await p.packages({ step: "installing", packages: ["boto3"] });
  assert.equal(p.state().installing, true, "after a succeeded one too");
});

test("a page that reconnects mid-install picks it up from marimo's state", async () => {
  const p = await marimoPage();
  await p.packages(null);
  await p.marimoSays("environment-state", { source: "kernel", state: { restart_required: false, operations: [
    { operation_id: "a", action: "prepare", status: { kind: "succeeded" }, packages: {}, logs: {}, source: "kernel" },
    { operation_id: "b", action: "install", status: { kind: "running" }, packages: { polars: "running" }, logs: {}, source: "kernel" },
  ] } });
  assert.equal(p.state().installing, true);
  assert.match(p.text(), /polarsinstalling/);
});

test("on a Python notebook the drawer's own tab row has no Live docs, which is marimo's own panel", async () => {
  const p = await marimoPage();
  assert.equal(p.window.document.querySelector('#endeavor-drawer > header [data-tab="docs"]'), null);
  assert.ok(p.window.document.querySelector('#endeavor-drawer > header [data-tab="packages"]'));
});

test("on marimo Packages stays at the bottom in a wide pane, where marimo's own docs panel is", async () => {
  const p = await marimoPage();
  p.window.document.documentElement.dataset.endeavorLook = "endeavor";
  Object.defineProperty(p.window, "innerWidth", { value: 1600, configurable: true });
  p.window.dispatchEvent(new p.window.Event("resize"));
  await p.packages({ step: "installing", packages: ["boto3"] });
  assert.equal(p.drawer(), "packages");
  assert.equal("endeavorDrawerSide" in p.window.document.documentElement.dataset, false);
});

test("marimo's docs panel has the drawer's grip, and a drag sets the height both drawers share, measured from above marimo's footer", async () => {
  const p = await marimoPage();
  const { document } = p.window;
  const panel = document.createElement("div");
  panel.dataset.testid = "helper";
  panel.getBoundingClientRect = () => ({ top: 400, bottom: p.window.innerHeight - 30, left: 48, right: 1000 });
  document.body.append(panel);
  const grip = document.querySelector("#endeavor-docs-grip");
  assert.ok(grip, "the docs panel gets a grip");
  grip.setPointerCapture = () => {};
  const pointer = (type, clientY) => grip.dispatchEvent(new p.window.MouseEvent(type, { clientY, bubbles: true }));
  pointer("pointerdown", 500);
  pointer("pointermove", 400);
  assert.equal(document.documentElement.style.getPropertyValue("user-select"), "none", "a drag selects no text in the notebook");
  pointer("pointerup", 400);
  assert.equal(document.documentElement.style.getPropertyValue("user-select"), "", "text can be selected again after the drag");
  const root = document.documentElement.style;
  const expected = `${p.window.innerHeight - 30 - 400}px`;
  assert.equal(root.getPropertyValue("--endeavor-drawer-h"), expected);
  assert.equal(root.getPropertyValue("--endeavor-drawer-pad"), expected);
  assert.equal(p.window.localStorage.getItem("endeavor-drawer-h"), String(p.window.innerHeight - 30 - 400));

  for (const ending of ["pointercancel", "lostpointercapture"]) {
    pointer("pointerdown", 400);
    pointer("pointermove", 350);
    pointer(ending, 350);
    pointer("pointermove", 200);
    pointer("pointerup", 200);
    assert.equal(root.getPropertyValue("--endeavor-drawer-h"), `${p.window.innerHeight - 30 - 350}px`, `a move after ${ending} changes nothing`);
  }
});

test("on marimo a drawer can't grow over marimo's floating buttons: it stops below the top ones, with room for the column at the bottom right", async () => {
  const p = await marimoPage();
  const { document } = p.window;
  const placed = (html, rect) => {
    const holder = document.createElement("div");
    holder.className = "absolute";
    holder.innerHTML = html;
    holder.getBoundingClientRect = () => rect;
    document.body.append(holder);
  };
  placed('<button data-testid="notebook-menu-dropdown"></button>', { top: 11, bottom: 38, height: 27 });
  placed('<button data-testid="command-palette-button"></button>', { top: 599, bottom: 803, height: 204 });
  const panel = document.createElement("div");
  panel.dataset.testid = "helper";
  panel.getBoundingClientRect = () => ({ top: 400, bottom: p.window.innerHeight, left: 48, right: 1000 });
  document.body.append(panel);
  const grip = document.querySelector("#endeavor-docs-grip");
  grip.setPointerCapture = () => {};
  const pointer = (type, clientY) => grip.dispatchEvent(new p.window.MouseEvent(type, { clientY, bubbles: true }));
  pointer("pointerdown", 400);
  pointer("pointermove", 20);
  pointer("pointerup", 20);
  const tallest = p.window.innerHeight - (38 + 8 + 96 + 8 + 204 + 20);
  assert.equal(document.documentElement.style.getPropertyValue("--endeavor-drawer-h"), `${tallest}px`);

  p.window.localStorage.setItem("endeavor-drawer-h", "700");
  p.window.__endeavor.receive({ type: "drawer", tab: "packages" });
  await p.settle();
  assert.equal(document.documentElement.style.getPropertyValue("--endeavor-drawer-h"), `${tallest}px`, "a taller saved height is fitted when a drawer opens");
});
