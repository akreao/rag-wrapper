//! The Settings screen's game settings, kept as rAthena override files in
//! the stack's `overrides/` folder. Compose mounts that folder into each
//! rAthena container as `conf/user/`, and `entrypoint.sh` imports whatever
//! it finds there after the stock configs. The app only edits the keys it
//! shows, so lines added to the same files by hand are kept.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// What the Settings screen shows. Rates are multipliers (1 = official),
/// the death penalty is the percentage of a level's exp lost. Exp rates
/// are in the battle config editor (`battleconf.rs`) with the rest of
/// `conf/battle/exp.conf`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GameSettings {
    pub drop_common: f64,
    pub drop_equip: f64,
    pub drop_card: f64,
    pub drop_mvp: f64,
    pub death_penalty: f64,
    pub server_name: String,
    pub motd: String,
    pub player_commands: bool,
    /// `groups.yml` was written by hand, so the app leaves it alone.
    #[serde(default)]
    pub player_commands_locked: bool,
}

const BATTLE: &str = "battle_conf.txt";
const CHAR: &str = "char_conf.txt";
const MAP: &str = "map_conf.txt";
const MOTD: &str = "motd.txt";
const GROUPS: &str = "groups.yml";

/// Each rate and the battle config keys it sets, all in rAthena's units
/// where 100 is 1x. Drop rates cover normal, boss and MVP monsters alike.
const RATES: &[(&str, &[&str])] = &[
    (
        "drop_common",
        &[
            "item_rate_common", "item_rate_common_boss", "item_rate_common_mvp",
            "item_rate_heal", "item_rate_heal_boss", "item_rate_heal_mvp",
            "item_rate_use", "item_rate_use_boss", "item_rate_use_mvp",
        ],
    ),
    ("drop_equip", &["item_rate_equip", "item_rate_equip_boss", "item_rate_equip_mvp"]),
    ("drop_card", &["item_rate_card", "item_rate_card_boss", "item_rate_card_mvp"]),
    ("drop_mvp", &["item_rate_mvp"]),
    // Each 100 is 1% of the level's exp, so the same scale works.
    ("death_penalty", &["death_penalty_base", "death_penalty_job"]),
];

const DEFAULT_SERVER_NAME: &str = "rAthena";
const MAX_RATE: f64 = 1000.0;
const MAX_MOTD_LINES: usize = 20;
const MAX_MOTD_LINE: usize = 200;

/// Marks `groups.yml` as the app's, so it can rewrite or delete it.
const GROUPS_MARKER: &str = "# Written by rag-wrapper's Settings screen.";

/// The atcommands rAthena's stock "Super Player" group (1) adds on top of
/// "Player" (0). Turning player commands on gives them to group 0, which
/// every account starts in.
const PLAYER_COMMANDS: &[&str] = &[
    "commands", "charcommands", "help", "rates", "uptime", "showdelay", "exp", "mobinfo",
    "iteminfo", "whodrops", "servertime", "jailtime", "hominfo", "homstats", "showexp",
    "showzeny", "whereis", "refresh", "noask", "noks", "autoloot", "alootid",
    "autoloottype", "autotrade", "request", "go", "breakguild", "channel", "langtype",
];

impl GameSettings {
    fn rate(&self, name: &str) -> f64 {
        match name {
            "drop_common" => self.drop_common,
            "drop_equip" => self.drop_equip,
            "drop_card" => self.drop_card,
            "drop_mvp" => self.drop_mvp,
            "death_penalty" => self.death_penalty,
            _ => unreachable!("unknown rate {name}"),
        }
    }

    fn rate_mut(&mut self, name: &str) -> &mut f64 {
        match name {
            "drop_common" => &mut self.drop_common,
            "drop_equip" => &mut self.drop_equip,
            "drop_card" => &mut self.drop_card,
            "drop_mvp" => &mut self.drop_mvp,
            "death_penalty" => &mut self.death_penalty,
            _ => unreachable!("unknown rate {name}"),
        }
    }
}

