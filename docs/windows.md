# Endeavor on Windows: what a port would take

_Estimate as of 2026-09-28; status updated 2026-10-03. Step 1 of the
suggested order is done: the workspace type-checks for Windows, and a Windows
CI workflow builds it and runs its tests. Nothing has run on a Windows
machine yet, and the workflow hasn't run either. The rest comes from reading
the code, the dependencies' sources, and [linux.md](linux.md), which records
how the Linux port went._

**Summary:** a Windows port is feasible, and no single item blocks it. It is
about **6–8 weeks** of work for one person to reach a usable app: roughly 3–4
weeks in the app and 2–3 weeks in `endeavor-remote` and `wire`. It costs more
than Linux did because Linux and macOS are both Unix, so the process and IPC
code carried over almost unchanged. On Windows that code needs replacing: Unix
signals, process groups, Unix sockets and `flock`. The UI and the web view
should carry over better than they did on Linux.

The scope follows [remote-sessions.md](remote-sessions.md): the app runs on
Windows, runs Julia locally, and connects over ssh to Linux and macOS servers.
A Windows *server* is out of scope.

## Status: it builds, with stubs

`cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` passes
on a Mac with no errors. Its one warning (`PREFIX` unused in `src/notify.rs`)
shows on Linux too. macOS and Linux behave as before: the Windows code sits
behind `#[cfg(windows)]` and the Unix code behind `#[cfg(unix)]`, as the Linux
port did with `src/platform.rs`.

What the check showed about the dependencies:

- **gpui-pre 0.3.6, gpui-pre-windows, lb-wry 0.53.3, webview2-com and
  gpui-component all type-check for Windows.** No dependency blocks the port.
- **gpui-pre's build script needs a resource compiler** for its Windows
  manifest (embed-resource): `rc.exe` from the Windows SDK, or `llvm-rc`. A
  Mac has neither, so checking for Windows on a Mac needs a stand-in `llvm-rc`
  on `PATH` (a script that answers `/?` with `OVERVIEW: LLVM Resource
  Converter` and writes an empty file at the path after `/fo`). Nothing links
  in `cargo check`, so the empty resource doesn't matter. On Windows the SDK
  that comes with Visual Studio has `rc.exe`.
- **gpui-pre-windows compiles its shaders with `fxc.exe` in release builds**
  (debug builds compile them at launch from the crate's source folder, so a
  debug build runs only on the machine that built it). It comes with the
  Windows SDK too.
- Two small crates were added for Windows only, both already in the tree:
  `chrono` in the app (the time zone) and `getrandom` in `endeavor-remote`
  (the bridge token).

The estimate doesn't change. Step 1 took under a day, as sized.

### What runs on Windows now

On Windows the app should open, but it can't yet run a notebook or reach a
server. These refuse with a plain error rather than half-work:

| Where | What it says | What it needs |
|---|---|---|
| `crates/endeavor-remote/src/lib.rs`, `start` | "Running Julia on Windows isn't supported yet." | Process control: start the core detached, Julia and its workers in a Job Object. |
| `src/runtime.rs`, `julia_binary` | Endeavor can't install Julia on Windows yet | Julia's win64 zip pinned, `bin\julia.exe`, the SHA-256 checked in Rust. |
| `src/agent.rs`, `adapter_command` | Endeavor can't install Node.js on Windows yet | Node's win-x64 zip pinned, `node.exe` and npm's layout. |
| `src/remote.rs`, `Askpass` | "Endeavor can't connect to servers from Windows yet." | The askpass transport (loopback TCP or a named pipe), and an answer on `SSH_ASKPASS_REQUIRE`. |
| `crates/endeavor-remote/src/askpass.rs`, `ask_app` | ssh prompts aren't supported on Windows yet | The same transport, on the helper's side. |

### Stubs left

Each stub is marked "Not ported" in the code.

