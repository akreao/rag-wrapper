//! The Docker Compose stack: where its files live and how to run `docker compose` on it.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;

use serde::{Deserialize, Serialize};

/// Fixed so the containers and the `db-data` volume keep their names
/// wherever the stack files live.
pub const PROJECT: &str = "rag-wrapper";

/// The folder of rAthena config overrides, mounted into the rAthena
/// containers as `conf/user`. Kept across app updates like `.env`.
pub const OVERRIDES: &str = "overrides";

/// The client patches the portal serves to the players' patcher. Kept
/// across app updates like the overrides.
pub const PATCHES: &str = "patches";

/// Server-side mods, mounted into the rAthena containers as `mods/`.
/// Kept across app updates like the overrides.
pub const MODS: &str = "mods";

/// The services that read the overrides, restarted to apply them.
pub const GAME_SERVERS: [&str; 3] = ["login", "char", "map"];

/// The settings template, compiled in: Tauri's resource bundling skips dotfiles.
pub const ENV_EXAMPLE: &str = include_str!("../../../.env.example");

/// Copies the stack files bundled with the app (compose.yaml, docker/,
/// portal/, the overrides, patches and mods READMEs) and `.env.example`
/// into `dest`, replacing older copies. `.env`, the user's override files,
/// published patches and mods are never bundled, so they survive app updates.
pub fn install(bundled: &Path, dest: &Path) -> std::io::Result<()> {
    copy_dir(bundled, dest)?;
    fs::create_dir_all(dest.join(OVERRIDES))?;
    fs::create_dir_all(dest.join(PATCHES))?;
    fs::create_dir_all(dest.join(MODS))?;
    fs::write(dest.join(".env.example"), ENV_EXAMPLE)
}

fn copy_dir(bundled: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(bundled)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn compose(dir: &Path) -> Command {
    let mut cmd = Command::new("docker");
    cmd.current_dir(dir)
        .args([
            "compose",
            "--ansi",
            "never",
            "--project-name",
            PROJECT,
            "--project-directory",
        ])
        .arg(dir)
        .arg("--file")
        .arg(dir.join("compose.yaml"))
        .stdin(Stdio::null());
    cmd
}

/// Runs a command to completion and returns stdout, or stderr as the error.
pub fn output(mut cmd: Command) -> Result<String, String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "Docker isn't installed (no `docker` command found).".to_owned(),
        _ => e.to_string(),
    })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Checks that Docker and its Compose plugin are usable by this user.
pub fn check_docker() -> Result<String, String> {
    let mut v = Command::new("docker");
    v.args(["compose", "version", "--short"]);
    let version = output(v).map_err(|e| format!("Docker Compose isn't available: {e}"))?;

    let mut info = Command::new("docker");
    info.args(["info", "--format", "{{.ServerVersion}}"]);
    output(info).map_err(|e| {
        if e.contains("permission denied") {
            "This user can't talk to Docker. Add it to the docker group \
             (`sudo usermod -aG docker $USER`) and log in again."
                .to_owned()
        } else {
            format!("The Docker daemon isn't reachable: {e}")
        }
    })?;
    Ok(version.trim().to_owned())
}

