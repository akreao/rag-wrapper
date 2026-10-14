//! "Publish client patch": turns what changed in the client-files folder
//! since the last patch into THOR patches in the stack's `patches/` folder,
//! which the portal serves to the players' patcher.
//!
//! The client-files folder mirrors the game client's folder. Files under
//! `data/` are packed into `rag-wrapper.grf`; everything else, such as the
//! patched `rag-wrapper.exe` or `System/` files, is copied into the client
//! folder as is. Once anything has gone into the GRF, every patch also
//! writes the client's `DATA.INI` so the client reads `rag-wrapper.grf`
//! before `data.grf`; rewriting it each time puts it back if a player's
//! translation install replaced it.
//!
//! `patches/published.json` remembers each published file's checksum and
//! the patch history, so the next patch carries only what changed.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::thor::ThorWriter;

/// Fixed names, shared with the portal (`portal/patcher.go`): the patcher's
/// Play button starts the exe, and GRF patches go into the GRF.
pub const EXE: &str = "rag-wrapper.exe";
pub const GRF: &str = "rag-wrapper.grf";

const PLIST: &str = "plist.txt";
const MANIFEST: &str = "published.json";
const DATA_INI: &str = "DATA.INI";
const DATA_INI_CONTENT: &str = "[Data]\r\n0=rag-wrapper.grf\r\n1=data.grf\r\n";

