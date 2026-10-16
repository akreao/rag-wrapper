//! Dashboard notices about the server's own accounts: passwords still on
//! `.env.example`'s public values, and an admin account `db-init` left as
//! it was. `db-init` logs both too, but nobody reads that log.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::envfile::{self, EnvFile};
use crate::stack;

/// The passwords `.env.example` comes with. `entrypoint.sh` warns about
/// the same values.
const PLACEHOLDERS: [(&str, &str); 3] = [
    ("DB_PASSWORD", "change-me"),
    ("DB_ROOT_PASSWORD", "change-me-too"),
    ("INTER_PASSWORD", "changeme123"),
];

/// Set to `yes` once the database passwords' warning was read; it can't be fixed from the app.
const DB_NOTED: &str = "PLACEHOLDER_DB_PASSWORDS_NOTED";

/// What the last start's `db-init` did instead of making the admin account
/// it was given, as `why:user`. Cleared when dismissed or on a new setup.
pub const ADMIN_NOTICE: &str = "ADMIN_NOTICE";

/// Which of the server's passwords are still the public example values.
#[derive(Serialize, Debug, PartialEq)]
pub struct Placeholders {
    /// INTER_PASSWORD, which the app can change safely.
    pub inter: bool,
    /// DB_PASSWORD or DB_ROOT_PASSWORD, until the warning is dismissed.
    pub database: bool,
}

pub fn placeholders(env: &EnvFile) -> Option<Placeholders> {
    let on = |key: &str| PLACEHOLDERS.iter().any(|(k, v)| *k == key && env.get(key).as_deref() == Some(*v));
    let inter = on("INTER_PASSWORD");
    let database = (on("DB_PASSWORD") || on("DB_ROOT_PASSWORD")) && env.get(DB_NOTED).as_deref() != Some("yes");
    (inter || database).then_some(Placeholders { inter, database })
}

/// The admin account `db-init` didn't make or change, and why: `password`
/// (it exists with another password), `player` (it exists outside the
/// admin group) or `inter` (it's INTER_USER's name).
#[derive(Serialize, Debug, PartialEq)]
pub struct AdminNotice {
    pub user: String,
    pub why: String,
}

pub fn admin_notice(env: &EnvFile) -> Option<AdminNotice> {
    let value = env.get(ADMIN_NOTICE)?;
    let (why, user) = value.split_once(':')?;
    Some(AdminNotice { user: user.to_owned(), why: why.to_owned() })
}

/// The admin notice from `db-init`'s log, as stored in [`ADMIN_NOTICE`].
/// A container that ran more than once keeps every run's output, so only
/// the last run counts: `init_db` starts each with one of two lines.
fn admin_outcome(log: &str) -> Option<String> {
    let last_run = ["Empty database: importing rAthena schema", "Schema already present: skipping import"]
        .iter()
        .filter_map(|marker| log.rfind(marker))
        .max()
        .map_or(log, |at| &log[at..]);
    last_run.lines().find_map(|line| {
        let (why, rest) = if let Some((_, rest)) = line.split_once("Admin account kept: ") {
            (if rest.contains(" as a player account ") { "player" } else { "password" }, rest)
        } else {
            ("inter", line.split_once("Admin account not created: ")?.1)
        };
        let user = rest.split_whitespace().next()?;
        Some(format!("{why}:{user}"))
    })
}

/// After a start whose `db-init` was handed an ADMIN_PASSWORD: notes in
/// `env` whether it kept an existing account rather than make one, from
/// its log in `dir`'s stack.
pub fn record_admin_outcome(dir: &std::path::Path, env: &mut EnvFile) {
    let mut logs = stack::compose(dir);
    logs.args(["logs", "--no-color", "--no-log-prefix", "db-init"]);
    // An image from before these lines logs neither, which reads as made.
    let outcome = stack::output(logs).ok().as_deref().and_then(admin_outcome);
    env.set(ADMIN_NOTICE, &outcome.unwrap_or_default());
}

#[tauri::command]
pub fn dismiss_admin_notice(app: State<crate::App>) -> Result<(), String> {
    app.edit_env(|env| {
        env.set(ADMIN_NOTICE, "");
        Ok(())
    })
}

