# marimo notebooks

Design for adding marimo (reactive Python notebooks) as a second notebook
backend next to Pluto. Nothing here is built yet. Settled work moves to
[roadmap.md](roadmap.md) once scheduled.

_Drafted 2026-09-26, against marimo 0.25.0 (released 2026-09-23)_

## Summary

marimo is the closest match to Pluto among Python notebooks: a notebook is a
plain `.py` file, each global is defined in one cell, and editing a cell
reruns the cells that depend on it. That keeps the notebook a verifiable
record, which is the reason Endeavor centres on Pluto.

We add it the way Pluto is added today: a runtime process on the host (this
Mac or a server) runs the notebook server and serves the agent's notebook
tools over the same bridge. The app, the relay and the agent see the same
protocol for both; what differs sits behind one boundary in the app and in the
page script injected into the notebook view.

Two things make this more than a port:

- **marimo has no stable control API.** The complete way to edit and run cells
  from outside is `marimo._code_mode`, whose docstring says "No versioning
  guarantees. May change or be removed without notice." marimo releases about
  weekly. We pin the version and test against it, as we pin Julia.
- **Cell IDs are not stored in the file.** marimo derives them from cell
  position when it loads a notebook, so they change across reloads and when a
  cell is inserted above. Pluto's IDs are in the file and last forever.

## Decisions

- **Build the notebook-backend boundary first**, designed against marimo
  rather than Jupyter. This is the `notebook-model` boundary of
  [pluto-agent-design-doc.md](pluto-agent-design-doc.md) §8, which no code
  implements yet. marimo shares Pluto's model (reactive graph, plain-text
  file), so the boundary stays small; Jupyter stays in
  [roadmap.md](roadmap.md) "Later".
- **Same tool names and shapes for both backends.** The agent calls
  `read_cell`, `edit_cell`, `execute_cell`, `get_cell_dependencies` and the
  rest whatever the notebook is. Language differences go in the skills, not
  the tool list. A tool one backend can't support returns a plain error.
- **Run through marimo's own server, not a reimplementation.** The runtime
  starts `marimo edit --headless` and drives it; the notebook pane shows
  marimo's unmodified frontend, as it shows Pluto's today.
- **Pin marimo and Python.** The runtime ships a `uv.lock` that pins marimo
  and its dependencies, and uv provides Python, as the app pins Julia 1.12.6.
  Upgrading marimo is a deliberate change that runs the runtime's tests.
- **Sandboxed notebooks.** Every notebook runs with `--sandbox` (its own uv
  environment built from the PEP 723 header at the top of the file), so its
  packages are declared in the file.

## How it fits the current design

Today:

- `crates/endeavor-remote` starts `julia --project=runtime runtime/boot.jl
  <pluto_port> <mcp_port>` with `ENDEAVOR_TOKEN`, `ENDEAVOR_STATE` and
  `ENDEAVOR_LAUNCHER` set. Once up, `boot.jl` writes `runtime.json`
  (ports, Pluto secret), and the helper relays the two loopback ports.
- `EndeavorRuntime` (Julia, about 3,200 lines) runs Pluto in-process and
  serves the agent's MCP tools plus the app's `/call` methods and `/events`
  stream.
- The app shows Pluto's page in a webview and injects `frontend/dist/page.js`,
  which reads Pluto's DOM for change highlighting, annotations and theming.

### What already works for both

- The ssh relay and the wire protocol (only field names say "pluto").
- The bridge: JSON-RPC over HTTP/SSE, bearer token, and the `/events` schema
  `{notebooks, cells: [cell_id, running, errored, unrun, author, before,
  version, name]}`.
- Messages between the app and the page script (string notebook and cell IDs).
- Run policy, idle stop, sharing checks, and the host tools (`list_folder`,
  `read_file`, `run_shell`).

### What becomes backend-specific

