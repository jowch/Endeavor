# Development

How to build Endeavor from source and work on it. For how to check a change
from a script, see [testing.md](testing.md); for what CI checks and how
the nightly is published, [ci.md](ci.md). For status and plans, see
[roadmap.md](roadmap.md) and [pluto-agent-design-doc.md](pluto-agent-design-doc.md). How the parts
fit is in [agent-connection.md](agent-connection.md) (the agent),
[remote-sessions.md](remote-sessions.md) (runtimes on this computer and on
servers), [notebook-web-view.md](notebook-web-view.md) (the notebook in the
window) and [updates.md](updates.md).

## Run from source

```
scripts/helpers.sh   # Linux servers' runtime helper, into target/helpers
cargo build          # the app and its helper for macOS servers, endeavor-helper
cargo run
```

`cargo run` alone builds only the app, so run `cargo build` after pulling.

On Linux, install the system packages first: the list for Ubuntu and Debian
is in [linux.md](linux.md#system-packages-ubuntu-2604). Rust comes from
rustup, and `rust-toolchain.toml` picks the version.

Run a debug build from source. A release build (`cargo run --release`) looks
for its files only in `Resources` beside the folder it runs from, as in
`Endeavor.app`, so from the source tree it shows "Endeavor's files are
missing" and starts nothing. To try a release build, bundle it (below).

To keep the Linux helpers current by themselves, turn on the repo's git hooks
once: `git config core.hooksPath .githooks`. After each pull or branch switch
(not a checkout of single files) they fetch the helpers GitHub built for that
commit.

`scripts/helpers.sh` checks out the EndeavorMCP commit that `Cargo.lock`
pins (in `target/endeavor-mcp`) and downloads the helper for Linux servers
from EndeavorMCP's Helpers release when that source was built there, else
builds it (EndeavorMCP's `scripts/build-helpers.sh`); it does nothing when
it's already up to date. Without it the app can't connect to Linux servers.

A helper the app downloads for a server (a Windows build, or a Mac build for a
platform it doesn't carry) is checked against the checksums EndeavorMCP's
build embeds for its own key, from a pin that includes EndeavorMCP#444. It
embeds them only when `Cargo.lock` pins a
"Plugins: pin build KEY" commit, the one EndeavorMCP's Helpers pin job pushes
(EndeavorMCP#422). `scripts/pinned-sums.sh KEY` checks that, and the nightly
and the Windows builds fail without it, on pull requests too, so a pin that
would break `main` can't merge.

To change EndeavorMCP and the app together, see
[testing.md](testing.md#changing-endeavormcp-and-the-app-together).

## Build the app

`scripts/bundle.sh` builds `target/release/Endeavor.app`: ad-hoc signed, with
the Linux helpers inside. It runs `scripts/helpers.sh` itself.

Working in a Claude Code cloud session (no display, Linux) is in
[cloud.md](cloud.md).

## Other parts of the repo

- App icon, logo and the Mac disk image's background: scripts in `assets/`
  draw them ([artwork.md](artwork.md)).
- Fonts: `fonts/build.py` builds Endeavor Sans ([fonts/README.md](../fonts/README.md)).
- Page script (the code injected into the notebook page): `frontend/`,
  TypeScript. After changing it, run `cd frontend && npm install && npm test`.
  That builds `dist/page.js`, which is committed so `cargo build` needs no
  Node, and `dist/view.js`, the look and Point for EndeavorMCP's notebook view
  in Claude's desktop app (`src/view.ts`; EndeavorMCP keeps a copy).
- The user guide is in [guide/](guide/). Its conventions are in
  [guide/README.md](guide/README.md). The documentation site publishes it;
  see below.

## Documentation site

`site/` builds the user guide into a website with Astro and Starlight, and
publishes it at <https://jowch.github.io/Endeavor/>. The pages stay in
`docs/guide/`, and the site reads them from there, so edit them in place.
The site's own files are the home page, the theme
(`site/src/styles/theme.css`) and its config (`site/astro.config.mjs`).

The home page is a product page of its own, not a Starlight page. Its words
are in `site/src/landing.ts`, and `site/src/pages/index.astro` lays them out
with its own styles. The turtle at the top is `site/src/components/Splash.astro`,
a canvas port of the app's setup splash (`src/splash.rs` and `src/turtle.rs`);
change it there too when the app's splash changes. The page shows screenshots
from `docs/guide/images/` (`main-window.png`, `approval-card.png`,
`safe-preview.png`, `point.png` and `cluster-resources.png`), so the build
fails while one of them is missing.

To run it locally, with Node.js 22 or later:

```
cd site && npm ci && npm run dev
```

Then open <http://localhost:4321/Endeavor/>. The dev server also shows pages
marked `draft: true`, with a notice; the published site leaves them out.

`npm run build` builds the site into `site/dist` and then checks every link
between pages, including `#` anchors, every image size in a `srcset`, and the
home page's `og:image`. A broken link fails the build.

`.github/workflows/docs.yml` builds the site on pull requests that touch the
guide, the site or the icons, and builds and publishes it on pushes to main.

GitHub Pages has to be turned on once, by someone with admin rights on the
repo: in the repo's **Settings**, **Pages**, under **Build and deployment**,
set **Source** to **GitHub Actions**. Until then the publish step fails.

## What the app installs

On first launch the app downloads and verifies Julia and Node.js and installs
the pinned ACP adapter into the app's data folder: `~/Library/Application
Support/endeavor/` on macOS, `~/.local/share/endeavor/` on Linux (or
`$XDG_DATA_HOME/endeavor`), `%LOCALAPPDATA%\Endeavor\` on Windows
(`install::app_dir`).
