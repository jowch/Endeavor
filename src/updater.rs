//! A nightly app updating itself to the newest nightly (app #80). The nightly workflow publishes
//! `nightly.json` beside the dmg (scripts/nightly-manifest.sh). Once a day,
//! and on Check now, the app reads it; when it names a build committed after
//! this one, the app downloads the dmg, checks its SHA-256 against the
//! manifest and its signature, copies Endeavor.app out of it, and checks that
//! copy's signature: a Developer ID of the same team as the running app, and
//! notarized. The hash only proves the file is the one the manifest names
//! (both come from the same release); the signature is what proves it is ours.
//! The manifest isn't signed, so the build it names is only trusted once the
//! new app's own Info.plist (sealed by its signature, scripts/bundle.sh) says
//! the same commit, committed after this build. Restart, or any quit, then
//! swaps the apps once this one has exited.
//!
//! On Windows the app downloads the installer and checks its SHA-256, and
//! Restart, or any quit, runs it silently once the app has exited (it waits
//! for the app with /UPDATE, scripts/installer.iss). The installer isn't
//! signed (#12), so the hash is the only check: a nightly Windows user trusts
//! the release as much as when they first installed from it, and someone who
//! can change the release could ship any program, not just an older one.
//!
//! Only a build made by the nightly workflow (`ENDEAVOR_CHANNEL=nightly`,
//! build.rs) checks: on a Mac from a signed Endeavor.app, on Windows from
//! where the installer put it. A build from source and Linux never do. When
//! the app can't replace itself (a Mac app not in Applications or run from the
//! disk image, a Windows build unzipped by hand), the newer build is offered
//! as a download instead.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;

/// The full hash, committer time and channel of this build (build.rs).
pub const COMMIT: &str = env!("ENDEAVOR_COMMIT");
pub const COMMITTED: &str = env!("ENDEAVOR_COMMITTED");
pub const CHANNEL: &str = env!("ENDEAVOR_CHANNEL");

const RELEASE: &str = "https://github.com/jowch/Endeavor/releases/download/nightly";
/// Where a newer build is offered when the app can't replace itself.
pub const RELEASE_PAGE: &str = "https://github.com/jowch/Endeavor/releases/tag/nightly";
/// This app's build in the manifest.
const PLATFORM: &str = if cfg!(windows) { "windows-x86_64" } else { "macos-arm64" };
/// The app's own name inside the disk image, and the bundle it replaces.
const APP: &str = "Endeavor.app";

#[derive(Debug, Deserialize)]
pub struct Manifest {
    schema: u32,
    commit: String,
    committed: String,
    files: std::collections::HashMap<String, File>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct File {
    name: String,
    sha256: String,
    size: u64,
}

/// A newer build to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub commit: String,
    file: File,
}

impl Offer {
    /// What the user sees: the short hash, as About shows a build.
    pub fn build(&self) -> String {
        self.commit[..7].to_owned()
    }
}

/// Read a manifest, refusing one this app doesn't understand.
pub fn parse(json: &str) -> Result<Manifest, String> {
    let m: Manifest = serde_json::from_str(json).map_err(|e| format!("the update list isn't readable ({e})"))?;
    if m.schema != 1 {
        return Err(format!("the update list is a newer kind ({}); download the newest build by hand", m.schema));
    }
    if m.commit.len() != 40 || !m.commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the update list names no build".into());
    }
    if !utc_time(&m.committed) {
        return Err("the update list has no build time".into());
    }
    Ok(m)
}

/// `2026-10-15T08:12:00Z`: the one format both sides write, so two of them
/// compare as strings.
fn utc_time(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 20
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'Z',
            _ => c.is_ascii_digit(),
        })
}

/// The manifest's Mac build, when it was committed after `committed` (this
/// build's time). An older or equal one is never offered, so a manifest that
/// lags (or a build newer than the nightly) doesn't "update" backwards.
pub fn offer(m: &Manifest, commit: &str, committed: &str) -> Option<Offer> {
    if !utc_time(committed) || m.commit == commit || m.committed.as_str() <= committed {
        return None;
    }
    let file = m.files.get(PLATFORM)?;
    // A plain file name: it is put after the release's URL.
    if file.name.is_empty() || !file.name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) || file.sha256.len() != 64 {
        return None;
    }
    Some(Offer { commit: m.commit.clone(), file: file.clone() })
}

/// Where this app is, and whether it can replace itself there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub app: PathBuf,
    pub replaceable: bool,
}

