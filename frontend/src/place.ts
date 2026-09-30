// Where Reply's pill and Point's comment bar open, in viewport coordinates.
// Pure, so the rules can be checked without a page.

export type Rect = { left: number; top: number; right: number; bottom: number };

/** Reply's pill (30 px tall): 6 px under the selection's last line, starting
 * just left of where the selection ends; above its first line when there's
 * no room below. `lines` are the selection's line boxes, in order. */
export function pillPlace(lines: Rect[], viewportHeight: number): { left: number; top: number } {
  const PILL = 30;
  const last = lines[lines.length - 1];
  const below = last.bottom + 6;
  const top = below + PILL <= viewportHeight ? below : lines[0].top - 6 - PILL;
  return { left: Math.max(last.right - 28, 4), top };
}

/** Point's comment bar, `barHeight` tall, for a pick at `pick` in a pane
 * `width` × `height`: 8 px under the pick with its left edge on the pick's;
 * with no room below, 8 px above it; with no room either way, pinned to the
 * pane's bottom edge. It is 380 px wide (at most the pane minus 32 px), not
 * the pick's width, so it leaves the cells beside a wide pick in view. It
 * stays inside the pane, so once the pick scrolls out of view the bar waits
 * at the pane's edge. */
export function barPlace(pick: Rect, width: number, height: number, barHeight: number): { left: number; top: number; width: number } {
  const GAP = 8;
  const EDGE = 16;
  const barWidth = Math.max(Math.min(380, width - 32), 0);
  const left = Math.min(Math.max(pick.left, EDGE), width - EDGE - barWidth);
  const fitsBelow = pick.bottom + GAP + barHeight <= height - GAP;
  const fitsAbove = pick.top - GAP - barHeight >= GAP;
  const lowest = height - EDGE - barHeight;
  let top: number;
  if (pick.bottom < 0 || fitsBelow) top = pick.bottom + GAP;
  else if (pick.top > height || fitsAbove) top = pick.top - GAP - barHeight;
  else top = lowest;
  return { left: Math.max(left, EDGE), top: Math.min(Math.max(top, GAP), lowest), width: barWidth };
}
