# How a nightly updates itself

A nightly build updates itself to the newest nightly (#80), in
`src/updater.rs`. The nightly workflow publishes `nightly.json` beside its
builds (`scripts/nightly-manifest.sh`). Once a day, and on Check now, the
app reads it. A check that failed (offline, or the release mid-publish) is
tried again an hour later.

## Which builds check

Only a build made by the nightly workflow (`ENDEAVOR_CHANNEL=nightly`,
`build.rs`) checks: on a Mac from a signed Endeavor.app, on Windows from
where the installer put it. A build from source and Linux never do. When the
app can't replace itself (a Mac app not in Applications or run from the disk
image, a Windows build unzipped by hand), the newer build is offered as a
download instead.

The manifest names each build by its commit and its committer time, written
as `2026-10-15T08:12:00Z` on both sides (`build.rs` and the manifest
script's `COMMITTED`) so two times compare as strings.
Only a build committed after this one is offered, so a manifest that lags,
or a build newer than the nightly, never "updates" backwards. A build with no
recorded time (not from git) never compares newer.

## On a Mac

The app downloads the disk image and checks, in order:

1. its SHA-256 against the manifest. This only proves the file is the one the
   manifest names, since both come from the same release;
2. the disk image's own signature, so one that isn't ours is never mounted;
3. the copied app's signature: a Developer ID Application certificate of the
   same team as the running app (not the team's development or App Store
   certificates, which `spctl` would refuse but nothing else here would once
   Gatekeeper is off), and notarized;
4. the copied app's Info.plist, sealed by that signature
   (`scripts/bundle.sh`): it must name the manifest's commit, committed after
   this build. The manifest isn't signed, so this is what stops a changed
   manifest offering an older signed build.

The app is copied out with `ditto`, which keeps its signature and
attributes. Restart, or any quit, then swaps the apps once this one has
exited, from a detached shell. The new app is first copied beside the old
one, so both renames stay on one volume and are atomic: a failed copy leaves
the old app untouched, and a failed rename puts it back. Either leaves a
marker for the next launch to report.

`cargo test --locked updater::tests::real_update_on_a_mac -- --ignored --nocapture`
runs the real fetch, checks and swap against the nightly published now, all
in a scratch folder (nothing in Applications or ~/Library). It needs the
network, for the download and `spctl`'s notarization lookup.

## On Windows

The app downloads the installer and checks its SHA-256. The installer isn't
signed (#12), so the hash is the only check: a nightly Windows user trusts
the release as much as when they first installed from it, and someone who
can change the release could ship any program, not just an older one.

Restart, or any quit, runs the installer silently once the app has exited
(`/UPDATE` makes it wait, `scripts/installer.iss`), detached from the app's
console and process group so it outlives the app. Silent, Setup's Restart
Manager would close any program holding one of the app's files (an editor,
a terminal) without asking; `/NOCLOSEAPPLICATIONS` makes it give up and roll
back instead, and the next launch says the update didn't take. The
installer's log is kept for whoever looks into a failed update.

A quit because Windows is logging off or shutting down (GPUI runs the quit
callbacks then) leaves the build ready for the next quit: Windows kills what
is still running, and an installer killed partway can leave a broken app.
