//! An agent's whole process tree ends with its connection. On Unix the ACP
//! library starts the adapter as its own process group and kills the group.
//! Windows has no groups, and the library ends only the process it started,
//! so an adapter that starts processes of its own (Claude's adapter starts
//! Claude Code; Codex's starts `codex app-server`) left them running. There
//! the adapter runs under this app instead: `endeavor --agent-job <app pid>
//! <program> <args…>` puts itself in a job that ends every process in it when
//! it ends, then starts the adapter inside it. It ends when the adapter does,
//! when the library kills it, or when the app is gone.

/// The first argument that runs the app as an agent's job.
pub const FLAG: &str = "--agent-job";

/// `program` and `args` as the agent connection starts them: on Windows
/// through this app's job, elsewhere as they are.
pub fn command(program: String, args: Vec<String>) -> Result<Vec<String>, String> {
    if cfg!(windows) {
        let app = std::env::current_exe().map_err(|e| format!("Couldn't find Endeavor's own program: {e}"))?;
        Ok([app.display().to_string(), FLAG.to_owned(), std::process::id().to_string(), program].into_iter().chain(args).collect())
    } else {
        Ok(std::iter::once(program).chain(args).collect())
    }
}

/// `endeavor --agent-job <app pid> <program> <args…>`: run the adapter in a
/// job, and exit with its exit code. Exiting closes the job, which ends what
/// the adapter left.
#[cfg(windows)]
pub fn run(args: Vec<String>) -> ! {
    let code = match run_in_job(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Couldn't start the agent: {e}");
            1
        }
    };
    std::process::exit(code)
}

#[cfg(windows)]
fn run_in_job(args: &[String]) -> Result<i32, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForMultipleObjects};

    let [app, program, rest @ ..] = args else { return Err("no program given".into()) };
    let app: u32 = app.parse().map_err(|_| format!("not a process id: {app}"))?;

    // Held open until this process exits: closing it sooner would end this
    // process too, before it could exit with the adapter's code. An agent
    // without a job still works; it only leaves its processes behind when
    // it's stopped, as it did before.
    match job() {
        Ok(job) => std::mem::forget(job),
        Err(e) => eprintln!("Starting the agent without a job, so what it starts may outlive it: {e}"),
    }

    // SAFETY: a plain open of another process for waiting on; null when it's gone.
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, app) };
    if raw.is_null() {
        return Err("Endeavor is no longer running".into());
    }
    // SAFETY: a handle OpenProcess just gave us.
    let app = unsafe { OwnedHandle::from_raw_handle(raw as _) };

    // The adapter takes this process's stdin, stdout and stderr: the app's pipes.
    let mut command = std::process::Command::new(program);
    command.args(rest);
    endeavor_mcp::client::no_window(&mut command);
    let mut child = command.spawn().map_err(|e| format!("{program}: {e}"))?;

    let handles = [child.as_raw_handle() as HANDLE, app.as_raw_handle() as HANDLE];
    // SAFETY: two handles we hold for the whole wait.
    let which = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
    if which == WAIT_OBJECT_0 + 1 {
        // The app is gone: so is the agent, when this process exits and
        // closes the job.
        return Ok(1);
    }
    // The adapter ended, or the wait failed: then wait on the adapter alone.
    let status = child.wait().map_err(|e| e.to_string())?;
    Ok(status.code().unwrap_or(1))
}

