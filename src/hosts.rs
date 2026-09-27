//! The machines sessions can run on besides This Mac, kept in `hosts.json` in
//! Application Support. This Mac is implicit and never stored.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::settings::IdleStop;

const FILE: &str = "hosts.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hosts {
    pub servers: Vec<Server>,
}

/// A plain server reached over SSH, where the runtime runs as a detached process.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    /// Stable across renames; what sessions will refer to.
    pub id: String,
    pub name: String,
    /// An alias from `~/.ssh/config`, or `user@host`.
    pub ssh_host: String,
    pub port: Option<u16>,
    /// A julia path, or a shell line such as `module load julia`; unset looks
    /// on the login shell's PATH, then downloads Endeavor's own Julia.
    pub julia: Option<String>,
    /// Overrides Settings' "Stop idle notebooks after" for this server.
    pub idle_stop: Option<IdleStop>,
}

/// Which machine a session runs on.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Where {
    #[default]
    ThisMac,
    Server(String),
}

impl Hosts {
    pub fn load() -> Hosts {
        crate::install::app_dir().ok().map(|d| Hosts::load_from(&d.join(FILE))).unwrap_or_default()
    }

    fn load_from(path: &Path) -> Hosts {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = crate::install::app_dir()?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        self.save_to(&dir.join(FILE))
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
    }

    pub fn server(&self, id: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// Add `server`, or replace the one with its id.
    pub fn put(&mut self, server: Server) {
        match self.servers.iter_mut().find(|s| s.id == server.id) {
            Some(existing) => *existing = server,
            None => self.servers.push(server),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.servers.retain(|s| s.id != id);
    }
}

impl Server {
    pub fn new_id() -> String {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        format!("server-{nanos:x}")
    }

    /// The dialog's SSH host field: `host`, or `host:port`.
    pub fn ssh_target(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.ssh_host),
            None => self.ssh_host.clone(),
        }
    }

    /// Read the dialog's SSH host field back: a trailing `:port` is the port.
    pub fn parse_target(text: &str) -> Result<(String, Option<u16>), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Enter an SSH host: an alias from ~/.ssh/config, or user@host.".into());
        }
        if text.chars().any(char::is_whitespace) || text.starts_with('-') {
            return Err(format!("\"{text}\" isn't an SSH host name."));
        }
        match text.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() && !host.contains(':') => match port.parse::<u16>() {
                Ok(port) if port > 0 => Ok((host.to_owned(), Some(port))),
                _ => Err(format!("\"{port}\" isn't a port number.")),
            },
            _ => Ok((text.to_owned(), None)),
        }
    }

    /// The helper's Julia arguments for this server. A path to a julia binary
    /// (which may hold spaces) is used as is; anything else is a shell line.
    pub fn julia_args(&self) -> [String; 2] {
        let is_path = |j: &str| (j.starts_with('/') || j.starts_with("~/")) && j.rsplit('/').next().is_some_and(|name| name.starts_with("julia"));
        match self.julia.as_deref().map(str::trim).filter(|j| !j.is_empty()) {
            None => ["--julia".into(), "auto".into()],
            Some(path) if is_path(path) => ["--julia".into(), path.into()],
            Some(line) => ["--julia-shell".into(), line.replace('\n', "; ")],
        }
    }
}

/// `Host` names in `~/.ssh/config` (and the files it `Include`s by plain
/// path), without patterns, for the SSH host field's suggestions.
pub fn ssh_config_hosts() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else { return Vec::new() };
    let ssh = Path::new(&home).join(".ssh");
    let mut hosts = Vec::new();
    collect_hosts(&ssh.join("config"), &ssh, &mut hosts, 0);
    hosts
}

