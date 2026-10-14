//! The Mods screen: server-side mods in the stack's `mods/` folder, one
//! folder each, switched on by name in `mods/enabled.txt`, whose order is
//! the load order. Compose mounts
//! the folder into the rAthena containers, and `entrypoint.sh` loads the
//! enabled mods' NPC scripts and database files on every start, so changes
//! apply when the game servers restart.
//!
//! `mods/.applied.json` records what the server loaded at its last start
//! from the app, so the screen can say which changes wait for a restart
//! and the restart can back up the database before new database files.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ENABLED: &str = "enabled.txt";
const APPLIED: &str = ".applied.json";

const ENABLED_HEADER: &str = "# The mods that are on, one folder name per line, in load order: where\n\
                              # two change the same entry, the later one wins. Rewritten by the\n\
                              # desktop app's Mods screen.\n";

/// Error and warning lines shown per mod, newest last.
const MAX_ERRORS: usize = 20;
/// Lines kept from one error, which for a script error includes the code around it.
const MAX_ERROR_LINES: usize = 14;

#[derive(Serialize, Debug, PartialEq)]
pub struct Mod {
    pub name: String,
    pub enabled: bool,
    /// The NPC scripts it loads, relative to the mod's folder.
    pub scripts: Vec<String>,
    /// The database files it loads, relative to the mod's folder.
    pub databases: Vec<String>,
    /// Why it can't be switched on.
    pub problem: Option<String>,
    /// Switched on or off, or its files changed, since the running server started.
    pub pending: bool,
    /// Errors and warnings about its files in the running map server's log.
    pub errors: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct Status {
    pub folder: String,
    pub mods: Vec<Mod>,
    /// The running game servers don't have the mods as they are now.
    pub restart_needed: bool,
    /// Mods are on, but the running image predates mod loading and ignores them.
    pub rebuild_needed: bool,
}

/// What `entrypoint.sh` prints on every start once it loads mods.
const LOADER_LINE: &str = "Mods enabled:";

/// What a mod's files were when loaded: checksums of their names, sizes
/// and modification times, all of them and the database ones alone.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
struct Fingerprint {
    files: String,
    db: String,
}

/// The rule `entrypoint.sh` applies to folder names, which end up in
/// config and YAML files.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The names in `enabled.txt` in load order, whether or not their folders exist.
fn enabled(dir: &Path) -> Vec<String> {
    let mut seen = BTreeSet::new();
    fs::read_to_string(dir.join(ENABLED))
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim())
        .filter(|l| !l.is_empty() && seen.insert(l.to_owned()))
        .map(str::to_owned)
        .collect()
}

fn write_enabled(dir: &Path, names: &[String]) -> Result<(), String> {
    let mut text = ENABLED_HEADER.to_owned();
    for n in names {
        text.push_str(n);
        text.push('\n');
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(ENABLED), text).map_err(|e| e.to_string())
}

/// The files a mod loads, as `entrypoint.sh` finds them: `.txt` files
/// anywhere under `npc/`, and `.yml` files in `db/`, `db/pre-re/` and
/// `db/re/` (only one of the last two loads, depending on the game mode).
fn files(root: &Path) -> (Vec<String>, Vec<String>) {
    let mut scripts = Vec::new();
    walk(root, "npc", &mut |rel| {
        if rel.to_ascii_lowercase().ends_with(".txt") {
            scripts.push(rel.to_owned());
        }
    });
    scripts.sort();
    let mut databases = Vec::new();
    for sub in ["db", "db/pre-re", "db/re"] {
        let Ok(entries) = fs::read_dir(root.join(sub)) else { continue };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".yml"))
            .map(|n| format!("{sub}/{n}"))
            .collect();
        names.sort();
        databases.extend(names);
    }
    (scripts, databases)
}

fn walk(root: &Path, rel: &str, found: &mut dyn FnMut(&str)) {
    let Ok(entries) = fs::read_dir(root.join(rel)) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let Ok(name) = entry.file_name().into_string() else { continue };
        let path = format!("{rel}/{name}");
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(root, &path, found),
            Ok(t) if t.is_file() => found(&path),
            _ => {}
        }
    }
}

fn checksum(root: &Path, files: &[String]) -> String {
    let mut hash = Sha256::new();
    for rel in files {
        let meta = fs::metadata(root.join(rel)).ok();
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        hash.update(format!("{rel}\0{}\0{modified}\n", meta.map_or(0, |m| m.len())));
    }
    format!("{:x}", hash.finalize())
}

