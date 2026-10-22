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
