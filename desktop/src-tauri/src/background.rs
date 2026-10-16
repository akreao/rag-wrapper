//! Running in the background: the tray icon, and starting the server when
//! the user logs in. What closing the window does is in `lib.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Emitter;

use crate::envfile::EnvFile;
use crate::menu;

/// Added to the program in the login entry, so the app knows to start the
/// server with its window hidden.
pub const AT_LOGIN: &str = "--at-login";

/// The `.env` key for what closing the window does: `no` stops the server
/// and quits; anything else, or none, keeps it running in the tray.
pub const KEEP_RUNNING: &str = "KEEP_RUNNING_ON_CLOSE";

/// Whether closing the window should leave the server running.
pub fn keep_running(env: Option<&EnvFile>) -> bool {
    env.is_some_and(|env| env.get(KEEP_RUNNING).as_deref() != Some("no"))
}

/// Adds the tray icon: open the window, start or stop the server, or quit.
/// Start and Stop go through the window's own buttons, so their output
/// shows in its Activity pane as if they were clicked there.
pub fn tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open rag-wrapper", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "Start server", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop server", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Stop server and quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &start,
            &stop,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("rag-wrapper")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|handle, event| match event.id().as_ref() {
            "open" => crate::show_main(handle),
            "start" | "stop" => {
                let _ = handle.emit("tray-task", event.id().as_ref());
            }
            // The window runs Stop first, so players are warned.
            "quit" => {
                crate::show_main(handle);
                let _ = handle.emit("tray-quit", ());
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// The login entry's name, after the app's identifier.
const ENTRY: &str = "app.ragwrapper.desktop.desktop";

/// Where the desktop looks for programs to run at login.
fn autostart_file() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))?;
    Some(config.join("autostart").join(ENTRY))
}

/// The program a login should run: the AppImage when running from one
/// (the program inside it lives in a mount that goes away), else this one.
fn program() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
}

pub fn start_at_login() -> bool {
    autostart_file().is_some_and(|f| f.exists())
}

/// Adds or removes the login entry. `data` is the app's data folder,
/// where the menu entry keeps the icon.
pub fn set_start_at_login(on: bool, data: &Path) -> Result<(), String> {
    let file = autostart_file().ok_or("Couldn't find your home folder.")?;
    if !on {
        return match fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Couldn't remove {}: {e}", file.display())),
            _ => Ok(()),
        };
    }
    let program = program().ok_or("Couldn't tell where rag-wrapper is.")?;
    let icon = data.join("rag-wrapper.png");
    let entry = login_entry(&program, icon.exists().then_some(icon.as_path()));
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    fs::write(&file, entry).map_err(|e| format!("Couldn't write {}: {e}", file.display()))
}

/// Points an existing login entry at wherever the app now is, as the menu
/// entry does, so it keeps working after the AppImage is moved or replaced.
pub fn refresh(data: &Path) {
    if !start_at_login() {
        return;
    }
    let (Some(file), Some(program)) = (autostart_file(), program()) else {
        return;
    };
    let icon = data.join("rag-wrapper.png");
    let entry = login_entry(&program, icon.exists().then_some(icon.as_path()));
    if fs::read_to_string(&file).ok().as_deref() != Some(entry.as_str()) {
        if let Err(e) = fs::write(&file, entry) {
            eprintln!("Couldn't update the login entry: {e}");
        }
    }
}

fn login_entry(program: &Path, icon: Option<&Path>) -> String {
    menu::desktop_entry(program, icon, AT_LOGIN) + "X-GNOME-Autostart-enabled=true\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_entry_starts_the_server() {
        let entry = login_entry(Path::new("/home/steven/Apps/rag-wrapper.AppImage"), None);
        assert!(entry.contains("\nExec=\"/home/steven/Apps/rag-wrapper.AppImage\" --at-login\n"));
        assert!(!entry.contains("Icon="));
        assert!(entry.ends_with("X-GNOME-Autostart-enabled=true\n"));
    }

    #[test]
    fn keeps_running_unless_told_not_to() {
        assert!(!keep_running(None));
        assert!(keep_running(Some(&EnvFile::parse("PUBLIC_IP=1.2.3.4"))));
        assert!(keep_running(Some(&EnvFile::parse("KEEP_RUNNING_ON_CLOSE=yes"))));
        assert!(!keep_running(Some(&EnvFile::parse("KEEP_RUNNING_ON_CLOSE=no"))));
    }
}
