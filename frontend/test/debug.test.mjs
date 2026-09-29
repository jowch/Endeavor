// A debug build's state dump asks the page what it shows: Point and the cells
// it picked, the drawer, the safe-preview callout, and the alerts it showed.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("debug answers with Point's picks and the alerts shown", async () => {
  const C = "22222222-2222-2222-2222-222222222222";
  const dom = new JSDOM(`<body><pluto-cell id="11111111-1111-1111-1111-111111111111"></pluto-cell><pluto-cell id="${C}"></pluto-cell></body>`, {
    url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
  });
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));

  window.__endeavor.receive({ type: "debug" });
  assert.deepEqual(sent.at(-1), { type: "debug", point: false, picked: [], box: false, point_status: "", comment: "", drawer: null, callout: false, alerts: null });

  window.__endeavor.receive({ type: "annotate", on: true });
  window.document.getElementById(C).dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  window.document.querySelector("#annotate-bar textarea").value = "what does this do?";
  window.__endeavorAlerts = ["Could not save"];
  window.__endeavor.receive({ type: "debug" });
  assert.deepEqual(sent.at(-1), {
    type: "debug",
    point: true,
    picked: [C],
    box: false,
    point_status: "1 cell selected",
    comment: "what does this do?",
    drawer: null,
    callout: false,
    alerts: ["Could not save"],
  });
});
