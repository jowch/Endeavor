// Find in the notebook (src/find.ts), for the view in Claude's desktop app, whose own ⌘F doesn't search the view.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { buildSync } from "esbuild";

function find(html) {
  const { outputFiles } = buildSync({ entryPoints: [new URL("../src/find.ts", import.meta.url).pathname], bundle: true, format: "iife", globalName: "find", write: false });
  const { window } = new JSDOM(`<body>${html}</body>`, { runScripts: "outside-only" });
  window.eval(`${outputFiles[0].text};window.find = find;`);
  const root = window.document.querySelector("pluto-notebook");
  return (q) => Array.from(window.find.findMatches(root, q), (r) => r.toString());
}

// A cell as CodeMirror draws it: each line its own element, a token per span.
const cell = (lines, output) =>
  `<pluto-cell><pluto-input><div class="cm-content">${lines.map((l) => `<div class="cm-line">${l}</div>`).join("")}</div></pluto-input>` +
  `<pluto-output>${output}</pluto-output></pluto-cell>`;

test("finds text in code and outputs, across CodeMirror's token spans, ignoring case", () => {
  const search = find(
    `<pluto-notebook>${cell(['<span class="v">rates</span> <span class="o">=</span> <span class="n">10</span>', "Rates2 = 2"], "<pre>rates are 10</pre>")}</pluto-notebook>`,
  );
  assert.deepEqual(search("rates"), ["rates", "Rates", "rates"]);
  assert.deepEqual(search("rates = 1"), ["rates = 1"], "one match over three spans");
  assert.deepEqual(search("RATES ARE"), ["rates are"]);
  assert.deepEqual(search(""), []);
  assert.deepEqual(search("nothing"), []);
});

test("a match doesn't run from one line, paragraph or cell into the next", () => {
  const search = find(`<pluto-notebook>${cell(["x = 1", "y = 2"], "<p>end</p><p>start</p>")}${cell(["z"], "")}</pluto-notebook>`);
  assert.deepEqual(search("1y"), []);
  assert.deepEqual(search("endstart"), []);
  assert.deepEqual(search("startz"), []);
  assert.deepEqual(search("y = 2"), ["y = 2"]);
  // A <br> breaks the line too.
  const br = find(`<pluto-notebook>${cell(["a"], "<span>one<br>two</span>")}</pluto-notebook>`);
  assert.deepEqual(br("onetwo"), []);
  assert.deepEqual(br("two"), ["two"]);
});

test("only the notebook: not scripts, styles or what's hidden from the page", () => {
  const search = find(
    `<div id="endeavor-bar">needle</div><pluto-notebook>${cell(["a"], `<script>needle</script><style>.needle{}</style><span hidden>needle</span><span aria-hidden="true">needle</span><b>needle</b>`)}</pluto-notebook>`,
  );
  assert.deepEqual(search("needle"), ["needle"]);
});

test("the view's toolbar has Find, before Docs", () => {
  const dom = new JSDOM(`<head></head><body><pluto-notebook></pluto-notebook></body>`, { url: "http://localhost/", runScripts: "outside-only", pretendToBeVisual: true });
  const { window } = dom;
  Object.defineProperty(window.navigator, "platform", { value: "MacIntel" });
  window.eval(readFileSync(new URL("../dist/view.js", import.meta.url), "utf8"));
  window.EndeavorView.start("0f381e2e-b8ca-11f1-b549-49cf0ce82801", { canSend: false, send: async () => {} });
  const buttons = [...window.document.querySelectorAll("#endeavor-bar button")].map((b) => b.getAttribute("aria-label"));
  assert.deepEqual(buttons.slice(0, 2), ["Find", "Docs"]);
  assert.match(window.document.getElementById("endeavor-find-button").title, /⌘F/);
  assert.ok(window.document.getElementById("endeavor-find"), "its box, shut");
  assert.ok(!window.document.getElementById("endeavor-find").classList.contains("open"));
});
