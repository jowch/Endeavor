#!/usr/bin/env python3
"""Build Endeavor Sans and fetch JuliaMono into this directory.

Endeavor Sans = Schibsted Grotesk (Latin, figures, kerning, metrics)
              + Greek from Inter (opsz 14 instance), with ν from Arimo
              + mark-to-base anchors so combining accents sit over any letter
              + ⁺ ⁻ ⁿ generated to match Schibsted's own ¹²³
              + ∇ ∈ ∉ ∝ from Noto Sans Math, emboldened to Schibsted's stroke weight; ℏ = Schibsted's ħ

Usage: python3 fonts/build.py [--gap 0.03]
"""
import argparse
import copy
import io
import json
import re
import shutil
import tarfile
import urllib.parse
import urllib.request
import warnings
from pathlib import Path

import pathops
from fontTools.otlLib import builder as otl
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.pointInsidePen import PointInsidePen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables import otTables as ot
from fontTools.varLib import instancer

warnings.filterwarnings("ignore", category=DeprecationWarning)

HERE = Path(__file__).resolve().parent
CACHE = HERE / ".cache"
WEIGHTS = {"Regular": 400, "Medium": 500, "SemiBold": 600}

BASE = "Schibsted Grotesk"
GREEK = "Inter"
GREEK_AXES = {"opsz": 14}
NU_FAMILY = "Arimo"
MATH = "Noto Sans Math"
JULIAMONO_TAG = "v0.63.2"

GREEK_RANGES = [(0x0370, 0x03FF), (0x1F00, 0x1FFF)]
KEEP_FROM_BASE = {0x00B5}
NU = 0x03BD
SUPERIOR_DIGITS = [0x2070, 0x00B9, 0x00B2, 0x00B3] + list(range(0x2074, 0x207A))
TOP_FRACTION = 0.20
ANCHOR_CLAMP = 0.15  # max shift of a top/bottom-part centre from the bbox centre, as a fraction of advance
FLOATING_ANCHOR = 0.20  # designed top anchors this far (em) above the glyph's yMax are replaced


# ---------- sources ----------

def slug(name):
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")


def fetch_family(family):
    """Google Fonts download manifest for a family, cached in .cache/<slug>/."""
    d = CACHE / slug(family)
    d.mkdir(parents=True, exist_ok=True)
    mpath = d / "manifest.json"
    if not mpath.exists():
        url = "https://fonts.google.com/download/list?family=" + urllib.parse.quote_plus(family)
        raw = urllib.request.urlopen(url).read().decode("utf-8")
        mpath.write_text(raw[raw.index("{"):])
    manifest = json.loads(mpath.read_text())["manifest"]
    for f in manifest["files"]:
        (d / Path(f["filename"]).name).write_text(f["contents"])
    return d, {r["filename"]: r["url"] for r in manifest["fileRefs"]}


def fetch_file(family, filename):
    d, refs = fetch_family(family)
    path = d / Path(filename).name
    if not path.exists():
        urllib.request.urlretrieve(refs[filename], path)
    return path


def static_font(family, weight):
    return TTFont(fetch_file(family, f"static/{family.replace(' ', '')}-{weight}.ttf"))


def variable_font(family):
    _, refs = fetch_family(family)
    fn = next(f for f in refs if "/" not in f and f.endswith(".ttf") and "Italic" not in f)
    return fetch_file(family, fn)


def instance(family, **axes):
    vf = TTFont(variable_font(family))
    for a in vf["fvar"].axes:
        axes.setdefault(a.axisTag, a.defaultValue)
    return instancer.instantiateVariableFont(vf, axes)


def fetch_juliamono():
    d = CACHE / "juliamono"
    d.mkdir(parents=True, exist_ok=True)
    tgz = d / f"JuliaMono-ttf-{JULIAMONO_TAG}.tar.gz"
    if not tgz.exists():
        url = f"https://github.com/cormullion/juliamono/releases/download/{JULIAMONO_TAG}/JuliaMono-ttf.tar.gz"
        urllib.request.urlretrieve(url, tgz)
    with tarfile.open(tgz) as tar:
        for name in ("JuliaMono-Regular.ttf", "JuliaMono-Bold.ttf"):
            (HERE / name).write_bytes(tar.extractfile(name).read())
        (HERE / "JuliaMono-LICENSE.txt").write_bytes(tar.extractfile("LICENSE").read())
    return TTFont(HERE / "JuliaMono-Regular.ttf")["name"].getDebugName(5).split(";")[0]


