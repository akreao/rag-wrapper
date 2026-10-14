//! Programs the app starts on the host: Docker, and the browser or file
//! manager behind `xdg-open`.
//!
//! In the AppImage, its launcher points `LD_LIBRARY_PATH`, `XDG_DATA_DIRS`,
//! GTK's module paths and the like at the libraries bundled inside it, which
//! the app needs and the host's programs don't: a browser or file manager
//! started with them loads the wrong GTK and fails. So every program the app
//! starts gets the environment the user's session had, with the AppImage's
//! entries taken out again.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

/// Set by the AppImage's launcher but not paths inside it: GTK's fixed
/// theme, and the AppImage's own location, which would tell an AppImage the
/// app starts that it is this one.
const LAUNCHER_ONLY: [&str; 4] = ["GTK_THEME", "APPIMAGE", "ARGV0", "OWD"];

/// A command for a program on the host, without the AppImage's environment.
pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    if let Some(appdir) = std::env::var_os("APPDIR") {
        for (key, value) in host_env(Path::new(&appdir), std::env::vars_os()) {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            };
        }
    }
    cmd
}

/// What to change in the app's own environment for a host program: each
/// variable that names a path inside the AppImage loses those entries (and is
/// removed if nothing is left), and the launcher's own settings are removed.
fn host_env(
    appdir: &Path,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, Option<OsString>)> {
    let mut changes = Vec::new();
    for (key, value) in vars {
        if LAUNCHER_ONLY.iter().any(|k| OsStr::new(k) == key) {
            changes.push((key, None));
            continue;
        }
        let entries: Vec<_> = std::env::split_paths(&value).collect();
        if !entries.iter().any(|e| e.starts_with(appdir)) {
            continue;
        }
        let kept: Vec<_> = entries
            .into_iter()
            .filter(|e| !e.starts_with(appdir) && !e.as_os_str().is_empty())
            .collect();
        let value = if kept.is_empty() {
            None
        } else {
            std::env::join_paths(kept).ok()
        };
        changes.push((key, value));
    }
    changes
}

/// Opens a folder or web address with the desktop's default program.
pub fn open(target: &OsStr) -> Result<(), String> {
    let mut child = command("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't run xdg-open: {e}"))?;
    // Reaped in the background: xdg-open can wait on the program it starts.
    thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        list.iter().map(|(k, v)| (k.into(), v.into())).collect()
    }

    fn change(key: &str, value: Option<&str>) -> (OsString, Option<OsString>) {
        (key.into(), value.map(OsString::from))
    }

    #[test]
    fn takes_the_appimage_out_of_the_environment() {
        let appdir = Path::new("/tmp/.mount_ragwr1");
        let changes = host_env(
            appdir,
            vars(&[
                ("PATH", "/tmp/.mount_ragwr1/usr/bin/:/tmp/.mount_ragwr1/usr/sbin/:/usr/local/bin:/usr/bin"),
                ("LD_LIBRARY_PATH", "/tmp/.mount_ragwr1/usr/lib/:/tmp/.mount_ragwr1/lib64/:"),
                ("XDG_DATA_DIRS", "/tmp/.mount_ragwr1/usr/share:/usr/share:/usr/local/share"),
                ("GTK_PATH", "/tmp/.mount_ragwr1//usr/lib/gtk-3.0"),
                ("APPDIR", "/tmp/.mount_ragwr1"),
                ("GTK_THEME", "Adwaita:light"),
                ("APPIMAGE", "/home/steven/Apps/rag-wrapper-0.1.0-x86_64.AppImage"),
                ("HOME", "/home/steven"),
                ("DOCKER_HOST", "unix:///run/docker.sock"),
            ]),
        );
        assert_eq!(
            changes,
            vec![
                change("PATH", Some("/usr/local/bin:/usr/bin")),
                change("LD_LIBRARY_PATH", None),
                change("XDG_DATA_DIRS", Some("/usr/share:/usr/local/share")),
                change("GTK_PATH", None),
                change("APPDIR", None),
                change("GTK_THEME", None),
                change("APPIMAGE", None),
            ]
        );
    }

    #[test]
    fn a_folder_that_only_starts_with_the_same_letters_stays() {
        let changes = host_env(
            Path::new("/tmp/.mount_ragwr1"),
            vars(&[("PATH", "/tmp/.mount_ragwr1/usr/bin:/tmp/.mount_ragwr12/bin")]),
        );
        assert_eq!(changes, vec![change("PATH", Some("/tmp/.mount_ragwr12/bin"))]);
    }
}
