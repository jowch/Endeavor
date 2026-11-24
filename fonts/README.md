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

## How `build.py` builds it

The sources are Google Fonts downloads, cached in `fonts/.cache/<family>/`.
`--gap` (0.03 em by default) is the space between a letter and a combining mark Schibsted has no anchor of its own for.

- **ν** comes from Arimo, since Inter's reads as a Latin v. Arimo is instanced at the weight whose ι stem, once scaled to the x-height, matches Inter's ι stem, and ν takes Schibsted's v sidebearings.
- **⁺ ⁻** are square-ended bars with the proportions of + and − at the size of ¹²³, and the bar-to-stem ratio of the full-size − and 1.
- **ⁿ** is Schibsted's n at the ¹²³ scale, from the variable font instanced at the weight whose scaled n stem equals the stem of ¹.
- **∈ ∉** are sized to the height of +, **∝** to its width, and **∇** to the cap height. They are centred like the glyphs around them, except that ∝ is centred on its own outline rather than on Noto's math axis. Each is thickened so its strokes match Schibsted's + bar. ℏ maps to Schibsted's own ħ.
- **Accents** come from a mark-to-base lookup added to GPOS's `mark` feature. Marks keep Schibsted's own mark anchors, so they space as on its accented letters. Letters get anchors over the top and under the bottom of their outline. The centre of a letter's top or bottom part moves at most 0.15 of its advance from the centre of its box (`ANCHOR_CLAMP`). Schibsted's own top anchors stay, unless one floats more than 0.20 em above the letter (`FLOATING_ANCHOR`).
- The fonts declare the Greek and Coptic Unicode range and code page 1253, so systems offer them for Greek text.
