//! Database backups: gzipped `mariadb-dump`s of the `ragnarok` database in
//! the app's backups folder, the newest few kept, and loading one back.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::Stdio;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;

use crate::stack;
use crate::task::Ctx;

/// How many backups of each kind are kept, made by hand and made
/// automatically; each new one deletes the oldest of its kind beyond this.
pub const KEEP: usize = 10;

const PREFIX: &str = "ragnarok-";
const SUFFIX: &str = ".sql.gz";

/// Most of rAthena's tables are MyISAM, so a consistent dump of a running
/// server needs a global read lock rather than `--single-transaction`.
/// Writes pause for the few seconds the dump takes.
///
/// MyISAM tables left open by an unclean shutdown are marked crashed.
/// MariaDB repairs them when they're next opened, but not under the dump's
/// read lock, so the dump first repairs any that weren't closed properly.
/// Its report goes to stderr, keeping stdout for the dump.
const DUMP: &str = r#"export MYSQL_PWD="$MARIADB_ROOT_PASSWORD"
mariadb-check --user=root --fast --auto-repair --silent ragnarok >&2 || exit
exec mariadb-dump --user=root --lock-all-tables --routines --events --triggers ragnarok"#;
const LOAD: &str = r#"MYSQL_PWD="$MARIADB_ROOT_PASSWORD" exec mariadb --user=root"#;

/// mariadb-dump's last line; a dump without it was cut short.
const COMPLETE: &[u8] = b"-- Dump completed";

/// Why a backup was made. Part of its file name.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Reason {
    Manual,
    Rebuild,
    /// Before a server update moves the rAthena commit.
    Update,
    BeforeRestore,
    /// Before the game servers first load a mod's database files.
    Mods,
    /// On stop or close, when there's no backup from the last day.
    Daily,
}

impl Reason {
    fn slug(self) -> &'static str {
        match self {
            Reason::Manual => "manual",
            Reason::Rebuild => "rebuild",
            Reason::Update => "update",
            Reason::BeforeRestore => "before-restore",
            Reason::Mods => "mods",
            Reason::Daily => "daily",
        }
    }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Backup {
    pub file: String,
    /// Local time, "YYYY-MM-DD HH:MM:SS".
    pub created: String,
    /// manual, rebuild, update, before-restore, mods or daily; anything else a later version wrote.
    pub reason: String,
    pub size: u64,
}

/// `ragnarok-20261014-075800-manual.sql.gz`: sorts by time, oldest first.
fn file_name(stamp: &str, reason: Reason) -> String {
    format!("{PREFIX}{stamp}-{}{SUFFIX}", reason.slug())
}

/// The time and reason in a backup's file name, or `None` for any other file.
fn parse(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (date, rest) = rest.split_once('-')?;
    let (time, reason) = rest.split_once('-')?;
    let digits = |s: &str, n| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(date, 8) || !digits(time, 6) || reason.is_empty() {
        return None;
    }
    let created = format!(
        "{}-{}-{} {}:{}:{}",
        &date[..4],
        &date[4..6],
        &date[6..],
        &time[..2],
        &time[2..4],
        &time[4..]
    );
    Some((created, reason.to_owned()))
}

/// The backups in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<Backup> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut backups: Vec<Backup> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let file = e.file_name().into_string().ok()?;
            let (created, reason) = parse(&file)?;
            let size = e.metadata().ok().filter(|m| m.is_file())?.len();
            Some(Backup {
                file,
                created,
                reason,
                size,
            })
        })
        .collect();
    backups.sort_by(|a, b| b.file.cmp(&a.file));
    backups
}

/// Deletes all but the newest `KEEP` backups made by hand and the newest
/// `KEEP` made automatically, sparing `spare`. Automatic backups never
/// push out one made by hand.
fn prune(dir: &Path, spare: Option<&str>) {
    let (manual, automatic): (Vec<_>, Vec<_>) =
        list(dir).into_iter().partition(|b| b.reason == Reason::Manual.slug());
    for old in [manual, automatic].into_iter().flat_map(|kind| kind.into_iter().skip(KEEP)) {
        if Some(old.file.as_str()) != spare {
            let _ = fs::remove_file(dir.join(&old.file));
        }
    }
}

