//! Julia on Windows comes from juliaup, as the plugin's does (EndeavorMCP's
//! docs/endeavor-mcp.md), not from the app's own download. The app finds
//! juliaup, or installs it for this Windows account (no admin), adds the
//! channel for the pinned Julia, and runs the `julia.exe` that channel
//! installed. A juliaup the person already has only gains that channel: its
//! default and other channels stay as they were.

#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::{Command, Stdio};

/// juliaup's id in the Microsoft Store, as its README installs it.
#[cfg(windows)]
const STORE_ID: &str = "9NJNWW8PVKMN";
/// juliaup's App Installer file, for a computer whose Store is blocked.
#[cfg(windows)]
const APP_INSTALLER: &str = "https://install.julialang.org/Julia.appinstaller";

/// The pinned Julia's `julia.exe`, installing juliaup and adding the channel
/// `version` first if they're missing. `progress` hears each step.
#[cfg(windows)]
pub fn julia(version: &str, progress: &dyn Fn(String, Option<f32>)) -> Result<PathBuf, String> {
    let juliaup = match find_juliaup() {
        Some(juliaup) => juliaup,
        None => {
            progress("Installing juliaup, which installs Julia".into(), None);
            install_juliaup()?;
            find_juliaup().ok_or("juliaup was installed, but Endeavor can't find it. Quit Endeavor and open it again.")?
        }
    };
    // juliaup puts its julia.exe launcher beside juliaup.exe.
    let launcher = juliaup.with_file_name("julia.exe");
    if let Some(julia) = channel_julia(&launcher, version) {
        return Ok(julia);
    }
    progress(format!("Downloading Julia {version} with juliaup"), None);
    let added = run(Command::new(&juliaup).args(["add", version]));
    channel_julia(&launcher, version).ok_or_else(|| {
        let why = added.err().map(|e| format!(" ({e})")).unwrap_or_default();
        format!("juliaup couldn't install Julia {version}{why}. Check the internet connection, then try again.")
    })
}

/// juliaup.exe: on the PATH, else where the Store app's alias or juliaup's
/// own installer put it (the PATH Endeavor started with may predate it).
#[cfg(windows)]
fn find_juliaup() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let local = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Microsoft").join("WindowsApps"));
    let home = std::env::home_dir().map(|h| h.join(".juliaup").join("bin"));
    path.into_iter().chain(local).chain(home).map(|dir| dir.join("juliaup.exe")).find(|exe| exists(exe))
}

/// A Store app's alias is a reparse point that some checks can't follow, so
/// ask only whether something is there.
#[cfg(windows)]
fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Install juliaup for this account: from the Microsoft Store with winget,
/// else from its App Installer file. Neither needs admin.
#[cfg(windows)]
fn install_juliaup() -> Result<(), String> {
    let store = run(Command::new("winget").args(["install", "--id", STORE_ID, "--exact", "--source", "msstore", "--accept-package-agreements", "--accept-source-agreements"]));
    let Err(store) = store else { return Ok(()) };
    eprintln!("Installing juliaup from the Microsoft Store failed ({store}); trying its App Installer file.");
    let script = format!("Add-AppxPackage -AppInstallerFile '{APP_INSTALLER}'");
    run(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script])).map(drop).map_err(|file| {
        format!(
            "Couldn't install juliaup, which Endeavor uses to install Julia. From the Microsoft Store: {store}. From its installer file: {file}. \
             Install Julia from the Microsoft Store, or choose a julia.exe in Settings, then try again."
        )
    })
}

/// The `julia.exe` the channel `version` runs, if juliaup has it: the one in
/// that Julia's `Sys.BINDIR`, not juliaup's launcher. Ending the runtime
/// ends what it started, which a launcher started through the Store app's
/// alias escapes (EndeavorMCP #55).
#[cfg(windows)]
fn channel_julia(launcher: &Path, version: &str) -> Option<PathBuf> {
    let mut command = Command::new(launcher);
    command.arg(format!("+{version}")).args(["--startup-file=no", "--history-file=no", "-e", "print(Sys.BINDIR)"]);
    let printed = run(&mut command).ok()?;
    bindir_julia(&printed).filter(|julia| julia.is_file())
}

/// Run `command` without a window or input; its output, or why it failed.
#[cfg(windows)]
fn run(command: &mut Command) -> Result<String, String> {
    endeavor_mcp::client::no_window(command);
    let output = command.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    // Its last words, from stderr if it wrote any (winget writes to stdout).
    let last = |text: &str| text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_owned);
    Err(last(&*String::from_utf8_lossy(&output.stderr)).or_else(|| last(stdout.as_str())).unwrap_or_else(|| output.status.to_string()))
}

/// The `julia.exe` in the `Sys.BINDIR` Julia printed (its last line).
#[cfg_attr(not(windows), allow(dead_code))]
fn bindir_julia(printed: &str) -> Option<std::path::PathBuf> {
    let bindir = printed.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    Some(std::path::Path::new(bindir).join("julia.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_julia_exe_is_the_one_in_the_bindir_julia_printed() {
        let bindir = r"C:\Users\someone\.julia\juliaup\julia-1.12.6+0.x64.w64.mingw32\bin";
        let julia = std::path::Path::new(bindir).join("julia.exe");
        assert_eq!(bindir_julia(bindir), Some(julia.clone()));
        assert_eq!(bindir_julia(&format!("a message first\n{bindir}\r\n")), Some(julia));
        assert_eq!(bindir_julia("\n "), None);
    }
}
