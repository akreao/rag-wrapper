//! The battle config editor: rAthena's `conf/battle/*.conf` files with
//! their comments, read from the rAthena image so they match the server
//! that runs. Values changed in the editor are kept in the overrides
//! folder's `battle_conf.txt`, which rAthena reads after every battle conf
//! file, so the stock files stay as they are and a server update can't
//! undo a change.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::envfile::EnvFile;
use crate::settings::ConfFile;
use crate::{host, images, stack};

/// Where the image's battle conf files are copied to, beside the image ID
/// they came from, so they're read from the image only after a rebuild.
const CACHE: &str = ".battle-conf";
const OVERRIDE: &str = "battle_conf.txt";

/// Values the stack sets before the overrides folder is read
/// (`entrypoint.sh`), with why.
const STACK_VALUES: &[(&str, &str, &str)] = &[(
    "charcommand_symbol",
    "*",
    "rag-wrapper sets this to * because the 20250716 client keeps # for itself.",
)];

/// The commands' symbols, which rAthena reads as one character.
const SYMBOLS: &[&str] = &["atcommand_symbol", "charcommand_symbol"];

#[derive(Serialize, Debug, PartialEq)]
pub struct BattleConf {
    pub files: Vec<File>,
}

/// One `conf/battle/*.conf` file.
#[derive(Serialize, Debug, PartialEq)]
pub struct File {
    pub name: String,
    /// The comment at the top of the file, which explains its notes.
    pub intro: String,
    pub groups: Vec<Group>,
}

/// Options that share one comment, or a comment on its own (a heading).
#[derive(Serialize, Debug, PartialEq, Default)]
pub struct Group {
    pub comment: String,
    pub options: Vec<Setting>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Setting {
    pub key: String,
    /// What the server uses: the overrides folder's value, else the default.
    pub value: String,
    /// The value without an override: the stack's, else the stock file's.
    pub default: String,
    /// The overrides folder sets it.
    pub changed: bool,
    /// `switch` (yes/no), `symbol` or `number`.
    pub kind: &'static str,
    /// Why the stack sets its own default, if it does.
    pub note: Option<&'static str>,
    /// The map server turned the value down at its latest start.
    pub rejected: Option<Rejected>,
}

/// A value out of the option's range, which rAthena replaces with its
/// default and logs. The range is only known from that log line.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Rejected {
    pub min: i64,
    pub max: i64,
    /// What the server used instead.
    pub used: i64,
}

/// The battle conf files of the server's image, with the values the map
/// server rejected according to `map_log`, its log since it last started
/// (empty when it isn't running). Errors when the image isn't there yet
/// or Docker can't copy out of it.
pub fn load(dir: &Path, overrides: &Path, env: &EnvFile, map_log: &str) -> Result<BattleConf, String> {
    let stock = stock_dir(dir, env)?;
    let mut conf = read(&stock, overrides);
    mark_rejected(&mut conf, map_log);
    Ok(conf)
}

/// How rAthena's `battle_set_value` starts its warning about a value out
/// of range: `Value for setting 'key': value is invalid (min:0 max:1)!
/// Defaulting to 0...`
const REJECTED: &str = "Value for setting '";
/// A line `entrypoint.sh` writes on every map server start.
const START_LINE: &str = "Mods enabled:";

fn mark_rejected(conf: &mut BattleConf, log: &str) {
    let log = log.rfind(START_LINE).map_or(log, |i| &log[i..]);
    let mut rejected = BTreeMap::new();
    for line in log.lines() {
        if let Some((key, value, r)) = line.split_once(REJECTED).and_then(|(_, rest)| parse_rejected(rest)) {
            rejected.insert(key, (value, r));
        }
    }
    for setting in conf.files.iter_mut().flat_map(|f| &mut f.groups).flat_map(|g| &mut g.options) {
        // Only while the option still has the value that was turned down.
        if let Some((value, r)) = rejected.get(setting.key.as_str()) {
            if *value == setting.value {
                setting.rejected = Some(r.clone());
            }
        }
    }
}