fn db_running(dir: &Path) -> bool {
    stack::status(dir)
        .unwrap_or_default()
        .iter()
        .any(|s| s.service == "db" && s.state == "running")
}

/// Starts the database if it isn't running and waits until it's healthy.
/// Returns whether it was already running.
fn ensure_db(ctx: &Ctx) -> Result<bool, String> {
    let was_running = db_running(&ctx.dir);
    let mut up = ctx.compose();
    up.args(["--progress", "plain", "up", "--detach", "--wait", "db"]);
    ctx.run(up)?;
    Ok(was_running)
}

fn stop_db(ctx: &Ctx) {
    let mut stop = ctx.compose();
    stop.args(["stop", "db"]);
    let _ = ctx.run(stop);
}

/// Dumps the database into a new backup, starting MariaDB for it if the
/// server is stopped, then deletes the oldest beyond `KEEP`. Returns the
/// file name, or `None` when there is no database yet.
/// Backs up a running database when no backup of any kind was made in
/// the last 24 hours. Used on stop and close, in place of a schedule.
/// A failure is reported but doesn't stop the server from going down.
pub fn daily(ctx: &Ctx) {
    if !db_running(&ctx.dir) {
        return;
    }
    let newest = list(&ctx.backups).into_iter().find_map(|b| {
        chrono::NaiveDateTime::parse_from_str(&b.created, "%Y-%m-%d %H:%M:%S").ok()
    });
    if !due(newest, chrono::Local::now().naive_local()) {
        return;
    }
    if let Err(e) = create(ctx, Reason::Daily, None) {
        ctx.line(e);
    }
}

fn due(newest: Option<chrono::NaiveDateTime>, now: chrono::NaiveDateTime) -> bool {
    newest.is_none_or(|t| now - t >= chrono::TimeDelta::hours(24))
}

pub fn create(ctx: &Ctx, reason: Reason, spare: Option<&str>) -> Result<Option<String>, String> {
    if !stack::volume_exists("db-data") {
        ctx.line("There's no database yet, so there's nothing to back up.");
        return Ok(None);
    }
    ctx.line("Backing up the database…");
    let was_running = ensure_db(ctx)?;
    let result = dump(ctx, reason);
    if !was_running {
        stop_db(ctx);
    }
    let file = result?;
    prune(&ctx.backups, spare);
    Ok(Some(file))
}

fn start_servers(ctx: &Ctx) -> Result<(), String> {
    ctx.line("Starting the server again…");
    stack::check_ports(&ctx.dir)?;
    let mut up = ctx.compose();
    up.args(["--progress", "plain", "up", "--detach"]);
    ctx.run(up)
}

fn dump(ctx: &Ctx, reason: Reason) -> Result<String, String> {
    ctx.check_cancelled()?;
    fs::create_dir_all(&ctx.backups).map_err(|e| format!("Couldn't create the backups folder: {e}"))?;
    let mut file = file_name(&chrono::Local::now().format("%Y%m%d-%H%M%S").to_string(), reason);
    while ctx.backups.join(&file).exists() {
        // Two backups in the same second; the second waits for its own name.
        std::thread::sleep(std::time::Duration::from_millis(250));
        file = file_name(&chrono::Local::now().format("%Y%m%d-%H%M%S").to_string(), reason);
    }
    let path = ctx.backups.join(&file);
    let partial = ctx.backups.join(format!(".{file}.partial"));

    let result = dump_to(ctx, &partial).and_then(|size| {
        fs::rename(&partial, &path).map_err(|e| e.to_string())?;
        Ok(size)
    });
    match result {
        Ok(size) => {
            ctx.line(format!("Saved {file} ({}).", human_size(size)));
            Ok(file)
        }
        Err(e) => {
            let _ = fs::remove_file(&partial);
            Err(format!("The backup failed: {e}"))
        }
    }
}

