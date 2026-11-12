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
  const told = { sent: [] };
  window.EndeavorView.start(NB, { canSend: true, send: async (t) => void told.sent.push(t) });
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
  // Claude's side panel goes down to about 320 px: smaller headings there, and long words wrap.
  const narrow = $("#endeavor-narrow").textContent;
  assert.match(narrow, /max-width: 480px[\s\S]*h1 \{ font-size: 1\.6rem/);
  assert.match(narrow, /overflow-wrap: break-word/);
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

test("a chat that takes no messages from the view gets the look and no Point; start runs once", () => {
  const dom = new JSDOM(`<head></head><body></body>`, { url: "http://localhost/", runScripts: "outside-only", pretendToBeVisual: true });
  const { window } = dom;
  window.eval(readFileSync(new URL("../dist/view.js", import.meta.url), "utf8"));
  const host = { canSend: false, send: async () => {} };
  window.EndeavorView.start(NB, host);
  assert.ok(window.document.getElementById("endeavor-theme"));
  assert.equal(window.document.getElementById("endeavor-point"), null);
  assert.equal(window.document.getElementById("annotate-bar"), null);
  window.EndeavorView.start(NB, { ...host, canSend: true });
  assert.equal(window.document.getElementById("endeavor-point"), null, "a second start does nothing");
});

test("a code completion list closes once the user is in Claude's chat, but not while Point is on", async () => {
  const { window, $ } = page();
  const escapes = [];
  const editor = window.document.createElement("div");
  editor.className = "cm-editor";
  editor.innerHTML = `<div class="cm-content"></div>`;
  $("pluto-input").append(editor);
  editor.querySelector(".cm-content").addEventListener("keydown", (e) => {
    escapes.push(e.key);
    window.document.querySelector(".cm-tooltip-autocomplete")?.remove();
  });
  const open = () => editor.insertAdjacentHTML("beforeend", `<div class="cm-tooltip-autocomplete"><ul><li>__precompile__</li></ul></div>`);
  window.__endeavor.receive({ type: "annotate", on: true });
  open();
  window.dispatchEvent(new window.Event("blur"));
  assert.deepEqual(escapes, [], "Point keeps Escape for itself");
  window.__endeavor.receive({ type: "annotate", on: false });
  window.dispatchEvent(new window.Event("blur"));
  assert.deepEqual(escapes, ["Escape"], "the list gets the Escape the user would give it");
  assert.equal($(".cm-tooltip-autocomplete"), null);
});

test("Pluto's bottom overlays sit round Point's button", () => {
  const { $ } = page();
  const css = [...$("head").querySelectorAll("style")].map((s) => s.textContent).join("\n");
  assert.match(css, /pluto-helpbox \{ bottom: 52px/, "Live docs and Status above the button");
  assert.match(css, /\.outline-frame-actions-container\.safe-preview \{ display: none/, "no floating safe-preview badge for Settings and FAQ to pass under");
  assert.match(css, /pluto-editor footer \{ padding-bottom: 104px/, "Settings and FAQ above them all at the page's end");
});

// The view with a way to post the page's messages as the user's ↩ would (jsdom makes no trusted keys).
function pageWithSend({ focused }) {
  const { outputFiles } = buildSync({
    stdin: { contents: `import "./src/view"; import { send } from "./src/bridge"; window.testSend = send;`, resolveDir: new URL("..", import.meta.url).pathname, loader: "ts" },
    bundle: true, format: "iife", write: false,
  });
  const dom = new JSDOM(
    `<head></head><body><pluto-cell id="${A}"><pluto-output><p>0.42</p></pluto-output><pluto-input><div class="cm-content"><div class="cm-line">x</div></div></pluto-input></pluto-cell></body>`,
    { url: "http://localhost/", runScripts: "outside-only", pretendToBeVisual: true },
  );
  const { window } = dom;
  window.document.hasFocus = () => focused.now;
  window.eval(outputFiles[0].text);
  const sent = [];
  window.EndeavorView.start(NB, { canSend: true, send: async (t) => void sent.push(t) });
  const quote = () => window.testSend({ type: "quote", notebook: NB, picks: [{ part: "output", cell: A, code: "x", text: "0.42" }], comment: "Why?", add: false, ref: 1 });
  return { window, sent, quote, on: () => window.document.body.classList.contains("annotating") };
}

test("after ↩, Point ends once the view has lost the keyboard to Claude's box, not before", async () => {
  const focused = { now: true };
  const { window, sent, quote, on } = pageWithSend({ focused });
  window.__endeavor.receive({ type: "annotate", on: true });
  quote();
  assert.match(window.document.getElementById("endeavor-sent").textContent, /^Sending to Claude's chat/);
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(sent.length, 1);
  assert.match(window.document.getElementById("endeavor-sent").textContent, /message box/);
  assert.ok(on(), "the keyboard is still here: Point stays on for the next pick");
  focused.now = false;
  window.dispatchEvent(new window.Event("blur"));
  assert.ok(!on(), "the user went to Claude's box: Point ends");
});

test("after ↩, Point ends at once when the keyboard is already in Claude's box", async () => {
  const { window, quote, on } = pageWithSend({ focused: { now: false } });
  window.__endeavor.receive({ type: "annotate", on: true });
  quote();
  await new Promise((r) => setTimeout(r, 10));
  assert.ok(!on());
});

test("the view brings Claude's new cell into view, until the user scrolls somewhere else", async () => {
  const { window, $ } = page();
  const scrolled = [];
  const add = (id) => {
    const cell = window.document.createElement("pluto-cell");
    cell.id = id;
    cell.getBoundingClientRect = () => ({ top: 2000, bottom: 2100, height: 100 });
    cell.scrollIntoView = () => scrolled.push(id);
    $("body").append(cell);
  };
  // Cells drawn as the page loads aren't followed.
  add("loading");
  await new Promise((r) => setTimeout(r, 1700));
  assert.deepEqual(scrolled, []);
  add("new1");
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(scrolled, ["new1"]);
  // The user scrolls away from the end (well after the view's own scroll): new cells stay out of view.
  await new Promise((r) => setTimeout(r, 1100));
  Object.defineProperty(window.document.documentElement, "scrollHeight", { value: 5000, configurable: true });
  window.dispatchEvent(new window.Event("scroll"));
  add("new2");
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(scrolled, ["new1"]);
  // Back at the end: followed again.
  Object.defineProperty(window.document.documentElement, "scrollHeight", { value: window.innerHeight, configurable: true });
  window.dispatchEvent(new window.Event("scroll"));
  add("new3");
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(scrolled, ["new1", "new3"]);
});