/// `key': value is invalid (min:0 max:1)! Defaulting to 0...`
fn parse_rejected(rest: &str) -> Option<(&str, &str, Rejected)> {
    let (key, rest) = rest.split_once("': ")?;
    let (value, rest) = rest.split_once(" is invalid (min:")?;
    let (min, rest) = rest.split_once(" max:")?;
    let (max, rest) = rest.split_once(")! Defaulting to ")?;
    let used = rest.trim_end().trim_end_matches('.');
    let int = |s: &str| s.trim().parse::<i64>().ok();
    Some((key, value, Rejected { min: int(min)?, max: int(max)?, used: int(used)? }))
}

/// Writes `changes` (key to value) to the overrides folder. A value equal
/// to the default removes the override. Returns whether anything changed,
/// in which case the game servers need a restart to pick it up.
pub fn save(dir: &Path, overrides: &Path, env: &EnvFile, changes: &BTreeMap<String, String>) -> Result<bool, String> {
    let conf = read(&stock_dir(dir, env)?, overrides);
    apply(&conf, overrides, changes)
}

fn apply(conf: &BattleConf, overrides: &Path, changes: &BTreeMap<String, String>) -> Result<bool, String> {
    let settings: BTreeMap<&str, &Setting> = conf
        .files
        .iter()
        .flat_map(|f| &f.groups)
        .flat_map(|g| &g.options)
        .map(|s| (s.key.as_str(), s))
        .collect();

    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    for (key, value) in changes {
        let setting = settings.get(key.as_str()).ok_or_else(|| format!("{key} isn't a battle config option."))?;
        values.insert(setting.key.as_str(), validate(setting, value)?);
    }
    let symbol = |key: &str| values.get(key).cloned().or_else(|| settings.get(key).map(|s| s.value.clone()));
    if let (Some(at), Some(char)) = (symbol(SYMBOLS[0]), symbol(SYMBOLS[1])) {
        if at == char {
            return Err("atcommand_symbol and charcommand_symbol must be different.".into());
        }
    }

    fs::create_dir_all(overrides).map_err(|e| e.to_string())?;
    let path = overrides.join(OVERRIDE);
    let before = fs::read_to_string(&path).ok();
    let mut file = ConfFile::load(&path);
    for (key, value) in values {
        let default = &settings[key].default;
        file.set(key, (!same(&value, default)).then_some(value.as_str()));
    }
    file.save().map_err(|e| e.to_string())?;
    Ok(fs::read_to_string(&path).ok() != before)
}

/// Checks a value the way rAthena reads it, and returns it trimmed.
fn validate(setting: &Setting, value: &str) -> Result<String, String> {
    let key = &setting.key;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{key} needs a value."));
    }
    match setting.kind {
        "symbol" => {
            let mut chars = value.chars();
            let ok = matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_graphic() && !"/%$".contains(c));
            if !ok {
                return Err(format!("{key} must be one character, and not / % or $ (the client's command, party and guild chat symbols)."));
            }
        }
        "switch" if is_switch(value) => {}
        kind => {
            if number(value).is_none() {
                let what = if kind == "switch" { "yes, no or a number" } else { "a whole number (or hex like 0x1F)" };
                return Err(format!("{key} must be {what}, not \"{value}\"."));
            }
        }
    }
    Ok(value.to_owned())
}

fn is_switch(value: &str) -> bool {
    ["yes", "no", "on", "off", "true", "false"].iter().any(|s| s.eq_ignore_ascii_case(value))
}

/// A number as rAthena's `config_switch` reads it (`strtol` with base 0),
/// limited to what fits its 32-bit settings.
fn number(value: &str) -> Option<i64> {
    let (neg, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let n = if let Some(hex) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()?
    } else if digits.chars().all(|c| c.is_ascii_digit()) {
        digits.parse().ok()?
    } else {
        return None;
    };
    let n = if neg { -n } else { n };
    (i32::MIN as i64..=u32::MAX as i64).contains(&n).then_some(n)
}

