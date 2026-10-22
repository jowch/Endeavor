<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/icon/logo-dark.svg">
    <img src="assets/icon/logo-light.svg" alt="Endeavor" height="46">
  </picture>
</h1>

Endeavor is a desktop app for analyzing data with an AI agent, reproducibly.
You ask in a chat, and the agent (Claude Code, Codex or Antigravity) writes
and runs code in a live Julia or R notebook beside it, while you watch, edit
and decide what runs. The notebook, with its code and results, is the record
of the analysis that you keep, check and run again.

- **Notebooks:** Julia ([Pluto.jl](https://plutojl.org)) and R
  ([Ember](https://github.com/jowch/Ember)). R isn't on Windows yet. Python,
  through [marimo](https://marimo.io), is coming soon.
- **Where they run:** on your computer, on a server you reach over ssh, or in
  a Slurm job on a cluster. Servers and clusters aren't on Windows yet.
- **Agents:** Claude Code and Codex. Antigravity is tested less.
- **Platforms:** test builds for Apple silicon Macs and Windows 10 or 11 update
  themselves. On Linux, [build it from source](#build-it). It runs under X11;
  under Wayland it should run through XWayland, which hasn't been tested.

<!-- agent-install: start. A copy of Endeavor's site/agent-install.md; edit it there and run site/scripts/agent-install.mjs. -->
<details>
<summary><strong>Installing with an agent?</strong> The commands for Claude Code, Codex and Antigravity.</summary>

To use Endeavor from an agent you already run, install the EndeavorMCP
plugin into it. The plugin gives the agent Endeavor's notebook tools: live
Julia and R notebooks that it edits and runs while you watch in a browser.
The Endeavor desktop app is a separate download that a person installs:
[Install Endeavor](https://jowch.github.io/Endeavor/install/).

**Claude Code:**

```sh
claude plugin marketplace add jowch/EndeavorMCP
claude plugin install endeavor@endeavor
```

**Codex:**

```sh
codex plugin marketplace add jowch/EndeavorMCP
codex plugin add endeavor@endeavor
```

**Antigravity** (the `agy` command line; tested less than the other two).
Give the URL of the `antigravity-plugin` folder, as here. The repository's own
URL installs the Claude Code plugin over this one, and the notebook tools
are then missing.

```sh
agy plugin install https://github.com/jowch/EndeavorMCP/tree/main/antigravity-plugin
```

On Windows, Antigravity needs Git for Windows' `bin` folder on the PATH, and
Codex isn't supported.

**Check that it worked.** `claude plugin list` (or `codex plugin list`) shows
`endeavor@endeavor` enabled. The tools load when a session starts, so ask the
user to start a new session in the folder for their notebooks. There, the
`endeavor` server's `session_status` tool answers without starting anything.
Its first start downloads the `endeavor` program (about 5 MB); if the tools
don't appear, reconnect the server (`/mcp` in Claude Code).

**Update an old plugin.** Claude Code: `claude plugin marketplace update
endeavor`, then `claude plugin update endeavor@endeavor`. Codex: `codex plugin
marketplace upgrade endeavor`. Antigravity: run the install line again. Then
start a new session. A notebook runtime that is already running keeps its old
build until it stops ([Update and remove](https://github.com/jowch/EndeavorMCP#update-and-remove)).

More detail for each agent is in
[EndeavorMCP's README](https://github.com/jowch/EndeavorMCP#install). The same
text for agents is at <https://jowch.github.io/Endeavor/llms.txt>.

</details>
<!-- agent-install: end -->

## Use it

Read the [user guide](docs/guide/overview.md), also published at
<https://jowch.github.io/Endeavor/>.

Download the test builds for
[Mac](https://github.com/jowch/Endeavor/releases/download/nightly/Endeavor-macos-arm64.dmg)
and [Windows](https://github.com/jowch/Endeavor/releases/download/nightly/Endeavor-windows-x86_64-setup.exe)
from the [nightly release](https://github.com/jowch/Endeavor/releases/tag/nightly);
the guide says [how to install them](docs/guide/install.md).

The notebook tools also work without the app, as a plugin for Claude Code,
Codex or Antigravity, on your computer, a workstation or a cluster node (the
block above has the commands). See
[EndeavorMCP's README](https://github.com/jowch/EndeavorMCP#readme).

## Build it

```
scripts/helpers.sh   # Linux servers' runtime helper, into target/helpers
cargo build          # the app and its helper for macOS servers
cargo run
```

On Linux, install the system packages first. The list for Ubuntu and Debian
is in [docs/linux.md](docs/linux.md#system-packages-ubuntu-2604).

`scripts/bundle.sh` builds `target/release/Endeavor.app`. More on building,
the parts of the repo and its git hooks is in
[docs/development.md](docs/development.md). Checking changes from a script is
in [docs/testing.md](docs/testing.md).
