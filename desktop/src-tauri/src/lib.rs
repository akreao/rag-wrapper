mod backup;
mod envfile;
mod jobs;
mod live;
mod net;
mod settings;
mod stack;
mod task;

use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use envfile::EnvFile;
use task::{Ctx, Running};

struct App {
    dir: PathBuf,
    /// Where database backups go.
    backups: PathBuf,
    /// The running start, stop, build, backup or restore, if any. Only one at a time.
    task: task::Slot,
    logs: Mutex<Option<Child>>,
    /// Bumped on every log switch so the UI can drop lines from the old stream.
    logs_generation: AtomicU64,
    /// Set once the window has asked to close and the stack is going down.
    closing: AtomicBool,
}

impl App {
    fn env_path(&self) -> PathBuf {
        self.dir.join(".env")
    }

    fn env(&self) -> Option<EnvFile> {
        EnvFile::load(&self.env_path()).ok()
    }

    fn overrides_dir(&self) -> PathBuf {
        self.dir.join(stack::OVERRIDES)
    }

    fn ctx(&self, handle: &AppHandle) -> Ctx {
        Ctx { handle: handle.clone(), dir: self.dir.clone(), backups: self.backups.clone(), slot: self.task.clone() }
    }
}

#[derive(Serialize)]
struct Overview {
    stack_dir: String,
    docker_version: Option<String>,
    docker_error: Option<String>,
    configured: bool,
    admin_user: String,
    public_ip: String,
    public_ip_auto: bool,
    addresses: Vec<net::Address>,
    packetver: String,
    prere: bool,
    task: Option<String>,
}

#[tauri::command]
fn overview(app: State<App>) -> Overview {
    let (docker_version, docker_error) = match stack::check_docker() {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e)),
    };
    let env = app.env();
    let get = |k: &str| env.as_ref().and_then(|e| e.get(k)).unwrap_or_default();
    Overview {
        stack_dir: app.dir.display().to_string(),
        docker_version,
        docker_error,
        configured: !get("ADMIN_USER").is_empty(),
        admin_user: get("ADMIN_USER"),
        public_ip: get("PUBLIC_IP"),
        public_ip_auto: get("PUBLIC_IP_AUTO") != "no",
        addresses: net::lan_addresses(),
        packetver: get("PACKETVER"),
        prere: get("PRERE") != "no",
        task: app.task.lock().unwrap().as_ref().map(|r| r.kind.clone()),
    }
}

/// First-run setup: writes `.env` with the admin account, the LAN address
/// and generated server passwords. `db-init` creates the admin on next start.
#[tauri::command]
fn setup(app: State<App>, admin_user: String, admin_password: String, public_ip: Option<String>) -> Result<(), String> {
    envfile::check_account_field("The username", &admin_user)?;
    envfile::check_account_field("The password", &admin_password)?;

    let path = app.env_path();
    let fresh = !path.exists();
    if fresh {
        std::fs::write(&path, stack::ENV_EXAMPLE).map_err(|e| e.to_string())?;
    }
    let mut env = EnvFile::load(&path).map_err(|e| e.to_string())?;

    // Replace the example's placeholder passwords, unless a database made
    // with them already exists and would stop accepting new ones.
    if fresh && !stack::volume_exists("db-data") {
        for key in ["DB_PASSWORD", "DB_ROOT_PASSWORD", "INTER_PASSWORD"] {
            env.set(key, &envfile::random_password(20).map_err(|e| e.to_string())?);
        }
    }
    env.set("ADMIN_USER", &admin_user);
    env.set("ADMIN_PASSWORD", &admin_password);
    apply_public_ip(&mut env, public_ip);
    env.save().map_err(|e| e.to_string())
}

/// Sets the build settings. Returns whether anything changed, in which
/// case the image needs a rebuild.
#[tauri::command]
fn save_build_settings(app: State<App>, packetver: String, prere: bool) -> Result<bool, String> {
    let packetver = packetver.trim();
    envfile::check_packetver(packetver)?;
    let mut env = app.env().ok_or("Finish setup first.")?;
    let prere = if prere { "yes" } else { "no" };
    let changed = env.get("PACKETVER").as_deref() != Some(packetver) || env.get("PRERE").as_deref() != Some(prere);
    env.set("PACKETVER", packetver);
    env.set("PRERE", prere);
    env.save().map_err(|e| e.to_string())?;
    Ok(changed)
}

#[tauri::command]
fn game_settings(app: State<App>) -> settings::GameSettings {
    settings::load(&app.overrides_dir())
}

/// Writes the game settings as rAthena overrides. Returns whether anything
/// changed, in which case the game servers need a restart.
#[tauri::command]
fn save_game_settings(app: State<App>, settings: settings::GameSettings) -> Result<bool, String> {
    settings::save(&app.overrides_dir(), &settings)
}

