import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

test("the notebook view's JuliaMono from the web is the release the app bundles", () => {
  const theme = readFileSync(new URL("../src/theme.ts", import.meta.url), "utf8");
  const build = readFileSync(new URL("../../fonts/build.py", import.meta.url), "utf8");
  const web = theme.match(/JULIA_MONO_TAG = "([^"]+)"/)[1];
  const bundled = build.match(/^JULIAMONO_TAG = "([^"]+)"/m)[1];
  assert.equal(web, bundled);
});
