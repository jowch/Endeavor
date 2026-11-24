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

test("a first run precompiling: package states, dependencies, progress, header text", () => {
  const m = status.statusModel({ ...installing(), julia_version: "v1.12.6" });
  assert.equal(m.headline, "Installing packages · 1 of 2");
  assert.equal(m.busy, "Installing packages · 1 of 2");
  assert.equal(m.runtime, "Julia 1.12.6");
  assert.deepEqual(m.install, { done: 1, total: 2 });
  assert.deepEqual(m.packages, [
    { name: "Colors", state: "precompiling", detail: "" },
    { name: "Statistics", state: "ready", detail: "standard library" },
  ]);
  assert.deepEqual(m.deps, { count: 11, done: 3, failed: 0 });
  assert.equal(m.failure, null);
  assert.ok(!("cells" in m) && !("steps" in m), "cells show their own run state; Packages doesn't repeat it");
});

test("cells running: the header counts from Pluto's run task, Packages says Ready", () => {
  const nb = installing();
  nb.process_status = "ready";
  nb.status_tree.subtasks.pkg = entry(3, 9);
  nb.status_tree.subtasks.run = entry(9, null, { evaluate: entry(9, null, { 1: entry(9, 10), 2: entry(10, null) }) });
  nb.nbpkg.busy_packages = [];
  nb.cell_results = { a: { runtime: 1_250_000_000 }, b: { running: true } };
  const m = status.statusModel(nb);
  assert.equal(m.headline, "Ready");
  assert.equal(m.busy, "Running 1 of 2");
  assert.equal(m.install, null);
  assert.deepEqual(m.packages.map((p) => [p.name, p.state, p.detail]), [["Colors", "ready", "0.13.2"], ["Statistics", "ready", "standard library"]]);
});

test("a package that fails to precompile: its row, the blocked cells, the headline, its log", () => {
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
  assert.deepEqual(m.packages.map((p) => [p.name, p.state]), [["Colors", "failed"], ["Statistics", "ready"]]);
  assert.deepEqual(m.failure, {
    name: "Colors",
    why: "couldn't be installed or precompiled.",
    cells: ["using Colors, Statistics", "c"],
    log: "Precompiling packages...\n  ✓ Statistics\n  ✗ Colors\n",
  });
  assert.equal(m.busy, null);
});