/// Opens the overrides folder in the file manager, for settings the
/// Settings screen doesn't cover.
#[tauri::command]
fn open_overrides(handle: AppHandle, app: State<App>) -> Result<(), String> {
    let dir = app.overrides_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    handle
        .opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// `None` follows the detected LAN address on every start; `Some` pins one.
#[tauri::command]
fn set_public_ip(app: State<App>, public_ip: Option<String>) -> Result<(), String> {
    let mut env = app.env().ok_or("Finish setup first.")?;
    apply_public_ip(&mut env, public_ip);
    env.save().map_err(|e| e.to_string())
}

fn apply_public_ip(env: &mut EnvFile, public_ip: Option<String>) {
    match public_ip {
        Some(ip) => {
            env.set("PUBLIC_IP_AUTO", "no");
            env.set("PUBLIC_IP", ip.trim());
        }
        None => {
            env.set("PUBLIC_IP_AUTO", "yes");
            env.set("PUBLIC_IP", &net::best_lan_ip().unwrap_or_else(|| "127.0.0.1".into()));
        }
    }
}

#[tauri::command]
fn services(app: State<App>) -> Result<Vec<stack::ServiceStatus>, String> {
    if !app.env_path().exists() {
        return Ok(Vec::new());
    }
    stack::status(&app.dir)
}

/// Who's online, uptime and each container's CPU and memory, for the
/// main screen. Takes a couple of seconds, so it runs off the main thread.
#[tauri::command]
async fn live(app: State<'_, App>) -> Result<live::Live, String> {
    if !app.env_path().exists() {
        return Ok(live::Live::default());
    }
    let dir = app.dir.clone();
    tauri::async_runtime::spawn_blocking(move || live::read(&dir))
        .await
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Clone)]
struct TaskEvent {
    kind: String,
    running: bool,
    ok: bool,
}

/// One step of a task, run in order until one fails.
enum Step {
    Compose(std::process::Command),
    Backup(backup::Reason),
    /// A backup if there's none from the last day; never fails the task.
    Daily,
    Restore(String),
}

/// Runs `start`, `restart`, `stop`, `build`, `backup` or `restore` (of
/// `file`), streaming its output as `task-line` events.
#[tauri::command]
fn run_task(handle: AppHandle, app: State<App>, kind: String, file: Option<String>) -> Result<(), String> {
    if app.closing.load(Ordering::SeqCst) {
        return Err("The app is closing.".into());
    }
    let mut slot = app.task.lock().unwrap();
    if let Some(running) = slot.as_ref() {
        return Err(format!("Wait for the current {} to finish.", running.kind));
    }

    let mut cmd = stack::compose(&app.dir);
    let steps = match kind.as_str() {
        "start" | "restart" => {
            let mut env = app.env().ok_or("Finish setup first.")?;
            if env.get("PUBLIC_IP_AUTO").as_deref() != Some("no") {
                apply_public_ip(&mut env, None);
                env.save().map_err(|e| e.to_string())?;
            }
            cmd.args(["--progress", "plain", "up", "--detach"]);
            if kind == "restart" {
                // The game servers read their configs only when they start;
                // the database and portal keep running.
                cmd.args(["--no-deps", "--force-recreate"]).args(stack::GAME_SERVERS);
            }
            vec![Step::Compose(cmd)]
        }
        "build" => {
            cmd.args(["--progress", "plain", "build"]);
            // A new build can upgrade the database on its next start.
            vec![Step::Backup(backup::Reason::Rebuild), Step::Compose(cmd)]
        }
        "stop" => {
            cmd.arg("down");
            vec![Step::Daily, Step::Compose(cmd)]
        }
        "backup" | "restore" => {
            app.env().ok_or("Finish setup first.")?;
            if kind == "backup" {
                vec![Step::Backup(backup::Reason::Manual)]
            } else {
                vec![Step::Restore(file.ok_or("Pick a backup to restore.")?)]
            }
        }
        _ => return Err(format!("Unknown task {kind}")),
    };

    *slot = Some(Running {
        kind: kind.clone(),
        pid: None,
        cancelled: false,
    });
    drop(slot);
    let _ = handle.emit(
        "task",
        TaskEvent {
            kind: kind.clone(),
            running: true,
            ok: true,
        },
    );

    let ctx = app.ctx(&handle);
    std::thread::spawn(move || {
        let result = steps.into_iter().try_for_each(|step| {
            ctx.check_cancelled()?;
            match step {
                Step::Compose(cmd) => ctx.run(cmd),
                Step::Backup(reason) => backup::create(&ctx, reason, None).map(|_| ()),
                Step::Daily => {
                    backup::daily(&ctx);
                    Ok(())
                }
                Step::Restore(file) => backup::restore(&ctx, &file),
            }
        });
        if let Err(e) = &result {
            ctx.line(e.clone());
        }
        let ok = result.is_ok();
        if ok && kind == "start" {
            forget_admin_password(&ctx.dir);
        }
        ctx.slot.lock().unwrap().take();
        let _ = handle.emit(
            "task",
            TaskEvent {
                kind,
                running: false,
                ok,
            },
        );
    });
    Ok(())
}

