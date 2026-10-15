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
//! Files that differ between the game modes, such as translation additions,
//! go in the folder's `Pre-Renewal/` and `Renewal/` folders, laid out the
//! same way. A patch carries the shared files plus the server's current
//! mode's folder, whose files win over shared ones of the same name, so
//! after a mode switch the next patch swaps the players' files.
//!
//! Enabled mods can carry client files too, in a `client/` folder laid out
//! the same way (mode folders included). They're added after the client
//! files folder, in load order: a mod's file replaces an earlier one of the
//! same path, except that Lua tables (`.lub`, `.lua`) are merged entry by
//! entry ([`crate::luamerge`]). Where two of them set something differently
//! the later one is used, and the clash is listed.
//!
//! `patches/published.json` remembers each published file's checksum and
//! the patch history, so the next patch carries only what changed.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::luamerge;
use crate::thor::ThorWriter;

/// Fixed names, shared with the portal (`portal/patcher.go`): the patcher's
/// Play button starts the exe, and GRF patches go into the GRF.
pub const EXE: &str = "rag-wrapper.exe";
pub const GRF: &str = "rag-wrapper.grf";

const PLIST: &str = "plist.txt";
const MANIFEST: &str = "published.json";
const DATA_INI: &str = "DATA.INI";
const DATA_INI_CONTENT: &str = "[Data]\r\n0=rag-wrapper.grf\r\n1=data.grf\r\n";

/// The folders for files of one game mode, by the mode's name.
const MODES: [&str; 2] = ["Pre-Renewal", "Renewal"];

/// The game mode's name, which is also its folder in the client files.
pub fn mode_name(renewal: bool) -> &'static str {
    MODES[usize::from(renewal)]
}

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
    /// The game mode of the latest patch.
    #[serde(default)]
    mode: Option<String>,
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
    /// The server's game mode, whose folder this patch carries.
    pub mode: &'static str,
    /// The mode of the latest patch, when it was another one.
    pub switched_from: Option<String>,
    /// What the next patch would carry.
    pub changes: Changes,
    /// Files that can't be published as they are; publishing waits for them.
    pub problems: Vec<String>,
    /// The enabled mods whose client files the patch carries, in load order.
    pub mods: Vec<String>,
    /// Where the client files folder and mods, or two mods, set something
    /// differently; the later one is used.
    pub clashes: Vec<String>,
    /// Newest first.
    pub history: Vec<Publish>,
}

/// What would be published, and what has been. `mods` are the enabled
/// mods' names and client folders, in load order.
pub fn status(client_files: &Path, mods: &[(String, PathBuf)], patches: &Path, renewal: bool, server_name: &str) -> Result<Status, String> {
    let manifest = Manifest::load(patches)?;
    let (files, layout) = scan(client_files, mods, renewal, server_name)?;
    let mode = mode_name(renewal);
    Ok(Status {
        folder: client_files.display().to_string(),
        mode,
        switched_from: manifest.mode.filter(|m| m != mode),
        changes: diff(&manifest.files, &files),
        problems: layout.problems,
        mods: layout.mods,
        clashes: layout.clashes.into_iter().map(|c| c.text).collect(),
        history: manifest.history.into_iter().rev().collect(),
    })
}

/// Where the client files folder and the enabled mods' client files, or
/// two mods', set something differently, as the next patch would have them.
pub fn clashes(client_files: &Path, mods: &[(String, PathBuf)], renewal: bool) -> Vec<Clash> {
    if mods.is_empty() {
        return Vec::new();
    }
    layout(client_files, mods, renewal).map(|l| l.clashes).unwrap_or_default()
}

