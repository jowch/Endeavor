import os

app = defines["app"]  # noqa: F821

format = "UDZO"
filesystem = "HFS+"
files = [app]
symlinks = {"Applications": "/Applications"}
icon = "assets/icon/Endeavor.icns"
background = "assets/dmg/background.png"

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
