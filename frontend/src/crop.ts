import type { Box } from "./quote";

const SIDE_MAX = 2400;

const OURS = "#annotate-frame, #annotate-box, #annotate-tag, #annotate-hint, #annotate-bar, #endeavor-bar, #endeavor-sent";

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

function inBox(box: Box): { el: Element; left: number; top: number; width: number; height: number }[] {
  const found = [];
  for (const el of document.querySelectorAll("img, canvas, svg:not(svg svg)")) {
    if (el.closest(OURS)) continue;
    const r = el.getBoundingClientRect();
    const [left, top] = [r.left + window.scrollX, r.top + window.scrollY];
    if (!r.width || !r.height || left >= box.right || left + r.width <= box.left || top >= box.bottom || top + r.height <= box.top) continue;
    found.push({ el, left, top, width: r.width, height: r.height });
  }
  return found;
}

export function mayDraw(box: Box): boolean {
  if (box.right - box.left < 2 || box.bottom - box.top < 2) return false;
  return inBox(box).some(({ el }) =>
    el instanceof SVGSVGElement || (el instanceof HTMLImageElement ? el.complete && el.naturalWidth > 0 && readable(el) : el instanceof HTMLCanvasElement && el.width > 0 && el.height > 0 && readable(el)),
  );
}

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
