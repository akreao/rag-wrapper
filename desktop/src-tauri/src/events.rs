//! The Events screen's EXP and drop events, kept in the characters
//! database's `wrapper_rate_events` table (made by db-init). The stack's
//! NPC script `docker/rathena/npc/rate_events.txt` reads it every 30
//! seconds, sets the rates with rAthena's `setbattleflag` (no restart) and
//! announces each event in game when it starts and ends.

use std::path::Path;

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};

use crate::{news, stack};

/// Runs `$2` in the characters database `$1` as the database's own
/// account, not root.
const SQL: &str = r#"MYSQL_PWD="$MARIADB_PASSWORD" exec mariadb --user="$MARIADB_USER" --batch --skip-column-names "$1" -e "$2""#;

/// Ended events stay on the screen this long, so the last one can be
/// checked or copied.
const KEEP_ENDED: i64 = 7 * 24 * 3600;

/// rAthena's limits: drop rates stop at 10000% (`item_rate_*` up to
/// 1000000 hundredths); the same cap keeps exp sensible.
const MAX_RATE: u32 = 10000;
const MIN_RATE: u32 = 10;

#[derive(Serialize, Debug, PartialEq)]
pub struct Event {
    pub id: u64,
    pub title: String,
    /// Unix seconds.
    pub starts_at: i64,
    pub ends_at: i64,
    /// Percent of the server's own rates.
    pub exp_rate: u32,
    pub drop_rate: u32,
    /// The map server has announced it.
    pub started: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NewEvent {
    pub title: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub exp_rate: u32,
    pub drop_rate: u32,
    /// Also add a post to the portal's News page.
    pub post_news: bool,
}

fn run(dir: &Path, sql: &str) -> Result<String, String> {
    let mut cmd = stack::compose(dir);
    cmd.args(["exec", "-T", "db", "sh", "-c", SQL, "sh", stack::chars_db(dir), sql]);
    stack::output(cmd).map_err(|e| {
        if e.contains("is not running") || e.contains("service \"db\" is not running") {
            "Start the server to see and plan events: they're kept in its database.".to_owned()
        } else if e.contains("wrapper_rate_events") && e.contains("doesn't exist") {
            "This server's build is older than the Events screen. Restart the server on the new build (or Rebuild) first.".to_owned()
        } else {
            e
        }
    })
}

/// Upcoming and running events, and those that ended in the last week,
/// soonest first.
pub fn list(dir: &Path) -> Result<Vec<Event>, String> {
    let text = run(
        dir,
        &format!(
            "SELECT id, title, starts_at, ends_at, exp_rate, drop_rate, started FROM wrapper_rate_events \
             WHERE ends_at > UNIX_TIMESTAMP() - {KEEP_ENDED} ORDER BY starts_at, id"
        ),
    )?;
    Ok(parse(&text))
}

fn parse(text: &str) -> Vec<Event> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let [id, title, starts_at, ends_at, exp_rate, drop_rate, started] = f[..] else {
                return None;
            };
            Some(Event {
                id: id.parse().ok()?,
                title: unescape(title),
                starts_at: starts_at.parse().ok()?,
                ends_at: ends_at.parse().ok()?,
                exp_rate: exp_rate.parse().ok()?,
                drop_rate: drop_rate.parse().ok()?,
                started: started == "1",
            })
        })
        .collect()
}

/// `mariadb --batch` escapes tabs, newlines and backslashes in values.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
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

/// A string as SQL, hex-encoded so no character in it can end the literal.
fn sql_string(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let hex: String = s.bytes().map(|b| format!("{b:02X}")).collect();
    format!("CONVERT(X'{hex}' USING utf8mb4)")
}