/// Builds and publishes a patch of what changed since the last one: up to
/// two THOR files, one for `rag-wrapper.grf` and one for the client folder.
/// Each `.thor` is written before its line is added to `plist.txt`, which is
/// what makes the portal serve it.
pub fn publish(client_files: &Path, mods: &[(String, PathBuf)], patches: &Path, renewal: bool, server_name: &str) -> Result<Publish, String> {
    let mut manifest = Manifest::load(patches)?;
    let (files, layout) = scan(client_files, mods, renewal, server_name)?;
    if !layout.problems.is_empty() {
        return Err(format!("Fix these files first:\n{}", layout.problems.join("\n")));
    }
    let sources = layout.sources;
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
            let content = sources[path.as_str()].read().map_err(|e| format!("{path}: {e}"))?;
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
    manifest.mode = Some(mode_name(renewal).to_owned());
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

/// Where one of the files players get comes from.
enum Source {
    File(PathBuf),
    /// Lua tables merged from several files.
    Merged(Vec<u8>),
}

impl Source {
    fn read(&self) -> io::Result<Vec<u8>> {
        match self {
            Source::File(path) => fs::read(path),
            Source::Merged(bytes) => Ok(bytes.clone()),
        }
    }

    fn state(&self) -> io::Result<FileState> {
        match self {
            Source::File(path) => hash_file(path),
            Source::Merged(bytes) => Ok(state_of(bytes)),
        }
    }
}

/// Something two of the layers set differently, and the mods it names.
#[derive(Debug, Clone, PartialEq)]
pub struct Clash {
    pub text: String,
    pub mods: Vec<String>,
}

/// The files players get in this mode, by their `/`-separated path in the
/// client folder, and where each comes from; what's wrong with any that
/// can't be published; and the clashes between layers.
#[derive(Default)]
struct Layout {
    sources: BTreeMap<String, Source>,
    problems: Vec<String>,
    mods: Vec<String>,
    clashes: Vec<Clash>,
}

/// The layout, and each file's checksum.
fn scan(root: &Path, mods: &[(String, PathBuf)], renewal: bool, server_name: &str) -> Result<(BTreeMap<String, FileState>, Layout), String> {
    let mut layout = layout(root, mods, renewal)?;
    name_custom_items(&mut layout, server_name)?;
    let mut files = BTreeMap::new();
    for (path, source) in &layout.sources {
        files.insert(path.clone(), source.state().map_err(|e| format!("{path}: {e}"))?);
    }
    Ok((files, layout))
}

/// The English translation's file for the server's own items, which it
/// loads after its display settings.
const ITEM_INFO_C: &str = "systemen/iteminfo_c.lua";

/// Sets the translation's name for the server's custom items to the server's
/// name, so players see it after them ("Kit Potion [MyServer]") rather than
/// the translation's "ExampleRO". Each player's own display settings still
/// decide whether and where it shows.
fn name_custom_items(layout: &mut Layout, server_name: &str) -> Result<(), String> {
    let Some((path, source)) = layout.sources.iter_mut().find(|(p, _)| p.to_lowercase() == ITEM_INFO_C) else {
        return Ok(());
    };
    let content = source.read().map_err(|e| format!("{path}: {e}"))?;
    let mut text = custom_server_line(server_name);
    text.extend_from_slice(&content);
    *source = Source::Merged(text);
    Ok(())
}

/// `CServerName = "..."` as a line of Lua. Non-ASCII names are written as
/// byte escapes in the client's code page (CP949), whose second bytes can
/// look like a backslash.
fn custom_server_line(server_name: &str) -> Vec<u8> {
    let name: String = server_name.trim().chars().filter(|c| !c.is_control()).collect();
    let (bytes, _, _) = encoding_rs::EUC_KR.encode(&name);
    let mut line = b"CServerName = \"".to_vec();
    for &b in bytes.iter() {
        match b {
            b'"' | b'\\' => line.extend_from_slice(&[b'\\', b]),
            0x80.. => line.extend_from_slice(format!("\\{b}").as_bytes()),
            _ => line.push(b),
        }
    }
    line.extend_from_slice(b"\" -- the server's name, from the desktop app's Settings\n");
    line
}

/// The label a layer's files go by in clashes.
const CLIENT_FILES: &str = "the client files folder";

fn layout(root: &Path, mods: &[(String, PathBuf)], renewal: bool) -> Result<Layout, String> {
    let mut layout = Layout::default();
    // Each file's layers, by its path as the first layer spells it.
    let mut layers: BTreeMap<String, Vec<(String, PathBuf)>> = BTreeMap::new();
    let mut spelling: BTreeMap<String, String> = BTreeMap::new();
    let roots = std::iter::once((None, root.to_path_buf())).chain(mods.iter().map(|(n, p)| (Some(n.as_str()), p.clone())));
    for (name, dir) in roots {
        if !dir.is_dir() {
            continue;
        }
        let mut files = BTreeMap::new();
        let mut problems = Vec::new();
        collect(&dir, renewal, &mut files, &mut problems)?;
        let (label, prefix) = match name {
            None => (CLIENT_FILES.to_owned(), String::new()),
            Some(n) => (format!("mods/{n}"), format!("mods/{n}/client/")),
        };
        if name.is_some() {
            if files.is_empty() {
                continue;
            }
            layout.mods.push(name.unwrap_or_default().to_owned());
            // The patcher's Play button starts the one patched client.
            if let Some(exe) = files.keys().find(|p| p.eq_ignore_ascii_case(EXE)).cloned() {
                files.remove(&exe);
                problems.push(format!("{exe}: only the client files folder carries the patched client. Delete this one."));
            }
        }
        layout.problems.extend(problems.into_iter().map(|p| format!("{prefix}{p}")));
        for (path, source) in files {
            // Windows doesn't tell `System/a.lub` from `system/A.lub`.
            let path = spelling.entry(path.to_lowercase()).or_insert(path).clone();
            layers.entry(path).or_default().push((label.clone(), source));
        }
    }
    for (path, mut found) in layers {
        let source = if found.len() == 1 {
            Source::File(found.remove(0).1)
        } else {
            resolve(&path, &found, &mut layout.clashes)?
        };
        layout.sources.insert(path, source);
    }
    // Once anything is published, the portal sends players to the
    // patcher, whose Play button starts the exe.
    if !layout.sources.is_empty() && !layout.sources.keys().any(|p| p.eq_ignore_ascii_case(EXE)) {
        layout.problems.push(format!("{EXE} is missing: every patch needs the patched client, which the patcher's Play button starts."));
    }
    layout.problems.sort();
    Ok(layout)
}

/// A file several layers have, from the first to the last: Lua tables are
/// merged, and anything else is the last layer's.
fn resolve(path: &str, found: &[(String, PathBuf)], clashes: &mut Vec<Clash>) -> Result<Source, String> {
    let read = |p: &Path| fs::read(p).map_err(|e| format!("{path}: {e}"));
    let lower = path.to_lowercase();
    let mut clash = |text: String, labels: &[&str]| {
        let mods = labels.iter().filter_map(|l| l.strip_prefix("mods/")).map(str::to_owned).collect();
        clashes.push(Clash { text, mods });
    };
    if !(lower.ends_with(".lub") || lower.ends_with(".lua")) {
        let (last, earlier) = found.split_last().expect("several layers");
        let content = read(&last.1)?;
        for (label, p) in earlier {
            if read(p)? != content {
                clash(format!("{path}: {label} and {} both have this file; the one from {} is used (later in load order)", last.0, last.0), &[label, &last.0]);
            }
        }
        return Ok(Source::File(last.1.clone()));
    }
    let mut text = read(&found[0].1)?;
    let mut owners: BTreeMap<(String, String), &str> = luamerge::keys(&text).into_iter().map(|k| (k, found[0].0.as_str())).collect();
    // The layers the text has come from since a whole file last replaced it.
    let mut from: Vec<&str> = vec![&found[0].0];
    for (label, p) in &found[1..] {
        let add = read(p)?;
        let merged = luamerge::unmergeable(&text).is_none().then(|| luamerge::merge(&text, &add)).flatten();
        let Some((next, outcomes)) = merged else {
            let reason = luamerge::unmergeable(&text).or(luamerge::unmergeable(&add)).unwrap_or("no tables could be read from it");
            if add != text {
                let earlier = from.join(" and ");
                let mut labels = from.clone();
                labels.push(label);
                clash(format!("{path}: {earlier} and {label} both have this file, and it can't be merged ({reason}); the one from {label} is used (later in load order)"), &labels);
            }
            text = add;
            owners = luamerge::keys(&text).into_iter().map(|k| (k, label.as_str())).collect();
            from = vec![label];
            continue;
        };
        for (table, key, outcome) in outcomes {
            let owner = owners.insert((table.clone(), key.clone()), label);
            if let (luamerge::Outcome::Replaced, Some(owner)) = (outcome, owner) {
                clash(format!("{path}, {table}[{key}]: {owner} and {label} set it differently; the one from {label} is used (later in load order)"), &[owner, label]);
            }
        }
        text = next;
        from.push(label);
    }
    Ok(Source::Merged(text))
}

/// Adds the files players get in this mode from one layer's folder to
/// `files`: the shared ones plus the mode's folder, whose files replace
/// shared ones of the same path.
fn collect(root: &Path, renewal: bool, files: &mut BTreeMap<String, PathBuf>, problems: &mut Vec<String>) -> Result<(), String> {
    walk(root, true, files, problems)?;
    // A mode folder's name may differ in case, as Windows folders often do.
    let entries = fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_str().is_some_and(|n| n.eq_ignore_ascii_case(mode_name(renewal))) && entry.path().is_dir() {
            let mut mode_problems = Vec::new();
            walk(&entry.path(), false, files, &mut mode_problems)?;
            let folder = name.to_string_lossy();
            problems.extend(mode_problems.into_iter().map(|p| format!("{folder}/{p}")));
        }
    }
    Ok(())
}

