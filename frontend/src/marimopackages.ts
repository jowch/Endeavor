import { on } from "./bridge";

export type PackageState = "waiting" | "installing" | "ready" | "failed";

export interface Environment {
  runtime: string;
  source: string | null;
  packages: { name: string; version: string | null; state: PackageState; why?: string }[];
  dependencies: { count: number; done: number; failed: number } | null;
  install: { done: number; total: number } | null;
  failure: { name: string; why: string; cells: string[]; log: string } | null;
  restart: "required" | "recommended" | null;
}

export type MarimoEnvironment = {
  python: string | null;
  packages: { name: string; version: string | null }[];
  package_date: string | null;
  install: { step: "asking" | "installing"; packages: string[] } | null;
  failed: { packages: string[]; log: string } | null;
};

const canonical = (name: string) => name.toLowerCase().replace(/[-_.]+/g, "-");

export function pythonLine(requirement: string | null): string {
  const lowest = requirement?.match(/^\s*>=\s*([\d.]+)\s*$/);
  if (lowest) return `Python ${lowest[1]} or later`;
  return requirement ? `Python ${requirement.trim()}` : "Python";
}

export function lastLogLine(log: string): string {
  const lines = log.split("\n").map((l) => l.replace(/\x1b\[[0-9;]*m/g, "").trim()).filter(Boolean);
  const telling = lines.filter((l) => /error|not found|no solution|because/i.test(l));
  return telling[telling.length - 1] ?? lines[lines.length - 1] ?? "";
}

export function fromMarimo(raw: MarimoEnvironment): Environment {
  const named = (list: string[] | undefined) => new Set((list ?? []).map(canonical));
  const failed = named(raw.failed?.packages);
  const installing = named(raw.install?.step === "installing" ? raw.install.packages : []);
  const asking = named(raw.install?.step === "asking" ? raw.install.packages : []);
  const why = raw.failed ? lastLogLine(raw.failed.log) : "";
  const state = (name: string): PackageState =>
    failed.has(name) ? "failed" : installing.has(name) ? "installing" : asking.has(name) ? "waiting" : "ready";
  const listed = new Set(raw.packages.map((p) => canonical(p.name)));
  const added = [...raw.failed?.packages ?? [], ...raw.install?.packages ?? []].filter((n, i, all) => !listed.has(canonical(n)) && all.findIndex((m) => canonical(m) === canonical(n)) === i);
  const packages = [...raw.packages, ...added.map((name) => ({ name, version: null }))].map((p) => {
    const s = state(canonical(p.name));
    return s === "failed" ? { ...p, state: s, why } : { ...p, state: s };
  });
  return {
    runtime: pythonLine(raw.python),
    source: raw.package_date ? `PyPI as of ${raw.package_date}` : null,
    packages,
    dependencies: null,
    install: installing.size ? { done: 0, total: installing.size } : null,
    failure: raw.failed ? { name: raw.failed.packages.join(", "), why, cells: [], log: raw.failed.log } : null,
    restart: null,
  };
}

let latest: Environment | null = null;

export function marimoEnvironment(): Environment | null {
  return latest;
}

export function initMarimoPackages(): void {
  on("packages", (msg) => {
    latest = fromMarimo(msg.environment);
  });
}
