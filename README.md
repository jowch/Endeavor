<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/icon/logo-dark.svg">
    <img src="assets/icon/logo-light.svg" alt="Endeavor" height="46">
  </picture>
</h1>

A native macOS app with a Claude Code agent beside a live Pluto.jl notebook.
See [docs/roadmap.md](docs/roadmap.md) for status and
[docs/pluto-agent-design-doc.md](docs/pluto-agent-design-doc.md) for the design.

The notebook tools (the runtime, the MCP server, the server helper and the
Pluto skills) live in [EndeavorMCP](https://github.com/jowch/EndeavorMCP),
which the app depends on as a Cargo git dependency. They also work without
the app: run `endeavor serve` on a workstation or cluster node and
connect any MCP agent and a browser. See
[EndeavorMCP's README](https://github.com/jowch/EndeavorMCP#readme).

Run from source:

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
[docs/testing.md](docs/testing.md#changing-endeavormcp-and-the-app-together).

Build the app: `scripts/bundle.sh` → `target/release/Endeavor.app` (ad-hoc
signed, with the Linux helpers inside; it runs `scripts/helpers.sh` itself).

- Check changes from a script: [docs/testing.md](docs/testing.md)
- App icon and logo: `python3 assets/icon/build.py` regenerates them from the artwork in that script.
- Page script (the code injected into the notebook page): `frontend/`, TypeScript.
  After changing it: `cd frontend && npm install && npm test` (builds
  `dist/page.js`, which is committed so `cargo build` needs no Node).

On first launch the app downloads and verifies Julia and Node.js and installs
the pinned ACP adapter into `~/Library/Application Support/endeavor/`. Settings
(bottom of the session bar) switch to your own julia, load your personal Claude
Code setup, or let new sessions run notebook code without asking.
