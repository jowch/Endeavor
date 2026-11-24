import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSync } from "esbuild";
import { JSDOM } from "jsdom";

const { outputFiles } = buildSync({
  entryPoints: [new URL("../src/marimodocs.ts", import.meta.url).pathname],
  bundle: true,
  format: "iife",
  globalName: "docs",
  write: false,
});

const load = async () => {
  const { window } = new JSDOM("<body></body>", { runScripts: "outside-only" });
  return window.eval(`${outputFiles[0].text};docs`);
};

test("marimo's docs panel counts as shown only when it is open and it is the documentation panel", async () => {
  const { showsDocs } = await load();
  assert.equal(showsDocs("30.0", "Documentation"), true);
  assert.equal(showsDocs("0.0", "Documentation"), false);
  assert.equal(showsDocs("30.0", "Variables"), false);
  assert.equal(showsDocs(null, ""), false);
});

test("a dropped press is pressed again, and a slow one is waited for rather than undone", async () => {
  const { setDocs } = await load();
  const wait = () => Promise.resolve();
  let shown = false;
  let presses = 0;
  const dropsFirst = () => { presses++; if (presses > 1) shown = !shown; return true; };
  assert.equal(await setDocs(true, () => shown, dropsFirst, wait), true);
  assert.equal(presses, 2);

  shown = false;
  presses = 0;
  let polls = 0;
  const slow = () => { presses++; return true; };
  const shownLate = () => { polls++; if (presses && polls > 5) shown = true; return shown; };
  assert.equal(await setDocs(true, shownLate, slow, wait), true);
  assert.equal(presses, 1, "one press, waited out");

  presses = 0;
  const pressed = () => { presses++; return true; };
  assert.equal(await setDocs(true, () => true, pressed, wait), true);
  assert.equal(presses, 0, "already open: nothing to press");
  assert.equal(await setDocs(true, () => false, pressed, wait), false, "gives up after a few tries");
  assert.equal(presses, 4);
});

test("a press asked for before marimo's sidebar is on the page waits for it rather than using up its tries", async () => {
  const { setDocs } = await load();
  let mounted = 0;
  let shown = false;
  let presses = 0;
  const press = () => { if (++mounted < 40) return false; presses++; shown = true; return true; };
  assert.equal(await setDocs(true, () => shown, press, () => Promise.resolve()), true);
  assert.equal(presses, 1);
  assert.equal(await setDocs(false, () => true, () => false, () => Promise.resolve()), false, "a sidebar that never comes gives up");
});

test("the drawer starts beside marimo's icon column and ends above its footer", async () => {
  const { besideIconColumn } = await load();
  assert.equal(JSON.stringify(besideIconColumn()), JSON.stringify({ left: 0, bottom: 0 }), "no column, the whole width");
});

test("marimo's sidebar counts as dragging only while its options are inert", async () => {
  const { window } = new JSDOM(`<body><div data-testid="chrome-sidebar"><div role="option" data-key="files"></div></div></body>`, { runScripts: "outside-only" });
  const { sidebarDragging } = window.eval(`${outputFiles[0].text};docs`);
  assert.equal(sidebarDragging(), false);
  window.document.querySelector("[role=option]").setAttribute("inert", "");
  assert.equal(sidebarDragging(), true);
});
