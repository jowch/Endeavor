# The icon, logo and disk image

Endeavor's artwork is drawn by scripts, so it can be changed and rebuilt
rather than redrawn by hand. Each script writes its files next to itself.

## Icon and logo (`assets/icon/build.py`)

`python3 assets/icon/build.py` writes:

- `icon.svg` and `icon-small.svg`: the icon on Apple's 1024 grid. The small
  art is for 16 and 32 px.
- `Endeavor.icns`: the macOS icon, which `scripts/bundle.sh` copies into the
  app.
- `endeavor-256.png` and `endeavor-512.png`, for Linux desktop files.
- `endeavor.ico`: the icon for the Windows installer and Start menu.
- `logo-dark.svg` and `logo-light.svg`: the sky badge with "Endeavor"
  beside it, its text turned into outlines.

It needs Chrome or Chromium (set `CHROME` to its binary if it isn't found),
macOS's `sips` and `iconutil`, fonttools and uharfbuzz
(`fonts/requirements.txt`), and Pillow for the .ico. With `NO_MAC` set, it
writes only the .ico.

What the artwork is:

- The icon's body is Apple's continuous-corner rounded rectangle, the shape
  of a macOS icon on the 1024 grid. Inside it, the turtle's head rises over
  its shell like a moon, on a dark sky.
- The small art (16 and 32 px) drops the stars and the horizon line. It has a
  bigger head and eye, so the eye survives at 16 px, and a stronger rim, so
  the icon keeps its edge against dark menus and Docks. Stars show from
  128 px up.
- The sky badge is the icon's scene in a round patch of sky, with a rim of
  about 1 px. Its stars show from 64 px up.
- The logo sets the name at 36 px in Endeavor Sans SemiBold. The badge is
  1.28 times that size and 0.28 times that away, and the two are centred on
  each other. The name is turned into outlines with HarfBuzz, so the logo
  needs no font.
- The Windows icon uses the small art at 16 to 32 px, as the .icns does.

## The Mac disk image (`assets/dmg/`)

`python3 assets/dmg/build.py` redraws the disk image's window background,
`background.png` (640 x 480) and `background@2x.png`, from the setup
window's scene. It needs Chrome or Chromium, as the icon script does.

- The top is the setup window's sky (`src/splash.rs`, `src/turtle.rs`): the
  turtle standing on the horizon and looking up at the stars, with the name
  and tagline. Its colours and the turtle's shape are copied from
  `src/theme.rs` and `src/turtle.rs`, and its gaze and stars from
  `src/splash.rs`.
- The bottom is light ground where the app and the Applications link sit.
  Finder draws their labels in black whatever the background, so they need a
  light place to stand.
- An arrow runs from the app (x 180) to Applications (x 460), between the
  icons.

`assets/dmg/settings.py` is dmgbuild's settings, used by the nightly
workflow ([ci.md](ci.md)). Run it from the repository's root:

```sh
dmgbuild -s assets/dmg/settings.py -D app=target/release/Endeavor.app Endeavor out.dmg
```

- The window shows the background, the app and a link to Applications.
  dmgbuild finds `background@2x.png` for Retina screens by its name.
- The icon positions match the background's arrow.
- The window's height counts its title bar, so the window shows a little
  less than the background (both are 640 x 480). The plain ground at the
  bottom is what gets cut, whatever the title bar's height, so the icons'
  labels stay in view.