pub fn volume_exists(name: &str) -> bool {
    Command::new("docker")
        .args(["volume", "inspect", &format!("{PROJECT}_{name}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether the running map server is on an older rAthena image than the
/// one now built for `packetver`, after a rebuild or server update.
pub fn runs_old_image(dir: &Path, packetver: &str) -> bool {
    let mut ps = compose(dir);
    ps.args(["ps", "--quiet", "map"]);
    let Some(container) = output(ps).ok().filter(|c| !c.trim().is_empty()) else {
        return false;
    };
    let mut running = Command::new("docker");
    running.args(["inspect", "--format", "{{.Image}}", container.trim()]);
    let mut built = Command::new("docker");
    built.args(["image", "inspect", "--format", "{{.Id}}", &format!("rag-wrapper/rathena:{packetver}")]);
    match (output(running), output(built)) {
        (Ok(running), Ok(built)) => running.trim() != built.trim(),
        _ => false,
    }
}

#[derive(Serialize, Default, Clone)]
pub struct ServiceStatus {
    pub service: String,
    /// created, running, restarting, exited, ... or "absent" when there is no container.
    pub state: String,
    pub health: String,
    pub exit_code: i64,
    pub status: String,
    pub ports: Vec<u16>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PsEntry {
    service: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    health: String,
    #[serde(default)]
    exit_code: i64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    publishers: Option<Vec<Publisher>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Publisher {
    #[serde(default)]
    published_port: u16,
}

/// Every service in compose.yaml with its container's state, in file order.
pub fn status(dir: &Path) -> Result<Vec<ServiceStatus>, String> {
    let mut services = compose(dir);
    services.args(["config", "--services"]);
    let names = output(services)?;

    let mut ps = compose(dir);
    ps.args(["ps", "--all", "--format", "json"]);
    let entries = parse_ps(&output(ps)?);

    Ok(names
        .lines()
        .filter(|n| !n.is_empty())
        .map(|name| match entries.iter().find(|e| e.service == name) {
            Some(e) => {
                let mut ports: Vec<u16> = e
                    .publishers
                    .iter()
                    .flatten()
                    .map(|p| p.published_port)
                    .filter(|p| *p != 0)
                    .collect();
                ports.sort();
                ports.dedup();
                ServiceStatus {
                    service: name.to_owned(),
                    state: e.state.clone(),
                    health: e.health.clone(),
                    exit_code: e.exit_code,
                    status: e.status.clone(),
                    ports,
                }
            }
            None => ServiceStatus {
                service: name.to_owned(),
                state: "absent".to_owned(),
                ..Default::default()
            },
        })
        .collect())
}

/// `docker compose ps --format json` prints one object per line in current
/// versions and a single array in older ones.
fn parse_ps(text: &str) -> Vec<PsEntry> {
    let text = text.trim();
    if text.starts_with('[') {
        return serde_json::from_str(text).unwrap_or_default();
    }
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Starts `cmd` with stdout and stderr piped, calling `on_line` for each
/// line of either from reader threads; stdin is left as `cmd` sets it.
/// Returns the child and a handle that finishes once both streams close.
pub fn spawn_lines<F>(mut cmd: Command, on_line: F) -> std::io::Result<(Child, thread::JoinHandle<()>)>
where
    F: Fn(String) + Send + Sync + 'static,
{
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let on_line = std::sync::Arc::new(on_line);
    let readers: Vec<_> = [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ]
    .into_iter()
    .map(|stream| {
        let on_line = on_line.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut buf = Vec::new();
            while matches!(reader.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
                let line = String::from_utf8_lossy(&buf);
                // Progress output redraws with \r; keep the last state.
                let line = line.trim_end().rsplit('\r').next().unwrap_or_default();
                on_line(line.to_owned());
                buf.clear();
            }
        })
    })
    .collect();
    let done = thread::spawn(move || {
        for r in readers {
            let _ = r.join();
        }
    });
    Ok((child, done))
}

/// Asks a process to stop the way Ctrl+C would, so `docker compose` can
/// cancel a build or log stream cleanly.
pub fn interrupt(pid: u32) {
    let _ = Command::new("kill").args(["-INT", &pid.to_string()]).status();
}

pub fn stack_dir_override() -> Option<PathBuf> {
    std::env::var_os("RAG_WRAPPER_STACK_DIR").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_ps_formats() {
        let lines = r#"{"Service":"db","State":"running","Health":"healthy","ExitCode":0,"Publishers":[]}
{"Service":"login","State":"running","ExitCode":0,"Publishers":[{"URL":"0.0.0.0","TargetPort":6900,"PublishedPort":6900,"Protocol":"tcp"}]}"#;
        let v = parse_ps(lines);
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].publishers.as_ref().unwrap()[0].published_port, 6900);

        let array = r#"[{"Service":"db-init","State":"exited","ExitCode":0,"Publishers":null}]"#;
        assert_eq!(parse_ps(array)[0].service, "db-init");
        assert!(parse_ps("").is_empty());
    }
}
