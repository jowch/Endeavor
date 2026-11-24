import { cellName as cellLabel, commentName } from "./cellname";

export type Phase = "waiting" | "busy" | "done" | "failed";
export type PkgState = "waiting" | "installing" | "precompiling" | "ready" | "failed";

export interface StatusEntry {
  success?: boolean | null;
  started_at: number | null;
  finished_at: number | null;
  subtasks: Record<string, StatusEntry>;
}

export interface EmberPackageRow {
  name: string;
  version?: string | null;
  direct?: boolean;
  status: string;
  message?: string | null;
}

export interface EmberPackages {
  snapshot?: string | null;
  r_version?: string | null;
  library?: {
    status?: string;
    message?: string | null;
    progress?: { done?: number; total?: number; current?: string } | null;
    log?: string | null;
    failures?: Array<{ package: string; kind?: string; detail?: string | null }>;
  };
  rows?: EmberPackageRow[];
}

export interface NotebookLike {
  process_status: string;
  status_tree: StatusEntry | null;
  julia_version?: string;
  nbpkg: {
    busy_packages?: string[];
    installed_versions?: Record<string, string>;
    terminal_outputs?: Record<string, string>;
    restart_required_msg?: string | null;
    restart_recommended_msg?: string | null;
  } | null;
  ember?: { packages?: EmberPackages | null } | null;
  cell_order: string[];
  cell_inputs: Record<string, { code: string }>;
  cell_results: Record<string, { running?: boolean; queued?: boolean; errored?: boolean; runtime?: number | null }>;
  cell_dependencies?: Record<string, { downstream_cells_map?: Record<string, unknown> }>;
}

export interface PkgRow {
  name: string;
  state: PkgState;
  detail: string;
}

export interface Failure {
  name: string;
  why: string;
  cells: string[];
  log: string;
}

