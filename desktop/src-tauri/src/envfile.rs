//! Reads and edits the stack's `.env`, keeping its comments and order.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Held across every load, change and save of `.env`, so a UI command and
/// a running task can't each save over the other's change.
static EDITING: Mutex<()> = Mutex::new(());

/// Loads the `.env` at `path`, lets `change` edit it and saves it, with no
/// other edit in between. Nothing is saved when `change` fails.
pub fn edit<T>(path: &Path, change: impl FnOnce(&mut EnvFile) -> Result<T, String>) -> Result<T, String> {
    let _held = EDITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut env = EnvFile::load(path).map_err(|e| e.to_string())?;
    let out = change(&mut env)?;
    env.save().map_err(|e| e.to_string())?;
    Ok(out)
}

pub struct EnvFile {
    path: PathBuf,
    lines: Vec<String>,
}

impl EnvFile {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = fs::read_to_string(path)?;
        Ok(Self {
            path: path.to_owned(),
            lines: text.lines().map(str::to_owned).collect(),
        })
    }

    /// Reads settings from text, such as the bundled `.env.example`. Not for saving.
    pub fn parse(text: &str) -> Self {
        Self {
            path: PathBuf::new(),
            lines: text.lines().map(str::to_owned).collect(),
        }
    }

    /// The key's value as Compose reads it: from the last line that sets
    /// it, without quotes or a ` #` comment after an unquoted value.
    /// The same settings, saved to `path`.
    pub fn at(self, path: &Path) -> Self {
        Self { path: path.to_owned(), ..self }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.lines.iter().rev().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| value(v))
        })
    }

    /// Replaces the key's value, or appends `KEY=value` if it isn't there.
    /// Of a key set more than once, the last line is the one Compose uses,
    /// so that's the one replaced.
    pub fn set(&mut self, key: &str, value: &str) {
        let entry = format!("{key}={value}");
        match self.lines.iter_mut().rev().find(|line| matches!(line.split_once('='), Some((k, _)) if k.trim() == key)) {
            Some(line) => *line = entry,
            None => self.lines.push(entry),
        }
    }

    /// Writes the file readable by its owner only, as it holds passwords.
    pub fn save(&self) -> std::io::Result<()> {
        let mut text = self.lines.join("\n");
        text.push('\n');
        write_private(&self.path, &text)
    }
}

/// Writes `text` to `path` readable by its owner only. It goes to a
/// temporary file that replaces `path` once it's on disk, so a full disk or
/// a crash can't leave the file empty or half written.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    let written = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut file| {
            // A file left from an earlier try keeps its mode.
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// A value as Compose reads it: quotes removed, or for an unquoted value,
/// a comment after ` #` and the spaces around it.
fn value(raw: &str) -> String {
    let v = raw.trim();
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q).and_then(|s| s.split_once(q)).map(|(inner, _)| inner) {
            return inner.to_owned();
        }
    }
    let v = match v.find(" #").or_else(|| v.find("\t#")) {
        Some(i) => &v[..i],
        None => v,
    };
    v.trim().to_owned()
}

/// A random password of letters and digits, which every rAthena config and
/// SQL statement the stack writes accepts as-is.
pub fn random_password(len: usize) -> std::io::Result<String> {
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut bytes = vec![0u8; len];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes
        .iter()
        .map(|b| CHARS[*b as usize % CHARS.len()] as char)
        .collect())
}

/// The rule `entrypoint.sh` enforces on INTER_* and ADMIN_*, plus rAthena's
/// 4 to 23 character account limits.
pub fn check_account_field(what: &str, value: &str) -> Result<(), String> {
    if value.len() < 4 || value.len() > 23 {
        return Err(format!("{what} must be 4 to 23 characters long."));
    }
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{what} may only use letters, digits and _."));
    }
    Ok(())
}

/// A client date as rAthena's `--enable-packetver` takes it: YYYYMMDD.
pub fn check_packetver(value: &str) -> Result<(), String> {
    let valid = value.len() == 8
        && value.bytes().all(|b| b.is_ascii_digit())
        && ("20000101".."20991232").contains(&value)
        && ("01"..="12").contains(&&value[4..6])
        && ("01"..="31").contains(&&value[6..8]);
    if valid {
        Ok(())
    } else {
        Err("The client date must be 8 digits, YYYYMMDD, such as 20250716.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_replaces_and_appends() {
        let dir = std::env::temp_dir().join(format!("envfile-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(&path, "# comment\nA=1\nB=\"two\"\n").unwrap();

        let mut env = EnvFile::load(&path).unwrap();
        assert_eq!(env.get("B").as_deref(), Some("two"));
        env.set("A", "9");
        env.set("C", "3");
        env.save().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "# comment\nA=9\nB=\"two\"\nC=3\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!dir.join(".env.tmp").exists());

        // A file someone made readable to all is written readable to its owner only.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        env.save().unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_values_the_way_compose_does() {
        let mut env = EnvFile::parse("A=1\nPRERE=no  # renewal\nA=2\nQ=\"x # y\"\nH=a#b\nE=\n");
        // The last line of a key set twice wins.
        assert_eq!(env.get("A").as_deref(), Some("2"));
        assert_eq!(env.get("PRERE").as_deref(), Some("no"));
        assert_eq!(env.get("Q").as_deref(), Some("x # y"));
        assert_eq!(env.get("H").as_deref(), Some("a#b"));
        assert_eq!(env.get("E").as_deref(), Some(""));
        env.set("A", "3");
        assert_eq!(env.get("A").as_deref(), Some("3"));
        assert_eq!(env.lines[0], "A=1");
    }

    #[test]
    fn edits_save_only_when_the_change_works() {
        let dir = std::env::temp_dir().join(format!("envfile-edit-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(&path, "A=1\n").unwrap();
        edit(&path, |env| {
            env.set("A", "2");
            Ok(())
        })
        .unwrap();
        let failed: Result<(), String> = edit(&path, |env| {
            env.set("A", "3");
            Err("no".into())
        });
        assert!(failed.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "A=2\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn account_fields() {
        assert!(check_account_field("x", "admin").is_ok());
        assert!(check_account_field("x", "adm").is_err());
        assert!(check_account_field("x", "ad min").is_err());
        assert!(check_account_field("x", "a'; DROP").is_err());
    }

    #[test]
    fn packetvers() {
        assert!(check_packetver("20220406").is_ok());
        assert!(check_packetver("20180620").is_ok());
        assert!(check_packetver("2022046").is_err());
        assert!(check_packetver("20221306").is_err());
        assert!(check_packetver("20220400").is_err());
        assert!(check_packetver("2022-4-6").is_err());
        assert!(check_packetver("19990101").is_err());
    }

    #[test]
    fn passwords_are_plain() {
        let p = random_password(20).unwrap();
        assert_eq!(p.len(), 20);
        assert!(check_account_field("p", &p).is_ok());
    }
}
