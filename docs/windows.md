# Endeavor on Windows: what a port would take

_Estimate as of 2026-09-28; status updated 2026-10-10. The app runs on
Windows 10: the CI installer installed it, a notebook was made and run in a
real Claude session, and reinstall and uninstall worked (October 2026, one
machine that already had Julia, Node, a Claude sign-in and Git for Windows;
the build predates the switch to Julia through juliaup).
Not run yet: a first Claude sign-in on Windows, a browser download through
SmartScreen, Windows 11, and servers, which the Windows build can't reach
because it carries no server helpers. Parts of this page below still
describe the port before those runs. User steps are in the guide's
[Install Endeavor](guide/install.md#on-windows)._

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

On Windows the app should open, install Node and the agent, get Julia
through juliaup on first run, start the local runtime and run a notebook. None of that has been tried
yet. Servers go through EndeavorMCP's ssh client (`client::Session`), whose
password and two-factor prompts reach the app over loopback TCP, as on the
Mac; the app hasn't tried a server from Windows yet. These refuse with a plain
error rather than half-work:

| Where | What it says | What it needs |
|---|---|---|
| `crates/endeavor-mcp/src/core.rs`, `main` | "Couldn't keep Julia's processes together with this one (Job Object): …" | Nothing, if Windows 8 or later: refuses rather than start a Julia whose workers could outlive it. |
| `crates/endeavor-mcp/src/lib.rs`, `Runtime::kill` | logs "the runtime (pid …) is gone or isn't the one recorded; not stopping it" | Nothing: it won't end a process whose start time doesn't match the record. |

### How process control works on Windows

- The helper starts the core with `CREATE_NO_WINDOW |
  CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB` (`lib.rs`, `start`).
  `CREATE_NO_WINDOW` gives a console-program core a console without a
  window. In the app the core is the app's own GUI program, which gets no
  console at all, so the core also starts Julia with `CREATE_NO_WINDOW`
  (`core.rs`), and Pluto's workers share Julia's hidden console.
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
- Each agent's adapter runs under the app's own program, as `endeavor.exe
  --agent-job <app pid> <program> <args…>` (`agent_job.rs`). That process
  puts itself in a new Job Object with `KILL_ON_JOB_CLOSE`, then starts the
  adapter inside it, with the app's pipes as its stdin and stdout. It ends
  when the adapter ends, when the agent connection kills it, or when the app
  is gone, and the job then ends everything the adapter started, such as the
  Claude Code process Claude's adapter runs. On Unix the ACP
  library does this with a process group; on Windows it only ends the
  process it started. The job allows breakaway, as a process group lets a
  process leave.

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

- `src/overlay_windows.rs`: menus, tips, Settings and dialogs over the
  notebook get a hole cut in the web view's window with `SetWindowRgn`, and
  the window is disabled while a menu or popover is open, so a click on the
  notebook reaches GPUI and closes it. Not yet tried on a Windows machine.
  As on Linux, the web view isn't dimmed behind Settings.
- `src/platform.rs`, `webcontent`: a crashed WebView2 process isn't noticed
  (`ProcessFailed`), find in the notebook always says "Not found", and
  `has_keyboard` is always false. `url` and `give_keyboard` (wry's `focus`)
  are real.
- `src/platform.rs`, `web_view_hooks` and `init`: a click in GPUI gives
  GPUI's window the keyboard, but app shortcuts still don't reach GPUI while
  the notebook has the keyboard. Needs WebView2's `AcceleratorKeyPressed`.
- `src/platform.rs`, `reduces_motion`: always false. Needs
  `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`.
- `src/platform.rs`, `set_open_panel_message`, `snapshot`, `dialogs`: the
  same no-ops as on Linux.
- `src/network.rs`, `monitor`: says the network is up once and never again.
  Needs `NotifyIpInterfaceChange`.

Ported for real: the app data folder (`%LOCALAPPDATA%\Endeavor`,
`src/install.rs`), the home folder for `~/.ssh` and Downloads, the depot list
(`;`), Julia's and Node's Windows downloads and layout (`bin\julia.exe`,
`node.exe`, npm at `node_modules\npm\bin\npm-cli.js`), Repair runtime's stop
(`stop_group`), the helper's own process group, the folder browser's paths
(`wire::files::real_path`), the time zone offset (`src/when.rs`,
`src/trouble.rs`, through chrono), Reveal (`explorer /select,`), the
notebook's light and dark theme (WebView2's `set_theme`), Bring All to Front,
and the server tarball's executable bits (EndeavorMCP's bootstrap: nothing in
`runtime/` is executable, and the helper is marked by name).

