//! The applications menu entry for the AppImage. An AppImage isn't installed,
//! so on every launch the app writes its own entry pointing at wherever the
//! AppImage now is, which keeps it working after the file is moved or
//! replaced by a newer version.

use std::fs;
use std::path::Path;

/// The entry's file name, after the app's identifier.
const ENTRY: &str = "app.ragwrapper.desktop.desktop";

/// Writes the menu entry and its icon, when running from an AppImage.
/// `data` is the app's data folder, where the icon is kept.
pub fn install(data: &Path) -> std::io::Result<()> {
    let (Some(appimage), Some(appdir), Some(home)) = (
        std::env::var_os("APPIMAGE"),
        std::env::var_os("APPDIR"),
        std::env::var_os("HOME"),
    ) else {
        return Ok(());
    };
    fs::create_dir_all(data)?;
    let icon = data.join("rag-wrapper.png");
    fs::copy(Path::new(&appdir).join("rag-wrapper.png"), &icon)?;
    let applications = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map_or_else(|| Path::new(&home).join(".local/share"), Into::into)
        .join("applications");
    fs::create_dir_all(&applications)?;
    let entry = desktop_entry(Path::new(&appimage), Some(&icon), "");
    let file = applications.join(ENTRY);
    if fs::read_to_string(&file).ok().as_deref() != Some(entry.as_str()) {
        fs::write(file, entry)?;
    }
    Ok(())
}

/// A desktop entry that runs `program`, followed by `args` (already quoted).
pub fn desktop_entry(program: &Path, icon: Option<&Path>, args: &str) -> String {
    let mut exec = exec_arg(&program.to_string_lossy());
    if !args.is_empty() {
        exec = format!("{exec} {args}");
    }
    let icon = icon.map_or_else(String::new, |icon| {
        format!("Icon={}\n", icon.to_string_lossy().replace('\\', "\\\\"))
    });
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=rag-wrapper\n\
         Comment=Runs a private rAthena server and its database\n\
         Exec={exec}\n\
         {icon}\
         Terminal=false\n\
         Categories=Game;\n\
         StartupWMClass=rag-wrapper\n",
    )
}

/// Quotes a path for a desktop entry's `Exec` line, which has its own
/// quoting rules, and then escapes the backslashes the entry format adds.
fn exec_arg(path: &str) -> String {
    let mut quoted = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted.replace('\\', "\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_the_menu_at_the_appimage() {
        let entry = desktop_entry(
            Path::new("/home/steven/Apps/rag-wrapper-0.2.0-x86_64.AppImage"),
            Some(Path::new("/home/steven/.local/share/app.ragwrapper.desktop/rag-wrapper.png")),
            "",
        );
        assert!(entry.contains("\nExec=\"/home/steven/Apps/rag-wrapper-0.2.0-x86_64.AppImage\"\n"));
        assert!(entry.contains("\nIcon=/home/steven/.local/share/app.ragwrapper.desktop/rag-wrapper.png\n"));
    }

    #[test]
    fn quotes_awkward_paths() {
        assert_eq!(exec_arg("/home/s/My Apps/rag"), "\"/home/s/My Apps/rag\"");
        assert_eq!(exec_arg("/a/$b`c\"d"), "\"/a/\\\\$b\\\\`c\\\\\"d\"");
        assert_eq!(exec_arg("/a/100%"), "\"/a/100%%\"");
    }
}