fn fingerprint(root: &Path) -> Fingerprint {
    let (scripts, databases) = files(root);
    let all: Vec<String> = scripts.iter().chain(&databases).cloned().collect();
    Fingerprint {
        files: checksum(root, &all),
        db: checksum(root, &databases),
    }
}

/// The mod folders, sorted by name.
fn folders(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// The enabled mods `entrypoint.sh` would load now, in order, with their fingerprints.
fn current(dir: &Path) -> Vec<(String, Fingerprint)> {
    enabled(dir)
        .into_iter()
        .filter(|n| valid_name(n) && dir.join(n).is_dir())
        .map(|n| {
            let print = fingerprint(&dir.join(&n));
            (n, print)
        })
        .collect()
}

fn applied(dir: &Path) -> Vec<(String, Fingerprint)> {
    fs::read(dir.join(APPLIED))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Records the enabled mods as loaded, after the game servers started.
pub fn record_applied(dir: &Path) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let json = serde_json::to_vec_pretty(&current(dir)).map_err(io::Error::other)?;
    fs::write(dir.join(APPLIED), json)
}

/// The enabled mods whose database files the server hasn't loaded yet,
/// which the next start backs up the database for.
pub fn new_databases(dir: &Path) -> Vec<String> {
    let applied: BTreeMap<String, Fingerprint> = applied(dir).into_iter().collect();
    current(dir)
        .into_iter()
        .filter(|(name, print)| {
            let (_, databases) = files(&dir.join(name));
            !databases.is_empty() && applied.get(name).map(|a| &a.db) != Some(&print.db)
        })
        .map(|(name, _)| name)
        .collect()
}

/// Every mod folder and its state: the enabled ones first, in load order,
/// then the rest by name. `running` says whether the map server is up, and
/// `log` is its log, searched for errors in each mod's files.
pub fn status(dir: &Path, running: bool, log: &str) -> Status {
    let on = enabled(dir);
    let loaded = applied(dir);
    let applied: BTreeMap<&String, &Fingerprint> = loaded.iter().map(|(n, p)| (n, p)).collect();
    let mut errors = errors_by_mod(log);
    let mut names = folders(dir);
    names.sort_by_key(|n| on.iter().position(|e| e == n).unwrap_or(usize::MAX));
    let mods: Vec<Mod> = names
        .into_iter()
        .map(|name| {
            let root = dir.join(&name);
            let (scripts, databases) = files(&root);
            let problem = if !valid_name(&name) {
                Some("Rename the folder to switch it on: only letters, digits, . _ and - work in a mod's name.".to_owned())
            } else if scripts.is_empty() && databases.is_empty() {
                Some("Nothing to load: put NPC scripts (.txt) in npc/ or database files (.yml) in db/.".to_owned())
            } else {
                None
            };
            let enabled = on.contains(&name) && valid_name(&name);
            let pending = running
                && match (enabled, applied.get(&name)) {
                    (true, Some(a)) => **a != fingerprint(&root),
                    (true, None) => true,
                    (false, loaded) => loaded.is_some(),
                };
            Mod {
                errors: errors.remove(&name).unwrap_or_default(),
                name,
                enabled,
                scripts,
                databases,
                problem,
                pending,
            }
        })
        .collect();
    Status {
        folder: dir.display().to_string(),
        // A new order counts too.
        restart_needed: running && current(dir) != loaded,
        rebuild_needed: running && !on.is_empty() && !log.lines().any(|l| l.starts_with(LOADER_LINE)),
        mods,
    }
}

/// Switches a mod on or off in `enabled.txt`.
pub fn set_enabled(dir: &Path, name: &str, on: bool) -> Result<(), String> {
    if !valid_name(name) {
        return Err(format!("\"{name}\" can't be a mod's name: only letters, digits, . _ and - work."));
    }
    if on && !dir.join(name).is_dir() {
        return Err(format!("There's no {name} folder in the mods folder any more."));
    }
    let mut names = enabled(dir);
    names.retain(|n| n != name);
    // A mod switched on loads last, after the ones already on.
    if on {
        names.push(name.to_owned());
    }
    write_enabled(dir, &names)
}

/// Moves an enabled mod one place earlier (`up`) or later in the load
/// order, past the next mod whose folder still exists.
pub fn move_mod(dir: &Path, name: &str, up: bool) -> Result<(), String> {
    let mut names = enabled(dir);
    let i = names.iter().position(|n| n == name).ok_or(format!("{name} isn't switched on."))?;
    let exists = |n: &String| valid_name(n) && dir.join(n).is_dir();
    let j = if up {
        names[..i].iter().rposition(exists)
    } else {
        names[i + 1..].iter().position(exists).map(|j| i + 1 + j)
    };
    if let Some(j) = j {
        names.swap(i, j);
        write_enabled(dir, &names)?;
    }
    Ok(())
}

/// rAthena's errors and warnings that name a file in a mod, by mod. Each
/// starts at a line with its `[Error]` or `[Warning]` tag and runs until
/// the next tagged line, which keeps a script error's code excerpt. A
/// database error names its file in a second tagged line, which stays
/// with the first.
fn errors_by_mod(log: &str) -> BTreeMap<String, Vec<String>> {
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut block: Vec<String> = Vec::new();
    let mut flush = |block: &mut Vec<String>| {
        if block.is_empty() {
            return;
        }
        let text = block.join("\n");
        block.clear();
        let mut names: Vec<&str> = mod_names(&text).collect();
        names.dedup();
        for name in names {
            let list = found.entry(name.to_owned()).or_default();
            list.push(text.clone());
            if list.len() > MAX_ERRORS {
                list.remove(0);
            }
        }
    };
    for raw in log.lines() {
        // Progress messages are overwritten with \r; the last one stands.
        let line = clean(raw.rsplit('\r').next().unwrap_or_default());
        if line.starts_with('[') {
            if !block.is_empty() && line.contains("Occurred in file") {
                block.push(line);
                continue;
            }
            flush(&mut block);
            if line.contains("[Error]") || line.contains("[Warning]") {
                block.push(line);
            }
        } else if !block.is_empty() && block.len() < MAX_ERROR_LINES && !line.trim().is_empty() {
            block.push(line);
        }
    }
    flush(&mut block);
    found
}

/// The mod names in `mods/<name>/` paths in `text`.
fn mod_names(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("mods/").filter_map(move |(i, m)| {
        let before = text[..i].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || "._-/".contains(c)) {
            return None;
        }
        let rest = &text[i + m.len()..];
        let name = &rest[..rest.find(['/', '\'', '"', '\n'])?];
        (rest[name.len()..].starts_with('/') && !name.is_empty()).then_some(name)
    })
}

