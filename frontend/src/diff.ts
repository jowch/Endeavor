import { on } from "./bridge";
import { onRedraw } from "./redraw";

const css = `
  .cm-line.endeavor-add { position: relative; z-index: 0; }
  .cm-line.endeavor-add::before {
    content: ""; position: absolute; z-index: -1; pointer-events: none;
    top: 0; bottom: 0; right: 0; left: calc(-1 * var(--indented, 0px));
    background: var(--e-diff-add-tint);
  }
  .cm-line.endeavor-add::after {
    content: "+"; position: absolute; top: 0; pointer-events: none;
    left: calc(-1 * var(--indented, 0px) - 13px); color: var(--e-diff-add); text-indent: 0;
  }
  .endeavor-add-ch { background: var(--e-diff-add-ch); border-radius: 2px; }
  .endeavor-del { position: relative; background: var(--e-diff-del-tint); color: var(--e-diff-del); white-space: break-spaces; word-break: break-word; overflow-wrap: anywhere; }
  .endeavor-del::before { content: "−"; position: absolute; left: -13px; }
  .endeavor-del-ch { background: var(--e-diff-del-ch); border-radius: 2px; }
`;

export type Hunk = {
  at: number;
  removed: string[];
  added: string[];
};

const LARGEST_LCS_TABLE = 250_000;

export function lineDiff(before: string, after: string): Hunk[] {
  const a = before === "" ? [] : before.split("\n");
  const b = after.split("\n");
  const n = a.length, m = b.length;
  if (n * m > LARGEST_LCS_TABLE) return [];
  const lcs = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  const hunks: Hunk[] = [];
  let i = 0, j = 0, open: Hunk | null = null;
  const hunk = () => (open ??= (hunks.push({ at: j, removed: [], added: [] }), hunks[hunks.length - 1]));
  while (i < n || j < m) {
    if (i < n && j < m && a[i] === b[j]) {
      open = null;
      i++, j++;
    } else if (j < m && (i >= n || lcs[i][j + 1] >= lcs[i + 1][j])) {
      hunk().added.push(b[j++]);
    } else {
      hunk().removed.push(a[i++]);
    }
  }
  return hunks;
}

export function changedSpan(a: string, b: string): [number, number, number] {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length, endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) endA--, endB--;
  return [start, endA, endB];
}

type View = any;
let cm: { Decoration: any; Compartment: any; appendConfig: any; decorations: any; extender: any } | null = null;

const stateEffectClassFromAStaticHelper = (EditorView: any) => EditorView.scrollIntoView(0).constructor;
const compartmentClassFromPlutosOwnCompartments = (view: View) => [...(view.state.config?.compartments?.keys() ?? [])][0]?.constructor;

function classes(view: View) {
  if (cm) return cm;
  const EditorView = view.constructor;
  const StateEffect = stateEffectClassFromAStaticHelper(EditorView);
  const Compartment = compartmentClassFromPlutosOwnCompartments(view);
  if (!Compartment) return null;
  for (const source of view.state.facet(EditorView.decorations)) {
    const set = typeof source === "function" ? source(view) : source;
    let c = set?.iter?.().value?.constructor;
    while (c && typeof c.line !== "function") c = Object.getPrototypeOf(c);
    if (c)
      return (cm = {
        Decoration: c,
        Compartment,
        appendConfig: StateEffect.appendConfig,
        decorations: EditorView.decorations,
        extender: view.state.constructor.transactionExtender,
      });
  }
  return null;
}

function viewOf(cell: Element): View | null {
  return (cell.querySelector("pluto-input .cm-content") as any)?.cmTile?.root?.view ?? null;
}

function removedLine(text: string, span: [number, number] | null) {
  return {
    text,
    toDOM() {
      const el = document.createElement("div");
      el.className = "endeavor-del";
      if (span) {
        const mark = document.createElement("span");
        mark.className = "endeavor-del-ch";
        mark.textContent = text.slice(span[0], span[1]);
        el.append(text.slice(0, span[0]), mark, text.slice(span[1]));
      } else {
        el.textContent = text || " ";
      }
      return el;
    },
    eq(other: any) { return other.text === text; },
    compare(other: any) { return other === this || other.text === text; },
    updateDOM() { return false; },
    estimatedHeight: -1,
    lineBreaks: 0,
    ignoreEvent() { return true; },
    coordsAt() { return null; },
    destroy() {},
  };
}

function decorate(doc: any, before: string) {
  const { Decoration } = cm!;
  const ranges: any[] = [];
  for (const h of lineDiff(before, doc.toString())) {
    const paired = h.removed.length === h.added.length;
    const spans = h.added.map((line, k) => (paired ? changedSpan(h.removed[k], line) : null));
    const at = h.at < doc.lines ? doc.line(h.at + 1).from : doc.length;
    h.removed.forEach((line, k) => {
      const span = spans[k];
      const widget = removedLine(line, span ? [span[0], span[1]] : null);
      ranges.push(Decoration.widget({ widget, block: true, side: h.at < doc.lines ? -1 : 1 }).range(at));
    });
    h.added.forEach((_, k) => {
      const line = doc.line(h.at + k + 1);
      ranges.push(Decoration.line({ class: "endeavor-add" }).range(line.from));
      const span = spans[k];
      if (span && span[2] > span[0]) ranges.push(Decoration.mark({ class: "endeavor-add-ch" }).range(line.from + span[0], line.from + span[2]));
    });
  }
  return Decoration.set(ranges, true);
}

const befores = new Map<string, string>();
const installed = new WeakMap<object, { compartment: any; shown: string | undefined }>();

const staticSetSinceBlockWidgetsCantComeFromAFunction = (doc: any, before: string | undefined) => (before === undefined ? [] : cm!.decorations.of(decorate(doc, before)));

function refresh() {
  for (const cell of document.querySelectorAll("pluto-cell")) {
    const view = viewOf(cell);
    if (!view || !classes(view)) continue;
    const id = cell.id;
    const before = befores.get(id);
    let entry = installed.get(view);
    if (!entry) {
      if (before === undefined) continue;
      entry = { compartment: new cm!.Compartment(), shown: before };
      installed.set(view, entry);
      const { compartment } = entry;
      const follow = cm!.extender.of((tr: any) =>
        tr.docChanged && befores.has(id) ? { effects: compartment.reconfigure(staticSetSinceBlockWidgetsCantComeFromAFunction(tr.newDoc, befores.get(id))) } : null,
      );
      view.dispatch({ effects: cm!.appendConfig.of([compartment.of(staticSetSinceBlockWidgetsCantComeFromAFunction(view.state.doc, before)), follow]) });
    } else if (entry.shown !== before) {
      entry.shown = before;
      view.dispatch({ effects: entry.compartment.reconfigure(staticSetSinceBlockWidgetsCantComeFromAFunction(view.state.doc, before)) });
    }
  }
}

export function initDiffs(): void {
  const style = document.createElement("style");
  style.textContent = css;
  document.head.append(style);
  on("cells", (msg) => {
    befores.clear();
    for (const c of msg.cells) if (typeof c.before === "string") befores.set(c.cell_id, c.before);
    refresh();
  });
  onRedraw(refresh);
}
