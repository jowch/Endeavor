# Endeavor on Windows: what a port would take

_Estimate as of 2026-09-28. Nothing has been built or run on Windows. This
comes from reading the code, the dependencies' sources, and
[linux.md](linux.md), which records how the Linux port went._

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

## What should carry over

- **GPUI** has a Windows backend: `gpui-pre-platform` 0.3.6 depends on
  `gpui-pre-windows` on Windows. Its source wasn't in the local registry, so
  its maturity is unchecked (IME, the folder picker, HiDPI).
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
- The lock becomes `std::fs::File::try_lock`, which works on every OS.
- The handover signal becomes a named event, or a "release" call on a
  loopback port.
- Smaller items: `/dev/urandom` becomes `getrandom`, `gethostname` becomes
  `COMPUTERNAME`, and there are `arg0` and file modes. `stdout_mux`
  (`File::from_raw_fd(1)`) and the `poll`/`recv(MSG_PEEK)` calls in `http.rs`
  need portable versions.

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
  - Dark mode: `set_theme`.
  - Reveal: `explorer /select,<path>`.
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
  with `dup2`, which needs a Windows version. Check that the `hook-pretool`
  output still reaches Claude Code.
- **Network-change watch (S).** Only macOS and Linux versions exist
  (`src/network.rs`). Windows: `NotifyIpInterfaceChange` or
  `INetworkListManager`.
- **Time zone (S).** `localtime_r` and `tm_gmtoff` (`src/when.rs:61`) don't
  exist on Windows. Use `GetTimeZoneInformation` or chrono.
- **Server install tarball (S).** Executable bits come from
  `PermissionsExt::mode()`. Windows has none, so set them by file name
  (`src/remote.rs`).
- **Title bar (S–M).** The header leaves room for the macOS window buttons.
  Windows needs its own caption buttons, drawn by GPUI or native.
- **Wording (S).** "This Mac" (about 60 places in `src/`), "Reveal in Finder", and
  `HostId::ThisMac`. This is shared with Linux; see
  [linux.md](linux.md#remaining-work-ranked).
- **Cargo (S).** `libc` is used unconditionally, and the `std::os::unix`
  imports stop the build outright. Add the `windows` crate.

### Claude Code on Windows (S–M, uncertain)

Claude Code on Windows has needed Git Bash. The plugin's hook command
(`plugin/hooks/hooks.json`: `"$ENDEAVOR_BIN" hook-pretool`) assumes a Unix
shell, and a Windows exe path with backslashes needs testing. Setup may need
to find Git for Windows or set `CLAUDE_CODE_GIT_BASH_PATH`.

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

### Tests (M, 3–5 days)

Many tests use `sh -c`, `kill -9`, `pkill`, `tar`, `shasum` or a fake Julia
written as a `#!/bin/sh` script (`crates/endeavor-remote/tests/common/mod.rs`),
and `wire`'s relay tests use `UnixStream::pair`. Replace the fake Julia with a
small Rust test binary, gate the signal tests with `#[cfg(unix)]`, and add a
Windows CI runner.

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
4. **GPUI's Windows backend maturity** is unchecked.
5. **Claude Code's shell requirements** on Windows.

## Suggested order

1. Make it build: gate the server-only modules with `#[cfg(unix)]` and add
   Windows stubs to `src/platform.rs`, as the Linux port did.
2. Run a local notebook: process control, downloads, paths.
3. Test ssh askpass on Windows 11. The result sets the size of the ssh work.
4. Notebook view: shortcuts, menus, snapshot.
5. Smaller fixes, tests and CI.
6. Installer and signing.