# ---------- measuring ----------

def cp_glyph(font, cp):
    return font.getBestCmap()[cp if isinstance(cp, int) else ord(cp)]


def glyph_bounds(font, g):
    gs = font.getGlyphSet()
    pen = BoundsPen(gs)
    gs[g].draw(pen)
    return pen.bounds


def ink(font, g, y=None, x=None, step=2):
    """Total ink length along a horizontal line at y, or a vertical line at x."""
    gs = font.getGlyphSet()
    b = glyph_bounds(font, g)
    total = 0
    lo, hi = (b[0], b[2]) if y is not None else (b[1], b[3])
    t = lo
    while t <= hi:
        pen = PointInsidePen(gs, (t, y) if y is not None else (x, t))
        gs[g].draw(pen)
        total += pen.getResult()
        t += step
    return total * step


def stem_at_mid(font, g, parts=1):
    b = glyph_bounds(font, g)
    return ink(font, g, y=(b[1] + b[3]) / 2) / parts


def plus_bar(font):
    """Thickness of the horizontal arm of '+', measured left of the vertical stem."""
    g = cp_glyph(font, "+")
    b = glyph_bounds(font, g)
    return ink(font, g, x=b[0] + (b[2] - b[0]) * 0.15)


# ---------- glyph building ----------

def record(font, g):
    rec = DecomposingRecordingPen(font.getGlyphSet())
    font.getGlyphSet()[g].draw(rec)
    return rec


def to_glyph(rec, transform=(1, 0, 0, 1, 0, 0), embolden=0.0):
    """Outline → TrueType glyph, optionally emboldened by `embolden` units of stroke thickness."""
    if not embolden:
        pen = TTGlyphPen(None)
        rec.replay(TransformPen(pen, transform))
        glyph = pen.glyph()
    else:
        path = pathops.Path()
        rec.replay(TransformPen(path.getPen(), transform))
        path.simplify()
        stroke = pathops.Path()
        path.draw(stroke.getPen())
        stroke.stroke(embolden, pathops.LineCap.BUTT_CAP, pathops.LineJoin.MITER_JOIN, 4)
        merged = pathops.op(path, stroke, pathops.PathOp.UNION)
        pen = TTGlyphPen(None)
        merged.draw(Cu2QuPen(pen, max_err=1.0, reverse_direction=True))
        glyph = pen.glyph()
    if glyph.numberOfContours:
        glyph.coordinates.toInt()
    return glyph


def add_glyph(font, name, glyph, advance, lsb=None):
    glyf = font["glyf"]
    glyf.glyphs[name] = glyph
    glyph.recalcBounds(glyf)
    if lsb is None:
        lsb = glyph.xMin if glyph.numberOfContours else 0
    font["hmtx"].metrics[name] = (int(round(advance)), int(lsb))
    order = font.getGlyphOrder()
    if name not in order:
        order.append(name)
        font.setGlyphOrder(order)
        glyf.glyphOrder = order


def set_cmap(font, cp, g):
    for t in font["cmap"].tables:
        if t.isUnicode():
            t.cmap[cp] = g


def copy_glyph(src, g, dst, name, scale, dx=0, dy=0, embolden=0.0):
    glyph = to_glyph(record(src, g), (scale, 0, 0, scale, dx + embolden / 2, dy), embolden)
    add_glyph(dst, name, glyph, src["hmtx"][g][0] * scale + embolden)


def respace(font, g, lsb, rsb):
    glyph = font["glyf"][g]
    glyph.recalcBounds(font["glyf"])
    shift = lsb - glyph.xMin
    glyph.coordinates.translate((shift, 0))
    glyph.recalcBounds(font["glyf"])
    font["hmtx"].metrics[g] = (int(round(glyph.xMax + rsb)), glyph.xMin)


def rect(pen, x0, y0, x1, y1):
    pen.moveTo((round(x0), round(y0)))
    pen.lineTo((round(x0), round(y1)))
    pen.lineTo((round(x1), round(y1)))
    pen.lineTo((round(x1), round(y0)))
    pen.closePath()


# ---------- recipe steps ----------

def x_height(font):
    return glyph_bounds(font, cp_glyph(font, "x"))[3]


