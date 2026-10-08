# Endeavor

A native desktop app (Rust, GPUI) for doing data analysis with an AI agent:
a chat with Claude Code (or Codex) over ACP on one side, the live Pluto.jl
notebook it writes and runs on the other. macOS is the shipping platform;
Linux runs under X11; Windows builds and tests in CI only.

The notebook runtime, MCP server and skills live in
[EndeavorMCP](https://github.com/jowch/EndeavorMCP), pinned by commit in
`Cargo.lock` (`wire`, `endeavor-mcp`).

## Where things are

- `src/`: the app, one crate. `main.rs` and `session.rs` are the hubs;
  `agent.rs`/`agent_process.rs`/`codex.rs` the ACP side; `connection.rs`,
  `remote.rs`, `runtime.rs` the runtimes on this computer and on servers;
  `notebook_pane.rs`, `webcontent.rs` the notebook web view; `debug_state.rs`
  the state dump; `linux/` the X11/GTK glue.
- `frontend/`: the TypeScript injected into Pluto's page. `dist/page.js` is
  built and **committed** (so `cargo build` needs no Node).
- `adapter/`, `adapter-codex/`: the pinned ACP adapters the app installs.
- `site/`: the documentation website, built from `docs/guide/`.
- `docs/`: start with `pluto-agent-design-doc.md` (why), `ui-spec.md` (how it
  looks and behaves), `roadmap.md`, `design-gaps.md`, `testing.md`,
  `development.md`. `marimo.md` is the design for the Python backend (not
  built).

## Build, test, lint

```sh
cargo build --locked --workspace --all-targets   # builds the app and endeavor-helper
cargo test --locked --workspace --no-fail-fast
cargo clippy --locked --workspace --all-targets   # about 25 warnings today; don't add more
cd frontend && npm install && npm test && npm run -s check   # rebuilds dist/page.js, then tests
```

- After changing `frontend/src/`, commit the rebuilt `dist/page.js` with it.
- The code is **not** rustfmt-formatted. Don't run `cargo fmt` on the tree;
  match the surrounding style.
- Check a feature change through the state dump, not screenshots:
  `scripts/app-state.sh <jq filter>` against a debug build
  ([docs/testing.md](docs/testing.md)). Take a screenshot only when how it
  looks is the change.
- A runtime, transport or helper change also needs EndeavorMCP's real-Julia
  tests and the smoke test with a real Claude turn (docs/testing.md).

## Rules that aren't obvious from the code

- **Changing EndeavorMCP and the app together:** use a `[patch]` in
  `.cargo/config.toml` (git ignores it) against a local checkout; land the
  EndeavorMCP change on its `main` first and wait for its Helpers release;
  then remove the patch, `cargo update -p endeavor-mcp`, and commit
  `Cargo.lock`. Never commit `Cargo.lock` while the patch is in place.
  `tests/helper_mode.rs` expects both crates at the same version.
- **The app never mutates a notebook directly.** All writes go through the
  agent's MCP tools and Pluto's session API.
- **Pluto stays on loopback** behind its secret. Never widen it, and never let
  the app's proxying go beyond loopback.
- **Pins move together:** Julia (`src/runtime.rs`, EndeavorMCP's
  `src/julia.rs`, `scripts/cloud-setup.sh`), Node and the adapters
  (`src/install.rs`, `adapter*/package-lock.json`). A test checks the cloud
  script's Julia and Rust pins.
- Deliberate shortcuts are marked `ponytail:` in the code and listed in
  `docs/roadmap.md`.

## Working in a Claude Code cloud session

[docs/cloud.md](docs/cloud.md) has the details. In short: `scripts/cloud-setup.sh`
installs the Linux libraries, Xvfb, Julia, marimo, clippy and rustfmt (the
SessionStart hook runs it in a session with only this repo; a Claude project
needs it as the environment's setup script). The app runs under `Xvfb :99`
and its state dump works there. Without Julia's hosts allowed, the app stops
at "Couldn't set up Julia"; without an API key there is no real Claude turn.
Say which of these limited a check rather than claiming it passed.

## Style

Commit subjects name the area and say what changed in plain words
("Sidebar: a new reply is a plain orange ring, so it isn't mistaken for the
filled needs-you dot"); the body says why. UI text and docs use plain, short
sentences and the user's words, not internal names. Keep `roadmap.md`,
`design-gaps.md` and the guide true when a change affects them.
