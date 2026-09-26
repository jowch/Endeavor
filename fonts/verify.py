#!/usr/bin/env python3
"""Check the built Endeavor Sans fonts. Usage: python3 fonts/verify.py"""
import sys
import tempfile
from pathlib import Path

import uharfbuzz as hb
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.pointInsidePen import PointInsidePen
from fontTools.pens.recordingPen import DecomposingRecordingPen, RecordingPen
from fontTools.ttLib import TTFont

HERE = Path(__file__).resolve().parent
SOURCE = HERE / ".cache" / "schibsted-grotesk"
WEIGHTS = ["Regular", "Medium", "SemiBold"]
ACCENTS = ["β̂", "σ̂", "θ̂", "λ̂", "μ̂", "x̂", "k̂", "d̂", "n̂",
           "ŷ", "Ĥ", "ā", "x̄", "ȳ", "ε̃", "ψ̇", "x̣"]
GREEK = [c for c in range(0x391, 0x3AA) if c != 0x3A2] + list(range(0x3B1, 0x3CA)) + \
        [0x3D1, 0x3D5, 0x3D6, 0x3F0, 0x3F1, 0x3F5]  # ϑ ϕ ϖ ϰ ϱ ϵ
PAGE_TEXT = ("Posterior means: μ = 2.31, σ = 0.18, λ = 0.042 h⁻¹, θ ∈ [0, π], ρ(x, y) = 0.87; 10⁻⁶ χ² ∂L/∂β ∇f "
             "∝ ∉ ℏ ± × ÷ − ≈ ≠ ≤ ≥ ∑ ∏ ∫ √ ∞ ° ‰ Å x⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻ⁿ → · • … – — “ ” ’")
failures = []


def check(ok, msg):
    if not ok:
        failures.append(msg)


def hbfont(path):
    return hb.Font(hb.Face(hb.Blob.from_file_path(str(path))))


def shape(font, text):
    buf = hb.Buffer()
    buf.add_str(text)
    buf.guess_segment_properties()
    hb.shape(font, buf)
    return buf.glyph_infos, buf.glyph_positions


def accents(path):
    f = hbfont(path)
    upm = f.face.upem
    rows = []
    for text in ACCENTS:
        infos, pos = shape(f, text)
        if len(infos) == 1:
            rows.append(f"{text:4} precomposed {f.get_glyph_name(infos[0].codepoint)}")
            continue
        be, me = f.get_glyph_extents(infos[0].codepoint), f.get_glyph_extents(infos[1].codepoint)
        adv = pos[0].x_advance
        off = (adv + pos[1].x_offset + me.x_bearing + me.width / 2 - (be.x_bearing + be.width / 2)) / adv
        b_top, b_bot = be.y_bearing, be.y_bearing + be.height
        m_top = pos[1].y_offset + me.y_bearing
        m_bot = m_top + me.height
        gap = (m_bot - b_top) if m_top + m_bot > b_top + b_bot else (b_bot - m_top)
        rows.append(f"{text:4} offset {off:+.3f}  gap {gap / upm:.3f} em")
        check(abs(off) < 0.2 and gap > 0, f"{path.name} {text}: offset {off:.3f}, gap {gap}")
    return rows


def kerning(path):
    f = hbfont(path)
    infos, pos = shape(f, "AV To Wa")
    return [(f.get_glyph_name(i.codepoint), p.x_advance - f.get_glyph_h_advance(i.codepoint))
            for i, p in zip(infos, pos) if p.x_advance != f.get_glyph_h_advance(i.codepoint)]


def bounds(font, g):
    pen = BoundsPen(font.getGlyphSet())
    font.getGlyphSet()[g].draw(pen)
    return pen.bounds


def runs(font, g, x=None, y=None, step=1):
    """Lengths of the ink runs along a vertical line at x or a horizontal line at y."""
    gs = font.getGlyphSet()
    b = bounds(font, g)
    lo, hi = (b[1], b[3]) if x is not None else (b[0], b[2])
    out, cur, t = [], 0, lo - 2
    while t <= hi + 2:
        pen = PointInsidePen(gs, (x, t) if x is not None else (t, y))
        gs[g].draw(pen)
        if pen.getResult():
            cur += step
        elif cur:
            out.append(cur)
            cur = 0
        t += step
    return out


