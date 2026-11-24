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

test("a ready Python notebook lists its packages with their versions, under the Python uv runs it on and its package date", async () => {
  const { marimoModel } = await load();
  const m = marimoModel({ ...ready, python_version: "3.12.8" });
  assert.equal(m.headline, "Ready");
  assert.equal(m.runtime, "Python 3.12.8 · uv · PyPI as of 2026-10-05");
  assert.deepEqual(plain(m.packages.map((p) => [p.name, p.state, p.detail])), [["boto3", "ready", "1.43.108"], ["marimo", "ready", "0.25.1"]]);
  assert.equal(m.install, null);
  assert.equal(m.failure, null);
  assert.equal(marimoModel({ ...ready, package_date: null }).runtime, "Python 3.10 or later · uv", "the header's requirement until uv has found a Python");
});

test("packages being installed or waiting for the user's OK show as such, and new ones are added to the list", async () => {
  const { marimoModel } = await load();
  const installing = marimoModel({ ...ready, install: { step: "installing", packages: ["Boto3", "polars"] } });
  assert.deepEqual(plain(installing.packages.map((p) => [p.name, p.state])), [["boto3", "installing"], ["marimo", "ready"], ["polars", "installing"]]);
  assert.deepEqual(plain(installing.install), { done: 0, total: 0 }, "uv doesn't count them, so the bar sweeps");
  assert.equal(installing.busy, "Installing packages");
  assert.equal(installing.headline, "Installing packages");
  const asking = marimoModel({ ...ready, install: { step: "asking", packages: ["polars"] } });
  assert.deepEqual(plain(asking.packages.map((p) => p.state)), ["ready", "ready", "waiting"]);
  assert.equal(asking.install, null);
  assert.equal(asking.busy, null);
});

test("a failed install names the package and gives the log's telling line as the reason", async () => {
  const { marimoModel } = await load();
  const log = "Installing endeavor_e2e_absent...\n  × No solution found when resolving dependencies:\n  ╰─▶ Because endeavor-e2e-absent was not found in the package registry\nfailed to install endeavor_e2e_absent\n";
  const m = marimoModel({ ...ready, failed: { packages: ["endeavor_e2e_absent"], log } });
  assert.equal(m.packages.find((p) => p.name === "endeavor_e2e_absent").state, "failed");
  assert.equal(m.headline, "Package failed · endeavor_e2e_absent");
  assert.equal(m.failure.name, "endeavor_e2e_absent");
  assert.match(m.failure.why, /^couldn't be installed: .*not found in the package registry$/);
  assert.equal(m.failure.log, log);
  assert.equal(m.log, log);
});

test("a header without a lowest Python version is shown as written", async () => {
  const { pythonLine } = await load();
  assert.equal(pythonLine(">=3.11,<3.13"), "Python >=3.11,<3.13");
  assert.equal(pythonLine(null), "Python");
  assert.equal(pythonLine(">=3.10", "3.12.8"), "Python 3.12.8");
});
