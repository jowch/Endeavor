# Endeavor on Linux

_Status as of 2026-09-29. Tested only on Ubuntu 26.04 aarch64 (the OrbStack
machine `endeavor-linux`), under Xvfb, with and without the Openbox window
manager. There is no x86_64 test yet, and no test on a full desktop such as
GNOME or KDE._

**Summary:** the app builds, passes its tests and runs on Linux under X11. The
window opens with the window manager's frame, setup installs Node and the
adapter, the local Julia runtime starts, and the Pluto notebook shows in the
web view and runs cells. The keyboard moves between the app and the notebook,
menus and Settings show over the notebook, light and dark follow the desktop,
and a session on a server works from a Linux client. The main gaps left are
packaging, the notebook page's own ⌘ shortcuts, and the macOS menu bar's items.

The remaining work is ranked at the end of this page.

## What works

- **Build and tests.** `cargo build` and `cargo test --workspace` pass on
  Linux (the `relay` test that used to fail was fixed in 09a4286).
- **Window and frame.** Linux windows use the window manager's frame and title
  bar. That is GPUI's default on X11; its client-side decorations need a
  compositor. Under Openbox the frame looks right, the window moves and
  resizes, and dragging a column header also moves the window. The headers
  leave no room for macOS's traffic lights.
- **UI.** GPUI renders the setup screen, the session list, the chat pane and
  the notebook pane, with correct fonts. Text boxes take typing.
- **Keyboard between the app and the notebook.** A click in the app gives GPUI's
  X window the keyboard, and a click in the notebook gives WebKit's window the
  keyboard. The app's shortcuts work while the notebook has the keyboard, in
  the same order as on macOS: Ctrl+Q, Ctrl+, Ctrl+B and zoom first, other Ctrl
  shortcuts only when the page doesn't use them (`src/linux/webkeys.rs`).
- **Menus, Settings and dialogs over the notebook.** While one is open, the
  notebook's web view gets a hole in its X Shape where the menu is, and clicks
  there reach GPUI (`src/linux/overlay.rs`). Without a compositor the notebook
  isn't dimmed behind Settings or a confirm dialog.
- **Shortcut hints and names.** Hints show `Ctrl+N`, `Ctrl+F` and so on. The
  local host is "This computer" (macOS: "This Mac"), and "Reveal in Finder" is
  "Show in Files". Both come from `src/platform.rs`.
- **First-run setup.** The app downloads and checks the pinned Linux Node.js,
  runs `npm ci` for the adapter, and precompiles Pluto and EndeavorRuntime.
- **Local runtime.** `endeavor-remote` starts Julia 1.12.6 and reports the
  Pluto and MCP ports. Quitting the app stops Julia.
- **Notebook.** Pluto loads in the WebKitGTK web view at the right place and
  size, and follows the pane when the sidebar opens or the window resizes. Run
  notebook runs cells. Typing, arrow keys and Shift+Enter work in cells.
- **Settings.** Every section opens and draws. Settings' search works. Show in
  Files opens the log folder with `xdg-open`.
- **Light and dark.** Dark and Light switch the whole window, notebook
  included. Match system follows the desktop's color scheme from
  xdg-desktop-portal (`org.freedesktop.appearance color-scheme`, GNOME's Dark
  Style) while the app runs. With no portal, or no preference, it's light.
  GTK's `gtk-application-prefer-dark-theme` isn't an input: the app sets it to
  make WebKitGTK's `prefers-color-scheme` match.
- **Reduce motion.** GTK's `gtk-enable-animations=false` stops the turtle's
  animation. As on macOS, the app reads it once at launch.
- **Servers.** From the Linux client, Add server, Test connection, a session on
  the server and running its notebook all work. Tested against the VM itself
  over `ssh localhost`. A Linux client sends a Linux server of its own
  architecture its own `target/*/endeavor-remote`.
- **Browse… folder picker.** GPUI asks xdg-desktop-portal, whose GTK backend
  shows its folder chooser. The chosen folder becomes the session's folder.
  The picker starts in Recent, not in the current folder.
- **Idle CPU.** GTK's events run from GPUI's loop without a timer: the main
  thread prepares, checks and dispatches GLib's main context, and a helper
  thread waits in `poll()` on its file descriptors (`src/linux/gtk_loop.rs`).
  Measured in the VM (9 cores, llvmpipe), as a share of one core:

  | Idle state | Before (8 ms poll) | After |
  |---|---|---|
  | Notebook open | 4.0%, 945 context switches/s | 1.4%, 330/s |
  | New-session screen, reduce motion on | 3.4%, 685/s | 1.0%, 114/s |

  About 60 of the remaining wakeups a second are GPUI's own X11 frame timer.
  With a notebook open, WebKit's display-refresh thread runs at about 90 Hz
  too. With reduce motion off, the new-session screen's turtle redraws every
  frame, which costs about 3.5 cores on llvmpipe, a software GPU; a real GPU
  makes it cheap.