/// Whether this build checks for updates at all; why not when it doesn't.
pub fn place() -> Result<Place, &'static str> {
    // Debug builds: `ENDEAVOR_TEST_UPDATE_MANIFEST` names a manifest file that
    // stands in for the release's, on any system, and the app never replaces
    // itself (a newer build is offered as a download).
    #[cfg(debug_assertions)]
    if test_manifest().is_some() {
        return Ok(Place { app: std::env::current_exe().unwrap_or_default(), replaceable: false });
    }
    if !cfg!(any(target_os = "macos", windows)) {
        return Err("only the Mac and Windows apps update themselves");
    }
    if CHANNEL != "nightly" {
        return Err("a build from source doesn't update itself");
    }
    let exe = std::env::current_exe().map_err(|_| "can't tell where the app is")?;
    if cfg!(windows) {
        let app = install_of(&exe).ok_or("can't tell where the app is")?;
        // Installed by the installer, which can replace it; not a build
        // unzipped by hand.
        let replaceable = app.join(UNINSTALLER).is_file();
        return Ok(Place { app, replaceable });
    }
    let app = bundle_of(&exe).ok_or("not running from Endeavor.app")?;
    let replaceable = app.parent().is_some_and(writable) && writable(&app);
    Ok(Place { app, replaceable })
}

#[cfg(debug_assertions)]
fn test_manifest() -> Option<PathBuf> {
    std::env::var_os("ENDEAVOR_TEST_UPDATE_MANIFEST").filter(|f| !f.is_empty()).map(PathBuf::from)
}

/// `…/Endeavor.app` from `…/Endeavor.app/Contents/MacOS/endeavor`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app" && exe.parent()?.ends_with("Contents/MacOS")).then(|| app.to_owned())
}

/// The installer's own uninstaller, beside bin\ in the folder it installs to.
const UNINSTALLER: &str = "unins000.exe";

/// `…\Endeavor` from `…\Endeavor\bin\endeavor.exe`.
fn install_of(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    (bin.file_name()? == "bin").then(|| bin.parent().map(Path::to_owned)).flatten()
}

#[cfg(not(unix))]
fn writable(_: &Path) -> bool {
    false
}

#[cfg(unix)]
fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid C string.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// How long after a check the next one is due: a day, or an hour after one
/// that failed (offline, or the release mid-publish).
const DAILY: Duration = Duration::from_secs(24 * 3600);
const RETRY: Duration = Duration::from_secs(3600);

/// The app's update, as the workspace keeps it.
pub struct Updater {
    /// Where the app is; why it doesn't update when it doesn't.
    pub place: Result<Place, &'static str>,
    pub state: State,
    /// The download's progress in thousandths, while `state` is Checking.
    pub progress: Arc<AtomicU32>,
    last_try: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Not checked yet this run.
    Waiting,
    /// Checking, then downloading when there's a newer build.
    Checking,
    UpToDate,
    Ready { build: String, commit: String, staged: PathBuf },
    Manual { build: String },
    Failed(String),
    /// This copy can't update (see `Outcome::Off`).
    Off(&'static str),
    /// Restart or quit started the swap.
    Installing,
}

impl Updater {
    pub fn new() -> Self {
        let place = place();
        let mut last_try = None;
        let state = match place {
            // The swap at the last quit failed (`install`): say so, and try
            // again in an hour rather than straight away.
            Ok(_) if folder().is_ok_and(|dir| last_install_failed(&dir, COMMIT)) => {
                last_try = Some(Instant::now());
                State::Failed("the last update couldn't be installed".into())
            }
            Ok(_) => State::Waiting,
            Err(why) => State::Off(why),
        };
        Updater { place, state, progress: Arc::default(), last_try }
    }

    /// Whether a check should start now (Check now starts one regardless).
    pub fn due(&self, now: Instant) -> bool {
        let wait = match self.state {
            State::Waiting => return true,
            State::UpToDate | State::Manual { .. } => DAILY,
            State::Failed(_) => RETRY,
            State::Checking | State::Ready { .. } | State::Off(_) | State::Installing => return false,
        };
        self.last_try.is_none_or(|t| now.duration_since(t) >= wait)
    }

    /// Start a check, returning where the app is; None when this copy doesn't
    /// update, or a check is running, or a build is ready.
    pub fn start(&mut self, now: Instant) -> Option<Place> {
        let place = self.place.clone().ok()?;
        if matches!(self.state, State::Checking | State::Ready { .. } | State::Off(_) | State::Installing) {
            return None;
        }
        self.state = State::Checking;
        self.last_try = Some(now);
        self.progress.store(0, Ordering::Relaxed);
        Some(place)
    }

    pub fn finish(&mut self, result: Result<Outcome, String>) {
        self.state = match result {
            Ok(Outcome::UpToDate) => State::UpToDate,
            Ok(Outcome::Ready { build, commit, staged }) => State::Ready { build, commit, staged },
            Ok(Outcome::Manual { build }) => State::Manual { build },
            Ok(Outcome::Off(why)) => State::Off(why),
            Err(e) => State::Failed(e),
        };
    }

