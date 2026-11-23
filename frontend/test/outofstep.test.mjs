// Pluto's "Oopsie!! please refresh your browser" alert loads the page again instead (#285).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("Pluto's out-of-step alert asks the app to load the page again, once, and other alerts still show", async () => {
  const dom = new JSDOM(`<body></body>`, { url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801", runScripts: "outside-only" });
  const { window } = dom;
  const shown = [];
  window.alert = (message) => shown.push(String(message));
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body).type) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  window.alert("Oopsie!! please refresh your browser and everything will be alright!");
  window.alert("Hoppla!! Bitte aktualisiere deinen Browser und alles wird in Ordnung sein!");
  window.alert("Something else");
  assert.deepEqual(shown, ["Something else"], "only the other alert shows");
  assert.equal(sent.filter((type) => type === "out_of_step").length, 1, "the app is asked once for this page");
});
