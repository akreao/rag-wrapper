//! rAthena's own custom NPCs, on the Mods screen: the scripts under
//! `npc/custom/` that rAthena ships switched off in
//! `npc/scripts_custom.conf`. The screen lists them from `custom-npcs.json`
//! with a description each, and writes the ones switched on to
//! `custom_npcs.txt` in the overrides folder, one script per line, which
//! `entrypoint.sh` loads on every start. Changes apply when the game
//! servers restart.
//!
//! A script that needs one game mode's maps or items is written as
//! `re: <path>` or `pre-re: <path>`, so it stays switched on but only loads
//! in that mode. Lines the screen doesn't know, such as scripts added by
//! hand, are kept.
//!
//! `.custom-npcs-applied` in the stack folder records the scripts the
//! server loaded at its last start from the app, so the screen can say
//! which changes wait for a restart.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// In the overrides folder, which the rAthena containers see as `conf/user/`.
pub const FILE: &str = "custom_npcs.txt";
/// In the stack folder.
const APPLIED: &str = ".custom-npcs-applied";

const HEADER: &str = "# rAthena's custom NPCs to load, one script per line. \"re: \" or \"pre-re: \"\n\
                      # before a script loads it only in that game mode. Rewritten by the desktop\n\
                      # app's Mods screen, which keeps any other lines.\n";

/// What `entrypoint.sh` prints on every start once it loads custom NPCs.
const LOADER_LINE: &str = "Custom NPCs on:";

/// The list the screen shows, compiled in. Each entry is one NPC or a set
/// of scripts that only work together.
const CATALOG: &str = include_str!("../custom-npcs.json");

/// Error and warning lines shown per NPC, newest last.
const MAX_ERRORS: usize = 20;

#[derive(Deserialize)]
struct Entry {
    group: String,
    name: String,
    description: String,
    files: Vec<String>,
    /// "re" or "pre-re" when it needs that game mode.
    mode: Option<String>,
    /// Something to know before switching it on.
    warning: Option<String>,
    /// Why it can't work on this server.
    unavailable: Option<String>,
}

fn catalog() -> Vec<Entry> {
    serde_json::from_str(CATALOG).expect("custom-npcs.json is checked by the tests")
}

#[derive(Serialize, Debug)]
pub struct Npc {
    /// Its first script, which names it to [`set_enabled`].
    pub id: String,
    pub group: String,
    pub name: String,
    pub description: String,
    pub files: Vec<String>,
    pub warning: Option<String>,
    /// Why it can't be switched on (or, when on, why it doesn't load).
    pub problem: Option<String>,
    pub enabled: bool,
    /// Switched on or off since the running server started.
    pub pending: bool,
    /// Errors and warnings about its scripts in the running map server's log.
    pub errors: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct Status {
    pub npcs: Vec<Npc>,
    /// Scripts in the file that aren't in the list, added by hand.
    pub by_hand: Vec<String>,
    /// The running game servers don't have the custom NPCs as they are now.
    pub restart_needed: bool,
    /// Custom NPCs are on, but the running image predates loading them.
    pub rebuild_needed: bool,
}

/// The file's lines without comments or blank lines, as `(mode, path)`.
fn lines(text: &str) -> Vec<(Option<&str>, &str)> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim())
        .filter(|l| !l.is_empty())
        .map(|l| match l.split_once(": ") {
            Some((mode, path)) => (Some(mode), path.trim()),
            None => (None, l),
        })
        .collect()
}

/// The scripts the file loads in this game mode, as `entrypoint.sh` reads it.
fn loaded(text: &str, renewal: bool) -> BTreeSet<String> {
    lines(text)
        .into_iter()
        .filter(|(mode, _)| match mode {
            None => true,
            Some("re") => renewal,
            Some("pre-re") => !renewal,
            Some(_) => false,
        })
        .map(|(_, path)| path.to_owned())
        .collect()
}

fn read(overrides: &Path) -> String {
    fs::read_to_string(overrides.join(FILE)).unwrap_or_default()
}

fn applied(dir: &Path) -> BTreeSet<String> {
    fs::read_to_string(dir.join(APPLIED))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Records the custom NPCs as loaded, after the game servers started.
pub fn record_applied(dir: &Path, overrides: &Path, renewal: bool) -> io::Result<()> {
    let text: String = loaded(&read(overrides), renewal).into_iter().map(|p| p + "\n").collect();
    fs::write(dir.join(APPLIED), text)
}

fn mode_name(mode: &str) -> &'static str {
    if mode == "re" { "Renewal" } else { "Pre-Renewal" }
}

