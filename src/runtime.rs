//! The app-owned Julia process (design doc §11): starts Pluto + PlutoMCP via
//! runtime/boot.jl, explains failures in plain terms, and reports if it dies.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind};
use std::net::TcpListener;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::UnboundedSender;

/// Julia needed by runtime/Project.toml's `[sources]` section.
const MIN_JULIA: (u32, u32) = (1, 11);
const STDERR_TAIL: usize = 40;

pub struct Runtime {
    // boot.jl exits when its stdin closes, so this handle is Julia's lifetime.
    stdin: ChildStdin,
    pub pluto_url: String,
    pub mcp_url: String,
    /// Reused on restart so the agent's MCP connection to the bridge reconnects.
    pub ports: [u16; 2],
}

impl Runtime {
    /// Make `dir` the folder Pluto suggests when saving a new notebook.
    pub fn suggest_folder(&mut self, dir: &std::path::Path) {
        use std::io::Write;
        let dir = dir.display().to_string();
        if !dir.contains('\n') {
            // ponytail: a dead Julia is reported by the crash watcher, not here.
            let _ = writeln!(self.stdin, "folder {dir}");
        }
    }
}

/// The Julia the app installs on first run (design doc §11), pinned with the
/// official tarballs' SHA-256 and size (bump all three per release).
pub const JULIA_VERSION: &str = "1.12.6";
#[cfg(target_arch = "aarch64")]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/mac/aarch64/1.12/julia-1.12.6-macaarch64.tar.gz",
    "277d82fbd2eda99d0963b3e41f3dc979d7486f181399f8430fb637318ccd6a31",
    231_027_185,
);
#[cfg(target_arch = "x86_64")]
const JULIA_TARBALL: (&str, &str, u64) = (
    "https://julialang-s3.julialang.org/bin/mac/x64/1.12/julia-1.12.6-mac64.tar.gz",
    "1a70b7c606d6bac38a246e722369e5b30914dccf9378499d2712fb3bd282642c",
    271_518_180,
);


/// The julia binary to run: the user's (Settings), else the app's own,
/// downloaded and verified on first run. `progress` gets status lines meanwhile.
fn julia_binary(progress: &dyn Fn(String)) -> Result<String, String> {
    if let Some(julia) = crate::settings::Settings::load().julia {
        return Ok(julia.display().to_string());
    }
    let dir = crate::install::app_dir()?.join(format!("julia-{JULIA_VERSION}"));
    let bin = dir.join("bin/julia");
    if !bin.exists() {
        crate::install::tarball(&dir, &format!("Julia {JULIA_VERSION}"), &format!("julia-{JULIA_VERSION}"), JULIA_TARBALL, progress)?;
    }
    Ok(bin.display().to_string())
}


/// Start Julia and block until boot.jl reports `READY`. `ports` pins the Pluto
/// and MCP ports (restart); `died` gets a message if Julia exits afterwards;
/// `progress` gets status lines during a first-run install.
pub fn start(ports: Option<[u16; 2]>, died: UnboundedSender<String>, progress: &dyn Fn(String)) -> Result<Runtime, String> {
    let root = crate::install::resources().display().to_string();
    let julia = julia_binary(progress)?;
    check_version(&julia)?;
    // Trailing ':' stacks the default depots (~/.julia) read-only behind ours.
    let depot = format!("{}/depot:", crate::install::app_dir()?.display());
    let ports = match ports {
        Some(ports) => ports,
        None => free_ports()?,
    };

    let mut child = Command::new(&julia)
        .arg("--color=no") // its log is shown in the panel on failure
        .arg(format!("--project={root}/runtime"))
        .arg(format!("{root}/runtime/boot.jl"))
        .args(ports.map(|p| p.to_string()))
        .env("JULIA_DEPOT_PATH", depot)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start {julia}: {e}"))?;

    // Echo Julia's log to our stderr and keep its tail for error messages.
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let stderr = BufReader::new(child.stderr.take().unwrap());
    let log = tail.clone();
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            eprintln!("{line}");
            let mut log = log.lock().unwrap();
            if log.len() == STDERR_TAIL {
                log.pop_front();
            }
            // The tail may be shown in the panel; Pluto's secret must not be.
            log.push_back(redact_secret(&line));
        }
    });
    let tail_text = move || tail.lock().unwrap().iter().cloned().collect::<Vec<_>>();

    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    for line in lines.by_ref() {
        let line = line.map_err(|e| e.to_string())?;
        let Some((pluto_url, mcp_url)) = line.strip_prefix("READY ").and_then(|r| r.split_once(' ')) else {
            println!("{line}");
            continue;
        };
        let (pluto_url, mcp_url) = (pluto_url.to_owned(), mcp_url.to_owned());
        let stdin = child.stdin.take().unwrap();
        // Keep draining stdout so a chatty Julia never blocks on a full pipe.
        std::thread::spawn(move || lines.map_while(Result::ok).for_each(|l| println!("{l}")));
        // Report an unexpected exit. When the app quits, this thread dies with it first.
        std::thread::spawn(move || {
            let status = child.wait().map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
            // A crash's log is just Pluto's startup banner: say why only if we know.
            let hint = hint(&tail_text().join("\n")).map(|h| format!(" {h}")).unwrap_or_default();
            let _ = died.unbounded_send(format!("Julia exited ({status}).{hint}"));
        });
        return Ok(Runtime { stdin, pluto_url, mcp_url, ports });
    }
    let status = child.wait().map(|s| s.to_string()).unwrap_or_default();
    Err(format!("Julia stopped before Pluto was ready ({status}). {}", diagnose(&tail_text())))
}

