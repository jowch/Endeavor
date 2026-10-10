//! First-run installs into Endeavor's Application Support folder: pinned,
//! checksummed downloads of runtimes the app doesn't bundle (design doc §11).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use endeavor_mcp::embedded;

/// The app's own files (`adapter/`): Contents/Resources inside Endeavor.app,
/// else the source tree (`cargo run`).
pub fn resources() -> PathBuf {
    bundle_resources().unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

/// The Julia side of the runtime (`runtime/`), unpacked from the helper crate
/// into the app's folder once per version.
pub fn runtime() -> Result<PathBuf, String> {
    let dir = endeavor_mcp::unpack(&app_dir()?.join("runtime-files"), embedded::RUNTIME_VERSION, embedded::RUNTIME_FILES)?;
    Ok(dir.join("runtime"))
}

/// The Pluto skills as the Claude Code plugin the app loads, unpacked the same
/// way. The app holds the folder while it runs: Claude sessions read it for
/// as long as they last, and another version's unpack removes unheld folders.
pub fn plugin() -> Result<PathBuf, String> {
    static LEASE: OnceLock<Option<endeavor_mcp::Lease>> = OnceLock::new();
    let dir = endeavor_mcp::unpack(&app_dir()?.join("plugin"), embedded::PLUGIN_VERSION, embedded::PLUGIN_FILES)?;
    LEASE.get_or_init(|| endeavor_mcp::lease(&dir));
    Ok(dir.join("plugin"))
}

/// Running from Endeavor.app, not a source checkout.
pub fn bundled() -> bool {
    bundle_resources().is_some()
}

fn bundle_resources() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("Resources")).filter(|r| r.join("adapter").is_dir())
}

/// Endeavor's folder in Application Support, on Linux in XDG_DATA_HOME, and on
/// Windows in %LOCALAPPDATA% (runtimes, Julia depot, app state).
pub fn app_dir() -> Result<PathBuf, String> {
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").filter(|d| !d.is_empty()).ok_or("LOCALAPPDATA isn't set")?;
        return Ok(PathBuf::from(local).join("Endeavor"));
    }
    let home = PathBuf::from(std::env::var("HOME").map_err(|e| e.to_string())?);
    if cfg!(target_os = "macos") {
        return Ok(home.join("Library/Application Support/endeavor"));
    }
    let data = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()).map_or_else(|| home.join(".local/share"), PathBuf::from);
    Ok(data.join("endeavor"))
}

