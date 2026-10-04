// How cells are named for the user, the same way as the app's `cell_label` (src/session.rs).

/**
 * A cell's name, as the app names it (`cell_label`): what it defines (`rates = …` → `rates`),
 * a markdown cell's first heading or words, "plot" for a cell that draws one, "text" for one
 * that builds Markdown, a `let` or `begin` block's last value, else its first line of code;
 * at most 28 characters.
 */
export function cellName(code: string): string {
  const markdown = markdownText(code);
  let label: string;
  if (markdown !== null) {
    const lines = markdown.split("\n").map((l) => l.trim()).filter((l) => l);
    label = (lines.find((l) => l.startsWith("#")) ?? lines[0] ?? "markdown").replace(/^#+/, "").trim();
  } else {
    const first = codeLines(code)[0];
    label = first === undefined ? "cell" : (definedName(code) ?? inferredName(code) ?? first);
  }
  const chars = [...label];
  return chars.length > 28 ? `${chars.slice(0, 27).join("")}…` : label;
}

function definedName(code: string): string | null {
  const lines = codeLines(code);
  const line = lines[0] === "begin" ? lines[1] : lines[0];
  if (line === undefined) return null;
  const word = (s: string) => s.trim().match(/^[\p{L}\p{N}_!]+/u)?.[0] ?? null;
  const block = ["function ", "macro ", "struct ", "mutable struct "].find((k) => line.startsWith(k));
  if (block) return word(line.slice(block.length));
  const eq = line.indexOf("=");
  if (eq < 0) return null;
  const lhs = line.slice(0, eq);
  // `f(x, k=1)` and `a == b` assign nothing.
  if (line[eq + 1] === "=" || lhs.split("(").length !== lhs.split(")").length) return null;
  return word(lhs.trim().replace(/^const /, ""));
}

const PLOTS = ["plot", "scatter", "heatmap", "histogram", "lines", "surface", "contour", "Figure"];

/**
 * A name for code of more than one line that defines nothing (a single line names itself): "plot" when it calls a plotting function, "text" when it
 * builds Markdown, else a `let` or `begin` block's last value when that's a name or a tuple of names.
 */
function inferredName(code: string): string | null {
  const lines = codeLines(code);
  if (lines.length < 2) return null;
  const plot = new RegExp(`(^|[^\\p{L}\\p{N}_.])(${PLOTS.join("|")})!*\\(`, "u");
  if (lines.some((l) => plot.test(l))) return "plot";
  if (lines.some((l) => l.includes("Markdown.parse(") || l.includes('md"'))) return "text";
  if (!["let", "begin"].includes(lines[0]) || lines[lines.length - 1] !== "end" || lines.length < 3) return null;
  const last = lines[lines.length - 2];
  return last.split(",").every((n) => /^[\p{L}\p{N}_]+$/u.test(n.trim())) ? last : null;
}

/** Code's lines, trimmed, without blank lines, `#` comments and `#= … =#` blocks. */
function codeLines(code: string): string[] {
  let inBlock = false;
  return code
    .split("\n")
    .map((l) => l.trim())
    .filter((line) => {
      if (inBlock || line.startsWith("#=")) {
        inBlock = !line.endsWith("=#");
        return false;
      }
      return line !== "" && !line.startsWith("#");
    });
}

/** A markdown cell's text: `md"""…"""` or `md"…"` without its quotes. */
function markdownText(code: string): string | null {
  const first = codeLines(code)[0];
  if (first === undefined) return null;
  const rest = code.slice(code.indexOf(first));
  const m = rest.match(/^md"(?:"")?/);
  return m ? rest.slice(m[0].length).trimEnd().replace(/"+$/, "") : null;
}
