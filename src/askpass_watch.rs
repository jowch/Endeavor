use std::time::Duration;

const GAVE_UP: &str = "the server stopped waiting for the sign-in.";

const EVERY: Duration = if cfg!(target_os = "linux") { Duration::from_secs(1) } else { Duration::from_secs(2) };

pub fn start() {
    let Some(ssh) = os::Parent::of_this_process() else { return };
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(EVERY);
            match ssh.connection() {
                Seen::Closed => {
                    eprintln!("{GAVE_UP}");
                    std::process::exit(1);
                }
                Seen::Gone => std::process::exit(1),
                Seen::Open | Seen::Unknown => {}
            }
        }
    });
}

#[derive(Debug, PartialEq)]
enum Seen {
    Open,
    Closed,
    Gone,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tcp {
    Listen,
    Established,
    CloseWait,
    Other,
}

fn judge(states: &[Tcp]) -> Seen {
    if states.contains(&Tcp::Established) {
        Seen::Open
    } else if states.contains(&Tcp::CloseWait) {
        Seen::Closed
    } else {
        Seen::Unknown
    }
}

#[cfg(target_os = "linux")]
mod os {
    use super::{Seen, Tcp, judge};

    pub struct Parent(libc::pid_t);

    impl Parent {
        pub fn of_this_process() -> Option<Parent> {
            Some(Parent(unsafe { libc::getppid() })).filter(|p| p.0 > 1)
        }

        pub fn connection(&self) -> Seen {
            if unsafe { libc::getppid() } != self.0 {
                return Seen::Gone;
            }
            let Ok(fds) = std::fs::read_dir(format!("/proc/{}/fd", self.0)) else { return Seen::Unknown };
            let sockets: Vec<String> = fds
                .flatten()
                .filter_map(|fd| std::fs::read_link(fd.path()).ok())
                .filter_map(|link| Some(link.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.to_owned()))
                .collect();
            let mut states = Vec::new();
            for table in ["tcp", "tcp6"] {
                if let Ok(text) = std::fs::read_to_string(format!("/proc/{}/net/{table}", self.0)) {
                    states.extend(tcp_states(&text, &sockets));
                }
            }
            judge(&states)
        }
    }

