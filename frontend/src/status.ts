// The Status tab's rows, from Pluto's own state (window.editor_state.notebook):
// its status tree gives the steps (Start Julia, Packages, Run cells) and each
// run's progress; its package state gives the notebook's packages. Pkg's log is
// read only for which packages it precompiled (✓) or failed to (✗), and for the
// dependency count (the manifest's added packages). Pure: tested with literal
// fixtures (test/status.test.mjs).

export type Phase = "waiting" | "busy" | "done" | "failed";
export type PkgState = "waiting" | "installing" | "precompiling" | "ready" | "failed";
export type CellState = "waiting" | "running" | "done" | "failed";

export interface StatusEntry {
  success?: boolean | null;
  started_at: number | null;
  finished_at: number | null;
  subtasks: Record<string, StatusEntry>;
}

export interface NotebookLike {
  process_status: string;
  status_tree: StatusEntry | null;
  nbpkg: {
    busy_packages?: string[];
    installed_versions?: Record<string, string>;
    terminal_outputs?: Record<string, string>;
    restart_required_msg?: string | null;
    restart_recommended_msg?: string | null;
  } | null;
  cell_order: string[];
  cell_inputs: Record<string, { code: string }>;
  cell_results: Record<string, { running?: boolean; queued?: boolean; errored?: boolean; runtime?: number | null }>;
  cell_dependencies?: Record<string, { downstream_cells_map?: Record<string, unknown> }>;
}

export interface PkgRow {
  name: string;
  state: PkgState;
  /** The installed version, or "standard library". */
  detail: string;
}

export interface CellRow {
  id: string;
  name: string;
  state: CellState;
  /** The last run's time, e.g. "0.4 s". */
  time: string | null;
}

export interface StatusModel {
  /** The bold line at the top: what's happening now. */
  headline: string;
  steps: Array<{ name: "Start Julia" | "Packages" | "Run cells"; phase: Phase }>;
  packages: PkgRow[];
  /** Packages the notebook's packages brought in, when Pkg added any this time. */
  deps: { count: number; precompiled: number; failed: number } | null;
  cells: CellRow[];
  failure: { name: string; cells: string[] } | null;
  /** Busy work, for the app's header while the drawer is shut. */
  busy: string | null;
  saveFailed: boolean;
  restart: "required" | "recommended" | null;
}

export function phaseOf(entry: StatusEntry | null | undefined): Phase {
  if (!entry) return "waiting";
  if (entry.success === false) return "failed";
  if (entry.finished_at != null) return "done";
  if (entry.started_at != null) return "busy";
  return "waiting";
}

