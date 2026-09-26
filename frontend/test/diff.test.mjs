// The line diff behind the in-editor diffs.
import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSync } from "esbuild";

const { outputFiles } = buildSync({ entryPoints: [new URL("../src/diff.ts", import.meta.url).pathname], bundle: true, format: "esm", write: false });
globalThis.window = {}; // the bridge registers itself on load
const { lineDiff, changedSpan } = await import("data:text/javascript," + encodeURIComponent(outputFiles[0].text));

test("a changed line is a hunk of one removed and one added line", () => {
  assert.deepEqual(lineDiff("a\nb\nc", "a\nB\nc"), [{ at: 1, removed: ["b"], added: ["B"] }]);
});

test("additions, removals and a new cell", () => {
  assert.deepEqual(lineDiff("a\nc", "a\nb\nc"), [{ at: 1, removed: [], added: ["b"] }]);
  assert.deepEqual(lineDiff("a\nb\nc", "a\nc"), [{ at: 1, removed: ["b"], added: [] }]);
  assert.deepEqual(lineDiff("a", "a\nb"), [{ at: 1, removed: [], added: ["b"] }]);
  assert.deepEqual(lineDiff("", "x = 1"), [{ at: 0, removed: [], added: ["x = 1"] }]);
  assert.deepEqual(lineDiff("same", "same"), []);
});

test("changed characters are the differing middle", () => {
  assert.deepEqual(changedSpan("y = x + 1", "y = x + 2"), [8, 9, 9]);
  assert.deepEqual(changedSpan("f(a)", "f(a, b)"), [3, 3, 6]);
});
