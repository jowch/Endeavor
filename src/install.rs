//! First-run installs into Endeavor's Application Support folder: pinned,
//! checksummed downloads of runtimes the app doesn't bundle (design doc §11).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Endeavor's folder in Application Support (runtimes, Julia depot, app state).
pub fn app_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    Ok(PathBuf::from(home).join("Library/Application Support/endeavor"))
}

/// Download a pinned tarball (resuming a partial one), check its SHA-256, and
/// unpack its `top` folder to `dir`. `what` names it in progress and errors.
pub fn tarball(dir: &Path, what: &str, top: &str, (url, sha256, size): (&str, &str, u64), progress: &dyn Fn(String)) -> Result<(), String> {
    let parent = dir.parent().unwrap();
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tarball = parent.join(format!("{top}.tar.gz.part"));

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
        progress(format!("Downloading {what} (first launch)… {}%", got * 100 / size));
        std::thread::sleep(Duration::from_millis(500));
    };
    if !status.success() {
        let mut err = String::new();
        let _ = std::io::Read::read_to_string(&mut curl.stderr.take().unwrap(), &mut err);
        return Err(format!(
            "Couldn't download {what} ({}). Check the internet connection and restart; the download resumes.",
            err.trim()
        ));
    }

    progress(format!("Checking {what}…"));
    let out = Command::new("shasum").args(["-a", "256"]).arg(&tarball).output().map_err(|e| e.to_string())?;
    let got = String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or_default().to_owned();
    if got != sha256 {
        let _ = std::fs::remove_file(&tarball);
        return Err(format!("The {what} download was corrupt or tampered with (SHA-256 {got}); it was deleted. Restart to try again."));
    }

    // Unpack beside the target, then rename, so a half-unpacked Julia is never used.
    progress(format!("Unpacking {what}…"));
    let staging = parent.join(format!("{top}.unpacking"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let untar = Command::new("tar").arg("-xzf").arg(&tarball).arg("-C").arg(&staging).status().map_err(|e| e.to_string())?;
    let unpacked = staging.join(top);
    if !untar.success() || !unpacked.is_dir() {
        return Err(format!("Couldn't unpack {what} ({untar})."));
    }
    std::fs::rename(&unpacked, dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&tarball);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_a_verified_tarball_and_rejects_a_bad_one() {
        let tmp = std::env::temp_dir().join(format!("endeavor-install-{}", std::process::id()));
        let src = tmp.join("src/thing-1.0/bin");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("thing"), "#!/bin/sh\n").unwrap();
        let archive = tmp.join("thing.tar.gz");
        let ok = Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(tmp.join("src")).arg("thing-1.0").status().unwrap();
        assert!(ok.success());
        let out = Command::new("shasum").args(["-a", "256"]).arg(&archive).output().unwrap();
        let sha = String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap().to_owned();
        let url = format!("file://{}", archive.display());
        let size = std::fs::metadata(&archive).unwrap().len();

        let bad = tmp.join("app/bad");
        let err = tarball(&bad, "Thing", "thing-1.0", (&url, &"0".repeat(64), size), &|_| {}).unwrap_err();
        assert!(err.contains("corrupt"), "{err}");
        assert!(!bad.exists() && !tmp.join("app/thing-1.0.tar.gz.part").exists());

        let good = tmp.join("app/good");
        tarball(&good, "Thing", "thing-1.0", (&url, &sha, size), &|_| {}).unwrap();
        assert!(good.join("bin/thing").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
