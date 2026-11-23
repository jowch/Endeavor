import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

test("Point's glyph in the page is the app's Glyph::Pointer, drawn on a 16px box instead of 12px", () => {
  const rust = readFileSync(new URL("../../src/new_session.rs", import.meta.url), "utf8");
  const page = readFileSync(new URL("../src/askbox.ts", import.meta.url), "utf8");
  const app = [...rust.match(/Glyph::Pointer => polyline\(&\[(.*?)\]\)/)[1].matchAll(/\(([\d.]+), ([\d.]+)\)/g)].map((m) => [+m[1], +m[2]]);
  const flat = page.match(/point: svg\(`<path d="M([^Z"]+)Z/)[1].trim().split(/\s+/).map(Number);
  const drawn = flat.flatMap((v, i) => (i % 2 ? [] : [[v, flat[i + 1]]]));
  const tenths = (v) => Math.round((v * 16 * 10) / 12) / 10;
  const closedByZ = app.slice(0, -1);
  assert.deepEqual(drawn, closedByZ.map(([x, y]) => [tenths(x), tenths(y)]));
});