/// Checks a new event against the others and the clock (`now`, Unix
/// seconds).
fn check(event: &NewEvent, others: &[Event], now: i64) -> Result<(), String> {
    let title = event.title.trim();
    if title.is_empty() {
        return Err("Give the event a name, such as \"Double EXP weekend\". Players see it in the announcement.".into());
    }
    if title.chars().count() > 60 {
        return Err("Keep the name to 60 characters: it starts the in-game announcement.".into());
    }
    // The game client shows the announcement in its own code page, so
    // anything past plain ASCII comes out garbled.
    if !title.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return Err("Use plain letters, digits and punctuation in the name: the game client can't show accented letters or symbols in announcements.".into());
    }
    if event.ends_at <= event.starts_at {
        return Err("The event has to end after it starts.".into());
    }
    if event.ends_at <= now {
        return Err("That time has already passed.".into());
    }
    for rate in [event.exp_rate, event.drop_rate] {
        if !(MIN_RATE..=MAX_RATE).contains(&rate) {
            return Err(format!("Rates go from {MIN_RATE}% to {MAX_RATE}% of the server's own."));
        }
    }
    if event.exp_rate == 100 && event.drop_rate == 100 {
        return Err("Set EXP or drops to something other than 100%, or the event changes nothing.".into());
    }
    if let Some(other) = others.iter().find(|o| o.starts_at < event.ends_at && event.starts_at < o.ends_at) {
        return Err(format!(
            "It overlaps \"{}\" ({}). Only one event can run at a time, so change the times or delete that one first.",
            other.title,
            span(other.starts_at, other.ends_at)
        ));
    }
    Ok(())
}

/// A rate as players read it: 200 is "x2", 150 "x1.5".
fn times(rate: u32) -> String {
    let s = format!("{:.2}", rate as f64 / 100.0);
    format!("x{}", s.trim_end_matches('0').trim_end_matches('.'))
}

fn rates(exp: u32, drop: u32) -> String {
    match (exp, drop) {
        (e, d) if e == d => format!("EXP and drops {}", times(e)),
        (100, d) => format!("drops {}", times(d)),
        (e, 100) => format!("EXP {}", times(e)),
        (e, d) => format!("EXP {} and drops {}", times(e), times(d)),
    }
}

fn local(t: i64) -> DateTime<Local> {
    Local.timestamp_opt(t, 0).single().unwrap_or_else(Local::now)
}

fn when(t: i64) -> String {
    local(t).format("%a %-d %b %H:%M").to_string()
}

fn span(start: i64, end: i64) -> String {
    format!("{} to {}", when(start), when(end))
}

/// The in-game lines: the start one is also what players see when they
/// log in during the event.
fn texts(title: &str, event: &NewEvent) -> (String, String) {
    (
        format!("{title}: {} until {}.", rates(event.exp_rate, event.drop_rate), when(event.ends_at)),
        format!("{title} has ended. Rates are back to normal."),
    )
}

/// Adds an event, and a News post about it when asked. Returns the events
/// as [`list`] does.
pub fn add(dir: &Path, overrides: &Path, event: &NewEvent) -> Result<Vec<Event>, String> {
    let others = list(dir)?;
    check(event, &others, Local::now().timestamp())?;
    let title = event.title.trim();
    let (start_text, end_text) = texts(title, event);
    run(
        dir,
        &format!(
            "INSERT INTO wrapper_rate_events (title, starts_at, ends_at, exp_rate, drop_rate, start_text, end_text) \
             VALUES ({}, {}, {}, {}, {}, {}, {})",
            sql_string(title),
            event.starts_at,
            event.ends_at,
            event.exp_rate,
            event.drop_rate,
            sql_string(&start_text),
            sql_string(&end_text)
        ),
    )?;
    if event.post_news {
        post_news(overrides, title, event).map_err(|e| format!("The event is planned, but the News post wasn't saved: {e}"))?;
    }
    list(dir)
}

fn post_news(overrides: &Path, title: &str, event: &NewEvent) -> Result<(), String> {
    let mut current = news::load(overrides);
    let today = Local::now().format("%-d %b %Y");
    let mut chars = rates(event.exp_rate, event.drop_rate).chars().collect::<Vec<_>>();
    chars[0] = chars[0].to_ascii_uppercase();
    let body = format!(
        "*{today}*\n\n{} from {} until {}.",
        chars.into_iter().collect::<String>(),
        when(event.starts_at),
        when(event.ends_at)
    );
    current.posts.insert(0, news::Post { title: title.to_owned(), body });
    news::save(overrides, &current).map(|_| ())
}

