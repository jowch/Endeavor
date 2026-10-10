# Working on Endeavor in Claude Code cloud sessions

How the cloud VMs that Claude Code runs in are set up for this repository and
EndeavorMCP, and what an agent can and can't check there.

_Written 2026-10-08, from a session that built and tested both repositories
on the cloud VM._

## What a cloud VM can do

The VM is Ubuntu 24.04, x86_64, running as root, with no display. With
`scripts/cloud-setup.sh` run:

| Check | Works | How |
| --- | --- | --- |
| Build the app and its tests | Yes | `cargo build`, `cargo test --workspace` (about 6 min cold) |
| Page script tests and typecheck | Yes | `cd frontend && npm test && npm run -s check` |
| Run the app and read its state dump | Yes | under Xvfb, below |
| EndeavorMCP's tests | Yes | `cargo test --workspace` there; its real-Julia tests need Julia's hosts (EndeavorMCP's CLAUDE.md) |
| Real Julia: local runtime, `e2e_julia`, notebooks in the app | Only if the network allows Julia's hosts | below |
| A real Claude turn in the app or through the plugin | Only with an API key | below |
| macOS-only behaviour (menu bar, Metal, notarization) | No | needs a Mac |

## Network hosts

The VM reaches the internet through a proxy that allows only the
environment's list. The default ("Trusted") list covers GitHub, crates.io,
npm, PyPI and `nodejs.org`, but not Julia. Add these under **Allowed domains**
with **Also include default list of common package managers** ticked:

```
julialang-s3.julialang.org
pkg.julialang.org
*.pkg.julialang.org
```

The first is the Julia download the app, the runtime and the setup script all
pin. The others serve Julia packages (Pluto and its dependencies):
`pkg.julialang.org` sends each client on to a regional server such as
`us-east.pkg.julialang.org`.

R notebooks need more. The first one installs Ember from
`codeload.github.com` and any package Ember needs that Ubuntu's R lacks from
`cloud.r-project.org`. A notebook's own packages come from
`packagemanager.posit.co` (CRAN at the notebook's date) and
`bioconductor.org`. Add those that the list doesn't already allow.

## The setup script

`scripts/cloud-setup.sh` installs the app's Linux libraries (the list in
[linux.md](linux.md)), Xvfb and Openbox, Julia 1.12.6 (checked against the
SHA-256 the app pins), R from Ubuntu's archive with the packages Ember needs,
marimo 0.25.1 as a uv tool, and clippy and rustfmt for the pinned Rust. It is
safe to run again, never fails the session, and takes a few minutes on a fresh
VM. EndeavorMCP keeps a copy; tests in both
repositories check that its Julia and Rust pins match the code's.

Where it runs:

- **A session with only this repository** runs it from the SessionStart hook
  in `.claude/settings.json`, which also installs the page script's npm
  packages.
- **A Claude project, or any session with both repositories**, doesn't run
  repository hooks. The environment's **Setup script** has to do it:

  ```bash
  #!/bin/bash
  curl -fsSL https://raw.githubusercontent.com/jowch/Endeavor/main/scripts/cloud-setup.sh | bash || true
  ```

  The environment caches the VM after the setup script, so a change to the
  script on `main` reaches new sessions only when the cache is rebuilt (when
  the environment's settings change, or after about a week). To pick it up
  sooner, run `scripts/cloud-setup.sh` in the session.

## Running the app without a display

```sh
Xvfb :99 -screen 0 1600x1000x24 &
mkdir -p /tmp/e
DISPLAY=:99 ENDEAVOR_STATE_REQUEST=/tmp/e/state.request ENDEAVOR_STATE_OUT=/tmp/e/state.json \
  target/debug/endeavor > /tmp/e/app.log 2>&1 &
ENDEAVOR_STATE_REQUEST=/tmp/e/state.request ENDEAVOR_STATE_OUT=/tmp/e/state.json \
  scripts/app-state.sh .window
DISPLAY=:99 import -window root /tmp/e/shot.png   # a screenshot, when how it looks matters
```

Check behaviour through the state dump ([testing.md](testing.md)), not
screenshots. `xdotool` drives clicks and keys. Without Julia's hosts the app
stops at the splash with "Couldn't set up Julia", which the dump shows as
`.window.setup.failed`.

## The accessibility tree

GPUI builds its accessibility tree only while a screen reader is on. To read
it under Xvfb, start a session bus and the accessibility bus, switch the
screen reader flag on, then start the app on that bus:

```sh
apt-get install -y at-spi2-core          # the bus launcher, once
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address=1 | head -1)
/usr/libexec/at-spi-bus-launcher --launch-immediately &
for flag in IsEnabled ScreenReaderEnabled; do
  dbus-send --session --dest=org.a11y.Bus /org/a11y/bus org.freedesktop.DBus.Properties.Set \
    string:org.a11y.Status string:$flag variant:boolean:true
done
DISPLAY=:99 target/debug/endeavor &      # with the state dump's variables as above
python3.12 scripts/atspi-tree.py         # the tree, as Orca gets it
python3.12 scripts/atspi-tree.py "Sign in"   # click a node by name, as a screen reader does
```

The system `python3` may not match the installed `gi`; `python3.12` does on
this VM. AccessKit on Linux and macOS doesn't pass on whether a control is
expanded; only Windows does. On a Mac, `scripts/ax-tree.swift` reads the
same tree as VoiceOver gets it.

A signed-in Claude hides the sign-in card. To see it, start the app with
`ENDEAVOR_CLAUDE_CLI` set to a script that prints `{"loggedIn": false}` for
`auth status` ([testing.md](testing.md)).

## A real Claude turn

The app's agent (Claude Code through the ACP adapter) and the plugin need a
signed-in Claude. A cloud VM has none, so the smoke test in
[testing.md](testing.md) and any trial of the skills can't run there unless
the environment gives the session an API key. That key is billed per use.