/// Whether two values mean the same to rAthena, so `on` and `yes` or
/// `0x10` and `16` don't count as a change.
fn same(a: &str, b: &str) -> bool {
    let switch = |v: &str| match v.to_ascii_lowercase().as_str() {
        "yes" | "on" | "true" => Some(1),
        "no" | "off" | "false" => Some(0),
        _ => number(v),
    };
    a == b || matches!((switch(a), switch(b)), (Some(x), Some(y)) if x == y)
}

/// Copies `conf/battle/` out of the rAthena image the first time it's
/// needed after a build or pull, and returns where the copy is.
fn stock_dir(dir: &Path, env: &EnvFile) -> Result<PathBuf, String> {
    const NOT_BUILT: &str = "The server isn't built yet. Start it once, then open the battle config again.";
    let image = images::current(dir, env).into_iter().next().ok_or(NOT_BUILT)?.local;
    let id = images::image_id(&image).ok_or(NOT_BUILT)?;
    let cache = dir.join(CACHE);
    let files = cache.join("battle");
    if fs::read_to_string(cache.join("image")).ok().as_deref() == Some(id.as_str()) && files.is_dir() {
        return Ok(files);
    }

    let _ = fs::remove_dir_all(&cache);
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let mut create = host::command("docker");
    create.args(["create", image.as_str()]);
    let container = stack::output(create).map_err(|e| format!("Couldn't read the battle config from the server image: {e}"))?;
    let container = container.trim();
    let mut cp = host::command("docker");
    cp.args(["cp", &format!("{container}:/rathena/conf/battle")]).arg(&files);
    let copied = stack::output(cp);
    let mut rm = host::command("docker");
    rm.args(["rm", container]);
    let _ = stack::output(rm);
    copied.map_err(|e| format!("Couldn't read the battle config from the server image: {e}"))?;
    fs::write(cache.join("image"), &id).map_err(|e| e.to_string())?;
    Ok(files)
}

/// Reads the stock files in `stock` with the overrides folder's values on top.
fn read(stock: &Path, overrides: &Path) -> BattleConf {
    let overridden = ConfFile::load(&overrides.join(OVERRIDE));
    let mut names: Vec<String> = fs::read_dir(stock)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".conf"))
        .collect();
    names.sort();
    let files = names
        .into_iter()
        .map(|name| {
            let text = fs::read_to_string(stock.join(&name)).unwrap_or_default();
            parse(name, &text, &overridden)
        })
        .collect();
    BattleConf { files }
}

/// Splits a conf file into its options and the comments above them.
/// A blank line ends a group; options right after one another share the
/// comment above the first. Comments that aren't above an option are
/// headings, and the first ones are the file's introduction.
fn parse(name: String, text: &str, overridden: &ConfFile) -> File {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut groups: Vec<Group> = Vec::new();
    let mut current = Group::default();
    let mut comment: Vec<&str> = Vec::new();
    let mut flush = |current: &mut Group, comment: &mut Vec<&str>| {
        current.comment = comment.join("\n").trim().to_owned();
        comment.clear();
        let group = std::mem::take(current);
        if !group.comment.is_empty() || !group.options.is_empty() {
            groups.push(group);
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush(&mut current, &mut comment);
        } else if let Some(c) = line.strip_prefix("//") {
            if !current.options.is_empty() {
                flush(&mut current, &mut comment);
            }
            // Lines of dashes or equals signs only frame the text.
            if !(c.len() > 3 && c.trim().chars().all(|c| c == '-' || c == '=')) {
                comment.push(c.strip_prefix(' ').unwrap_or(c).trim_end());
            }
        } else if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            // rAthena reads the first word after the colon.
            let value = value.split_whitespace().next().unwrap_or_default();
            if key.eq_ignore_ascii_case("import") || key.is_empty() || value.is_empty() {
                continue;
            }
            current.options.push(setting(key, value, overridden));
        }
    }
    flush(&mut current, &mut comment);

    let first = groups.iter().position(|g| !g.options.is_empty()).unwrap_or(groups.len());
    let intro = groups.drain(..first).map(|g| g.comment).collect::<Vec<_>>().join("\n\n");
    File { name, intro, groups }
}

