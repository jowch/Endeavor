export type Rect = { left: number; top: number; right: number; bottom: number };

export function pillPlace(lines: Rect[], viewportHeight: number): { left: number; top: number } {
  const PILL = 30;
  const last = lines[lines.length - 1];
  const below = last.bottom + 6;
  const top = below + PILL <= viewportHeight ? below : lines[0].top - 6 - PILL;
  return { left: Math.max(last.right - 28, 4), top };
}

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
