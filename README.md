<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/icon/logo-dark.svg">
    <img src="assets/icon/logo-light.svg" alt="Endeavor" height="46">
  </picture>
</h1>

Endeavor is a Mac app for doing your own data analysis with Claude. You ask
in a chat, and Claude writes and runs Julia code in a live Pluto.jl notebook
beside it, while you watch, edit and decide what runs. The notebook, with its
code and results, is the record of the analysis that you keep and can check.

## Use it

Read the [user guide](docs/guide/overview.md), also published at
<https://jowch.github.io/Endeavor/>.

Test builds for Apple silicon Macs and Windows are on the
[nightly release](https://github.com/jowch/Endeavor/releases/tag/nightly);
the guide says [how to install them](docs/guide/install.md).

The notebook tools also work without the app: run `endeavor serve` on a
workstation or cluster node and connect any MCP agent and a browser. See
[EndeavorMCP's README](https://github.com/jowch/EndeavorMCP#readme).

## Build it

```
scripts/helpers.sh   # Linux servers' runtime helper, into target/helpers
cargo build          # the app and its helper for macOS servers
cargo run
```

`scripts/bundle.sh` builds `target/release/Endeavor.app`. More on building,
the parts of the repo and its git hooks is in
[docs/development.md](docs/development.md). Checking changes from a script is
in [docs/testing.md](docs/testing.md).
