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
    ["x == 1", "x == 1"],
    ["using Plots", "using Plots"],
    ["# just a note", "cell"],
    ["a_rather_long_variable_name_for_tests = 1", "a_rather_long_variable_name…"],
  ];
  for (const [code, name] of cases) assert.equal(cellName(code), name, code);
});