def import_greek(base, weight, log):
    greek = instance(GREEK, wght=WEIGHTS[weight], **GREEK_AXES)
    scale = x_height(base) / x_height(greek)
    cps = [c for lo, hi in GREEK_RANGES for c in range(lo, hi + 1) if c not in KEEP_FROM_BASE]
    for t in base["cmap"].tables:
        for c in cps:
            t.cmap.pop(c, None)
    gcmap = greek.getBestCmap()
    n = 0
    for c in cps:
        if c in gcmap:
            name = "grk." + gcmap[c]
            if name not in base["glyf"].glyphs:
                copy_glyph(greek, gcmap[c], base, name, scale)
            set_cmap(base, c, name)
            n += 1
    log.append(f"  Greek: {n} codepoints from {GREEK} (opsz 14, wght {WEIGHTS[weight]}), scale {scale:.4f}")
    return greek, scale


def import_nu(base, weight, greek, greek_scale, log):
    """ν from Arimo (Inter's reads as Latin v). Arimo is instanced at the weight whose ι stem, after
    x-height scaling, matches Inter's ι stem; ν is spaced with Schibsted's v sidebearings."""
    target = stem_at_mid(greek, cp_glyph(greek, 0x03B9)) * greek_scale

    def attempt(wght):
        f = instance(NU_FAMILY, wght=wght)
        s = x_height(base) / x_height(f)
        return f, s, stem_at_mid(f, cp_glyph(f, 0x03B9)) * s

    lo, hi = 400.0, 700.0
    for _ in range(9):
        mid = (lo + hi) / 2
        if attempt(mid)[2] < target:
            lo = mid
        else:
            hi = mid
    wght = round((lo + hi) / 2, 1)
    arimo, s, stem = attempt(wght)
    g = cp_glyph(arimo, NU)
    copy_glyph(arimo, g, base, "arimo.nu", s)
    v = cp_glyph(base, "v")
    vb = glyph_bounds(base, v)
    respace(base, "arimo.nu", vb[0], base["hmtx"][v][0] - vb[2])
    set_cmap(base, NU, "arimo.nu")
    log.append(f"  ν: Arimo wght {wght}, scale {s:.4f}, ι stem {stem:.0f}u vs Inter ι {target:.0f}u; "
               f"advance {base['hmtx']['arimo.nu'][0]} (v {base['hmtx'][v][0]}, Inter ν "
               f"{round(greek['hmtx'][cp_glyph(greek, NU)][0] * greek_scale)})")
    return arimo


def superior_scale(font):
    sup = glyph_bounds(font, cp_glyph(font, 0x00B9))
    lin = glyph_bounds(font, cp_glyph(font, "1"))
    return (sup[3] - sup[1]) / (lin[3] - lin[1]), sup[1]


def superior_signs(base):
    """⁺ ⁻ as square-ended bars: + and − proportions at the ¹²³ size, bar/stem ratio of the lining −/1."""
    s, _ = superior_scale(base)
    one = cp_glyph(base, 0x00B9)
    ob = glyph_bounds(base, one)
    cy = (ob[1] + ob[3]) / 2
    thick = stem_at_mid(base, one) * plus_bar(base) / stem_at_mid(base, cp_glyph(base, "1"))
    pb = glyph_bounds(base, cp_glyph(base, "+"))
    arm = (pb[2] - pb[0]) * s
    adv = base["hmtx"][cp_glyph(base, "+")][0] * s
    cx = adv / 2
    t = round(thick)
    for cp, name in ((0x207A, "plussuperior"), (0x207B, "minussuperior")):
        pen = TTGlyphPen(None)
        rect(pen, cx - arm / 2, cy - t / 2, cx + arm / 2, cy + t / 2)
        if cp == 0x207A:
            rect(pen, cx - t / 2, cy - arm / 2, cx + t / 2, cy - t / 2)
            rect(pen, cx - t / 2, cy + t / 2, cx + t / 2, cy + arm / 2)
        add_glyph(base, name, pen.glyph(), adv)
        set_cmap(base, cp, name)