fn setting(key: &str, stock: &str, overridden: &ConfFile) -> Setting {
    let stack = STACK_VALUES.iter().find(|(k, ..)| *k == key);
    let default = stack.map_or(stock, |(_, v, _)| v).to_owned();
    let value = overridden.get(key);
    let kind = if SYMBOLS.contains(&key) {
        "symbol"
    } else if is_switch(stock) {
        "switch"
    } else {
        "number"
    };
    Setting {
        key: key.to_owned(),
        changed: value.is_some(),
        value: value.unwrap_or_else(|| default.clone()),
        default,
        kind,
        note: stack.map(|(.., why)| *why),
        rejected: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXP: &str = "\u{feff}//--------------------------------------------------------------\n\
        // rAthena Battle Configuration File\n\
        //--------------------------------------------------------------\n\
        // Note 1: Value is a config switch (on/off, yes/no or 1/0)\n\
        //--------------------------------------------------------------\n\
        \n\
        // Rate at which exp. is given. (Note 2)\n\
        base_exp_rate: 100\n\
        \n\
        // Allow multi level up until a certain level?\n\
        multi_level_up_base: 0\n\
        multi_level_up_job: 0\n\
        // 0: Never lose\n\
        //guild_skill_relog_type: 0\n\
        \n\
        // Turn this on to allow multi level up. (Note 1)\n\
        multi_level_up: no\t// trailing\n\
        import: conf/x.txt\n";

    const GM: &str = "// Symbols\natcommand_symbol: @\ncharcommand_symbol: #\n";

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("battleconf-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("stock")).unwrap();
        fs::write(dir.join("stock/exp.conf"), EXP).unwrap();
        fs::write(dir.join("stock/gm.conf"), GM).unwrap();
        fs::write(dir.join("stock/notes.txt"), "not a conf").unwrap();
        dir
    }

    fn find<'a>(conf: &'a BattleConf, key: &str) -> &'a Setting {
        conf.files.iter().flat_map(|f| &f.groups).flat_map(|g| &g.options).find(|s| s.key == key).unwrap()
    }

    #[test]
    fn parses_comments_and_groups() {
        let dir = temp_dir("parse");
        let conf = read(&dir.join("stock"), &dir.join("overrides"));
        assert_eq!(conf.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["exp.conf", "gm.conf"]);
        let exp = &conf.files[0];
        assert_eq!(exp.intro, "rAthena Battle Configuration File\nNote 1: Value is a config switch (on/off, yes/no or 1/0)");
        assert_eq!(exp.groups.len(), 4);
        assert_eq!(exp.groups[0].comment, "Rate at which exp. is given. (Note 2)");
        assert_eq!(exp.groups[0].options[0].kind, "number");
        let keys: Vec<_> = exp.groups[1].options.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["multi_level_up_base", "multi_level_up_job"]);
        // A comment right after options starts a group of its own.
        assert_eq!(exp.groups[2].comment, "0: Never lose\nguild_skill_relog_type: 0");
        assert!(exp.groups[2].options.is_empty());
        let multi = &exp.groups[3].options;
        assert_eq!((multi.len(), multi[0].value.as_str(), multi[0].kind), (1, "no", "switch"));

        let gm = &conf.files[1];
        assert_eq!(gm.intro, "");
        assert_eq!(gm.groups[0].options[1].kind, "symbol");
        // The stack's own value is the default.
        let char = find(&conf, "charcommand_symbol");
        assert_eq!((char.value.as_str(), char.default.as_str(), char.changed), ("*", "*", false));
        assert!(char.note.is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saves_overrides_and_keeps_other_lines() {
        let dir = temp_dir("save");
        let (stock, overrides) = (dir.join("stock"), dir.join("overrides"));
        fs::create_dir_all(&overrides).unwrap();
        fs::write(overrides.join(OVERRIDE), "// mine\nitem_rate_card: 500\n").unwrap();

        let changes = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let conf = read(&stock, &overrides);
        assert!(apply(&conf, &overrides, &changes(&[("base_exp_rate", " 300 "), ("multi_level_up", "yes"), ("charcommand_symbol", "#")])).unwrap());
        assert_eq!(
            fs::read_to_string(overrides.join(OVERRIDE)).unwrap(),
            "// mine\nitem_rate_card: 500\nbase_exp_rate: 300\ncharcommand_symbol: #\nmulti_level_up: yes\n"
        );
        let conf = read(&stock, &overrides);
        let rate = find(&conf, "base_exp_rate");
        assert_eq!((rate.value.as_str(), rate.default.as_str(), rate.changed), ("300", "100", true));

        // Back to the default (or something meaning the same) drops the line.
        assert!(apply(&conf, &overrides, &changes(&[("base_exp_rate", "100"), ("multi_level_up", "off"), ("charcommand_symbol", "*")])).unwrap());
        assert_eq!(fs::read_to_string(overrides.join(OVERRIDE)).unwrap(), "// mine\nitem_rate_card: 500\n");
        let conf = read(&stock, &overrides);
        assert!(!apply(&conf, &overrides, &changes(&[("base_exp_rate", "100")])).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn marks_values_the_server_rejected() {
        let dir = temp_dir("rejected");
        let (stock, overrides) = (dir.join("stock"), dir.join("overrides"));
        fs::create_dir_all(&overrides).unwrap();
        fs::write(overrides.join(OVERRIDE), "base_exp_rate: 0\nmulti_level_up_base: 9999\n").unwrap();
        let mut conf = read(&stock, &overrides);
        let log = "[Warning]: Value for setting 'base_exp_rate': 5 is invalid (min:1 max:100)! Defaulting to 100...\n\
            Mods enabled: \n\
            \x1b[33m[Warning]\x1b[0m: Value for setting 'base_exp_rate': 0 is invalid (min:1 max:1000000000)! Defaulting to 100...\n\
            [Warning]: Value for setting 'multi_level_up_base': 50 is invalid (min:0 max:10)! Defaulting to 0...\n";
        mark_rejected(&mut conf, log);
        assert_eq!(find(&conf, "base_exp_rate").rejected, Some(Rejected { min: 1, max: 1_000_000_000, used: 100 }));
        // Changed since the server turned it down.
        assert_eq!(find(&conf, "multi_level_up_base").rejected, None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_bad_values() {
        let dir = temp_dir("bad");
        let conf = read(&dir.join("stock"), &dir.join("overrides"));
        let bad = |key: &str, value: &str| apply(&conf, &dir.join("overrides"), &[(key.to_owned(), value.to_owned())].into()).is_err();
        assert!(bad("base_exp_rate", "1.5"));
        assert!(bad("base_exp_rate", ""));
        assert!(bad("base_exp_rate", "99999999999"));
        assert!(bad("multi_level_up", "maybe"));
        assert!(bad("charcommand_symbol", "%"));
        assert!(bad("charcommand_symbol", "##"));
        assert!(bad("charcommand_symbol", "@"));
        assert!(bad("no_such_option", "1"));
        assert!(!bad("base_exp_rate", "0x1F"));
        assert!(!bad("multi_level_up", "1"));
        assert!(!bad("charcommand_symbol", "!"));
        fs::remove_dir_all(dir).unwrap();
    }
}