export interface StatusModel {
  headline: string;
  runtime: string | null;
  packages: PkgRow[];
  deps: { count: number; done: number; failed: number } | null;
  install: { done: number; total: number } | null;
  failure: Failure | null;
  log: string;
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
const plain = (log: string | null | undefined) => (log ?? "").replace(ansi, "");

export function parsePkgLog(log: string): { precompiled: string[]; failed: string[]; added: string[] } {
  const precompiled: string[] = [];
  const failed: string[] = [];
  const added: string[] = [];
  let inManifest = false;
  for (const raw of plain(log).split("\n")) {
    const line = raw.trimEnd();
    const mark = line.match(/^\s*(?:[\d.]+\s*ms)?\s*([✓✗])\s+([\w.]+)/);
    if (mark) (mark[1] === "✓" ? precompiled : failed).push(mark[2]);
    if (/^\s*Updating\s+`.*Manifest\.toml`/.test(line)) {
      inManifest = true;
      continue;
    }
    const entry = line.match(/\[[0-9a-f]{8}\]\s+(\+)?\s*([\w.]+)/);
    if (!entry) inManifest = false;
    else if (inManifest && entry[1]) added.push(entry[2]);
  }
  return { precompiled, failed, added };
}

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

export function definedNames(map: Record<string, unknown> | undefined): string[] {
  return Object.keys(map ?? {}).filter((n) => !n.startsWith("__ExprExpl_anon__"));
}

function cellName(nb: NotebookLike, id: string): string {
  const named = commentName(nb.cell_inputs[id]?.code ?? "");
  if (named) return cellLabel(nb.cell_inputs[id].code);
  const defined = definedNames(nb.cell_dependencies?.[id]?.downstream_cells_map);
  if (defined.length) return defined.slice(0, 2).join(", ") + (defined.length > 2 ? ", …" : "");
  return cellLabel(nb.cell_inputs[id]?.code ?? "");
}

const failedCells = (nb: NotebookLike) => nb.cell_order.filter((id) => nb.cell_results[id]?.errored).map((id) => cellName(nb, id));

function restartOf(nb: NotebookLike): StatusModel["restart"] {
  return nb.nbpkg?.restart_required_msg ? "required" : nb.nbpkg?.restart_recommended_msg ? "recommended" : null;
}

function headlineOf(nb: NotebookLike, language: string, failure: Failure | null, busy: string | null, restart: StatusModel["restart"]): string {
  if (nb.process_status === "waiting_for_permission") return "Safe preview · nothing has run";
  if (failure) return `Package failed · ${failure.name}`;
  if (busy && !busy.startsWith("Running")) return busy;
  if (restart === "required") return "Restart needed";
  if (nb.process_status === "no_process" || nb.process_status === "waiting_to_restart") return `${language} stopped`;
  return "Ready";
}

export function statusModel(nb: NotebookLike): StatusModel {
  return nb.ember?.packages ? emberModel(nb, nb.ember.packages) : plutoModel(nb);
}

function plutoModel(nb: NotebookLike): StatusModel {
  const tree = nb.status_tree?.subtasks ?? {};
  const pkgTask = tree.pkg;
  const pkgPhase = phaseOf(pkgTask);
  const nbpkg = nb.nbpkg ?? {};
  const installed = nbpkg.installed_versions ?? {};
  const busySet = new Set(nbpkg.busy_packages ?? []);
  const fullLog = plain(nbpkg.terminal_outputs?.nbpkg_sync);
  const log = parsePkgLog(fullLog);
  const precompiled = new Set(log.precompiled);
  const failedSet = new Set(log.failed);
  const precompiling = phaseOf(pkgTask?.subtasks?.precompile) === "busy";

  const direct = importedPackages(nb.cell_order.map((id) => nb.cell_inputs[id]?.code ?? ""));
  for (const name of [...Object.keys(installed), ...busySet].sort()) {
    if (!name.startsWith("__internal") && name !== "nbpkg_sync" && !direct.includes(name)) direct.push(name);
  }
  const packages: PkgRow[] = direct.map((name) => {
    const version = installed[name];
    const detail = version === "stdlib" ? "standard library" : version ?? "";
    let state: PkgState;
    if (pkgPhase === "failed" && (failedSet.has(name) || (failedSet.size === 0 && busySet.has(name)))) state = "failed";
    else if (busySet.has(name) && pkgPhase === "busy") state = precompiled.has(name) ? "ready" : precompiling ? "precompiling" : "installing";
    else if (version != null) state = "ready";
    else if (pkgPhase === "done") state = Object.keys(installed).length ? "failed" : "ready";
    else state = "waiting";
    const notFound = state === "failed" && version == null && pkgPhase === "done";
    return { name, state, detail: state === "ready" ? detail : notFound ? "not found" : "" };
  });

  const added = log.added.filter((n) => !direct.includes(n));
  const deps = added.length
    ? { count: added.length, done: added.filter((n) => precompiled.has(n)).length, failed: added.filter((n) => failedSet.has(n)).length }
    : null;

  const failedPkg = packages.find((p) => p.state === "failed");
  const failure: Failure | null = failedPkg
    ? {
        name: failedPkg.name,
        why: failedPkg.detail === "not found" ? "isn't a package Pkg can find: a typo, or not in the registry." : "couldn't be installed or precompiled.",
        cells: failedCells(nb),
        log: plain(nbpkg.terminal_outputs?.[failedPkg.name]) || fullLog,
      }
    : null;

  const readyPkgs = packages.filter((p) => p.state === "ready").length;
  const install = pkgPhase === "busy" ? { done: readyPkgs, total: packages.length } : null;
  const evaluate = tree.run?.subtasks?.evaluate?.subtasks ?? {};
  const runTotal = Object.keys(evaluate).length;
  const runDone = Object.values(evaluate).filter((e) => e.finished_at != null).length;
  let busy: string | null = null;
  if (phaseOf(tree.workspace) === "busy") busy = "Starting Julia";
  else if (install) busy = packages.length ? `Installing packages · ${readyPkgs} of ${packages.length}` : "Installing packages";
  else if (phaseOf(tree.run) === "busy") busy = runTotal ? `Running ${runDone} of ${runTotal}` : "Running";

  const restart = restartOf(nb);
  const version = String(nb.julia_version ?? "").replace(/^v/, "");
  return {
    headline: headlineOf(nb, "Julia", failure, busy, restart),
    runtime: version ? `Julia ${version}` : null,
    packages,
    deps,
    install,
    failure,
    log: fullLog,
    busy,
    saveFailed: tree.saving?.success === false,
    restart,
  };
}

const emberCouldntLook = new Set(["lookup_failed", "source_failed"]);

function emberRow(row: EmberPackageRow): PkgRow {
  if (row.status === "installed") return { name: row.name, state: "ready", detail: row.version ?? "" };
  if (row.status === "installing") return { name: row.name, state: "installing", detail: "" };
  if (row.status === "not_found") return { name: row.name, state: "failed", detail: "not found" };
  if (emberCouldntLook.has(row.status)) return { name: row.name, state: "failed", detail: "couldn't look up" };
  if (row.status === "failed") return { name: row.name, state: "failed", detail: "" };
  return { name: row.name, state: "waiting", detail: "" };
}

export const withoutRWarnings = (message: string | null | undefined): string =>
  plain(message).replace(/\s*In addition: Warning messages?:[\s\S]*$/, "").trim();

export function installerReason(message: string | null | undefined): string {
  const lines = withoutRWarnings(message).split("\n").map((l) => l.trim()).filter(Boolean);
  const first = (lines[0] ?? "").replace(/^install failed[:,]?\s*/, "");
  return first || lines[1] || "";
}

function emberWhy(row: EmberPackageRow | undefined, libraryMessage: string | null | undefined): string {
  if (row?.status === "not_found") return "isn't a package R can find: a typo, or not on CRAN or Bioconductor.";
  if (row && emberCouldntLook.has(row.status)) return row.message ? `couldn't be looked up: ${withoutRWarnings(row.message)}` : "couldn't be looked up.";
  const reason = installerReason(row?.message ?? libraryMessage);
  return reason ? `couldn't be installed: ${reason}` : "couldn't be installed.";
}

function emberModel(nb: NotebookLike, pkgs: EmberPackages): StatusModel {
  const library = pkgs.library ?? {};
  const rows = pkgs.rows ?? [];
  const direct = rows.filter((r) => r.direct);
  const packages = direct.map(emberRow);
  const indirect = rows.filter((r) => !r.direct).map(emberRow);
  const deps = indirect.length
    ? { count: indirect.length, done: indirect.filter((r) => r.state === "ready").length, failed: indirect.filter((r) => r.state === "failed").length }
    : null;

  const log = plain(library.log);
  const failedRow = direct.find((r) => r.status === "failed" || r.status === "not_found" || emberCouldntLook.has(r.status));
  const failedName = failedRow?.name ?? (library.status === "failed" ? library.failures?.[0]?.package : undefined);
  const failure: Failure | null = failedName
    ? { name: failedName, why: emberWhy(failedRow, library.message), cells: failedCells(nb), log }
    : null;

  const progress = library.progress;
  const install =
    library.status !== "installing" ? null
    : progress?.total ? { done: progress.done ?? 0, total: progress.total }
    : { done: rows.filter((r) => r.status === "installed").length, total: rows.length };
  const waiting = nb.cell_order.filter((id) => nb.cell_results[id]?.running || nb.cell_results[id]?.queued).length;
  let busy: string | null = null;
  if (nb.process_status === "starting") busy = "Starting R";
  else if (install) busy = install.total ? `Installing packages · ${install.done} of ${install.total}` : "Installing packages";
  else if (library.status === "asking") busy = "Download waits for your OK";
  else if (waiting) busy = `Running · ${waiting} cell${waiting === 1 ? "" : "s"} left`;

  const restart = restartOf(nb);
  const r = pkgs.r_version ? `R ${pkgs.r_version}` : "R";
  return {
    headline: headlineOf(nb, "R", failure, busy, restart),
    runtime: pkgs.snapshot ? `${r} · CRAN snapshot ${pkgs.snapshot}` : r,
    packages,
    deps,
    install,
    failure,
    log,
    busy,
    saveFailed: false,
    restart,
  };
}