    /// The download's progress, 0 to 100, once it has started.
    pub fn percent(&self) -> Option<u8> {
        let p = self.progress.load(Ordering::Relaxed);
        (self.state == State::Checking && p > 0).then(|| (p.min(1000) / 10) as u8)
    }

    /// Swap in the fetched build once the app has exited (and start it again
    /// when `relaunch`). Nothing to do unless one is ready.
    pub fn install(&mut self, relaunch: bool) -> Result<bool, String> {
        let (State::Ready { staged, commit, .. }, Ok(place)) = (&self.state, &self.place) else { return Ok(false) };
        if cfg!(windows) {
            run_installer(staged, commit, &folder()?, relaunch)?;
        } else {
            swap(staged, &place.app, std::process::id(), relaunch, &folder()?.join(FAILED))?;
        }
        self.state = State::Installing;
        Ok(true)
    }
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    /// Downloaded and checked, waiting for a restart.
    Ready { build: String, commit: String, staged: PathBuf },
    /// Newer, but the app can't replace itself where it is.
    Manual { build: String },
    /// This copy of the app isn't one that can update (not Developer ID signed).
    Off(&'static str),
}

/// Check, and fetch a newer build when there is one: newer than the build
/// committed as `commit` at `committed` (this one's COMMIT and COMMITTED).
/// Blocking; `progress` gets the download's fraction.
pub fn check(place: &Place, commit: &str, committed: &str, progress: &dyn Fn(f32)) -> Result<Outcome, String> {
    let m = parse(&manifest()?)?;
    let Some(offer) = offer(&m, commit, committed) else { return Ok(Outcome::UpToDate) };
    if !place.replaceable {
        return Ok(Outcome::Manual { build: offer.build() });
    }
    let staged = if cfg!(windows) {
        fetch_installer(RELEASE, &folder()?, &offer, progress)?
    } else {
        let Some(team) = team_of(&place.app) else { return Ok(Outcome::Off("this copy isn't signed with a Developer ID")) };
        fetch(&folder()?, &offer, &team, committed, progress)?
    };
    Ok(Outcome::Ready { build: offer.build(), commit: offer.commit, staged })
}

fn manifest() -> Result<String, String> {
    #[cfg(debug_assertions)]
    if let Some(file) = test_manifest() {
        return std::fs::read_to_string(&file).map_err(|e| format!("couldn't read {} ({e})", file.display()));
    }
    let mut curl = Command::new("curl");
    endeavor_mcp::client::no_window(&mut curl);
    curl.args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--max-time", "60"]).arg(format!("{RELEASE}/nightly.json"));
    let out = curl.stdin(Stdio::null()).output().map_err(|e| format!("couldn't start curl ({e})"))?;
    if !out.status.success() {
        return Err(format!("couldn't reach GitHub ({})", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The Developer ID team that signed the app at `app`, from codesign's
/// "TeamIdentifier=…" line (on stderr); None for ad-hoc or unsigned.
fn team_of(app: &Path) -> Option<String> {
    let out = Command::new("codesign").args(["-dv", "--verbose=2"]).arg(app).output().ok()?;
    team_in(&String::from_utf8_lossy(&out.stderr))
}

fn team_in(codesign: &str) -> Option<String> {
    let team = codesign.lines().find_map(|l| l.strip_prefix("TeamIdentifier="))?.trim();
    (team.len() == 10 && team.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| team.to_owned())
}

/// The folder updates are fetched into, in the app's own folder.
fn folder() -> Result<PathBuf, String> {
    Ok(crate::install::app_dir()?.join("update"))
}

/// Left in `folder()` by a swap that failed, for the next launch to report.
const FAILED: &str = "failed";

/// Fetch the offer into `dir` and check it, the app's own Info.plist
/// included. Returns the checked copy. What other builds left in `dir` goes.
fn fetch(dir: &Path, offer: &Offer, team: &str, committed: &str, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(&offer.commit) {
            let _ = std::fs::remove_dir_all(entry.path()).or_else(|_| std::fs::remove_file(entry.path()));
        }
    }
    let staged = dir.join(&offer.commit).join(APP);
    if staged.is_dir() && verify(&staged, team).and_then(|()| same_build(&staged, &offer.commit, committed)).is_ok() {
        return Ok(staged);
    }
    let staged = stage(RELEASE, dir, offer, team, progress)?;
    if let Err(e) = same_build(&staged, &offer.commit, committed) {
        let _ = std::fs::remove_dir_all(staged.parent().unwrap());
        return Err(e);
    }
    Ok(staged)
}

/// Download the offer's disk image from `base`, check it, and copy a signed,
/// notarized app out of it into `dir`. Returns the copy's path.
fn stage(base: &str, dir: &Path, offer: &Offer, team: &str, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    let staged = dir.join(&offer.commit).join(APP);
    let dmg = dir.join(format!("{}.dmg.part", offer.commit));
    let url = format!("{base}/{}", offer.file.name);
    crate::install::download(&url, &dmg, "the update", offer.file.size, &|_, f| progress(f.unwrap_or(0.)))?;
    let got = crate::install::sha256_of(&dmg)?;
    if got != offer.file.sha256 {
        let _ = std::fs::remove_file(&dmg);
        return Err("the download didn't match the update list; it was deleted and is tried again later".into());
    }
    // The disk image is signed too: one that isn't ours is never mounted.
    if let Err(e) = run(Command::new("codesign").args(["--verify", "-R"]).arg(requirement(team)).arg(&dmg), "check the disk image's signature") {
        let _ = std::fs::remove_file(&dmg);
        return Err(e);
    }

    let mount = dir.join(format!("{}.mount", offer.commit));
    // Left mounted by a check that was cut short.
    if mount.exists() {
        let _ = run(Command::new("hdiutil").args(["detach", "-force"]).arg(&mount), "close the disk image");
    }
    let _ = std::fs::remove_dir_all(staged.parent().unwrap());
    std::fs::create_dir_all(staged.parent().unwrap()).map_err(|e| e.to_string())?;
    run(Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"]).arg(&mount).arg(&dmg), "open the disk image")?;
    // ditto keeps the app's signature and attributes, as the disk image does.
    let copied = run(Command::new("ditto").arg(mount.join(APP)).arg(&staged), "copy the app out of the disk image");
    if run(Command::new("hdiutil").arg("detach").arg(&mount), "close the disk image").is_err() {
        let _ = run(Command::new("hdiutil").args(["detach", "-force"]).arg(&mount), "close the disk image");
    }
    copied?;
    let _ = std::fs::remove_file(&dmg);
    if let Err(e) = verify(&staged, team) {
        let _ = std::fs::remove_dir_all(staged.parent().unwrap());
        return Err(e);
    }
    Ok(staged)
}

/// Signed with a Developer ID Application certificate of `team`: not the
/// team's development or App Store certificates, which `spctl` would refuse
/// but nothing else here would once Gatekeeper is off.
fn requirement(team: &str) -> String {
    format!("=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"{team}\"")
}

/// The new app is signed by the same Developer ID team as this one, and
/// notarized.
fn verify(app: &Path, team: &str) -> Result<(), String> {
    run(Command::new("codesign").args(["--verify", "--strict", "--deep", "-R"]).arg(requirement(team)).arg(app), "check the new app's signature")?;
    run(Command::new("spctl").args(["--assess", "--type", "execute"]).arg(app), "check that the new app is notarized")
}

/// The checked app at `app` is the build `commit`, committed after
/// `committed`, by its own Info.plist. The signature seals that file, so this
/// is what stops a changed manifest offering an older signed build.
fn same_build(app: &Path, commit: &str, committed: &str) -> Result<(), String> {
    let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).map_err(|e| format!("couldn't read the new app's Info.plist ({e})"))?;
    is_build(&plist, commit, committed)
}

fn is_build(plist: &str, commit: &str, committed: &str) -> Result<(), String> {
    let (Some(its_commit), Some(its_time)) = (plist_string(plist, "EndeavorCommit"), plist_string(plist, "EndeavorCommitted")) else {
        return Err("the new app doesn't say which build it is".into());
    };
    if its_commit != commit {
        return Err("the new app isn't the build the update list names".into());
    }
    if !utc_time(its_time) || !utc_time(committed) || its_time <= committed {
        return Err("the new app isn't newer than this one".into());
    }
    Ok(())
}

/// `<key>KEY</key><string>VALUE</string>` in an XML Info.plist, as
/// scripts/bundle.sh writes it.
fn plist_string<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let rest = &plist[plist.find(&format!("<key>{key}</key>"))? + key.len() + 11..];
    let rest = rest.trim_start().strip_prefix("<string>")?;
    Some(&rest[..rest.find("</string>")?])
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("couldn't {what} ({e})"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("couldn't {what} ({})", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Swap the apps once process `pid` (this app) has exited, and start the new
/// one when `relaunch`. A detached shell does it, since the app has to be
/// gone first. The new app is copied beside the old one first, so both
/// renames stay on one volume and are atomic: a copy that fails leaves the
/// old app untouched, and a failed rename puts it back. Either leaves
/// `failed` for the next launch to report.
fn swap(staged: &Path, app: &Path, pid: u32, relaunch: bool, failed: &Path) -> Result<std::process::Child, String> {
    const SCRIPT: &str = r#"
pid=$1 app=$2 new=$3 open=$4 failed=$5
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
next="$app.next" old="$app.replaced"
rm -rf "$next" "$old"
if ditto "$new" "$next" && mv "$app" "$old"; then
  if mv "$next" "$app"; then rm -rf "$old" "$new"; else mv "$old" "$app"; rm -rf "$next"; : > "$failed"; fi
else
  rm -rf "$next"; : > "$failed"
fi
[ -z "$open" ] || open "$app"
"#;
    Command::new("/bin/sh")
        .args(["-c", SCRIPT, "sh"])
        .arg(pid.to_string())
        .arg(app)
        .arg(staged)
        .arg(if relaunch { "open" } else { "" })
        .arg(failed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start the update ({e})"))
}

/// Windows: download the offer's installer from `base` into `dir` and check
/// its SHA-256, keeping one already there that matches. Returns its path.
/// What other builds left in `dir` goes.
fn fetch_installer(base: &str, dir: &Path, offer: &Offer, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&offer.commit) && name != INSTALLING {
            let _ = std::fs::remove_dir_all(entry.path()).or_else(|_| std::fs::remove_file(entry.path()));
        }
    }
    let setup = dir.join(format!("{}-setup.exe", offer.commit));
    if setup.is_file() && crate::install::sha256_of(&setup).is_ok_and(|h| h == offer.file.sha256) {
        return Ok(setup);
    }
    let part = dir.join(format!("{}-setup.exe.part", offer.commit));
    crate::install::download(&format!("{base}/{}", offer.file.name), &part, "the update", offer.file.size, &|_, f| progress(f.unwrap_or(0.)))?;
    if crate::install::sha256_of(&part)? != offer.file.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err("the download didn't match the update list; it was deleted and is tried again later".into());
    }
    std::fs::rename(&part, &setup).map_err(|e| e.to_string())?;
    Ok(setup)
}

/// Left in `folder()` while a Windows update installs, naming the build it
/// installs, for the next launch to check.
const INSTALLING: &str = "installing";

/// Whether the update the last quit started didn't take: the Mac swap left
/// its marker, or the installer ran and this is still not the build it was
/// installing. Either marker is gone afterwards.
fn last_install_failed(dir: &Path, commit: &str) -> bool {
    let swap_failed = std::fs::remove_file(dir.join(FAILED)).is_ok();
    let installing = std::fs::read_to_string(dir.join(INSTALLING)).ok();
    if installing.is_some() {
        let _ = std::fs::remove_file(dir.join(INSTALLING));
    }
    swap_failed || installing.is_some_and(|c| c.trim() != commit)
}

/// Windows: run the fetched installer silently. It waits for this app to
/// quit (/UPDATE), replaces it, and opens the new one when `relaunch`
/// (/RELAUNCH). Its log goes beside it.
fn run_installer(setup: &Path, commit: &str, dir: &Path, relaunch: bool) -> Result<(), String> {
    std::fs::write(dir.join(INSTALLING), commit).map_err(|e| format!("couldn't start the update ({e})"))?;
    let mut cmd = Command::new(setup);
    cmd.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/UPDATE"]).arg(format!("/LOG={}", dir.join("install.log").display()));
    if relaunch {
        cmd.arg("/RELAUNCH");
    }
    // Not tied to this app's console or process group: it outlives the app.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(drop).map_err(|e| {
        let _ = std::fs::remove_file(dir.join(INSTALLING));
        format!("couldn't start the update ({e})")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn manifest(commit: &str, committed: &str) -> String {
        format!(
            r#"{{ "schema": 1, "build": "{}", "commit": "{commit}", "committed": "{committed}", "published": "2026-10-16T09:40:00Z",
                "files": {{ "macos-arm64": {{ "name": "Endeavor-macos-arm64.dmg", "sha256": "{}", "size": 123 }},
                            "windows-x86_64": {{ "name": "Endeavor-windows-x86_64-setup.exe", "sha256": "{}", "size": 45 }} }} }}"#,
            &commit[..7],
            "1".repeat(64),
            "2".repeat(64)
        )
    }

    #[test]
    fn a_later_build_is_offered() {
        let m = parse(&manifest(B, "2026-10-16T08:00:00Z")).unwrap();
        let o = offer(&m, A, "2026-10-15T08:00:00Z").unwrap();
        assert_eq!((o.build().as_str(), o.file.name.as_str(), o.file.size), ("bbbbbbb", "Endeavor-macos-arm64.dmg", 123));
    }

    #[test]
    fn the_same_or_an_older_build_is_not() {
        let same = parse(&manifest(A, "2026-10-16T08:00:00Z")).unwrap();
        assert_eq!(offer(&same, A, "2026-10-16T08:00:00Z"), None);
        let older = parse(&manifest(B, "2026-10-14T08:00:00Z")).unwrap();
        assert_eq!(offer(&older, A, "2026-10-15T08:00:00Z"), None);
        let equal_time = parse(&manifest(B, "2026-10-15T08:00:00Z")).unwrap();
        assert_eq!(offer(&equal_time, A, "2026-10-15T08:00:00Z"), None);
        // A build with no recorded time (not from git) never compares newer.
        let newer = parse(&manifest(B, "2026-10-16T08:00:00Z")).unwrap();
        assert_eq!(offer(&newer, A, ""), None);
    }

    #[test]
    fn no_mac_build_or_an_odd_name_is_not_offered() {
        let json = manifest(B, "2026-10-16T08:00:00Z").replace("macos-arm64", "macos-x86_64");
        assert_eq!(offer(&parse(&json).unwrap(), A, "2026-10-15T08:00:00Z"), None);
        let json = manifest(B, "2026-10-16T08:00:00Z").replace("Endeavor-macos-arm64.dmg", "../x.dmg");
        assert_eq!(offer(&parse(&json).unwrap(), A, "2026-10-15T08:00:00Z"), None);
    }

    #[test]
    fn a_manifest_this_app_doesnt_understand_is_refused() {
        assert!(parse(&manifest(B, "2026-10-16T08:00:00Z").replace("\"schema\": 1", "\"schema\": 2")).unwrap_err().contains("newer kind"));
        assert!(parse(&manifest("abc1234", "2026-10-16T08:00:00Z")).unwrap_err().contains("names no build"));
        assert!(parse(&manifest(B, "2026-10-16 08:00")).is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn team_comes_from_codesign() {
        let signed = "Executable=/Applications/Endeavor.app/Contents/MacOS/endeavor\nAuthority=Developer ID Application: X (AB12CD34EF)\nTeamIdentifier=AB12CD34EF\nSealed Resources version=2";
        assert_eq!(team_in(signed).as_deref(), Some("AB12CD34EF"));
        assert_eq!(team_in("Signature=adhoc\nTeamIdentifier=not set"), None);
        assert_eq!(team_in("code object is not signed at all"), None);
    }

    #[test]
    fn bundle_is_three_folders_up() {
        assert_eq!(bundle_of(Path::new("/Applications/Endeavor.app/Contents/MacOS/endeavor")), Some(PathBuf::from("/Applications/Endeavor.app")));
        assert_eq!(bundle_of(Path::new("/home/me/Endeavor/target/debug/endeavor")), None);
    }

    #[test]
    fn checks_are_daily_and_retried_hourly_after_a_failure() {
        let place = Place { app: PathBuf::from("/Applications/Endeavor.app"), replaceable: true };
        let mut u = Updater { place: Ok(place.clone()), state: State::Waiting, progress: Arc::default(), last_try: None };
        let t0 = Instant::now();
        assert!(u.due(t0));
        assert_eq!(u.start(t0), Some(place));
        assert!(!u.due(t0 + DAILY) && u.start(t0).is_none(), "one at a time");
        u.finish(Err("offline".into()));
        assert!(!u.due(t0 + RETRY / 2) && u.due(t0 + RETRY));
        u.start(t0 + RETRY);
        u.finish(Ok(Outcome::UpToDate));
        assert!(!u.due(t0 + RETRY * 2) && u.due(t0 + RETRY + DAILY));
        u.start(t0 + RETRY + DAILY);
        u.finish(Ok(Outcome::Ready { build: "bbbbbbb".into(), commit: B.into(), staged: PathBuf::from("/x/Endeavor.app") }));
        assert!(!u.due(t0 + DAILY * 9) && u.start(t0).is_none(), "nothing more once one is ready");
    }

    #[test]
    fn a_copy_that_cant_update_never_checks() {
        let mut u = Updater { place: Err("a build from source doesn't update itself"), state: State::Off("a build from source doesn't update itself"), progress: Arc::default(), last_try: None };
        assert!(!u.due(Instant::now()));
        assert_eq!(u.start(Instant::now()), None);
        assert_eq!(u.install(true), Ok(false));
    }

    fn plist(commit: &str, committed: &str) -> String {
        format!("<dict>\n  <key>CFBundleExecutable</key><string>endeavor</string>\n  <key>EndeavorCommit</key><string>{commit}</string>\n  <key>EndeavorCommitted</key><string>{committed}</string>\n</dict>")
    }

    #[test]
    fn the_new_app_must_say_it_is_the_newer_build_offered() {
        assert_eq!(is_build(&plist(B, "2026-10-16T08:00:00Z"), B, "2026-10-15T08:00:00Z"), Ok(()));
        // An older signed build that a changed manifest calls newer.
        assert!(is_build(&plist(B, "2026-10-14T08:00:00Z"), B, "2026-10-15T08:00:00Z").unwrap_err().contains("isn't newer"));
        assert!(is_build(&plist(A, "2026-10-16T08:00:00Z"), B, "2026-10-15T08:00:00Z").unwrap_err().contains("isn't the build"));
        assert!(is_build(&plist(B, ""), B, "2026-10-15T08:00:00Z").is_err());
        // A nightly from before the Info.plist carried its build.
        assert!(is_build("<dict><key>CFBundleExecutable</key><string>endeavor</string></dict>", B, "2026-10-15T08:00:00Z").unwrap_err().contains("doesn't say"));
        assert_eq!(plist_string("<key>EndeavorCommit</key>\n\t<string>x</string>", "EndeavorCommit"), Some("x"));
    }

    #[test]
    fn the_windows_install_folder_is_above_bin() {
        assert_eq!(install_of(Path::new("/u/AppData/Local/Programs/Endeavor/bin/endeavor.exe")), Some(PathBuf::from("/u/AppData/Local/Programs/Endeavor")));
        assert_eq!(install_of(Path::new("/u/Downloads/endeavor.exe")), None);
    }

    #[test]
    fn the_installer_is_fetched_once_and_checked() {
        let w = std::env::temp_dir().join(format!("endeavor-installer-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&w);
        let src = w.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("Endeavor-windows-x86_64-setup.exe"), b"installer").unwrap();
        let sha256 = crate::install::sha256_of(&src.join("Endeavor-windows-x86_64-setup.exe")).unwrap();
        let offer = Offer { commit: B.into(), file: File { name: "Endeavor-windows-x86_64-setup.exe".into(), sha256, size: 9 } };
        let dir = w.join("update");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{A}-setup.exe")), b"older").unwrap();
        let base = format!("file://{}{}", if cfg!(windows) { "/" } else { "" }, src.display().to_string().replace('\\', "/"));
        let setup = fetch_installer(&base, &dir, &offer, &|_| {}).unwrap();
        assert_eq!(setup, dir.join(format!("{B}-setup.exe")));
        assert!(!dir.join(format!("{A}-setup.exe")).exists(), "another build's installer goes");
        // Kept, not fetched again, once it's there.
        std::fs::remove_file(src.join("Endeavor-windows-x86_64-setup.exe")).unwrap();
        assert_eq!(fetch_installer(&base, &dir, &offer, &|_| {}).unwrap(), setup);
        // A changed file is refused and deleted.
        std::fs::write(src.join("Endeavor-windows-x86_64-setup.exe"), b"installed").unwrap();
        std::fs::remove_file(&setup).unwrap();
        assert!(fetch_installer(&base, &dir, &offer, &|_| {}).unwrap_err().contains("didn't match"));
        assert!(!setup.exists() && !dir.join(format!("{B}-setup.exe.part")).exists());
        std::fs::remove_dir_all(&w).unwrap();
    }

    #[test]
    fn the_next_launch_says_when_the_last_install_didnt_take() {
        let dir = std::env::temp_dir().join(format!("endeavor-marker-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!last_install_failed(&dir, A), "nothing was installing");
        std::fs::write(dir.join(INSTALLING), B).unwrap();
        assert!(!last_install_failed(&dir, B), "this is the build it installed");
        assert!(!dir.join(INSTALLING).exists());
        std::fs::write(dir.join(INSTALLING), B).unwrap();
        assert!(last_install_failed(&dir, A), "still the old build");
        assert!(!last_install_failed(&dir, A), "said once");
        std::fs::write(dir.join(FAILED), "").unwrap();
        assert!(last_install_failed(&dir, A), "the Mac swap's marker");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn this_build_records_its_commit_and_time() {
        assert!(COMMIT.is_empty() || COMMIT.len() == 40, "{COMMIT}");
        assert!(COMMITTED.is_empty() || utc_time(COMMITTED), "{COMMITTED}");
    }

    /// The real fetch, checks and swap on a Mac, against the nightly published
    /// now, all in a scratch folder (nothing in Applications or ~/Library):
    ///   cargo test --locked updater::tests::real_update_on_a_mac -- --ignored --nocapture
    /// Needs the network (the download, and `spctl`'s notarization lookup).
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn real_update_on_a_mac() {
        use std::io::Write as _;
        let w = std::env::temp_dir().join(format!("endeavor-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&w);
        let src = w.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let name = "Endeavor-macos-arm64.dmg";
        let dmg = src.join(name);
        crate::install::download(&format!("{RELEASE}/{name}"), &dmg, "the nightly", 1, &|_, _| {}).unwrap();
        let size = std::fs::metadata(&dmg).unwrap().len();
        let sha256 = crate::install::sha256_of(&dmg).unwrap();

        // The "installed" app: the same nightly, copied out by hand.
        let apps = w.join("apps");
        std::fs::create_dir_all(&apps).unwrap();
        let mount = w.join("mnt");
        run(Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"]).arg(&mount).arg(&dmg), "attach").unwrap();
        run(Command::new("ditto").arg(mount.join(APP)).arg(apps.join(APP)), "copy").unwrap();
        run(Command::new("hdiutil").arg("detach").arg(&mount), "detach").unwrap();
        let installed = apps.join(APP);
        let team = team_of(&installed).expect("the nightly is Developer ID signed");
        assert!(run(Command::new("codesign").args(["--verify", "-R"]).arg(requirement(&team)).arg(&dmg), "check").is_ok(), "the disk image passes");
        assert!(run(Command::new("codesign").args(["--verify", "-R"]).arg(requirement("ZZZZZZZZZZ")).arg(&dmg), "check").is_err(), "another team's requirement fails");

        let offer = Offer { commit: "c".repeat(40), file: File { name: name.into(), sha256: sha256.clone(), size } };
        let base = |dir: &Path| format!("file://{}", dir.display());

        // A disk image with one byte changed is refused by its hash.
        let bad = w.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        let mut bytes = std::fs::read(&dmg).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 1;
        std::fs::write(bad.join(name), bytes).unwrap();
        // fetch() makes the folder before stage() runs.
        std::fs::create_dir_all(w.join("staging-bad")).unwrap();
        let err = stage(&base(&bad), &w.join("staging-bad"), &offer, &team, &|_| {}).unwrap_err();
        assert!(err.contains("didn't match"), "{err}");

        // The real one: hash, disk image signature, copy, app signature, notarization.
        let staging = w.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let staged = stage(&base(&src), &staging, &offer, &team, &|_| {}).unwrap();
        assert!(verify(&staged, "ZZZZZZZZZZ").is_err(), "another team is refused");

        // Its Info.plist: refused as the wrong build either way; accepted as
        // itself once the nightly carries its build (the first with app #80's fix).
        assert!(same_build(&staged, &offer.commit, "2000-01-01T00:00:00Z").is_err());
        let plist = std::fs::read_to_string(staged.join("Contents/Info.plist")).unwrap();
        match plist_string(&plist, "EndeavorCommit") {
            Some(commit) => same_build(&staged, commit, "2000-01-01T00:00:00Z").unwrap(),
            None => eprintln!("note: this nightly's Info.plist doesn't name its build yet, so fetch() would refuse it"),
        }

        // A changed file inside the app breaks its signature.
        let tampered = w.join("tampered").join(APP);
        run(Command::new("ditto").arg(&staged).arg(&tampered), "copy").unwrap();
        std::fs::OpenOptions::new().append(true).open(tampered.join("Contents/Resources/Endeavor.icns")).unwrap().write_all(b"x").unwrap();
        assert!(verify(&tampered, &team).is_err(), "a changed app is refused");

        // The swap, once a stand-in for the app has exited.
        let failed = w.join(FAILED);
        let mut app = Command::new("sleep").arg("1").spawn().unwrap();
        let mut script = swap(&staged, &installed, app.id(), false, &failed).unwrap();
        app.wait().unwrap();
        script.wait().unwrap();
        verify(&installed, &team).unwrap();
        assert!(!failed.exists() && !staged.exists());
        assert!(!apps.join("Endeavor.app.next").exists() && !apps.join("Endeavor.app.replaced").exists());

        // On another volume with no room for the new app, the copy fails and
        // the old app stays as it was.
        let image = w.join("small.dmg");
        run(Command::new("hdiutil").args(["create", "-size", "16m", "-fs", "APFS", "-volname", "EndeavorSwapTest"]).arg(&image), "create").unwrap();
        let vol = w.join("vol");
        run(Command::new("hdiutil").args(["attach", "-nobrowse", "-noautoopen", "-mountpoint"]).arg(&vol).arg(&image), "attach").unwrap();
        let small = vol.join(APP);
        std::fs::create_dir_all(small.join("Contents")).unwrap();
        std::fs::write(small.join("Contents/Info.plist"), "old").unwrap();
        let mut app = Command::new("true").spawn().unwrap();
        let mut script = swap(&installed, &small, app.id(), false, &failed).unwrap();
        app.wait().unwrap();
        script.wait().unwrap();
        let old = std::fs::read_to_string(small.join("Contents/Info.plist"));
        let left: Vec<_> = std::fs::read_dir(&vol).unwrap().flatten().map(|e| e.file_name()).collect();
        let _ = run(Command::new("hdiutil").args(["detach", "-force"]).arg(&vol), "detach");
        assert_eq!(old.as_deref().ok(), Some("old"), "the old app is untouched");
        assert!(failed.exists(), "the next launch reports it");
        assert!(left.iter().all(|n| !n.to_string_lossy().contains(".next")), "{left:?}");

        std::fs::remove_dir_all(&w).unwrap();
    }
}
