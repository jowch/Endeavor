import { on, send } from "./bridge";
import { drawerTab, showModel } from "./drawer";
import type { PkgRow, PkgState, StatusModel } from "./status";

export type MarimoEnvironment = {
  python: string | null;
  python_version?: string | null;
  packages: { name: string; version: string | null }[];
  package_date: string | null;
  install: { step: "asking" | "installing"; packages: string[] } | null;
  failed: { packages: string[]; log: string } | null;
};

const canonical = (name: string) => name.toLowerCase().replace(/[-_.]+/g, "-");

export function pythonLine(requirement: string | null, found?: string | null): string {
  if (found) return `Python ${found}`;
  const lowest = requirement?.match(/^\s*>=\s*([\d.]+)\s*$/);
  if (lowest) return `Python ${lowest[1]} or later`;
  return requirement ? `Python ${requirement.trim()}` : "Python";
}

export function lastLogLine(log: string): string {
  const lines = log.split("\n").map((l) => l.replace(/\x1b\[[0-9;]*m/g, "").trim()).filter(Boolean);
  const telling = lines.filter((l) => /error|not found|no solution|because/i.test(l));
  return telling[telling.length - 1] ?? lines[lines.length - 1] ?? "";
}

export function marimoModel(raw: MarimoEnvironment): StatusModel {
  const named = (list: string[] | undefined) => new Set((list ?? []).map(canonical));
  const failed = named(raw.failed?.packages);
  const installing = named(raw.install?.step === "installing" ? raw.install.packages : []);
  const asking = named(raw.install?.step === "asking" ? raw.install.packages : []);
  const state = (name: string): PkgState =>
    failed.has(name) ? "failed" : installing.has(name) ? "installing" : asking.has(name) ? "waiting" : "ready";
  const listed = new Set(raw.packages.map((p) => canonical(p.name)));
  const added = [...(raw.failed?.packages ?? []), ...(raw.install?.packages ?? [])].filter(
    (n, i, all) => !listed.has(canonical(n)) && all.findIndex((m) => canonical(m) === canonical(n)) === i,
  );
  const packages: PkgRow[] = [...raw.packages, ...added.map((name) => ({ name, version: null }))].map((p) => {
    const s = state(canonical(p.name));
    return { name: p.name, state: s, detail: s === "ready" ? (p.version ?? "") : "" };
  });
  const said = raw.failed ? lastLogLine(raw.failed.log) : "";
  const failure = raw.failed
    ? { name: raw.failed.packages.join(", "), why: said ? `couldn't be installed: ${said}` : "couldn't be installed.", cells: [], log: raw.failed.log }
    : null;
  const install = installing.size ? { done: 0, total: 0 } : null;
  const busy = install ? "Installing packages" : null;
  const runtime = [pythonLine(raw.python, raw.python_version), "uv", raw.package_date ? `PyPI as of ${raw.package_date}` : null].filter(Boolean).join(" · ");
  return {
    headline: failure ? `Package failed · ${failure.name}` : busy ?? "Ready",
    runtime,
    packages,
    deps: null,
    install,
    failure,
    log: raw.failed?.log ?? "",
    busy,
    saveFailed: false,
    restart: null,
  };
}

let model: StatusModel | null = null;
let notebook = "";
let sent = "";

function report(): void {
  if (!notebook || !model) return;
  const msg = {
    type: "state" as const,
    notebook,
    safe: false,
    busy: model.busy,
    installing: model.install !== null,
    restart: model.restart,
    save_failed: false,
    package_failed: model.failure?.name ?? null,
    dead: false,
    connected: true,
    drawer: drawerTab(),
  };
  const json = JSON.stringify(msg);
  if (json === sent) return;
  sent = json;
  send(msg);
}

export function initMarimoPackages(): void {
  const changed = showModel(() => model);
  on("context", (msg) => {
    notebook = msg.notebook ?? notebook;
    report();
  });
  on("packages", (msg) => {
    model = marimoModel(msg.environment);
    changed(model);
    report();
  });
  new MutationObserver(report).observe(document.documentElement, { attributes: true, attributeFilter: ["data-endeavor-drawer"] });
}