test("a package Pkg can't find fails, though Pluto's package step finished", () => {
  const nb = installing();
  nb.process_status = "ready";
  nb.status_tree.subtasks.pkg = entry(3, 9);
  nb.status_tree.subtasks.run = entry(9, 12);
  nb.nbpkg.busy_packages = [];
  nb.cell_inputs.a.code = "using Colors, Statistics, NotARealPkgQ";
  nb.cell_results = { a: { errored: true, runtime: 1000 }, b: { runtime: 2000 } };
  const m = status.statusModel(nb);
  assert.deepEqual(m.packages.map((p) => [p.name, p.state, p.detail]), [
    ["Colors", "ready", "0.13.2"],
    ["Statistics", "ready", "standard library"],
    ["NotARealPkgQ", "failed", "not found"],
  ]);
  assert.equal(m.headline, "Package failed · NotARealPkgQ");
  assert.equal(m.failure.why, "isn't a package Pkg can find: a typo, or not in the registry.");
  assert.deepEqual(m.failure.cells, ["using Colors, Statistics, NotARealPkgQ"]);
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

function ember(library, rows, extra = {}) {
  return {
    process_status: "ready",
    status_tree: null,
    nbpkg: { busy_packages: ["DESeq2"], installed_versions: {}, terminal_outputs: { nbpkg_sync: "Installing DESeq2 (3/40)..." } },
    ember: { packages: { snapshot: "2026-10-01", r_version: "4.4.1", library, rows } },
    cell_order: ["a", "b"],
    cell_inputs: { a: { code: "library(DESeq2)\nlibrary(ggplot2)" }, b: { code: "dds <- DESeq(dds)" } },
    cell_results: {},
    ...extra,
  };
}

const rRows = (deseq = "installing") => [
  { name: "DESeq2", version: "1.46.0", direct: true, status: deseq },
  { name: "ggplot2", version: "3.5.1", direct: true, status: "installed" },
  { name: "S4Vectors", version: "0.44.0", direct: false, status: "installed" },
  { name: "IRanges", version: "2.40.0", direct: false, status: "installing" },
];

test("an R notebook installing: Ember's own package state, not Pluto's status tree, which Ember never sends", () => {
  const nb = ember({ status: "installing", progress: { done: 12, total: 40, current: "IRanges" } }, rRows(), {
    cell_results: { a: { running: true }, b: { queued: true } },
  });
  const m = status.statusModel(nb);
  assert.equal(m.headline, "Installing packages · 12 of 40");
  assert.equal(m.busy, "Installing packages · 12 of 40");
  assert.deepEqual(m.install, { done: 12, total: 40 });
  assert.equal(m.runtime, "R 4.4.1 · CRAN snapshot 2026-10-01");
  assert.deepEqual(m.packages, [
    { name: "DESeq2", state: "installing", detail: "" },
    { name: "ggplot2", state: "ready", detail: "3.5.1" },
  ]);
  assert.deepEqual(m.deps, { count: 2, done: 1, failed: 0 });
});

test("an R notebook running cells, and a package R can't find", () => {
  const running = status.statusModel(ember({ status: "ready" }, rRows("installed"), { cell_results: { a: { running: true }, b: { queued: true } } }));
  assert.equal(running.headline, "Ready");
  assert.equal(running.busy, "Running · 2 cells left");
  const rows = [...rRows("installed"), { name: "DESeq3", direct: true, status: "not_found", message: "no such package" }];
  const failed = status.statusModel(ember({ status: "ready" }, rows, { cell_results: { a: { errored: true } } }));
  assert.equal(failed.headline, "Package failed · DESeq3");
  assert.equal(failed.failure.why, "isn't a package R can find: a typo, or not on CRAN or Bioconductor.");
  assert.deepEqual(failed.packages.at(-1), { name: "DESeq3", state: "failed", detail: "not found" });
});

test("an R library that failed: the direct package Ember marked failed, with Ember's own reason, not a failure's detail", () => {
  const rows = [
    { name: "ggplot2", version: "3.5.1", direct: true, status: "not_installed" },
    { name: "DESeq2", version: "1.46.0", direct: true, status: "failed" },
    { name: "Rcpp", version: "1.0.13", direct: false, status: "failed" },
    { name: "S4Vectors", version: "0.44.0", direct: false, status: "not_installed" },
  ];
  const library = {
    status: "failed",
    message: "install failed:\nERROR: compilation failed for package 'Rcpp'",
    log: "\x1b[31mERROR\x1b[0m: compilation failed for package 'Rcpp'",
    failures: [
      { package: "S4Vectors", kind: "dependency", detail: "Rcpp" },
      { package: "Rcpp", kind: "compile", detail: null, needed_by: ["DESeq2"] },
    ],
  };
  const m = status.statusModel(ember(library, rows));
  assert.equal(m.headline, "Package failed · DESeq2");
  assert.equal(m.failure.why, "couldn't be installed: ERROR: compilation failed for package 'Rcpp'.");
  assert.equal(m.failure.log, "ERROR: compilation failed for package 'Rcpp'");
  assert.deepEqual(m.packages.map((p) => [p.name, p.state]), [["ggplot2", "waiting"], ["DESeq2", "failed"]], "a package that built but never landed isn't the cause");
  assert.deepEqual(m.deps, { count: 2, done: 0, failed: 1 });
  assert.equal(m.busy, null);
});

test("a failed library with no direct row marked: the first named failure, and an installer status line without its prefix", () => {
  const m = status.statusModel(ember({ status: "failed", message: "install failed, status 1", failures: [{ package: "Rcpp", kind: "compile" }] }, rRows("not_installed")));
  assert.equal(m.failure.name, "Rcpp");
  assert.equal(m.failure.why, "couldn't be installed: status 1.");
});

test("Bioconductor couldn't be looked in: the row's own message, not \"a typo\"", () => {
  const rows = [...rRows("installed"), { name: "limma", direct: true, status: "lookup_failed", message: "Bioconductor didn't answer" }];
  const m = status.statusModel(ember({ status: "ready" }, rows));
  assert.equal(m.failure.name, "limma");
  assert.equal(m.failure.why, "couldn't be looked up: Bioconductor didn't answer.");
  assert.deepEqual(m.packages.at(-1), { name: "limma", state: "failed", detail: "couldn't look up" });
});

test("an R warning Ember's lookup message carries stays out of the reason, and its brackets stay closed", () => {
  const message = "notARealPkgZ isn't on CRAN at this date, and Ember couldn't look in Bioconductor (the package index bioc/2026-11-24/3.23 could not be fetched: no index for that date (404) In addition: Warning message: package 'ember' was built under R version 4.6.2); running again tries again";
  const rows = [...rRows("installed"), { name: "notARealPkgZ", direct: true, status: "lookup_failed", message }];
  const m = status.statusModel(ember({ status: "ready" }, rows));
  assert.equal(m.failure.why, "couldn't be looked up: notARealPkgZ isn't on CRAN at this date, and Ember couldn't look in Bioconductor (the package index bioc/2026-11-24/3.23 could not be fetched: no index for that date (404)); running again tries again.", "a sentence, so the cells after it read as the next one");
  assert.equal(status.withoutRWarnings("install failed:\nERROR: x\nIn addition: Warning messages:\n1: a\n2: b"), "install failed:\nERROR: x");
});

test("the cells a failed package blocks are named by the line that uses it", () => {
  const rows = [...rRows("installed"), { name: "notARealPkgZ", direct: true, status: "lookup_failed", message: "m" }];
  const m = status.statusModel(ember({ status: "ready" }, rows, {
    cell_inputs: { a: { code: "library(tidyr)\nlibrary(lubridate)\nlibrary(notARealPkgZ)" }, b: { code: "x <- 1" } },
    cell_results: { a: { errored: true }, b: { errored: true } },
  }));
  assert.deepEqual(m.failure.cells, ["library(notARealPkgZ)", "x <- 1"]);
});

test("an R install Ember gives no progress for shows no count, so the drawer's bar runs without a stuck 0 of N", () => {
  const m = status.statusModel(ember({ status: "installing" }, rRows()));
  assert.deepEqual(m.install, { done: 0, total: 0 });
  assert.equal(m.busy, "Installing packages");
});

test("run times read like Pluto's", () => {
  assert.deepEqual([400, 12_000, 4_200_000, 420_000_000, 4_200_000_000, 72_000_000_000].map(status.prettyTime), [
    "400 ns", "12 µs", "4 ms", "420 ms", "4.2 s", "1 min 12 s",
  ]);
});

test("a cell's names leave out the ones Pluto makes up for anonymous functions", () => {
  assert.deepEqual(status.definedNames({ __ExprExpl_anon__7243518869190234: [], c: [] }), ["c"]);
  assert.deepEqual(status.definedNames(undefined), []);
});
