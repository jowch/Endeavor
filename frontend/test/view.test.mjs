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
  told.opened = [];
  window.EndeavorView.start(NB, { canSend: true, send: async (t) => void told.sent.push(t), openLink: async (u) => void told.opened.push(u) });
  return { window, told, $: (s) => window.document.querySelector(s) };
}

test("picks become the same <quote> text the app sends (but for a long output, cut below)", () => {
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
  // An R notebook is Ember's, as the app names it.
  assert.equal(quoteText(NB, [{ part: "output", cell: A, code: CODE, text: "[1] 1 4 9" }], "", "ember"), `<quote cell="rates" part="output" uri="notebook://ember/${NB}/cell/${A}">\n[1] 1 4 9\n</quote>`);
});

test("a huge output's text is cut to its start and end, saying how much is left out", () => {
  const { outputFiles } = buildSync({ entryPoints: [new URL("../src/view.ts", import.meta.url).pathname], bundle: true, format: "iife", globalName: "view", write: false });
  const { window } = new JSDOM("", { runScripts: "outside-only" });
  window.eval(`${outputFiles[0].text};window.view = view;`);
  const { quoteText, OUTPUT_MAX } = window.view;
  const short = "x".repeat(OUTPUT_MAX);
  assert.ok(quoteText(NB, [{ part: "output", cell: A, code: CODE, text: short }], "").includes(`\n${short}\n`), "up to the limit, all of it");
  const big = Array.from({ length: 200000 }, (_, i) => String(i + 1)).join("\n");
  const said = quoteText(NB, [{ part: "output", cell: A, code: CODE, text: big }], "Why?");
  assert.ok(said.length < OUTPUT_MAX + 400, `cut (${said.length} characters)`);
  assert.ok(said.includes("\n1\n2\n3\n") && said.includes("\n199999\n200000\n</quote>\nWhy?"), "its start and its end");
  assert.match(said, /\n\[1,284,894 characters of this output left out here\.\]\n/);
  // Never between the halves of a surrogate pair.
  const emoji = quoteText(NB, [{ part: "output", cell: A, code: CODE, text: "a".repeat(OUTPUT_MAX - 501) + "\u{1F600}".repeat(1000) }], "");
  assert.ok(!/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/.test(emoji), "no half pairs");
});

