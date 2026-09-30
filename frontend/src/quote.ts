// Quotes from the notebook page: what Point picks and what Reply quotes from a
// selection go to the app as one "quote" message (src/annotate.rs turns it
// into attach::Quote). A figure or a box goes with a picture the app takes of
// the viewport, one at a time: the page brings it into view, hides its own
// overlay, and waits for the app's "shot".

import { on, send } from "./bridge";

/** A box in page coordinates (scrolls with the notebook). */
export type Box = { left: number; top: number; right: number; bottom: number };

/** One quote, as the app parses it (`pick_quote` in src/annotate.rs). */
export type Pick =
  | { part: "cell"; cell: string; code: string }
  | { part: "lines"; cell: string; code: string; lines: [number, number]; text: string }
  | { part: "output"; cell: string; code: string; text: string }
  | { part: "figure"; cell: string; code: string; shot?: number }
  | { part: "box"; cells: string[]; shot?: number };

/** While the app takes a picture, the notebook shows as it is. */
export const SHOOTING = "endeavor-shooting";

const waiting = new Map<number, () => void>();
let nextShot = 1;

const nextFrame = () => new Promise<void>((done) => requestAnimationFrame(() => done()));

/** Take a picture of `b` (page coordinates); resolves with its id once the app has it. */
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
  // The app may never answer (no picture taken): go on without it.
  await new Promise<void>((done) => {
    waiting.set(id, done);
    setTimeout(done, 3000);
    send({ type: "shoot", id, rect });
  });
  waiting.delete(id);
  document.body.classList.remove(SHOOTING);
  return id;
}

/** Send picks and the comment: now as a message, or (`add`) into the composer's. */
export function sendQuote(picks: Pick[], comment: string, add: boolean): void {
  const notebook = new URLSearchParams(location.search).get("id");
  send({ type: "quote", notebook, picks, comment, add });
}

export function initQuote(): void {
  on("shot", (msg) => waiting.get(msg.id)?.());
}
