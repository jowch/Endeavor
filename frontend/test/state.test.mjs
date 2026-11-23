import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("a feature's own context handler sees the agent from the same message, since initState's handler runs first", async () => {
  const dom = new JSDOM(`<body></body>`, { url: "http://localhost/edit?id=nb", runScripts: "outside-only", pretendToBeVisual: true });
  const { window } = dom;
  window.ipc = { postMessage: () => {} };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const hint = () => [...window.document.querySelectorAll("style")].map((s) => s.textContent).find((t) => t.includes("cm-placeholder::after"));
  assert.match(hint(), /to ask Claude"/);
  window.__endeavor.receive({ type: "context", host: "Local", agent: "Codex", asking: false, readonly: false });
  assert.match(hint(), /to ask Codex"/);
});
