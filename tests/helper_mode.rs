//! The app binary is also This Mac's runtime helper (`endeavor --helper …`)
//! and ssh's askpass program, so neither needs `endeavor-remote` beside it.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use wire::askpass::{Answer, Ask, Kind, SOCKET_ENV};
use wire::{Frame, ToApp, ToHelper};

#[test]
fn the_app_runs_as_the_helper() {
    let dir = std::env::temp_dir().join(format!("endeavor-helper-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut helper = Command::new(env!("CARGO_BIN_EXE_endeavor"))
        .arg0("endeavor-remote")
        .args(["--helper", "connect", "--state-dir"])
        .arg(dir.join("state"))
        .args(["--julia", "/nonexistent/julia", "--runtime"])
        .arg(&dir)
        .args(["--depot", "/nonexistent/depot"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = helper.stdout.take().unwrap();
    let Some(Frame::Control(json)) = Frame::read_from(&mut stdout).unwrap() else { panic!("no hello") };
    match serde_json::from_slice(&json).unwrap() {
        ToApp::Hello { version, .. } => assert_eq!(version, env!("CARGO_PKG_VERSION")),
        other => panic!("expected hello, got {other:?}"),
    }
    assert!(dir.join("state").is_dir(), "it made its state folder");
    drop(helper.stdin.take());
    assert!(helper.wait().unwrap().success(), "it leaves when the app closes its stdin");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_app_starts_the_runtime_core_as_the_helper() {
    let dir = std::env::temp_dir().join(format!("endeavor-helper-core-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A julia that records the command line of the process that started it (the core).
    let julia = dir.join("julia");
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo 'julia version 1.12.0'; exit 0; }}\nps -o command= -p $PPID > {core}.tmp\necho $PPID >> {core}.tmp\nmv {core}.tmp {core}\nexec sleep 60\n",
        core = dir.join("core").display()
    );
    std::fs::write(&julia, script).unwrap();
    std::fs::set_permissions(&julia, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut helper = Command::new(env!("CARGO_BIN_EXE_endeavor"))
        .arg0("endeavor-remote")
        .args(["--helper", "connect", "--state-dir"])
        .arg(dir.join("state"))
        .arg("--julia")
        .arg(&julia)
        .arg("--runtime")
        .arg(&dir)
        .args(["--depot", "/nonexistent/depot"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let start = serde_json::to_vec(&ToHelper::StartRuntime { job: None }).unwrap();
    Frame::Control(start).write_to(helper.stdin.as_mut().unwrap()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !dir.join("core").exists() {
        assert!(std::time::Instant::now() < deadline, "julia never started");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let recorded = std::fs::read_to_string(dir.join("core")).unwrap();
    let (command, pid) = recorded.trim().rsplit_once('\n').unwrap();
    // SAFETY: the core leads its own process group; this stops it and Julia.
    unsafe { libc::kill(-pid.parse::<i32>().unwrap(), libc::SIGKILL) };
    let _ = helper.kill();
    let _ = helper.wait();
    assert!(command.starts_with("endeavor-remote --helper core --state-dir"), "{command}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_app_is_sshs_askpass() {
    let dir = std::env::temp_dir().join(format!("endeavor-askpass-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("s");
    let listener = UnixListener::bind(&socket).unwrap();
    let app = std::thread::spawn(move || {
        let (connection, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&connection).read_line(&mut line).unwrap();
        let ask: Ask = serde_json::from_str(&line).unwrap();
        (&connection).write_all(format!("{}\n", serde_json::to_string(&Answer { text: Some("hunter2".into()) }).unwrap()).as_bytes()).unwrap();
        ask
    });
    let out = Command::new(env!("CARGO_BIN_EXE_endeavor")).arg("jc@lab's password: ").env(SOCKET_ENV, &socket).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hunter2\n");
    assert_eq!(app.join().unwrap(), Ask { kind: Kind::Secret, prompt: "jc@lab's password:".into() });
    let _ = std::fs::remove_dir_all(&dir);
}
