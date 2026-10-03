// ⏎ in the notebook answers the chat's waiting card only while nothing in the
// page has the keyboard (src/cardkey.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

const A = "11111111-1111-1111-1111-111111111111";
const NB = "0f381e2e-b8ca-11f1-b549-49cf0ce82801";

async function page() {
  const dom = new JSDOM(
    `<body><pluto-notebook><pluto-cell id="${A}"><pluto-input><div class="cm-content" contenteditable="true" tabindex="0"></div></pluto-input>` +
      `<pluto-output><input type="text"><button>Go</button></pluto-output></pluto-cell></pluto-notebook></body>`,
    { url: `http://localhost/edit?id=${NB}`, runScripts: "outside-only" },
  );
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.requestAnimationFrame = () => 0;
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  const context = (card) => window.__endeavor.receive({ type: "context", host: "This Mac", asking: false, readonly: false, card });
  const page = [];
  window.document.addEventListener("keydown", (e) => page.push(e.key));
  const enter = (target, init = {}) => {
    target.focus?.();
    return target.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...init }));
  };
  const answers = () => sent.filter((m) => m.type === "answer_card");
  return { window, context, enter, answers, page };
}

test("with a card waiting and nothing focused, ⏎ answers that card and the page doesn't see the key", async () => {
  const p = await page();
  p.context(12);
  const body = p.window.document.body;
  assert.equal(p.enter(body), false);
  assert.deepEqual(p.answers(), [{ type: "answer_card", notebook: NB, card: 12 }]);
  assert.deepEqual(p.page, []);
});

test("in a cell's editor, a field or on a button, ⏎ is the page's own", async () => {
  const p = await page();
  p.context(12);
  const doc = p.window.document;
  for (const target of [doc.querySelector(".cm-content"), doc.querySelector("input"), doc.querySelector("button")]) {
    assert.equal(p.enter(target), true);
  }
  assert.deepEqual(p.answers(), []);
  assert.deepEqual(p.page, ["Enter", "Enter", "Enter"]);
});

test("no card, a modifier, a held key or Point: ⏎ answers nothing", async () => {
  const p = await page();
  const doc = p.window.document;
  p.enter(doc.body);
  p.context(12);
  p.enter(doc.body, { shiftKey: true });
  p.enter(doc.body, { metaKey: true });
  p.enter(doc.body, { ctrlKey: true });
  p.enter(doc.body, { repeat: true });
  doc.body.classList.add("annotating");
  p.enter(doc.body);
  doc.body.classList.remove("annotating");
  p.context(null);
  p.enter(doc.body);
  assert.deepEqual(p.answers(), []);
});