- **Claude sign-in.** "Claude subscription" runs `claude auth login --claudeai`.
  That command opens the OAuth URL with `xdg-open`. Nobody finished a sign-in:
  that needs a real account in the browser.

## What changed for Linux

- `Cargo.toml` adds these Linux-only dependencies:
  - GPUI's `x11` feature.
  - `gtk` 0.18 and `gdkx11`, the versions wry already uses.
  - `x11` (Xlib). `src/linux/overlay.rs` also links libXext for XShape.
- `src/platform.rs` holds the code that differs by OS:
  - **Names:** `this_computer!()`, `shortcut!()`, `REVEAL`, `REVEAL_FOLDER`,
    `SHOW_LOGS` and `MATCH_SYSTEM`.
  - **Reveal:** macOS runs `open -R`. Linux runs `xdg-open` on the folder.
  - **Reduce motion:** macOS reads NSWorkspace. Linux reads GTK's
    `gtk-enable-animations`.
  - **Web view host:** wry's Linux web view is WebKitGTK in an X11 child window.
    GTK is started at launch and `src/linux/gtk_loop.rs` runs its events. GPUI
    gives an XCB window handle, but wry accepts only Xlib, so `webview_parent`
    passes the same X window ID as an Xlib handle.
- `src/linux/` has the Linux versions of macOS's `webkeys` (key routing) and
  `overlay` (menus over the notebook).