| Where | Today | Change |
| --- | --- | --- |
| Tool prefix `mcp__pluto__` in `gate.rs`, `celldiff.rs`, `session.rs` | hard-coded | one name for the bridge MCP server (e.g. `notebook`), used by both |
| Notebook detection, `crates/wire/src/notebooks.rs` | Pluto header, `# ╔═╡` cells | also detect `app = marimo.App` in `.py` files; parse `@app.cell` for the new-session preview |
| Notebook URL and ID, `main.rs:53` | `/edit?id=` | per backend (marimo's form, believed `/?file=`, to confirm) |
| Annotation URI, `annotate.rs` | `pluto://notebook/…/cell/…` | `notebook://<backend>/…` |
| Page script, `frontend/` | Pluto DOM, `--pluto-*` variables | a second adapter for marimo's DOM and CSS variables |
| Runtime launch, `endeavor-remote`, `remote.rs`, `runtime.rs` | find or download Julia, ship `runtime/` | also find or download uv; ship `runtime-py/` |
| Runtime state, `runtime.json` | one runtime per host | one per backend per host, each started lazily when a notebook of its kind opens |
| Settings, splash | "My julia", "Restart Julia" | per backend, shown only once that backend is used |
| Skills, prompt text in `main.rs` | Pluto and Julia content | marimo versions, chosen by the session's notebook |

The app-side boundary is a small enum, not a trait object: `Backend::{Pluto,
Marimo}` with the handful of differences above (detection, URL, page adapter,
launch command, skill set). Two known backends don't need dynamic dispatch.

## The Python runtime

A new directory `runtime-py/` holds a Python package, `endeavor_runtime`,
with a `uv.lock`. The helper starts it the same way as the Julia runtime:

```
uv run --project runtime-py python -m endeavor_runtime <notebook_port> <mcp_port>
```

with the same environment variables, and it writes the same `runtime.json`
(with `notebook_port` and `notebook_secret` in place of the Pluto names, and a
`backend` field).

Inside, it:

1. Starts `marimo edit --headless --sandbox --host 127.0.0.1 --port
   <notebook_port> --token-password-file … --watch` as a child process.
   Whether one server can sandbox every notebook in a folder, or needs one
   server per notebook, is to confirm in step 2 of the build order.
2. Opens a websocket client to each open notebook. A headless marimo server
   has no kernel until a client connects, and may end the kernel when the
   last client leaves; the runtime's client keeps notebooks running when the
   app disconnects (our "notebooks keep running" rule). It also sets
   `--session-ttl` high enough to cover a reconnect.
3. Turns marimo's websocket messages (`kernel-ready`, `cell-op`,
   `notebook-document-transaction`) into the bridge's `/events` stream.
4. Serves the agent's tools. Reads come from the websocket state; edits and
   runs go through marimo's code-mode execute endpoint (what `marimo pair`
   uses), running `marimo._code_mode` calls in the kernel.
5. Serves the host tools, ported from `HostTools.jl`.

All calls into marimo internals live in one module, `marimo_api.py`, so an
upgrade touches one file. The Julia runtime's lesson
([design-notes.md](design-notes.md): Pluto calls ended up spread across eight
files) applies here too.

### Differences the runtime has to cover

- **Cell IDs.** See [Open questions](#open-questions). Within one kernel
  session marimo's IDs are stable, which is enough for the tools, `author` and
  `before` tracking, and staged edits.
- **Running a cell runs its stale ancestors first.** marimo's `run_cell` does
  not, and the result is a `NameError`. `execute_cell` runs the unrun
  ancestors, then the cell, matching Pluto.
- **Opening a notebook runs it** (subject to run policy, as with Pluto's safe
  preview). marimo's default is `auto_instantiate = false`; the runtime calls
  instantiate once policy allows execution.
- **Lazy mode for external edits.** With `--watch`, marimo marks changed cells
  stale rather than running them. The runtime reports them as `unrun`, the
  same state Pluto's staged edits produce.
- **Mutation is not tracked.** `df.append(...)` in one cell does not rerun
  readers in another. Pluto has the same rule; the marimo skill says so, and
  `validate_cell` warns on mutation of a global defined elsewhere.
- **Outputs.** marimo returns HTML and mime bundles. `view_cell_output`
  returns text directly and renders images through the kernel, as
  `Output.jl` does for Pluto. Errors map to the same structured form.
- **Dependency tools.** `get_cell_dependencies`, `get_cell_dependents` and
  the symbol lookups read marimo's graph (`ctx`'s cell defs and refs).