**`endeavor-remote` (the app's local runtime runs it too):**

- `lib.rs`, `Runtime::kill`: logs and does nothing. Needs `TerminateProcess`
  on the core, which closes its Job Object.
- `lib.rs`, `stop_workers`: does nothing. The Job Object replaces it.
- `lib.rs`, `pid_alive`: always false, so no recorded runtime counts as
  running. Needs `OpenProcess` plus `GetExitCodeProcess`, and the process's
  start time, since Windows reuses pids quickly.
- `lib.rs`, `ask_to_hand_over`, `watch_replace_signal`, `block_sigusr1`: no
  handover between clients, so a second client waits for the lock (about 30 s)
  and fails. Needs a named event, or a "release" call on a loopback port.
- `core.rs`, `main`: Julia starts as the core's child, but nothing ends it
  with the core (Unix: the process group, `PR_SET_PDEATHSIG`, and passing on
  stop signals). Needs the Job Object.
- `http.rs`, `wait_readable`: doesn't watch the client, so a client that hangs
  up during an event stream is noticed only when a write to it fails. Needs
  `WSAPoll`.
- `mcp.rs`, `closed`: always false, so a tool call goes on after its client
  hangs up. Needs a non-blocking peek with winsock's `recv`.

These stay Unix-only on purpose, because they run only on a Linux or macOS
server: `host_tools.rs`'s `run_shell` (refuses on Windows), and in `slurm.rs`
the job script's `exec` and the SIGTERM to `srun`.

Ported for real: the bridge token's random bytes (`getrandom`), the host name
(`COMPUTERNAME`), stdin and stdout as files (their handles), file times
(`Metadata::modified`), the home folder (`std::env::home_dir`), and the state
folder's lock (`File::try_lock`). Files that are 0600 on Unix get no special
permissions on Windows: the state folder is meant to live under the user's
own `%LOCALAPPDATA%`.

**The app:**

- `src/platform.rs`, `overlay`: no holes in the web view, so menus, tips,
  Settings and dialogs show under the notebook. Needs `SetWindowRgn`, or
  hiding the web view while one is open.
- `src/platform.rs`, `webcontent`: a crashed WebView2 process isn't noticed
  (`ProcessFailed`), find in the notebook always says "Not found", and
  `has_keyboard` is always false. `url` and `give_keyboard` (wry's `focus`)
  are real.
- `src/platform.rs`, `web_view_hooks` and `init`: nothing, so app shortcuts
  don't reach GPUI while the notebook has the keyboard. Needs WebView2's
  `AcceleratorKeyPressed`.
- `src/platform.rs`, `reduces_motion`: always false. Needs
  `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`.
- `src/platform.rs`, `set_open_panel_message`, `snapshot`, `dialogs`: the
  same no-ops as on Linux.
- `src/network.rs`, `monitor`: says the network is up once and never again.
  Needs `NotifyIpInterfaceChange`.
- `src/logs.rs`, `start`: no redirection to the log file. Needs
  `SetStdHandle`. It returns before that anyway, since `logs::path` reads
  `HOME`.
- `src/runtime.rs`, `stop_group`: logs and does nothing, so Repair runtime
  can't stop a runtime (none can start yet).
- `src/runtime.rs`, `connect`: the helper doesn't get its own process group,
  and the depot list still ends in `:` (Windows needs `;`).
- `src/remote.rs`, `kill_group`: does nothing. No ssh runs, since `Askpass`
  refuses.
- `src/signin.rs`, `Login::cancel`: doesn't stop the sign-in's CLI. Needs a
  Job Object.
- `src/install.rs`, `app_dir`, and the other uses of `HOME`: Windows has no
  `HOME`, so these fail or return nothing. This is the "Home and app data"
  item under Paths.

Ported for real: the time zone offset (`src/when.rs`, `src/trouble.rs`, through
chrono), Reveal (`explorer /select,`), the notebook's light and dark theme
(WebView2's `set_theme`), Bring All to Front, and the server tarball's
executable bits (`src/remote.rs`: nothing in `runtime/` is executable, and the
helper is marked by name).

### CI

`.github/workflows/windows.yml` runs on every push and pull request, on
`windows-latest`: `cargo build --locked --workspace --all-targets`, then
`cargo test --locked --workspace --no-fail-fast`. It turns off git's CRLF
conversion before checkout. It hasn't run yet, so its first run may still
show Windows-only test failures that reading the code missed.

Compiled out on Windows with `#[cfg(unix)]`, because they need `sh`, signals,
`tar`, symlinks or Unix sockets:

- `crates/endeavor-remote/tests/connect.rs`, `core.rs` and `e2e_julia.rs`
  (whole files: the stand-in Julia is a `#!/bin/sh` script).
- `tests/helper_mode.rs` (whole file).
- `crates/wire/src/relay.rs`: all its tests (`UnixStream::pair`).
- `crates/wire/src/files.rs`: `uploads_stay_inside_the_session_folder`.
- `src/remote.rs`: `the_bootstrap_survives_any_login_shell`,
  `tar_holds_the_helper_and_runtime`,
  `bootstrap_installs_then_reuses_the_helper_and_attaches`,
  `a_helper_that_ends_with_no_julia_is_a_drop_and_a_detach_is_not`,
  `a_server_without_a_helper_build_is_refused_plainly`, `askpass_round_trip`,
  `a_cancelled_password_prompt_ends_the_connect`.
- `src/runtime.rs`: `repair_clears_stale_state_and_keeps_the_rest`.
- `crates/endeavor-remote/src/notebooks/tests.rs`: the `file_info` check in
  `the_apps_notebook_actions_restart_move_file_info_and_new_notebook`.

Ignored on Windows (`#[cfg_attr(windows, ignore = "…")]`), because they wait
on parts not ported yet. Step 2 should turn them back on:

- `crates/endeavor-remote/src/notebooks/tests.rs`, notebook paths:
  `opening_and_making_notebooks`, `one_notebook_per_session`,
  `list_notebooks_says_which_notebook_is_this_sessions`,
  `the_apps_notebook_actions_restart_move_file_info_and_new_notebook`,
  `stop_notebook_shuts_it_down_and_says_if_it_was_in_safe_preview`,
  `idle_notebooks_stop_but_running_kept_alive_and_recently_used_ones_dont`,
  `a_notebooks_state_goes_when_it_shuts_down_however_it_shuts_down`,
  `parses_ids_and_paths_as_julia_did`.
- `crates/endeavor-remote/src/julia.rs`, the home folder:
  `versions_and_home_paths`.
- `crates/wire/src/files.rs`, the home folder: `home_relative_paths`.
- `src/hosts.rs`, the app data folder:
  `server_sessions_share_one_agent_folder_per_server`.
- `src/install.rs`, downloads: `installs_a_verified_tarball_and_rejects_a_bad_one`.

The tests marked `#[ignore]` on every OS (the live and real-Julia ones) stay
ignored.

## What should carry over

- **GPUI** has a Windows backend: `gpui-pre-platform` 0.3.6 depends on
  `gpui-pre-windows` 0.3.6, a snapshot of Zed's `gpui_windows` (about 12,800
  lines). It draws with DirectX through DirectComposition, and it handles IME
  input and the native file and folder picker. Only two functions are left
  unimplemented, both macOS ideas that Endeavor doesn't call
  (`hide_other_apps`, `unhide_other_apps`). Zed's Windows support is new, so
  this backend has had much less use than the macOS one. Expect bugs that
  Zed hasn't met yet, particularly around child windows such as the web view.
- **The web view.** lb-wry 0.53.3 embeds WebView2 as a child window of the
  app's window (`build_as_child` takes a Win32 handle). gpui-wry's
  `focus_parent` gives keyboard focus back to that window, so the worst Linux
  bug (focus stuck in the notebook) probably doesn't happen here.
- **Page dialogs.** WebView2 shows the page's alert(), confirm() and prompt()
  itself, so `dialogs` can likely stay a no-op.
- **`runtime/`** (the Julia code) is nearly portable. `chmod(tmp, 0o600)` in
  `boot.jl` does little on Windows, but the per-user permissions on
  `%LOCALAPPDATA%` cover the same need.
- **The relay** (`wire::relay`) is plain frames over ssh's stdin and stdout.
  Port forwarding happens on the server, so the client needs no `ssh -L`.
- **Server-only code** runs only on the Linux or macOS server, so it can be
  gated with `#[cfg(unix)]` and left alone. This covers `host_tools.rs` (shell
  and file tools), `slurm.rs`, and the Julia download and shell-install paths
  in `julia.rs`.

## Work, by area

Sizes: S is under a day, M is 1–3 days, L is more than that.

### Process control (L, about 1–1.5 weeks)

This is the largest item. Today the code:

- starts the runtime in its own session with `setsid`
  (`crates/endeavor-remote/src/lib.rs`, `start()`), and stops it with
  `kill(-pid, SIGTERM/SIGKILL)` (`signal_group`, `stop_workers`);
- checks whether a pid is alive with `kill(pid, 0)` (`pid_alive`);
- takes the one-client lock with `flock` (lib.rs:595) and hands a runtime over
  to a new client with SIGUSR1 (lib.rs:614, 634–655);
- blocks and forwards signals in the core, and ties Julia's life to the core
  with `PR_SET_PDEATHSIG` (`core.rs:85`);
- kills process groups from the app too (`src/runtime.rs`, `src/signin.rs`,
  `src/remote.rs`).

The Windows equivalents:

- Start the core detached (`DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP |
  CREATE_BREAKAWAY_FROM_JOB`).
- The core puts Julia in a Job Object with `KILL_ON_JOB_CLOSE`. That replaces
  both `PDEATHSIG` and the group kill, and Pluto's worker processes join the
  same job.
- A graceful stop already exists over the bridge (`endeavor/shutdown`). A
  hard stop becomes `TerminateProcess` on the core, which closes the job.
- `pid_alive` becomes `OpenProcess` plus `GetExitCodeProcess`. Windows reuses
  pids quickly, so also record the process start time.
- The lock becomes `std::fs::File::try_lock`, which works on every OS (done
  on Windows; Unix keeps `flock`).
- The handover signal becomes a named event, or a "release" call on a
  loopback port.
- Smaller items: `/dev/urandom` becomes `getrandom`, `gethostname` becomes
  `COMPUTERNAME`, `stdout_mux` uses the stdout handle, and Windows skips
  `arg0` and file modes (all done). The `poll`/`recv(MSG_PEEK)` calls in
  `http.rs` and `mcp.rs` still need Windows versions (stubs, see above).

### ssh from Windows (M if askpass works, L if not)

- **Askpass transport (S).** The password and 2FA prompts reach the app over a
  Unix socket (`src/remote.rs:619`, `crates/endeavor-remote/src/askpass.rs:34`).
  Rust's standard library has no Unix sockets on Windows. Use loopback TCP
  plus a per-launch token in an environment variable, or a named pipe with a
  user-only ACL.
- **Connection sharing.** Windows OpenSSH has no ControlMaster. The app
  doesn't use it, but a user's `~/.ssh/config` might turn it on. Pass
  `-o ControlMaster=no -o ControlPath=none` to the ssh command
  (`Transport::command` in `src/remote.rs`).
- **Open question:** whether the ssh in Windows 10 and 11 honours
  `SSH_ASKPASS_REQUIRE=force`, including for Duo prompts (the same question
  as in remote-sessions.md). If it doesn't, the fallbacks are running ssh
  under a hidden console (ConPTY) or an ssh library in Rust such as `russh`.
  Either adds about two weeks.

### Paths (M, about 1 week)

- **Server paths vs local paths.** Wire messages carry server paths as
  `PathBuf` (`wire/src/files.rs`, `wire/src/notebooks.rs`). On a Windows
  client, `join` would put `\` into Linux paths. Make server paths a `String`
  or a `RemotePath` type, and update the app's call sites.
- **Home and app data.** `HOME` becomes `%LOCALAPPDATA%\Endeavor` for app data
  and logs, and `std::env::home_dir()` for `.ssh` and Downloads
  (`src/install.rs`, `src/logs.rs`, `src/hosts.rs`, `wire/src/files.rs`).
- **List separators.** PATH is joined with `:`; use `env::join_paths`. The
  depot string `"{}/depot:"` (`src/runtime.rs`) needs `;`, because
  `JULIA_DEPOT_PATH` uses `;` on Windows.
- **Canonical paths.** `fs::canonicalize` returns `\\?\C:\…` on Windows, which
  won't match Julia's `realpath` (`notebooks.rs`, `canonical_path`). Use the
  `dunce` crate or ask Julia.
- **Upload names** (`files.rs`) must also reject `\`, `:` and reserved names
  such as `CON`.

### First-run setup and downloads (S–M, 2–3 days)

- Pin Windows builds: Node `win-x64` and `win-arm64` zips, and Julia `win64`.
  There is no official Windows ARM64 build of Julia 1.12 that we know of, so
  ship x64 first.
- Windows 10 and later include `curl.exe` and `tar.exe`, and that `tar` reads
  .zip. There is no `shasum`, so hash in Rust with `sha2`.
- Executable layout differs: `bin\julia.exe`, `node.exe` at the top of the
  Node folder, and npm at `node_modules\npm\bin\npm-cli.js`
  (`src/install.rs`, `src/agent.rs`, `src/runtime.rs`).

### Notebook view (M–L, about 1–1.5 weeks)

- **Menus over the notebook (M).** WebView2 is a child window, so it draws
  above GPUI's content, as on Linux. Port `overlay::set_hole` by cutting a
  hole in the web view window with `SetWindowRgn`. It's unknown whether
  GPUI's DirectComposition surface shows through that hole. The fallback is
  to hide the web view while a menu is open.
- **App shortcuts while the notebook has focus (M).** Use WebView2's
  `AcceleratorKeyPressed` to send Ctrl+B, Ctrl+Q, Ctrl+, and the zoom keys to
  GPUI actions (the macOS version is `webkeys.rs`). Turn off browser
  accelerator keys so F5, Ctrl+P and Ctrl+F don't act on Pluto.
- **Snapshot (S–M).** Use `CapturePreview` and crop, or the DevTools
  `Page.captureScreenshot` with a clip.
- **Small items (S).**
  - Dark mode: `set_theme` (done).
  - Reveal: `explorer /select,<path>` (done).
  - Reduce motion: `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`.
  - Open a URL: `ShellExecute`.
  - Set a WebView2 data folder: the default, next to the exe, isn't writable
    under Program Files.
- **Custom scheme (S).** wry serves `endeavor://` as
  `http://endeavor.localhost/` on Windows. The page's font URLs hard-code
  `endeavor://localhost/` (`frontend/src/theme.ts:104`).

### Smaller app fixes (about 1 week in total)

- **Console windows (S–M).** Build as a GUI app
  (`#![windows_subsystem = "windows"]`) and pass `CREATE_NO_WINDOW` to every
  child process, or each one flashes a console. `logs.rs` redirects output
  with `dup2`, which needs a Windows version.
- **Network-change watch (S).** Only macOS and Linux versions exist
  (`src/network.rs`). Windows: `NotifyIpInterfaceChange` or
  `INetworkListManager`.
- **Time zone (S, done).** chrono gives the offset on Windows.
- **Server install tarball (S, done).** On Windows no `runtime/` file is
  executable, and the helper is marked by name (`src/remote.rs`).
- **Title bar (S–M).** The header leaves room for the macOS window buttons.
  Windows needs its own caption buttons, drawn by GPUI or native.
- **Wording (S).** "This Mac" (about 60 places in `src/`), "Reveal in Finder", and
  `HostId::ThisMac`. This is shared with Linux; see
  [linux.md](linux.md#remaining-work-ranked).
- **Cargo (S, done).** The `std::os::unix` imports and Unix-only `libc`
  calls are gated. The `windows` crate isn't added yet; the stubs above will
  need it.

### Claude Code on Windows (S–M, uncertain)

Claude Code on Windows has needed Git Bash. Setup may need to find Git for
Windows or set `CLAUDE_CODE_GIT_BASH_PATH`.

### Packaging (M–L, about 1 week)

- `scripts/bundle.sh` builds only a macOS .app. Windows needs an installer
  (MSI, MSIX or Inno Setup) and an icon resource from `build.rs`.
- Sign the app with Authenticode, or SmartScreen warns every user.
- Windows 11 includes WebView2. Windows 10 may not, so bundle Microsoft's
  Evergreen bootstrapper.
- `install::resources()` looks for `../Resources` next to the executable.
  Windows needs its own layout.
- The installer must carry the Linux and macOS server helpers. The macOS
  helper can't be cross-built from Windows, so the release build needs a Mac.

### Tests (M, 3–5 days; gating and CI done)

Many tests use `sh -c`, `kill -9`, `pkill`, `tar`, `shasum` or a fake Julia
written as a `#!/bin/sh` script (`crates/endeavor-remote/tests/common/mod.rs`),
and `wire`'s relay tests use `UnixStream::pair`. These are gated with
`#[cfg(unix)]` now, and a Windows CI workflow runs the rest (see
[CI](#ci)). Left: replace the fake Julia with a small Rust test binary so the
helper and core tests run on Windows, and give the relay tests a loopback TCP
pair.

## Limits we can't fix from Endeavor

- **Stopping a running cell.** Endeavor stops cells through Pluto
  (`Adapter.jl:396`, `interrupt_workspace`). On Windows, Pluto's worker
  library sends an interrupt message instead of a signal, so it lands only
  when the code yields. We believe this from memory of that library; its
  source hasn't been checked. A tight loop can't be stopped. The app should
  offer "Restart notebook" when a stop doesn't take effect.
- **Windows on ARM.** No official Julia 1.12 build that we know of. x64 Julia
  may run under emulation, but that is untested.

## Risks, ranked

1. **ssh prompts.** Whether Windows OpenSSH shows askpass prompts decides
   whether ssh work takes days or weeks. Test this first, on a real Windows 11
   machine, against a server with Duo.
2. **Process control.** Getting the Job Object and detaching right so that
   Pluto workers never leak, and handling pid reuse.
3. **Menus over the notebook.** The `SetWindowRgn` hole may not work with how
   GPUI draws. Hiding the web view is the fallback.
4. **GPUI's Windows backend is new.** It looks complete, but it has had
   little use, and Zed doesn't embed a child window the way Endeavor does.
5. **Claude Code's shell requirements** on Windows.

## Suggested order

1. Make it build: gate the server-only modules with `#[cfg(unix)]` and add
   Windows stubs to `src/platform.rs`, as the Linux port did. **Done**, with a
   Windows CI workflow (see [Status](#status-it-builds-with-stubs)).
2. Run a local notebook: process control, downloads, paths.
3. Test ssh askpass on Windows 11. The result sets the size of the ssh work.
4. Notebook view: shortcuts, menus, snapshot.
5. Smaller fixes and tests (CI is done).
6. Installer and signing.
