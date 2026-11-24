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

test("a failed install names the package and gives uv's reason in plain words", async () => {
  const { marimoModel } = await load();
  const log = "Installing endeavor_e2e_absent...\n  × No solution found when resolving dependencies:\n  ╰─▶ Because endeavor-e2e-absent was not found in the package registry\nfailed to install endeavor_e2e_absent\n";
  const m = marimoModel({ ...ready, failed: { packages: ["endeavor_e2e_absent"], log } });
  assert.equal(m.packages.find((p) => p.name === "endeavor_e2e_absent").state, "failed");
  assert.equal(m.headline, "Package failed · endeavor_e2e_absent");
  assert.equal(m.failure.name, "endeavor_e2e_absent");
  assert.equal(m.failure.why, "couldn't be installed: it isn't in the package index");
  assert.equal(m.failure.log, log);
  assert.equal(m.log, log);
});

test("a header without a lowest Python version is shown as written", async () => {
  const { pythonLine } = await load();
  assert.equal(pythonLine(">=3.11,<3.13"), "Python >=3.11,<3.13");
  assert.equal(pythonLine(null), "Python");
  assert.equal(pythonLine(">=3.10", "3.12.8"), "Python 3.12.8");
});

test("uv's reason is its first sentence joined across the lines uv wraps, without its conclusion", async () => {
  const { failureReason } = await load();
  const log = "`uv --quiet add --script nb.py humanize==4.17.0` exited with code 1:\n× No solution found when resolving dependencies for split (markers:\n  │ python_full_version == '3.10.*'):\n  ╰─▶ Because the requested Python version (>=3.10) does not satisfy\n      Python>=3.11 and humanize==4.17.0 depends on Python>=3.11, we can\n      conclude that humanize==4.17.0 cannot be used.\n      And because you require humanize==4.17.0, we can conclude that your\n      requirements are unsatisfiable.\nFailed to install humanize\n";
  assert.equal(failureReason(log), "the requested Python version (>=3.10) does not satisfy Python>=3.11 and humanize==4.17.0 depends on Python>=3.11");
  assert.equal(failureReason("  ╰─▶ Because notarealpkgzq was not found in the package registry and you\n      require notarealpkgzq, we can conclude that your requirements are\n      unsatisfiable.\n"), "it isn't in the package index");
  assert.equal(failureReason("error: Failed to fetch: https://pypi.org/simple/x/\n"), "error: Failed to fetch: https://pypi.org/simple/x/");
});

test("marimo's install operations replace the runtime's install state while they run, and their failure while they've failed", async () => {
  const { applyOperation, withLive } = await load();
  const op = (status, packages, logs = {}) => ({ operation_id: "x", action: "install", source: "kernel", status: { kind: status }, packages, logs });
  let live = applyOperation(null, op("running", { polars: "running" }, { polars: "Installing polars...\n" }), "replace");
  live = applyOperation(live, op("running", { polars: "running" }, { polars: "Resolved 3 packages\n" }), "append");
  assert.equal(live.logs.polars, "Installing polars...\nResolved 3 packages\n");
  assert.deepEqual(plain(withLive(ready, live).install), { step: "installing", packages: ["polars"] });
  assert.equal(applyOperation(live, { ...op("running", { ruff: "running" }), source: "server" }, "append"), live, "marimo's own server tools aren't the notebook's packages");
  assert.equal(applyOperation(live, { ...op("running", {}), action: "prepare" }, "append"), live);
  const failed = withLive(ready, applyOperation(live, op("failed", { polars: "failed" }), "append"));
  assert.deepEqual(plain(failed.failed.packages), ["polars"]);
  assert.equal(failed.install, null);
  const earlier = { ...ready, failed: { packages: ["Polars"], log: "" } };
  assert.equal(withLive(earlier, applyOperation(live, op("succeeded", { polars: "succeeded" }), "append")).failed, null);
  assert.ok(withLive(earlier, applyOperation(live, op("succeeded", { numpy: "succeeded" }), "append")).failed);
  const settled = applyOperation(live, op("failed", { polars: "failed" }), "append");
  const later = { ...ready, install: { step: "installing", packages: ["numpy"] } };
  assert.deepEqual(plain(withLive(later, settled)), plain(later), "the runtime's own install beats a settled one");
  const both = withLive({ ...ready, failed: { packages: ["Polars"], log: "polars log" } }, applyOperation(null, op("failed", { polars: "failed", humanize: "failed" }, { humanize: "humanize log" }), "replace"));
  assert.deepEqual(plain(both.failed), { packages: ["Polars", "humanize"], log: "polars log\nhumanize log" }, "the runtime's failure and marimo's both show, the runtime's first");
});
