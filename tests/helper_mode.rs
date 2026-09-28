//! The app binary is also This Mac's runtime helper (`endeavor --helper …`)
//! and ssh's askpass program, so neither needs `endeavor-remote` beside it.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use wire::askpass::{Answer, Ask, Kind, SOCKET_ENV};
use wire::{Frame, ToApp};

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
