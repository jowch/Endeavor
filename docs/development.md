# Development

How to build Endeavor from source and work on it. For how to check a change
from a script, see [testing.md](testing.md). For status and plans, see
[roadmap.md](roadmap.md) and [pluto-agent-design-doc.md](pluto-agent-design-doc.md).

## Run from source

```
scripts/helpers.sh   # Linux servers' runtime helper, into target/helpers
cargo build          # the app and its helper for macOS servers, endeavor-helper
cargo run
```

`cargo run` alone builds only the app, so run `cargo build` after pulling.

To keep the Linux helpers current by themselves, turn on the repo's git hooks
once: `git config core.hooksPath .githooks`. After each pull or branch switch
they fetch the helpers GitHub built for that commit.

`scripts/helpers.sh` checks out the EndeavorMCP commit that `Cargo.lock`
pins (in `target/endeavor-mcp`) and downloads the helper for Linux servers
from EndeavorMCP's Helpers release when that source was built there, else
builds it (EndeavorMCP's `scripts/build-helpers.sh`); it does nothing when
it's already up to date. Without it the app can't connect to Linux servers.

To change EndeavorMCP and the app together, see
[testing.md](testing.md#changing-endeavormcp-and-the-app-together).

## Build the app

`scripts/bundle.sh` builds `target/release/Endeavor.app`: ad-hoc signed, with
the Linux helpers inside. It runs `scripts/helpers.sh` itself.

## Other parts of the repo

- App icon and logo: `python3 assets/icon/build.py` regenerates them from the
  artwork in that script.
- Page script (the code injected into the notebook page): `frontend/`,
  TypeScript. After changing it, run `cd frontend && npm install && npm test`.
  That builds `dist/page.js`, which is committed so `cargo build` needs no
  Node.
- The user guide is in [guide/](guide/). Its conventions are in
  [guide/README.md](guide/README.md).

## What the app installs

On first launch the app downloads and verifies Julia and Node.js and installs
the pinned ACP adapter into `~/Library/Application Support/endeavor/`.
