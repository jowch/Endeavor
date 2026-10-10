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
use std::sync::atomic::{AtomicUsize, Ordering};

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
        if skipped_callback(record) {
            // Windows delivers window messages re-entrantly while the app is busy; GPUI skips
            // that one callback and the next frame catches up. Say so once, then only count them.
            if SKIPPED.fetch_add(1, Ordering::Relaxed) == 0 {
                eprintln!("GPUI skipped a window callback because the app was busy (harmless; later ones are counted, not logged)");
            }
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

/// Window callbacks GPUI skipped this run because the app was already borrowed.
static SKIPPED: AtomicUsize = AtomicUsize::new(0);

pub fn skipped_callbacks() -> usize {
    SKIPPED.load(Ordering::Relaxed)
}

/// GPUI's `Window::new` wires each platform callback to `handle.update(..).log_err()`, which
/// logs the `BorrowMutError` when the message arrives during an update. Only that exact error
/// from GPUI's window.rs, so a different borrow error or any other GPUI error still shows.
fn skipped_callback(record: &log::Record) -> bool {
    let from_window = record.file().is_some_and(|f| {
        let f = f.replace('\\', "/");
        f.contains("gpui") && f.ends_with("/src/window.rs")
    });
    record.level() == log::Level::Error && from_window && record.args().to_string() == "RefCell already borrowed"
}

/// Show the log file in Finder (on Linux, its folder in the file manager).
pub fn reveal() {
    if let Some(path) = path() {
        crate::platform::reveal(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(file: &str, message: &str, check: impl FnOnce(&log::Record) -> bool) -> bool {
        check(&log::Record::builder().level(log::Level::Error).target("").file(Some(file)).line(Some(1843)).args(format_args!("{message}")).build())
    }

    #[test]
    fn only_gpuis_skipped_window_callbacks_are_dropped() {
        let unix = "/home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-pre-0.3.6/src/window.rs";
        let windows = r"C:\Users\u\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\gpui-pre-0.3.6\src\window.rs";
        assert!(record(unix, "RefCell already borrowed", skipped_callback));
        assert!(record(windows, "RefCell already borrowed", skipped_callback));
        // Any other GPUI error, or the same error from elsewhere, still comes through.
        assert!(!record(unix, "app is quitting", skipped_callback));
        assert!(!record("/home/u/.cargo/registry/src/x/gpui-pre-0.3.6/src/app.rs", "RefCell already borrowed", skipped_callback));
        assert!(!record("src/window.rs", "RefCell already borrowed", skipped_callback));
    }
}