- Still stubs on Linux: `snapshot` (a picture of the notebook for Point's drawn
  box, which goes as its cells instead) and `dialogs` (the page's alert(),
  confirm() and prompt() fall back to WebKitGTK's own GTK dialogs, untested).
- **Files:** app data goes to `$XDG_DATA_HOME/endeavor` (default
  `~/.local/share/endeavor`). The log goes to `$XDG_STATE_HOME/endeavor/endeavor.log`
  (default `~/.local/state/endeavor/endeavor.log`).
- **Pinned downloads:** Linux Julia and Node tarballs are pinned for aarch64
  and x86_64. The Julia pins are the ones the helper already uses.
- **Agent text:** the agent's prompt, the MCP refusal, the host tools'
  descriptions and the pluto-session skill say "the user's computer", not
  "the user's Mac". A server's runtime serves some of that text and can't know
  the client's OS.

## System packages (Ubuntu 26.04)

This is the set installed on `endeavor-linux`. It hasn't been reduced to the
smallest set that works.

```
sudo apt-get install -y build-essential clang cmake pkg-config libssl-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
  libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libasound2-dev \
  libzstd-dev libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev libxdo-dev mesa-vulkan-drivers
# To run without a desktop, and for screenshots and input:
sudo apt-get install -y xvfb imagemagick xdotool
# A window manager, and CPU measurement:
sudo apt-get install -y openbox sysstat
```

At runtime the binary links these libraries: WebKitGTK 4.1, GTK 3, xcb,
xkbcommon(-x11), Xlib, libXext, fontconfig and libwayland-client.
`mesa-vulkan-drivers` provides llvmpipe, a software GPU driver, for a VM with
no GPU. The app also calls `curl`, `tar` and `shasum` (Ubuntu's `perl` package
provides `shasum`). Browse… and Match system need `xdg-desktop-portal` and a
backend such as `xdg-desktop-portal-gtk`, which desktops install anyway.

## Build and run in the VM

OrbStack machines have no display of their own. Run the app under Xvfb and take
screenshots.

1. Copy the worktree into the VM and build it there:

   ```
   rsync -a --delete --exclude target --exclude .git --exclude node_modules ./ endeavor-linux:endeavor/
   ssh endeavor-linux 'cd endeavor && ~/.cargo/bin/cargo build'
   ```

2. Optional: to skip the 300 MB Julia download, point the app at the helper's
   copy of Julia:

   ```
   ssh endeavor-linux 'mkdir -p ~/.local/share/endeavor && ln -sfn ~/.cache/endeavor/julia-1.12.6 ~/.local/share/endeavor/julia-1.12.6'
   ```

3. Start Xvfb, optionally Openbox, and the app:

   ```
   ssh endeavor-linux 'Xvfb :99 -screen 0 1600x1000x24 & sleep 1; DISPLAY=:99 openbox & cd endeavor && DISPLAY=:99 nohup target/debug/endeavor >/dev/null 2>&1 &'
   ```

4. Take a screenshot, or click and type:

   ```
   ssh endeavor-linux 'DISPLAY=:99 import -window root png:-' > shot.png
   ssh endeavor-linux 'DISPLAY=:99 xdotool mousemove 800 680 click 1'
   ```

5. Stop the app, Openbox and Xvfb:

   ```
   ssh endeavor-linux 'pkill -x endeavor; pkill -x openbox; pkill -x Xvfb'
   ```

In an OrbStack machine, `xdg-open` is OrbStack's own version, which opens URLs
in the Mac's browser. To see sign-in URLs without opening them, put a fake
`xdg-open` first on `PATH` that only logs its arguments.

Setup waits until Claude is signed in. To reach the workspace without an
account, set `ENDEAVOR_CLAUDE_CLI` to a script that answers `auth status` as
signed in (see [testing.md](testing.md#other-debug-switches)):

```sh
#!/bin/sh
if [ "$1 $2" = "auth status" ]; then echo '{"loggedIn":true,"authMethod":"claude.ai"}'; fi
```

Claude's turns then fail with "Sign in to Claude again", but the notebook,
Settings and servers all work. The state dump (`scripts/app-state.sh`) works
as on macOS.

xdg-desktop-portal's GTK backend is a GTK program started by systemd, which
has no `DISPLAY` in the VM. For Browse… and Match system, give it Xvfb's and
restart the portal once Xvfb runs:

```
ssh endeavor-linux 'systemctl --user set-environment DISPLAY=:99; systemctl --user restart xdg-desktop-portal-gtk xdg-desktop-portal'
```

Then `gsettings set org.gnome.desktop.interface color-scheme prefer-dark` (or
`prefer-light`, `default`) switches the desktop's scheme.

## Remaining work, ranked

Sizes: S is under a day, M is 1–3 days, L is more than that or needs upstream
changes.

1. **Linux packaging (M).**
   - `scripts/bundle.sh` builds only a macOS .app.
   - `install::resources()` looks for `../Resources` next to the executable.
     Linux needs its own layout, such as a tarball, AppImage or .deb with
     `share/endeavor/{runtime,plugin,adapter}`.
   - Linux also needs a `.desktop` file and a window icon (the window
     manager shows a generic one). The icon is ready:
     `assets/icon/endeavor-256.png` and `endeavor-512.png` (for
     `share/icons/hicolor/{256x256,512x512}/apps/endeavor.png`).
   - The Linux runtime dependencies above need to be declared.
2. **The notebook page's shortcuts (S).** The page script checks `metaKey`
   for ⌘K (ask Claude about a cell), ⌘⏎ in its prompt and comment boxes, and
   ⌘⇧K (leave Point). On Linux Ctrl+K in a cell does nothing. The page needs
   Ctrl on Linux, checked against Pluto's own Ctrl shortcuts, and its hints
   (the cell placeholder "⌘K to ask", the comment box's title) need the same
   `mac` switch the shortcuts list in `frontend/src/actions.ts` already has.
3. **The menu bar's items (S).** GPUI shows no menu bar on Linux, so the
   About window, Help ▸ Endeavor Help, Report an Issue and Window ▸ Zoom have
   no way in. Settings ▸ About has the version, updates, Help and Licences,
   and Troubleshooting has Report a problem, so the About window is the only
   thing missing. It needs a way in, such as a link in Settings ▸ About.
4. **Wayland (L).** wry can embed a child web view only in an X11 window. The
   app builds GPUI with only the `x11` feature, so on a Wayland desktop it runs
   through XWayland. That should work but is untested. Native Wayland needs a
   different way to embed the web view, likely including upstream work in
   wry and gpui-wry.
5. **Untested on Linux (S each):**
   - A full desktop (GNOME, KDE) with a compositor, including XWayland.
   - Finishing Claude sign-in, and a real Claude turn.
   - x86_64.
   - Reduce motion changed while the app runs (not followed on macOS either).
6. **Smaller gaps (S each):**
   - Without a compositor the notebook isn't dimmed behind Settings or a
     confirm dialog, and the corners of Settings' rounded panel show the
     notebook's edge.
   - Windows with their own title in the content, such as Licences, repeat
     the window manager's title.
   - The Browse… picker doesn't start in the current folder
     (`set_open_panel_message` and the start folder are macOS-only).
7. **Pinch zoom in the notebook (S).** This is macOS-only (`webkeys`).
   WebKitGTK zooms with Ctrl+scroll by default, which may be enough.