/// Adds the files under `root` to `files`, replacing any of the same path.
/// `skip_modes` leaves out the mode folders at the top.
fn walk(root: &Path, skip_modes: bool, files: &mut BTreeMap<String, PathBuf>, problems: &mut Vec<String>) -> Result<(), String> {
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
                let mode_folder = MODES.iter().any(|m| m.eq_ignore_ascii_case(name));
                if skip_modes && rel.as_os_str().is_empty() && mode_folder {
                    continue;
                }
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
            files.insert(path, entry.path());
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
            publish(&self.files(), &[], &self.patches(), false, "rAthena")
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
        let s = status(&d.files(), &[], &d.patches(), false, "rAthena").unwrap();
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
        let s = status(&d.files(), &[], &d.patches(), false, "rAthena").unwrap();
        assert_eq!(s.problems.len(), 5, "{:?}", s.problems);
        assert!(s.problems.iter().any(|p| p.starts_with(&format!("{EXE} is missing"))));
        assert_eq!(s.changes.added, ["data/ok.txt"]);
        assert!(d.publish().unwrap_err().starts_with("Fix these files first"));
        assert!(!d.patches().join(PLIST).exists());
    }

    #[test]
    fn each_mode_gets_its_own_files() {
        let d = Dirs::new("modes");
        d.put(EXE, b"MZ");
        d.put("System/shared.lub", b"shared");
        d.put("System/tr.lub", b"shared tr");
        d.put("Pre-Renewal/System/tr.lub", b"pre tr");
        d.put("renewal/System/tr.lub", b"re tr");
        d.put("renewal/data/re.txt", b"re only");
        d.publish().unwrap();
        let a = d.thor("0001_client.thor");
        assert_eq!(a.files[&b"System\\tr.lub"[..]].as_deref(), Some(&b"pre tr"[..]));
        assert!(a.files.contains_key(&b"System\\shared.lub"[..]));
        assert!(!a.files.keys().any(|k| k.starts_with(b"Pre-Renewal") || k.starts_with(b"renewal")));

        let s = status(&d.files(), &[], &d.patches(), true, "rAthena").unwrap();
        assert_eq!((s.mode, s.switched_from.as_deref()), ("Renewal", Some("Pre-Renewal")));
        assert_eq!(s.changes.changed, ["System/tr.lub"]);
        assert_eq!(s.changes.added, ["data/re.txt"]);
        let p = publish(&d.files(), &[], &d.patches(), true, "rAthena").unwrap();
        assert_eq!(p.patches, ["0002_data.thor", "0003_client.thor"]);
        assert_eq!(d.thor("0003_client.thor").files[&b"System\\tr.lub"[..]].as_deref(), Some(&b"re tr"[..]));
        assert_eq!(status(&d.files(), &[], &d.patches(), true, "rAthena").unwrap().switched_from, None);

        // Switching back takes the Renewal-only file away again.
        let s = status(&d.files(), &[], &d.patches(), false, "rAthena").unwrap();
        assert_eq!(s.changes.removed, ["data/re.txt"]);
        assert_eq!(s.changes.changed, ["System/tr.lub"]);
    }

    #[test]
    fn mods_client_files_are_layered_and_lua_tables_merged() {
        let d = Dirs::new("mods");
        d.put(EXE, b"MZ");
        d.put("System/itemInfo.lua", b"tbl = {\n\t[501] = { name = \"Red Potion\" },\n}\nfunction main() return true end\n");
        d.put("System/shared.txt", b"from the folder");
        let mods = d.root.join("mods");
        let put = |path: &str, content: &[u8]| {
            let p = mods.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        };
        put("a/client/System/iteminfo.lua", b"tbl = { [501] = { name = \"Red\" }, [30000] = { name = \"Coin\" } }");
        put("a/client/data/sprite/a.spr", b"a");
        put("a/client/Renewal/System/re.lub", b"re = {}");
        put("b/client/System/itemInfo.lua", b"tbl = { [30000] = { name = \"Eden Coin\" } }");
        put("b/client/System/shared.txt", b"from b");
        put("b/client/System/compiled.lub", b"\x1bLuaQ b");
        put("a/client/System/compiled.lub", b"\x1bLuaQ a");
        put("b/client/rag-wrapper.exe", b"MZ b");
        let layers = vec![("a".to_owned(), mods.join("a/client")), ("b".to_owned(), mods.join("b/client")), ("c".to_owned(), mods.join("c/client"))];

        let s = status(&d.files(), &layers, &d.patches(), false, "rAthena").unwrap();
        assert_eq!(s.mods, ["a", "b"]);
        assert_eq!(s.problems, ["mods/b/client/rag-wrapper.exe: only the client files folder carries the patched client. Delete this one."]);
        assert_eq!(
            s.clashes,
            [
                "System/compiled.lub: mods/a and mods/b both have this file, and it can't be merged (it's compiled Lua); the one from mods/b is used (later in load order)",
                "System/itemInfo.lua, tbl[501]: the client files folder and mods/a set it differently; the one from mods/a is used (later in load order)",
                "System/itemInfo.lua, tbl[30000]: mods/a and mods/b set it differently; the one from mods/b is used (later in load order)",
                "System/shared.txt: the client files folder and mods/b both have this file; the one from mods/b is used (later in load order)",
            ]
        );
        let found = clashes(&d.files(), &layers, false);
        assert_eq!(found[1].mods, ["a"]);
        assert_eq!(found[2].mods, ["a", "b"]);

        fs::remove_file(mods.join("b/client/rag-wrapper.exe")).unwrap();
        publish(&d.files(), &layers, &d.patches(), false, "rAthena").unwrap();
        assert!(d.thor("0001_data.thor").files.contains_key(&b"data\\sprite\\a.spr"[..]));
        let client = d.thor("0002_client.thor");
        let info = String::from_utf8(client.files[&b"System\\itemInfo.lua"[..]].clone().unwrap()).unwrap();
        assert_eq!(info, "tbl = {\n\t[501] = { name = \"Red\" },\n\t[30000] = { name = \"Eden Coin\" },\n}\nfunction main() return true end\n");
        assert_eq!(client.files[&b"System\\shared.txt"[..]].as_deref(), Some(&b"from b"[..]));
        assert_eq!(client.files[&b"System\\compiled.lub"[..]].as_deref(), Some(&b"\x1bLuaQ b"[..]));
        assert!(!client.files.contains_key(&b"System\\re.lub"[..]), "another mode's file");

        // Switching a mod off takes its files away with the next patch.
        let s = status(&d.files(), &layers[1..], &d.patches(), false, "rAthena").unwrap();
        assert_eq!(s.changes.removed, ["data/sprite/a.spr"]);
        assert_eq!(s.changes.changed, ["System/itemInfo.lua"]);
        assert!(status(&d.files(), &[], &d.patches(), false, "rAthena").unwrap().clashes.is_empty());
    }

    #[test]
    fn custom_items_carry_the_server_s_name() {
        let d = Dirs::new("cservername");
        d.put(EXE, b"exe");
        d.put("SystemEN/itemInfo_C.lua", b"tbl_custom = {\n}\n");
        d.put("System/other.lua", b"x = {}\n");
        let s = status(&d.files(), &[], &d.patches(), false, "rAthena").unwrap();
        assert_eq!(s.changes.added.len(), 3);
        publish(&d.files(), &[], &d.patches(), false, " My \"RO\" ").unwrap();
        let client = d.thor("0001_client.thor");
        let info = client.files[&b"SystemEN\\itemInfo_C.lua"[..]].clone().unwrap();
        assert_eq!(info, b"CServerName = \"My \\\"RO\\\"\" -- the server's name, from the desktop app's Settings\ntbl_custom = {\n}\n");
        assert_eq!(client.files[&b"System\\other.lua"[..]].clone().unwrap(), b"x = {}\n");
        // A new name is a change to publish.
        let s = status(&d.files(), &[], &d.patches(), false, "Other").unwrap();
        assert_eq!(s.changes.changed, ["SystemEN/itemInfo_C.lua"]);
        assert_eq!(custom_server_line("이")[..23], *b"CServerName = \"\\192\\204");
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
