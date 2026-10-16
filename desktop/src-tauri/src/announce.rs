//! Messages to everyone in game: the warning before a restart or stop,
//! and messages the host types on the Dashboard. Each is a row in the
//! characters database's `wrapper_announcements`, which the image's NPC
//! script (`docker/rathena/announcements.txt`) shows within a couple of
//! seconds and marks shown. A map server that never marks it runs an
//! image from before the script.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use tauri::Emitter;

use crate::stack;
use crate::task::Ctx;

/// The longest message: rAthena's broadcasts and the client's chat window
/// take more, but a longer line wraps badly.
pub const MAX_LEN: usize = 200;

/// How long players are warned before a restart or stop.
const WARNING: u64 = 5 * 60;

/// When the second warning goes out.
const LAST_WARNING: u64 = 60;

/// The script looks every 2 seconds; this leaves room for a busy map server.
const PICKED_UP: Duration = Duration::from_secs(10);

/// Runs SQL in the characters database (`$1`) with the database's own
/// account, as the live view does. Prints the result without headers.
const SQL: &str = r#"MYSQL_PWD="$MARIADB_PASSWORD" exec mariadb --user="$MARIADB_USER" --batch --skip-column-names "$1" -e "$2""#;

/// What players are told is about to happen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Going {
    Restart,
    Stop,
}

impl Going {
    fn noun(self) -> &'static str {
        match self {
            Going::Restart => "restarting",
            Going::Stop => "shutting down",
        }
    }

    fn cancelled(self) -> &'static str {
        match self {
            Going::Restart => "The server restart is cancelled.",
            Going::Stop => "The server shutdown is cancelled.",
        }
    }
}

/// The warning players see with `secs` to go.
fn warning(going: Going, secs: u64) -> String {
    let when = match secs {
        0 => "now".to_owned(),
        60 => "in 1 minute".to_owned(),
        s if s % 60 == 0 => format!("in {} minutes", s / 60),
        s => format!("in {s} seconds"),
    };
    format!("Server {} {when}.", going.noun())
}

/// A message as it can go out: trimmed, not empty, at most [`MAX_LEN`]
/// characters, and plain ASCII, since the game client shows other
/// characters in its own code page, garbled.
pub fn check(message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Type a message first.".into());
    }
    if let Some(c) = message.chars().find(|c| !(' '..='~').contains(c)) {
        return Err(format!(
            "The game client can't show \"{c}\". Use plain letters, digits and punctuation."
        ));
    }
    if message.len() > MAX_LEN {
        return Err(format!("Keep it to {MAX_LEN} characters; this one has {}.", message.len()));
    }
    Ok(message.to_owned())
}

/// The message as an SQL hex literal, so nothing in it needs quoting.
fn hex_literal(message: &str) -> String {
    let hex: String = message.bytes().map(|b| format!("{b:02X}")).collect();
    format!("X'{hex}'")
}

fn sql(dir: &Path, query: &str) -> Result<String, String> {
    let mut cmd = stack::compose(dir);
    cmd.args(["exec", "-T", "db", "sh", "-c", SQL, "sh", stack::chars_db(dir), query]);
    stack::output(cmd)
}

/// Whether the map server is up to show anything.
pub fn map_running(dir: &Path) -> bool {
    stack::status(dir)
        .unwrap_or_default()
        .iter()
        .any(|s| s.service == "map" && s.state == "running")
}

/// How many characters are online, or `None` when that can't be read.
fn online(dir: &Path) -> Option<u64> {
    sql(dir, "SELECT COUNT(*) FROM `char` WHERE online = 1").ok()?.trim().parse().ok()
}

/// Adds a message for the map server to show and returns its row.
fn queue(dir: &Path, message: &str) -> Result<u64, String> {
    let message = check(message)?;
    let out = sql(
        dir,
        &format!("INSERT INTO wrapper_announcements (message) VALUES ({}); SELECT LAST_INSERT_ID();", hex_literal(&message)),
    )?;
    out.trim()
        .parse()
        .map_err(|_| format!("The database didn't take the message: {}", out.trim()))
}

/// Waits up to `limit` for the map server to show row `id`.
fn wait_shown(dir: &Path, id: u64, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        let shown = sql(dir, &format!("SELECT shown_at IS NOT NULL AND skipped = 0 FROM wrapper_announcements WHERE id = {id}"));
        if shown.is_ok_and(|s| s.trim() == "1") {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(500));
    }
}

/// What to say when the map server never picked a message up.
const NOT_SHOWN: &str = "The map server didn't show it: the running server was built before \
    in-game messages. Once a Rebuild or Restart on the new build has run, they work.";

/// Sends a message of the host's own to everyone in game and waits until
/// it's shown.
pub fn send(dir: &Path, message: &str) -> Result<(), String> {
    let message = check(message)?;
    if !map_running(dir) {
        return Err("The map server isn't running, so nobody would see it.".into());
    }
    let id = queue(dir, &message)?;
    if wait_shown(dir, id, PICKED_UP) {
        Ok(())
    } else {
        Err(NOT_SHOWN.into())
    }
}

