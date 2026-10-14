//! The live view on the main screen: who's online, how long the game
//! has been up, and each container's CPU and memory.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{host, jobs, stack};

/// The characters rAthena marks online. The char server sets the flag and
/// clears every one when it starts, so it's only meaningful while it runs.
/// Uses the database's own account, not root. Takes the characters
/// database as `$1`.
const PLAYERS: &str = r#"MYSQL_PWD="$MARIADB_PASSWORD" exec mariadb --user="$MARIADB_USER" --batch --skip-column-names "$1" \
  -e 'SELECT name, class, base_level, job_level, last_map FROM `char` WHERE online = 1 ORDER BY name'"#;

#[derive(Serialize, Default, Debug)]
pub struct Live {
    /// `None` while the char server or database isn't running, or the
    /// database couldn't be read.
    pub players: Option<Vec<Player>>,
    /// Seconds since the map server started; `None` while it isn't running.
    pub uptime: Option<i64>,
    /// One entry per running container.
    pub usage: Vec<Usage>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Player {
    pub name: String,
    pub class: String,
    pub base_level: u32,
    pub job_level: u32,
    pub map: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Usage {
    pub service: String,
    /// Percent of the whole machine's CPU. (`docker stats` counts percent
    /// of one core, which passes 100 on a multi-core machine.)
    pub cpu: f64,
    /// Bytes.
    pub memory: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Container {
    service: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Stats {
    name: String,
    #[serde(rename = "CPUPerc")]
    cpu_perc: String,
    mem_usage: String,
}

/// Reads everything at once; `docker stats` alone takes about two seconds
/// to sample, so the three queries run side by side.
pub fn read(dir: &Path) -> Live {
    let mut ps = stack::compose(dir);
    ps.args(["ps", "--format", "json"]);
    let running: Vec<Container> = stack::output(ps).map(|t| parse_lines(&t)).unwrap_or_default();
    if running.is_empty() {
        return Live::default();
    }
    let is_up = |s: &str| running.iter().any(|c| c.service == s);
    let name_of = |s: &str| running.iter().find(|c| c.service == s).map(|c| c.name.clone());

    std::thread::scope(|scope| {
        let players = scope.spawn(|| {
            if !is_up("db") || !is_up("char") {
                return None;
            }
            let mut cmd = stack::compose(dir);
            cmd.args(["exec", "-T", "db", "sh", "-c", PLAYERS, "sh", stack::chars_db(dir)]);
            stack::output(cmd).ok().map(|t| parse_players(&t))
        });
        let uptime = scope.spawn(|| {
            let mut cmd = host::command("docker");
            cmd.args(["inspect", "--format", "{{.State.StartedAt}}"]).arg(name_of("map")?);
            let started = stack::output(cmd).ok()?;
            uptime_since(started.trim(), Utc::now())
        });
        let usage = scope.spawn(|| {
            let mut cmd = host::command("docker");
            cmd.args(["stats", "--no-stream", "--format", "{{json .}}"])
                .args(running.iter().map(|c| &c.name));
            let stats: Vec<Stats> = stack::output(cmd).map(|t| parse_lines(&t)).unwrap_or_default();
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
            stats
                .into_iter()
                .filter_map(|s| {
                    let service = running.iter().find(|c| c.name == s.name)?.service.clone();
                    Some(Usage {
                        service,
                        cpu: s.cpu_perc.trim_end_matches('%').parse().unwrap_or(0.0) / cores,
                        memory: parse_size(s.mem_usage.split(" / ").next().unwrap_or_default()).unwrap_or(0),
                    })
                })
                .collect()
        });
        Live {
            players: players.join().unwrap_or(None),
            uptime: uptime.join().unwrap_or(None),
            usage: usage.join().unwrap_or_default(),
        }
    })
}

/// One JSON object per line, as `docker compose ps` and `docker stats`
/// print them with `--format json`; older Compose prints a single array.
fn parse_lines<T: for<'de> Deserialize<'de>>(text: &str) -> Vec<T> {
    let text = text.trim();
    if text.starts_with('[') {
        return serde_json::from_str(text).unwrap_or_default();
    }
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// `mariadb --batch` output: tab-separated, with tabs, newlines and
/// backslashes in values escaped.
fn parse_players(text: &str) -> Vec<Player> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let [name, class, base, job, map] = f[..] else {
                return None;
            };
            Some(Player {
                name: unescape(name),
                class: jobs::name(class.parse().ok()?),
                base_level: base.parse().ok()?,
                job_level: job.parse().ok()?,
                map: unescape(map),
            })
        })
        .collect()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// `docker stats` sizes such as `85.3MiB`, `512KiB` or `1.2GB`, in bytes.
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, unit) = s.split_at(split);
    let number: f64 = number.trim().parse().ok()?;
    let scale = match unit {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    Some((number * scale).round() as u64)
}

/// Seconds from Docker's `StartedAt` (RFC 3339, nanoseconds) until `now`.
fn uptime_since(started: &str, now: DateTime<Utc>) -> Option<i64> {
    let started = DateTime::parse_from_rfc3339(started).ok()?;
    Some((now - started.with_timezone(&Utc)).num_seconds().max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_players() {
        let text = "Alice\t4008\t99\t70\tprontera\nBob\\tby\t0\t1\t1\tnew_1-1\nbroken line\n";
        let players = parse_players(text);
        assert_eq!(
            players,
            vec![
                Player {
                    name: "Alice".into(),
                    class: "Lord Knight".into(),
                    base_level: 99,
                    job_level: 70,
                    map: "prontera".into(),
                },
                Player {
                    name: "Bob\tby".into(),
                    class: "Novice".into(),
                    base_level: 1,
                    job_level: 1,
                    map: "new_1-1".into(),
                },
            ]
        );
        assert_eq!(jobs::name(99999), "Class 99999");
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("85.5MiB"), Some(89_653_248));
        assert_eq!(parse_size("512KiB"), Some(524_288));
        assert_eq!(parse_size("1.5GB"), Some(1_500_000_000));
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("--"), None);
    }

    #[test]
    fn parses_stats_and_containers() {
        let stats: Vec<Stats> = parse_lines(
            r#"{"BlockIO":"0B / 0B","CPUPerc":"1.25%","Container":"abc","ID":"abc","MemPerc":"1.1%","MemUsage":"85.5MiB / 7.6GiB","Name":"rag-wrapper-map-1","NetIO":"1kB / 2kB","PIDs":"9"}"#,
        );
        assert_eq!(stats[0].name, "rag-wrapper-map-1");
        assert_eq!(stats[0].cpu_perc, "1.25%");
        let ps: Vec<Container> = parse_lines(r#"[{"Service":"db","Name":"rag-wrapper-db-1","State":"running"}]"#);
        assert_eq!(ps[0].service, "db");
    }

    #[test]
    fn computes_uptime() {
        let now = DateTime::parse_from_rfc3339("2026-10-14T12:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(uptime_since("2026-10-14T10:59:30.123456789Z", now), Some(3629));
        assert_eq!(uptime_since("garbage", now), None);
    }
}
