<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/icon/logo-dark.svg">
    <img src="assets/icon/logo-light.svg" alt="Endeavor" height="46">
  </picture>
</h1>

A native macOS app with a Claude Code agent beside a live Pluto.jl notebook.
See [docs/roadmap.md](docs/roadmap.md) for status and
[docs/pluto-agent-design-doc.md](docs/pluto-agent-design-doc.md) for the design.

- Run from source: `cargo run`
- Check changes from a script: [docs/testing.md](docs/testing.md)
- Connect to Linux servers from a source build: `scripts/build-helpers.sh` first
  (builds their runtime helper into `target/helpers`)
- Build the app: `scripts/bundle.sh` → `target/release/Endeavor.app` (ad-hoc signed)
- App icon and logo: `python3 assets/icon/build.py` regenerates them from the artwork in that script.
- Page script (the code injected into the notebook page): `frontend/`, TypeScript.
  After changing it: `cd frontend && npm install && npm test` (builds
  `dist/page.js`, which is committed so `cargo build` needs no Node).

On first launch the app downloads and verifies Julia and Node.js and installs
the pinned ACP adapter into `~/Library/Application Support/endeavor/`. Settings
(bottom of the session bar) switch to your own julia, load your personal Claude
Code setup, or let new sessions run notebook code without asking.