/// Streams `mariadb-dump` through gzip into `dest`. Returns the file's size.
fn dump_to(ctx: &Ctx, dest: &Path) -> Result<u64, String> {
    let mut cmd = ctx.compose();
    cmd.args(["exec", "-T", "db", "sh", "-c", DUMP])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    ctx.track(Some(child.id()));
    let errors = child.stderr.take().map(|stderr| {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for line in io::BufRead::lines(BufReader::new(stderr)).map_while(Result::ok) {
                ctx.line(line);
            }
        })
    });

    let mut stdout = child.stdout.take().unwrap();
    let copied = (|| -> io::Result<bool> {
        let mut gz = GzEncoder::new(BufWriter::new(File::create(dest)?), Compression::default());
        let mut tail = Tail::default();
        let mut buf = vec![0; 64 * 1024];
        loop {
            let n = stdout.read(&mut buf)?;
            if n == 0 {
                break;
            }
            tail.push(&buf[..n]);
            gz.write_all(&buf[..n])?;
        }
        gz.finish()?.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(tail.complete())
    })();

    let status = child.wait().map_err(|e| e.to_string());
    ctx.track(None);
    if let Some(t) = errors {
        let _ = t.join();
    }
    ctx.check_cancelled()?;
    if !status?.success() {
        return Err("mariadb-dump stopped with an error.".into());
    }
    match copied {
        Ok(true) => fs::metadata(dest).map(|m| m.len()).map_err(|e| e.to_string()),
        Ok(false) => Err("the dump ended early.".into()),
        Err(e) => Err(format!("couldn't write the file: {e}")),
    }
}

/// Keeps the last bytes seen, to check a dump's final line.
#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    const LEN: usize = 256;

    fn push(&mut self, bytes: &[u8]) {
        self.0
            .extend_from_slice(&bytes[bytes.len().saturating_sub(Self::LEN)..]);
        let excess = self.0.len().saturating_sub(Self::LEN);
        self.0.drain(..excess);
    }

    fn complete(&self) -> bool {
        let text = self.0.trim_ascii_end();
        let last = text.rsplit(|&b| b == b'\n').next().unwrap_or_default();
        last.starts_with(COMPLETE)
    }
}

/// Reads a backup through, checking it is a whole dump.
fn check(path: &Path) -> Result<(), String> {
    let mut gz = GzDecoder::new(BufReader::new(File::open(path).map_err(|e| e.to_string())?));
    let mut tail = Tail::default();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = gz.read(&mut buf).map_err(|e| format!("the file is damaged ({e})."))?;
        if n == 0 {
            break;
        }
        tail.push(&buf[..n]);
    }
    if tail.complete() {
        Ok(())
    } else {
        Err("the file doesn't hold a whole database dump.".into())
    }
}

/// Replaces the database with a backup: checks the file, stops everything
/// that uses the database, backs up the current database, loads the dump
/// into an emptied `ragnarok` database, and starts the server again if it
/// was running.
pub fn restore(ctx: &Ctx, file: &str) -> Result<(), String> {
    if parse(file).is_none() {
        return Err(format!("{file} isn't a backup."));
    }
    let path = ctx.backups.join(file);
    ctx.line(format!("Checking {file}…"));
    check(&path).map_err(|e| format!("Can't restore {file}: {e}"))?;

    let services = stack::status(&ctx.dir)?;
    let was_serving = services
        .iter()
        .any(|s| stack::GAME_SERVERS.contains(&s.service.as_str()) && s.state == "running");
    let users: Vec<&str> = services
        .iter()
        .map(|s| s.service.as_str())
        .filter(|s| !["db", "db-init"].contains(s))
        .collect();
    ctx.line("Stopping the servers…");
    let mut stop = ctx.compose();
    stop.arg("stop").args(&users);
    ctx.run(stop)?;

    // Backed up after the servers stop, so it has their last saves.
    let had_db = stack::volume_exists("db-data");
    let db_was_running = ensure_db(ctx)?;
    if had_db {
        ctx.line("Backing up the current database first…");
        if let Err(e) = dump(ctx, Reason::BeforeRestore) {
            if was_serving {
                let _ = start_servers(ctx);
            }
            return Err(format!("{e} Nothing was restored."));
        }
        prune(&ctx.backups, Some(file));
    }

    ctx.line(format!("Loading {file}…"));
    if let Err(e) = load(ctx, &path) {
        return Err(format!(
            "Loading failed: {e} The database may be incomplete, so the servers stay stopped. \
             Restore the \"before a restore\" backup to go back."
        ));
    }
    ctx.line("Restored.");

    if was_serving {
        start_servers(ctx)?;
    } else if !db_was_running {
        stop_db(ctx);
    }
    Ok(())
}