/// Every NPC in the list and its state, in the list's order. `running`
/// says whether the map server is up and `log` is its log.
pub fn status(dir: &Path, overrides: &Path, renewal: bool, running: bool, log: &str) -> Status {
    let text = read(overrides);
    let on: BTreeSet<&str> = lines(&text).into_iter().map(|(_, path)| path).collect();
    let now = loaded(&text, renewal);
    let before = applied(dir);
    let problems = crate::mods::log_problems(log);
    let entries = catalog();
    let known: BTreeSet<&str> = entries.iter().flat_map(|e| e.files.iter().map(String::as_str)).collect();
    let npcs = entries
        .iter()
        .map(|e| {
            let enabled = e.files.iter().all(|f| on.contains(f.as_str()));
            let other_mode = e.mode.as_deref().filter(|m| (*m == "re") != renewal);
            let problem = match (&e.unavailable, other_mode) {
                (Some(why), _) => Some(why.clone()),
                (None, Some(mode)) if enabled => {
                    Some(format!("Only loads in {}, which needs its maps and quests.", mode_name(mode)))
                }
                (None, Some(mode)) => Some(format!("{} only: it needs that mode's maps and quests.", mode_name(mode))),
                (None, None) => None,
            };
            let mut errors: Vec<String> =
                problems.iter().filter(|p| e.files.iter().any(|f| mentions(p, f))).cloned().collect();
            if errors.len() > MAX_ERRORS {
                errors.drain(..errors.len() - MAX_ERRORS);
            }
            Npc {
                id: e.files[0].clone(),
                group: e.group.clone(),
                name: e.name.clone(),
                description: e.description.clone(),
                files: e.files.clone(),
                warning: e.warning.clone(),
                problem,
                enabled,
                pending: running && e.files.iter().any(|f| now.contains(f) != before.contains(f)),
                errors,
            }
        })
        .collect();
    Status {
        npcs,
        by_hand: on.iter().filter(|p| !known.contains(*p)).map(|p| (*p).to_owned()).collect(),
        restart_needed: running && now != before,
        rebuild_needed: running && !now.is_empty() && !log.lines().any(|l| l.starts_with(LOADER_LINE)),
    }
}

/// Whether `text` names the script at `path`, rather than a longer path
/// ending in it (such as a mod's copy).
fn mentions(text: &str, path: &str) -> bool {
    text.match_indices(path).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + path.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
            && !after.is_some_and(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
    })
}