### CI

`.github/workflows/windows.yml` runs on every push and pull request, on
`windows-latest`: `cargo build --locked --workspace --all-targets`, then
`cargo test --locked --workspace --no-fail-fast`. It turns off git's CRLF
conversion before checkout. It passes on `main`.

On `main` and on pull requests, a second job (`package`) makes a release
build and keeps it as a workflow artifact for 30 days (see [Try a build from
CI](#try-a-build-from-ci)). It builds with the MSVC toolchain and links the C
runtime in (`+crt-static`), and WebView2's loader is linked in on MSVC too, so
the exe needs no DLL that Windows doesn't ship. The job checks that with
`dumpbin /dependents` and fails if it finds one. A release build is needed
because a debug build compiles its shaders at launch from the crate's source
folder, so it runs only on the machine that built it.

`.github/workflows/juliaup.yml` runs the app's first-run juliaup install for
real on runners that have no juliaup: the ignored test
`installs_juliaup_and_julia_where_there_is_none` installs juliaup, adds Julia
1.12.6 and checks that channel's `julia.exe`. Windows Server 2025 has winget,
so it takes the Microsoft Store route; Windows Server 2022 has none, so it
falls back to juliaup's App Installer file. On the first runs (October 2026)
the Store route took 1 min, the file 27 s, and `juliaup add` about 20 s. It
runs when `src/juliaup.rs`, `src/runtime.rs`, `Cargo.lock`, the toolchain or
the workflow changes, weekly, and by hand. Windows Server isn't a Windows 10 or
11 desktop. The 2022 fallback starts because winget is missing; on a desktop
whose Store is blocked, winget is there and fails its own way, so that case is
still untried.

Compiled out on Windows with `#[cfg(unix)]`, because they need `sh`, signals,
`tar`, symlinks or Unix sockets:

- `crates/endeavor-mcp/tests/connect.rs`, `core.rs` and `e2e_julia.rs`
  (whole files: the stand-in Julia is a `#!/bin/sh` script).
- `tests/helper_mode.rs` (whole file).
- `crates/wire/src/relay.rs`: all its tests (`UnixStream::pair`).
- `crates/wire/src/files.rs`: `uploads_stay_inside_the_session_folder`.
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
  lines). It draws with DirectX, and it handles IME
  input and the native file and folder picker. Only two functions are left
  unimplemented, both macOS ideas that Endeavor doesn't call
  (`hide_other_apps`, `unhide_other_apps`). Zed's Windows support is new, so
  this backend has had much less use than the macOS one. Expect bugs that
  Zed hasn't met yet, particularly around child windows such as the web view.
  One already met: GPUI's DirectComposition target is topmost, so it covers
  the web view's child window and the notebook pane stays blank. The app
  turns DirectComposition off (`GPUI_DISABLE_DIRECT_COMPOSITION`, set only
  while GPUI starts, in `platform::application`), so GPUI draws into a plain
  window swap chain instead. That loses only per-pixel window transparency,
  which the app doesn't use.
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

- **Askpass transport. Done.** The password and 2FA prompts reach the app over
  loopback TCP with a per-connect token in an environment variable
  (EndeavorMCP's `client::Asker`, `wire::askpass`), on every platform.
- **Connection sharing. Done.** Windows OpenSSH has no ControlMaster, and a
  user's `~/.ssh/config` might turn it on, so EndeavorMCP's ssh command passes
  `-o ControlMaster=no -o ControlPath=none` there.
- **Open question:** whether the ssh in Windows 10 and 11 honours
  `SSH_ASKPASS_REQUIRE=force`, including for Duo prompts (the same question
  as in remote-sessions.md). If it doesn't, the fallbacks are running ssh
  under a hidden console (ConPTY) or an ssh library in Rust such as `russh`.
  Either adds about two weeks.

### Paths (written; server half untried on Windows)

Done for a local notebook: home and app data, the depot list's separator,
canonical paths without `\\?\`, and notebook paths as Julia's Windows rules
give them (see "Ported for real" above). Left: PATH joins in code that runs
only with servers.

- **Server paths.** A server's folders and notebooks are plain strings with
  `/` rules, in the app (`Place::path`, the folder browser, uploads) and in
  EndeavorMCP's wire messages; `wire::server_path` and `HostId`'s path
  methods take them apart. Only This Mac's paths use `PathBuf`. Untried:
  the folder browser and an upload from Windows to a Linux server.
- **Upload names.** A Windows machine refuses names it can't hold (`\`,
  `:`, device names such as `CON`, a trailing dot or space) in
  `wire::files::place` and `write`. Linux and macOS servers still take them.

### First-run setup and downloads (S–M, 2–3 days; written, untried)

- Pinned, the same versions as macOS and Linux:

  | Download | SHA-256 | Size |
  |---|---|---|
  | `node-v24.21.0-win-x64.zip` | `158f7685b44de51f6c0df1d153526cbcd3e1bc739a8dfc607721cef75de9e541` | 37,618,919 |
  | `node-v24.21.0-win-arm64.zip` | `8779b1bde1d39f8d420e3b57aa657b39891af434d3de44a919044cec06785921` | 33,679,608 |

  The checksums come from the published lists,
  `https://nodejs.org/dist/v24.21.0/SHASUMS256.txt` (their macOS entries
  match the existing pins), and the sizes from the server's
  `Content-Length`. Each zip's top folder and layout was read from its
  central directory: `node-v24.21.0-win-x64\node.exe` and
  `…\node_modules\npm\bin\npm-cli.js`.
- **Julia comes from juliaup** (`src/juliaup.rs`), as for the plugin, not
  from a download of the app's own. The app finds `juliaup.exe` (the PATH,
  the Store app's alias folder, `~\.juliaup\bin`); with none, it installs
  juliaup for this Windows account, from the Microsoft Store with `winget`,
  else from juliaup's App Installer file. Neither needs admin. It then adds
  the channel for the pinned Julia (`juliaup add 1.12.6`) if juliaup lacks it,
  and runs that channel's own `julia.exe`, which `juliaup api getconfig1`
  lists. It never runs juliaup's launcher: on a fresh juliaup the launcher
  first downloads the latest Julia, and a Julia started through the Store
  app's alias outlived the runtime
  (EndeavorMCP #55). A juliaup the person already has keeps its default and
  other channels. A `julia-1.12.6` folder an older Endeavor downloaded is used
  while juliaup can't be set up, and removed once a runtime starts with
  juliaup's Julia. Each install route gets 10 minutes and `juliaup add` 45,
  then the next route or an error. The App Installer file gets a second try
  after a failure other than the time limit: in CI it once failed and then
  worked, cause unknown. Installing juliaup on a computer without
  it runs in CI on Windows Server, both routes (see [CI](#ci)).
- Windows 10 and later include `curl.exe` and `tar.exe` (bsdtar), and that
  `tar` reads .zip. The app runs `%SystemRoot%\System32\tar.exe` by path,
  since a `tar` earlier on PATH may be Git's GNU tar, which reads no zips. The
  app hashes downloads in Rust with `sha2` on every platform.
- Executable layout differs: `node.exe` at the top of the Node folder, and npm at `node_modules\npm\bin\npm-cli.js`
  (`src/install.rs`, `src/agent.rs`, `src/runtime.rs`).
- The server's own Julia download (`crates/endeavor-mcp/src/julia.rs`)
  stays Unix-only: a Windows server is out of scope.

### Notebook view (M–L, about 1–1.5 weeks)

- **Menus over the notebook (M).** Written, untried: WebView2 is a child
  window, so it draws above GPUI's content, as on Linux. `overlay::set_hole`
  cuts a hole in the web view's window with `SetWindowRgn`; with
  DirectComposition off, GPUI's own drawing should show through it. If it
  doesn't, the fallback is to hide the web view while a menu is open.
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

- **Console windows: done.** The app is a GUI program
  (`#![windows_subsystem = "windows"]`) and starts its console children
  (Julia's `--version`, curl, tar, Node, npm, the sign-in CLI) with
  `CREATE_NO_WINDOW`; the core does the same for Julia. `logs.rs` makes the
  log file the app's stdout and stderr (`SetStdHandle`), so on Windows the
  app's output goes there even when it is started from a terminal. Run by
  hand in cmd or PowerShell (for example with `--helper` while debugging),
  `endeavor.exe` returns to the prompt at once and prints nothing there; pipe
  its output (`2>&1 | Out-Host`) to see it.
- **Network-change watch (S).** Only macOS and Linux versions exist
  (`src/network.rs`). Windows: `NotifyIpInterfaceChange` or
  `INetworkListManager`.
- **Title bar: native for now.** Windows keeps its own title bar, with the
  window's buttons, above the column headers. Drawing the buttons in the
  sidebar's header instead (GPUI's `WindowControlArea`) would save its height.
- **Wording (S).** "This Mac" (about 60 places in `src/`), "Reveal in Finder", and
  `HostId::ThisMac`. This is shared with Linux; see
  [linux.md](linux.md#remaining-work-ranked).
- **Cargo (S).** The stubs left will need more of `windows-sys`' features.

### Claude Code on Windows (S–M, uncertain)

Claude Code on Windows has needed Git Bash. Setup may need to find Git for
Windows or set `CLAUDE_CODE_GIT_BASH_PATH`.

### Packaging (M–L, about 1 week)

- Done: CI builds a per-user installer with Inno Setup
  (`scripts/installer.iss`, see [Install](#install)). It uses the layout
  `install::resources()` already looks for (`bin\endeavor.exe`, `Resources\`
  beside `bin`) and runs Microsoft's Evergreen WebView2 bootstrapper when the
  runtime is missing. Untried on a real machine.
- The exe has no icon resource yet (`build.rs`); the Start menu entry uses
  `assets/icon/endeavor.ico`.
- Sign the app and the installer with Authenticode, or SmartScreen warns
  every user.
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

## Install

The installer needs no administrator. It puts Endeavor in
`%LOCALAPPDATA%\Programs\Endeavor`, adds it to the Start menu and to
Settings → Apps for uninstalling, and installs Microsoft's WebView2 Runtime
for this user if it's missing.

1. Open the repository's Actions tab, then the latest **Windows** run on
   `main`, or the one on a pull request. Under Artifacts, download
   `Endeavor-windows-x86_64-setup-<build>` and unzip it.
2. Run `Endeavor-setup-<build>.exe`. It isn't signed, so SmartScreen may say
   "Windows protected your PC"; choose More info, then Run anyway.
3. Click Install, then Finish. Endeavor opens and sets itself up.

Installing a newer build over an older one asks you to quit Endeavor if it's
open, then stops a runtime kept running after Endeavor quit (`endeavor.exe
--stop-runtime`), since that runtime runs from the installed `endeavor.exe`.
Its notebooks are already saved. Uninstalling does the same. Uninstalling leaves
`%LOCALAPPDATA%\Endeavor` (Julia, the agents, sessions and settings).

## Try a build from CI

No Rust or Visual Studio is needed for this.

1. Open the repository's Actions tab, then the latest **Windows** run on
   `main`, or the one on a pull request. Under Artifacts, download
   `Endeavor-windows-x86_64-<build>`. You need to be signed in to GitHub.
   `<build>` is the commit, the same number About Endeavor shows (on a pull
   request, the commit GitHub made by merging it into `main`).
2. Extract all of it somewhere you can write to, such as Downloads, not
   Program Files. Running the exe from inside the zip doesn't work: the
   window only says that Endeavor's files are missing. WebView2 keeps its
   data in `endeavor.exe.WebView2` next to the exe.
3. Run `Endeavor\bin\endeavor.exe`. Keep `bin` and `Resources` side by side:
   the app finds its agents' pinned versions in `Resources`. The exe isn't
   signed, so SmartScreen may say "Windows protected your PC"; choose More
   info, then Run anyway.

What the machine needs:

- Windows 10 or 11, x64. Windows on ARM is untested (see
  [Limits](#limits-we-cant-fix-from-endeavor)).
- The WebView2 Runtime. Windows 11 has it, and so does Windows 10 once
  Microsoft Edge's updates have installed it. If it's missing, install
  Microsoft's Evergreen WebView2 Runtime, or use the [installer](#install),
  which does.
- An internet connection on first run: the app downloads Node and the agent
  into `%LOCALAPPDATA%\Endeavor`, and gets Julia through juliaup (installing
  juliaup for this account if it isn't there).

The app opens no console window, and neither do the runtime, Julia or
Pluto's workers (see Console windows under
[Smaller app fixes](#smaller-app-fixes-about-1-week-in-total)).

## Try first on a real Windows machine

In this order, on Windows 11 x64 with Visual Studio's C++ tools (for
`rc.exe` and `fxc.exe`). Each step needs the ones before it. CI already
passes on `windows-latest`, including the Job Object test, which runs the
test binary as its own stand-in core.

1. **Build and open.** `cargo run`. The window should open with the setup
   screen.
2. **First-run downloads.** Setup downloads Node's zip, checks it, unpacks it
   with `System32\tar.exe` and runs `npm ci` with `node.exe`; then finds or
   installs juliaup and adds Julia 1.12.6 to it. Watch for antivirus holding
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
   - While Claude is connected, the `endeavor.exe --agent-job` process, the
     adapter's `node.exe` and the Claude Code `node.exe` it starts share one
     job. Quit the app, or end the `--agent-job` process in Task Manager (the
     app restarts Claude): all of them go.
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
3. **Menus over the notebook.** The `SetWindowRgn` hole is untested with how
   GPUI draws (a plain window swap chain, since DirectComposition is off).
   Hiding the web view is the fallback.
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