/// Files the file manager leaves behind, never published.
const IGNORED: [&str; 2] = ["thumbs.db", "desktop.ini"];

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct FileState {
    size: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Publish {
    /// Local time, "YYYY-MM-DD HH:MM".
    pub published: String,
    /// The patch files, in the order they apply.
    pub patches: Vec<String>,
    #[serde(flatten)]
    pub changes: Changes,
}

#[derive(Serialize, Deserialize, Default)]
struct Manifest {
    /// Every file the players have, by its path in the client-files folder.
    files: BTreeMap<String, FileState>,
    /// Whether anything has been published into `rag-wrapper.grf`.
    grf: bool,
    /// Oldest first.
    history: Vec<Publish>,
}

impl Manifest {
    fn load(patches: &Path) -> Result<Self, String> {
        match fs::read(patches.join(MANIFEST)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{MANIFEST} in the patches folder is damaged: {e}")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Serialize)]
pub struct Status {
    pub folder: String,
    /// What the next patch would carry.
    pub changes: Changes,
    /// Files that can't be published as they are; publishing waits for them.
    pub problems: Vec<String>,
    /// Newest first.
    pub history: Vec<Publish>,
}

/// What would be published, and what has been.
pub fn status(client_files: &Path, patches: &Path) -> Result<Status, String> {
    let manifest = Manifest::load(patches)?;
    let (files, problems) = scan(client_files)?;
    Ok(Status {
        folder: client_files.display().to_string(),
        changes: diff(&manifest.files, &files),
        problems,
        history: manifest.history.into_iter().rev().collect(),
    })
}

/// Builds and publishes a patch of what changed since the last one: up to
/// two THOR files, one for `rag-wrapper.grf` and one for the client folder.
/// Each `.thor` is written before its line is added to `plist.txt`, which is
/// what makes the portal serve it.
pub fn publish(client_files: &Path, patches: &Path) -> Result<Publish, String> {
    let mut manifest = Manifest::load(patches)?;
    let (files, problems) = scan(client_files)?;
    if !problems.is_empty() {
        return Err(format!("Fix these files first:\n{}", problems.join("\n")));
    }
    let changes = diff(&manifest.files, &files);
    if changes.is_empty() {
        return Err("Nothing has changed since the last patch.".into());
    }
    fs::create_dir_all(patches).map_err(|e| e.to_string())?;
    let plist = fs::read_to_string(patches.join(PLIST)).unwrap_or_default();
    let mut number = next_number(&plist);
    let mut built = Vec::new();

    let updated: Vec<&String> = changes.added.iter().chain(&changes.changed).collect();
    for grf_part in [true, false] {
        let update: Vec<&String> = updated.iter().copied().filter(|p| in_grf(p) == grf_part).collect();
        let remove: Vec<&String> = changes.removed.iter().filter(|p| in_grf(p) == grf_part).collect();
        let data_ini = !grf_part && manifest.grf;
        if update.is_empty() && remove.is_empty() && !data_ini {
            continue;
        }
        let name = format!("{number:04}_{}.thor", if grf_part { "data" } else { "client" });
        let tmp = patches.join(format!(".{name}.tmp"));
        let out = BufWriter::new(File::create(&tmp).map_err(|e| e.to_string())?);
        let mut thor = ThorWriter::new(out, grf_part.then_some(GRF.as_bytes())).map_err(|e| e.to_string())?;
        for path in update {
            let content = fs::read(client_files.join(path)).map_err(|e| format!("{path}: {e}"))?;
            thor.add(&encode(path)?, &content).map_err(|e| format!("{path}: {e}"))?;
            manifest.files.insert(path.clone(), state_of(&content));
        }
        for path in remove {
            thor.remove(&encode(path)?);
            manifest.files.remove(path);
        }
        if data_ini {
            thor.add(DATA_INI.as_bytes(), DATA_INI_CONTENT.as_bytes()).map_err(|e| e.to_string())?;
        }
        thor.finish().map_err(|e| e.to_string())?;
        fs::rename(&tmp, patches.join(&name)).map_err(|e| e.to_string())?;
        manifest.grf |= grf_part;
        built.push((number, name));
        number += 1;
    }

    let mut list = plist;
    if !list.is_empty() && !list.ends_with('\n') {
        list.push('\n');
    }
    for (n, name) in &built {
        list.push_str(&format!("{n} {name}\n"));
    }
    let publish = Publish {
        published: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        patches: built.into_iter().map(|(_, name)| name).collect(),
        changes,
    };
    manifest.history.push(publish.clone());
    write_atomic(&patches.join(PLIST), list.as_bytes())?;
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    write_atomic(&patches.join(MANIFEST), &json)?;
    Ok(publish)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// One past the highest number in `plist.txt`. Numbers only go up: each
/// player's patcher remembers the last one it applied.
fn next_number(plist: &str) -> u32 {
    plist
        .lines()
        .filter_map(|l| l.split_whitespace().next()?.parse::<u32>().ok())
        .max()
        .map_or(1, |n| n + 1)
}

fn diff(published: &BTreeMap<String, FileState>, now: &BTreeMap<String, FileState>) -> Changes {
    let mut c = Changes::default();
    for (path, state) in now {
        match published.get(path) {
            None => c.added.push(path.clone()),
            Some(old) if old != state => c.changed.push(path.clone()),
            _ => {}
        }
    }
    c.removed = published.keys().filter(|p| !now.contains_key(*p)).cloned().collect();
    c
}

fn state_of(content: &[u8]) -> FileState {
    FileState {
        size: content.len() as u64,
        sha256: format!("{:x}", Sha256::digest(content)),
    }
}

fn hash_file(path: &Path) -> io::Result<FileState> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    let mut size = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok(FileState {
        size,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

/// Every file in the client-files folder, by its `/`-separated path, and
/// what's wrong with any that can't be published.
fn scan(root: &Path) -> Result<(BTreeMap<String, FileState>, Vec<String>), String> {
    let mut files = BTreeMap::new();
    let mut problems = Vec::new();
    if !root.exists() {
        return Ok((files, problems));
    }
    walk(root, &mut files, &mut problems)?;
    // Once anything is published, the portal sends players to the
    // patcher, whose Play button starts the exe.
    if !files.is_empty() && !files.keys().any(|p| p.eq_ignore_ascii_case(EXE)) {
        problems.push(format!("{EXE} is missing: every patch needs the patched client, which the patcher's Play button starts."));
    }
    problems.sort();
    Ok((files, problems))
}

fn walk(root: &Path, files: &mut BTreeMap<String, FileState>, problems: &mut Vec<String>) -> Result<(), String> {
    let mut dirs = vec![PathBuf::new()];
    while let Some(rel) = dirs.pop() {
        let entries = fs::read_dir(root.join(&rel)).map_err(|e| format!("{}: {e}", root.join(&rel).display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                problems.push(format!("{}: the name isn't valid text", rel.join(&name).display()));
                continue;
            };
            if name.starts_with('.') || IGNORED.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let path = rel.join(name);
            // Follows a link to a file, but not to a folder, which could loop.
            let meta = fs::metadata(entry.path()).map_err(|e| format!("{}: {e}", path.display()))?;
            if meta.is_dir() {
                if !entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                    dirs.push(path);
                }
                continue;
            }
            let path = path.to_string_lossy().into_owned();
            if let Err(problem) = check(&path, meta.len()) {
                problems.push(format!("{path}: {problem}"));
                continue;
            }
            let state = hash_file(&entry.path()).map_err(|e| format!("{path}: {e}"))?;
            files.insert(path, state);
        }
    }
    Ok(())
}

/// Whether a file goes into `rag-wrapper.grf` rather than the client folder.
fn in_grf(path: &str) -> bool {
    path.split_once('/').is_some_and(|(top, _)| top.eq_ignore_ascii_case("data"))
}

/// Why a file can't be handed out as it is, if it can't.
fn check(path: &str, size: u64) -> Result<(), String> {
    if !path.contains('/') {
        let lower = path.to_lowercase();
        if lower == DATA_INI.to_lowercase() {
            return Err(format!("the app writes {DATA_INI} itself, listing {GRF} above data.grf. Delete this one."));
        }
        if lower.ends_with(".grf") {
            return Err(format!("put the files from a GRF under data/ instead; they're packed into {GRF} for the players."));
        }
        if lower.ends_with(".exe") && lower != EXE {
            return Err(format!("name the patched client {EXE}. No other programs are handed out."));
        }
    }
    if size > u64::from(u32::MAX) {
        return Err("a file in a patch can't be larger than 4 GB.".into());
    }
    encode(path).map(|_| ())
}

/// The path as the client spells it: `\` separators, and outside ASCII
/// the client's Korean code page (CP949). Names copied out of a GRF on
/// Windows often show that code page as Latin letters (`À¯ÀúÀÎÅÍÆäÀÌ½º` for
/// `유저인터페이스`); those are turned back into the same bytes.
fn encode(path: &str) -> Result<Vec<u8>, String> {
    let grf = in_grf(path);
    let path = if grf { format!("data{}", &path[4..]) } else { path.to_owned() };
    let path = path.replace('/', "\\");
    let bytes = if path.is_ascii() {
        path.into_bytes()
    } else if !grf {
        return Err("outside data/, file and folder names must be plain English letters, digits and punctuation.".into());
    } else {
        let (latin, _, latin_bad) = encoding_rs::WINDOWS_1252.encode(&path);
        let (korean, _, korean_bad) = encoding_rs::EUC_KR.encode(&path);
        match (latin_bad, korean_bad) {
            (false, _) => latin.into_owned(),
            (true, false) => korean.into_owned(),
            (true, true) => return Err("the name has characters the game client can't read.".into()),
        }
    };
    if bytes.len() > 255 {
        return Err("the path is longer than the 255 bytes a patch allows.".into());
    }
    Ok(bytes)
}

/// Creates the client-files folder, so it can be opened before the first patch.
pub fn ensure_folder(client_files: &Path) -> io::Result<()> {
    fs::create_dir_all(client_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thor::read;

    struct Dirs {
        root: PathBuf,
    }

    impl Dirs {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("rag-wrapper-clientpatch-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("client-files")).unwrap();
            Self { root }
        }
        fn files(&self) -> PathBuf {
            self.root.join("client-files")
        }
        fn patches(&self) -> PathBuf {
            self.root.join("patches")
        }
        fn put(&self, path: &str, content: &[u8]) {
            let p = self.files().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        fn thor(&self, name: &str) -> read::Archive {
            read::archive(&fs::read(self.patches().join(name)).unwrap())
        }
        fn publish(&self) -> Result<Publish, String> {
            publish(&self.files(), &self.patches())
        }
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn first_patch_carries_the_exe() {
        let d = Dirs::new("exe");
        d.put(EXE, b"MZ patched");
        let p = d.publish().unwrap();
        assert_eq!(p.patches, ["0001_client.thor"]);
        assert_eq!(p.changes.added, [EXE]);
        assert_eq!(fs::read_to_string(d.patches().join(PLIST)).unwrap(), "1 0001_client.thor\n");
        let a = d.thor("0001_client.thor");
        assert!(!a.grf_merging);
        assert_eq!(a.files[EXE.as_bytes()].as_deref(), Some(&b"MZ patched"[..]));
        // No GRF yet, so DATA.INI is left alone.
        assert!(!a.files.contains_key(DATA_INI.as_bytes()));
        assert_eq!(d.publish().unwrap_err(), "Nothing has changed since the last patch.");

        // Players can't lose the exe the Play button starts.
        fs::remove_file(d.files().join(EXE)).unwrap();
        d.put("data/a.txt", b"a");
        assert!(d.publish().unwrap_err().contains("is missing"));
    }

    #[test]
    fn data_goes_into_the_grf_and_data_ini_follows() {
        let d = Dirs::new("grf");
        d.put(EXE, b"MZ");
        d.publish().unwrap();
        d.put("data/sprite/new.spr", b"spr");
        d.put("System/custom.lub", b"lub");
        let p = d.publish().unwrap();
        assert_eq!(p.patches, ["0002_data.thor", "0003_client.thor"]);

        let grf = d.thor("0002_data.thor");
        assert!(grf.grf_merging);
        assert_eq!(grf.grf, GRF.as_bytes());
        assert!(grf.files.contains_key(&b"data\\sprite\\new.spr"[..]));
        assert!(!grf.files.contains_key(&b"System\\custom.lub"[..]));

        let client = d.thor("0003_client.thor");
        assert!(client.files.contains_key(&b"System\\custom.lub"[..]));
        assert_eq!(client.files[DATA_INI.as_bytes()].as_deref(), Some(DATA_INI_CONTENT.as_bytes()));
        assert!(!client.files.contains_key(EXE.as_bytes()), "unchanged files aren't sent again");

        // A later GRF-only change still rewrites DATA.INI.
        d.put("data/sprite/new.spr", b"spr v2");
        let p = d.publish().unwrap();
        assert_eq!(p.changes.changed, ["data/sprite/new.spr"]);
        assert_eq!(p.patches, ["0004_data.thor", "0005_client.thor"]);
        assert!(d.thor("0005_client.thor").files.contains_key(DATA_INI.as_bytes()));
    }

    #[test]
    fn deleted_files_are_removed_from_the_players() {
        let d = Dirs::new("removed");
        d.put(EXE, b"MZ");
        d.put("data/a.txt", b"a");
        d.put("System/b.lub", b"b");
        d.publish().unwrap();
        fs::remove_dir_all(d.files().join("data")).unwrap();
        fs::remove_dir_all(d.files().join("System")).unwrap();
        let p = d.publish().unwrap();
        assert_eq!(p.changes.removed, ["System/b.lub", "data/a.txt"]);
        assert_eq!(d.thor("0003_data.thor").files[&b"data\\a.txt"[..]], None);
        assert_eq!(d.thor("0004_client.thor").files[&b"System\\b.lub"[..]], None);
        let s = status(&d.files(), &d.patches()).unwrap();
        assert!(s.changes.is_empty());
        assert_eq!(s.history.len(), 2);
        assert_eq!(s.history[0].patches, ["0003_data.thor", "0004_client.thor"]);
    }

    #[test]
    fn numbering_continues_after_hand_made_patches() {
        let d = Dirs::new("numbers");
        fs::create_dir_all(d.patches()).unwrap();
        fs::write(d.patches().join(PLIST), "1 a.thor\n7 b.thor").unwrap();
        d.put(EXE, b"MZ");
        d.publish().unwrap();
        assert_eq!(fs::read_to_string(d.patches().join(PLIST)).unwrap(), "1 a.thor\n7 b.thor\n8 0008_client.thor\n");
    }

    #[test]
    fn problems_block_publishing() {
        let d = Dirs::new("problems");
        d.put("Ragexe.exe", b"MZ");
        d.put("data.grf", b"");
        d.put("Data.ini", b"");
        d.put("System/한글.lub", b"");
        d.put(".hidden", b"");
        d.put("data/Thumbs.db", b"");
        d.put("data/ok.txt", b"");
        let s = status(&d.files(), &d.patches()).unwrap();
        assert_eq!(s.problems.len(), 5, "{:?}", s.problems);
        assert!(s.problems.iter().any(|p| p.starts_with(&format!("{EXE} is missing"))));
        assert_eq!(s.changes.added, ["data/ok.txt"]);
        assert!(d.publish().unwrap_err().starts_with("Fix these files first"));
        assert!(!d.patches().join(PLIST).exists());
    }

    #[test]
    fn korean_names_use_the_clients_code_page() {
        // 유저인터페이스 ("user interface"), the usual texture folder.
        let cp949 = b"\xc0\xaf\xc0\xfa\xc0\xce\xc5\xcd\xc6\xe4\xc0\xcc\xbd\xba";
        let mut want = b"data\\texture\\".to_vec();
        want.extend_from_slice(cp949);
        want.extend_from_slice(b"\\a.bmp");
        assert_eq!(encode("data/texture/유저인터페이스/a.bmp").unwrap(), want);
        assert_eq!(encode("Data/texture/À¯ÀúÀÎÅÍÆäÀÌ½º/a.bmp").unwrap(), want);
        assert_eq!(encode("rag-wrapper.exe").unwrap(), b"rag-wrapper.exe");
        assert!(encode("data/\u{1F600}/a.bmp").is_err());
    }
}
