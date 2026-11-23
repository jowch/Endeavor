# CI and the nightly

What each workflow in `.github/workflows/` checks, when it runs, and the
safeguards it keeps. How to run the same checks by hand is in
[testing.md](testing.md); building and packaging is in
[development.md](development.md).

## Every change

- **`ci.yml`** builds the app and runs its tests on macOS (the platform
  Endeavor ships on) and Linux, and checks the page script. `npm test`
  rebuilds `dist/page.js` and `dist/view.js` first, and the committed copies
  must match them. Tests that need a display, real Julia or a real Claude turn
  are ignored by default and don't run here.
- **`windows.yml`** builds the workspace on Windows and runs the tests that can
  run there ([windows.md](windows.md)). Tests that need Unix (sh, signals, Unix
  sockets, tar, the `#!/bin/sh` stand-in Julia) are compiled out with
  `cfg(unix)`; tests that wait on parts not yet ported are ignored with that
  reason. Checkouts keep LF line endings, since tests compare checked-in text
  byte for byte. On main and on pull requests it also keeps a release build
  and its installer:
  - A release build, because debug builds compile their shaders at launch
    from the crate's source folder and run only where they were built. The
    layout is `Endeavor\bin\endeavor.exe` and `Endeavor\Resources\`, which
    `install::resources()` looks for.
  - The C runtime is linked in, so the exe doesn't need the Visual C++
    Redistributable; WebView2's loader is linked statically already. A step
    fails if the exe needs a DLL a fresh Windows doesn't have.
  - The build ships no server helpers. It downloads the one a server needs
    from EndeavorMCP's Helpers release when it first connects, by the key of
    the EndeavorMCP commit `Cargo.lock` pins, and must carry that build's
    helper checksums (EndeavorMCP#422). It fails if the Helpers release has no
    build for the pinned commit (never built, or pruned by
    `scripts/prune-helpers.sh`) rather than ship an app that can't set up
    servers.
  - It starts the release build as a user would and fails if it exits or never
    shows its window (#198), keeping a screenshot of the setup screen.
  - The installer bundles Microsoft's Evergreen WebView2 bootstrapper, which
    changes with each Edge release, so its Microsoft signature is checked
    instead of a pin. Inno Setup comes from the runner image, with a pinned 6.x
    as the fallback so a new major version can't change the script's meaning.
  - A build of main is the one the nightly publishes and updates itself; a
    pull request's doesn't.
- **`docs.yml`** builds the documentation site in `site/` from `docs/guide`,
  with the link check. Only main publishes it to GitHub Pages.

## Windows paths CI runs for real

These download Julia or reach a server, so they run only when the code they
cover changes (including `Cargo.lock`, which pins EndeavorMCP), once a week,
and by hand.

- **`juliaup.yml`**: the first run on a Windows computer with no juliaup
  (#62). It installs juliaup for the account (the Store through winget, else
  juliaup's App Installer file), adds the pinned Julia's channel, and checks
  that the channel is juliaup's default, that the runtime finds the channel's
  own `julia.exe` rather than juliaup's launcher, and that Remove leaves the
  default in place (EndeavorMCP #115). Windows Server 2025 has winget; 2022
  doesn't, so it takes the App Installer route. That isn't a desktop whose
  Store is blocked, where winget is there and fails its own way. The job
  fails unless exactly the one test ran, since a filter that matches nothing
  passes with "0 passed", and says so when a retried App Installer install
  passed. The runner's inventory uses Windows PowerShell, since PowerShell 7
  can't load the Appx module there.
- **`windows-julia.yml`**: a real Julia notebook on Windows through the runtime
  the app runs (#198). Nothing else does: EndeavorMCP's real-Julia tests are
  Unix only. `scripts/julia-smoke.py` starts the app's exe as the plugin's
  stdio MCP server (`endeavor.exe --helper mcp`) and, as an agent would, makes
  a notebook, runs a cell, checks its output, closes it and stops the
  runtime. The runner has no Julia, so this is a first run (several minutes).
  It is a debug build, so `ENDEAVOR_TEST_BROWSER` keeps it from opening a
  browser. It doesn't run again on the push to main, since the pull request
  ran it on the same code.
- **`windows-server.yml`**: the Windows app on a Linux server over ssh (#257).
  The server is Ubuntu in WSL on the same runner, with sshd on
  127.0.0.1:2222. The same smoke script goes through EndeavorMCP's machine
  tools, which use the same ssh, channel and helper download as the app's Add
  server: `add_machine`, `use_machine`, a notebook and one cell, then
  `stop_machine`, saying yes to each install. The key has an empty passphrase
  (`-N ''`, which PowerShell 7.3+ passes as an empty string), since BatchMode
  can't type one.

## The nightly

**`nightly.yml`** keeps one "nightly" prerelease holding the newest build of
main that passed CI on macOS, Linux and Windows: the Mac disk image (with the
Linux servers' helpers inside), the Windows installer `windows.yml` built from
the same commit, and `nightly.json`, which an installed nightly reads to
update itself ([updates.md](updates.md)). Each run replaces the files and
moves the tag, so the links stay the same. A scheduled run does nothing when
the nightly already holds that commit, and publishes only while the
repository variable `NIGHTLY_PUBLISH` is true. A run by hand builds (and on
main signs) the disk image as an artifact and publishes only when asked. On a
pull request that changes the Mac packaging it builds the disk image from the
pull request, with a screenshot of its window and the manifest it would get,
and publishes nothing.

- **Signing.** The secrets live in the "release" environment, which only main
  may use, so pull requests never sign or publish. A publishing run stops
  before building if any of the six is missing, rather than publish a Mac
  build Gatekeeper blocks. The certificate goes in a throwaway keychain,
  deleted at the end of the job. Signing goes inside out: every Mach-O file in
  the app, then the app, with the hardened runtime and no entitlements, since
  the app only starts other programs and shows WKWebView, whose web content
  runs in WebKit's own processes.
- **The disk image** comes from `dmgbuild`, which writes the window's layout
  without Finder and copies the app with `ditto`, keeping its signature. The
  app comes from the picked commit, but the packaging (`assets/dmg`) from the
  workflow's own commit, since the picked commit can be older than the
  packaging. Stapling fetches the ticket from Apple's CloudKit, which drops
  the connection now and then, sometimes for hours, so it retries for about a
  quarter of an hour. The screenshot doesn't fail the build, since Finder on a
  hosted runner can be slow or refuse.
- **Helpers.** As on Windows, the build fails if EndeavorMCP's Helpers release
  has no build for the pinned commit, and must carry its helpers' checksums.
- **Publishing order.** The inputs are checked first, so a bad one stops the
  run before the release changes. The manifest goes up after the files it
  names, so an app that reads it early sees the previous manifest or a hash
  that doesn't match yet, and keeps its build. The tag moves last, so it means
  "this build is fully published": if anything failed, it still names the old
  commit and the next scheduled run tries again.

The Windows installer isn't signed (#12) and the Mac build is Apple Silicon
only; both are rows in [roadmap.md](roadmap.md)'s Known shortcuts.

## The cloud setup script

`scripts/cloud-setup.sh` provisions a Claude Code cloud VM
([cloud.md](cloud.md)). Each step checks first, and a step that can't finish
says why on stderr while the rest go on, so it never fails the session. The
steps that don't depend on each other run in parallel, since the
environment's cache is kept only when the script finishes in about five
minutes. `--no-gui` skips the app's Linux libraries and Xvfb, which
EndeavorMCP doesn't need. Its pins move with EndeavorMCP's
`crates/endeavor-mcp/src/julia.rs` (Julia), `rust-toolchain.toml` (Rust,
which comes without clippy and rustfmt, so the script adds them) and
`docs/marimo.md` (marimo). EndeavorMCP keeps a copy of the script; change
both.
