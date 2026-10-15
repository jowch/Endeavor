//! A nightly Mac app updating itself to the newest nightly (app #80). The nightly workflow publishes
//! `nightly.json` beside the dmg (scripts/nightly-manifest.sh). Once a day,
//! and on Check now, the app reads it; when it names a build committed after
//! this one, the app downloads the dmg, checks its SHA-256 against the
//! manifest, copies Endeavor.app out of it, and checks that copy's signature:
//! the same Developer ID team as the running app, and notarized. The hash only
//! proves the file is the one the manifest names (both come from the same
//! release); the signature is what proves it is ours. Restart, or any quit,
//! then swaps the apps once this one has exited.
//!
//! Only a build made by the nightly workflow (`ENDEAVOR_CHANNEL=nightly`,
//! build.rs) running from a signed Endeavor.app checks. A build from source,
//! the Windows installer and Linux never do. When the app's folder isn't
//! writable (not in Applications, or run from the disk image), the newer build
//! is offered as a download instead.

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
const PLATFORM: &str = "macos-arm64";
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
    if !cfg!(target_os = "macos") {
        return Err("only the Mac app updates itself");
    }
    if CHANNEL != "nightly" {
        return Err("a build from source doesn't update itself");
    }
    let exe = std::env::current_exe().map_err(|_| "can't tell where the app is")?;
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
    Ready { build: String, staged: PathBuf },
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
        let state = match place {
            Ok(_) => State::Waiting,
            Err(why) => State::Off(why),
        };
        Updater { place, state, progress: Arc::default(), last_try: None }
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
            Ok(Outcome::Ready { build, staged }) => State::Ready { build, staged },
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
        let (State::Ready { staged, .. }, Ok(place)) = (&self.state, &self.place) else { return Ok(false) };
        install(staged, &place.app, relaunch)?;
        self.state = State::Installing;
        Ok(true)
    }
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    /// Downloaded and checked, waiting for a restart.
    Ready { build: String, staged: PathBuf },
    /// Newer, but the app can't replace itself where it is.
    Manual { build: String },
    /// This copy of the app isn't one that can update (not Developer ID signed).
    Off(&'static str),
}

/// Check, and fetch a newer build when there is one. Blocking; `progress`
/// gets the download's fraction.
pub fn check(place: &Place, progress: &dyn Fn(f32)) -> Result<Outcome, String> {
    let m = parse(&manifest()?)?;
    let Some(offer) = offer(&m, COMMIT, COMMITTED) else { return Ok(Outcome::UpToDate) };
    if !place.replaceable {
        return Ok(Outcome::Manual { build: offer.build() });
    }
    let Some(team) = team_of(&place.app) else { return Ok(Outcome::Off("this copy isn't signed with a Developer ID")) };
    let staged = fetch(&offer, &team, progress)?;
    Ok(Outcome::Ready { build: offer.build(), staged })
}

fn manifest() -> Result<String, String> {
    #[cfg(debug_assertions)]
    if let Some(file) = test_manifest() {
        return std::fs::read_to_string(&file).map_err(|e| format!("couldn't read {} ({e})", file.display()));
    }
    let mut curl = Command::new("curl");
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

/// Download the offer's disk image, check it, and copy a checked app out of
/// it. Returns the copy's path. What other builds left in the folder goes.
fn fetch(offer: &Offer, team: &str, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    let dir = folder()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(&offer.commit) {
            let _ = std::fs::remove_dir_all(entry.path()).or_else(|_| std::fs::remove_file(entry.path()));
        }
    }
    let staged = dir.join(&offer.commit).join(APP);
    if staged.is_dir() && verify(&staged, team).is_ok() {
        return Ok(staged);
    }

    let dmg = dir.join(format!("{}.dmg.part", offer.commit));
    let url = format!("{RELEASE}/{}", offer.file.name);
    crate::install::download(&url, &dmg, "the update", offer.file.size, &|_, f| progress(f.unwrap_or(0.)))?;
    let got = crate::install::sha256_of(&dmg)?;
    if got != offer.file.sha256 {
        let _ = std::fs::remove_file(&dmg);
        return Err("the download didn't match the update list; it was deleted and is tried again later".into());
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

/// The new app is signed by the same Developer ID team as this one, and
/// notarized.
fn verify(app: &Path, team: &str) -> Result<(), String> {
    let requirement = format!("=anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
    run(Command::new("codesign").args(["--verify", "--strict", "--deep", "-R"]).arg(requirement).arg(app), "check the new app's signature")?;
    run(Command::new("spctl").args(["--assess", "--type", "execute"]).arg(app), "check that the new app is notarized")
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("couldn't {what} ({e})"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("couldn't {what} ({})", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Swap the apps once this one has exited, and start the new one when
/// `relaunch`. A detached shell does it, since this process has to be gone
/// first; if the second rename fails, the old app goes back.
pub fn install(staged: &Path, app: &Path, relaunch: bool) -> Result<(), String> {
    const SCRIPT: &str = r#"
pid=$1 app=$2 new=$3 open=$4
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
old="$app.replaced"
rm -rf "$old"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then rm -rf "$old"; else mv "$old" "$app"; fi
fi
[ -z "$open" ] || open "$app"
"#;
    Command::new("/bin/sh")
        .args(["-c", SCRIPT, "sh"])
        .arg(std::process::id().to_string())
        .arg(app)
        .arg(staged)
        .arg(if relaunch { "open" } else { "" })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("couldn't start the update ({e})"))
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
        u.finish(Ok(Outcome::Ready { build: "bbbbbbb".into(), staged: PathBuf::from("/x/Endeavor.app") }));
        assert!(!u.due(t0 + DAILY * 9) && u.start(t0).is_none(), "nothing more once one is ready");
    }

    #[test]
    fn a_copy_that_cant_update_never_checks() {
        let mut u = Updater { place: Err("a build from source doesn't update itself"), state: State::Off("a build from source doesn't update itself"), progress: Arc::default(), last_try: None };
        assert!(!u.due(Instant::now()));
        assert_eq!(u.start(Instant::now()), None);
        assert_eq!(u.install(true), Ok(false));
    }

    #[test]
    fn this_build_records_its_commit_and_time() {
        assert!(COMMIT.is_empty() || COMMIT.len() == 40, "{COMMIT}");
        assert!(COMMITTED.is_empty() || utc_time(COMMITTED), "{COMMITTED}");
    }
}
