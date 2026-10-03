// The user's own run asks first when it would reach a cell a chat card is
// asking to run (src/runguard.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { buildSync } from "esbuild";

const A = "11111111-1111-1111-1111-111111111111",
  B = "22222222-2222-2222-2222-222222222222",
  C = "33333333-3333-3333-3333-333333333333",
  D = "44444444-4444-4444-4444-444444444444";

test("a run reaches its cells and everything downstream of them", async () => {
  const { outputFiles } = buildSync({
    entryPoints: [new URL("../src/runguard.ts", import.meta.url).pathname],
    bundle: true,
    format: "esm",
    write: false,
  });
  globalThis.window ??= {};
  const { reach } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString("base64")}`);
  // a = 5; b = a + 1; c = b * 2 and d = a - 1, as Pluto maps them; e stands alone.
  const deps = {
    [A]: { downstream_cells_map: { a: [B, D] }, upstream_cells_map: {} },
    [B]: { downstream_cells_map: { b: [C] }, upstream_cells_map: { a: [A] } },
    [C]: { downstream_cells_map: {}, upstream_cells_map: { b: [B] } },
    [D]: { downstream_cells_map: { d: [] }, upstream_cells_map: { a: [A] } },
  };
  assert.deepEqual(reach([A], deps).sort(), [A, B, C, D]);
  assert.deepEqual(reach([B], deps).sort(), [B, C]);
  assert.deepEqual(reach([D, "e"], deps).sort(), [D, "e"]);
});

/** A notebook page with cells a, b, c (b = a + 1, c = b * 2), `a` edited in its editor and not submitted. */
async function page({ safe = false } = {}) {
  const cell = (id) =>
    `<pluto-cell id="${id}"><pluto-runarea class="run"><button class="runcell"></button></pluto-runarea>` +
    `<pluto-input><div class="cm-content" tabindex="0"><div class="cm-line">x</div></div></pluto-input></pluto-cell>`;
  const dom = new JSDOM(`<body><pluto-editor><main><pluto-notebook>${cell(A)}${cell(B)}${cell(C)}</pluto-notebook></main></pluto-editor></body>`, {
    url: "http://localhost/edit?id=0f381e2e-b8ca-11f1-b549-49cf0ce82801",
    runScripts: "outside-only",
  });
  const { window } = dom;
  const sent = [];
  window.ipc = { postMessage: (body) => sent.push(JSON.parse(body)) };
  window.requestAnimationFrame = () => 0;
  const results = { [A]: { last_run_timestamp: 1 }, [B]: { last_run_timestamp: 1 }, [C]: { last_run_timestamp: 1 } };
  window.editor_state = {
    notebook: {
      process_status: safe ? "waiting_for_permission" : "ready",
      cell_order: [A, B, C],
      cell_inputs: { [A]: { code: "a = 5" }, [B]: { code: "b = a + 1" }, [C]: { code: "c = b * 2" } },
      cell_results: results,
      cell_dependencies: {
        [A]: { downstream_cells_map: { a: [B] } },
        [B]: { downstream_cells_map: { b: [C] } },
        [C]: { downstream_cells_map: { c: [] } },
      },
    },
    cell_inputs_local: { [A]: { code: "a = 6" }, [B]: { code: "b = a + 1" }, [C]: { code: "c = b * 2" } },
    selected_cells: [],
  };
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  // Pluto's own handlers, which see only what gets past the guard.
  const pluto = [];
  window.document.addEventListener("keydown", (e) => pluto.push(`${e.ctrlKey ? "Ctrl+" : ""}${e.shiftKey ? "Shift+" : ""}${e.key}`));
  for (const button of window.document.querySelectorAll("button.runcell")) button.addEventListener("click", () => pluto.push(`run ${button.closest("pluto-cell").id}`));
  const waiting = (cells) =>
    window.__endeavor.receive({ type: "context", host: "This Mac", asking: false, readonly: false, ask_cells: cells.map((c) => c.id), waiting_runs: cells });
  const key = (target, init) => target.dispatchEvent(new window.KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  const save = () => key(window.document.querySelector(`[id="${A}"] .cm-content`), { key: "s", ctrlKey: true });
  const dialog = () => window.document.getElementById("endeavor-runguard");
  const button = (name) => dialog().querySelector(`button.${name}`);
  return { window, sent, pluto, results, waiting, key, save, dialog, button };
}

test("nothing waiting, or nothing reached: the run goes straight to Pluto", async () => {
  const p = await page();
  p.save();
  assert.deepEqual(p.pluto, ["Ctrl+s"]);
  assert.equal(p.dialog(), null);

  // A card asks to run a, but ⇧Enter in c reaches only c.
  p.waiting([{ id: A, name: "a" }]);
  p.key(p.window.document.querySelector(`[id="${C}"] .cm-content`), { key: "Enter", shiftKey: true });
  assert.deepEqual(p.pluto, ["Ctrl+s", "Shift+Enter"]);
  assert.equal(p.dialog(), null);

  // Safe preview: ⌘S saves without running, so it never asks.
  const safe = await page({ safe: true });
  safe.waiting([{ id: B, name: "b" }]);
  safe.save();
  assert.deepEqual(safe.pluto, ["Ctrl+s"]);
  assert.equal(safe.dialog(), null);
});

test("a run reaching a cell Claude asks to run asks first: Show, Cancel, Run anyway", async () => {
  const p = await page();
  p.waiting([{ id: B, name: "b" }]);

  // ⌘S submits a, whose change re-runs b.
  p.save();
  assert.deepEqual(p.pluto, [], "Pluto never saw it");
  assert.equal(p.dialog().querySelector("b").textContent, "Claude is waiting for your answer on b.");
  assert.match(p.dialog().textContent, /Running your changes now also runs it\./);
  assert.deepEqual([...p.dialog().querySelectorAll("button")].map((b) => b.textContent), ["Show b", "Cancel", "Run anyway"]);

  // Show b outlines it, and the question stays.
  p.button("show").click();
  assert.ok(p.window.document.getElementById(B).classList.contains("endeavor-guard-show"));
  assert.ok(p.dialog());

  // Cancel (or Esc): nothing runs, nothing is answered.
  p.button("cancel").click();
  assert.equal(p.dialog(), null);
  assert.ok(!p.window.document.getElementById(B).classList.contains("endeavor-guard-show"));
  p.save();
  p.key(p.window.document.body, { key: "Escape" });
  assert.equal(p.dialog(), null);
  assert.deepEqual(p.pluto, []);
  assert.deepEqual(p.sent.filter((m) => m.type === "run_anyway"), []);

  // Run anyway (⏎): the user's ⌘S goes to Pluto, and once b is under way the card is answered.
  p.save();
  p.key(p.window.document.body, { key: "Enter" });
  assert.equal(p.dialog(), null);
  assert.deepEqual(p.pluto, ["Ctrl+s"]);
  await new Promise((done) => setTimeout(done, 120));
  assert.deepEqual(p.sent.filter((m) => m.type === "run_anyway"), [], "not before b is queued");
  p.results[B] = { queued: true, last_run_timestamp: 1 };
  await new Promise((done) => setTimeout(done, 120));
  assert.deepEqual(p.sent.filter((m) => m.type === "run_anyway"), [
    { type: "run_anyway", notebook: "0f381e2e-b8ca-11f1-b549-49cf0ce82801", cells: [{ id: B, last_run: 1 }] },
  ]);
});

test("the run button and ⇧Enter on a cell ask too, naming every waiting cell they reach", async () => {
  const p = await page();
  p.waiting([
    { id: B, name: "b" },
    { id: C, name: null },
  ]);
  p.window.document.querySelector(`[id="${A}"] button.runcell`).click();
  assert.equal(p.dialog().querySelector("b").textContent, "Claude is waiting for your answer on b and c.");
  assert.match(p.dialog().textContent, /Running this now also runs them\./);
  assert.equal(p.button("show").textContent, "Show them");
  p.button("run").click();
  assert.deepEqual(p.pluto, [`run ${A}`], "the click goes to Pluto's button");

  p.key(p.window.document.querySelector(`[id="${B}"] .cm-content`), { key: "Enter", shiftKey: true });
  assert.equal(p.dialog().querySelector("b").textContent, "Claude is waiting for your answer on b and c.");
  p.button("cancel").click();
  assert.deepEqual(p.pluto, [`run ${A}`]);
});
