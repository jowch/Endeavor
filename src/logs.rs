use std::io::IsTerminal;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::collections::BTreeMap;
use std::sync::Mutex;

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
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    if std::io::stderr().is_terminal() {
        return;
    }
    let Some(path) = path() else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::rename(&path, path.with_file_name("endeavor.old.log"));
    let file = if cfg!(windows) { std::fs::OpenOptions::new().create(true).append(true).open(&path) } else { std::fs::File::create(&path) };
    let Ok(file) = file else { return };
    // SAFETY: dup2 onto the standard fds; `file` stays open until the calls return.
    #[cfg(unix)]
    unsafe {
        libc::dup2(file.as_raw_fd(), 1);
        libc::dup2(file.as_raw_fd(), 2);
    }
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

struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Some(count) = count_skip(&SKIPPED, record) {
            if count == 1 {
                eprintln!("{} {}:{}: GPUI skipped a window callback because the app was busy; later ones from this line are counted, not logged", record.level(), record.file().unwrap_or_default(), record.line().unwrap_or_default());
            }
            return;
        }
        match (record.target(), record.file(), record.line()) {
            ("", Some(file), Some(line)) => eprintln!("{} {file}:{line}: {}", record.level(), record.args()),
            (target, ..) => eprintln!("{} {target}: {}", record.level(), record.args()),
        }
    }

    fn flush(&self) {}
}

static SKIPPED: Mutex<BTreeMap<u32, usize>> = Mutex::new(BTreeMap::new());

pub fn skipped_callbacks() -> BTreeMap<u32, usize> {
    SKIPPED.lock().map(|counts| counts.clone()).unwrap_or_default()
}

fn count_skip(counts: &Mutex<BTreeMap<u32, usize>>, record: &log::Record) -> Option<usize> {
    if !skipped_callback(record) {
        return None;
    }
    let mut counts = counts.lock().ok()?;
    let count = counts.entry(record.line().unwrap_or_default()).or_default();
    *count += 1;
    Some(*count)
}

fn skipped_callback(record: &log::Record) -> bool {
    let from_window = record.file().is_some_and(|f| {
        let f = f.replace('\\', "/");
        f.contains("gpui") && f.ends_with("/src/window.rs")
    });
    let message = record.args().to_string();
    let busy = message == "RefCell already borrowed" || message.contains(": RefCell already borrowed: attempted mutable App borrow at ");
    record.level() == log::Level::Error && from_window && busy
}

pub fn reveal() {
    if let Some(path) = path() {
        crate::platform::reveal(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record<T>(file: &str, line: u32, message: &str, check: impl FnOnce(&log::Record) -> T) -> T {
        check(&log::Record::builder().level(log::Level::Error).target("").file(Some(file)).line(Some(line)).args(format_args!("{message}")).build())
    }

    #[test]
    fn only_gpuis_skipped_window_callbacks_are_dropped() {
        let unix = "/home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-pre-0.3.6/src/window.rs";
        let windows = r"C:\Users\u\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\gpui-pre-0.3.6\src\window.rs";
        assert!(record(unix, 1811, "RefCell already borrowed", skipped_callback));
        assert!(record(windows, 1811, "RefCell already borrowed", skipped_callback));
        assert!(!record(unix, 1811, "app is quitting", skipped_callback));
        assert!(!record("/home/u/.cargo/registry/src/x/gpui-pre-0.3.6/src/app.rs", 1811, "RefCell already borrowed", skipped_callback));
        assert!(!record("src/window.rs", 1811, "RefCell already borrowed", skipped_callback));
        let windows = r"C:\Users\u\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\gpui-pre-0.3.11\src\window.rs";
        let busy = r"drawing and presenting frame for window WindowId(1v1): RefCell already borrowed: attempted mutable App borrow at C:\x\gpui-pre-0.3.11\src\window.rs:2029:30; existing mutable borrow group began at C:\x\gpui-pre-0.3.11\src\app.rs:319:33";
        assert!(record(windows, 2049, busy, skipped_callback));
        assert!(!record(windows, 2049, &busy.replace("attempted mutable", "attempted shared"), skipped_callback));
    }

    #[test]
    fn skipped_window_callbacks_are_logged_once_per_line() {
        let unix = "/home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-pre-0.3.6/src/window.rs";
        let counts = Mutex::new(BTreeMap::new());
        let skip = |line| record(unix, line, "RefCell already borrowed", |r| count_skip(&counts, r));
        assert_eq!(skip(1811), Some(1));
        assert_eq!(skip(1811), Some(2));
        assert_eq!(skip(1943), Some(1));
        assert_eq!(skip(1811), Some(3));
        assert_eq!(record(unix, 1811, "app is quitting", |r| count_skip(&counts, r)), None);
        assert_eq!(*counts.lock().unwrap(), BTreeMap::from([(1811, 3), (1943, 1)]));
    }
}