/// Tells players the server goes down now, without waiting, as the window
/// closes. Gives the map server a few seconds to show it.
pub fn now(dir: &Path, going: Going) {
    if !map_running(dir) || online(dir).unwrap_or(0) == 0 {
        return;
    }
    if let Ok(id) = queue(dir, &warning(going, 0)) {
        wait_shown(dir, id, Duration::from_secs(4));
    }
}

/// The step before a restart or stop: warns everyone in game
/// [`WARNING`] seconds ahead and again at [`LAST_WARNING`], then says it's
/// happening now. Skipped when the map server is down or nobody's online,
/// and cut short when everyone logs off or the host skips the wait
/// (`warning` events carry the seconds left to the window). A cancel tells
/// players it's off. Only a cancel fails it: a warning that can't go out
/// never holds up the restart.
pub fn warn(ctx: &Ctx, going: Going) -> Result<(), String> {
    let dir = &ctx.dir;
    if !map_running(dir) {
        return Ok(());
    }
    let players = match online(dir) {
        Some(0) => {
            ctx.line("Nobody's online, so there's no warning to wait for.");
            return Ok(());
        }
        Some(n) => n,
        None => {
            ctx.line("Couldn't tell who's online, so players aren't warned.");
            return Ok(());
        }
    };
    let first = match queue(dir, &warning(going, WARNING)) {
        Ok(id) => id,
        Err(e) => {
            ctx.line(format!("Couldn't warn players: {e}"));
            return Ok(());
        }
    };
    if !wait_shown(dir, first, PICKED_UP) {
        ctx.line(format!("Couldn't warn players. {NOT_SHOWN}"));
        return Ok(());
    }
    let who = if players == 1 { "the player online".to_owned() } else { format!("the {players} players online") };
    ctx.line(format!(
        "Told {who}: \"{}\" Skip the wait to go ahead now, or Cancel to stay up.",
        warning(going, WARNING)
    ));

    let start = Instant::now();
    let mut last_check = Instant::now();
    let mut warned_last = false;
    let result = loop {
        let left = WARNING.saturating_sub(start.elapsed().as_secs());
        let _ = ctx.handle.emit("warning", Some(left));
        if let Err(e) = ctx.check_cancelled() {
            if let Ok(id) = queue(dir, going.cancelled()) {
                wait_shown(dir, id, Duration::from_secs(4));
            }
            ctx.line("Players were told it's cancelled.");
            break Err(e);
        }
        if left == 0 {
            break Ok(());
        }
        if ctx.skip_requested() {
            ctx.line("Skipped the rest of the wait.");
            break Ok(());
        }
        if left <= LAST_WARNING && !warned_last {
            warned_last = true;
            let _ = queue(dir, &warning(going, LAST_WARNING));
        }
        if last_check.elapsed() >= Duration::from_secs(15) {
            last_check = Instant::now();
            if online(dir) == Some(0) {
                ctx.line("Everyone has logged off, so there's no more wait.");
                break Ok(());
            }
        }
        thread::sleep(Duration::from_millis(250));
    };
    let _ = ctx.handle.emit("warning", None::<u64>);
    result?;
    if let Ok(id) = queue(dir, &warning(going, 0)) {
        // Shown before the map server goes, so players see why they're dropped.
        wait_shown(dir, id, Duration::from_secs(4));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_say_how_long_is_left() {
        assert_eq!(warning(Going::Restart, 300), "Server restarting in 5 minutes.");
        assert_eq!(warning(Going::Restart, 60), "Server restarting in 1 minute.");
        assert_eq!(warning(Going::Stop, 0), "Server shutting down now.");
        assert_eq!(warning(Going::Stop, 30), "Server shutting down in 30 seconds.");
    }

    #[test]
    fn messages_are_trimmed_plain_and_short() {
        assert_eq!(check("  Back in 10!  ").unwrap(), "Back in 10!");
        assert!(check("   ").is_err());
        assert!(check("Café open").unwrap_err().contains("é"));
        assert!(check("two\nlines").is_err());
        assert!(check(&"a".repeat(MAX_LEN)).is_ok());
        assert!(check(&"a".repeat(MAX_LEN + 1)).is_err());
    }

    #[test]
    fn messages_go_into_sql_as_hex() {
        assert_eq!(hex_literal("It's \\ \"ok\""), "X'49742773205C20226F6B22'");
        assert_eq!(hex_literal("Hi"), "X'4869'");
    }

    /// Sends a message through a running stack in `STACK_DIR`:
    /// `STACK_DIR=... cargo test real_stack -- --ignored`.
    #[test]
    #[ignore]
    fn real_stack() {
        let dir = std::path::PathBuf::from(std::env::var("STACK_DIR").unwrap());
        assert!(map_running(&dir));
        assert_eq!(online(&dir), Some(0));
        send(&dir, "It's a test: \\ \"quoted\" `ticks` $HOME").unwrap();
        assert!(send(&dir, "").is_err());
    }
}
