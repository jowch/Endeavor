# Fonts

**Endeavor Sans** is the interface font. It is Schibsted Grotesk with the pieces a statistics notebook needs added:

- Greek from Inter (opsz 14 instance), scaled to Schibsted's x-height. ν comes from Arimo instead, because Inter's ν looks like a Latin v.
- Mark-to-base anchors, so combining accents (β̂, x̄, ε̃, x̣) sit over or under any letter.
- ⁺ ⁻ ⁿ generated to match Schibsted's own superscript digits.
- ∇ ∈ ∉ ∝ from Noto Sans Math, thickened to Schibsted's stroke weight per weight. ℏ uses Schibsted's ħ.

Schibsted's Latin letters, kerning and vertical metrics are unchanged. Weights: Regular, Medium, SemiBold.
**JuliaMono** (Regular, Bold, and RegularItalic for Pluto's italic comments) is the code font, shipped unmodified from its official release (v0.63.2).

Rebuild everything (downloads sources into `fonts/.cache/`, which git ignores):
    pip3 install -r fonts/requirements.txt
    python3 fonts/build.py
    python3 fonts/verify.py
`verify.py` shapes accented letters with HarfBuzz and checks accent placement, kerning, Greek coverage, math stroke weights and superscripts. It exits non-zero on any failure.

Licence: every source is under the SIL Open Font License 1.1. `OFL.txt` covers Endeavor Sans and lists each source's copyright. `JuliaMono-LICENSE.txt` covers JuliaMono, which has the Reserved Font Name "JuliaMono".