test("the view's look leaves Pluto's header and docs panel to the toolbar's rules, and shows a Point button", () => {
  const { window, $ } = page();
  const css = $("#endeavor-theme").textContent;
  assert.ok(css.includes("--main-bg-color"), "Endeavor's colours");
  assert.ok(!css.includes("header#pluto-nav"), "not the app's rules: the view has no app header");
  assert.ok(!css.includes("#helpbox-wrapper"), "not the app's drawer");
  assert.equal(window.document.documentElement.dataset.endeavorLook, "endeavor");
  assert.match($("#endeavor-point").textContent, /Point⌘⇧E/);
  // Claude's side panel goes down to about 320 px: smaller headings there, and long words wrap.
  const narrow = $("#endeavor-narrow").textContent;
  assert.match(narrow, /max-width: 600px[\s\S]*h1 \{ font-size: 1\.6rem/);
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
  assert.ok(window.document.getElementById("endeavor-bar"), "the toolbar, without Point");
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

test("Pluto's website chrome gives way to one toolbar: the file, what Pluto is doing, Docs, Status and Point", async () => {
  const { window, $ } = page();
  const css = $("#endeavor-chrome").textContent;
  assert.match(css, /footer \{ display: none/, "no footer (Settings, FAQ)");
  assert.match(css, /header#pluto-nav \{ display: none/, "no Pluto header (file box, Move, share)");
  assert.match(css, /process_waiting_for_permission[^{]*> header#pluto-nav \{\s*display: block/, "but its band when the notebook needs the user");
  assert.match(css, /pluto-helpbox\.hidden \{ display: none/, "no floating Live docs and Status tabs");
  assert.match(css, /pluto-helpbox > header \{ display: none/, "nor the panel's own tabs: the toolbar opens it");
  assert.match(css, /\.outline-frame-actions-container\.safe-preview \{ display: none/, "no floating safe-preview badge");
  const bar = $("#endeavor-bar");
  assert.equal(bar, window.document.body.firstElementChild);
  assert.ok(bar.querySelector(".panel.docs"));
  assert.ok(bar.querySelector(".panel.process"), "Status, Pluto's other bottom tab");
  const opened = [];
  window.addEventListener("open_bottom_right_panel", (e) => opened.push(e.detail));
  bar.querySelector(".panel.process").click();
  assert.deepEqual(opened, [], "a script's click doesn't open the panel: only the user's");
  assert.match($("#endeavor-space").textContent, /padding-left: var\(--e-left\)/, "the cells reach from the eye to the scrollbar");
  assert.equal(bar.querySelector("#endeavor-point"), $("#endeavor-point"), "Point is in the toolbar");
  window.editor_state = { notebook: { path: "/home/me/work/growth.jl", process_status: "ready", status_tree: null, nbpkg: null, cell_order: [], cell_inputs: {}, cell_results: {} } };
  window.document.body.append(window.document.createElement("i"));
  await new Promise((r) => setTimeout(r, 300));
  assert.equal(bar.querySelector(".file").textContent, "growth.jl", "the name, not the folder");
  assert.equal(bar.querySelector(".doing").textContent, "", "nothing while idle");
});

test("the toolbar says what Pluto is doing, and leaves what needs the user to Pluto's band", () => {
  const { outputFiles } = buildSync({ entryPoints: [new URL("../src/view.ts", import.meta.url).pathname], bundle: true, format: "iife", globalName: "view", write: false });
  const { window } = new JSDOM("", { runScripts: "outside-only" });
  window.eval(`${outputFiles[0].text};window.view = view;`);
  const { doing, fileName, outsideLink } = window.view;
  assert.equal(outsideLink(" https://example.com/a "), "https://example.com/a");
  assert.equal(outsideLink("mailto:a@b.c"), "mailto:a@b.c");
  for (const inside of ["#cell", "javascript:;", "./a.jl", "", null]) assert.equal(outsideLink(inside), null, String(inside));
  const at = (started, finished) => ({ started_at: started, finished_at: finished, subtasks: {} });
  const nb = (tree, extra = {}) => ({ process_status: "ready", status_tree: tree, nbpkg: null, cell_order: ["a"], cell_inputs: { a: { code: "x = 1" } }, cell_results: {}, ...extra });
  const running = { ...at(1, null), subtasks: { run: { ...at(1, null), subtasks: { evaluate: { ...at(1, null), subtasks: { a: at(1, 2), b: at(2, null) } } } } } };
  assert.equal(doing(nb(running)), "Running 1 of 2");
  assert.equal(doing(nb({ ...at(1, 2), subtasks: { saving: { ...at(1, 2), success: false, subtasks: {} } } })), "Couldn't save");
  assert.equal(doing(nb(null, { process_status: "waiting_for_permission" })), "", "Safe preview is Pluto's band");
  assert.equal(doing(undefined), "");
  assert.equal(fileName("C:\\work\\a.jl"), "a.jl");
  assert.equal(fileName(undefined), "Notebook");
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

test("no link replaces the view with a blank page, and only the user's own clicks open one", () => {
  const { window, told } = page();
  const doc = window.document;
  doc.body.insertAdjacentHTML("beforeend", `<a id="hash" href="#">r</a><a id="rel" href="./other.jl">o</a><a id="file" href="file:///etc/x">f</a><a id="js" href="javascript:;">j</a><a id="web" href="https://example.com">w</a><a id="handled" href="./x">h</a><p id="target">t</p><a id="jump" href="#target">j</a>`);
  doc.getElementById("handled").addEventListener("click", (e) => e.preventDefault());
  const click = (id) => { const e = new window.MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }); doc.getElementById(id).dispatchEvent(e); return e.defaultPrevented; };
  for (const id of ["hash", "rel", "file", "web", "jump"]) assert.ok(click(id), `${id} doesn't navigate`);
  assert.equal(click("js"), false, "javascript: links run as they are");
  assert.ok(click("handled"), "a click the page handled stays handled");
  assert.deepEqual(told.opened, [], "a script's made-up click opens nothing");
});
test("in Claude's panel the page takes Claude's palette, with our accent, once the host's variables are there", async () => {
  const { window, $ } = page();
  const root = window.document.documentElement;
  assert.ok(!root.classList.contains("claude-palette"), "no variables: the Endeavor look as it is");
  const css = $("#endeavor-claude-palette").textContent;
  assert.match(css, /--e-bg-page: var\(--color-background-primary\)/);
  assert.match(css, /--main-bg-color: var\(--color-background-primary\)/);
  assert.ok(!/--e-accent|--cm-color|--e-diff/.test(css), "our accent, code and diff colours stay");
  for (const v of ["background", "text", "border"]) for (const k of ["primary", "secondary", "tertiary"]) root.style.setProperty(`--color-${v}-${k}`, "light-dark(#fff, #000)");
  await new Promise((r) => setTimeout(r, 0));
  assert.ok(root.classList.contains("claude-palette"));
  root.style.removeProperty("--color-text-tertiary");
  await new Promise((r) => setTimeout(r, 0));
  assert.ok(!root.classList.contains("claude-palette"), "not with some missing");
});
