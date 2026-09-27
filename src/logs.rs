//! The app's log file. Launched from Finder, stdout and stderr (our messages,
//! Julia's log we echo, and child processes that inherit them) go to
//! ~/Library/Logs/Endeavor/endeavor.log (on Linux, $XDG_STATE_HOME/endeavor/endeavor.log);
//! the previous run's is kept as endeavor.old.log.
//! From a terminal, output stays in the terminal.

use std::io::IsTerminal;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

pub fn path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").ok()?);
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Logs/Endeavor/endeavor.log"));
    }
    let state = std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()).map_or_else(|| home.join(".local/state"), PathBuf::from);
    Some(state.join("endeavor/endeavor.log"))
}

pub fn start() {
    if std::io::stderr().is_terminal() {
        return;
    }
    let Some(path) = path() else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::rename(&path, path.with_file_name("endeavor.old.log"));
    // ponytail: no rotation within a run; a very long run makes a big file.
    let Ok(file) = std::fs::File::create(&path) else { return };
    // SAFETY: dup2 onto the standard fds; `file` stays open until the calls return.
    unsafe {
        libc::dup2(file.as_raw_fd(), 1);
        libc::dup2(file.as_raw_fd(), 2);
    }
    eprintln!("Endeavor {} started", env!("CARGO_PKG_VERSION"));
}

/// Show the log file in Finder (on Linux, its folder in the file manager).
pub fn reveal() {
    if let Some(path) = path() {
        crate::platform::reveal(&path);
    }
}