/// A job that ends every process in it when its last handle closes, with this
/// process in it, so everything it starts from here on is too. Nothing
/// inherits the handle, so it closes when this process exits, however it
/// exits.
#[cfg(windows)]
fn job() -> Result<std::os::windows::io::OwnedHandle, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: no security attributes and no name: a private job.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: a handle CreateJobObjectW just gave us, owned from here on.
    let job = unsafe { OwnedHandle::from_raw_handle(raw as _) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    // A process that asks to leave the job may, as one that leaves its
    // process group may on Unix.
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    let size = std::mem::size_of_val(&limits) as u32;
    // SAFETY: the job we own, and a limit structure of the size given.
    if unsafe { SetInformationJobObject(job.as_raw_handle() as HANDLE, JobObjectExtendedLimitInformation, (&raw const limits).cast(), size) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // In the job before the adapter starts, so everything it starts is too.
    // Windows 8 and later nest jobs, so this works inside the app's own.
    // SAFETY: the job we own, and the pseudo-handle for this process.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle() as HANDLE, GetCurrentProcess()) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::process::{Child, Command, Stdio};

    #[cfg(windows)]
    const ROLE: &str = "ENDEAVOR_AGENT_JOB_TEST_ROLE";
    #[cfg(windows)]
    const PIDS: &str = "ENDEAVOR_AGENT_JOB_TEST_PIDS";
    #[cfg(windows)]
    const APP: &str = "ENDEAVOR_AGENT_JOB_TEST_APP";
    #[cfg(windows)]
    const WAITS: &str = "ENDEAVOR_AGENT_JOB_TEST_WAITS";

    /// This test binary again, running only `test`.
    #[cfg(windows)]
    fn rerun_args(test: &str) -> Vec<String> {
        ["--exact", test, "--nocapture", "--test-threads=1"].map(str::to_owned).to_vec()
    }

    /// Whether the process `pid` ends within `ms` (or is gone already).
    #[cfg(windows)]
    fn ends_within(pid: u32, ms: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        // SAFETY: a plain open for waiting; null when the process is gone.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return true;
        }
        // SAFETY: the handle just opened, closed once waited on.
        let ended = unsafe { WaitForSingleObject(handle, ms) } == WAIT_OBJECT_0;
        unsafe { CloseHandle(handle) };
        ended
    }

    /// About a minute of ping, to stand for what an adapter starts.
    #[cfg(windows)]
    fn ping(seconds: u32) -> Child {
        Command::new("ping").args(["-n", &seconds.to_string(), "127.0.0.1"]).stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap()
    }

    /// Plays the part this run of the test binary was started for, if any:
    /// `endeavor --agent-job` (with the test as the app), or the adapter,
    /// which starts ping, writes its own pid and ping's, then exits or waits
    /// on ping. Returns whether it played one.
    #[cfg(windows)]
    fn played(test: &str) -> bool {
        match std::env::var(ROLE).as_deref() {
            Ok("job") => {
                let app = std::env::var(APP).unwrap();
                let me = std::env::current_exe().unwrap().display().to_string();
                let args: Vec<String> = [app, me].into_iter().chain(rerun_args(test)).collect();
                // The adapter inherits this environment, so it runs as the adapter.
                // SAFETY: this process runs one test on one thread.
                unsafe { std::env::set_var(ROLE, "adapter") };
                std::process::exit(run_in_job(&args).unwrap());
            }
            Ok("adapter") => {
                let mut ping = ping(60);
                let path = std::env::var(PIDS).unwrap();
                std::fs::write(format!("{path}.part"), format!("{} {}", std::process::id(), ping.id())).unwrap();
                std::fs::rename(format!("{path}.part"), &path).unwrap();
                if std::env::var(WAITS).is_ok() {
                    let _ = ping.wait();
                } else {
                    // Left running on purpose: what the job has to end.
                    std::mem::forget(ping);
                }
                true
            }
            _ => false,
        }
    }

    /// Starts `test` as the job, for the app `app`, and returns it with the
    /// adapter's pid and ping's.
    #[cfg(windows)]
    fn start(test: &str, app: u32, adapter_waits: bool) -> (Child, u32, u32) {
        let file = std::env::temp_dir().join(format!("endeavor-agent-job-test-{}-{}", std::process::id(), test.rsplit(':').next().unwrap()));
        let _ = std::fs::remove_file(&file);
        let mut job = Command::new(std::env::current_exe().unwrap());
        job.args(rerun_args(test)).env(ROLE, "job").env(PIDS, &file).env(APP, app.to_string());
        if adapter_waits {
            job.env(WAITS, "1");
        }
        let job = job.stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap();
        let mut pids = None;
        for _ in 0..300 {
            if let Ok(text) = std::fs::read_to_string(&file) {
                let mut words = text.split_whitespace().map(|w| w.parse::<u32>().unwrap());
                pids = Some((words.next().unwrap(), words.next().unwrap()));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = std::fs::remove_file(&file);
        let (adapter, ping) = pids.expect("the adapter wrote its pids");
        (job, adapter, ping)
    }

    /// Antigravity's case: the adapter starts a process and exits, leaving it.
    #[cfg(windows)]
    #[test]
    fn what_the_adapter_leaves_ends_with_the_job() {
        const TEST: &str = "agent_job::tests::what_the_adapter_leaves_ends_with_the_job";
        if played(TEST) {
            return;
        }
        let (mut job, _, ping) = start(TEST, std::process::id(), false);
        assert_eq!(job.wait().unwrap().code(), Some(0), "the job ends when the adapter does, with its code");
        assert!(ends_within(ping, 10_000), "ping (pid {ping}) outlived the job");
    }

    /// Restart and quit: the connection kills the job while the adapter and
    /// what it started still run.
    #[cfg(windows)]
    #[test]
    fn killing_the_job_ends_the_adapter_and_what_it_started() {
        const TEST: &str = "agent_job::tests::killing_the_job_ends_the_adapter_and_what_it_started";
        if played(TEST) {
            return;
        }
        let (mut job, adapter, ping) = start(TEST, std::process::id(), true);
        assert!(!ends_within(adapter, 500), "the adapter waits on ping");
        job.kill().unwrap();
        job.wait().unwrap();
        assert!(ends_within(adapter, 10_000), "the adapter (pid {adapter}) outlived the job");
        assert!(ends_within(ping, 10_000), "ping (pid {ping}) outlived the job");
    }

    /// The app is gone (it crashed, or was ended): the agent goes with it.
    #[cfg(windows)]
    #[test]
    fn the_agent_ends_when_the_app_does() {
        const TEST: &str = "agent_job::tests::the_agent_ends_when_the_app_does";
        if played(TEST) {
            return;
        }
        // A few seconds of ping stands for the app: long enough for the job
        // and the adapter to start.
        let mut app = ping(6);
        let (mut job, adapter, ping) = start(TEST, app.id(), true);
        app.wait().unwrap();
        assert_eq!(job.wait().unwrap().code(), Some(1), "the job ends when the app does");
        assert!(ends_within(adapter, 10_000), "the adapter (pid {adapter}) outlived the app");
        assert!(ends_within(ping, 10_000), "ping (pid {ping}) outlived the app");
    }

    #[test]
    fn elsewhere_the_command_is_the_program_and_its_arguments() {
        let command = command("node".into(), vec!["index.js".into()]).unwrap();
        if cfg!(windows) {
            assert_eq!(&command[1..], [FLAG, &std::process::id().to_string(), "node", "index.js"]);
        } else {
            assert_eq!(command, ["node", "index.js"]);
        }
    }
}
