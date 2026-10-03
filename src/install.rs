//! First-run installs into Endeavor's Application Support folder: pinned,
//! checksummed downloads of runtimes the app doesn't bundle (design doc §11).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

/// The Pluto skills as the Claude Code plugin the app loads, unpacked the same way.
pub fn plugin() -> Result<PathBuf, String> {
    let dir = endeavor_mcp::unpack(&app_dir()?.join("plugin"), embedded::PLUGIN_VERSION, embedded::PLUGIN_FILES)?;
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
    let mut curl = Command::new("curl")
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

#[cfg(unix)]
fn sha256_of(file: &Path) -> Result<String, String> {
    let out = Command::new("shasum").args(["-a", "256"]).arg(file).output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or_default().to_owned())
}

/// Windows has no `shasum`.
#[cfg(windows)]
fn sha256_of(file: &Path) -> Result<String, String> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let mut file = std::fs::File::open(file).map_err(|e| e.to_string())?;
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
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
    Command::new(system.filter(|tar| tar.exists()).unwrap_or_else(|| PathBuf::from("tar.exe")))
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
}
