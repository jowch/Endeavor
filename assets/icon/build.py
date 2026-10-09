#!/usr/bin/env python3
"""Regenerate the app icon and logo from the artwork defined here.

Writes, next to this script:
  icon.svg, icon-small.svg     the icon (1024 grid); the small art is for 16 and 32 px
  Endeavor.icns                the macOS icon (scripts/bundle.sh copies it into the app)
  endeavor-256.png, -512.png   for Linux desktop files
  endeavor.ico                 the Windows installer's and Start menu's icon
  logo-dark.svg, logo-light.svg  the sky badge + "Endeavor", text as outlines

Needs Chrome or Chromium (set CHROME to its binary if it isn't found),
macOS's sips and iconutil, fonttools + uharfbuzz (fonts/requirements.txt), and
Pillow for the .ico. With NO_MAC set it writes only the .ico.
"""
import glob
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FONT = os.path.join(HERE, "..", "..", "fonts", "EndeavorSans-SemiBold.ttf")


def squircle(x0=100, y0=100, w=824, r=185.4):
    """Apple's continuous-corner rounded rect: the macOS icon body on the 1024 grid."""
    corner = [("L", [(1.52866483, 0)]),
              ("C", [(1.08849323, 0), (0.86840689, 0.02196844), (0.66993427, 0.06549600)]),
              ("L", [(0.63149399, 0.07491176)]),
              ("C", [(0.37282392, 0.16905899), (0.16905899, 0.37282392), (0.07491176, 0.63149399)]),
              ("L", [(0.06549600, 0.66993427)]),
              ("C", [(0.02196844, 0.86840689), (0, 1.08849323), (0, 1.52866483)])]
    x1, y1 = x0 + w, y0 + w
    frames = [((x1, y0), (1, 0), (0, 1)), ((x1, y1), (0, 1), (-1, 0)),
              ((x0, y1), (-1, 0), (0, -1)), ((x0, y0), (0, -1), (1, 0))]
    d = []
    for (cx, cy), (ex, ey), (nx, ny) in frames:
        for op, pts in corner:
            xy = [f"{cx - ex*a*r + nx*b*r:.2f},{cy - ey*a*r + ny*b*r:.2f}" for a, b in pts]
            d.append(op + " ".join(xy))
    d[0] = "M" + d[0][1:]
    return " ".join(d) + " Z"


SQ = squircle()


SKY_TOP, SKY_BOTTOM = "#18203A", "#2C3553"
STAR = "#F3EEE7"


