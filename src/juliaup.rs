//! Julia on Windows comes from juliaup, as the plugin's does (EndeavorMCP's
//! docs/endeavor-mcp.md), not from the app's own download. The app finds
//! juliaup, or installs it for this Windows account (no admin), adds the
//! channel for the pinned Julia, and runs the `julia.exe` that channel
//! installed. A juliaup the person already has only gains that channel: its
//! default and other channels stay as they were.

use std::path::PathBuf;
#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use std::process::{Command, Stdio};
#[cfg(windows)]
use std::time::{Duration, Instant};

/// juliaup's id in the Microsoft Store, as its README installs it.
#[cfg(windows)]
const STORE_ID: &str = "9NJNWW8PVKMN";
/// juliaup's App Installer file, for a computer whose Store is blocked.
#[cfg(windows)]
const APP_INSTALLER: &str = "https://install.julialang.org/Julia.appinstaller";

/// How long each install route may take before the next is tried. A Store
/// install can sit queued behind other updates for good.
#[cfg(windows)]
const INSTALL_LIMIT: Duration = Duration::from_secs(10 * 60);
/// How long `juliaup add` may take: about 300 MB on a slow connection.
#[cfg(windows)]
const ADD_LIMIT: Duration = Duration::from_secs(45 * 60);
/// How long juliaup may take to list its channels.
#[cfg(windows)]
const LIST_LIMIT: Duration = Duration::from_secs(60);

/// The pinned Julia's `julia.exe`, installing juliaup and adding the channel
/// `version` first if they're missing. `progress` hears each step.
#[cfg(windows)]
pub fn julia(version: &str, progress: &dyn Fn(String, Option<f32>)) -> Result<PathBuf, String> {
    let juliaup = match find_juliaup() {
        Some(juliaup) => juliaup,
        None => {
            progress("Installing juliaup, which installs Julia".into(), None);
            install_juliaup()?;
            find_juliaup().ok_or(
                "juliaup was installed, but Endeavor can't find juliaup.exe. If Julia's app execution aliases are turned off in Windows Settings, turn them on; otherwise quit Endeavor and open it again.",
            )?
        }
    };
    // Asked of juliaup itself, never through its julia.exe launcher: on a
    // juliaup with no setup yet, the launcher first downloads the latest
    // Julia and makes it the default.
    if let Some(julia) = channel_julia(&juliaup, version) {
        return Ok(julia);
    }
    let what = format!("Downloading Julia {version} with juliaup (about 300 MB)");
    progress(what.clone(), None);
    let tick = |waited: Duration| progress(format!("{what}, {} so far", minutes(waited)), None);
    let added = run(Command::new(&juliaup).args(["add", version]), ADD_LIMIT, &tick);
    channel_julia(&juliaup, version).ok_or_else(|| {
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
    let quiet = |_: Duration| {};
    let store = run(
        Command::new("winget").args(["install", "--id", STORE_ID, "--exact", "--source", "msstore", "--accept-package-agreements", "--accept-source-agreements"]),
        INSTALL_LIMIT,
        &quiet,
    );
    let Err(store) = store else {
        eprintln!("Installed juliaup from the Microsoft Store.");
        return Ok(());
    };
    eprintln!("Installing juliaup from the Microsoft Store failed ({store}); trying its App Installer file.");
    let script = format!("Add-AppxPackage -AppInstallerFile '{APP_INSTALLER}'");
    match run(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]), INSTALL_LIMIT, &quiet) {
        Ok(_) => {
            eprintln!("Installed juliaup from its App Installer file.");
            Ok(())
        }
        Err(file) => Err(format!(
            "Couldn't install juliaup, which Endeavor uses to install Julia. From the Microsoft Store: {store}. From its installer file: {file}. \
             If this computer blocks app installs, install Julia from julialang.org/downloads and choose its julia.exe in Settings."
        )),
    }
}

/// The `julia.exe` juliaup's channel `version` runs, if it has that channel:
/// the real one, not juliaup's launcher. Ending the runtime ends what it
/// started, which a launcher started through the Store app's alias escapes
/// (EndeavorMCP #55).
#[cfg(windows)]
fn channel_julia(juliaup: &Path, version: &str) -> Option<PathBuf> {
    let listed = run(Command::new(juliaup).args(["api", "getconfig1"]), LIST_LIMIT, &|_| {}).ok()?;
    channel_file(&listed, version).filter(|julia| julia.is_file())
}

/// Run `command` without a window or input, for at most `limit`, telling
/// `tick` every 15 s how long it has run; its output, or why it failed.
#[cfg(windows)]
fn run(command: &mut Command, limit: Duration, tick: &dyn Fn(Duration)) -> Result<String, String> {
    use std::io::Read;
    endeavor_mcp::client::no_window(command);
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    // Read both pipes while it runs, so a full pipe can't stall it.
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut text);
            }
            String::from_utf8_lossy(&text).into_owned()
        })
    };
    let stdout = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let stderr = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = Instant::now();
    let mut told = Duration::ZERO;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        let waited = started.elapsed();
        if waited >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("it didn't finish within {}", minutes(limit)));
        }
        if waited >= told + Duration::from_secs(15) {
            told = waited;
            tick(waited);
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let (stdout, stderr) = (stdout.join().unwrap_or_default(), stderr.join().unwrap_or_default());
    if status.success() {
        return Ok(stdout);
    }
    // Its last words, from stderr if it wrote any (winget writes to stdout).
    let last = |text: &str| text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_owned);
    Err(last(stderr.as_str()).or_else(|| last(stdout.as_str())).unwrap_or_else(|| status.to_string()))
}

/// "4 min", or "30 s" under a minute.
#[cfg(windows)]
fn minutes(time: Duration) -> String {
    match time.as_secs() {
        s if s < 60 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

/// The `File` of the channel named `version` in `juliaup api getconfig1`'s
/// JSON (`DefaultChannel` and `OtherChannels`): that channel's real julia.exe.
#[cfg_attr(not(windows), allow(dead_code))]
fn channel_file(listed: &str, version: &str) -> Option<PathBuf> {
    let config: serde_json::Value = serde_json::from_str(listed.trim()).ok()?;
    let others = config["OtherChannels"].as_array().into_iter().flatten();
    std::iter::once(&config["DefaultChannel"]).chain(others).find(|c| c["Name"] == version).and_then(|c| c["File"].as_str()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_julia_exe_is_the_file_of_the_channel_juliaup_lists() {
        let file = r"C:\Users\someone\.julia\juliaup\julia-1.12.6+0.x64.w64.mingw32\bin\julia.exe";
        let listed = serde_json::json!({
            "DefaultChannel": { "Name": "release", "File": r"C:\j\julia-1.12.7\bin\julia.exe", "Args": [], "Version": "1.12.7", "Arch": "x64" },
            "OtherChannels": [{ "Name": "1.12.6", "File": file, "Args": [], "Version": "1.12.6", "Arch": "x64" }],
        })
        .to_string();
        assert_eq!(channel_file(&listed, "1.12.6"), Some(PathBuf::from(file)));
        assert_eq!(channel_file(&listed, "1.12.5"), None);
        let as_default = serde_json::json!({ "DefaultChannel": { "Name": "1.12.6", "File": file }, "OtherChannels": [] }).to_string();
        assert_eq!(channel_file(&as_default, "1.12.6"), Some(PathBuf::from(file)));
        assert_eq!(channel_file(r#"{"DefaultChannel":null,"OtherChannels":[]}"#, "1.12.6"), None, "a juliaup with no channels yet");
        assert_eq!(channel_file("not json", "1.12.6"), None);
    }
}
