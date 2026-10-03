//! The runtime helper a macOS server is sent, installed there as `endeavor`
//! (scripts/bundle.sh puts it in the app's `helpers/darwin-<arch>/`). The app's
//! own binary already has that name here. This Mac runs the app itself as its
//! helper; tests run this one, since a test binary can't be the helper.

fn main() {
    endeavor_mcp::run(std::env::args().skip(1).collect())
}