def icon_svg(small, with_stars=None):
    """The turtle's head rising over its shell like a moon, on a dark sky.

    The small art (16 and 32 px) drops the stars and the horizon line, has a
    bigger head and eye so the eye survives at 16 px, and a stronger rim so the
    icon keeps its edge against dark menus and Docks. Stars show from 128 px up.
    """
    if with_stars is None:
        with_stars = not small
    hx, hy, hr = (600, 560, 190) if small else (600, 560, 150)
    ex, ey, erx, ery = (hx + 58, hy - 44, 44, 52) if small else (hx + 50, hy - 40, 24, 28)
    stars = "" if not with_stars else "".join(
        f'<circle cx="{x}" cy="{y}" r="{r}" fill="{STAR}" opacity="{o}"/>'
        for x, y, r, o in ((250, 300, 7, .8), (380, 210, 5, .55), (800, 330, 6, .7), (300, 470, 4, .45)))
    horizon = "" if small else '<circle cx="512" cy="1250" r="640" fill="none" stroke="#E08A5E" stroke-opacity=".5" stroke-width="8"/>'
    rim = f'<path d="{SQ}" fill="none" stroke="#fff" stroke-opacity="{.28 if small else .14}" stroke-width="{56 if small else 14}"/>'
    highlight = "" if small else f'<path d="{SQ}" fill="none" stroke="url(#hl)" stroke-width="4"/>'
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
<defs>
<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{SKY_TOP}"/><stop offset="1" stop-color="{SKY_BOTTOM}"/></linearGradient>
<radialGradient id="shell" cx="0.38" cy="0.2" r="0.9"><stop offset="0" stop-color="#E65A1E"/><stop offset="1" stop-color="#A83400"/></radialGradient>
<radialGradient id="head" cx="0.4" cy="0.3" r="0.8"><stop offset="0" stop-color="#F0A47C"/><stop offset="1" stop-color="#D07448"/></radialGradient>
<linearGradient id="hl" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".45"/><stop offset=".35" stop-color="#fff" stop-opacity=".06"/><stop offset="1" stop-color="#fff" stop-opacity=".04"/></linearGradient>
<linearGradient id="sheen" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".10"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient>
<filter id="shadow" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="12" stdDeviation="14" flood-color="#000" flood-opacity=".32"/></filter>
<clipPath id="c"><path d="{SQ}"/></clipPath>
</defs>
<path d="{SQ}" fill="#000" filter="url(#shadow)"/>
<g clip-path="url(#c)">
<rect width="1024" height="1024" fill="url(#sky)"/>{stars}
<circle cx="{hx}" cy="{hy}" r="{hr}" fill="url(#head)"/>
<ellipse cx="{ex}" cy="{ey}" rx="{erx}" ry="{ery}" fill="#151517"/>
<circle cx="512" cy="1250" r="640" fill="url(#shell)"/>{horizon}
{rim}
<rect width="1024" height="1024" fill="url(#sheen)"/>
</g>
{highlight}
</svg>
"""


def badge(size):
    """The sky badge: the icon's scene in a round patch of sky with a ~1 px rim; stars from 64 px up."""
    s = size / 100
    rim = max(1.2, 100 / size) * 2
    stars = "" if size < 64 else (
        f'<circle cx="26" cy="30" r="1.1" fill="{STAR}" opacity=".8"/><circle cx="40" cy="19" r=".8" fill="{STAR}" opacity=".55"/>'
        f'<circle cx="80" cy="36" r="1" fill="{STAR}" opacity=".7"/>')
    return f"""<g transform="scale({s:.4f})">
<defs>
<linearGradient id="bsky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{SKY_TOP}"/><stop offset="1" stop-color="{SKY_BOTTOM}"/></linearGradient>
<radialGradient id="bshell" cx="0.4" cy="0.05" r="0.6"><stop offset="0" stop-color="#E65A1E"/><stop offset="1" stop-color="#A83400"/></radialGradient>
<radialGradient id="bhead" cx="0.4" cy="0.3" r="0.8"><stop offset="0" stop-color="#F0A47C"/><stop offset="1" stop-color="#D07448"/></radialGradient>
<clipPath id="bclip"><circle cx="50" cy="50" r="50"/></clipPath>
</defs>
<g clip-path="url(#bclip)">
<rect width="100" height="100" fill="url(#bsky)"/>{stars}
<circle cx="62" cy="47" r="15" fill="url(#bhead)"/>
<ellipse cx="68" cy="42" rx="2.6" ry="3" fill="#151517"/>
<circle cx="50" cy="136" r="78" fill="url(#bshell)"/>
<circle cx="50" cy="50" r="50" fill="none" stroke="#fff" stroke-opacity=".2" stroke-width="{rim:.2f}"/>
</g>
</g>"""


def wordmark_paths(size, tracking=-0.02):
    """'Endeavor' in Endeavor Sans SemiBold as outlines: (svg path d, advance width, cap height), in px."""
    import uharfbuzz as hb
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    from fontTools.ttLib import TTFont

    data = open(FONT, "rb").read()
    font = TTFont(FONT)
    upem = font["head"].unitsPerEm
    face = hb.Face(data)
    hbfont = hb.Font(face)
    buf = hb.Buffer()
    buf.add_str("Endeavor")
    buf.guess_segment_properties()
    hb.shape(hbfont, buf, {"kern": True, "liga": True})
    glyphs = font.getGlyphSet()
    order = font.getGlyphOrder()
    scale = size / upem
    pen = SVGPathPen(glyphs)
    x = 0
    for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
        name = order[info.codepoint]
        # Flip y: font units go up, SVG goes down; the baseline is y = 0.
        glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, (x + pos.x_offset) * scale, -pos.y_offset * scale)))
        x += pos.x_advance + tracking * upem
    x -= tracking * upem
    cap = font["OS/2"].sCapHeight * scale
    return pen.getCommands(), x * scale, cap


