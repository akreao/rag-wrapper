//! The custom item kit: a template mod with both halves of two custom
//! items, which the Mods screen's "New item mod" button copies into the
//! mods folder, and a check that a mod's custom items have both halves.
//!
//! The server half is an entry in the mod's `db/item_db.yml`. The client
//! half is an entry in `tbl_custom` in the mod's
//! `client/SystemEN/itemInfo_C.lua`: llchrisll's English translation, which
//! the players install, loads that file from its `SystemEN/itemInfo.lua`
//! and adds those items to its own list, so a mod never has to carry the
//! whole list.

use std::collections::BTreeSet;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use crate::luamerge;

/// Item Ids reserved for custom items: free in rAthena's item databases and
/// in the client's own item list.
pub const CUSTOM_IDS: RangeInclusive<u32> = 40000..=49999;

/// The translation's file for a server's own items, from the client folder.
const ITEM_INFO_C: &str = "SystemEN/itemInfo_C.lua";

/// The template's files, as the new mod gets them.
const TEMPLATE: [(&str, &[u8]); 4] = [
    ("README.md", include_bytes!("../item-mod/README.md")),
    ("db/item_db.yml", include_bytes!("../item-mod/db/item_db.yml")),
    ("npc/item_shop.txt", include_bytes!("../item-mod/npc/item_shop.txt")),
    ("client/SystemEN/itemInfo_C.lua", include_bytes!("../item-mod/client/SystemEN/itemInfo_C.lua")),
];

/// Copies the template into a new mod folder called `name`, switched off,
/// and returns its path.
pub fn create(mods_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if !crate::mods::valid_name(name) {
        return Err(format!("\"{name}\" can't be a mod's name: only letters, digits, . _ and - work."));
    }
    let root = mods_dir.join(name);
    if root.exists() {
        return Err(format!("The mods folder already has a {name} folder. Pick another name."));
    }
    for (rel, content) in TEMPLATE {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap_or(&root)).map_err(|e| e.to_string())?;
        fs::write(&path, content).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(root)
}

/// Problems with a mod's custom items: an item in the custom Id range that
/// its `db/` adds without a `tbl_custom` entry in its client files, which
/// players see as an unknown item, or the other way round, and itemInfo
/// files that would stand in for the players' whole item list.
pub fn check(root: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    let server = server_ids(root);
    let mut client: BTreeSet<u32> = BTreeSet::new();
    let mut info_files = Vec::new();
    for rel in client_files(root) {
        let lower = rel.to_ascii_lowercase();
        let file = lower.rsplit('/').next().unwrap_or_default();
        if !file.starts_with("iteminfo") || !(file.ends_with(".lua") || file.ends_with(".lub")) {
            continue;
        }
        let shared = mode_free(&lower);
        if shared.eq_ignore_ascii_case(ITEM_INFO_C) {
            let text = fs::read(root.join("client").join(&rel)).unwrap_or_default();
            if let Some(why) = luamerge::unmergeable(&text) {
                warnings.push(format!("client/{rel}: {why}, so its items can't be checked or merged with other mods'."));
            }
            client.extend(
                luamerge::keys(&text)
                    .into_iter()
                    .filter(|(table, _)| table == "tbl_custom")
                    .filter_map(|(_, key)| key.parse::<u32>().ok()),
            );
            info_files.push(format!("client/{rel}"));
        } else {
            warnings.push(format!(
                "client/{rel}: players' clients read items from the English translation's SystemEN/itemInfo.lua, so this file either replaces every item's name or isn't read at all. Put the mod's items in tbl_custom in client/{ITEM_INFO_C} instead."
            ));
        }
    }
    let where_client = info_files.first().cloned().unwrap_or_else(|| format!("client/{ITEM_INFO_C}"));
    for id in server.iter().filter(|id| CUSTOM_IDS.contains(id) && !client.contains(id)) {
        warnings.push(format!(
            "Item {id} is in the mod's item_db.yml but not in tbl_custom in {where_client}: players would see an unknown item."
        ));
    }
    for id in client.iter().filter(|id| !server.contains(id)) {
        warnings.push(format!(
            "Item {id} is in {where_client} but not in the mod's item_db.yml: the server has no such item, unless another mod adds it."
        ));
    }
    warnings
}