- **`read_notebook_code`** hides marimo's boilerplate (`import marimo`,
  `app = marimo.App`, the decorator and `return` lines) the way the Pluto
  version hides package cells and `@bind` shims.
- **`@bind` equivalent.** `mo.ui.*` elements; the tools read and set their
  values through code mode (`set_ui_value`).

## Reproducibility

Pluto always writes the full package manifest into the notebook. marimo's
sandbox writes loose requirements into the PEP 723 header; exact pins are
optional and live in a separate `nb.py.lock` (`uv lock --script nb.py`).

To keep marimo notebooks as trustworthy a record, the runtime:

- runs every notebook with `--sandbox`;
- writes or updates `nb.py.lock` next to the notebook whenever its packages
  change, and treats the pair as the record;
- adds packages through code mode (`ctx.packages.add`), which updates the
  header, rather than letting the agent `pip install`.

## Testing

- **Runtime:** pytest against a real `marimo edit --headless` on a fixture
  notebook, mirroring `runtime/EndeavorRuntime/test/runtests.jl`: every tool,
  read guards, staging, reactivity, events. A separate small suite covers
  only `marimo_api.py`, so a marimo upgrade shows exactly which internal call
  broke.
- **Page script:** the existing node:test + JSDOM setup with a fake marimo
  DOM next to the fake `<pluto-cell>` one.
- **Rust:** notebook detection and preview parsing for marimo files; the
  `Backend` choices.
- **Remote helper:** the fake-runtime tests in
  `crates/endeavor-remote/tests/connect.rs` gain a Python-runtime case.

## Build order

1. The app-side boundary with Pluto as the only backend: rename the MCP server
   and URI scheme, add `Backend`, move Pluto-specific strings behind it. No
   behaviour change; existing tests pass.
2. `runtime-py` with the launch contract, marimo server, websocket client and
   `/events`, plus read-only tools. The app opens and shows a marimo notebook.
3. Editing and running tools, staging, run policy, ancestors-first runs.
4. Page script adapter for marimo (change highlighting, annotations, theme).
5. Skills for marimo; session prompt text by backend.
6. Sandboxes and lockfiles; remote launch (uv download on servers).

Each step ends in a working app.

## Open questions

- **Durable cell identity.** Chat history and annotations refer to cells by
  ID, and a marimo ID changes when the notebook is reloaded. Options:
  1. Accept session-scoped IDs. After a reload, old references are resolved
     by matching cell code, and unmatched ones are shown as "cell no longer
     found". No change to the user's file. (Recommended to start.)
  2. Give every cell a function name (`def load_data(...)`), which marimo
     keeps in the file. Durable, but it writes names into the user's
     notebook and names must be unique identifiers.
- **Lockfile policy.** Is a separate `nb.py.lock` acceptable, or should exact
  pins go in the PEP 723 header so the notebook is one file, as with Pluto?
- **One folder, both kinds.** A folder with both `.jl` Pluto notebooks and
  marimo `.py` notebooks needs both runtimes on the host. Starting each only
  when a notebook of its kind opens avoids the cost for most users; confirm
  that's the behaviour we want.
- **marimo's own AI features.** marimo has built-in AI editing and an MCP
  server. We turn off Pluto's AI features (`enable_ai_editor_features=false`);
  do the same for marimo so there is one agent.
- **Upstream.** marimo's maintainers are building agent support (`marimo
  pair`, code mode). A stable, public version of that API would remove most
  of the risk here; worth raising with them once we have concrete needs.

## Tracking

[jowch/Endeavor#1](https://github.com/jowch/Endeavor/issues/1)