/// A log line without colour codes or the bell rAthena rings on script errors.
fn clean(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI sequences end with a letter.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mods-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn lists_mods_and_what_they_load() {
        let dir = temp_dir("list");
        write(&dir, "b-shop/npc/sub/shop.txt", "x");
        write(&dir, "b-shop/npc/README.md", "x");
        write(&dir, "b-shop/db/item_db.yml", "x");
        write(&dir, "b-shop/db/pre-re/mob_db.yml", "x");
        write(&dir, "a-empty/notes.txt", "x");
        write(&dir, "bad name/npc/x.txt", "x");
        write(&dir, ".hidden/npc/x.txt", "x");
        write(&dir, "README.md", "x");

        let s = status(&dir, false, "");
        let names: Vec<&str> = s.mods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["a-empty", "b-shop", "bad name"]);
        assert!(s.mods[0].problem.as_deref().unwrap().starts_with("Nothing to load"));
        assert_eq!(s.mods[1].scripts, ["npc/sub/shop.txt"]);
        assert_eq!(s.mods[1].databases, ["db/item_db.yml", "db/pre-re/mob_db.yml"]);
        assert!(s.mods[1].problem.is_none());
        assert!(s.mods[2].problem.as_deref().unwrap().starts_with("Rename"));
        assert!(!s.restart_needed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn switching_on_waits_for_a_restart() {
        let dir = temp_dir("pending");
        write(&dir, "shop/npc/shop.txt", "x");
        write(&dir, "items/db/item_db.yml", "x");
        write(&dir, ENABLED, "# by hand\nwarper # gone\n");

        set_enabled(&dir, "shop", true).unwrap();
        assert!(set_enabled(&dir, "nope", true).is_err());
        assert!(set_enabled(&dir, "../x", false).is_err());
        assert_eq!(fs::read_to_string(dir.join(ENABLED)).unwrap(), format!("{ENABLED_HEADER}warper\nshop\n"));

        // Stopped: nothing is pending, it loads at the next start.
        assert!(!status(&dir, false, "").mods.iter().any(|m| m.pending));
        let s = status(&dir, true, "");
        assert!(s.restart_needed);
        assert!(s.mods.iter().find(|m| m.name == "shop").unwrap().pending);
        assert!(new_databases(&dir).is_empty());

        record_applied(&dir).unwrap();
        let s = status(&dir, true, "");
        assert!(!s.restart_needed);
        assert!(!s.mods.iter().any(|m| m.pending));

        // A mod with database files asks for a backup until it's loaded.
        set_enabled(&dir, "items", true).unwrap();
        assert_eq!(new_databases(&dir), ["items"]);
        record_applied(&dir).unwrap();
        assert!(new_databases(&dir).is_empty());

        // Editing a file is a change too.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&dir, "items/db/item_db.yml", "changed");
        assert_eq!(new_databases(&dir), ["items"]);
        assert!(status(&dir, true, "").mods.iter().find(|m| m.name == "items").unwrap().pending);
        record_applied(&dir).unwrap();

        // Switching off shows as pending until the restart.
        set_enabled(&dir, "shop", false).unwrap();
        let s = status(&dir, true, "");
        assert!(s.restart_needed);
        assert!(s.mods.iter().find(|m| m.name == "shop").unwrap().pending);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_order_follows_enabled_txt() {
        let dir = temp_dir("order");
        for m in ["a", "b", "c", "d"] {
            write(&dir, &format!("{m}/npc/x.txt"), "x");
        }
        write(&dir, ENABLED, "c\ngone\na\nc\n");
        set_enabled(&dir, "b", true).unwrap();
        assert_eq!(enabled(&dir), ["c", "gone", "a", "b"]);
        let order = |dir: &Path| status(dir, false, "").mods.into_iter().map(|m| m.name).collect::<Vec<_>>();
        assert_eq!(order(&dir), ["c", "a", "b", "d"]);

        record_applied(&dir).unwrap();
        // Up skips the missing folder; the first can't go further up.
        move_mod(&dir, "a", true).unwrap();
        assert_eq!(enabled(&dir), ["a", "gone", "c", "b"]);
        move_mod(&dir, "a", true).unwrap();
        move_mod(&dir, "b", false).unwrap();
        assert_eq!(enabled(&dir), ["a", "gone", "c", "b"]);
        assert!(move_mod(&dir, "d", true).is_err());
        // Reordering waits for a restart too.
        assert!(status(&dir, true, "").restart_needed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spots_an_image_without_mod_loading() {
        let dir = temp_dir("rebuild");
        write(&dir, "shop/npc/x.txt", "x");
        assert!(!status(&dir, true, "[Info]: old image\n").rebuild_needed);
        set_enabled(&dir, "shop", true).unwrap();
        assert!(status(&dir, true, "[Info]: old image\n").rebuild_needed);
        assert!(!status(&dir, true, "Mods enabled: shop \nMod: shop\n").rebuild_needed);
        assert!(!status(&dir, false, "").rebuild_needed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_errors_in_mod_files() {
        let log = "\
Mod: shop
[Status]: Loading 'mods/items/db/item_db.yml'...\r[Warning]: Database version 4 is outdated and should be updated. Current version is: 5
[Warning]: mods/items/db/bogus.yml skipped: it isn't named after one of rAthena's databases in db/import
[Status]: Loading '12' entries in 'mods/items/db/item_db.yml'
[Error]: \x07
script error on mods/shop/npc/shop.txt line 3
    parse_line: need ';'
     1: prontera,150,150,4\tscript\tShop\t4_M_01,{
*    3:     mes \"hi\"'
\x1b[0m[Error]: npc_parse_script: Missing left curly in file 'npc/custom/other.txt', line '2'.
[Info]: Done loading '3000' NPCs
[Error]: Node \"Id\" is missing a value.
[Error]: Occurred in file 'mods/items/db/item_db.yml' on line 4 and column 5.
";
        let found = errors_by_mod(log);
        assert_eq!(found.len(), 2);
        let shop = &found["shop"];
        assert_eq!(shop.len(), 1);
        assert!(shop[0].starts_with("[Error]:\nscript error on mods/shop/npc/shop.txt line 3\n    parse_line: need ';'"));
        assert!(shop[0].contains("*    3:"));
        let items = &found["items"];
        assert_eq!(items.len(), 2);
        assert!(items[0].starts_with("[Warning]: mods/items/db/bogus.yml"));
        assert_eq!(
            items[1],
            "[Error]: Node \"Id\" is missing a value.\n[Error]: Occurred in file 'mods/items/db/item_db.yml' on line 4 and column 5."
        );
        assert!(!found.contains_key("other"));
    }

    #[test]
    fn mod_names_in_paths() {
        let names: Vec<&str> = mod_names("in 'mods/a-b/x.txt' and npc/mods/c/ and mods/d and \"mods/bad name/\"").collect();
        assert_eq!(names, ["a-b", "bad name"]);
    }
}
