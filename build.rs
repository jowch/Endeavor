//! The build number shown in About Endeavor: the short hash of the commit built.
//! Also what the nightly updater compares (src/updater.rs): the full hash, the
//! commit's committer time, and the channel (`ENDEAVOR_CHANNEL=nightly` from
//! the nightly workflow; anything else is a build that doesn't update itself).

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).env("TZ", "UTC").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn main() {
    let hash = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=ENDEAVOR_BUILD={hash}");
    println!("cargo:rustc-env=ENDEAVOR_COMMIT={}", git(&["rev-parse", "HEAD"]).unwrap_or_default());
    // The same field and format as scripts/nightly-manifest.sh's COMMITTED.
    println!("cargo:rustc-env=ENDEAVOR_COMMITTED={}", git(&["log", "-1", "--date=format-local:%Y-%m-%dT%H:%M:%SZ", "--format=%cd"]).unwrap_or_default());
    println!("cargo:rustc-env=ENDEAVOR_CHANNEL={}", std::env::var("ENDEAVOR_CHANNEL").unwrap_or_else(|_| "dev".into()));
    println!("cargo:rerun-if-env-changed=ENDEAVOR_CHANNEL");
    // A worktree has its own git dir; logs/HEAD changes on every commit and checkout.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/logs/HEAD");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