const ansi = /\x1b\[[0-9;]*m/g;

/** What Pkg's log says: packages precompiled (✓) or failed (✗), and the manifest's additions. */
export function parsePkgLog(log: string): { precompiled: string[]; failed: string[]; added: string[] } {
  const precompiled: string[] = [];
  const failed: string[] = [];
  const added: string[] = [];
  let inManifest = false;
  for (const raw of log.replace(ansi, "").split("\n")) {
    const line = raw.trimEnd();
    const mark = line.match(/^\s*(?:[\d.]+\s*ms)?\s*([✓✗])\s+([\w.]+)/);
    if (mark) (mark[1] === "✓" ? precompiled : failed).push(mark[2]);
    if (/^\s*Updating\s+`.*Manifest\.toml`/.test(line)) {
      inManifest = true;
      continue;
    }
    // The manifest's lines each name a package by its UUID prefix, "[5ae59095] + Colors v0.13.2".
    const entry = line.match(/\[[0-9a-f]{8}\]\s+(\+)?\s*([\w.]+)/);
    if (!entry) inManifest = false;
    else if (inManifest && entry[1]) added.push(entry[2]);
  }
  return { precompiled, failed, added };
}

/** Packages in the order the notebook imports them (`using A, B`, `import C: f`). */
export function importedPackages(codes: string[]): string[] {
  const names: string[] = [];
  for (const code of codes) {
    for (const line of code.split("\n")) {
      const m = line.match(/^\s*(?:using|import)\s+([^#]+)/);
      if (!m) continue;
      const list = m[1].split(":")[0];
      for (const part of list.split(",")) {
        const name = part.trim().split(/[.\s]/)[0];
        if (/^[A-Za-z_]\w*$/.test(name) && !["Base", "Core", "Main"].includes(name) && !names.includes(name)) names.push(name);
      }
    }
  }
  return names;
}

export function prettyTime(ns: number): string {
  if (ns < 1e3) return `${Math.round(ns)} ns`;
  if (ns < 1e6) return `${Math.round(ns / 1e3)} µs`;
  if (ns < 1e9) return `${Math.round(ns / 1e6)} ms`;
  const s = ns / 1e9;
  if (s < 60) return `${s < 10 ? s.toFixed(1) : Math.round(s)} s`;
  return `${Math.floor(s / 60)} min ${Math.round(s % 60)} s`;
}

function cellName(nb: NotebookLike, id: string): string {
  const defined = Object.keys(nb.cell_dependencies?.[id]?.downstream_cells_map ?? {});
  if (defined.length) return defined.slice(0, 2).join(", ") + (defined.length > 2 ? ", …" : "");
  const first = (nb.cell_inputs[id]?.code ?? "").split("\n").find((l) => l.trim()) ?? "";
  const line = first.trim();
  return line.length > 28 ? `${line.slice(0, 27)}…` : line;
}

export function statusModel(nb: NotebookLike): StatusModel {
  const tree = nb.status_tree?.subtasks ?? {};
  const pkgTask = tree.pkg;
  const pkgPhase = phaseOf(pkgTask);
  const runTask = tree.run;
  const workspace = phaseOf(tree.workspace);
  const running = nb.process_status === "ready" || nb.process_status === "starting";
  const steps: StatusModel["steps"] = [
    { name: "Start Julia", phase: tree.workspace ? workspace : running ? "done" : "waiting" },
    { name: "Packages", phase: pkgTask ? pkgPhase : running ? "done" : "waiting" },
    { name: "Run cells", phase: phaseOf(runTask) },
  ];

  const nbpkg = nb.nbpkg ?? {};
  const installed = nbpkg.installed_versions ?? {};
  const busy = new Set(nbpkg.busy_packages ?? []);
  const log = parsePkgLog(nbpkg.terminal_outputs?.nbpkg_sync ?? "");
  const precompiled = new Set(log.precompiled);
  const failedSet = new Set(log.failed);
  const precompiling = phaseOf(pkgTask?.subtasks?.precompile) === "busy";

  const codes = nb.cell_order.map((id) => nb.cell_inputs[id]?.code ?? "");
  const direct = importedPackages(codes);
  for (const name of [...Object.keys(installed), ...busy].sort()) {
    if (!name.startsWith("__internal") && name !== "nbpkg_sync" && !direct.includes(name)) direct.push(name);
  }
  const packages: PkgRow[] = direct.map((name) => {
    const version = installed[name];
    const detail = version === "stdlib" ? "standard library" : version ?? "";
    let state: PkgState;
    if (pkgPhase === "failed" && (failedSet.has(name) || (failedSet.size === 0 && busy.has(name)))) state = "failed";
    else if (busy.has(name) && pkgPhase === "busy") state = precompiled.has(name) ? "ready" : precompiling ? "precompiling" : "installing";
    else if (version != null) state = "ready";
    // Pluto manages the environment but couldn't add this one: Pkg can't find it.
    else if (pkgPhase === "done") state = Object.keys(installed).length ? "failed" : "ready";
    else state = "waiting";
    const notFound = state === "failed" && version == null && pkgPhase === "done";
    return { name, state, detail: state === "ready" ? detail : notFound ? "not found" : "" };
  });

  const deps = log.added.filter((n) => !direct.includes(n));
  const depsRow = deps.length
    ? { count: deps.length, precompiled: deps.filter((n) => precompiled.has(n)).length, failed: deps.filter((n) => failedSet.has(n)).length }
    : null;

  const cells: CellRow[] = nb.cell_order.map((id) => {
    const r = nb.cell_results[id] ?? {};
    const state: CellState = r.running ? "running" : r.queued ? "waiting" : r.errored ? "failed" : r.runtime != null ? "done" : "waiting";
    const time = (state === "done" || state === "failed") && r.runtime != null ? prettyTime(r.runtime) : null;
    return { id, name: cellName(nb, id), state, time };
  });

  const failedPkg = packages.find((p) => p.state === "failed");
  // Without the package, the cells that use it error; those are the ones to name.
  const failure = failedPkg ? { name: failedPkg.name, cells: cells.filter((c) => c.state === "failed").map((c) => c.name) } : null;

  if (failedPkg && steps[1].phase === "done") steps[1].phase = "failed";
  const readyPkgs = packages.filter((p) => p.state === "ready").length;
  const evaluate = runTask?.subtasks?.evaluate?.subtasks ?? {};
  const runTotal = Object.keys(evaluate).length;
  const runDone = Object.values(evaluate).filter((e) => e.finished_at != null).length;
  let busyText: string | null = null;
  if (steps[0].phase === "busy") busyText = "Starting Julia";
  else if (pkgPhase === "busy") busyText = packages.length ? `Installing packages · ${readyPkgs} of ${packages.length}` : "Installing packages";
  else if (steps[2].phase === "busy") busyText = runTotal ? `Running ${runDone} of ${runTotal}` : "Running";

  const restart = nbpkg.restart_required_msg ? "required" : nbpkg.restart_recommended_msg ? "recommended" : null;
  let headline: string;
  if (nb.process_status === "waiting_for_permission") headline = "Safe preview · nothing has run";
  else if (failure) headline = `Package failed · ${failure.name}`;
  else if (busyText?.startsWith("Running")) headline = `Running cells · ${runDone} of ${runTotal}`;
  else if (busyText) headline = busyText;
  else if (restart === "required") headline = "Restart needed";
  else if (nb.process_status === "no_process" || nb.process_status === "waiting_to_restart") headline = "Julia stopped";
  else headline = "Ready";

  return {
    headline,
    steps,
    packages,
    deps: depsRow,
    cells,
    failure,
    busy: busyText,
    saveFailed: tree.saving?.success === false,
    restart,
  };
}