def logo_svg(text_color):
    """The name at 36 px; the badge 1.28 times that, 0.28 times that away, centred on each other."""
    size = 36
    mark, gap = round(size * 1.28), round(size * 0.28)
    d, width, cap = wordmark_paths(size)
    h = mark
    baseline = h / 2 + cap / 2
    w = mark + gap + width
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0f}" height="{h}" viewBox="0 0 {w:.2f} {h}" role="img" aria-label="Endeavor">
{badge(mark)}
<path transform="translate({mark + gap} {baseline:.2f})" fill="{text_color}" d="{d}"/>
</svg>
"""


def chrome():
    for c in [os.environ.get("CHROME", ""),
              "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
              "/Applications/Chromium.app/Contents/MacOS/Chromium",
              *sorted(glob.glob(os.path.expanduser("~/Library/Caches/ms-playwright/chromium_headless_shell-*/*/chrome-headless-shell")))]:
        if c and os.path.exists(c):
            return c
    sys.exit("Chrome not found; set CHROME to a Chrome or Chromium binary.")


def render(svg_path, png_path, size=1024):
    html = png_path + ".html"
    with open(html, "w") as f:
        f.write(f'<html><body style="margin:0;background:transparent"><img src="file://{svg_path}" width="{size}" height="{size}" style="display:block"></body></html>')
    subprocess.run([chrome(), "--headless", "--disable-gpu", "--hide-scrollbars", "--allow-file-access-from-files",
                    "--force-device-scale-factor=1", "--default-background-color=00000000",
                    f"--window-size={size},{size}", f"--screenshot={png_path}", f"file://{html}"],
                   check=True, capture_output=True)
    os.remove(html)


def resize(src, dst, px):
    shutil.copy(src, dst)
    subprocess.run(["sips", "-z", str(px), str(px), dst], check=True, capture_output=True)


def write_ico(little, mid, big, path):
    """The Windows icon: the small art at 16 to 32 px, as in the .icns."""
    from PIL import Image
    sizes = (16, 24, 32, 48, 64, 128, 256)
    images = [Image.open(little if px <= 32 else mid if px < 128 else big).convert("RGBA").resize((px, px), Image.LANCZOS) for px in sizes]
    images[-1].save(path, format="ICO", sizes=[(px, px) for px in sizes], append_images=images[:-1])


def main():
    for small in (False, True):
        with open(os.path.join(HERE, "icon-small.svg" if small else "icon.svg"), "w") as f:
            f.write(icon_svg(small))
    with open(os.path.join(HERE, "logo-dark.svg"), "w") as f:
        f.write(logo_svg("#ECECEC"))
    with open(os.path.join(HERE, "logo-light.svg"), "w") as f:
        f.write(logo_svg("#1A1A1C"))

    with tempfile.TemporaryDirectory() as tmp:
        big, little = os.path.join(tmp, "big.png"), os.path.join(tmp, "small.png")
        render(os.path.join(HERE, "icon.svg"), big)
        render(os.path.join(HERE, "icon-small.svg"), little)
        mid, mid_svg = os.path.join(tmp, "mid.png"), os.path.join(tmp, "mid.svg")
        with open(mid_svg, "w") as f:
            f.write(icon_svg(False, with_stars=False))
        render(mid_svg, mid)
        write_ico(little, mid, big, os.path.join(HERE, "endeavor.ico"))
        if os.environ.get("NO_MAC"):
            return
        iconset = os.path.join(tmp, "Endeavor.iconset")
        os.mkdir(iconset)
        for pt in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                px = pt * scale
                name = f"icon_{pt}x{pt}{'@2x' if scale == 2 else ''}.png"
                resize(little if px <= 32 else mid if px < 128 else big, os.path.join(iconset, name), px)
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(HERE, "Endeavor.icns")], check=True)
        for px in (256, 512):
            resize(big, os.path.join(HERE, f"endeavor-{px}.png"), px)
        if os.environ.get("KEEP_ICONSET"):
            shutil.copytree(iconset, os.environ["KEEP_ICONSET"], dirs_exist_ok=True)


if __name__ == "__main__":
    main()
