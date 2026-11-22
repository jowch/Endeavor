//! A debug build's last few clicks, as each layer saw them: the window's
//! routing around the notebook's web view (overlay.rs), GPUI's root, and the
//! server dialog's buttons. In the state dump as `.window.clicks`, for a
//! check of a click that the app shows but doesn't act on (#282).

use std::collections::VecDeque;
use std::sync::Mutex;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const KEPT: usize = 24;

/// Record `what`, in a debug build only.
pub fn note(what: impl FnOnce() -> String) {
    if !cfg!(debug_assertions) {
        return;
    }
    let mut log = LOG.lock().unwrap();
    if log.len() == KEPT {
        log.pop_front();
    }
    log.push_back(what());
}

/// The clicks noted, oldest first.
pub fn all() -> Vec<String> {
    LOG.lock().unwrap().iter().cloned().collect()
}