pub fn load(dir: &Path) -> GameSettings {
    let battle = ConfFile::load(&dir.join(BATTLE));
    let char = ConfFile::load(&dir.join(CHAR));
    let groups = fs::read_to_string(dir.join(GROUPS)).ok();

    let mut s = GameSettings {
        drop_common: 1.0,
        drop_equip: 1.0,
        drop_card: 1.0,
        drop_mvp: 1.0,
        death_penalty: 1.0,
        server_name: char.get("server_name").unwrap_or_else(|| DEFAULT_SERVER_NAME.into()),
        motd: fs::read_to_string(dir.join(MOTD)).unwrap_or_default().trim_end().to_owned(),
        player_commands: groups.is_some(),
        player_commands_locked: groups.as_deref().is_some_and(|g| !g.starts_with(GROUPS_MARKER)),
    };
    for (name, keys) in RATES {
        if let Some(v) = battle.get(keys[0]).and_then(|v| v.parse::<f64>().ok()) {
            *s.rate_mut(name) = v / 100.0;
        }
    }
    s
}

/// Writes the override files. Returns whether anything changed, in which
/// case the game servers need a restart to pick it up.
pub fn save(dir: &Path, new: &GameSettings) -> Result<bool, String> {
    validate(new)?;
    let old = load(dir);
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    let mut battle = ConfFile::load(&dir.join(BATTLE));
    for (name, keys) in RATES {
        // Only a rate changed here is written, so a value set for one of
        // its keys alone (in the battle config editor or by hand) stays.
        if new.rate(name) == old.rate(name) {
            continue;
        }
        // Official rates are left out, so the stock config applies.
        let value = to_units(new.rate(name));
        let value = (value != 100).then(|| value.to_string());
        for key in *keys {
            battle.set(key, value.as_deref());
        }
    }
    battle.save().map_err(|e| e.to_string())?;

    let mut char = ConfFile::load(&dir.join(CHAR));
    let name = new.server_name.trim();
    char.set("server_name", (name != DEFAULT_SERVER_NAME).then_some(name));
    char.save().map_err(|e| e.to_string())?;

    let motd = new.motd.trim_end();
    let mut map = ConfFile::load(&dir.join(MAP));
    if motd.is_empty() {
        map.set("motd_txt", None);
        remove(&dir.join(MOTD))?;
    } else {
        map.set("motd_txt", Some("conf/user/motd.txt"));
        fs::write(dir.join(MOTD), format!("{motd}\n")).map_err(|e| e.to_string())?;
    }
    map.save().map_err(|e| e.to_string())?;

    if !old.player_commands_locked && new.player_commands != old.player_commands {
        if new.player_commands {
            fs::write(dir.join(GROUPS), groups_yml()).map_err(|e| e.to_string())?;
        } else {
            remove(&dir.join(GROUPS))?;
        }
    }

    let mut saved = load(dir);
    saved.player_commands_locked = old.player_commands_locked;
    Ok(saved != old)
}

/// A multiplier in rAthena's percent units, where 100 is 1x.
fn to_units(x: f64) -> i64 {
    (x * 100.0).round() as i64
}

