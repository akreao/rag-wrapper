//! How many players were online over the last week, for the chart on the
//! Dashboard. The app counts them every few minutes while it runs, which
//! is whenever the server runs: closing the window stops the server.

use std::fs;
use std::path::Path;
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;

use crate::stack;

/// Beside the backups, not in the stack folder, so a reinstall of the
/// stack files keeps it.
pub const FILE: &str = "player-history.tsv";

/// How often the app counts. Five minutes keeps a day to 288 points.
pub const EVERY: Duration = Duration::from_secs(5 * 60);

/// Samples older than this are dropped.
const KEEP_SECS: i64 = 7 * 24 * 60 * 60;

/// Characters online in the server's game mode, the count the Dashboard's
/// Online list would show. Takes the characters database as `$1`.
const COUNT: &str = r#"MYSQL_PWD="$MARIADB_PASSWORD" exec mariadb --user="$MARIADB_USER" --batch --skip-column-names "$1" \
  -e 'SELECT COUNT(*) FROM `char` WHERE online = 1'"#;

#[derive(Serialize, Debug, PartialEq, Clone, Copy)]
pub struct Sample {
    /// Unix seconds.
    pub at: i64,
    pub players: u32,
}

/// Counts the players online now, or `None` while the char server or the
/// database isn't running: the char server clears every online flag when
/// it starts, so the flags mean nothing while it's down.
pub fn count(dir: &Path) -> Option<u32> {
    let up = stack::status(dir).ok()?;
    let running = |s: &str| up.iter().any(|x| x.service == s && x.state == "running");
    if !running("db") || !running("char") {
        return None;
    }
    let mut cmd = stack::compose(dir);
    cmd.args(["exec", "-T", "db", "sh", "-c", COUNT, "sh", stack::chars_db(dir)]);
    stack::output(cmd).ok()?.trim().parse().ok()
}

/// Counts the players and adds the sample to the file, dropping old ones.
/// Nothing is written while the server is off, which the chart shows as a gap.
pub fn sample(dir: &Path, file: &Path) {
    let Some(players) = count(dir) else { return };
    let now = Utc::now().timestamp();
    let mut samples = read(file, now);
    samples.push(Sample { at: now, players });
    let _ = write(file, &samples);
}

/// The samples from the last week, oldest first. A missing or damaged file
/// reads as no samples, or as the lines that still parse.
pub fn read(file: &Path, now: i64) -> Vec<Sample> {
    fs::read_to_string(file).map(|t| parse(&t, now)).unwrap_or_default()
}

fn parse(text: &str, now: i64) -> Vec<Sample> {
    let mut samples: Vec<Sample> = text
        .lines()
        .filter_map(|line| {
            let (at, players) = line.split_once('\t')?;
            Some(Sample { at: at.parse().ok()?, players: players.parse().ok()? })
        })
        .filter(|s| s.at > now - KEEP_SECS && s.at <= now)
        .collect();
    samples.sort_by_key(|s| s.at);
    samples
}

fn write(file: &Path, samples: &[Sample]) -> std::io::Result<()> {
    let text: String = samples.iter().map(|s| format!("{}\t{}\n", s.at, s.players)).collect();
    let tmp = file.with_extension("tsv.partial");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_week_in_order() {
        let now = 1_000_000;
        let text = format!(
            "{}\t3\n{}\t1\nbroken\n{}\t2\n{}\tx\n{}\t9\n",
            now - 60,
            now - KEEP_SECS - 1,
            now - 600,
            now - 300,
            now + 60,
        );
        assert_eq!(
            parse(&text, now),
            vec![Sample { at: now - 600, players: 2 }, Sample { at: now - 60, players: 3 }]
        );
    }

    #[test]
    fn writes_what_it_reads() {
        let dir = std::env::temp_dir().join(format!("rag-wrapper-history-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(FILE);
        let now = Utc::now().timestamp();
        let samples = vec![Sample { at: now - 300, players: 0 }, Sample { at: now, players: 4 }];
        write(&file, &samples).unwrap();
        assert_eq!(read(&file, now), samples);
        assert_eq!(read(&dir.join("missing.tsv"), now), Vec::new());
        fs::remove_dir_all(&dir).unwrap();
    }
}
