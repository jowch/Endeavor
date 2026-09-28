# Endeavor on Linux

_Status as of 2026-09-27. Tested only on Ubuntu 26.04 aarch64 (the OrbStack
machine `endeavor-linux`), under Xvfb. There is no x86_64 test yet._

**Summary:** the app builds and runs on Linux under X11. The window opens,
setup installs Node and the adapter, the local Julia runtime starts, and the
Pluto notebook shows in the web view and runs cells. Claude sign-in gets as far
as opening the browser. Two gaps make it unusable day to day:

- Keyboard focus doesn't come back from the notebook to the app.
- Menus that open over the notebook are hidden behind it.

The remaining work is ranked at the end of this page.

## What works

Screenshots of each step are in the L1 run's `shots/l1-*.png`.

- **Build.** `cargo build` and `cargo test --workspace` both work. On Linux one
  test fails; see [the remaining work](#remaining-work-ranked).
- **Window and UI.** GPUI renders the setup screen, the session list, the chat
  pane and the notebook pane, with correct fonts. Text boxes take typing, and
  Ctrl+B toggles the sidebar.
- **First-run setup.** The app downloads and checks the pinned Linux Node.js,
  runs `npm ci` for the adapter, and precompiles Pluto and EndeavorRuntime.
- **Local runtime.** `endeavor-remote` starts Julia 1.12.6 and reports the
  Pluto and MCP ports. Quitting the app stops Julia.
- **Notebook.** Pluto loads in the WebKitGTK web view at the right place and
  size, in dark mode. **Run notebook code** runs cells (`x = sum(1:10)` shows
  55). Typing, arrow keys and Shift+Enter work in cells. Editing a cell shows
  "You edited `x`" in the chat, so the page script and IPC work.
- **Claude sign-in.** "Claude subscription" runs `claude auth login --claudeai`.
  That command opens the OAuth URL with `xdg-open`. Nobody finished a sign-in:
  that needs a real account in the browser.

## What changed for Linux

- `Cargo.toml` adds these Linux-only dependencies:
  - GPUI's `x11` feature.
  - `gtk` 0.18, the version wry already uses.

  `serde` also moved to the general dependencies. It had been listed as
  macOS-only by mistake.
- `src/platform.rs` holds the code that differs by OS:
  - **Reveal:** macOS runs `open -R`. Linux runs `xdg-open` on the folder.
  - **Reduce motion:** macOS reads NSWorkspace. Linux reads GTK's
    `gtk-enable-animations`.
  - **Web view host:** wry's Linux web view is WebKitGTK in an X11 child window.
    GTK is initialised at launch and its events are pumped every 8 ms from
    GPUI's loop. GPUI gives an XCB window handle, but wry accepts only Xlib, so
    `webview_parent` passes the same X window ID as an Xlib handle.
  - **No-op stubs** stand in for `overlay` (menus over the notebook),
    `webkeys` (key routing and pinch zoom) and `dialogs` (the page's alert(),
    confirm() and prompt() as macOS sheets). All three use Objective-C and stay
    macOS-only. WebKitGTK shows the page's dialogs with its own GTK dialogs by
    default, so they may already work here; that is untested, including
    whether the dialog appears over the app's window.
- **Files:** app data goes to `$XDG_DATA_HOME/endeavor` (default
  `~/.local/share/endeavor`). The log goes to `$XDG_STATE_HOME/endeavor/endeavor.log`
  (default `~/.local/state/endeavor/endeavor.log`).
- **Pinned downloads:** Linux Julia and Node tarballs are pinned for aarch64
  and x86_64. The Julia pins are the ones the helper already uses.
- **Light and dark:** the web view follows GTK's
  `gtk-application-prefer-dark-theme`. WebKitGTK's `prefers-color-scheme`
  follows that setting.
- **Shortcuts:** the app's shortcuts use `secondary-`, which is Cmd on macOS
  and Ctrl on Linux.
- **Folder picker:** the NSOpenPanel start-folder default is macOS-only.

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
```

At runtime the binary links these libraries: WebKitGTK 4.1, GTK 3, xcb,
xkbcommon(-x11), fontconfig and libwayland-client. `mesa-vulkan-drivers`
provides llvmpipe, a software GPU driver, for a VM with no GPU. The app also
calls `curl`, `tar` and `shasum` (Ubuntu's `perl` package provides `shasum`).

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

3. Start Xvfb and the app:

   ```
   ssh endeavor-linux 'Xvfb :99 -screen 0 1600x1000x24 & cd endeavor && DISPLAY=:99 nohup target/debug/endeavor >/dev/null 2>&1 &'
   ```

4. Take a screenshot, or click and type:

   ```
   ssh endeavor-linux 'DISPLAY=:99 import -window root png:-' > shot.png
   ssh endeavor-linux 'DISPLAY=:99 xdotool mousemove 800 680 click 1'
   ```

5. Stop the app and Xvfb:

   ```
   ssh endeavor-linux 'pkill -x endeavor; pkill -x Xvfb'
   ```

In an OrbStack machine, `xdg-open` is OrbStack's own version, which opens URLs
in the Mac's browser. To see sign-in URLs without opening them, put a fake
`xdg-open` first on `PATH` that only logs its arguments.

Setup waits until Claude is signed in. To reach the workspace without an
account, the L1 test patched the VM's copy of `src/main.rs` so that
`SignedIn(false)` counted as signed in. That patch isn't committed.

## Remaining work, ranked

Sizes: S is under a day, M is 1–3 days, L is more than that or needs upstream
changes.

1. **Keyboard focus between the app and the notebook (M).** After a click in
   the notebook, X keyboard focus stays in WebKitGTK's window. Clicking a GPUI
   text box doesn't take it back, because gpui-wry's `focus_parent` focuses
   wry's GTK container, not GPUI's X window. Checked with
   `xdotool getwindowfocus`: typing reached the chat box only after focus was
   forced to GPUI's window. The fix is to set X input focus to GPUI's window
   when a click lands outside the web view. The same work is needed for app
   shortcuts while the notebook has the keyboard (Ctrl+B, zoom, Ctrl+Q). GTK
   receives those keys, and the macOS equivalent in `webkeys` doesn't exist
   here.
2. **Menus over the notebook are hidden (M).** An example is the notebook's
   "…" menu. The web view is an X11 child window, so it's always above what
   GPUI draws. `overlay::set_hole` could be ported with the XShape extension:
   cut the hole out of the web view window's bounding and input regions. The
   other option is to hide the web view while a menu is open.
3. **Linux packaging (M).**
   - `scripts/bundle.sh` builds only a macOS .app.
   - `install::resources()` looks for `../Resources` next to the executable.
     Linux needs its own layout, such as a tarball, AppImage or .deb with
     `share/endeavor/{runtime,plugin,adapter}`.
   - Linux also needs a `.desktop` file. Its icon is ready:
     `assets/icon/endeavor-256.png` and `endeavor-512.png` (for
     `share/icons/hicolor/{256x256,512x512}/apps/endeavor.png`).
   - The Linux runtime dependencies above need to be declared.
4. **Wayland (L).** wry can embed a child web view only in an X11 window. The
   app builds GPUI with only the `x11` feature, so on a Wayland desktop it runs
   through XWayland. That should work but is untested. Native Wayland needs a
   different way to embed the web view, likely including upstream work in
   wry and gpui-wry.
5. **Mac-specific wording (S).** Examples: "This Mac" (about 40 places),
   "Reveal in Finder", and "the user's Mac" in the agent's prompt. These need
   an OS-neutral name such as "This computer", or a per-OS name.
6. **Title bar and window frame (S–M).**
   - The header still leaves room at the left for macOS window buttons, which
     Linux doesn't have.
   - The window's frame under a real window manager is untested: Xvfb has no
     window manager, so the window had no frame and no title bar. The app asks
     for `appears_transparent`.
   - Linux needs a decision between window-manager decorations and GPUI's own
     client-side decorations.
7. **GTK event pump (S).** GTK events are checked every 8 ms, which keeps the
   CPU busy even when the app is idle. Better: add GLib's main-context file
   descriptors to GPUI's calloop loop, or pump only while the web view is
   visible.
8. **Untested on Linux (S each):**
   - Finishing Claude sign-in.
   - The Browse… folder picker. GPUI uses xdg-desktop-portal, which must be
     installed and running.
   - Settings.
   - Remote servers and clusters started from a Linux client.
   - Following the system light/dark setting.
   - Reduce motion on a real desktop.
   - x86_64.
10. **Pinch zoom in the notebook (S).** This is macOS-only (`webkeys`).
    WebKitGTK zooms with Ctrl+scroll by default, which may be enough.
