# gpui-wry, vendored

The notebook's web view sits in GPUI through this crate. It is a copy of
[gpui-wry](https://crates.io/crates/gpui-wry) 0.6.6 from Longbridge's
[gpui-kit](https://github.com/longbridge/gpui-kit) (`crates/webview`, Apache-2.0).
Every gpui-wry release pins one exact gpui-pre version, and upstream renamed the crate
gpui-webview and rewrote it (native-view clipping, GPUI Fast composition) after 0.7.
This copy keeps the 0.6.6 behaviour the app's overlay holes, focus and Linux glue are
built around, and lets the GPUI pin in `Cargo.toml` move with the app's.

`src/lib.rs` is the upstream file, byte for byte, from the published crate
`gpui-wry-0.6.6.crate` (sha256
`529390779b0b86f97444b0512e4d5605e755f16210a53617eaa7c54bee21a4d0`, the checksum
`Cargo.lock` recorded for it). Keep it unchanged unless a GPUI upgrade needs a change,
and say here what changed.

This folder is under the Apache License 2.0 (`LICENSE-APACHE`, copied from gpui-kit),
not the app's license.

To upgrade GPUI, change the `gpui-pre` version here and in the app's `Cargo.toml` together.
