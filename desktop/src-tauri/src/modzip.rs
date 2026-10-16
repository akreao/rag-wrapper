//! The Mods screen's "Install from zip" button: unpacks a mod someone sent
//! as a zip into a new folder in the mods folder, switched off.
//!
//! A zip is untrusted, so nothing in it is written outside the new folder:
//! an entry whose path climbs out of it (`..`, an absolute path, a drive
//! letter) stops the install, links are left out, and the unpacked size is
//! capped. It unpacks into a hidden folder first, which the Mods screen
//! ignores, and only takes the mod's name once it's complete.
//!
//! A zip for a mod that's already installed, as when updating it, replaces
//! the old folder only once the screen has asked. The mod keeps its place in
//! `enabled.txt`, so it stays on or off and keeps its load order.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Seek};
use std::path::Path;

use serde::Serialize;
use zip::ZipArchive;

use crate::mods;

/// The most a zip may unpack to. A mod with client files (sprites, a GRF)
/// can be large, but not this large.
const MAX_BYTES: u64 = 4 << 30;
/// The biggest zip the screen sends. The whole zip is held in memory, in
/// the window and again in the app, so a bigger one is unzipped by hand
/// into the mods folder instead. `ui/app.js` checks the same size first.
pub const MAX_ZIP: u64 = 1 << 30;
/// The most files and folders a zip may have.
const MAX_ENTRIES: usize = 100_000;

/// The folders `entrypoint.sh` and the client patch read from a mod.
const MOD_FOLDERS: [&str; 3] = ["npc", "db", "client"];

#[derive(Serialize, Debug, PartialEq)]
pub struct Installed {
    /// The new mod's folder name.
    pub name: String,
    /// How many files it got.
    pub files: usize,
    /// What was left out of the zip, and why.
    pub skipped: Vec<String>,
    /// The mods folder already has this mod, and nothing was installed:
    /// the screen asks before installing again with `replace`.
    pub exists: bool,
}

/// One entry, with its path split into folder and file names.
struct Entry {
    index: usize,
    parts: Vec<String>,
    dir: bool,
    link: bool,
}

/// Unpacks the zip `bytes`, sent as the file `file_name`, into a new mod
/// folder in `mods_dir`. The mod's name is the zip's single top folder when
/// everything but loose files is inside one, as when someone zips a mod's
/// folder (perhaps with a README beside it), and otherwise the zip's file
/// name. With `replace`, the zip's files replace an installed mod of that
/// name.
pub fn install(mods_dir: &Path, file_name: &str, bytes: &[u8], replace: bool) -> Result<Installed, String> {
    if bytes.len() as u64 > MAX_ZIP {
        return Err(too_big(file_name));
    }
    let mut zip = ZipArchive::new(io::Cursor::new(bytes)).map_err(|e| format!("{file_name} isn't a zip file the app can read: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{file_name} has more than {MAX_ENTRIES} files, too many for a mod."));
    }
    let mut entries = Vec::new();
    for index in 0..zip.len() {
        let file = zip.by_index_raw(index).map_err(|e| format!("{file_name}: {e}"))?;
        let raw = file.name().map_err(|e| format!("{file_name}: {e}"))?.into_owned();
        let parts = safe_parts(&raw).ok_or_else(|| {
            format!("{file_name} has a file that would land outside the mod's folder ({raw}), so it wasn't installed.")
        })?;
        let ignored = parts.is_empty()
            || parts[0] == "__MACOSX"
            || parts.last().is_some_and(|n| n == ".DS_Store" || n.eq_ignore_ascii_case("Thumbs.db"));
        if ignored {
            continue;
        }
        let link = file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000);
        entries.push(Entry { index, parts, dir: file.is_dir(), link });
    }

    let (name, strip) = mod_name(file_name, &entries);
    if !entries.iter().any(|e| e.parts.len() > strip && MOD_FOLDERS.contains(&e.parts[strip].as_str())) {
        return Err(format!(
            "{file_name} has no npc/, db/ or client/ folder{}, so it isn't a mod. A mod's zip has those folders at its top, or inside one folder.",
            if strip == 1 { format!(" in {name}/") } else { String::new() }
        ));
    }
    let root = mods_dir.join(&name);
    let exists = fs::symlink_metadata(&root).is_ok();
    if exists && !replace {
        return Ok(Installed { name, files: 0, skipped: Vec::new(), exists: true });
    }
    if exists && !root.is_dir() {
        return Err(format!("The mods folder has a file called {name} where the mod's folder would go. Move it, then install the zip again."));
    }

    fs::create_dir_all(mods_dir).map_err(|e| e.to_string())?;
    let temp = mods_dir.join(format!(".install-{name}"));
    if fs::symlink_metadata(&temp).is_ok() {
        fs::remove_dir_all(&temp).map_err(|e| format!("{}: {e}", temp.display()))?;
    }
    let result = unpack(&mut zip, &entries, strip, &temp).and_then(|(files, skipped)| {
        if exists {
            // The old folder moves aside before the new one takes its name,
            // so a failure leaves one of them in place.
            let old = mods_dir.join(format!(".replaced-{name}"));
            if fs::symlink_metadata(&old).is_ok() {
                fs::remove_dir_all(&old).map_err(|e| format!("{}: {e}", old.display()))?;
            }
            fs::rename(&root, &old).map_err(|e| format!("{}: {e}", root.display()))?;
            if let Err(e) = fs::rename(&temp, &root) {
                let _ = fs::rename(&old, &root);
                return Err(format!("{}: {e}", root.display()));
            }
            // A link is removed, never followed.
            let _ = if fs::symlink_metadata(&old).is_ok_and(|m| m.is_dir()) { fs::remove_dir_all(&old) } else { fs::remove_file(&old) };
        } else {
            fs::rename(&temp, &root).map_err(|e| format!("{}: {e}", root.display()))?;
        }
        Ok(Installed { name, files, skipped, exists: false })
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp);
    }
    result
}