def math_stems(font):
    c = font.getBestCmap()
    frac = lambda g, fx: bounds(font, g)[0] + (bounds(font, g)[2] - bounds(font, g)[0]) * fx
    mid_y = lambda g: (bounds(font, g)[1] + bounds(font, g)[3]) / 2
    plus = runs(font, c[ord("+")], x=frac(c[ord("+")], 0.15))[0]
    stems = {
        "+": plus,
        "∈": sorted(runs(font, c[0x2208], x=frac(c[0x2208], 0.75)))[1],
        "∉": sorted(runs(font, c[0x2209], x=frac(c[0x2209], 0.9)))[1],
        "∝": runs(font, c[0x221D], y=mid_y(c[0x221D]))[0],
        "∇": runs(font, c[0x2207], x=frac(c[0x2207], 0.5))[-1],
        "∞": runs(font, c[0x221E], y=mid_y(c[0x221E]))[0],
    }
    return {k: (v, v / plus) for k, v in stems.items()}


def outline(font, g):
    pen = RecordingPen()
    font.getGlyphSet()[g].draw(pen)
    return pen.value


def main():
    for weight in WEIGHTS:
        path = HERE / f"EndeavorSans-{weight}.ttf"
        src_path = SOURCE / f"SchibstedGrotesk-{weight}.ttf"
        font, src = TTFont(path), TTFont(src_path)
        cmap = font.getBestCmap()
        print(f"\n== {path.name}  ({font['name'].getDebugName(4)}, usWeightClass {font['OS/2'].usWeightClass})")
        print("\n".join(accents(path)))

        k_new, k_src = kerning(path), kerning(src_path)
        print("kerning 'AV To Wa':", k_new, "(source identical)" if k_new == k_src else f"SOURCE {k_src}")
        check(k_new and k_new == k_src, f"{path.name} kerning differs")

        missing = "".join(chr(c) for c in GREEK if c not in cmap)
        print("Greek + variants missing:", missing or "none")
        check(not missing, f"{path.name} missing Greek {missing}")
        nu = cmap.get(0x03BD)
        print(f"ν glyph: {nu}; ν advance {font['hmtx'][nu][0]} vs v {font['hmtx'][cmap[ord('v')]][0]}, "
              f"sidebearings ν {bounds(font, nu)[0]}/{font['hmtx'][nu][0] - bounds(font, nu)[2]} "
              f"v {bounds(font, cmap[ord('v')])[0]}/{font['hmtx'][cmap[ord('v')]][0] - bounds(font, cmap[ord('v')])[2]}")
        check(nu == "arimo.nu", f"{path.name} ν is {nu}")

        stems = math_stems(font)
        print("math stems (u, ratio to '+' bar):", ", ".join(f"{k} {v[0]} ({v[1]:.2f})" for k, v in stems.items()))
        for k in "∈∉∝∇":
            check(0.85 <= stems[k][1] <= 1.15, f"{path.name} {k} stem ratio {stems[k][1]:.2f}")
        print(f"ℏ → {cmap.get(0x210F)} (ħ is {cmap.get(0x0127)})")
        check(cmap.get(0x210F) == cmap.get(0x0127), f"{path.name} ℏ not mapped to ħ")

        one = cmap[0x00B9]
        one_stem = runs(font, one, y=(bounds(font, one)[1] + bounds(font, one)[3]) / 2)
        n = cmap[0x207F]
        n_stem = runs(font, n, y=(bounds(font, n)[1] + bounds(font, n)[3]) / 2)
        ratio = n_stem[0] / max(one_stem)
        print(f"ⁿ stem {n_stem[0]}u vs ¹ stem {max(one_stem)}u ({ratio:.3f}); ⁺ ⁻ present: "
              f"{0x207A in cmap and 0x207B in cmap}")
        check(abs(ratio - 1) <= 0.03, f"{path.name} ⁿ stem ratio {ratio:.3f}")

        for ch in "AagRx1&":
            check(outline(font, cmap[ord(ch)]) == outline(src, src.getBestCmap()[ord(ch)]),
                  f"{path.name} Latin outline changed: {ch}")
        fallback = sorted({ch for ch in PAGE_TEXT if not ch.isspace() and ord(ch) not in cmap})
        print("sample text glyphs missing:", "".join(fallback) or "none")
        check(not fallback, f"{path.name} missing {''.join(fallback)}")

        with tempfile.TemporaryDirectory() as tmp:
            font.save(Path(tmp) / "rt.ttf")
            rt = TTFont(Path(tmp) / "rt.ttf")
            gs = rt.getGlyphSet()
            for g in rt.getGlyphOrder():
                gs[g].draw(DecomposingRecordingPen(gs))
            print(f"round trip: {len(rt.getGlyphOrder())} glyphs drawn")

    print("\nFAILURES:\n" + "\n".join(failures) if failures else "\nALL CHECKS PASSED")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
