//! Whether this computer has a network path, from the system: Network
//! framework's path monitor on macOS, route changes over netlink on Linux.
//! Offline is waiting, not failure: the app holds Claude's messages and
//! reconnects servers once a path is back.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

/// Every change in whether the network is reachable (true: online), starting
/// with the current state.
pub fn watch() -> UnboundedReceiver<bool> {
    let (tx, rx) = unbounded();
    let system = unbounded::<bool>();
    platform::monitor(system.0);
    std::thread::spawn(move || combine(system.1, tx));
    rx
}

/// Try now: can Claude's API be reached this moment? DNS and a TCP connect, a few seconds at most.
pub fn probe() -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    if forced_offline() {
        return false;
    }
    let Ok(addrs) = ("api.anthropic.com", 443).to_socket_addrs() else { return false };
    addrs.take(2).any(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok())
}

/// Debug builds only: offline while the file `ENDEAVOR_FORCE_OFFLINE` names
/// exists, whatever the system says, to test offline without cutting the network.
fn forced_offline() -> bool {
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("ENDEAVOR_FORCE_OFFLINE") {
        return std::path::Path::new(&file).exists();
    }
    false
}

/// The system's answers, with the debug override, sent on when they change.
fn combine(system: UnboundedReceiver<bool>, tx: UnboundedSender<bool>) {
    use futures::StreamExt;
    use std::time::Duration;
    let mut system = system;
    let mut online = true;
    let mut sent: Option<bool> = None;
    // The override is a file, so it is looked at twice a second while it's set.
    let poll = cfg!(debug_assertions) && std::env::var_os("ENDEAVOR_FORCE_OFFLINE").is_some();
    loop {
        let next = if poll {
            std::thread::sleep(Duration::from_millis(500));
            let mut latest = None;
            while let Ok(value) = system.try_recv() {
                latest = Some(value);
            }
            latest
        } else {
            match futures::executor::block_on(system.next()) {
                Some(value) => Some(value),
                None => return,
            }
        };
        if let Some(value) = next {
            online = value;
        }
        let now = online && !forced_offline();
        if sent != Some(now) {
            sent = Some(now);
            if tx.unbounded_send(now).is_err() {
                return;
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_int, c_void};

    use block2::RcBlock;
    use futures::channel::mpsc::UnboundedSender;

    /// nw_path_status_satisfied.
    const SATISFIED: c_int = 1;

    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        fn nw_path_monitor_create() -> *mut c_void;
        fn nw_path_monitor_set_update_handler(monitor: *mut c_void, handler: &block2::Block<dyn Fn(*mut c_void)>);
        fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        fn nw_path_monitor_start(monitor: *mut c_void);
        fn nw_path_get_status(path: *mut c_void) -> c_int;
    }

    unsafe extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
    }

    /// The monitor and its queue live as long as the app.
    pub fn monitor(tx: UnboundedSender<bool>) {
        unsafe {
            let monitor = nw_path_monitor_create();
            let handler = RcBlock::new(move |path: *mut c_void| {
                let _ = tx.unbounded_send(nw_path_get_status(path) == SATISFIED);
            });
            nw_path_monitor_set_update_handler(monitor, &handler);
            let queue = dispatch_queue_create(c"endeavor.network".as_ptr(), std::ptr::null_mut());
            nw_path_monitor_set_queue(monitor, queue);
            nw_path_monitor_start(monitor);
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use futures::channel::mpsc::UnboundedSender;

    /// Listen for route and address changes over netlink; after each, look for
    /// a default route again.
    pub fn monitor(tx: UnboundedSender<bool>) {
        let _ = tx.unbounded_send(online());
        std::thread::spawn(move || unsafe {
            let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE);
            if fd < 0 {
                return;
            }
            let mut addr: libc::sockaddr_nl = std::mem::zeroed();
            addr.nl_family = libc::AF_NETLINK as u16;
            addr.nl_groups = (libc::RTMGRP_LINK | libc::RTMGRP_IPV4_IFADDR | libc::RTMGRP_IPV4_ROUTE | libc::RTMGRP_IPV6_IFADDR | libc::RTMGRP_IPV6_ROUTE) as u32;
            if libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_nl>() as u32) < 0 {
                return;
            }
            let mut buf = [0u8; 8192];
            while libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) >= 0 {
                if tx.unbounded_send(online()).is_err() {
                    break;
                }
            }
        });
    }

    fn online() -> bool {
        let v4 = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
        let v6 = std::fs::read_to_string("/proc/net/ipv6_route").unwrap_or_default();
        super::has_default_route(&v4, &v6)
    }
}

/// A default route on an interface other than loopback, in /proc/net/route's
/// and /proc/net/ipv6_route's formats.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn has_default_route(v4: &str, v6: &str) -> bool {
    const RTF_UP: u32 = 0x1;
    let v4_default = v4.lines().skip(1).any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        f.len() > 3 && f[0] != "lo" && f[1] == "00000000" && u32::from_str_radix(f[3], 16).is_ok_and(|flags| flags & RTF_UP != 0)
    });
    let v6_default = v6.lines().any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        f.len() == 10 && f[9] != "lo" && f[0].bytes().all(|b| b == b'0') && f[1] == "00" && u32::from_str_radix(f[8], 16).is_ok_and(|flags| flags & RTF_UP != 0)
    });
    v4_default || v6_default
}

#[cfg(test)]
mod tests {
    use super::has_default_route;

    const V4_ONLINE: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
        eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
        eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
    const V4_LAN_ONLY: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
        eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
    const V6_LOOPBACK: &str = "00000000000000000000000000000001 80 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000001 00000000 80200001       lo\n";
    const V6_ONLINE: &str = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00450003     eth0\n";

    #[test]
    fn a_default_route_off_loopback_means_online() {
        assert!(has_default_route(V4_ONLINE, ""));
        assert!(has_default_route(V4_LAN_ONLY, V6_ONLINE));
        assert!(!has_default_route(V4_LAN_ONLY, V6_LOOPBACK));
        assert!(!has_default_route("", ""));
    }
}
