//! The Docker Compose stack: where its files live and how to run `docker compose` on it.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::host;

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

/// The services that use `PUBLIC_IP`, restarted when it changes.
pub const READDRESSED: [&str; 3] = ["char", "map", "portal"];

/// The services that publish ports, all restarted when `BIND_IP` changes.
pub const PUBLISHED: [&str; 5] = ["login", "char", "map", "web", "portal"];

/// The settings template, compiled in: Tauri's resource bundling skips dotfiles.
/// The database with the characters of the game mode `.env` in `dir` sets
/// (`PRERE`), one per mode; db-init makes it on the first start in that mode.
pub fn chars_db(dir: &Path) -> &'static str {
    let renewal = crate::envfile::EnvFile::load(&dir.join(".env"))
        .is_ok_and(|env| env.get("PRERE").as_deref() == Some("no"));
    if renewal { "ragnarok_re" } else { "ragnarok_prere" }
}

pub const ENV_EXAMPLE: &str = include_str!("../../../.env.example");

/// Copies the stack files bundled with the app (compose.yaml, docker/,
/// portal/, the overrides, patches and mods READMEs) and `.env.example`
/// into `dest`, replacing older copies. `.env`, the user's override files,
/// published patches and mods are never bundled, so they survive app updates.
pub fn install(bundled: &Path, dest: &Path) -> std::io::Result<()> {
    // The build folders are replaced whole, so a file an older version had
    // doesn't end up in an image or change its prebuilt image's name.
    for dir in ["docker", "portal"] {
        match fs::remove_dir_all(dest.join(dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
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
    let mut cmd = host::command("docker");
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

/// How long a quick Docker call (a status, an inspect, a config) may take
/// before it's given up on, so a hung Docker daemon can't hang the app.
pub const QUICK: Duration = Duration::from_secs(60);

/// Runs a command to completion and returns stdout, or stderr as the error.
/// Gives up after [`QUICK`].
pub fn output(cmd: Command) -> Result<String, String> {
    output_within(cmd, QUICK)
}

/// [`output`] for a command that may take up to `limit`. One that takes
/// longer is killed and reported as not answering.
pub fn output_within(mut cmd: Command, limit: Duration) -> Result<String, String> {
    let what = std::iter::once(cmd.get_program())
        .chain(cmd.get_args().take(if cmd.get_program() == "docker" { 2 } else { 1 }))
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "Docker isn't installed (no `docker` command found).".to_owned(),
            _ => e.to_string(),
        })?;
    // Read from threads, so a full pipe can't stall the command.
    let read = |stream: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = read(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let stderr = read(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let deadline = Instant::now() + limit;
    let mut pause = Duration::from_millis(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // The readers are left behind: a process the command
                // started may still hold the pipes open.
                return Err(format!("`{what}` didn't answer within {} seconds. Is Docker stuck?", limit.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if status.success() {
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&stderr).trim().to_owned())
    }
}

/// Checks that Docker and its Compose plugin are usable by this user.
pub fn check_docker() -> Result<String, String> {
    let mut v = host::command("docker");
    v.args(["compose", "version", "--short"]);
    let version = output(v).map_err(|e| format!("Docker Compose isn't available: {e}"))?;
    if too_old(&version) {
        return Err(format!(
            "Docker Compose {} is too old: the server needs 2.24 or newer to read its compose.yaml. \
             Update the Docker Compose plugin (docker-compose-plugin from Docker's own packages).",
            version.trim()
        ));
    }

    let mut info = host::command("docker");
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

/// Whether `docker compose version --short` names a version before 2.24,
/// which can't read `compose.yaml`'s `!reset`. A version it can't read
/// (a distro's own numbering) is given the benefit of the doubt.
fn too_old(version: &str) -> bool {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let mut number = || -> Option<u32> {
        let part = parts.next()?;
        let digits = part.find(|c: char| !c.is_ascii_digit()).map_or(part, |end| &part[..end]);
        digits.parse().ok()
    };
    match (number(), number()) {
        (Some(major), Some(minor)) => (major, minor) < (2, 24),
        _ => false,
    }
}

pub fn volume_exists(name: &str) -> bool {
    let mut inspect = host::command("docker");
    inspect.args(["volume", "inspect", &format!("{PROJECT}_{name}")]);
    output(inspect).is_ok()
}

/// The running map server's log since its container last started, kept
/// between reads so each read only fetches the lines added since the last.
#[derive(Default)]
pub struct MapLog {
    /// The container and when it started, which the text belongs to.
    run: String,
    /// The newest line's timestamp, which the next read starts from.
    last: String,
    text: String,
}

/// The running map server's log since it last started, or `None` when it
/// isn't running. The Mods screen reads it every few seconds, so only new
/// lines are fetched.
pub fn map_log(dir: &Path, cache: &std::sync::Mutex<MapLog>) -> Option<String> {
    let mut ps = compose(dir);
    ps.args(["ps", "--quiet", "map"]);
    let container = output(ps).ok()?.trim().to_owned();
    if container.is_empty() {
        return None;
    }
    let mut inspect = host::command("docker");
    inspect.args(["inspect", "--format", "{{.State.Running}} {{.State.StartedAt}}", &container]);
    let state = output(inspect).ok()?;
    let started = state.trim().strip_prefix("true ")?.to_owned();
    let run = format!("{container} {started}");

    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if cache.run != run {
        *cache = MapLog { run, last: started.clone(), text: String::new() };
    }
    let mut logs = compose(dir);
    logs.args(["logs", "--no-log-prefix", "--timestamps", "--since", &cache.last, "map"]);
    // On a failed read, what was read before still stands.
    if let Ok(new) = output(logs) {
        let MapLog { last, text, .. } = &mut *cache;
        append_log(&new, last, text);
    }
    Some(cache.text.clone())
}

/// Adds the lines of `new`, a timestamped `logs` read, that are newer than
/// `last` to `text`, without their timestamps, and moves `last` to the
/// newest. `--since` includes the line at `last`, which was already added.
/// Stdout and stderr lines can come slightly out of time order, so only
/// `last` from the read before decides what's new.
fn append_log(new: &str, last: &mut String, text: &mut String) {
    let parse = |t: &str| chrono::DateTime::parse_from_rfc3339(t).ok();
    let after = parse(last);
    let mut newest = after;
    for line in new.lines() {
        let (stamp, rest) = line.split_once(' ').unwrap_or((line, ""));
        let Some(at) = parse(stamp) else {
            // Not a timestamp: part of the line before.
            text.push_str(line);
            text.push('\n');
            continue;
        };
        if after.is_some_and(|after| at <= after) {
            continue;
        }
        if newest.is_none_or(|newest| at > newest) {
            newest = Some(at);
            stamp.clone_into(last);
        }
        text.push_str(rest);
        text.push('\n');
    }
}

/// Whether the running map server is on an older rAthena image than the
/// one now built for `packetver`, after a rebuild or server update.
pub fn runs_old_image(dir: &Path, packetver: &str) -> bool {
    runs_other_image(dir, "map", &format!("rag-wrapper/rathena:{packetver}"))
}

/// Whether the running portal is on an older image than the one now
/// built, after a Rebuild that changed only the portal.
pub fn portal_runs_old_image(dir: &Path) -> bool {
    runs_other_image(dir, "portal", "rag-wrapper/portal")
}

fn runs_other_image(dir: &Path, service: &str, image: &str) -> bool {
    let mut ps = compose(dir);
    ps.args(["ps", "--quiet", service]);
    let Some(container) = output(ps).ok().filter(|c| !c.trim().is_empty()) else {
        return false;
    };
    let mut running = host::command("docker");
    running.args(["inspect", "--format", "{{.Image}}", container.trim()]);
    let mut built = host::command("docker");
    built.args(["image", "inspect", "--format", "{{.Id}}", image]);
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

/// What each service publishes, from the stack's compose config:
/// (host port, service).
fn published_ports(dir: &Path) -> Result<Vec<(u16, String)>, String> {
    let mut config = compose(dir);
    config.args(["config", "--format", "json"]);
    Ok(parse_published(&output(config)?))
}

fn parse_published(json: &str) -> Vec<(u16, String)> {
    let Ok(config) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut ports: Vec<(u16, String)> = config["services"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(service, def)| {
            def["ports"].as_array().into_iter().flatten().filter_map(move |p| {
                let published = match &p["published"] {
                    serde_json::Value::String(s) => s.parse().ok(),
                    v => v.as_u64().and_then(|n| u16::try_from(n).ok()),
                }?;
                Some((published, service.clone()))
            })
        })
        .collect();
    ports.sort();
    ports.dedup();
    ports
}

/// The host ports in a `docker ps` Ports column, such as
/// `0.0.0.0:8080->8080/tcp, [::]:8080->8080/tcp`.
fn parse_ps_ports(column: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for mapping in column.split(", ") {
        let Some((host, _)) = mapping.split_once("->") else {
            continue;
        };
        let Some((_, range)) = host.rsplit_once(':') else {
            continue;
        };
        let (first, last) = range.split_once('-').unwrap_or((range, range));
        if let (Ok(first), Ok(last)) = (first.parse::<u16>(), last.parse::<u16>()) {
            ports.extend(first..=last);
        }
    }
    ports
}

/// Before a start: finds a port the stack publishes that something else
/// already holds, and says what holds it and how to free it, instead of
/// Docker's "address already in use" halfway through the start.
pub fn check_ports(dir: &Path) -> Result<(), String> {
    // With "This network only" on, ports are published on that one address,
    // which Docker can't do while the computer doesn't have it.
    let bind = crate::envfile::EnvFile::load(&dir.join(".env"))
        .ok()
        .and_then(|env| env.get("BIND_IP"))
        .filter(|ip| !ip.is_empty());
    let host: std::net::IpAddr = match &bind {
        Some(ip) => {
            let ip = ip.parse().map_err(|_| format!("BIND_IP in .env, \"{ip}\", isn't an IPv4 address."))?;
            missing_address(ip)?;
            ip
        }
        None => std::net::Ipv4Addr::UNSPECIFIED.into(),
    };
    // Without the config there's nothing to check; `up` reports the problem.
    let Ok(wanted) = published_ports(dir) else {
        return Ok(());
    };
    let mut ps = host::command("docker");
    ps.args(["ps", "--format", "{{.Names}}\t{{.Label \"com.docker.compose.project\"}}\t{{.Ports}}"]);
    let containers = output(ps).unwrap_or_default();
    let holders: Vec<(&str, &str, Vec<u16>)> = containers
        .lines()
        .filter_map(|l| {
            let mut cols = l.splitn(3, '\t');
            Some((cols.next()?, cols.next()?, parse_ps_ports(cols.next().unwrap_or_default())))
        })
        .collect();

    for (port, service) in wanted {
        let what = match service.as_str() {
            "portal" => "the account portal",
            "login" => "the login server",
            "char" => "the char server",
            "map" => "the map server",
            "web" => "rAthena's web server",
            other => other,
        };
        let fix = if service == "portal" {
            " or give the portal another port under Connecting"
        } else {
            ""
        };
        match holders.iter().find(|(_, _, ports)| ports.contains(&port)) {
            // This stack's own container; `up` replaces it as needed.
            Some((_, project, _)) if *project == PROJECT => {}
            Some((name, _, _)) => {
                return Err(format!(
                    "Port {port}, for {what}, is taken by the Docker container {name}, which isn't part of this \
                     server. Stop it (`docker stop {name}`){fix}, then start again."
                ))
            }
            None => {
                if let Err(e) = std::net::TcpListener::bind((host, port)) {
                    if e.kind() == std::io::ErrorKind::AddrInUse {
                        return Err(format!(
                            "Port {port}, for {what}, is in use by another program on this computer. \
                             `sudo ss -ltnp 'sport = :{port}'` shows which one. Stop it{fix}, then start again."
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Fails when this computer doesn't have `ip`, such as a VPN address while
/// the VPN is down or a LAN address DHCP has since given to another machine.
fn missing_address(ip: std::net::IpAddr) -> Result<(), String> {
    match std::net::TcpListener::bind((ip, 0)) {
        Err(e) if e.kind() == std::io::ErrorKind::AddrNotAvailable => Err(format!(
            "\"This network only\" is on, but this computer doesn't have the address {ip} right now. \
             Connect to that network or VPN, pick another address under Connecting, or switch \"This network only\" off, \
             then start again."
        )),
        _ => Ok(()),
    }
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
    let _ = host::command("kill").args(["-INT", &pid.to_string()]).status();
}

pub fn stack_dir_override() -> Option<PathBuf> {
    std::env::var_os("RAG_WRAPPER_STACK_DIR").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_before_2_24_is_too_old() {
        for old in ["2.20.2", "v2.23.3", "2.20.2+ds1-0ubuntu1~22.04.1", "1.29.2\n"] {
            assert!(too_old(old), "{old}");
        }
        for new in ["2.24.0", "v2.24.6", "2.40.3-desktop.1", "5.6.0\n", "10.0", "", "unknown"] {
            assert!(!too_old(new), "{new}");
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn install_replaces_the_stack_files_and_keeps_the_user_s() {
        let root = std::env::temp_dir().join(format!("stack-install-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (bundled, dest) = (root.join("bundled"), root.join("stack"));
        write(&bundled.join("compose.yaml"), "new compose");
        write(&bundled.join("docker/rathena/entrypoint.sh"), "new entrypoint");
        write(&bundled.join("portal/main.go"), "new portal");
        write(&bundled.join("overrides/README.md"), "new readme");

        write(&dest.join("compose.yaml"), "old compose");
        write(&dest.join("docker/rathena/removed.sh"), "from an older version");
        write(&dest.join("portal/removed.go"), "from an older version");
        write(&dest.join(".env"), "DB_PASSWORD=mine");
        write(&dest.join("overrides/char_conf.txt"), "server_name: Mine");
        write(&dest.join("mods/enabled.txt"), "warper");
        write(&dest.join("patches/2026-10-15-1.thor"), "patch");

        install(&bundled, &dest).unwrap();

        let read = |rel: &str| fs::read_to_string(dest.join(rel)).unwrap();
        assert_eq!(read("compose.yaml"), "new compose");
        assert_eq!(read("docker/rathena/entrypoint.sh"), "new entrypoint");
        assert_eq!(read("overrides/README.md"), "new readme");
        assert_eq!(read(".env.example"), ENV_EXAMPLE);
        // The build folders are replaced whole, so their prebuilt image names match.
        assert!(!dest.join("docker/rathena/removed.sh").exists());
        assert!(!dest.join("portal/removed.go").exists());
        // What the user made survives an app update.
        assert_eq!(read(".env"), "DB_PASSWORD=mine");
        assert_eq!(read("overrides/char_conf.txt"), "server_name: Mine");
        assert_eq!(read("mods/enabled.txt"), "warper");
        assert_eq!(read("patches/2026-10-15-1.thor"), "patch");

        // A first install makes the folders the containers mount.
        let fresh = root.join("fresh");
        install(&bundled, &fresh).unwrap();
        for dir in [OVERRIDES, PATCHES, MODS] {
            assert!(fresh.join(dir).is_dir(), "{dir}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chars_db_follows_the_game_mode() {
        let dir = std::env::temp_dir().join(format!("stack-chars-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(chars_db(&dir), "ragnarok_prere");
        fs::write(dir.join(".env"), "PRERE=yes\n").unwrap();
        assert_eq!(chars_db(&dir), "ragnarok_prere");
        fs::write(dir.join(".env"), "PRERE=no\n").unwrap();
        assert_eq!(chars_db(&dir), "ragnarok_re");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_only_new_log_lines() {
        let (mut last, mut text) = ("2026-10-16T10:00:00.000000000Z".to_owned(), String::new());
        // Docker's own order: a stderr line can be stamped before the stdout line above it.
        append_log(
            "2026-10-16T10:00:01.634490356Z Mods enabled: a\n2026-10-16T10:00:01.634458660Z [Warning]: early\n2026-10-16T10:00:02Z [Error]: bad\n",
            &mut last,
            &mut text,
        );
        assert_eq!(text, "Mods enabled: a\n[Warning]: early\n[Error]: bad\n");
        assert_eq!(last, "2026-10-16T10:00:02Z");
        // The next read starts at the last line, which is already there.
        append_log("2026-10-16T10:00:02.000000000Z [Error]: bad\n2026-10-16T10:00:03.1Z ok\n", &mut last, &mut text);
        assert_eq!(text, "Mods enabled: a\n[Warning]: early\n[Error]: bad\nok\n");
    }

    #[test]
    fn gives_up_on_a_command_that_doesn_t_answer() {
        let mut sleep = Command::new("sleep");
        sleep.arg("5");
        let started = Instant::now();
        let err = output_within(sleep, Duration::from_millis(200)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2), "{err}");
        assert!(err.contains("`sleep 5` didn't answer"), "{err}");

        let mut echo = Command::new("sh");
        echo.args(["-c", "echo out; echo err >&2"]);
        assert_eq!(output(echo).unwrap(), "out\n");
        let mut fail = Command::new("sh");
        fail.args(["-c", "echo out; echo err >&2; exit 1"]);
        assert_eq!(output(fail).unwrap_err(), "err");
    }

    #[test]
    fn an_address_this_computer_lacks_stops_the_start() {
        assert!(missing_address("127.0.0.1".parse().unwrap()).is_ok());
        // TEST-NET-3, which no machine running the tests has.
        let err = missing_address("203.0.113.77".parse().unwrap()).unwrap_err();
        assert!(err.contains("doesn't have the address 203.0.113.77"), "{err}");
    }

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

    #[test]
    fn reads_published_ports() {
        let config = r#"{"services":{"portal":{"ports":[{"target":8080,"published":"9090","protocol":"tcp"}]},
            "login":{"ports":[{"target":6900,"published":"6900"}]},"db":{}}}"#;
        assert_eq!(parse_published(config), vec![(6900, "login".into()), (9090, "portal".into())]);
        assert!(parse_published("not json").is_empty());

        assert_eq!(parse_ps_ports("0.0.0.0:8080->8080/tcp, [::]:8080->8080/tcp"), vec![8080, 8080]);
        assert_eq!(parse_ps_ports(":::8080->8080/tcp, 3306/tcp"), vec![8080]);
        assert_eq!(parse_ps_ports("127.0.0.1:7000-7001->7000-7001/tcp"), vec![7000, 7001]);
        assert!(parse_ps_ports("").is_empty());
    }
}