/// Download a pinned tarball (a zip on Windows), resuming a partial one, check
/// its SHA-256, and unpack its `top` folder to `dir`. `what` names it in
/// progress and errors.
pub fn tarball(dir: &Path, what: &str, top: &str, (url, sha256, size): (&str, &str, u64), progress: &dyn Fn(String, Option<f32>)) -> Result<(), String> {
    let parent = dir.parent().unwrap();
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let kind = if url.ends_with(".zip") { "zip" } else { "tar.gz" };
    let tarball = parent.join(format!("{top}.{kind}.part"));

    // ponytail: curl outlives an app quit mid-download; a relaunch that overlaps it
    // fails the SHA check and starts over. Kill it on quit if that bites.
    let mut curl = Command::new("curl");
    endeavor_mcp::client::no_window(&mut curl);
    let mut curl = curl
        .args(["-fsSL", "--retry", "3", "-C", "-", "-o"])
        .arg(&tarball)
        .arg(url)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start curl to download {what}: {e}"))?;
    let status = loop {
        if let Some(status) = curl.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        let got = std::fs::metadata(&tarball).map(|m| m.len()).unwrap_or(0);
        progress(format!("Downloading {what}… {}%", got * 100 / size), Some(got as f32 / size as f32));
        std::thread::sleep(Duration::from_millis(500));
    };
    if !status.success() {
        let mut err = String::new();
        let _ = std::io::Read::read_to_string(&mut curl.stderr.take().unwrap(), &mut err);
        return Err(format!(
            "Couldn't download {what} ({}). Check the internet connection and try again; the download resumes.",
            err.trim()
        ));
    }

    progress(format!("Checking {what}…"), None);
    let got = sha256_of(&tarball)?;
    if got != sha256 {
        let _ = std::fs::remove_file(&tarball);
        return Err(format!("The {what} download was corrupt or tampered with (SHA-256 {got}); it was deleted. Try again to download it afresh."));
    }

    // Unpack beside the target, then rename, so a half-unpacked Julia is never used.
    progress(format!("Unpacking {what}…"), None);
    let staging = parent.join(format!("{top}.unpacking"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let untar = unpack(&tarball, &staging)?;
    let unpacked = staging.join(top);
    if !untar.success() || !unpacked.is_dir() {
        return Err(format!("Couldn't unpack {what} ({untar})."));
    }
    std::fs::rename(&unpacked, dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&tarball);
    Ok(())
}

fn sha256_of(file: &Path) -> Result<String, String> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let mut file = std::fs::File::open(file).map_err(|e| e.to_string())?;
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Remove what older pins left in the app's folder: the folders (and partial
/// downloads) named `prefix` and a version other than `version`, such as
/// `julia-1.12.5` beside `julia-1.12.6`. Those holding a path in `keep` stay.
/// Best effort: what can't be removed (a file in use on Windows) stays until next time.
pub fn remove_other_versions(prefix: &str, version: &str, keep: &[&Path]) {
    if let Ok(app) = app_dir() {
        remove_other_versions_in(&app, prefix, version, keep);
    }
}

fn remove_other_versions_in(app: &Path, prefix: &str, version: &str, keep: &[&Path]) {
    let Ok(entries) = std::fs::read_dir(app) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(prefix) else { continue };
        // A version, and not this one or its download or unpacking.
        let this_one = rest.strip_prefix(version).is_some_and(|after| !after.starts_with(|c: char| c.is_ascii_digit()));
        if !rest.starts_with(|c: char| c.is_ascii_digit()) || this_one || keep.iter().any(|k| k.starts_with(&path)) {
            continue;
        }
        let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match removed {
            Ok(()) => eprintln!("Removed {}, which an older version of Endeavor installed.", path.display()),
            Err(e) => eprintln!("Couldn't remove {}, which an older version of Endeavor installed: {e}", path.display()),
        }
    }
}

#[cfg(unix)]
fn unpack(tarball: &Path, into: &Path) -> Result<std::process::ExitStatus, String> {
    Command::new("tar").arg("-xzf").arg(tarball).arg("-C").arg(into).status().map_err(|e| e.to_string())
}

#[cfg(windows)]
fn unpack(zip: &Path, into: &Path) -> Result<std::process::ExitStatus, String> {
    windows_tar().arg("-xf").arg(zip).arg("-C").arg(into).status().map_err(|e| e.to_string())
}

/// Windows' own bsdtar, which reads zips. A `tar` earlier on PATH may be GNU
/// tar from Git for Windows, which doesn't, and takes `C:` for a remote host.
#[cfg(windows)]
fn windows_tar() -> Command {
    let system = std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join("System32").join("tar.exe"));
    let mut tar = Command::new(system.filter(|tar| tar.exists()).unwrap_or_else(|| PathBuf::from("tar.exe")));
    endeavor_mcp::client::no_window(&mut tar);
    tar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn app_data_and_logs_are_in_local_app_data() {
        let ours = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("Endeavor");
        assert_eq!(app_dir().unwrap(), ours);
        assert_eq!(crate::logs::path().unwrap(), ours.join("Logs").join("endeavor.log"));
    }

    #[test]
    fn installs_a_verified_tarball_and_rejects_a_bad_one() {
        let tmp = std::env::temp_dir().join(format!("endeavor-install-{}", std::process::id()));
        let src = tmp.join("src/thing-1.0/bin");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("thing"), "#!/bin/sh\n").unwrap();
        let kind = if cfg!(windows) { "zip" } else { "tar.gz" };
        let archive = tmp.join(format!("thing.{kind}"));
        #[cfg(unix)]
        let mut pack = Command::new("tar");
        #[cfg(unix)]
        pack.arg("-czf");
        // -a: the format from the file name.
        #[cfg(windows)]
        let mut pack = windows_tar();
        #[cfg(windows)]
        pack.arg("-a").arg("-cf");
        assert!(pack.arg(&archive).arg("-C").arg(tmp.join("src")).arg("thing-1.0").status().unwrap().success());
        let sha = sha256_of(&archive).unwrap();
        assert_eq!(sha.len(), 64);
        let url = format!("file://{}{}", if cfg!(windows) { "/" } else { "" }, archive.display().to_string().replace('\\', "/"));
        let size = std::fs::metadata(&archive).unwrap().len();

        let bad = tmp.join("app/bad");
        let err = tarball(&bad, "Thing", "thing-1.0", (&url, &"0".repeat(64), size), &|_, _| {}).unwrap_err();
        assert!(err.contains("corrupt"), "{err}");
        assert!(!bad.exists() && !tmp.join(format!("app/thing-1.0.{kind}.part")).exists());

        let good = tmp.join("app/good");
        tarball(&good, "Thing", "thing-1.0", (&url, &sha, size), &|_, _| {}).unwrap();
        assert!(good.join("bin/thing").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn removes_only_other_versions() {
        let app = std::env::temp_dir().join(format!("endeavor-versions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&app);
        for dir in ["julia-1.12.5", "julia-1.12.6", "julia-1.12.6.unpacking", "julia-1.11.7", "julia-1.12.61", "node-v22.1.0", "adapter-0.80.0", "codex-adapter-2.0.0", "depot"] {
            std::fs::create_dir_all(app.join(dir).join("bin")).unwrap();
        }
        for file in ["julia-1.12.5.tar.gz.part", "julia-1.12.6.tar.gz.part"] {
            std::fs::write(app.join(file), "").unwrap();
        }
        let chosen = app.join("julia-1.11.7/bin/julia");
        remove_other_versions_in(&app, "julia-", "1.12.6", &[&chosen]);
        remove_other_versions_in(&app, "adapter-", "0.81.2", &[]);
        let mut left: Vec<_> = std::fs::read_dir(&app).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["codex-adapter-2.0.0", "depot", "julia-1.11.7", "julia-1.12.6", "julia-1.12.6.tar.gz.part", "julia-1.12.6.unpacking", "node-v22.1.0"]);
        let _ = std::fs::remove_dir_all(&app);
    }
}
