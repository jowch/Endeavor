import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSync } from "esbuild";
import { readFileSync } from "node:fs";

const { outputFiles } = buildSync({
  entryPoints: [new URL("../src/cellname.ts", import.meta.url).pathname],
  bundle: true,
  format: "esm",
  write: false,
});
const { cellName } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString("base64")}`);

const cases = JSON.parse(readFileSync(new URL("./cell-names.json", import.meta.url), "utf8"));

test("a cell is named by what it defines, its heading, or its first line, as the app's cell_label names it (cell-names.json)", () => {
  for (const { code, name } of cases) assert.equal(cellName(code), name, code);
});