/// Once `db-init` has finished, the admin account exists and the password
/// has no further use, so no plain-text copy is kept in `.env`.
fn forget_admin_password(dir: &std::path::Path) {
    let Ok(mut env) = EnvFile::load(&dir.join(".env")) else {
        return;
    };
    if env.get("ADMIN_PASSWORD").unwrap_or_default().is_empty() {
        return;
    }
    let initialised = stack::status(dir)
        .unwrap_or_default()
        .iter()
        .any(|s| s.service == "db-init" && s.state == "exited" && s.exit_code == 0);
    if initialised {
        env.set("ADMIN_PASSWORD", "");
        let _ = env.save();
    }
}

#[tauri::command]
fn cancel_task(app: State<App>) {
    if let Some(running) = app.task.lock().unwrap().as_mut() {
        running.cancel();
    }
}

/// The backups, newest first.
#[tauri::command]
fn backups(app: State<App>) -> Vec<backup::Backup> {
    backup::list(&app.backups)
}

/// Opens the backups folder in the file manager, so backups can be copied elsewhere.
#[tauri::command]
fn open_backups(handle: AppHandle, app: State<App>) -> Result<(), String> {
    std::fs::create_dir_all(&app.backups).map_err(|e| e.to_string())?;
    handle
        .opener()
        .open_path(app.backups.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Clone)]
struct LogLine {
    generation: u64,
    line: String,
}

/// Follows one service's logs as `log-line` events, replacing any earlier
/// stream. Returns the stream's generation. `None` just stops streaming.
#[tauri::command]
fn follow_logs(handle: AppHandle, app: State<App>, service: Option<String>) -> Result<u64, String> {
    let generation = app.logs_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let mut slot = app.logs.lock().unwrap();
    if let Some(mut old) = slot.take() {
        let _ = old.kill();
        let _ = old.wait();
    }
    let Some(service) = service else {
        return Ok(generation);
    };

    let mut cmd = stack::compose(&app.dir);
    cmd.args(["logs", "--follow", "--no-log-prefix", "--tail", "500"])
        .arg(&service);
    let (child, _) = stack::spawn_lines(cmd, move |line| {
        let _ = handle.emit("log-line", LogLine { generation, line });
    })
    .map_err(|e| e.to_string())?;
    *slot = Some(child);
    Ok(generation)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let (dir, backups) = match stack::stack_dir_override() {
                Some(dir) => {
                    let backups = dir.join("backups");
                    (dir, backups)
                }
                None => {
                    let data = app.path().app_data_dir()?;
                    let dir = data.join("stack");
                    let bundled = app.path().resource_dir()?.join("stack");
                    stack::install(&bundled, &dir)?;
                    (dir, data.join("backups"))
                }
            };
            app.manage(App {
                dir,
                backups,
                task: Arc::new(Mutex::new(None)),
                logs: Mutex::new(None),
                logs_generation: AtomicU64::new(0),
                closing: AtomicBool::new(false),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            setup,
            set_public_ip,
            save_build_settings,
            game_settings,
            save_game_settings,
            open_overrides,
            services,
            live,
            run_task,
            cancel_task,
            backups,
            open_backups,
            follow_logs
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let handle = window.app_handle().clone();
                let app = handle.state::<App>();
                if !app.closing.swap(true, Ordering::SeqCst) {
                    let _ = handle.emit("closing", ());
                    let handle = handle.clone();
                    std::thread::spawn(move || stop_and_exit(handle));
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                let app = handle.state::<App>();
                let follower = app.logs.lock().unwrap().take();
                if let Some(mut child) = follower {
                    let _ = child.kill();
                }
            }
        });
}

/// The server only runs while the app is open: closing the window cancels
/// any build or start (a backup or restore is left to finish), backs up
/// the database if there's no backup from the last day, takes the stack
/// down (the database volume is kept) and then exits.
fn stop_and_exit(handle: AppHandle) {
    let app = handle.state::<App>();
    if let Some(running) = app.task.lock().unwrap().as_mut() {
        if !["backup", "restore"].contains(&running.kind.as_str()) {
            running.cancel();
        }
    }
    while app.task.lock().unwrap().is_some() {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    if app.env_path().exists() {
        backup::daily(&app.ctx(&handle));
        let mut cmd = stack::compose(&app.dir);
        cmd.arg("down");
        let lines = handle.clone();
        if let Ok((mut child, done)) = stack::spawn_lines(cmd, move |line| {
            let _ = lines.emit("task-line", line);
        }) {
            let _ = done.join();
            let _ = child.wait();
        }
    }
    handle.exit(0);
}