#[tauri::command]
pub fn dismiss_db_placeholders(app: State<crate::App>) -> Result<(), String> {
    app.edit_env(|env| {
        env.set(DB_NOTED, "yes");
        Ok(())
    })
}

/// Gives INTER_PASSWORD a random value. `db-init` sets the account to it on
/// every start and the servers read it from their configs, so it only has
/// to reach all of them at once: a running server is started again in full
/// (`db-init` first), never just the game servers. Returns whether that
/// restart began.
#[tauri::command]
pub async fn change_inter_password(handle: AppHandle) -> Result<bool, String> {
    crate::off_main(handle, |handle, app| {
        if let Some(running) = app.task.lock().unwrap().as_ref() {
            return Err(format!("Wait for the current {} to finish.", running.kind));
        }
        let new = envfile::random_password(20).map_err(|e| e.to_string())?;
        let old = app.edit_env(|env| {
            let old = env.get("INTER_PASSWORD").unwrap_or_default();
            env.set("INTER_PASSWORD", &new);
            Ok(old)
        })?;
        let running = stack::status(&app.dir)
            .unwrap_or_default()
            .iter()
            .any(|s| stack::GAME_SERVERS.contains(&s.service.as_str()) && s.state == "running");
        if !running {
            return Ok(false);
        }
        if let Err(e) = crate::start_task(handle, app, "start".into(), None) {
            // Left changed, the next restart of only the game servers would
            // hand them a password the database doesn't have yet.
            let _ = app.edit_env(|env| {
                env.set("INTER_PASSWORD", &old);
                Ok(())
            });
            return Err(e);
        }
        Ok(true)
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_passwords_are_the_placeholders() {
        let example = EnvFile::parse(stack::ENV_EXAMPLE);
        for (key, value) in PLACEHOLDERS {
            assert_eq!(example.get(key).as_deref(), Some(value));
        }
        assert_eq!(placeholders(&example), Some(Placeholders { inter: true, database: true }));
    }

    #[test]
    fn placeholders_are_warned_about_until_changed_or_noted() {
        let mut env = EnvFile::parse("DB_PASSWORD=change-me\nDB_ROOT_PASSWORD=x\nINTER_PASSWORD=random\n");
        assert_eq!(placeholders(&env), Some(Placeholders { inter: false, database: true }));
        env.set(DB_NOTED, "yes");
        assert_eq!(placeholders(&env), None);

        env.set("INTER_PASSWORD", "changeme123");
        assert_eq!(placeholders(&env), Some(Placeholders { inter: true, database: false }));
        assert_eq!(placeholders(&EnvFile::parse("INTER_PASSWORD=abc\n")), None);
    }

    #[test]
    fn reads_what_db_init_did_with_the_admin_account() {
        let kept = "Schema already present: skipping import\nAdmin account kept: Steven already exists, so its password wasn't changed to ADMIN_PASSWORD. Change it on the portal's account page.\nDatabase ready\n";
        assert_eq!(admin_outcome(kept).as_deref(), Some("password:Steven"));

        let player = "Empty database: importing rAthena schema\nAdmin account kept: bob already exists as a player account (group 0), so it wasn't made an admin.\n";
        assert_eq!(admin_outcome(player).as_deref(), Some("player:bob"));

        let inter = "Schema already present: skipping import\nAdmin account not created: wrapper_inter is the name of the server-to-server login (INTER_USER). Pick another admin name.\n";
        assert_eq!(admin_outcome(inter).as_deref(), Some("inter:wrapper_inter"));

        // Only the last run of the container counts.
        let rerun = format!("{kept}Schema already present: skipping import\nCreated admin account Steven2\nDatabase ready\n");
        assert_eq!(admin_outcome(&rerun), None);
        assert_eq!(admin_outcome("Created admin account Steven\n"), None);

        let mut env = EnvFile::parse("");
        env.set(ADMIN_NOTICE, "password:Steven");
        assert_eq!(admin_notice(&env), Some(AdminNotice { user: "Steven".into(), why: "password".into() }));
        env.set(ADMIN_NOTICE, "");
        assert_eq!(admin_notice(&env), None);
    }
}
