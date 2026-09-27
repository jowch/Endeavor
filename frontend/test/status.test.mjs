// The Status tab's rows (src/status.ts) from literal Pluto states and a real Pkg log.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { buildSync } from "esbuild";

const { outputFiles } = buildSync({
  entryPoints: [new URL("../src/status.ts", import.meta.url).pathname],
  bundle: true,
  format: "esm",
  write: false,
});
const status = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString("base64")}`);
const colorsLog = readFileSync(new URL("./fixtures/pkg-colors.log", import.meta.url), "utf8");

const entry = (started, finished, subtasks = {}, success = null) => ({ started_at: started, finished_at: finished, success, subtasks });

/** A notebook `using Colors, Statistics` whose first run is precompiling Colors. */
function installing() {
  return {
    process_status: "starting",
    status_tree: entry(1, null, {
      workspace: entry(1, 3),
      pkg: entry(3, null, { resolve: entry(3, 4), instantiate1: entry(4, 6), precompile: entry(6, null) }),
      run: entry(null, null),
    }),
    nbpkg: {
      busy_packages: ["Colors", "Statistics"],
      installed_versions: { Colors: "0.13.2", Statistics: "stdlib" },
      terminal_outputs: { nbpkg_sync: colorsLog.split("   2155.6 ms")[0] },
    },
    cell_order: ["a", "b"],
    cell_inputs: { a: { code: "using Colors, Statistics" }, b: { code: "c = colorant\"red\"" } },
    cell_results: { a: { queued: true }, b: { queued: true } },
    cell_dependencies: {},
  };
}

test("Pkg's log: what it precompiled, and what it added to the manifest", () => {
  const log = status.parsePkgLog(colorsLog);
  assert.deepEqual(log.precompiled, ["Reexport", "Statistics", "FixedPointNumbers", "ColorTypes", "Colors"]);
  assert.deepEqual(log.failed, []);
  assert.deepEqual(log.added, [
    "ColorTypes", "Colors", "FixedPointNumbers", "Reexport", "Statistics", "Artifacts", "Libdl", "LinearAlgebra",
    "Random", "SHA", "CompilerSupportLibraries_jll", "OpenBLAS_jll", "libblastrampoline_jll",
  ]);
  assert.deepEqual(status.parsePkgLog("Precompiling packages...\n  ✗ Plots\n  0 dependencies successfully precompiled in 9 seconds").failed, ["Plots"]);
});

test("imports in notebook order, without Base or relative modules", () => {
  assert.deepEqual(status.importedPackages(["using CSV, DataFrames", "import LsqFit: curve_fit\nusing Base.Threads", "using .Local, Plots # plots"]), [
    "CSV", "DataFrames", "LsqFit", "Plots",
  ]);
});

test("a first run precompiling: steps, package states, dependencies, header text", () => {
  const m = status.statusModel(installing());
  assert.equal(m.headline, "Installing packages · 1 of 2");
  assert.equal(m.busy, "Installing packages · 1 of 2");
  assert.deepEqual(m.steps.map((s) => s.phase), ["done", "busy", "waiting"]);
  assert.deepEqual(m.packages, [
    { name: "Colors", state: "precompiling", detail: "" },
    { name: "Statistics", state: "ready", detail: "standard library" },
  ]);
  assert.deepEqual(m.deps, { count: 11, precompiled: 3, failed: 0 });
  assert.deepEqual(m.cells.map((c) => c.state), ["waiting", "waiting"]);
  assert.equal(m.failure, null);
});

test("cells running: progress from Pluto's run task, times from each cell's runtime", () => {
  const nb = installing();
  nb.process_status = "ready";
  nb.status_tree.subtasks.pkg = entry(3, 9);
  nb.status_tree.subtasks.run = entry(9, null, { evaluate: entry(9, null, { 1: entry(9, 10), 2: entry(10, null) }) });
  nb.nbpkg.busy_packages = [];
  nb.cell_results = { a: { runtime: 1_250_000_000 }, b: { running: true } };
  nb.cell_dependencies = { b: { downstream_cells_map: { c: [] } } };
  const m = status.statusModel(nb);
  assert.equal(m.headline, "Running cells · 1 of 2");
  assert.equal(m.busy, "Running 1 of 2");
  assert.deepEqual(m.packages.map((p) => [p.name, p.state, p.detail]), [["Colors", "ready", "0.13.2"], ["Statistics", "ready", "standard library"]]);
  assert.deepEqual(m.cells, [
    { id: "a", name: "using Colors, Statistics", state: "done", time: "1.3 s" },
    { id: "b", name: "c", state: "running", time: null },
  ]);
});

test("a package that fails to precompile: its row, the blocked cells, the headline", () => {
  const nb = installing();
  nb.process_status = "ready";
  nb.status_tree.subtasks.pkg = entry(3, 9, {}, false);
  nb.status_tree.subtasks.run = entry(9, 12);
  nb.nbpkg.busy_packages = [];
  delete nb.nbpkg.installed_versions.Colors;
  nb.nbpkg.terminal_outputs.nbpkg_sync = "Precompiling packages...\n  ✓ Statistics\n  ✗ Colors\n";
  nb.cell_results = { a: { errored: true, runtime: 1000 }, b: { errored: true, runtime: 2000 } };
  const m = status.statusModel(nb);
  assert.equal(m.headline, "Package failed · Colors");
  assert.deepEqual(m.steps.map((s) => s.phase), ["done", "failed", "done"]);
  assert.deepEqual(m.packages.map((p) => [p.name, p.state]), [["Colors", "failed"], ["Statistics", "ready"]]);
  assert.deepEqual(m.failure, { name: "Colors", cells: ["using Colors, Statistics", "c = colorant\"red\""] });
  assert.equal(m.busy, null);
});

test("safe preview, a failed save, a required restart", () => {
  const nb = installing();
  nb.process_status = "waiting_for_permission";
  nb.status_tree = entry(1, null, { saving: entry(5, 6, {}, false) });
  nb.nbpkg = { restart_required_msg: "Yes, restart" };
  const m = status.statusModel(nb);
  assert.equal(m.headline, "Safe preview · nothing has run");
  assert.equal(m.saveFailed, true);
  assert.equal(m.restart, "required");
  assert.deepEqual(m.packages.map((p) => [p.name, p.state]), [["Colors", "waiting"], ["Statistics", "waiting"]]);
});

test("run times read like Pluto's", () => {
  assert.deepEqual([400, 12_000, 4_200_000, 420_000_000, 4_200_000_000, 72_000_000_000].map(status.prettyTime), [
    "400 ns", "12 µs", "4 ms", "420 ms", "4.2 s", "1 min 12 s",
  ]);
});
