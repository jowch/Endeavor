//! The app's log file. Launched from Finder, stdout and stderr (our messages,
//! Julia's log we echo, and child processes that inherit them) go to
//! ~/Library/Logs/Endeavor/endeavor.log (on Linux, $XDG_STATE_HOME/endeavor/endeavor.log;
//! on Windows, %LOCALAPPDATA%\Endeavor\Logs\endeavor.log);
//! the previous run's is kept as endeavor.old.log.
//! From a terminal, output stays in the terminal, except on Windows, where the
//! app is a GUI program with no console and its output always goes to the file.

use std::io::IsTerminal;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::PathBuf;

pub fn path() -> Option<PathBuf> {
    if cfg!(windows) {
        return crate::install::app_dir().ok().map(|dir| dir.join("Logs").join("endeavor.log"));
    }
    let home = PathBuf::from(std::env::var("HOME").ok()?);
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Logs/Endeavor/endeavor.log"));
    }
    let state = std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()).map_or_else(|| home.join(".local/state"), PathBuf::from);
    Some(state.join("endeavor/endeavor.log"))
}

pub fn start() {
    // GPUI's own warnings and errors, which otherwise go nowhere.
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    if std::io::stderr().is_terminal() {
        return;
    }
    let Some(path) = path() else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::rename(&path, path.with_file_name("endeavor.old.log"));
    // ponytail: no rotation within a run; a very long run makes a big file.
    // On Windows, appending keeps the children's writes from overwriting each other.
    let file = if cfg!(windows) { std::fs::OpenOptions::new().create(true).append(true).open(&path) } else { std::fs::File::create(&path) };
    let Ok(file) = file else { return };
    // SAFETY: dup2 onto the standard fds; `file` stays open until the calls return.
    #[cfg(unix)]
    unsafe {
        libc::dup2(file.as_raw_fd(), 1);
        libc::dup2(file.as_raw_fd(), 2);
    }
    // The app is a GUI program there, with no console: the file becomes its
    // standard output and error, which Rust looks up on each write and the
    // children it starts with inherited output receive. The handles stay open
    // for the app's life.
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
        let Ok(out) = file.try_clone() else { return };
        // SAFETY: valid handles, which the process now owns.
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, out.into_raw_handle());
            SetStdHandle(STD_ERROR_HANDLE, file.into_raw_handle());
        }
    }
    eprintln!("Endeavor {} started", env!("CARGO_PKG_VERSION"));
}

/// Writes log records (GPUI's) to stderr, which is the log file once `start` has run.
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // GPUI's `log_err` leaves the target empty outside Zed's own tree; its file and line say where.
        match (record.target(), record.file(), record.line()) {
            ("", Some(file), Some(line)) => eprintln!("{} {file}:{line}: {}", record.level(), record.args()),
            (target, ..) => eprintln!("{} {target}: {}", record.level(), record.args()),
        }
    }

    fn flush(&self) {}
}

/// Show the log file in Finder (on Linux, its folder in the file manager).
pub fn reveal() {
    if let Some(path) = path() {
        crate::platform::reveal(&path);
    }
}
