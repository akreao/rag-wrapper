//! Database backups: gzipped `mariadb-dump`s of the `ragnarok` databases in
//! the app's backups folder, the newest few kept, loading one back, and
//! test-loading one into a throwaway database to prove it would restore.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use crate::{host, stack};
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
///
/// It holds the accounts database and each game mode's characters
/// database there is so far, with the statements that create them.
const DUMP: &str = r#"export MYSQL_PWD="$MARIADB_ROOT_PASSWORD"
dbs=$(mariadb --user=root --skip-column-names -e "SELECT schema_name FROM information_schema.schemata
  WHERE schema_name IN ('ragnarok', 'ragnarok_prere', 'ragnarok_re') ORDER BY schema_name") || exit
mariadb-check --user=root --fast --auto-repair --silent --databases $dbs >&2 || exit
exec mariadb-dump --user=root --lock-all-tables --routines --events --triggers --databases $dbs"#;
const LOAD: &str = r#"MYSQL_PWD="$MARIADB_ROOT_PASSWORD" exec mariadb --user=root"#;

/// mariadb-dump's last line; a dump without it was cut short.
const COMPLETE: &[u8] = b"-- Dump completed";

/// How `mariadb-dump --databases` starts each database, followed by the
/// name in backquotes.
const CREATE_DATABASE: &[u8] = b"CREATE DATABASE /*!32312 IF NOT EXISTS*/ `";

/// The databases a backup can hold: accounts, then each game mode's characters.
const DATABASES: [&str; 3] = ["ragnarok", "ragnarok_prere", "ragnarok_re"];

/// The throwaway MariaDB a restore test loads a backup into: the stack's
/// image, no network, and no volume but the image's anonymous one, which
/// goes when the container is removed.
const TEST_CONTAINER: &str = "rag-wrapper-restore-test";

/// Runs SQL given as `$1` as root, in the test container or the stack's database.
const SQL: &str = r#"MYSQL_PWD="$MARIADB_ROOT_PASSWORD" exec mariadb --user=root --batch --skip-column-names -e "$1""#;

/// Checks every table of the databases given as arguments. `--silent`
/// leaves only the tables that aren't OK.
const CHECK: &str =
    r#"MYSQL_PWD="$MARIADB_ROOT_PASSWORD" exec mariadb-check --user=root --check --silent --databases "$@""#;

/// The restore tests' results, by backup file, in the backups folder.
/// A dotfile, so the folder shows only backups.
const TESTS: &str = ".restore-tests.json";

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
    /// The last restore test of this backup, if it had one.
    pub test: Option<RestoreTest>,
}

