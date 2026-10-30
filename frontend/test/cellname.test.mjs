// A cell's name on the page (src/cellname.ts), the same as the app's `cell_label`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSync } from "esbuild";

const { outputFiles } = buildSync({
  entryPoints: [new URL("../src/cellname.ts", import.meta.url).pathname],
  bundle: true,
  format: "esm",
  write: false,
});
const { cellName } = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString("base64")}`);

test("a cell is named by what it defines, its heading, or its first line", () => {
  const cases = [
    ["flips = rand(Bool, 100)", "flips"],
    ["# true = heads, false = tails\n\nflips = rand(Bool, 100)", "flips"],
    ["#= Draws\n  x = 1 =#\nn_heads = count(flips)", "n_heads"],
    ["model(S, p) = p[1] * S", "model"],
    ["function fit!(p)\n  p\nend", "fit!"],
    ["const K = 3", "K"],
    ["struct Fit\n  k::Float64\nend", "Fit"],
    ["begin\n  x = 1\n  y = 2\nend", "x"],
    ['md"""\n# Coin flips\nWe flip a coin.\n"""', "Coin flips"],
    ['md"""\nWe flip a fair coin a hundred times.\n"""', "We flip a fair coin a hundr…"],
    ['md"## Results"', "Results"],
    ['scatter(t, counts, label = "data")', "scatter(t, counts, label = …"],
    ["let\n  n = 1:length(flips)\n  p = plot(n, share)\n  hline!(p, [0.5])\n  p\nend", "plot"],
    ['let\n  A, k = coef(fit)\n  Markdown.parse("k = $k")\nend', "text"],
    ["let\n  a = sum(xs)\n  b = length(xs)\n  a, b\nend", "a, b"],
    ["let\n  a = sum(xs)\n  a / 2\nend", "let"],
    ["splot(x)", "splot(x)"],
    ["x == 1", "x == 1"],
    ["using Plots", "using Plots"],
    ["# just a note", "just a note"],
    ["# Growth curves (by strain)\nlet\n  p = plot(t, y)\nend", "Growth curves (by strain)"],
    ["\n  # Outliers removed\nclean = filter(ok, data)", "Outliers removed"],
    ["# plot(t, y)\nscatter(t, y)", "scatter(t, y)"],
    ["# using CSV\ndata = 1", "data"],
    ["# data <- read.csv(f)\ndata <- 1", "data <- 1"],
    ["x = 1\n# Not a name", "x"],
    ["#= Draws =#\nn = 1", "n"],
    ["#' Roxygen\nf <- function(x) x", "f <- function(x) x"],
    ["a_rather_long_variable_name_for_tests = 1", "a_rather_long_variable_name…"],
  ];
  for (const [code, name] of cases) assert.equal(cellName(code), name, code);
});
