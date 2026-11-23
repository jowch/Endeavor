// A picture of a box the user drew with Point in Claude's notebook view (view.ts). The app takes pictures
// natively; the view's frame can't take a screenshot, so it draws what it can of the box itself: images,
// SVG and canvases. Text, HTML and WebGL (drawn without preserveDrawingBuffer, as Makie's and plotly's 3D
// views are) don't come out, and an image from another site can't be read back (headless, 2026-11-23).
// An SVG is drawn from a copy on its own, so the page's CSS and web fonts don't reach it: a figure styled
// by the page or set in a web font comes out in its own attributes' look and a fallback font.

import type { Box } from "./quote";

/** The most pixels a side of the picture gets, after the screen's scale. */
const SIDE_MAX = 2400;

/** Point's own marks and the view's chrome, which the picture leaves out. */
const OURS = "#annotate-frame, #annotate-box, #annotate-tag, #annotate-hint, #annotate-bar, #endeavor-bar, #endeavor-sent";

/** `el` as something a canvas draws, or null. */
async function drawable(el: Element): Promise<CanvasImageSource | null> {
  if (el instanceof HTMLImageElement) return el.complete && el.naturalWidth ? el : null;
  if (el instanceof HTMLCanvasElement) return el.width && el.height ? el : null;
  if (el instanceof SVGSVGElement) {
    const copy = el.cloneNode(true) as SVGSVGElement;
    const r = el.getBoundingClientRect();
    copy.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    copy.setAttribute("width", String(r.width));
    copy.setAttribute("height", String(r.height));
    const img = new Image();
    img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(new XMLSerializer().serializeToString(copy));
    try {
      await img.decode();
      return img;
    } catch {
      return null;
    }
  }
  return null;
}

/** Whether drawing `source` leaves a canvas readable and shows anything (a WebGL canvas reads back blank). */
function readable(source: CanvasImageSource): boolean {
  const test = document.createElement("canvas");
  test.width = test.height = 8;
  const g = test.getContext("2d");
  if (!g) return false;
  try {
    g.drawImage(source, 0, 0, 8, 8);
    const data = g.getImageData(0, 0, 8, 8).data;
    for (let i = 3; i < data.length; i += 4) if (data[i]) return true;
    return false;
  } catch {
    return false;
  }
}

/** The images, canvases and outer SVGs in `box` (page coordinates), with their page rects, leaving out the view's own. */
function inBox(box: Box): { el: Element; left: number; top: number; width: number; height: number }[] {
  const found = [];
  // Nested svgs come with their outer one.
  for (const el of document.querySelectorAll("img, canvas, svg:not(svg svg)")) {
    if (el.closest(OURS)) continue;
    const r = el.getBoundingClientRect();
    const [left, top] = [r.left + window.scrollX, r.top + window.scrollY];
    if (!r.width || !r.height || left >= box.right || left + r.width <= box.left || top >= box.bottom || top + r.height <= box.top) continue;
    found.push({ el, left, top, width: r.width, height: r.height });
  }
  return found;
}

/**
 * Whether anything in `box` may draw, told at once, before the message goes: an image or canvas that draws
 * readable, or an SVG (which only drawing tells). False for a box over text alone, which gets no picture.
 */
export function mayDraw(box: Box): boolean {
  if (box.right - box.left < 2 || box.bottom - box.top < 2) return false;
  return inBox(box).some(({ el }) =>
    el instanceof SVGSVGElement || (el instanceof HTMLImageElement ? el.complete && el.naturalWidth > 0 && readable(el) : el instanceof HTMLCanvasElement && el.width > 0 && el.height > 0 && readable(el)),
  );
}

/**
 * A PNG (base64, no `data:` prefix) of what the view can draw in `box` (page coordinates), on the page's
 * background, or null when nothing in it can be drawn.
 */
export async function drawBox(box: Box): Promise<string | null> {
  const width = box.right - box.left;
  const height = box.bottom - box.top;
  if (width < 2 || height < 2) return null;
  const scale = Math.min(window.devicePixelRatio || 1, SIDE_MAX / Math.max(width, height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(width * scale);
  canvas.height = Math.round(height * scale);
  const g = canvas.getContext("2d");
  if (!g) return null;
  g.fillStyle = getComputedStyle(document.body).backgroundColor || "white";
  g.fillRect(0, 0, canvas.width, canvas.height);
  g.scale(scale, scale);
  let drawn = 0;
  for (const { el, left, top, width: w, height: h } of inBox(box)) {
    const source = await drawable(el);
    if (!source || !readable(source)) continue;
    g.drawImage(source, left - box.left, top - box.top, w, h);
    drawn++;
  }
  if (!drawn) return null;
  try {
    return canvas.toDataURL("image/png").replace(/^data:image\/png;base64,/, "");
  } catch {
    return null;
  }
}