fn free_ports() -> Result<[u16; 2], String> {
    // Hold both listeners at once so the OS can't hand out the same port twice.
    let pluto = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let mcp = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok([&pluto, &mcp].map(|l| l.local_addr().unwrap().port()))
}

fn check_version(julia: &str) -> Result<(), String> {
    let output = Command::new(julia).arg("--version").output().map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            format!(
                "Julia wasn't found at `{julia}`. In Settings, choose a julia binary \
                 or switch back to Endeavor's own Julia."
            )
        } else {
            format!("Couldn't run {julia}: {e}")
        }
    })?;
    let text = String::from_utf8_lossy(&output.stdout);
    match parse_version(&text) {
        Some(v) if v < MIN_JULIA => Err(format!(
            "Endeavor needs Julia {}.{} or newer; `{julia}` is {}.{}. Update it (e.g. `juliaup update`) \
             or choose a newer julia in Settings.",
            MIN_JULIA.0, MIN_JULIA.1, v.0, v.1
        )),
        _ => Ok(()),
    }
}

/// "julia version 1.12.6" -> (1, 12)
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().rsplit(' ').next()?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Mask Pluto's `secret=…` URL token.
fn redact_secret(line: &str) -> String {
    let Some(start) = line.find("secret=").map(|i| i + "secret=".len()) else { return line.to_string() };
    let end = line[start..].find(|c: char| !c.is_ascii_alphanumeric()).map_or(line.len(), |i| start + i);
    format!("{}…{}", &line[..start], &line[end..])
}

/// Plain-language cause for common failures in Julia's log, if recognized.
fn hint(log: &str) -> Option<&'static str> {
    if ["Could not resolve host", "failed to clone", "Couldn't connect", "network"]
        .iter()
        .any(|p| log.contains(p))
    {
        Some("It couldn't download packages; check the internet connection (the first launch installs Pluto and PlutoMCP).")
    } else if log.contains("Unsatisfiable requirements") {
        Some("Package versions in Endeavor's runtime environment conflict.")
    } else if log.contains("EADDRINUSE") || log.contains("Address already in use") {
        Some("A port it needs is already in use.")
    } else {
        None
    }
}

/// Plain-language cause for common failures, else the end of Julia's log.
fn diagnose(tail: &[String]) -> String {
    let hint = hint(&tail.join("\n"));
    let recent: Vec<&str> = tail.iter().rev().take(12).rev().map(String::as_str).collect();
    match hint {
        Some(hint) => hint.to_string(),
        None if recent.is_empty() => "It printed nothing.".to_string(),
        None => format!("Last output:\n{}", recent.join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::{diagnose, parse_version, redact_secret};

    #[test]
    fn redacts_pluto_secret() {
        let line = "│ Go to http://localhost:58250/?secret=wwD760ru in your browser";
        assert_eq!(redact_secret(line), "│ Go to http://localhost:58250/?secret=… in your browser");
        assert_eq!(redact_secret("no token here"), "no token here");
    }

    #[test]
    fn parses_julia_version() {
        assert_eq!(parse_version("julia version 1.12.6\n"), Some((1, 12)));
        assert_eq!(parse_version("julia version 1.10.0-rc1"), Some((1, 10)));
        assert!(parse_version("julia version 1.10.0").unwrap() < (1, 11));
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn diagnoses_common_failures_or_shows_the_log() {
        let lines = |s: &str| s.lines().map(String::from).collect::<Vec<_>>();
        assert!(diagnose(&lines("ERROR: Could not resolve host: github.com")).contains("internet"));
        assert!(diagnose(&lines("ERROR: Unsatisfiable requirements detected")).contains("conflict"));
        assert!(diagnose(&lines("IOError: listen: address already in use (EADDRINUSE)")).contains("port"));
        let other = diagnose(&lines("ERROR: LoadError: boom\nStacktrace: …"));
        assert!(other.starts_with("Last output:") && other.contains("boom"));
        assert_eq!(diagnose(&[]), "It printed nothing.");
    }
}

/// Live check of the recovery path (slow; starts Julia twice):
/// `cargo test -- --ignored live_die_and_restart --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn live_die_and_restart() {
    use futures::StreamExt;
    let (died, mut deaths) = futures::channel::mpsc::unbounded();
    let first = start(None, died.clone(), &|l| println!("{l}")).expect("start");
    // SIGKILL, like a crash or OOM kill. (SIGTERM can leave Julia hung mid-exit; the
    // app never sends it: quitting closes stdin and boot.jl exits itself.)
    let pattern = format!("boot.jl {} {}", first.ports[0], first.ports[1]);
    Command::new("pkill").args(["-9", "-f", &pattern]).status().unwrap();
    let reason = futures::executor::block_on(deaths.next()).expect("death reported");
    println!("died: {reason}");
    assert!(reason.starts_with("Julia exited"));

    let second = start(Some(first.ports), died, &|l| println!("{l}")).expect("restart on the same ports");
    assert_eq!(second.mcp_url, first.mcp_url, "agent's MCP URL must survive a restart");
    assert_ne!(second.pluto_url, first.pluto_url, "new Pluto secret");
    let list = crate::pluto::call_tool(&second.mcp_url, "list_notebooks", serde_json::json!({})).unwrap();
    println!("restarted; list_notebooks = {list}");
}

#[cfg(test)]
#[test]
fn a_missing_julia_points_to_settings() {
    let err = check_version("/nonexistent/julia").unwrap_err();
    println!("{err}");
    assert!(err.contains("wasn't found") && err.contains("Settings"));
}
