# The notebook's web view in the window

The notebook is Pluto's page in a native web view: WKWebView on macOS,
WebKitGTK in an X11 child window on Linux, WebView2 in a child window on
Windows. It sits on top of everything GPUI draws, so the app has to manage
three things GPUI can't: what shows over it, where the keyboard goes, and
what the page itself does (find, its process ending, pictures of it).
`src/platform.rs` holds the per-platform entry points; the macOS code is in
`overlay.rs`, `webkeys.rs`, `webcontent.rs` and `snapshot.rs`, Linux's in
`src/linux/`, and Windows' in `overlay_windows.rs`, `webview_windows.rs` and
`snapshot_windows.rs`.

## What floats over it

Everything that floats (a menu, popover, notice, dialog) is placed with
`layer::place` at a `Layer` from `src/layer.rs`. GPUI paints deferred
elements by priority, and equal priorities in the order they were rendered.
Numbers picked at each site drifted (the ⋮ menu and the failure notice were
both 1, so the notice covered the menu, #234), so the order lives in `Layer`
alone. Something the user just opened goes above something that only tells
them, and a menu inside a panel has its own layer just above that panel.

A floating thing shows over the notebook only where it cuts a hole in the
web view (`layer::cut`). The hole is pinned to a plain box around the body,
since drawn inside the body it would start inside its border and padding and
the web view would cover its edge. A menu, a tip and a tooltip can each have
a hole at once, and each hole keeps its corner radius, so the notebook shows
right up to rounded corners. A menu over the notebook shows at once, without
its fade: the hole can't fade with it.

- **macOS** masks the web view's layer around the holes, and routes the
  window's events (`send_event`): a click in a hole goes straight to GPUI's
  view, because the web view claims clicks even where it declines to
  hit-test (WebKit's gesture recognizers take mouse-downs over the whole
  view). A press that went to GPUI keeps going there until it ends, since
  AppKit sends a mouse-up only to the view that took the mouse-down (#282).
  A routed click also makes GPUI's view the first responder and its text
  input context current, which AppKit does only for events through the
  window's own `sendEvent`; without that, clicks on GPUI's controls over the
  page were lost until one landed outside the web view. A dialog over the
  page takes the keys from it; a menu, tip or notice leaves them, since the
  user may be typing. Behind a panel, the web view is dimmed to match the
  scrim.
- **Linux** cuts the holes out of wry's container window with the X Shape
  extension. Without a compositor there are no see-through child windows, so
  the web view isn't dimmed behind a panel. wry maps and unmaps its windows
  with Xlib behind GDK's back, so the app asks X, not GDK, whether a window
  shows.
- **Windows** sets a window region on wry's container window. A region can't
  let the notebook show while its clicks go elsewhere, so while a menu or
  popover is open the web view's window is disabled instead, and Windows
  gives its clicks to GPUI's window. A disabled window keeps the keyboard if
  it has it, and then keys go nowhere, so GPUI's window takes it first. GPUI
  draws without DirectComposition there: its composition target is topmost,
  which would cover the web view's window. As on Linux, the web view isn't
  dimmed behind a panel.

While any menu or popover is open, a mouse-down anywhere on the web view goes
to GPUI too, so the shared click-outside handling closes it, as a native menu
closes for a click anywhere outside it (`Workspace::dismissible_open`).

Tooltips over the notebook close their hole in a frame that draws none
(`tooltip_hole_keeper`): GPUI hides a tooltip while drawing a frame but keeps
its view until it draws the next one, which may be much later. The pointer
moving from GPUI onto the web view leaves GPUI's window without a last mouse
move, so the app sends one from outside the window, which closes tooltips and
clears hovers.

## The keyboard

A click GPUI gets was outside the web view or in a hole, so the keyboard goes
to GPUI's window (`web_view_hooks`, drawn every frame). gpui-wry does this
only outside the web view's bounds, so without it a click in Settings over
the notebook left the keyboard in the notebook. On Linux, gpui-wry answers
the same click by focusing the web view's container, so the app's move is
deferred and sent on GDK's own Xlib connection, to reach the X server second.

The app's shortcuts reach GPUI while the page has the keyboard:

- **macOS.** WebKit handles arrow keys as key equivalents, but GPUI's root
  view answers every key equivalent itself, and wry makes a child web view
  refuse them all so menu shortcuts fire. Refused, arrow keys fall through to
  text input and type private-use glyphs, breaking CodeMirror's cursor keys.
  So while the keyboard is in the web view, GPUI's view offers the key to it
  first, and the web view gives the app's menu first pick and hands the rest
  to WKWebView's own handling.
- **Linux.** GTK sends keys to the web view, so the app hands them to GPUI in
  the same order as on macOS: the menu bar's shortcuts before the page sees
  them, other Ctrl shortcuts only when the page doesn't use them (WebKit
  hands an unhandled key to the web view's parent). GTK runs key handlers in
  the order they're connected. X sends keys to the window under the pointer
  when it is inside the focused window, so with GPUI focused the web view can
  still get keys; those go on to GPUI. GDK follows X focus only for top-level
  windows, so the app tells GTK when the web view has the keyboard; without
  that the page thinks it's in the background, editors draw no cursor (#145)
  and paste does nothing (#147).
- **Windows.** The menu bar's shortcuts go to GPUI first, less Ctrl+M, which
  is Pluto's. WebView2 doesn't say which keys the page used, so the app's
  other Ctrl shortcuts stay with the page.

Since gpui-pre 0.3.11 a `secondary-` key fails to parse, so bindings are
spelled `cmd-` or `ctrl-` at build time (`platform::keys!`), and a test
checks that every app-first key parses.

Giving the keyboard back to the page on Windows waits until the key that
asked is handled (Esc in the find bar asked while GPUI still handled it, and
a MoveFocus then left the keyboard nowhere), and the page puts its own focus
and caret back, since after a find WebView2 returns the keyboard to the page
but not to the cell. Cut, Copy, Paste and Select All from the app's menu go
in through the DevTools protocol there: WebView2 has no editing commands and
a script's `execCommand` can't paste. Linux's WebKit takes the command by
name.

## Find, a page that ended, and pictures

- **Find** uses each engine's own. WebKit gives no count; hiding its
  highlight means searching for a character no page has. WebKitGTK answers
  each search once, in order, without saying which. WebView2 raises
  MatchCountChanged only for a change, and the count it has when a search
  starts can be the last search's, so a search is answered once the count has
  stayed put for `SETTLE_MS`.
- **The page's process ending** (a crash, or the system ending it for memory)
  leaves the view blank; the app loads the page again. On macOS, wry's
  navigation delegate has no such method, so it is added to the delegate's
  class and the delegate set again, since WebKit reads a delegate's methods
  when it's set; having it also stops WebKit reloading by itself. The user's
  view and the cell pictures' second view share the class, so handlers go by
  view. wry's `url()` panics after the process ended, so the app reads the
  address itself.
- **A box drawn with Point** becomes a PNG of that rect from the engine's own
  snapshot (WKWebView's, WebKitGTK's, or DevTools' `Page.captureScreenshot`),
  which draws the page itself, plots and WebGL included, without
  screen-recording permission. Wide boxes are scaled down to keep the PNG
  well under the image cap.
- **The right-click menu** loses the items that open a new window for an
  image, video or frame: the app opens no second window, and a figure's
  `data:` or `blob:` URL has no browser to go to (#151).