/// Deletes an event. One that's on ends at the map server's next check,
/// with its end announcement.
pub fn delete(dir: &Path, id: u64) -> Result<Vec<Event>, String> {
    run(dir, &format!("DELETE FROM wrapper_rate_events WHERE id = {id}"))?;
    list(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(starts_at: i64, ends_at: i64, exp_rate: u32, drop_rate: u32) -> NewEvent {
        NewEvent { title: "Double EXP".into(), starts_at, ends_at, exp_rate, drop_rate, post_news: false }
    }

    fn event(id: u64, starts_at: i64, ends_at: i64) -> Event {
        Event { id, title: "Other".into(), starts_at, ends_at, exp_rate: 200, drop_rate: 100, started: false }
    }

    #[test]
    fn parses_batch_output() {
        let text = "1\tDouble\\tEXP\t100\t200\t200\t150\t1\n2\tx\t1\nbad\n";
        assert_eq!(
            parse(text),
            vec![Event { id: 1, title: "Double\tEXP".into(), starts_at: 100, ends_at: 200, exp_rate: 200, drop_rate: 150, started: true }]
        );
    }

    #[test]
    fn strings_are_hex_in_sql() {
        assert_eq!(sql_string("a'"), "CONVERT(X'6127' USING utf8mb4)");
        assert_eq!(sql_string(""), "''");
    }

    #[test]
    fn rates_read_as_multipliers() {
        assert_eq!(rates(200, 200), "EXP and drops x2");
        assert_eq!(rates(150, 100), "EXP x1.5");
        assert_eq!(rates(100, 125), "drops x1.25");
        assert_eq!(rates(300, 50), "EXP x3 and drops x0.5");
    }

    #[test]
    fn checks_new_events() {
        let now = 1_000;
        assert!(check(&new(900, 2_000, 200, 100), &[], now).is_ok());
        assert!(check(&new(2_000, 2_000, 200, 100), &[], now).unwrap_err().contains("end after"));
        assert!(check(&new(100, 900, 200, 100), &[], now).unwrap_err().contains("passed"));
        assert!(check(&new(1_100, 2_000, 100, 100), &[], now).unwrap_err().contains("changes nothing"));
        assert!(check(&new(1_100, 2_000, 5, 100), &[], now).unwrap_err().contains("Rates go"));
        assert!(check(&new(1_100, 2_000, 200, 100_000), &[], now).unwrap_err().contains("Rates go"));
        let mut accented = new(1_100, 2_000, 200, 100);
        accented.title = "Double EXP \u{e9}t\u{e9}".into();
        assert!(check(&accented, &[], now).unwrap_err().contains("plain letters"));
        let mut blank = new(1_100, 2_000, 200, 100);
        blank.title = "  ".into();
        assert!(check(&blank, &[], now).is_err());
    }

    #[test]
    fn events_cannot_overlap() {
        let others = [event(1, 2_000, 3_000)];
        assert!(check(&new(1_100, 2_500, 200, 100), &others, 1_000).unwrap_err().contains("overlaps \"Other\""));
        assert!(check(&new(2_500, 4_000, 200, 100), &others, 1_000).is_err());
        // Back to back is fine.
        assert!(check(&new(1_100, 2_000, 200, 100), &others, 1_000).is_ok());
        assert!(check(&new(3_000, 4_000, 200, 100), &others, 1_000).is_ok());
    }

    #[test]
    fn in_game_lines() {
        let (start, end) = texts("Weekend", &new(0, 1_000_000, 200, 150));
        assert!(start.starts_with("Weekend: EXP x2 and drops x1.5 until "), "{start}");
        assert!(start.is_ascii());
        assert_eq!(end, "Weekend has ended. Rates are back to normal.");
    }

    #[test]
    fn news_post_goes_on_top() {
        let dir = std::env::temp_dir().join(format!("rag-wrapper-events-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("portal")).unwrap();
        std::fs::write(dir.join(news::FILE), "## Old\n\nolder post\n").unwrap();
        post_news(&dir, "Weekend", &new(0, 3_600, 100, 200)).unwrap();
        let saved = news::load(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(saved.posts.len(), 2);
        assert_eq!(saved.posts[0].title, "Weekend");
        assert!(saved.posts[0].body.contains("Drops x2 from "), "{}", saved.posts[0].body);
        assert_eq!(saved.posts[1].title, "Old");
    }
}