/// The `Id`s in the mod's `item_db.yml` files, both game modes'.
fn server_ids(root: &Path) -> BTreeSet<u32> {
    ["db", "db/pre-re", "db/re"]
        .iter()
        .filter_map(|sub| fs::read_to_string(root.join(sub).join("item_db.yml")).ok())
        .flat_map(|text| {
            text.lines()
                .filter_map(|l| l.trim_start().strip_prefix("- Id:")?.split('#').next()?.trim().parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A client file's path without its `Pre-Renewal/` or `Renewal/` folder.
fn mode_free(rel: &str) -> &str {
    for mode in ["pre-renewal/", "renewal/"] {
        if rel.len() >= mode.len() && rel[..mode.len()].eq_ignore_ascii_case(mode) {
            return &rel[mode.len()..];
        }
    }
    rel
}

/// The mod's client files, relative to its `client/` folder.
fn client_files(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut dirs = vec![String::new()];
    while let Some(rel) = dirs.pop() {
        let Ok(entries) = fs::read_dir(root.join("client").join(&rel)) else { continue };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(path),
                Ok(t) if t.is_file() => found.push(path),
                _ => {}
            }
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("itemkit-test-{name}-{}", std::process::id()));
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
    fn the_template_has_both_halves_of_each_item() {
        let dir = temp_dir("template");
        let root = create(&dir, " my-items ").unwrap();
        assert_eq!(root, dir.join("my-items"));
        assert!(root.join("npc/item_shop.txt").is_file());
        assert_eq!(check(&root), Vec::<String>::new());
        assert_eq!(server_ids(&root), BTreeSet::from([40000, 40001]));
        // The template's Lua is plain ASCII, so it reads the same in any code page.
        let lua = fs::read(root.join("client/SystemEN/itemInfo_C.lua")).unwrap();
        assert!(lua.is_ascii());
        let keys = luamerge::keys(&lua);
        assert!(keys.contains(&("tbl_custom".to_owned(), "40001".to_owned())));

        assert!(create(&dir, "my-items").unwrap_err().contains("already has"));
        assert!(create(&dir, "../x").is_err());
        assert!(!dir.join(crate::mods::ENABLED).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_half_is_reported() {
        let dir = temp_dir("halves");
        write(
            &dir,
            "db/re/item_db.yml",
            "Header:\n  Type: ITEM_DB\n  Version: 3\nBody:\n  - Id: 40000\n    AegisName: A\n  - Id: 40002 # no client half\n  - Id: 501\n    Buy: 1\n",
        );
        write(
            &dir,
            "client/Renewal/SystemEN/iteminfo_c.lua",
            "tbl_custom = {\n\t[40000] = { identifiedDisplayName = \"A\" },\n\t[40003] = { identifiedDisplayName = \"B\" },\n}\ntbl_override = {\n\t[501] = { identifiedDisplayName = \"Red\" },\n}\n",
        );
        write(&dir, "client/System/itemInfo_true_V5.lua", "tbl = {}\n");
        let warnings = check(&dir);
        assert_eq!(warnings.len(), 3, "{warnings:#?}");
        assert!(warnings[0].starts_with("client/System/itemInfo_true_V5.lua: players' clients read items from"));
        assert_eq!(
            warnings[1],
            "Item 40002 is in the mod's item_db.yml but not in tbl_custom in client/Renewal/SystemEN/iteminfo_c.lua: players would see an unknown item."
        );
        assert!(warnings[2].starts_with("Item 40003 is in client/Renewal/SystemEN/iteminfo_c.lua but not in the mod's item_db.yml"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mods_without_custom_items_have_nothing_to_report() {
        let dir = temp_dir("none");
        write(&dir, "db/item_db.yml", "Body:\n  - Id: 501\n    Buy: 5\n");
        write(&dir, "client/SystemEN/OngoingQuests_C.lub", "x");
        assert!(check(&dir).is_empty());
        let empty = temp_dir("empty");
        assert!(check(&empty).is_empty());
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(empty).unwrap();
    }
}
