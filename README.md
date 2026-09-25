# Endeavor

A native macOS app with a Claude Code agent beside a live Pluto.jl notebook.
See [docs/roadmap.md](docs/roadmap.md) for status and
[docs/pluto-agent-design-doc.md](docs/pluto-agent-design-doc.md) for the design.

- Run from source: `cargo run`
- Build the app: `scripts/bundle.sh` → `target/release/Endeavor.app` (ad-hoc signed)

On first launch the app downloads and verifies Julia and Node.js and installs
the pinned ACP adapter into `~/Library/Application Support/endeavor/`. Settings
(bottom of the session bar) switch to your own julia, load your personal Claude
Code setup, or let new sessions run notebook code without asking.
