//! First-run installs into Endeavor's Application Support folder: pinned,
//! checksummed downloads of runtimes the app doesn't bundle (design doc §11).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use endeavor_mcp::embedded;

/// The app's own files (`adapter/`): Contents/Resources inside Endeavor.app
/// (Resources beside bin on Windows), else, in a debug build, the source tree
/// (`cargo run`). A release build never falls back to the source tree, which
/// exists only on the machine that built it: `missing_files` says so instead.
pub fn resources() -> PathBuf {
    bundle_resources().unwrap_or_else(|| if source_fallback() { PathBuf::from(env!("CARGO_MANIFEST_DIR")) } else { expected_resources().unwrap_or_default() })
}

/// Debug builds use the source tree's files. `ENDEAVOR_TEST_NO_SOURCE_FALLBACK`
/// makes one behave like a release build here, to check `missing_files`.
fn source_fallback() -> bool {
    cfg!(debug_assertions) && std::env::var_os("ENDEAVOR_TEST_NO_SOURCE_FALLBACK").is_none()
}

/// What the app says at startup when its files aren't beside it: a release
/// build run from inside a zip Explorer opened, or a copy of the program taken
/// out of Endeavor.app.
pub fn missing_files() -> Option<String> {
    if bundle_resources().is_some() || source_fallback() {
        return None;
    }
    let what = if cfg!(windows) {
        "Endeavor's files are missing. If you opened it from a zip, extract all of it first, then run bin\\endeavor.exe."
    } else if cfg!(target_os = "macos") {
        "Endeavor's files are missing. Open Endeavor.app itself, not a copy of the program inside it."
    } else {
        "Endeavor's files are missing. A release build needs its Resources folder beside the folder it runs from. On Linux, run Endeavor with cargo run for now."
    };
    Some(match expected_resources() {
        Some(dir) => format!("{what}\n\nEndeavor looked for them in {}.", dir.display()),
        None => what.to_owned(),
    })
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
    expected_resources().filter(|r| r.join("adapter").is_dir())
}

