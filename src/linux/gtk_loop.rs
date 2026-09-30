//! GTK's events, run from GPUI's main loop. The app can't add GLib's file
//! descriptors to GPUI's own event loop, so it hosts GLib's main context the
//! way GLib lets another loop do it: the main thread prepares, checks and
//! dispatches, and a helper thread only waits in poll() on GLib's descriptors,
//! so an idle app doesn't wake up.

use std::future::Future;
use std::os::fd::RawFd;
use std::pin::Pin;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::task::{Context, Poll};

use futures::channel::oneshot;
use gpui::App;
use gtk::glib::ffi::{self, GPollFD};

/// The longest a wait lasts, for work poll() can't see (see `wake`).
const MAX_WAIT_MS: i32 = 250;

/// An eventfd the helper's poll() also waits on.
static WAKE: OnceLock<RawFd> = OnceLock::new();

/// Stop the wait. GLib wakes its own poll when another thread adds work, but
/// not when the main thread does, which is where GPUI's calls into GTK run
/// (moving the web view, loading a page), and X events Xlib has already read
/// don't make its descriptor readable. So the app calls this after each frame.
pub fn wake() {
    if let Some(&fd) = WAKE.get() {
        let one: u64 = 1;
        unsafe { libc::write(fd, (&one as *const u64).cast(), 8) };
    }
}

struct Wait {
    fds: Vec<GPollFD>,
    timeout: i32,
    done: oneshot::Sender<Vec<GPollFD>>,
}

fn poll_forever(wake: RawFd, waits: mpsc::Receiver<Wait>) {
    for Wait { mut fds, timeout, done } in waits {
        let mut polled: Vec<libc::pollfd> = fds.iter().map(|f| libc::pollfd { fd: f.fd, events: f.events as i16, revents: 0 }).collect();
        polled.push(libc::pollfd { fd: wake, events: libc::POLLIN, revents: 0 });
        unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as libc::nfds_t, timeout) };
        if polled.last().is_some_and(|w| w.revents != 0) {
            let mut count = 0u64;
            unsafe { libc::read(wake, (&mut count as *mut u64).cast(), 8) };
        }
        for (f, p) in fds.iter_mut().zip(&polled) {
            f.revents = p.revents as u16;
        }
        let _ = done.send(fds);
    }
}

/// Lets the rest of GPUI's main loop run between GLib iterations.
struct YieldNow(bool);

impl Future for YieldNow {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if std::mem::replace(&mut self.0, true) {
            return Poll::Ready(());
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Run GLib's default main context from now on. `gtk::init` has made the main
/// thread its owner.
pub fn start(cx: &mut App) {
    let wake = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if wake < 0 || WAKE.set(wake).is_err() {
        return;
    }
    let (waits, requests) = mpsc::channel::<Wait>();
    std::thread::Builder::new().name("gtk-poll".into()).spawn(move || poll_forever(wake, requests)).expect("GTK's poll thread");
    cx.spawn(async move |_| {
        let context = unsafe { ffi::g_main_context_default() };
        let mut fds: Vec<GPollFD> = Vec::new();
        loop {
            let mut priority = 0;
            let ready = unsafe { ffi::g_main_context_prepare(context, &mut priority) } != 0;
            let mut timeout = 0;
            loop {
                let n = unsafe { ffi::g_main_context_query(context, priority, &mut timeout, fds.as_mut_ptr(), fds.len() as i32) } as usize;
                let fits = n <= fds.len();
                fds.resize(n, GPollFD { fd: -1, events: 0, revents: 0 });
                if fits {
                    break;
                }
            }
            if !ready && timeout != 0 {
                let timeout = if timeout < 0 { MAX_WAIT_MS } else { timeout.min(MAX_WAIT_MS) };
                let (done, polled) = oneshot::channel();
                if waits.send(Wait { fds, timeout, done }).is_err() {
                    return;
                }
                let Ok(polled) = polled.await else { return };
                fds = polled;
            }
            unsafe {
                if ffi::g_main_context_check(context, priority, fds.as_mut_ptr(), fds.len() as i32) != 0 {
                    ffi::g_main_context_dispatch(context);
                }
            }
            YieldNow(false).await;
        }
    })
    .detach();
}