/// Switches the NPC whose first script is `id` on or off.
pub fn set_enabled(overrides: &Path, id: &str, on: bool) -> Result<(), String> {
    let entries = catalog();
    let entry = entries.iter().find(|e| e.files[0] == id).ok_or(format!("{id} isn't one of rAthena's custom NPCs."))?;
    if on {
        if let Some(why) = &entry.unavailable {
            return Err(format!("{} can't be switched on: {why}", entry.name));
        }
    }
    let text = read(overrides);
    let header: BTreeSet<&str> = HEADER.lines().collect();
    let mut out = HEADER.to_owned();
    for line in text.lines() {
        let path = lines(line).first().map(|(_, path)| *path);
        if header.contains(line) || path.is_some_and(|p| entry.files.iter().any(|f| f == p)) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if on {
        for file in &entry.files {
            match &entry.mode {
                Some(mode) => out.push_str(&format!("{mode}: {file}\n")),
                None => out.push_str(&format!("{file}\n")),
            }
        }
    }
    fs::create_dir_all(overrides).map_err(|e| e.to_string())?;
    fs::write(overrides.join(FILE), out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("custom-npcs-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("overrides")).unwrap();
        dir
    }

    fn npc<'a>(s: &'a Status, id: &str) -> &'a Npc {
        s.npcs.iter().find(|n| n.id == id).unwrap()
    }

    const WARPER: &str = "npc/custom/warper.txt";
    const THQ: &str = "npc/custom/quests/thq/THQS_ChattingNPC.txt";
    const GMT: &str = "npc/custom/official/GeffenMagicTournament.txt";

    #[test]
    fn the_list_is_well_formed() {
        let entries = catalog();
        let mut seen = BTreeSet::new();
        for e in &entries {
            assert!(!e.name.is_empty() && !e.group.is_empty() && !e.description.is_empty(), "{}", e.name);
            assert!(!e.files.is_empty(), "{}", e.name);
            for f in &e.files {
                assert!(f.starts_with("npc/") && f.ends_with(".txt") && !f.contains(".."), "{f}");
                assert!(f.bytes().all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b)), "{f}");
                assert!(seen.insert(f.clone()), "{f} is listed twice");
            }
            assert!(e.mode.as_deref().is_none_or(|m| m == "re" || m == "pre-re"), "{}", e.name);
        }
    }

    #[test]
    fn switching_on_and_off_keeps_other_lines() {
        let dir = temp_dir("toggle");
        let overrides = dir.join("overrides");
        fs::write(overrides.join(FILE), "npc/custom/mine.txt # by hand\n").unwrap();

        set_enabled(&overrides, WARPER, true).unwrap();
        set_enabled(&overrides, THQ, true).unwrap();
        set_enabled(&overrides, GMT, true).unwrap();
        set_enabled(&overrides, WARPER, true).unwrap();
        let text = read(&overrides);
        assert!(text.starts_with(HEADER));
        assert_eq!(text.matches(WARPER).count(), 1);
        assert!(text.contains("npc/custom/mine.txt # by hand\n"));
        assert!(text.contains("npc/custom/quests/thq/THQS_TTShop.txt\n"));
        assert!(text.contains(&format!("\nre: {GMT}\n")));

        let s = status(&dir, &overrides, false, false, "");
        assert!(npc(&s, WARPER).enabled && npc(&s, THQ).enabled);
        assert_eq!(s.by_hand, ["npc/custom/mine.txt"]);
        // A Renewal-only NPC stays on in Pre-Renewal, but says it doesn't load.
        let gmt = npc(&s, GMT);
        assert!(gmt.enabled && gmt.problem.as_deref().unwrap().starts_with("Only loads in Renewal"));
        assert!(status(&dir, &overrides, true, false, "").npcs.iter().all(|n| n.id != GMT || n.problem.is_none()));

        set_enabled(&overrides, THQ, false).unwrap();
        set_enabled(&overrides, WARPER, false).unwrap();
        let text = read(&overrides);
        assert!(!text.contains("THQS_") && !text.contains(WARPER));
        assert_eq!(text.matches(HEADER).count(), 1);
        assert!(set_enabled(&overrides, "npc/custom/nope.txt", true).is_err());
        assert!(set_enabled(&overrides, "npc/custom/card_seller.txt", true).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn changes_wait_for_a_restart() {
        let dir = temp_dir("pending");
        let overrides = dir.join("overrides");
        set_enabled(&overrides, WARPER, true).unwrap();
        // Stopped: nothing is pending, it loads at the next start.
        assert!(!status(&dir, &overrides, false, false, "").restart_needed);
        let s = status(&dir, &overrides, false, true, "Custom NPCs on: \n");
        assert!(s.restart_needed && npc(&s, WARPER).pending && !s.rebuild_needed);

        record_applied(&dir, &overrides, false).unwrap();
        let s = status(&dir, &overrides, false, true, "Custom NPCs on: npc/custom/warper.txt\n");
        assert!(!s.restart_needed && !npc(&s, WARPER).pending);

        // A Renewal-only NPC switched on in Pre-Renewal changes nothing loaded.
        set_enabled(&overrides, GMT, true).unwrap();
        assert!(!status(&dir, &overrides, false, true, "").restart_needed);

        set_enabled(&overrides, WARPER, false).unwrap();
        let s = status(&dir, &overrides, false, true, "");
        assert!(s.restart_needed && npc(&s, WARPER).pending);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spots_an_image_without_custom_npcs() {
        let dir = temp_dir("rebuild");
        let overrides = dir.join("overrides");
        assert!(!status(&dir, &overrides, false, true, "Mods enabled: \n").rebuild_needed);
        set_enabled(&overrides, WARPER, true).unwrap();
        assert!(status(&dir, &overrides, false, true, "Mods enabled: \n").rebuild_needed);
        assert!(!status(&dir, &overrides, false, false, "").rebuild_needed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_errors_in_its_scripts() {
        let dir = temp_dir("errors");
        let overrides = dir.join("overrides");
        let log = "\
Custom NPCs on: npc/custom/etc/bank.txt
[Warning]: conf/user/custom_npcs.txt: npc/custom/warper.txt skipped: this server's rAthena has no such script
[Error]: npc_parse_script: Missing left curly in file 'npc/custom/etc/bank.txt', line '2'.
[Error]: script error on mods/a/npc/custom/etc/bank.txt line 3
[Warning]: Script command 'query_sql' returned failure.
[Debug]: Source (NPC): Banker is located in: npc/custom/etc/bank_kafra.txt
";
        let s = status(&dir, &overrides, false, true, log);
        assert_eq!(npc(&s, WARPER).errors.len(), 1);
        assert_eq!(
            npc(&s, "npc/custom/etc/bank.txt").errors,
            ["[Error]: npc_parse_script: Missing left curly in file 'npc/custom/etc/bank.txt', line '2'."]
        );
        let kafra = &npc(&s, "npc/custom/etc/bank_kafra.txt").errors;
        assert_eq!(kafra.len(), 1);
        assert!(kafra[0].starts_with("[Warning]: Script command 'query_sql' returned failure.\n[Debug]: Source"));
        fs::remove_dir_all(dir).unwrap();
    }
}