    pub(super) fn tcp_states(table: &str, sockets: &[String]) -> Vec<Tcp> {
        table
            .lines()
            .skip(1)
            .filter_map(|row| {
                let columns: Vec<&str> = row.split_whitespace().collect();
                let (state, inode) = (*columns.get(3)?, *columns.get(9)?);
                sockets.iter().any(|s| s == inode).then_some(match state {
                    "01" => Tcp::Established,
                    "08" => Tcp::CloseWait,
                    "0A" => Tcp::Listen,
                    _ => Tcp::Other,
                })
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::{Seen, Tcp, judge};

    pub struct Parent(libc::pid_t);

    impl Parent {
        pub fn of_this_process() -> Option<Parent> {
            Some(Parent(unsafe { libc::getppid() })).filter(|p| p.0 > 1)
        }

        pub fn connection(&self) -> Seen {
            if unsafe { libc::getppid() } != self.0 {
                return Seen::Gone;
            }
            let output = std::process::Command::new("/usr/sbin/lsof").args(["-nP", "-a", "-p", &self.0.to_string(), "-iTCP", "-FT"]).stderr(std::process::Stdio::null()).output();
            let Ok(output) = output else { return Seen::Unknown };
            judge(&lsof_states(&String::from_utf8_lossy(&output.stdout)))
        }
    }

    pub(super) fn lsof_states(text: &str) -> Vec<Tcp> {
        text.lines()
            .filter_map(|line| line.strip_prefix("TST="))
            .map(|state| match state {
                "ESTABLISHED" => Tcp::Established,
                "CLOSE_WAIT" => Tcp::CloseWait,
                "LISTEN" => Tcp::Listen,
                _ => Tcp::Other,
            })
            .collect()
    }
}

#[cfg(windows)]
mod os {
    use super::{Seen, Tcp, judge};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, NO_ERROR, WAIT_OBJECT_0};
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 23;

    pub struct Parent {
        pid: u32,
        handle: HANDLE,
    }

    unsafe impl Send for Parent {}

    impl Drop for Parent {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    impl Parent {
        pub fn of_this_process() -> Option<Parent> {
            let pid = parent_pid()?;
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            (!handle.is_null()).then_some(Parent { pid, handle })
        }

        pub fn connection(&self) -> Seen {
            if unsafe { WaitForSingleObject(self.handle, 0) } == WAIT_OBJECT_0 {
                return Seen::Gone;
            }
            let mut states = rows::<MIB_TCPROW_OWNER_PID>(AF_INET).into_iter().filter(|r| r.dwOwningPid == self.pid).map(|r| state(r.dwState)).collect::<Vec<_>>();
            states.extend(rows::<MIB_TCP6ROW_OWNER_PID>(AF_INET6).into_iter().filter(|r| r.dwOwningPid == self.pid).map(|r| state(r.dwState)));
            judge(&states)
        }
    }

    fn state(state: u32) -> Tcp {
        match state {
            2 => Tcp::Listen,
            5 => Tcp::Established,
            8 => Tcp::CloseWait,
            _ => Tcp::Other,
        }
    }

    fn rows<Row: Copy>(family: u32) -> Vec<Row> {
        let mut size = 0u32;
        unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, family, TCP_TABLE_OWNER_PID_ALL, 0) };
        let mut buffer = vec![0u32; (size as usize + 4096) / 4];
        let mut size = (buffer.len() * 4) as u32;
        if unsafe { GetExtendedTcpTable(buffer.as_mut_ptr().cast(), &mut size, 0, family, TCP_TABLE_OWNER_PID_ALL, 0) } != NO_ERROR {
            return Vec::new();
        }
        let count = buffer[0] as usize;
        let first = unsafe { buffer.as_ptr().add(1).cast::<Row>() };
        (0..count).map(|i| unsafe { first.add(i).read_unaligned() }).collect()
    }

    fn parent_pid() -> Option<u32> {
        let me = unsafe { GetCurrentProcessId() };
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot.is_null() || snapshot as isize == -1 {
            return None;
        }
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more {
            if entry.th32ProcessID == me {
                found = Some(entry.th32ParentProcessID);
                break;
            }
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot) };
        found
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod os {
    use super::Seen;

    pub struct Parent;

    impl Parent {
        pub fn of_this_process() -> Option<Parent> {
            None
        }

        pub fn connection(&self) -> Seen {
            Seen::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_the_server_closed_is_told_from_an_open_one() {
        assert_eq!(judge(&[Tcp::Established]), Seen::Open);
        assert_eq!(judge(&[Tcp::CloseWait]), Seen::Closed);
        assert_eq!(judge(&[Tcp::Listen, Tcp::CloseWait]), Seen::Closed, "a forwarded port isn't the connection");
        assert_eq!(judge(&[Tcp::Listen]), Seen::Unknown);
        assert_eq!(judge(&[]), Seen::Unknown, "through a ProxyJump ssh has no TCP socket of its own");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_procs_tcp_table() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:08AE 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 627 1 0000000000000000 100 0 0 10 0\n  41: 0100007F:C430 0100007F:08AE 08 00000000:00000001 02:000AF7C0 00000000     0        0 4580 2 0000000000000000 20 4 22 16 -1\n  42: 0100007F:C432 0100007F:08AE 01 00000000:00000000 02:000AF7C0 00000000     0        0 4581 2 0000000000000000 20 4 22 16 -1\n";
        assert_eq!(os::tcp_states(table, &["4580".into()]), vec![Tcp::CloseWait]);
        assert_eq!(os::tcp_states(table, &["4581".into(), "627".into()]), vec![Tcp::Listen, Tcp::Established]);
        assert_eq!(os::tcp_states(table, &[]), vec![]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_lsofs_states() {
        assert_eq!(os::lsof_states("p35580\nf3\nTST=ESTABLISHED\nTQR=0\nTQS=0\n"), vec![Tcp::Established]);
        assert_eq!(os::lsof_states("p35580\nf3\nTST=CLOSE_WAIT\nTQR=1\nTQS=0\n"), vec![Tcp::CloseWait]);
    }
}
