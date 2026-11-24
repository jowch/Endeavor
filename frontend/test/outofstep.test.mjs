import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM, VirtualConsole } from "jsdom";

const settle = (ms) => new Promise((done) => setTimeout(done, ms));
const OUT_OF_STEP = "Oopsie!! please refresh your browser and everything will be alright!";

async function page(reloadedAt) {
  const reloads = [];
  const virtualConsole = new VirtualConsole();
  virtualConsole.on("jsdomError", (e) => reloads.push(e.message));
  const dom = new JSDOM(`<body><pluto-editor><main><pluto-notebook></pluto-notebook></main></pluto-editor></body>`, {
    url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
    virtualConsole,
  });
  const { window } = dom;
  window.ipc = { postMessage: () => {} };
  if (reloadedAt != null) window.sessionStorage.setItem("endeavor-out-of-step-reloaded-at", String(reloadedAt));
  const alerts = [];
  window.alert = (message) => alerts.push(message);
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await settle(20);
  return { window, alerts, reloads };
}

test("Pluto's out-of-step alert reloads the page instead of showing", async () => {
  const { window, alerts, reloads } = await page();
  window.alert(OUT_OF_STEP);
  window.alert(OUT_OF_STEP);
  await settle(50);
  assert.deepEqual(alerts, []);
  assert.equal(reloads.filter((m) => /navigation|reload/i.test(m)).length, 1, reloads.join("\n"));
  assert.ok(Number(window.sessionStorage.getItem("endeavor-out-of-step-reloaded-at")) > 0);
});

test("right after a reload, the next one waits instead of reloading in a loop", async () => {
  const { window, alerts, reloads } = await page(Date.now());
  window.alert(OUT_OF_STEP);
  await settle(50);
  assert.deepEqual(alerts, []);
  assert.equal(reloads.filter((m) => /navigation|reload/i.test(m)).length, 0, reloads.join("\n"));
});

test("other alerts still show", async () => {
  const { window, alerts } = await page();
  window.alert("Could not save");
  assert.deepEqual(alerts, ["Could not save"]);
});