fn collect_hosts(path: &Path, ssh_dir: &Path, hosts: &mut Vec<String>, depth: usize) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for line in text.lines() {
        let line = line.trim();
        let (keyword, rest) = line.split_once(|c: char| c.is_whitespace() || c == '=').unwrap_or((line, ""));
        let words = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=').split_whitespace();
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                for name in words {
                    if !name.contains(['*', '?', '!']) && !hosts.iter().any(|h| h == name) {
                        hosts.push(name.to_owned());
                    }
                }
            }
            "include" if depth < 4 => {
                for file in words.filter(|f| !f.contains(['*', '?'])) {
                    let file = match file.strip_prefix("~/") {
                        Some(rest) => ssh_dir.parent().unwrap_or(ssh_dir).join(rest),
                        None => ssh_dir.join(file),
                    };
                    collect_hosts(&file, ssh_dir, hosts, depth + 1);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_servers() {
        let dir = std::env::temp_dir().join(format!("endeavor-hosts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        assert_eq!(Hosts::load_from(&path), Hosts::default());
        let mut hosts = Hosts::default();
        let lab = Server { id: "a".into(), name: "lab-server".into(), ssh_host: "lab".into(), idle_stop: Some(IdleStop::Week), ..Default::default() };
        hosts.put(lab.clone());
        hosts.put(Server { name: "renamed".into(), ..lab });
        hosts.put(Server { id: "b".into(), name: "other".into(), ssh_host: "jc@10.0.0.2".into(), port: Some(2222), ..Default::default() });
        hosts.save_to(&path).unwrap();
        let loaded = Hosts::load_from(&path);
        assert_eq!(loaded, hosts);
        assert_eq!(loaded.servers.len(), 2);
        assert_eq!(loaded.server("a").unwrap().name, "renamed");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"idle_stop\": \"week\"") && text.contains("\"port\": 2222"), "{text}");
        hosts.remove("a");
        assert_eq!(hosts.servers.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_the_ssh_host_field() {
        assert_eq!(Server::parse_target(" lab "), Ok(("lab".into(), None)));
        assert_eq!(Server::parse_target("jc@lab.example.edu:2222"), Ok(("jc@lab.example.edu".into(), Some(2222))));
        assert!(Server::parse_target("lab:ssh").is_err());
        assert!(Server::parse_target("").is_err());
        assert!(Server::parse_target("-oProxyCommand=x").is_err());
        assert!(Server::parse_target("two words").is_err());
        let s = Server { ssh_host: "lab".into(), port: Some(2222), ..Default::default() };
        assert_eq!(Server::parse_target(&s.ssh_target()), Ok(("lab".into(), Some(2222))));
    }

    #[test]
    fn julia_setting_becomes_helper_arguments() {
        let with = |j: Option<&str>| Server { julia: j.map(String::from), ..Default::default() }.julia_args();
        assert_eq!(with(None), ["--julia", "auto"]);
        assert_eq!(with(Some("  ")), ["--julia", "auto"]);
        assert_eq!(with(Some("/opt/julia/bin/julia")), ["--julia", "/opt/julia/bin/julia"]);
        assert_eq!(with(Some("~/julia-1.11/bin/julia")), ["--julia", "~/julia-1.11/bin/julia"]);
        assert_eq!(with(Some("/Users/jc/Library/Application Support/julia/bin/julia")), ["--julia", "/Users/jc/Library/Application Support/julia/bin/julia"]);
        assert_eq!(with(Some("module load julia/1.11")), ["--julia-shell", "module load julia/1.11"]);
        assert_eq!(with(Some("/opt/lmod/setup.sh && module load julia")), ["--julia-shell", "/opt/lmod/setup.sh && module load julia"]);
    }

    #[test]
    fn suggests_plain_hosts_from_ssh_config() {
        let dir = std::env::temp_dir().join(format!("endeavor-sshconfig-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();
        std::fs::write(
            dir.join("config"),
            "Include conf.d/lab\nInclude conf.d/*\n\nHost *\n  ForwardAgent no\nHost hoffman2 h2\n  HostName hoffman2.idre.ucla.edu\nhost=gpu-box\nHost *.cluster !bad lab-server\nMatch host x\n",
        )
        .unwrap();
        std::fs::write(dir.join("conf.d/lab"), "Host lab-server\nHost bench\n").unwrap();
        let mut hosts = Vec::new();
        collect_hosts(&dir.join("config"), &dir, &mut hosts, 0);
        assert_eq!(hosts, ["lab-server", "bench", "hoffman2", "h2", "gpu-box"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