def superior_n(base, log):
    """ⁿ = Schibsted's n at the ¹²³ scale, from the variable font instanced at the weight whose
    scaled n stem equals the ¹ stem."""
    s, baseline = superior_scale(base)
    target = stem_at_mid(base, cp_glyph(base, 0x00B9))

    def stem(w):
        f = instance(BASE, wght=w)
        return f, stem_at_mid(f, cp_glyph(f, "n"), parts=2) * s

    lo, hi = 400.0, 900.0
    for _ in range(9):
        mid = (lo + hi) / 2
        if stem(mid)[1] < target:
            lo = mid
        else:
            hi = mid
    w = round((lo + hi) / 2, 1)
    src, got = stem(w)
    copy_glyph(src, cp_glyph(src, "n"), base, "nsuperior", s, 0, baseline)
    set_cmap(base, 0x207F, "nsuperior")
    log.append(f"  ⁿ: Schibsted n at wght {w}, stem {got:.0f}u vs ¹ stem {target:.0f}u")


def import_math(base, log):
    """∈ ∉ sized to '+' height, ∝ to '+' width, ∇ to cap height; all centred like the base glyphs and
    emboldened so their strokes match Schibsted's '+' bar. ℏ maps to Schibsted's own ħ."""
    math = TTFont(fetch_file(MATH, "NotoSansMath-Regular.ttf"))
    mplus = glyph_bounds(math, cp_glyph(math, "+"))
    bplus = glyph_bounds(base, cp_glyph(base, "+"))
    m_bar, b_bar = plus_bar(math), plus_bar(base)
    s_op = (bplus[3] - bplus[1]) / (mplus[3] - mplus[1])
    s_prop = (bplus[2] - bplus[0]) / (lambda b: b[2] - b[0])(glyph_bounds(math, cp_glyph(math, 0x221D)))
    s_cap = glyph_bounds(base, cp_glyph(base, "H"))[3] / glyph_bounds(math, cp_glyph(math, "H"))[3]
    axis_base = (bplus[1] + bplus[3]) / 2
    axis_math = (mplus[1] + mplus[3]) / 2
    for cp, s, centred in ((0x2208, s_op, True), (0x2209, s_op, True), (0x221D, s_prop, True),
                           (0x2207, s_cap, False)):
        g = cp_glyph(math, cp)
        dy = axis_base - s * axis_math if centred else 0
        if cp == 0x221D:  # centre ∝ on its own bbox, not Noto's axis
            b = glyph_bounds(math, g)
            dy = axis_base - s * (b[1] + b[3]) / 2
        grow = max(0.0, b_bar - m_bar * s)
        copy_glyph(math, g, base, "math." + g, s, 0, round(dy), embolden=grow)
        set_cmap(base, cp, "math." + g)
    set_cmap(base, 0x210F, cp_glyph(base, 0x0127))
    log.append(f"  math: '+' bar {b_bar}u; Noto bar×scale ∈ {m_bar * s_op:.0f}u, ∝ {m_bar * s_prop:.0f}u, "
               f"∇ {m_bar * s_cap:.0f}u, emboldened to match; ℏ → {cp_glyph(base, 0x0127)}")
    return math


# ---------- GPOS / GDEF ----------

def gdef_classes(font):
    if "GDEF" not in font:
        gdef = newTable("GDEF")
        gdef.table = ot.GDEF()
        gdef.table.Version = 0x00010000
        font["GDEF"] = gdef
    t = font["GDEF"].table
    if t.GlyphClassDef is None:
        t.GlyphClassDef = ot.GlyphClassDef()
        t.GlyphClassDef.classDefs = {}
    return t.GlyphClassDef.classDefs


def markbase_subtables(gpos):
    for lookup in gpos.LookupList.Lookup:
        for st in lookup.SubTable:
            st = st.ExtSubTable if lookup.LookupType == 9 else st
            if st.LookupType == 4:
                yield st


def glyph_points(font, g):
    return [a for _, args in record(font, g).value for a in args if isinstance(a, tuple)]


def part_centre(pts, bounds, top, advance):
    xmin, ymin, xmax, ymax = bounds
    h = ymax - ymin
    sel = [x for x, y in pts if (y >= ymax - TOP_FRACTION * h if top else y <= ymin + TOP_FRACTION * h)]
    cx = (xmin + xmax) / 2
    if not sel:
        return cx
    lim = ANCHOR_CLAMP * advance
    return max(cx - lim, min(cx + lim, (min(sel) + max(sel)) / 2))


