// The notebook view in Claude's desktop app (src/view.ts): the quote text Claude
// gets, the look with Pluto's own header, and Point only from the user's own keys.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { buildSync } from "esbuild";

const NB = "0f381e2e-b8ca-11f1-b549-49cf0ce82801";
const A = "11111111-1111-1111-1111-111111111111";
const CODE = "rates = map(1:1000) do _\n  fit(rows)\nend";

function page() {
  const dom = new JSDOM(
    `<head></head><body><header id="pluto-nav">nav</header><pluto-cell id="${A}"><pluto-output><p>0.42</p></pluto-output>` +
      `<pluto-input><div class="cm-content">${CODE.split("\n").map((l) => `<div class="cm-line">${l}</div>`).join("")}</div></pluto-input></pluto-cell></body>`,
    { url: "http://localhost/", runScripts: "outside-only", pretendToBeVisual: true },
  );
  const { window } = dom;
  Object.defineProperty(window.navigator, "platform", { value: "MacIntel" });
  window.eval(readFileSync(new URL("../dist/view.js", import.meta.url), "utf8"));
  const told = { sent: [], context: [] };
  window.EndeavorView.start(NB, { send: async (t) => void told.sent.push(t), context: async (t) => void told.context.push(t) });
  return { window, told, $: (s) => window.document.querySelector(s) };
}

test("picks become the same <quote> text the app sends", () => {
  const { outputFiles } = buildSync({ entryPoints: [new URL("../src/view.ts", import.meta.url).pathname], bundle: true, format: "iife", globalName: "view", write: false });
  const { window } = new JSDOM("", { runScripts: "outside-only" });
  window.eval(`${outputFiles[0].text};window.view = view;`);
  const { quoteText } = window.view;
  const uri = `notebook://pluto/${NB}/cell/${A}`;
  assert.equal(
    quoteText(NB, [{ part: "lines", cell: A, code: CODE, lines: [2, 2], text: "  fit(rows)" }, { part: "figure", cell: A, code: CODE, shot: 1 }], "Why fit here?"),
    `<quote cell="rates" lines="2-2" uri="${uri}">\n  fit(rows)\n</quote>\n\n<quote cell="rates" part="figure" uri="${uri}"/>\nWhy fit here?`,
  );
  assert.equal(quoteText(NB, [{ part: "box", cells: [A, A] }], ""), `<quote part="box" uri="${uri} ${uri}"/>`);
});

test("the view's look keeps Pluto's header and docs panel, and shows a Point button", () => {
  const { window, $ } = page();
  const css = $("#endeavor-theme").textContent;
  assert.ok(css.includes("--main-bg-color"), "Endeavor's colours");
  assert.ok(!css.includes("header#pluto-nav"), "Pluto's header stays: the view has no app header");
  assert.ok(!css.includes("#helpbox-wrapper"), "Pluto's Live docs stay: the view has no drawer");
  assert.equal(window.document.documentElement.dataset.endeavorLook, "endeavor");
  assert.match($("#endeavor-point").textContent, /Point⌘⇧E/);
});

test("Point sends nothing for keys a page script makes up", async () => {
  const { window, told, $ } = page();
  window.__endeavor.receive({ type: "annotate", on: true });
  $("pluto-output p").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  $("#annotate-bar textarea").value = "Send this as the user";
  $("#annotate-bar textarea").dispatchEvent(new window.KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(told.sent, [], "an untrusted ↩ doesn't reach Claude's chat");
  // ⌘⇧E from a script doesn't toggle Point either.
  window.dispatchEvent(new window.KeyboardEvent("keydown", { key: "E", metaKey: true, shiftKey: true }));
  assert.ok(window.document.body.classList.contains("annotating"));
});
