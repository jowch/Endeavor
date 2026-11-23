import { on, send } from "./bridge";
import { cellName } from "./cellname";

export type Box = { left: number; top: number; right: number; bottom: number };

export type Pick =
  | { part: "cell"; cell: string; code: string }
  | { part: "lines"; cell: string; code: string; lines: [number, number]; text: string }
  | { part: "output"; cell: string; code: string; text: string; selected?: boolean }
  | { part: "figure"; cell: string; code: string; shot?: number }
  | { part: "box"; cells: string[]; shot?: number; rect?: Box };

export const SHOOTING = "endeavor-shooting";

const waiting = new Map<number, () => void>();
let nextShot = 1;

const nextFrame = () => new Promise<void>((done) => requestAnimationFrame(() => done()));

export async function shoot(b: Box): Promise<number> {
  const top = b.top - window.scrollY;
  const bottom = b.bottom - window.scrollY;
  if (top < 0 || bottom > window.innerHeight) window.scrollBy(0, top < 0 || bottom - top > window.innerHeight ? top - 8 : bottom - window.innerHeight + 8);
  document.body.classList.add(SHOOTING);
  await nextFrame();
  await nextFrame();
  const x = Math.max(0, b.left - window.scrollX);
  const y = Math.max(0, b.top - window.scrollY);
  const rect = {
    x,
    y,
    width: Math.min(window.innerWidth, b.right - window.scrollX) - x,
    height: Math.min(window.innerHeight, b.bottom - window.scrollY) - y,
  };
  const id = nextShot++;
  await new Promise<void>((done) => {
    waiting.set(id, done);
    setTimeout(done, 3000);
    send({ type: "shoot", id, rect });
  });
  waiting.delete(id);
  document.body.classList.remove(SHOOTING);
  return id;
}

export function pickSource(pick: Pick): string {
  if (pick.part === "box") return `Box · ${pick.cells.length} cell${pick.cells.length === 1 ? "" : "s"}`;
  const part =
    pick.part === "lines" ? (pick.lines[0] === pick.lines[1] ? `line ${pick.lines[0]}` : `lines ${pick.lines[0]}–${pick.lines[1]}`) : pick.part;
  return `${cellName(pick.code)} · ${part}`;
}

export function sendQuote(picks: Pick[], comment: string, add: boolean, ref?: number): void {
  const notebook = new URLSearchParams(location.search).get("id");
  send({ type: "quote", notebook, picks, comment, add, ...(ref === undefined ? {} : { ref }) });
}

export function initQuote(): void {
  on("shot", (msg) => waiting.get(msg.id)?.());
}