def add_mark_to_base(font, gap, log):
    """Append a MarkToBase lookup to the existing GPOS 'mark' feature. Marks reuse Schibsted's designed
    mark anchors (so spacing matches its own accented letters); bases get anchors over the top/bottom
    of their outline. Designed base anchors are kept unless they float far above the glyph."""
    upm = font["head"].unitsPerEm
    cmap = font.getBestCmap()
    classes = gdef_classes(font)
    gpos = font["GPOS"].table
    designed = {}
    for st in markbase_subtables(gpos):
        for g, rec in zip(st.MarkCoverage.glyphs, st.MarkArray.MarkRecord):
            designed.setdefault(g, (rec.MarkAnchor.XCoordinate, rec.MarkAnchor.YCoordinate))

    mark_info = {}
    for m in sorted({cmap[c] for c in cmap if 0x0300 <= c <= 0x036F}):
        b = glyph_bounds(font, m)
        if b is None:
            continue
        cx = round((b[0] + b[2]) / 2)
        bottom = b[3] <= 0 or b[1] < 0
        if m in designed:
            anchor = designed[m]
        else:
            anchor = (cx, round(b[3] + gap * upm) if bottom else round(b[1] - gap * upm))
        mark_info[m] = (1 if bottom else 0, otl.buildAnchor(*anchor))
        classes[m] = 3
        font["hmtx"].metrics[m] = (0, font["hmtx"][m][1])

    base_anchors = {}
    for g in font.getGlyphOrder():
        if classes.get(g, 0) in (2, 3) or g in mark_info:
            continue
        b = glyph_bounds(font, g)
        if b is None:
            continue
        adv = font["hmtx"][g][0]
        pts = glyph_points(font, g)
        base_anchors[g] = {0: otl.buildAnchor(round(part_centre(pts, b, True, adv)), b[3]),
                           1: otl.buildAnchor(round(part_centre(pts, b, False, adv)), b[1])}
        classes[g] = 1

    top_marks = {m for m, (c, _) in mark_info.items() if c == 0}
    floating = set()
    existing = []
    for st in markbase_subtables(gpos):
        existing.append((set(st.MarkCoverage.glyphs), set(st.BaseCoverage.glyphs)))
        top_classes = {r.Class for g, r in zip(st.MarkCoverage.glyphs, st.MarkArray.MarkRecord) if g in top_marks}
        for g, rec in zip(st.BaseCoverage.glyphs, st.BaseArray.BaseRecord):
            b = glyph_bounds(font, g)
            for c in top_classes:
                a = rec.BaseAnchor[c]
                if b and a is not None and a.YCoordinate > b[3] + FLOATING_ANCHOR * upm:
                    floating.add(g)

    groups = {}
    for m in mark_info:
        covered = {g for ms, bs in existing if m in ms for g in bs}
        if m in top_marks:
            covered -= floating
        groups.setdefault(frozenset(covered), []).append(m)
    glyph_map = font.getReverseGlyphMap(rebuild=True)
    subtables = []
    for covered, ms in groups.items():
        bases = {g: a for g, a in base_anchors.items() if g not in covered}
        subtables.extend(otl.buildMarkBasePos({m: mark_info[m] for m in ms}, bases, glyph_map))
    gpos.LookupList.Lookup.append(otl.buildLookup(subtables))
    gpos.LookupList.LookupCount = len(gpos.LookupList.Lookup)
    new_index = gpos.LookupList.LookupCount - 1

    mark_features = [i for i, fr in enumerate(gpos.FeatureList.FeatureRecord) if fr.FeatureTag == "mark"]
    if not mark_features:
        fr = ot.FeatureRecord()
        fr.FeatureTag = "mark"
        fr.Feature = ot.Feature()
        fr.Feature.FeatureParams = None
        fr.Feature.LookupListIndex = []
        gpos.FeatureList.FeatureRecord.append(fr)
        gpos.FeatureList.FeatureCount = len(gpos.FeatureList.FeatureRecord)
        mark_features = [gpos.FeatureList.FeatureCount - 1]
    for i in mark_features:
        f = gpos.FeatureList.FeatureRecord[i].Feature
        f.LookupListIndex.append(new_index)
        f.LookupCount = len(f.LookupListIndex)

    add_grek_script(gpos)
    for sr in gpos.ScriptList.ScriptRecord:
        for ls in [sr.Script.DefaultLangSys] + [l.LangSys for l in sr.Script.LangSysRecord]:
            if ls is not None and not any(i in ls.FeatureIndex for i in mark_features):
                ls.FeatureIndex.append(mark_features[0])
                ls.FeatureIndex.sort()
                ls.FeatureCount = len(ls.FeatureIndex)
    log.append(f"  anchors: {len(mark_info)} marks, {len(base_anchors)} bases; replaced floating designed "
               f"anchors on {' '.join(sorted(floating))}")


