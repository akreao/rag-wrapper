//! Reads and edits the stack's `.env`, keeping its comments and order.

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

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

    pub fn get(&self, key: &str) -> Option<String> {
        self.lines.iter().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| unquote(v.trim()).to_owned())
        })
    }

    /// Replaces the key's value, or appends `KEY=value` if it isn't there.
    pub fn set(&mut self, key: &str, value: &str) {
        let entry = format!("{key}={value}");
        for line in &mut self.lines {
            if matches!(line.split_once('='), Some((k, _)) if k.trim() == key) {
                *line = entry;
                return;
            }
        }
        self.lines.push(entry);
    }

    /// Writes the file readable by its owner only: it holds passwords.
    pub fn save(&self) -> std::io::Result<()> {
        let mut text = self.lines.join("\n");
        text.push('\n');
        fs::write(&self.path, text)?;
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
    }
}

fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
            return inner;
        }
    }
    v
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