fn validate(s: &GameSettings) -> Result<(), String> {
    for (name, label) in [
        ("drop_common", "Common item drops"),
        ("drop_equip", "Equipment drops"),
        ("drop_card", "Card drops"),
        ("drop_mvp", "MVP rewards"),
    ] {
        let x = s.rate(name);
        if !x.is_finite() || !(0.01..=MAX_RATE).contains(&x) {
            return Err(format!("{label} must be a rate from 0.01 to {MAX_RATE}."));
        }
    }
    if !s.death_penalty.is_finite() || !(0.0..=100.0).contains(&s.death_penalty) {
        return Err("Exp lost on death must be a percentage from 0 to 100.".into());
    }

    // rAthena keeps 19 characters, and the client stores guild emblems in
    // a folder named after the server, so Windows filename rules apply.
    let name = s.server_name.trim();
    if name.is_empty() || name.len() > 19 {
        return Err("The server name must be 1 to 19 characters long.".into());
    }
    if !name.chars().all(|c| c.is_ascii_graphic() && !r#"\/:*?"<>|"#.contains(c)) {
        return Err(r#"The server name can't use spaces, accents or any of \ / : * ? " < > |"#.into());
    }

    let lines: Vec<&str> = s.motd.trim_end().lines().collect();
    if lines.len() > MAX_MOTD_LINES {
        return Err(format!("The message of the day can be at most {MAX_MOTD_LINES} lines."));
    }
    for line in lines {
        if line.len() > MAX_MOTD_LINE {
            return Err(format!("Each line of the message of the day can be at most {MAX_MOTD_LINE} characters."));
        }
        // The game client shows plain ASCII reliably; other text depends
        // on the client's language.
        if !line.chars().all(|c| c == ' ' || c.is_ascii_graphic()) {
            return Err("The message of the day can only use plain letters, digits and punctuation.".into());
        }
        // rAthena warns that this sequence crashes newer clients.
        if line.contains(" :") {
            return Err("The message of the day can't contain a space followed by a colon (\" :\"); it crashes newer clients.".into());
        }
    }
    Ok(())
}

fn groups_yml() -> String {
    let mut text = format!(
        "{GROUPS_MARKER}\n\
         # It is rewritten or deleted when \"Player commands\" changes there.\n\
         # To manage groups by hand instead, delete this comment block.\n\
         Header:\n  Type: PLAYER_GROUP_DB\n  Version: 1\n\nBody:\n  - Id: 0\n    Commands:\n"
    );
    for c in PLAYER_COMMANDS {
        text.push_str(&format!("      {c}: true\n"));
    }
    text
}

fn remove(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// An rAthena `key: value` config file, edited line by line so comments
/// and keys the app doesn't manage stay as they are. A missing file reads
/// as empty, and a file left with nothing in it is deleted.
pub(crate) struct ConfFile {
    path: std::path::PathBuf,
    lines: Vec<String>,
}

impl ConfFile {
    pub(crate) fn load(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        Self {
            path: path.to_owned(),
            lines: text.lines().map(str::to_owned).collect(),
        }
    }

    fn key_of(line: &str) -> Option<&str> {
        let line = line.trim_start();
        if line.starts_with("//") {
            return None;
        }
        line.split_once(':').map(|(k, _)| k.trim())
    }

    /// rAthena reads every line, so the last one with the key wins.
    pub(crate) fn get(&self, key: &str) -> Option<String> {
        self.lines.iter().rev().find_map(|line| {
            (Self::key_of(line)? == key).then(|| line.split_once(':').unwrap().1.trim().to_owned())
        })
    }

    /// Sets the key, or removes every line for it when `value` is `None`.
    pub(crate) fn set(&mut self, key: &str, value: Option<&str>) {
        let mut placed = false;
        self.lines.retain_mut(|line| {
            if Self::key_of(line) != Some(key) {
                return true;
            }
            match value {
                Some(v) if !placed => {
                    *line = format!("{key}: {v}");
                    placed = true;
                    true
                }
                _ => false,
            }
        });
        if let (Some(v), false) = (value, placed) {
            self.lines.push(format!("{key}: {v}"));
        }
    }

    pub(crate) fn save(&self) -> std::io::Result<()> {
        if self.lines.iter().all(|l| l.trim().is_empty()) {
            return match fs::remove_file(&self.path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        let mut text = self.lines.join("\n");
        text.push('\n');
        fs::write(&self.path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("settings-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_write_nothing() {
        let dir = temp_dir("defaults");
        let s = load(&dir);
        assert_eq!(s.drop_equip, 1.0);
        assert_eq!(s.server_name, "rAthena");
        assert!(!s.player_commands);
        assert!(!save(&dir, &s).unwrap());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn round_trip_keeps_hand_edits() {
        let dir = temp_dir("roundtrip");
        fs::write(dir.join(BATTLE), "// mine\nitem_rate_equip: 300\nmulti_level_up: yes\n").unwrap();

        let mut s = load(&dir);
        assert_eq!(s.drop_equip, 3.0);
        s.drop_equip = 5.0;
        s.drop_card = 2.5;
        s.death_penalty = 0.0;
        s.server_name = "MyServer".into();
        s.motd = "Welcome!\n\nHave fun".into();
        s.player_commands = true;
        assert!(save(&dir, &s).unwrap());

        let battle = fs::read_to_string(dir.join(BATTLE)).unwrap();
        assert!(battle.starts_with("// mine\nitem_rate_equip: 500\nmulti_level_up: yes\n"));
        assert!(battle.contains("item_rate_card_boss: 250\n"));
        assert!(battle.contains("death_penalty_job: 0\n"));
        assert!(!battle.contains("item_rate_mvp"));
        assert_eq!(fs::read_to_string(dir.join(CHAR)).unwrap(), "server_name: MyServer\n");
        assert_eq!(fs::read_to_string(dir.join(MAP)).unwrap(), "motd_txt: conf/user/motd.txt\n");
        assert!(fs::read_to_string(dir.join(GROUPS)).unwrap().contains("      autoloot: true\n"));

        let mut again = load(&dir);
        assert_eq!(again, s);
        assert!(!save(&dir, &again).unwrap());

        // Back to official: the app's lines go, the hand-written ones stay.
        again.drop_equip = 1.0;
        again.drop_card = 1.0;
        again.death_penalty = 1.0;
        again.server_name = "rAthena".into();
        again.motd = String::new();
        again.player_commands = false;
        assert!(save(&dir, &again).unwrap());
        assert_eq!(fs::read_to_string(dir.join(BATTLE)).unwrap(), "// mine\nmulti_level_up: yes\n");
        for f in [CHAR, MAP, MOTD, GROUPS] {
            assert!(!dir.join(f).exists(), "{f} should be gone");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unchanged_rates_leave_their_keys_alone() {
        let dir = temp_dir("keys");
        let battle = "base_exp_rate: 300\nitem_rate_card: 200\nitem_rate_card_boss: 50\n";
        fs::write(dir.join(BATTLE), battle).unwrap();
        let mut s = load(&dir);
        s.motd = "Hi".into();
        assert!(save(&dir, &s).unwrap());
        assert_eq!(fs::read_to_string(dir.join(BATTLE)).unwrap(), battle);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hand_written_groups_are_left_alone() {
        let dir = temp_dir("groups");
        fs::write(dir.join(GROUPS), "Header:\n  Type: PLAYER_GROUP_DB\n").unwrap();
        let mut s = load(&dir);
        assert!(s.player_commands_locked);
        s.player_commands = false;
        save(&dir, &s).unwrap();
        assert!(dir.join(GROUPS).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_bad_values() {
        let dir = temp_dir("bad");
        let ok = load(&dir);
        let bad = |f: fn(&mut GameSettings)| {
            let mut s = ok.clone();
            f(&mut s);
            save(&dir, &s).is_err()
        };
        assert!(bad(|s| s.drop_equip = 0.0));
        assert!(bad(|s| s.drop_mvp = 5000.0));
        assert!(bad(|s| s.death_penalty = -1.0));
        assert!(bad(|s| s.server_name = "My Server".into()));
        assert!(bad(|s| s.server_name = "a/b".into()));
        assert!(bad(|s| s.server_name = "x".repeat(20)));
        assert!(bad(|s| s.motd = "Héllo".into()));
        assert!(bad(|s| s.motd = "Note : this".into()));
        assert!(bad(|s| s.motd = "x\n".repeat(21)));
        assert!(!bad(|s| s.server_name = "Prontera_LAN".into()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn conf_file_set_and_get() {
        let dir = temp_dir("conf");
        let path = dir.join("x.txt");
        fs::write(&path, "a: 1\n// a: 9\nb: 2\na: 3\n").unwrap();
        let mut f = ConfFile::load(&path);
        assert_eq!(f.get("a").as_deref(), Some("3"));
        f.set("a", Some("4"));
        assert_eq!(f.lines, ["a: 4", "// a: 9", "b: 2"]);
        f.set("c", Some("5"));
        f.set("b", None);
        assert_eq!(f.lines, ["a: 4", "// a: 9", "c: 5"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