def add_grek_script(table):
    if any(sr.ScriptTag == "grek" for sr in table.ScriptList.ScriptRecord):
        return
    dflt = next((sr for sr in table.ScriptList.ScriptRecord if sr.ScriptTag == "DFLT"),
                table.ScriptList.ScriptRecord[0])
    sr = ot.ScriptRecord()
    sr.ScriptTag = "grek"
    sr.Script = ot.Script()
    sr.Script.DefaultLangSys = copy.deepcopy(dflt.Script.DefaultLangSys)
    sr.Script.LangSysRecord = []
    sr.Script.LangSysCount = 0
    table.ScriptList.ScriptRecord.append(sr)
    table.ScriptList.ScriptRecord.sort(key=lambda r: r.ScriptTag)
    table.ScriptList.ScriptCount = len(table.ScriptList.ScriptRecord)


# ---------- naming ----------

def copyright_of(font):
    return (font["name"].getDebugName(0) or "").strip()


def rename(font, weight, copyrights, designers):
    name = font["name"]
    for nid in (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 21, 22, 25):
        name.removeNames(nameID=nid)
    family, ps = "Endeavor Sans", f"EndeavorSans-{weight}"
    entries = {
        0: " ".join(copyrights),
        3: f"1.000;ENDV;{ps}",
        4: f"{family} {weight}",
        5: "Version 1.000",
        6: ps,
        9: "; ".join(designers),
        13: "This Font Software is licensed under the SIL Open Font License, Version 1.1. "
            "This license is available with a FAQ at: https://openfontlicense.org",
        14: "https://openfontlicense.org",
    }
    if weight == "Regular":
        entries[1], entries[2] = family, "Regular"
    else:
        entries[1], entries[2] = f"{family} {weight}", "Regular"
        entries[16], entries[17] = family, weight
    for nid, s in entries.items():
        name.setName(s, nid, 3, 1, 0x409)
    font["head"].fontRevision = 1.0
    os2 = font["OS/2"]
    os2.ulUnicodeRange1 |= 1 << 7  # Greek and Coptic
    os2.ulCodePageRange1 |= 1 << 3  # 1253 Greek


def build(args):
    log = []
    copyrights = []
    for weight in WEIGHTS:
        base = static_font(BASE, weight)
        log.append(f"{weight}:")
        greek, gscale = import_greek(base, weight, log)
        arimo = import_nu(base, weight, greek, gscale, log)
        cmap = base.getBestCmap()
        missing_sups = [chr(c) for c in SUPERIOR_DIGITS if c not in cmap]
        if missing_sups:
            raise SystemExit(f"Schibsted lacks superior digits {''.join(missing_sups)}")
        superior_signs(base)
        superior_n(base, log)
        math = import_math(base, log)
        add_mark_to_base(base, args.gap, log)
        sources = [base, greek, arimo, math]
        copyrights = [copyright_of(f) for f in sources]
        designers = [n for n in (f["name"].getDebugName(9) for f in sources) if n]
        rename(base, weight, copyrights, designers)
        base.save(HERE / f"EndeavorSans-{weight}.ttf")

    ofl = (CACHE / slug(BASE) / "OFL.txt").read_text()
    body = ofl[ofl.index("This Font Software"):]
    lines = [f"{fam}: {c}" for fam, c in zip((BASE, GREEK, NU_FAMILY, MATH), copyrights)]
    (HERE / "OFL.txt").write_text("Endeavor Sans is built from:\n" + "\n".join(lines) + "\n\n" + body)
    version = fetch_juliamono()
    log.append(f"JuliaMono {version}: JuliaMono-Regular.ttf, JuliaMono-Bold.ttf, JuliaMono-LICENSE.txt")
    print("\n".join(log))


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--gap", type=float, default=0.03,
                    help="mark-to-base gap (em) for combining marks Schibsted has no designed anchor for")
    build(ap.parse_args())