fn load(ctx: &Ctx, path: &Path) -> Result<(), String> {
    ctx.check_cancelled()?;
    let mut cmd = ctx.compose();
    cmd.args(["exec", "-T", "db", "sh", "-c", LOAD]).stdin(Stdio::piped());
    let lines = ctx.clone();
    let (mut child, done) = stack::spawn_lines(cmd, move |line| lines.line(line)).map_err(|e| e.to_string())?;
    ctx.track(Some(child.id()));
    let mut stdin = child.stdin.take().unwrap();
    let fed = (|| -> io::Result<()> {
        // Tables made after the backup would otherwise survive the restore.
        stdin.write_all(b"DROP DATABASE IF EXISTS ragnarok;\nCREATE DATABASE ragnarok;\nUSE ragnarok;\n")?;
        io::copy(&mut GzDecoder::new(BufReader::new(File::open(path)?)), &mut stdin)?;
        Ok(())
    })();
    drop(stdin);
    let _ = done.join();
    let status = child.wait().map_err(|e| e.to_string());
    ctx.track(None);
    if !status?.success() {
        return Err("MariaDB stopped with an error.".into());
    }
    fed.map_err(|e| e.to_string())
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / (1 << 10) as f64),
        b => format!("{b} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        let name = file_name("20261014-075800", Reason::BeforeRestore);
        assert_eq!(name, "ragnarok-20261014-075800-before-restore.sql.gz");
        assert_eq!(
            parse(&name),
            Some(("2026-10-14 07:58:00".to_owned(), "before-restore".to_owned()))
        );
        for other in [
            "ragnarok-20261014-0758-manual.sql.gz",
            "ragnarok-20261014-075800-.sql.gz",
            ".ragnarok-20261014-075800-manual.sql.gz.partial",
            "../ragnarok-20261014-075800-manual.sql.gz",
            "notes.txt",
        ] {
            assert_eq!(parse(other), None, "{other}");
        }
    }

    #[test]
    fn keeps_the_newest_and_spares_one() {
        let dir = std::env::temp_dir().join(format!("rag-wrapper-backups-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for day in 1..=12 {
            let stamp = format!("202610{day:02}-120000");
            fs::write(dir.join(file_name(&stamp, Reason::Manual)), b"x").unwrap();
        }
        for day in 1..=11 {
            let stamp = format!("202609{day:02}-120000");
            fs::write(dir.join(file_name(&stamp, Reason::Rebuild)), b"x").unwrap();
        }
        fs::write(dir.join("notes.txt"), b"mine").unwrap();
        let oldest = file_name("20261001-120000", Reason::Manual);

        prune(&dir, Some(&oldest));
        let left = list(&dir);
        assert_eq!(left.len(), 2 * KEEP + 1);
        assert!(!dir.join(file_name("20260901-120000", Reason::Rebuild)).exists());
        assert!(dir.join(file_name("20260902-120000", Reason::Rebuild)).exists());
        let left: Vec<_> = left.into_iter().filter(|b| b.reason == "manual").collect();
        assert_eq!(left[0].file, file_name("20261012-120000", Reason::Manual));
        assert_eq!(left[KEEP].file, oldest);
        assert!(!dir.join(file_name("20261002-120000", Reason::Manual)).exists());
        assert!(dir.join("notes.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn daily_is_due_after_a_day() {
        let at = |s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let now = at("2026-10-15 12:00:00");
        assert!(due(None, now));
        assert!(!due(Some(at("2026-10-14 12:00:01")), now));
        assert!(due(Some(at("2026-10-14 12:00:00")), now));
    }

    #[test]
    fn spots_a_cut_dump() {
        let mut tail = Tail::default();
        tail.push(b"INSERT INTO login VALUES (1);\n");
        assert!(!tail.complete());
        tail.push(&[b'x'; 1000]);
        tail.push(b"\n-- Dump completed on 2026-10-14  7:58:00\n");
        assert!(tail.complete());
        tail.push(b"\n\n");
        assert!(tail.complete());
    }
}
