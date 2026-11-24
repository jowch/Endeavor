import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSync } from "esbuild";
import { JSDOM } from "jsdom";

const { outputFiles } = buildSync({
  entryPoints: [new URL("../src/marimopackages.ts", import.meta.url).pathname],
  bundle: true,
  format: "iife",
  globalName: "packages",
  write: false,
});

const load = async () => {
  const { window } = new JSDOM("<body></body>", { runScripts: "outside-only" });
  return window.eval(`${outputFiles[0].text};packages`);
};

const plain = (value) => JSON.parse(JSON.stringify(value));

const ready = {
  python: ">=3.10",
  packages: [{ name: "boto3", version: "1.43.108" }, { name: "marimo", version: "0.25.1" }],
  package_date: "2026-10-05",
  install: null,
  failed: null,
};

test("a ready Python notebook lists its packages with their versions, under its Python and package date", async () => {
  const { fromMarimo } = await load();
  const env = fromMarimo(ready);
  assert.equal(env.runtime, "Python 3.10 or later");
  assert.equal(env.source, "PyPI as of 2026-10-05");
  assert.deepEqual(plain(env.packages.map((p) => [p.name, p.version, p.state])), [["boto3", "1.43.108", "ready"], ["marimo", "0.25.1", "ready"]]);
  assert.equal(env.install, null);
  assert.equal(env.failure, null);
});

test("packages being installed or waiting for the user's OK show as such, and new ones are added to the list", async () => {
  const { fromMarimo } = await load();
  const installing = fromMarimo({ ...ready, install: { step: "installing", packages: ["Boto3", "polars"] } });
  assert.deepEqual(plain(installing.packages.map((p) => [p.name, p.state])), [["boto3", "installing"], ["marimo", "ready"], ["polars", "installing"]]);
  assert.deepEqual(plain(installing.install), { done: 0, total: 2 });
  const asking = fromMarimo({ ...ready, install: { step: "asking", packages: ["polars"] } });
  assert.deepEqual(plain(asking.packages.map((p) => p.state)), ["ready", "ready", "waiting"]);
  assert.equal(asking.install, null);
});

test("a failed install names the package and gives the log's telling line as the reason", async () => {
  const { fromMarimo } = await load();
  const log = "Installing endeavor_e2e_absent...\n  × No solution found when resolving dependencies:\n  ╰─▶ Because endeavor-e2e-absent was not found in the package registry\nfailed to install endeavor_e2e_absent\n";
  const env = fromMarimo({ ...ready, failed: { packages: ["endeavor_e2e_absent"], log } });
  const row = env.packages.find((p) => p.name === "endeavor_e2e_absent");
  assert.equal(row.state, "failed");
  assert.match(row.why, /not found in the package registry/);
  assert.equal(env.failure.name, "endeavor_e2e_absent");
  assert.equal(env.failure.log, log);
});

test("a header without a lowest Python version is shown as written", async () => {
  const { pythonLine } = await load();
  assert.equal(pythonLine(">=3.11,<3.13"), "Python >=3.11,<3.13");
  assert.equal(pythonLine(null), "Python");
});
