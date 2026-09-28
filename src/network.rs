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

const API: (&str, u16) = ("api.anthropic.com", 443);

/// Try now: can Claude's API be reached this moment, the way Claude Code
/// reaches it? A few seconds at most.
pub fn probe() -> bool {
    !forced_offline() && reachable(&route(|name| std::env::var(name).ok()), API)
}

/// How Claude Code gets to the API, from the environment the agent inherits
/// from this app. It reads the proxy variables only, never the system's proxy
/// settings, so neither does this.
#[derive(Debug, PartialEq)]
enum Route {
    Direct,
    Proxy(Proxy),
}

#[derive(Debug, PartialEq)]
struct Proxy {
    host: String,
    port: u16,
    /// `user:password` from the proxy's URL, decoded.
    auth: Option<String>,
    /// An `http://` proxy, which takes a CONNECT. Any other (TLS to the proxy,
    /// SOCKS) is only checked for being there.
    http: bool,
}

/// Claude Code's choice: the first of `https_proxy`, `HTTPS_PROXY`,
/// `http_proxy` and `HTTP_PROXY` that is set, unless `no_proxy`/`NO_PROXY`
/// names the API's host.
fn route(env: impl Fn(&str) -> Option<String>) -> Route {
    let set = |name: &str| env(name).filter(|v| !v.is_empty());
    let Some(url) = ["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"].into_iter().find_map(set) else { return Route::Direct };
    let no_proxy = [set("no_proxy"), set("NO_PROXY")];
    if no_proxy.iter().flatten().any(|list| bypasses(list, API)) {
        return Route::Direct;
    }
    proxy(&url).map_or(Route::Direct, Route::Proxy)
}

/// A `NO_PROXY` list names `host`: `*`, the host, `.domain` for it and its
/// subdomains, or `host:port`.
fn bypasses(list: &str, (host, port): (&str, u16)) -> bool {
    if list.trim() == "*" {
        return true;
    }
    list.split([',', ' ', '\t']).map(|entry| entry.trim().to_lowercase()).filter(|entry| !entry.is_empty()).any(|entry| {
        if entry.contains(':') {
            entry == format!("{host}:{port}")
        } else if let Some(domain) = entry.strip_prefix('.') {
            host == domain || host.ends_with(&entry)
        } else {
            host == entry
        }
    })
}

/// A proxy URL; a bare `host:port` is taken as `http://`.
fn proxy(url: &str) -> Option<Proxy> {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let scheme = scheme.to_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?;
    let (auth, hostport) = match authority.rsplit_once('@') {
        Some((auth, hostport)) => (Some(percent_decode(auth)), hostport),
        None => (None, authority),
    };
    let default = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => 1080,
    };
    let (host, port) = match hostport.strip_prefix('[') {
        Some(v6) => v6.split_once(']')?,
        None => hostport.rsplit_once(':').unwrap_or((hostport, "")),
    };
    let port = match port.trim_start_matches(':') {
        "" => default,
        port => port.parse().ok()?,
    };
    (!host.is_empty()).then(|| Proxy { host: host.to_owned(), port, auth, http: scheme == "http" })
}

/// Can `target` be reached by `route`: a TCP connect straight to it, or a
/// CONNECT through the proxy that the proxy answers with 200.
fn reachable(route: &Route, (host, port): (&str, u16)) -> bool {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let connect = |host: &str, port: u16| -> Option<TcpStream> {
        (host, port).to_socket_addrs().ok()?.take(2).find_map(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(3)).ok())
    };
    let proxy = match route {
        Route::Direct => return connect(host, port).is_some(),
        Route::Proxy(proxy) => proxy,
    };
    let Some(mut stream) = connect(&proxy.host, proxy.port) else { return false };
    if !proxy.http {
        return true;
    }
    let auth = proxy.auth.as_ref().map(|a| format!("Proxy-Authorization: Basic {}\r\n", base64(a.as_bytes()))).unwrap_or_default();
    let request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n{auth}\r\n");
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut status = String::new();
    if BufReader::new(stream).read_line(&mut status).is_err() {
        return false;
    }
    status.split_whitespace().nth(1) == Some("200")
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
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
    use super::{API, Proxy, Route, has_default_route, reachable, route};

    fn route_with(vars: &[(&str, &str)]) -> Route {
        route(|name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()))
    }

    fn proxy(host: &str, port: u16, auth: Option<&str>, http: bool) -> Route {
        Route::Proxy(Proxy { host: host.into(), port, auth: auth.map(Into::into), http })
    }

    #[test]
    fn the_probe_takes_the_proxy_claude_code_would() {
        assert_eq!(route_with(&[]), Route::Direct);
        assert_eq!(route_with(&[("HTTPS_PROXY", "http://proxy.lab:3128")]), proxy("proxy.lab", 3128, None, true));
        // Lowercase first, then HTTP_PROXY when no HTTPS one is set; an empty value doesn't count.
        assert_eq!(route_with(&[("HTTPS_PROXY", "http://upper:1"), ("https_proxy", "http://lower:2")]), proxy("lower", 2, None, true));
        assert_eq!(route_with(&[("https_proxy", ""), ("HTTP_PROXY", "plain:8080")]), proxy("plain", 8080, None, true));
        assert_eq!(route_with(&[("HTTPS_PROXY", "http://me%40lab:p%3Ass@proxy/")]), proxy("proxy", 80, Some("me@lab:p:ss"), true));
        assert_eq!(route_with(&[("HTTPS_PROXY", "https://[::1]")]), proxy("::1", 443, None, false));
        assert_eq!(route_with(&[("HTTPS_PROXY", "socks5://gate:1081")]), proxy("gate", 1081, None, false));
    }

    #[test]
    fn no_proxy_sends_the_probe_straight() {
        let with = |no_proxy: &str| route_with(&[("HTTPS_PROXY", "http://proxy:3128"), ("NO_PROXY", no_proxy)]);
        for bypass in ["*", "api.anthropic.com", ".anthropic.com", "localhost, .ANTHROPIC.com", "api.anthropic.com:443"] {
            assert_eq!(with(bypass), Route::Direct, "{bypass}");
        }
        for through in ["anthropic.com", "*.anthropic.com", "api.anthropic.com:8443", "localhost,*"] {
            assert_eq!(with(through), proxy("proxy", 3128, None, true), "{through}");
        }
        assert_eq!(route_with(&[("HTTPS_PROXY", "http://proxy:3128"), ("no_proxy", "x"), ("NO_PROXY", ".anthropic.com")]), Route::Direct);
    }

    /// A one-shot proxy on localhost that answers a CONNECT with `status`
    /// and hands back the request it got.
    fn fake_proxy(status: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let thread = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            while reader.read_line(&mut request).unwrap() > 2 {}
            (&stream).write_all(format!("HTTP/1.1 {status}\r\n\r\n").as_bytes()).unwrap();
            request
        });
        (port, thread)
    }

    #[test]
    fn the_probe_connects_through_the_proxy() {
        let (port, request) = fake_proxy("200 Connection Established");
        assert!(reachable(&proxy("127.0.0.1", port, Some("me:pw"), true), API));
        assert_eq!(request.join().unwrap(), "CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: api.anthropic.com:443\r\nProxy-Authorization: Basic bWU6cHc=\r\n\r\n");

        let (port, _) = fake_proxy("502 Bad Gateway");
        assert!(!reachable(&proxy("127.0.0.1", port, None, true), API));

        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(!reachable(&proxy("127.0.0.1", closed, None, true), API));
    }

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
