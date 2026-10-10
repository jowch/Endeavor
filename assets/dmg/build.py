#!/usr/bin/env python3
"""Regenerate the Mac disk image's window background from the splash scene.

Writes, next to this script, background.png (640 x 440) and background@2x.png;
dmgbuild combines them (settings.py, used by .github/workflows/nightly.yml).

The top is the setup window's sky (src/splash.rs, src/turtle.rs): the turtle
standing on the horizon looking up at the stars, with the name and tagline.
The bottom is light ground where the app and the Applications link sit:
Finder draws their labels in black whatever the background, so they need a
light place to stand.

Needs Chrome or Chromium (set CHROME to its binary if it isn't found).
"""
import glob
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FONTS = os.path.join(HERE, "..", "..", "fonts")
W, H = 640, 440

PAGE = """<!doctype html>
<html><head><style>
@font-face { font-family: EndeavorSans; src: url("file://%(fonts)s/EndeavorSans-SemiBold.ttf"); font-weight: 600; }
@font-face { font-family: EndeavorSans; src: url("file://%(fonts)s/EndeavorSans-Regular.ttf"); font-weight: 400; }
html, body { margin: 0; background: #F3F3F5; }
canvas { display: block; width: %(w)dpx; height: %(h)dpx; }
</style></head><body><canvas width="%(cw)d" height="%(ch)d"></canvas><script>
// Colors and turtle geometry from src/theme.rs and src/turtle.rs.
const SKY = '#151517', SKY_TEXT = '#ECECEC', SKY_MUTED = '#8C8C8C', GROUND = '#F3F3F5', ARROW = '#B4B4BA';
const SHELL = '#CC3F00', SKIN = '#E08A5E', EYE = '#151517', STAR = '#F2E6D0';
const HEAD_R = 0.36, HEAD_X = 1.02, HEAD_Y = -0.14, FOOT_R = 0.21, FEET_X = [-0.6, 0.4];
// The splash's "looking at the stars" gaze, and its stars (src/splash.rs).
const GAZE = { eyeX: 1, look: 1, dx: 0.06, dy: -0.28 };
const STARS = [[1.5, -2.7, 3.7], [2.4, -2.1, 2.8], [0.8, -3.2, 3], [3, -3, 4.6], [2, -3.6, 2.5], [-0.3, -2.9, 2.3], [3.6, -2.3, 2.5]];
const HORIZON = 222, R = 34, ICON_Y = 300;

const canvas = document.querySelector('canvas');
const ctx = canvas.getContext('2d');
ctx.scale(canvas.width / %(w)d, canvas.height / %(h)d);
const disc = (x, y, rx, ry, color) => { ctx.fillStyle = color; ctx.beginPath(); ctx.ellipse(x, y, rx, ry, 0, 0, 2 * Math.PI); ctx.fill(); };
const halfDisc = (x, y, rx, ry, from, color) => { ctx.fillStyle = color; ctx.beginPath(); ctx.ellipse(x, y, rx, ry, 0, from, from + Math.PI); ctx.closePath(); ctx.fill(); };

ctx.fillStyle = SKY; ctx.fillRect(0, 0, %(w)d, HORIZON);
ctx.fillStyle = GROUND; ctx.fillRect(0, HORIZON, %(w)d, %(h)d - HORIZON);

const gx = %(w)d / 2 - 0.2 * R, gy = HORIZON;
STARS.forEach(([dx, dy, size], i) => {
  ctx.globalAlpha = 0.7 + 0.3 * Math.sin(i * 1.7);
  const r = (size * R) / 60;
  disc(gx + dx * R, gy + dy * R, r, r, STAR);
});
ctx.globalAlpha = 1;
FEET_X.forEach((fx) => halfDisc(gx + fx * R, gy - FOOT_R * R, FOOT_R * R, FOOT_R * R, 0, SKIN));
const base = -FOOT_R, hr = HEAD_R * R;
const hx = gx + (HEAD_X + GAZE.dx) * R, hy = gy + (base + HEAD_Y + GAZE.dy) * R;
halfDisc(gx, gy + base * R, R, R, Math.PI, SHELL);
disc(hx, hy, hr, hr, SKIN);
const er = hr * 0.13;
disc(hx + hr * 0.36 * GAZE.eyeX, hy - hr * (0.16 + 0.22 * GAZE.look), er, er, EYE);

ctx.textAlign = 'center';
document.fonts.load('600 26px EndeavorSans').then(() => document.fonts.load('400 13px EndeavorSans')).then(() => {
  ctx.fillStyle = SKY_TEXT; ctx.font = '600 26px EndeavorSans'; ctx.fillText('Endeavor', %(w)d / 2, 52);
  ctx.fillStyle = SKY_MUTED; ctx.font = '400 13px EndeavorSans'; ctx.fillText('Build our future', %(w)d / 2, 74);
  // An arrow from the app (x 180) to Applications (x 460), between the icons.
  ctx.strokeStyle = ARROW; ctx.fillStyle = ARROW; ctx.lineWidth = 2; ctx.lineCap = 'round';
  ctx.beginPath(); ctx.moveTo(262, ICON_Y); ctx.lineTo(370, ICON_Y); ctx.stroke();
  ctx.beginPath(); ctx.moveTo(380, ICON_Y); ctx.lineTo(368, ICON_Y - 7); ctx.lineTo(368, ICON_Y + 7); ctx.closePath(); ctx.fill();
  document.title = 'done';
});
</script></body></html>
"""


def chrome():
    if os.environ.get("CHROME"):
        return os.environ["CHROME"]
    for name in ("google-chrome", "chromium", "chromium-browser"):
        if shutil.which(name):
            return shutil.which(name)
    mac = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    found = [mac] if os.path.exists(mac) else glob.glob("/opt/pw-browsers/chromium-*/chrome-linux/chrome")
    if not found:
        sys.exit("No Chrome or Chromium found; set CHROME.")
    return found[0]


def render(scale, out):
    with tempfile.TemporaryDirectory() as tmp:
        page = os.path.join(tmp, "page.html")
        with open(page, "w") as f:
            f.write(PAGE % {"fonts": os.path.abspath(FONTS), "w": W, "h": H, "cw": W * scale, "ch": H * scale})
        subprocess.run([chrome(), "--headless", "--disable-gpu", "--no-sandbox", "--hide-scrollbars", "--allow-file-access-from-files",
                        f"--force-device-scale-factor={scale}", f"--window-size={W},{H}", "--virtual-time-budget=3000",
                        f"--screenshot={out}", "file://" + page], check=True, capture_output=True)
    print(out)


render(1, os.path.join(HERE, "background.png"))
render(2, os.path.join(HERE, "background@2x.png"))