/// Where the app's files belong: Resources beside the program's own folder.
fn expected_resources() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("Resources"))
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
/// its SHA-256, and unpack its `top` folder to `dir` (the whole of it when
/// `top` is empty: a zip with its files at the top). `what` names it in
/// progress and errors.
pub fn tarball(dir: &Path, what: &str, top: &str, (url, sha256, size): (&str, &str, u64), progress: &dyn Fn(String, Option<f32>)) -> Result<(), String> {
    let parent = dir.parent().unwrap();
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let kind = if url.ends_with(".zip") { "zip" } else { "tar.gz" };
    let name = if top.is_empty() { dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { top.to_owned() };
    let tarball = parent.join(format!("{name}.{kind}.part"));

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
    let staging = parent.join(format!("{name}.unpacking"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let untar = unpack(&tarball, &staging)?;
    let unpacked = if top.is_empty() { staging.clone() } else { staging.join(top) };
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
/// downloads) named `prefix` and a version older than `version`, such as
/// `julia-1.12.5` beside `julia-1.12.6`. Newer ones stay: another build
/// sharing the folder (an installed app beside a development build) may use
/// them. Those holding a path in `keep` stay too. Best effort: what can't be
/// removed (a file in use on Windows) stays until next time.
pub fn remove_other_versions(prefix: &str, version: &str, keep: &[&Path]) {
    if let Ok(app) = app_dir() {
        remove_other_versions_in(&app, prefix, version, keep);
    }
}

/// Remove every version named `prefix` from the app's folder, but those
/// holding a path in `keep`: on Windows juliaup's Julia replaces the app's own.
pub fn remove_all_versions(prefix: &str, keep: &[&Path]) {
    if let Ok(app) = app_dir() {
        remove_versions_in(&app, prefix, None, keep);
    }
}

fn remove_other_versions_in(app: &Path, prefix: &str, version: &str, keep: &[&Path]) {
    if let Some(current) = version_of(version) {
        remove_versions_in(app, prefix, Some(&current), keep);
    }
}

/// Those older than `below`, or with none, all.
fn remove_versions_in(app: &Path, prefix: &str, below: Option<&[u64]>, keep: &[&Path]) {
    let Ok(entries) = std::fs::read_dir(app) else { return };
    // Compared canonical, so a Settings path in another case or form still matches.
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    let keep: Vec<PathBuf> = keep.iter().map(|k| canonical(k)).collect();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // A version older than this one (not this one's download or unpacking).
        let older = name.strip_prefix(prefix).and_then(version_of).is_some_and(|v| below.is_none_or(|below| v.as_slice() < below));
        if !older || keep.iter().any(|k| k.starts_with(canonical(&path))) {
            continue;
        }
        let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match removed {
            Ok(()) => eprintln!("Removed {}, which an older version of Endeavor installed.", path.display()),
            Err(e) => eprintln!("Couldn't remove {}, which an older version of Endeavor installed: {e}", path.display()),
        }
    }
}

/// The version a name starts with, as numbers: "1.12.6.unpacking" -> [1, 12, 6].
fn version_of(name: &str) -> Option<Vec<u64>> {
    let digits = name.split(|c: char| !c.is_ascii_digit() && c != '.').next()?;
    let parts: Vec<u64> = digits.split('.').filter(|p| !p.is_empty()).map(|p| p.parse().ok()).collect::<Option<_>>()?;
    (name.starts_with(|c: char| c.is_ascii_digit()) && !parts.is_empty()).then_some(parts)
}

#[cfg(unix)]
fn unpack(tarball: &Path, into: &Path) -> Result<std::process::ExitStatus, String> {
    let zip = tarball.to_string_lossy().ends_with(".zip.part");
    // GNU tar can't read a zip; the Mac's bsdtar can, keeping the programs' modes.
    if zip && cfg!(target_os = "linux") {
        return Command::new("unzip").arg("-q").arg(tarball).arg("-d").arg(into).status().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "unzip isn't installed. Install it (for example `sudo apt install unzip`), then try again.".to_owned(),
            _ => format!("Couldn't run unzip: {e}"),
        });
    }
    Command::new("tar").arg(if zip { "-xf" } else { "-xzf" }).arg(tarball).arg("-C").arg(into).status().map_err(|e| e.to_string())
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
    #[cfg(debug_assertions)]
    fn a_debug_build_uses_the_source_tree() {
        assert!(std::env::var_os("ENDEAVOR_TEST_NO_SOURCE_FALLBACK").is_none());
        assert_eq!(resources(), PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        assert!(resources().join("adapter").is_dir());
        assert_eq!(missing_files(), None);
    }

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

    /// A zip with its files at the top, as Antigravity's are: unpacked with an
    /// empty `top`, its program still runnable.
    #[test]
    #[cfg(unix)]
    fn installs_a_zip_and_keeps_its_programs_runnable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("endeavor-install-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let program = src.join("server.par");
        std::fs::write(&program, "#!/bin/sh\necho ok\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(src.join("helper"), "").unwrap();
        let archive = tmp.join("server.zip");
        assert!(Command::new("zip").arg("-q").arg(&archive).arg("server.par").arg("helper").current_dir(&src).status().unwrap().success());
        let sha = sha256_of(&archive).unwrap();
        let url = format!("file://{}", archive.display());
        let size = std::fs::metadata(&archive).unwrap().len();

        let dir = tmp.join("app/server-1.0");
        tarball(&dir, "Server", "", (&url, &sha, size), &|_, _| {}).unwrap();
        assert!(dir.join("helper").exists());
        let mode = std::fs::metadata(dir.join("server.par")).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "lost its executable bit: {mode:o}");
        assert_eq!(Command::new(dir.join("server.par")).output().unwrap().stdout, b"ok\n");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn removes_only_other_versions() {
        let app = std::env::temp_dir().join(format!("endeavor-versions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&app);
        for dir in ["julia-1.12.5", "julia-1.12.6", "julia-1.12.6.unpacking", "julia-1.11.7", "julia-1.12.10", "julia-1.13.0", "node-v22.1.0", "adapter-0.80.0", "adapter-0.9.0", "adapter-0.100.0", "codex-adapter-0.1.0", "depot"] {
            std::fs::create_dir_all(app.join(dir).join("bin")).unwrap();
        }
        for file in ["julia-1.12.5.tar.gz.part", "julia-1.12.6.tar.gz.part"] {
            std::fs::write(app.join(file), "").unwrap();
        }
        let chosen = app.join("julia-1.11.7/bin/julia");
        std::fs::write(&chosen, "").unwrap();
        remove_other_versions_in(&app, "julia-", "1.12.6", &[&chosen]);
        remove_other_versions_in(&app, "adapter-", "0.81.2", &[]);
        let mut left: Vec<_> = std::fs::read_dir(&app).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["adapter-0.100.0", "codex-adapter-0.1.0", "depot", "julia-1.11.7", "julia-1.12.10", "julia-1.12.6", "julia-1.12.6.tar.gz.part", "julia-1.12.6.unpacking", "julia-1.13.0", "node-v22.1.0"]);

        remove_versions_in(&app, "julia-", None, &[&chosen]);
        let mut left: Vec<_> = std::fs::read_dir(&app).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["adapter-0.100.0", "codex-adapter-0.1.0", "depot", "julia-1.11.7", "node-v22.1.0"], "with no version, every Julia but the kept one");
        let _ = std::fs::remove_dir_all(&app);
    }
}
