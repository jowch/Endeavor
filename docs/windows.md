# Endeavor on Windows: what a port would take

_Estimate as of 2026-09-28; status updated 2026-10-03. Step 1 of the
suggested order is done: the workspace builds for Windows, and a Windows CI
workflow builds it and runs its tests, which pass. Step 2 (run a local
notebook) is written but untried: process control, downloads and paths have
Windows code, checked only by type-checking on a Mac and by tests that CI runs
on `windows-latest`. Nothing has run on a real Windows machine yet. The rest
comes from reading the code, the dependencies' sources, and
[linux.md](linux.md), which records how the Linux port went._

**Summary:** a Windows port is feasible, and no single item blocks it. It is
about **6–8 weeks** of work for one person to reach a usable app: roughly 3–4
weeks in the app and 2–3 weeks in `endeavor-mcp` and `wire`. It costs more
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
- Small crates added for Windows only, all already in the tree: `chrono` in
  the app (the time zone), `getrandom` in `endeavor-mcp` (the bridge
  token), `windows-sys` 0.61 in both (process control), `sha2` in the app
  (checking downloads) and `dunce` in `wire` (paths without `\\?\`).

Step 2 hasn't run on a real machine, so expect some days of fixes once it
does (see [Try first on a real Windows
machine](#try-first-on-a-real-windows-machine)).

### What runs on Windows now

On Windows the app should open, install Node, the agent and Julia on first
run, start the local runtime and run a notebook. None of that has been tried
yet. It can't reach a server. These refuse with a plain error rather than
half-work:

| Where | What it says | What it needs |
|---|---|---|
| `src/remote.rs`, `Askpass` | "Endeavor can't connect to servers from Windows yet." | The askpass transport (loopback TCP or a named pipe), and an answer on `SSH_ASKPASS_REQUIRE`. |
| `crates/endeavor-mcp/src/askpass.rs`, `ask_app` | ssh prompts aren't supported on Windows yet | The same transport, on the helper's side. |
| `crates/endeavor-mcp/src/core.rs`, `main` | "Couldn't keep Julia's processes together with this one (Job Object): …" | Nothing, if Windows 8 or later: refuses rather than start a Julia whose workers could outlive it. |
| `crates/endeavor-mcp/src/lib.rs`, `Runtime::kill` | logs "the runtime (pid …) is gone or isn't the one recorded; not stopping it" | Nothing: it won't end a process whose start time doesn't match the record. |

### How process control works on Windows

- The helper starts the core with `CREATE_NO_WINDOW |
  CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB` (`lib.rs`, `start`).
  `CREATE_NO_WINDOW` gives the core a console without a window, which Julia
  and Pluto's workers inherit, so none of them opens a console window.
  Breaking away from a job the app runs in lets the runtime outlive the app,
  as on Unix. A job that forbids that refuses the start; the helper then
  starts the core inside it and logs that the runtime ends with the app.
- Before starting Julia, the core puts itself in a new Job Object with
  `KILL_ON_JOB_CLOSE` (`winproc.rs`, `job_ending_with_this_process`). Only the
  core holds the job's handle, so when the core ends, however it ends,
  Windows ends Julia and every worker in the job. The job forbids breakaway,
  and nested jobs (Windows 8 and later) keep libuv's own job for Julia's
  children inside it. This replaces the process group, `PR_SET_PDEATHSIG`
  and `stop_workers`.
- The core writes its start time (`GetProcessTimes`, FILETIME units) into
  `runtime.json` as `started`. A pid counts as the runtime only while the
  process behind it started at that time (`winproc::Process::open`), so a pid
  Windows has handed to another process is never attached to or ended.
- A graceful stop goes over the bridge (`endeavor/shutdown`), as on Unix. A
  hard stop is `TerminateProcess` on the core, which ends the job. Repair
  runtime does the same (`end_recorded_runtime`).
- The app starts the helper with `CREATE_NEW_PROCESS_GROUP`, so a Ctrl+C in
  the app's terminal doesn't reach it.

### Stubs left

Each stub is marked "Not ported" in the code.

**`endeavor-mcp` (the app's local runtime runs it too):**

- `lib.rs`, `ask_to_hand_over`, `watch_replace_signal`, `block_sigusr1`: no
  handover between clients, so a second client waits for the lock (about 30 s)
  and fails. Needs a named event, or a "release" call on a loopback port.
  Locally this matters only if a helper is still running when the app starts
  again; a helper exits when the app that started it goes away.
- `http.rs`, `wait_readable`: doesn't watch the client, so a client that hangs
  up during an event stream is noticed only when a write to it fails. Needs
  `WSAPoll`.
- `mcp.rs`, `closed`: always false, so a tool call goes on after its client
  hangs up. Needs a non-blocking peek with winsock's `recv`.

These stay Unix-only on purpose, because they run only on a Linux or macOS
server: `host_tools.rs`'s `run_shell` (refuses on Windows), and in `slurm.rs`
the job script's `exec` and the SIGTERM to `srun`.

Ported for real: starting, watching and stopping the runtime (above),
`pid_alive` with the start time, the bridge token's random bytes
(`getrandom`), the host name (`COMPUTERNAME`), stdin and stdout as files
(their handles), file times (`Metadata::modified`), the home folder
(`std::env::home_dir`, also for a Julia path's `~`), the state folder's lock
(`File::try_lock`), and notebook paths (`notebooks.rs`: `canonical_path`,
`absolute_path` and the folder checks follow Julia's Windows rules, where
`expanduser` leaves paths alone and `realpath` gives `C:\…`). Files that are
0600 on Unix get no special permissions on Windows: the state folder lives
under the user's own `%LOCALAPPDATA%`.

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
  `SetStdHandle`. `logs::path` is `%LOCALAPPDATA%\Endeavor\Logs\endeavor.log`,
  but nothing writes there yet.
- `src/remote.rs`, `kill_group`: does nothing. No ssh runs, since `Askpass`
  refuses.
- `src/signin.rs`, `Login::cancel`: doesn't stop the sign-in's CLI. Needs a
  Job Object (`endeavor_mcp`'s `winproc` has the pieces).

Ported for real: the app data folder (`%LOCALAPPDATA%\Endeavor`,
`src/install.rs`), the home folder for `~/.ssh` and Downloads, the depot list
(`;`), Julia's and Node's Windows downloads and layout (`bin\julia.exe`,
`node.exe`, npm at `node_modules\npm\bin\npm-cli.js`), Repair runtime's stop
(`stop_group`), the helper's own process group, the folder browser's paths
(`wire::files::real_path`), the time zone offset (`src/when.rs`,
`src/trouble.rs`, through chrono), Reveal (`explorer /select,`), the
notebook's light and dark theme (WebView2's `set_theme`), Bring All to Front,
and the server tarball's executable bits (`src/remote.rs`: nothing in
`runtime/` is executable, and the helper is marked by name).

### CI

`.github/workflows/windows.yml` runs on every push and pull request, on
`windows-latest`: `cargo build --locked --workspace --all-targets`, then
`cargo test --locked --workspace --no-fail-fast`. It turns off git's CRLF
conversion before checkout. It passes on `main`.

Compiled out on Windows with `#[cfg(unix)]`, because they need `sh`, signals,
`tar`, symlinks or Unix sockets:

- `crates/endeavor-mcp/tests/connect.rs`, `core.rs` and `e2e_julia.rs`
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
- `crates/endeavor-mcp/src/notebooks/tests.rs`: the `file_info` check in
  `the_apps_notebook_actions_restart_move_file_info_and_new_notebook`.

These run on Windows too, with paths and folders that follow Windows rules:

- `crates/endeavor-mcp/src/notebooks/tests.rs`, notebook paths (their
  paths now use the platform's separator, and the Unix-only checks, such as
  `~user` and `/tmp`, are gated): `opening_and_making_notebooks`,
  `one_notebook_per_session`,
  `list_notebooks_says_which_notebook_is_this_sessions`,
  `the_apps_notebook_actions_restart_move_file_info_and_new_notebook`,
  `stop_notebook_shuts_it_down_and_says_if_it_was_in_safe_preview`,
  `idle_notebooks_stop_but_running_kept_alive_and_recently_used_ones_dont`,
  `a_notebooks_state_goes_when_it_shuts_down_however_it_shuts_down`,
  `parses_ids_and_paths_as_julia_did`.
- `crates/endeavor-mcp/src/julia.rs`, the home folder:
  `versions_and_home_paths`.
- `crates/wire/src/files.rs`, the home folder: `home_relative_paths` (also
  checks `~\` on Windows).
- `src/hosts.rs`, the app data folder:
  `server_sessions_share_one_agent_folder_per_server`.
- `src/install.rs`, downloads: `installs_a_verified_tarball_and_rejects_a_bad_one`
  (on Windows it packs a zip with the system's `tar.exe` and fetches it with
  `curl` from a `file:///C:/…` URL).

Tests that run only on Windows:

- `crates/endeavor-mcp/src/winproc.rs`,
  `ending_the_jobs_first_process_ends_everything_it_started`: the test binary
  runs itself as a stand-in core that puts itself in the Job Object and
  starts a worker, which starts `ping`. Ending the core must end the worker
  and `ping` within 10 s.
- `winproc.rs`, `a_recorded_pid_counts_only_while_its_start_time_matches`:
  a pid opens only with its own start time, not a later one or none, and an
  exited child isn't alive.
- `src/install.rs`, `app_data_and_logs_are_in_local_app_data`.

On every OS, `src/runtime.rs`'s
`the_depot_list_stacks_the_default_depots_behind_ours` checks the depot list.

The tests marked `#[ignore]` on every OS (the live and real-Julia ones) stay
ignored. Nothing in CI starts the real core on Windows: the helper and core
tests still need the fake Julia rewritten in Rust (see Tests below).

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

### Process control (L, about 1–1.5 weeks; written, untried)

This is the largest item. Starting, watching and stopping the runtime is
written (see [How process control works on
Windows](#how-process-control-works-on-windows)). Left:

- The handover between clients: the signal (SIGUSR1 on Unix) becomes a named
  event, or a "release" call on a loopback port.
- The `poll`/`recv(MSG_PEEK)` calls in `http.rs` and `mcp.rs` need Windows
  versions (stubs, see above).

### ssh from Windows (M if askpass works, L if not)

- **Askpass transport (S).** The password and 2FA prompts reach the app over a
  Unix socket (`src/remote.rs:619`, `crates/endeavor-mcp/src/askpass.rs:34`).
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

### Paths (M, about 1 week; the local half done)

Done for a local notebook: home and app data, the depot list's separator,
canonical paths without `\\?\`, and notebook paths as Julia's Windows rules
give them (see "Ported for real" above). Left: server paths, PATH joins in
code that runs only with servers, and upload names.

- **Server paths vs local paths.** Wire messages carry server paths as
  `PathBuf` (`wire/src/files.rs`, `wire/src/notebooks.rs`). On a Windows
  client, `join` would put `\` into Linux paths. Make server paths a `String`
  or a `RemotePath` type, and update the app's call sites.
- **Upload names** (`files.rs`) must also reject `\`, `:` and reserved names
  such as `CON`.

### First-run setup and downloads (S–M, 2–3 days; written, untried)

- Pinned, the same versions as macOS and Linux:

  | Download | SHA-256 | Size |
  |---|---|---|
  | `julia-1.12.6-win64.zip` | `a63d991976e6893f508c512e3dc7bca1836c1a1f6ad1f3e4aedec159b6733e89` | 275,091,967 |
  | `node-v24.21.0-win-x64.zip` | `158f7685b44de51f6c0df1d153526cbcd3e1bc739a8dfc607721cef75de9e541` | 37,618,919 |
  | `node-v24.21.0-win-arm64.zip` | `8779b1bde1d39f8d420e3b57aa657b39891af434d3de44a919044cec06785921` | 33,679,608 |

  The checksums come from the published lists,
  `https://julialang-s3.julialang.org/bin/checksums/julia-1.12.6.sha256` and
  `https://nodejs.org/dist/v24.21.0/SHASUMS256.txt` (their macOS entries
  match the existing pins), and the sizes from the servers'
  `Content-Length`. Each zip's top folder and layout was read from its
  central directory: `julia-1.12.6\bin\julia.exe`,
  `node-v24.21.0-win-x64\node.exe` and
  `…\node_modules\npm\bin\npm-cli.js`. There is no official Windows ARM64
  build of Julia 1.12, so Windows on ARM gets the x64 one, untested under
  emulation.
- Windows 10 and later include `curl.exe` and `tar.exe` (bsdtar), and that
  `tar` reads .zip. The app runs `%SystemRoot%\System32\tar.exe` by path,
  since a `tar` earlier on PATH may be Git's GNU tar, which reads no zips. There
  is no `shasum`, so Windows hashes in Rust with `sha2`.
- Executable layout differs: `bin\julia.exe`, `node.exe` at the top of the
  Node folder, and npm at `node_modules\npm\bin\npm-cli.js`
  (`src/install.rs`, `src/agent.rs`, `src/runtime.rs`).
- The server's own Julia download (`crates/endeavor-mcp/src/julia.rs`)
  stays Unix-only: a Windows server is out of scope.

### Notebook view (M–L, about 1–1.5 weeks)

- **Menus over the notebook (M).** WebView2 is a child window, so it draws
  above GPUI's content, as on Linux. Port `overlay::set_hole` by cutting a
  hole in the web view window with `SetWindowRgn`. It's unknown whether
  GPUI's DirectComposition surface shows through that hole. The fallback is
  to hide the web view while a menu is open.
- **App shortcuts while the notebook has focus (M).** Use WebView2's
  `AcceleratorKeyPressed` to send Ctrl+B, Ctrl+Q, Ctrl+, Ctrl+Shift+E (Point) and the zoom keys to
  GPUI actions (the macOS version is `webkeys.rs`). Turn off browser
  accelerator keys so F5, Ctrl+P and Ctrl+F don't act on Pluto.
- **Snapshot (S–M).** Use `CapturePreview` and crop, or the DevTools
  `Page.captureScreenshot` with a clip.
- **Small items (S).**
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
- **Title bar (S–M).** The header leaves room for the macOS window buttons.
  Windows needs its own caption buttons, drawn by GPUI or native.
- **Wording (S).** "This Mac" (about 60 places in `src/`), "Reveal in Finder", and
  `HostId::ThisMac`. This is shared with Linux; see
  [linux.md](linux.md#remaining-work-ranked).
- **Cargo (S).** The stubs left will need more of `windows-sys`' features.

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
written as a `#!/bin/sh` script (`crates/endeavor-mcp/tests/common/mod.rs`),
and `wire`'s relay tests use `UnixStream::pair`. These are gated with
`#[cfg(unix)]`, and a Windows CI workflow runs the rest (see [CI](#ci)).
Left: replace the fake Julia with a small Rust test binary so the
helper and core tests run on Windows, and give the relay tests a loopback TCP
pair.

## Try first on a real Windows machine

In this order, on Windows 11 x64 with Visual Studio's C++ tools (for
`rc.exe` and `fxc.exe`). Each step needs the ones before it. CI already
passes on `windows-latest`, including the Job Object test, which runs the
test binary as its own stand-in core.

1. **Build and open.** `cargo run`. The window should open with the setup
   screen.
2. **First-run downloads.** Setup downloads Node's zip, checks it, unpacks it
   with `System32\tar.exe` and runs `npm ci` with `node.exe`; then Julia's
   275 MB zip into `%LOCALAPPDATA%\Endeavor\julia-1.12.6`. Watch for paths
   over 260 characters while unpacking Julia, and for antivirus holding
   unpacked files so the rename from `….unpacking` fails.
3. **Start the runtime.** No console window should open, for the core, Julia
   or later Pluto's workers. `%LOCALAPPDATA%\Endeavor\runtime\runtime.json`
   should have `started`, and `runtime.log` should fill. If the app's log says
   "the runtime can't leave the job this helper runs in", note which terminal
   or launcher started the app.
4. **Run a notebook.** Open one, run its cells, make a new one in a session's
   folder, rename it, and use a folder on another drive and one whose name has
   spaces and non-ASCII letters. The folder browser should show `C:\…`, never
   `\\?\C:\…`.
5. **The process tree.** In Process Explorer, the core (`endeavor.exe` run as
   the helper's `core`), `julia.exe` and each worker `julia.exe` should share
   one job (the Job tab). Then:
   - Quit the app with the runtime set to stop: all of them go.
   - End the core in Task Manager: Julia and its workers go within a second
     or two, and the app says Julia stopped.
   - End the app's process while the runtime is set to keep running: the
     runtime stays, and the next launch reattaches to it (pid and start time).
   - Repair runtime with a runtime running: all of it goes.
6. **Stop a running cell.** Stop a tight loop and a `sleep(60)`; see
   [Limits](#limits-we-cant-fix-from-endeavor).

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

1. Make it build, with a Windows CI workflow. **Done** (see
   [Status](#status-it-builds-with-stubs)).
2. Run a local notebook: process control, downloads, paths. **Written,
   untried**: the list above on a real machine.
3. Test ssh askpass on Windows 11. The result sets the size of the ssh work.
4. Notebook view: shortcuts, menus, snapshot.
5. Smaller fixes and tests (CI is done).
6. Installer and signing.
