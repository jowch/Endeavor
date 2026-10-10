# dmgbuild settings for the Mac disk image (.github/workflows/nightly.yml).
# Run from the repository's root:
#   dmgbuild -s assets/dmg/settings.py -D app=target/release/Endeavor.app Endeavor out.dmg
# The window shows background.png (and background@2x.png on Retina screens,
# which dmgbuild picks up by name), the app and a link to Applications.
# build.py redraws the background; the icon positions below match its arrow.
import os

app = defines["app"]  # noqa: F821 (dmgbuild passes -D values in defines)

format = "UDZO"
filesystem = "HFS+"
files = [app]
symlinks = {"Applications": "/Applications"}
icon = "assets/icon/Endeavor.icns"
background = "assets/dmg/background.png"

# The window's height counts its title bar, so it is taller than the
# background (640 x 440) to show all of it, labels under the icons included.
window_rect = ((200, 120), (640, 480))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
icon_size = 96
text_size = 13
icon_locations = {os.path.basename(app): (180, 300), "Applications": (460, 300)}
