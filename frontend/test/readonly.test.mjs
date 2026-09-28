// While a server is out of reach its notebook page is readable but takes no edits or reconnects.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

test("read-only blocks typing in cells and holds reconnects until the server is back", async () => {
  const dom = new JSDOM(`<body><pluto-notebook><pluto-cell><div class="cm-content" tabindex="0">x = 1</div></pluto-cell></pluto-notebook></body>`, {
    url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
  });
  const { window } = dom;
  const opened = [];
  window.WebSocket = function (url) {
    opened.push(String(url));
  };
  window.ipc = { postMessage: () => {} };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const cell = window.document.querySelector(".cm-content");
  const type = (key, extra = {}) => cell.dispatchEvent(new window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...extra }));

  window.__endeavor.receive({ type: "context", host: "lab-server", asking: false, readonly: true });
  assert.ok(window.document.body.classList.contains("endeavor-readonly"));
  assert.equal(type("x"), false, "typing is refused");
  assert.equal(type("c", { metaKey: true }), true, "copying still works");
  new window.WebSocket("ws://localhost:1234/?secret=abc");
  assert.equal(opened.at(-1), "ws://127.0.0.1:9/");

  window.__endeavor.receive({ type: "context", host: "lab-server", asking: false, readonly: false });
  assert.ok(!window.document.body.classList.contains("endeavor-readonly"));
  assert.equal(type("x"), true);
  new window.WebSocket("ws://localhost:1234/?secret=abc");
  assert.equal(opened.at(-1), "ws://localhost:1234/?secret=abc");
});
