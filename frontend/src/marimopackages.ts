import { on, send } from "./bridge";
import { drawerTab, showModel } from "./drawer";
import { onMarimo } from "./engine";
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

const plainLog = (log: string) => log.replace(/\x1b\[[0-9;]*m/g, "");

export function lastLogLine(log: string): string {
  const lines = plainLog(log).split("\n").map((l) => l.trim()).filter(Boolean);
  const telling = lines.filter((l) => /error|not found|no solution|because/i.test(l));
  return telling[telling.length - 1] ?? lines[lines.length - 1] ?? "";
}

export function failureReason(log: string): string {
  const text = plainLog(log);
  if (/not found in the package registry/.test(text)) return "it isn't in the package index";
  const because = text.match(/╰─▶\s*([^]*?)(?:\n\s*\n|\n\S|$)/);
  if (!because) return lastLogLine(log);
  const said = because[1].split("\n").map((l) => l.trim()).join(" ");
  const first = said.split(/, we can conclude|\.\s/)[0].replace(/^because\s+/i, "").replace(/\.$/, "");
  return first.charAt(0).toLowerCase() + first.slice(1);
}

export type LiveInstall = { id: string; packages: Record<string, string>; logs: Record<string, string>; status: string; error: string };

type Operation = {
  operation_id: string;
  action: string;
  source?: string;
  status?: { kind: string; error?: string };
  packages?: Record<string, string>;
  logs?: Record<string, string>;
};

export function applyOperation(prev: LiveInstall | null, op: Operation, mode: "append" | "replace"): LiveInstall | null {
  if (op.action !== "install" || op.source === "server") return prev;
  const logs = prev?.id === op.operation_id ? { ...prev.logs } : {};
  for (const [stream, text] of Object.entries(op.logs ?? {})) logs[stream] = mode === "replace" ? text : (logs[stream] ?? "") + text;
  return { id: op.operation_id, packages: op.packages ?? {}, logs, status: op.status?.kind ?? "running", error: op.status?.error ?? "" };
}

export function withLive(raw: MarimoEnvironment, live: LiveInstall | null): MarimoEnvironment {
  if (!live) return raw;
  const names = Object.keys(live.packages);
  if (live.status === "running") return { ...raw, install: { step: "installing", packages: names }, failed: null };
  if (raw.install?.step === "installing") return raw;
  const asking = raw.install?.step === "asking" ? raw.install : null;
  if (live.status === "failed") {
    const failing = names.filter((n) => live.packages[n] !== "succeeded");
    const ours = raw.failed?.packages ?? [];
    const theirs = (failing.length ? failing : names).filter((n) => !ours.some((m) => canonical(m) === canonical(n)));
    const log = [raw.failed?.log, ...Object.values(live.logs), live.error].filter(Boolean).join("\n");
    return { ...raw, install: asking, failed: { packages: [...ours, ...theirs], log } };
  }
  const installed = new Set(names.filter((n) => live.packages[n] === "succeeded").map(canonical));
  const stillFailed = raw.failed?.packages.some((n) => !installed.has(canonical(n)));
  return { ...raw, install: asking, failed: stillFailed ? raw.failed : null };
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
  const said = raw.failed ? failureReason(raw.failed.log) : "";
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

let raw: MarimoEnvironment | null = null;
let live: LiveInstall | null = null;
let model: StatusModel | null = null;
let changed: ((m: StatusModel) => void) | null = null;
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

function refresh(): void {
  if (!raw) return;
  model = marimoModel(withLive(raw, live));
  changed?.(model);
  report();
}

function heard(text: string): void {
  if (!text.startsWith('{"op": "environment-')) return;
  const { data } = JSON.parse(text) as { data: { op: string; log_mode?: "append" | "replace"; state?: { operations: Operation[] } } & Operation };
  if (data.op === "environment-operation") live = applyOperation(live, data, data.log_mode ?? "append");
  else if (data.op === "environment-state") for (const op of data.state?.operations ?? []) live = applyOperation(live, op, "replace");
  refresh();
}

function listenToMarimo(): void {
  window.WebSocket = new Proxy(window.WebSocket, {
    construct(Real, args) {
      const socket = Reflect.construct(Real, args) as WebSocket;
      socket.addEventListener?.("message", (e: MessageEvent) => {
        if (typeof e.data === "string") heard(e.data);
      });
      return socket;
    },
  });
}

const marimoInstallTitles = ["Installing packages", "All packages installed!", "Some packages failed to install"];

function hideMarimoInstallAlerts(): void {
  for (const alert of document.querySelectorAll<HTMLElement>("div.fixed.top-5.left-12")) {
    const title = alert.textContent?.trimStart() ?? "";
    if (marimoInstallTitles.some((t) => title.startsWith(t))) alert.style.display = "none";
  }
}

if (onMarimo()) listenToMarimo();

export function initMarimoPackages(): void {
  changed = showModel(() => model);
  on("context", (msg) => {
    notebook = msg.notebook ?? notebook;
    report();
  });
  on("packages", (msg) => {
    raw = msg.environment;
    refresh();
  });
  new MutationObserver(report).observe(document.documentElement, { attributes: true, attributeFilter: ["data-endeavor-drawer"] });
  new MutationObserver(hideMarimoInstallAlerts).observe(document.body, { childList: true, subtree: true });
}
