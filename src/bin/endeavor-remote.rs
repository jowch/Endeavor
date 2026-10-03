//! The runtime helper a macOS server is sent. This Mac runs the app itself as
//! its helper; tests run this one, since a test binary can't be the helper.

fn main() {
    endeavor_remote::run(std::env::args().skip(1).collect())
}