/// Why a zip over [`MAX_ZIP`] isn't installed, and what to do instead.
pub fn too_big(file_name: &str) -> String {
    format!(
        "{file_name} is bigger than {} GB, too big to install from here. Unzip it yourself and copy the mod's folder into the mods folder (Open mods folder).",
        MAX_ZIP >> 30
    )
}

/// Writes the entries into `temp`, a folder this creates, leaving out the
/// first `strip` parts of each path. Returns how many files it wrote and
/// what it left out.
fn unpack<R: Read + Seek>(zip: &mut ZipArchive<R>, entries: &[Entry], strip: usize, temp: &Path) -> Result<(usize, Vec<String>), String> {
    fs::create_dir(temp).map_err(|e| format!("{}: {e}", temp.display()))?;
    let mut written = 0u64;
    let mut files = 0;
    let mut skipped = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in entries {
        let rel = entry.parts[strip.min(entry.parts.len())..].join("/");
        if rel.is_empty() {
            if !entry.dir {
                skipped.push(format!("{}: beside the mod's folder in the zip, not in it.", entry.parts.join("/")));
            }
            continue;
        }
        if entry.link {
            skipped.push(format!("{rel}: a link, which a mod can't carry. Put the file itself in the zip."));
            continue;
        }
        // The folder was made by this function and nothing in it is a link,
        // so these paths stay inside it.
        let path = temp.join(&rel);
        if entry.dir {
            fs::create_dir_all(&path).map_err(|e| format!("{rel}: {e}"))?;
            continue;
        }
        if !seen.insert(rel.to_lowercase()) {
            skipped.push(format!("{rel}: in the zip twice (or with different capitals); the first one was kept."));
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{rel}: {e}"))?;
        }
        let mut file = zip.by_index(entry.index).map_err(|e| format!("{rel}: {e}"))?;
        let mut out = fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("{rel}: {e}"))?;
        // Counts what actually comes out, not the size the zip claims.
        let left = MAX_BYTES - written;
        let copied = io::copy(&mut (&mut file).take(left + 1), &mut out).map_err(|e| format!("{rel}: {e}"))?;
        if copied > left {
            return Err(format!("The zip unpacks to more than {} GB, too much for a mod.", MAX_BYTES >> 30));
        }
        written += copied;
        files += 1;
    }
    Ok((files, skipped))
}

/// The path's folder and file names, without empty and `.` parts, or
/// `None` when it could point outside the folder it's unpacked into.
/// Windows zips sometimes separate folders with backslashes.
fn safe_parts(raw: &str) -> Option<Vec<String>> {
    if raw.starts_with('/') || raw.starts_with('\\') {
        return None;
    }
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p if p.contains(':') || p.contains('\0') => return None,
            p => parts.push(p.to_owned()),
        }
    }
    Some(parts)
}

