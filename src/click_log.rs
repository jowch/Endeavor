use std::collections::VecDeque;
use std::sync::Mutex;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const KEPT: usize = 24;

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

pub fn all() -> Vec<String> {
    LOG.lock().unwrap().iter().cloned().collect()
}