/// What a restore test found.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RestoreTest {
    /// Local time, "YYYY-MM-DD HH:MM".
    pub tested: String,
    /// Why the backup didn't load or check out; `None` when it did.
    pub problem: Option<String>,
    /// What the loaded backup holds, when it loaded.
    pub counts: Option<Counts>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Counts {
    /// Players' accounts, not the servers' own (`sex = 'S'`).
    pub accounts: u64,
    pub characters: u64,
    /// Rows of `inventory`.
    pub items: u64,
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
    let mut tests = read_tests(dir);
    let mut backups: Vec<Backup> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let file = e.file_name().into_string().ok()?;
            let (created, reason) = parse(&file)?;
            let size = e.metadata().ok().filter(|m| m.is_file())?.len();
            Some(Backup {
                test: tests.remove(&file),
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

/// Dumps the database into a new backup, starting MariaDB for it if the
/// server is stopped, then deletes the oldest beyond `KEEP`. Returns the
/// file name, or `None` when there is no database yet.
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

/// Reads a backup through, checking it is a whole dump. Returns the
/// databases it creates, which a backup from before each game mode had its
/// own characters database doesn't name.
fn check(path: &Path) -> Result<Vec<String>, String> {
    let gz = GzDecoder::new(BufReader::new(File::open(path).map_err(|e| e.to_string())?));
    let mut reader = BufReader::with_capacity(64 * 1024, gz);
    let mut databases = Vec::new();
    let mut tail = Tail::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = io::BufRead::read_until(&mut reader, b'\n', &mut line)
            .map_err(|e| format!("the file is damaged ({e})."))?;
        if n == 0 {
            break;
        }
        tail.push(&line);
        if let Some(name) = created_database(&line) {
            databases.push(name);
        }
    }
    if tail.complete() {
        Ok(databases)
    } else {
        Err("the file doesn't hold a whole database dump.".into())
    }
}

/// The database a dump line creates, if it's a `CREATE DATABASE` line.
fn created_database(line: &[u8]) -> Option<String> {
    let rest = line.strip_prefix(CREATE_DATABASE)?;
    let end = rest.iter().position(|&b| b == b'`')?;
    String::from_utf8(rest[..end].to_vec()).ok()
}

/// Replaces the database with a backup: checks the file, stops everything
/// that uses the database, backs up the current database, loads the dump
/// into emptied `ragnarok` databases, and starts the server again if it
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
    let mut cmd = ctx.compose();
    cmd.args(["exec", "-T", "db", "sh", "-c", LOAD]);
    feed(ctx, cmd, path)
}

/// Feeds a backup to `mariadb` started by `cmd`, into emptied databases.
fn feed(ctx: &Ctx, mut cmd: Command, path: &Path) -> Result<(), String> {
    ctx.check_cancelled()?;
    cmd.stdin(Stdio::piped());
    let last = Arc::new(Mutex::new(String::new()));
    let lines = ctx.clone();
    let seen = last.clone();
    let (mut child, done) = stack::spawn_lines(cmd, move |line| {
        seen.lock().unwrap().clone_from(&line);
        lines.line(line);
    })
    .map_err(|e| e.to_string())?;
    ctx.track(Some(child.id()));
    let mut stdin = child.stdin.take().unwrap();
    let fed = (|| -> io::Result<()> {
        // Tables and databases made after the backup would otherwise
        // survive the restore. A backup from before each game mode had its
        // own characters database names none, so it goes into `ragnarok`,
        // and the next start moves its characters out again.
        stdin.write_all(
            b"DROP DATABASE IF EXISTS ragnarok;\nDROP DATABASE IF EXISTS ragnarok_prere;\nDROP DATABASE IF EXISTS ragnarok_re;\n\
              CREATE DATABASE ragnarok;\nUSE ragnarok;\n",
        )?;
        io::copy(&mut GzDecoder::new(BufReader::new(File::open(path)?)), &mut stdin)?;
        Ok(())
    })();
    drop(stdin);
    let _ = done.join();
    let status = child.wait().map_err(|e| e.to_string());
    ctx.track(None);
    ctx.check_cancelled()?;
    if !status?.success() {
        let last = last.lock().unwrap();
        return Err(if last.is_empty() {
            "MariaDB stopped with an error.".into()
        } else {
            format!("MariaDB stopped with an error: {last}")
        });
    }
    fed.map_err(|e| e.to_string())
}

/// Loads a backup into a throwaway MariaDB container and checks it: every
/// database it names is there, every table checks OK, and it has accounts
/// and characters to count. The result is kept with the backup and shown
/// in its row. The live database is never touched; its counts are read for
/// comparison when it's running.
pub fn test_restore(ctx: &Ctx, file: &str) -> Result<(), String> {
    if parse(file).is_none() {
        return Err(format!("{file} isn't a backup."));
    }
    let path = ctx.backups.join(file);
    let chars = stack::chars_db(&ctx.dir);
    ctx.line(format!("Checking {file}…"));
    let result = match check(&path) {
        Ok(named) => {
            let image = db_image(ctx)?;
            remove_test_container();
            ctx.line(format!("Starting a throwaway database ({image}) to load it into…"));
            let started = start_test_container(ctx, &image).and_then(|()| wait_ready(ctx));
            let result = started.map(|()| {
                ctx.line(format!("Loading {file}…"));
                let mut cmd = host::command("docker");
                cmd.args(["exec", "-i", TEST_CONTAINER, "sh", "-c", LOAD]);
                feed(ctx, cmd, &path).and_then(|()| inspect(ctx, &named, chars))
            });
            remove_test_container();
            // Docker failing to run the test isn't the backup's fault, and
            // neither is a cancel: neither is recorded.
            match result? {
                Err(e) if ctx.check_cancelled().is_err() => return Err(e),
                r => r,
            }
        }
        Err(e) => Err(e),
    };

    let test = RestoreTest {
        tested: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        problem: result.as_ref().err().cloned(),
        counts: result.as_ref().ok().copied(),
    };
    if let Err(e) = save_test(&ctx.backups, file, test) {
        ctx.line(format!("Couldn't save the result: {e}"));
    }
    let counts = result.map_err(|e| format!("The restore test failed: {e}"))?;
    ctx.line(format!("{file} loads, and every table checks OK. It holds {}.", describe(&counts)));
    if db_running(&ctx.dir) {
        match live_counts(ctx, chars) {
            Ok(live) => ctx.line(format!("The live database holds {}.", describe(&live))),
            Err(e) => ctx.line(format!("Couldn't count the live database: {e}")),
        }
    }
    Ok(())
}

/// The stack's database image, so the test runs the MariaDB version the backup came from.
fn db_image(ctx: &Ctx) -> Result<String, String> {
    let mut config = ctx.compose();
    config.args(["config", "--format", "json"]);
    let config: serde_json::Value = serde_json::from_str(&stack::output(config)?).map_err(|e| e.to_string())?;
    config["services"]["db"]["image"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "The stack has no database image.".into())
}

fn start_test_container(ctx: &Ctx, image: &str) -> Result<(), String> {
    let mut run = host::command("docker");
    // A fixed password is fine for a database nothing can reach.
    run.args(["run", "--detach", "--quiet", "--name", TEST_CONTAINER, "--network", "none"])
        .args(["--env", "MARIADB_ROOT_PASSWORD=restore-test", image]);
    ctx.run(run).map_err(|e| format!("Couldn't start the throwaway database: {e}"))
}

/// Waits for the test container's MariaDB to take connections, the way
/// the stack's health check does. `--connect` needs TCP, which the
/// image's first-start set-up keeps off until it's done.
fn wait_ready(ctx: &Ctx) -> Result<(), String> {
    for _ in 0..180 {
        ctx.check_cancelled()?;
        let ready = host::command("docker")
            .args(["exec", TEST_CONTAINER, "healthcheck.sh", "--connect", "--innodb_initialized"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ready {
            return Ok(());
        }
        let mut running = host::command("docker");
        running.args(["inspect", "--format", "{{.State.Running}}", TEST_CONTAINER]);
        if stack::output(running).is_ok_and(|r| r.trim() == "false") {
            return Err("The throwaway database stopped while starting.".into());
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Err("The throwaway database didn't start in time.".into())
}

fn remove_test_container() {
    let _ = host::command("docker")
        .args(["rm", "--force", "--volumes", TEST_CONTAINER])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn in_test_container(script: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = host::command("docker");
    cmd.args(["exec", TEST_CONTAINER, "sh", "-c", script, "sh"]).args(args);
    stack::output(cmd)
}

/// Checks the loaded backup and counts what it holds.
fn inspect(ctx: &Ctx, named: &[String], chars: &str) -> Result<Counts, String> {
    ctx.line("Checking the loaded tables…");
    let present = in_test_container(SQL, &[&schemas_sql()])?;
    let present: Vec<&str> = present.lines().map(str::trim).collect();
    if let Some(missing) = named.iter().find(|db| !present.contains(&db.as_str())) {
        return Err(format!("the {missing} database didn't load."));
    }
    let report = in_test_container(CHECK, &present)
        .map_err(|e| format!("mariadb-check stopped with an error: {e}"))?;
    // The table's name, then what's wrong with it.
    let problem: Vec<&str> = report.split_whitespace().take(12).collect();
    if !problem.is_empty() {
        for line in report.lines() {
            ctx.line(line);
        }
        return Err(format!("a table doesn't check OK: {}", problem.join(" ")));
    }
    // The server's mode's characters, else the other mode's, else from
    // before each mode had its own database, when they were in `ragnarok`.
    let chars = std::iter::once(chars)
        .chain(DATABASES[1..].iter().copied())
        .find(|db| present.contains(db))
        .unwrap_or("ragnarok");
    parse_counts(&in_test_container(SQL, &[&counts_sql(chars)])?)
}

fn live_counts(ctx: &Ctx, chars: &str) -> Result<Counts, String> {
    let mut cmd = ctx.compose();
    cmd.args(["exec", "-T", "db", "sh", "-c", SQL, "sh", &counts_sql(chars)]);
    parse_counts(&stack::output(cmd)?)
}

fn schemas_sql() -> String {
    let names: Vec<String> = DATABASES.iter().map(|db| format!("'{db}'")).collect();
    format!(
        "SELECT schema_name FROM information_schema.schemata WHERE schema_name IN ({})",
        names.join(", ")
    )
}

/// `chars` is one of `DATABASES`.
fn counts_sql(chars: &str) -> String {
    format!(
        "SELECT (SELECT COUNT(*) FROM ragnarok.login WHERE sex <> 'S'), \
         (SELECT COUNT(*) FROM `{chars}`.`char`), (SELECT COUNT(*) FROM `{chars}`.inventory)"
    )
}

fn parse_counts(out: &str) -> Result<Counts, String> {
    let numbers: Vec<u64> = out.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    match numbers[..] {
        [accounts, characters, items] => Ok(Counts {
            accounts,
            characters,
            items,
        }),
        _ => Err(format!("couldn't count the accounts and characters ({}).", out.trim())),
    }
}

fn describe(c: &Counts) -> String {
    let n = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{}, {} and {}",
        n(c.accounts, "account", "accounts"),
        n(c.characters, "character", "characters"),
        n(c.items, "inventory item", "inventory items")
    )
}

fn read_tests(dir: &Path) -> BTreeMap<String, RestoreTest> {
    fs::read(dir.join(TESTS))
        .ok()
        .and_then(|t| serde_json::from_slice(&t).ok())
        .unwrap_or_default()
}

/// Keeps `test` as `file`'s result, dropping the results of backups that are gone.
fn save_test(dir: &Path, file: &str, test: RestoreTest) -> io::Result<()> {
    let mut tests = read_tests(dir);
    tests.insert(file.to_owned(), test);
    tests.retain(|f, _| dir.join(f).exists());
    let partial = dir.join(format!("{TESTS}.partial"));
    fs::write(&partial, serde_json::to_vec_pretty(&tests)?)?;
    fs::rename(partial, dir.join(TESTS))
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
    fn finds_the_databases_a_dump_creates() {
        let dir = std::env::temp_dir().join(format!("rag-wrapper-check-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.sql.gz");
        let write = |text: &[u8]| {
            let mut gz = GzEncoder::new(File::create(&path).unwrap(), Compression::default());
            gz.write_all(text).unwrap();
            gz.finish().unwrap();
        };
        write(
            b"CREATE DATABASE /*!32312 IF NOT EXISTS*/ `ragnarok` /*!40100 DEFAULT CHARACTER SET utf8mb4 */;\n\
              USE `ragnarok`;\nINSERT INTO login VALUES (1,'CREATE DATABASE /*!32312 IF NOT EXISTS*/ `x`');\n\
              CREATE DATABASE /*!32312 IF NOT EXISTS*/ `ragnarok_prere` /*!40100 DEFAULT CHARACTER SET utf8mb4 */;\n\
              -- Dump completed on 2026-10-15 12:07:58\n",
        );
        assert_eq!(check(&path), Ok(vec!["ragnarok".to_owned(), "ragnarok_prere".to_owned()]));
        write(b"USE `ragnarok`;\nINSERT INTO login VALUES (1);\n");
        assert!(check(&path).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_counts() {
        let counts = Counts {
            accounts: 12,
            characters: 1,
            items: 900,
        };
        assert_eq!(parse_counts("12\t1\t900\n"), Ok(counts));
        assert!(parse_counts("ERROR 1146 (42S02): Table 'ragnarok_re.char' doesn't exist").is_err());
        assert_eq!(describe(&counts), "12 accounts, 1 character and 900 inventory items");
    }

    #[test]
    fn keeps_test_results_of_backups_still_here() {
        let dir = std::env::temp_dir().join(format!("rag-wrapper-tests-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let kept = file_name("20261015-120000", Reason::Manual);
        let gone = file_name("20261014-120000", Reason::Daily);
        fs::write(dir.join(&kept), b"x").unwrap();
        fs::write(dir.join(&gone), b"x").unwrap();
        let test = |problem: Option<&str>| RestoreTest {
            tested: "2026-10-15 12:30".into(),
            problem: problem.map(str::to_owned),
            counts: None,
        };
        save_test(&dir, &gone, test(Some("the file is damaged."))).unwrap();
        fs::remove_file(dir.join(&gone)).unwrap();
        save_test(&dir, &kept, test(None)).unwrap();

        assert_eq!(read_tests(&dir).into_keys().collect::<Vec<_>>(), std::slice::from_ref(&kept));
        let listed = list(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].test, Some(test(None)));
        fs::remove_dir_all(&dir).unwrap();
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