/// The mod's folder name, and how many leading path parts to leave out:
/// one when everything is in a single folder that isn't itself one of a
/// mod's folders. Loose files beside that folder, like a README, don't
/// count.
fn mod_name(file_name: &str, entries: &[Entry]) -> (String, usize) {
    let tops: BTreeSet<&str> = entries.iter().filter(|e| e.parts.len() > 1 || e.dir).map(|e| e.parts[0].as_str()).collect();
    if tops.len() == 1 {
        let top = *tops.iter().next().unwrap();
        if !MOD_FOLDERS.contains(&top) {
            return (folder_name(top), 1);
        }
    }
    let stem = file_name.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem = stem.strip_suffix(".zip").or_else(|| stem.strip_suffix(".ZIP")).unwrap_or(stem);
    (folder_name(stem), 0)
}

/// `text` made into a name [`mods::valid_name`] accepts: other characters
/// become `-`.
fn folder_name(text: &str) -> String {
    let name: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' })
        .collect();
    let name = name.trim_start_matches('.').trim_matches('-').to_owned();
    if mods::valid_name(&name) {
        name
    } else {
        "mod".to_owned()
    }
}

/// Decodes a `%`-encoded header value, as the screen's `encodeURIComponent` sends it.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rag-wrapper-modzip-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A zip of `files` (path, content); a content starting with `->` makes a link to the rest.
    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        for (path, content) in files {
            if let Some(target) = content.strip_prefix("->") {
                zip.add_symlink(*path, target, SimpleFileOptions::default()).unwrap();
            } else if path.ends_with('/') {
                zip.add_directory(*path, SimpleFileOptions::default()).unwrap();
            } else {
                zip.start_file(*path, SimpleFileOptions::default()).unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_zipped_mod_folder_takes_its_folder_s_name() {
        let dir = temp_dir("folder");
        let zip = zip_of(&[("eden-rework/", ""), ("eden-rework/npc/a.txt", "npc"), ("eden-rework/db/re/item_db.yml", "db"), ("__MACOSX/eden-rework/._a.txt", "x")]);
        let installed = install(&dir, "eden-rework-v2.zip", &zip, false).unwrap();
        assert_eq!(installed, Installed { name: "eden-rework".into(), files: 2, skipped: vec![], exists: false });
        assert_eq!(fs::read_to_string(dir.join("eden-rework/npc/a.txt")).unwrap(), "npc");
        assert_eq!(fs::read_to_string(dir.join("eden-rework/db/re/item_db.yml")).unwrap(), "db");
        assert!(!dir.join("__MACOSX").exists());
        // Switched off, and no hidden install folder left behind.
        assert!(!dir.join(mods::ENABLED).exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn loose_files_beside_the_mod_s_folder_are_left_out() {
        let dir = temp_dir("readme");
        let zip = zip_of(&[("README.txt", "read me"), ("eden/npc/a.txt", "npc"), ("LICENSE", "l")]);
        let installed = install(&dir, "eden-v2.zip", &zip, false).unwrap();
        assert_eq!(installed.name, "eden");
        assert_eq!(installed.files, 1);
        assert_eq!(installed.skipped.len(), 2, "{:?}", installed.skipped);
        assert!(installed.skipped[0].starts_with("README.txt: "));
        assert_eq!(fs::read_to_string(dir.join("eden/npc/a.txt")).unwrap(), "npc");
        assert!(!dir.join("eden/README.txt").exists());
    }

    #[test]
    fn a_zip_of_the_mod_s_folders_takes_the_zip_s_name() {
        let dir = temp_dir("loose");
        let zip = zip_of(&[("npc/warper.txt", "w"), ("client\\data\\sprite.spr", "s")]);
        let installed = install(&dir, "My Warper (1).zip", &zip, false).unwrap();
        assert_eq!(installed.name, "My-Warper--1");
        assert!(mods::valid_name(&installed.name));
        assert!(dir.join("My-Warper--1/client/data/sprite.spr").is_file());
    }

    #[test]
    fn refuses_paths_out_of_the_mod_s_folder() {
        for bad in ["../evil.txt", "mod/../../evil.txt", "/etc/evil.txt", "C:/evil.txt", "mod\\..\\..\\evil.txt"] {
            let dir = temp_dir("escape");
            let zip = zip_of(&[("mod/npc/a.txt", "a"), (bad, "evil")]);
            let err = install(&dir.join("mods"), "mod.zip", &zip, false).unwrap_err();
            assert!(err.contains("outside the mod's folder"), "{bad}: {err}");
            assert!(!dir.join("evil.txt").exists());
            assert!(!dir.join("mods").exists() || fs::read_dir(dir.join("mods")).unwrap().count() == 0, "{bad}");
        }
    }

    #[test]
    fn leaves_links_out() {
        let dir = temp_dir("links");
        let zip = zip_of(&[("mod/npc/a.txt", "a"), ("mod/client/secret", "->/etc/passwd"), ("mod/npc/up", "->../..")]);
        let installed = install(&dir, "mod.zip", &zip, false).unwrap();
        assert_eq!(installed.files, 1);
        assert_eq!(installed.skipped.len(), 2);
        assert!(fs::symlink_metadata(dir.join("mod/client/secret")).is_err());
        assert!(fs::symlink_metadata(dir.join("mod/npc/up")).is_err());
    }

    #[test]
    fn replaces_an_installed_mod_only_when_asked() {
        let dir = temp_dir("exists");
        fs::create_dir_all(dir.join("mod/npc")).unwrap();
        fs::write(dir.join("mod/npc/mine.txt"), "mine").unwrap();
        fs::write(dir.join(mods::ENABLED), "mod\n").unwrap();
        let zip = zip_of(&[("mod/npc/a.txt", "a")]);
        let asked = install(&dir, "mod.zip", &zip, false).unwrap();
        assert!(asked.exists);
        assert_eq!(fs::read_to_string(dir.join("mod/npc/mine.txt")).unwrap(), "mine");
        assert!(!dir.join("mod/npc/a.txt").exists());

        let replaced = install(&dir, "mod.zip", &zip, true).unwrap();
        assert_eq!(replaced, Installed { name: "mod".into(), files: 1, skipped: vec![], exists: false });
        assert!(!dir.join("mod/npc/mine.txt").exists());
        assert_eq!(fs::read_to_string(dir.join("mod/npc/a.txt")).unwrap(), "a");
        // Still on, and nothing hidden left behind.
        assert_eq!(fs::read_to_string(dir.join(mods::ENABLED)).unwrap(), "mod\n");
        let mut left: Vec<String> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, [mods::ENABLED, "mod"]);
    }

    #[test]
    fn replacing_a_linked_mod_leaves_its_target_alone() {
        let dir = temp_dir("linked");
        let elsewhere = dir.join("elsewhere");
        fs::create_dir_all(elsewhere.join("npc")).unwrap();
        fs::write(elsewhere.join("npc/keep.txt"), "keep").unwrap();
        let mods = dir.join("mods");
        fs::create_dir_all(&mods).unwrap();
        std::os::unix::fs::symlink(&elsewhere, mods.join("mod")).unwrap();
        install(&mods, "mod.zip", &zip_of(&[("mod/npc/a.txt", "a")]), true).unwrap();
        assert_eq!(fs::read_to_string(elsewhere.join("npc/keep.txt")).unwrap(), "keep");
        assert!(fs::symlink_metadata(mods.join("mod")).unwrap().is_dir());
        assert!(fs::symlink_metadata(mods.join(".replaced-mod")).is_err());
    }

    #[test]
    fn refuses_what_isn_t_a_mod() {
        let dir = temp_dir("notmod");
        let err = install(&dir, "pics.zip", &zip_of(&[("pics/a.png", "p")]), false).unwrap_err();
        assert!(err.contains("no npc/, db/ or client/ folder in pics/"), "{err}");
        let err = install(&dir, "junk.zip", b"not a zip", false).unwrap_err();
        assert!(err.contains("isn't a zip file"), "{err}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn a_duplicate_file_keeps_the_first() {
        let dir = temp_dir("dup");
        let zip = zip_of(&[("npc/a.txt", "first"), ("npc/A.txt", "second")]);
        let installed = install(&dir, "dup.zip", &zip, false).unwrap();
        assert_eq!(installed.files, 1);
        assert_eq!(installed.skipped.len(), 1);
        assert_eq!(fs::read_to_string(dir.join("dup/npc/a.txt")).unwrap(), "first");
    }

    #[test]
    fn decodes_the_file_name() {
        assert_eq!(percent_decode("My%20Mod%20%C3%A9.zip"), "My Mod é.zip");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
