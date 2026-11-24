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
  const dropsFirst = () => { presses++; if (presses > 1) shown = !shown; };
  assert.equal(await setDocs(true, () => shown, dropsFirst, wait), true);
  assert.equal(presses, 2);

  shown = false;
  presses = 0;
  let polls = 0;
  const slow = () => { presses++; };
  const shownLate = () => { polls++; if (presses && polls > 5) shown = true; return shown; };
  assert.equal(await setDocs(true, shownLate, slow, wait), true);
  assert.equal(presses, 1, "one press, waited out");

  presses = 0;
  assert.equal(await setDocs(true, () => true, () => presses++, wait), true);
  assert.equal(presses, 0, "already open: nothing to press");
  assert.equal(await setDocs(true, () => false, () => presses++, wait), false, "gives up after a few tries");
  assert.equal(presses, 4);
});

test("the drawer starts beside marimo's icon column and ends above its footer", async () => {
  const { besideIconColumn } = await load();
  assert.equal(JSON.stringify(besideIconColumn()), JSON.stringify({ left: 0, bottom: 0 }), "no column, the whole width");
});
