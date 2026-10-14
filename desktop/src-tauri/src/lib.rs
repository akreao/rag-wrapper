mod backup;
mod clientpatch;
mod envfile;
mod host;
mod jobs;
mod live;
mod menu;
mod mods;
mod net;
mod settings;
mod stack;
mod task;
mod thor;
mod update;

use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use envfile::EnvFile;
use task::{Ctx, Running};

struct App {
    dir: PathBuf,
    /// Where database backups go.
    backups: PathBuf,
    /// The files players get through the patcher, as they sit in the client folder.
    client_files: PathBuf,
    /// Held while a client patch is being built, so two can't take the same number.
    publishing: Arc<Mutex<()>>,
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

    fn patches_dir(&self) -> PathBuf {
        self.dir.join(stack::PATCHES)
    }

    fn mods_dir(&self) -> PathBuf {
        self.dir.join(stack::MODS)
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
    /// The rAthena commit this app version comes with, when the server isn't on it yet.
    server_update: Option<update::ServerUpdate>,
    /// The game servers run an older image than the one built, and restart onto it when asked.
    restart_pending: bool,
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
        docker_error: docker_error.clone(),
        configured: !get("ADMIN_USER").is_empty(),
        admin_user: get("ADMIN_USER"),
        public_ip: get("PUBLIC_IP"),
        public_ip_auto: get("PUBLIC_IP_AUTO") != "no",
        addresses: net::lan_addresses(),
        packetver: get("PACKETVER"),
        prere: get("PRERE") != "no",
        server_update: env.as_ref().filter(|_| !get("ADMIN_USER").is_empty()).and_then(update::pending),
        restart_pending: docker_error.is_none() && stack::runs_old_image(&app.dir, &get("PACKETVER")),
        task: app.task.lock().unwrap().as_ref().map(|r| r.kind.clone()),
    }
}

/// First-run setup: writes `.env` with the admin account, the LAN address
/// and generated server passwords. `db-init` creates the admin on next start.
#[tauri::command]
fn setup(app: State<App>, admin_user: String, admin_password: String, public_ip: Option<String>) -> Result<(), String> {
    envfile::check_account_field("The username", &admin_user)?;
    envfile::check_account_field("The password", &admin_password)?;
    if let Some(ip) = &public_ip {
        net::check_address(ip)?;
    }

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
    // A new install already builds this version's commit.
    if fresh {
        if let Some(update) = update::pending(&env) {
            update::skip(&mut env, &update);
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

/// Keeps the server on its current rAthena commit and hides the offer
/// until an app version comes with another one.
#[tauri::command]
fn skip_server_update(app: State<App>) -> Result<(), String> {
    let mut env = app.env().ok_or("Finish setup first.")?;
    if let Some(update) = update::pending(&env) {
        update::skip(&mut env, &update);
        env.save().map_err(|e| e.to_string())?;
    }
    Ok(())
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

/// Opens a web page (the account portal, an update's changes) in the browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("Not a web address: {url}"));
    }
    host::open(url.as_ref())
}

/// Opens the overrides folder in the file manager, for settings the
/// Settings screen doesn't cover.
#[tauri::command]
fn open_overrides(app: State<App>) -> Result<(), String> {
    let dir = app.overrides_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    host::open(dir.as_os_str())
}

/// `None` follows the detected LAN address on every start; `Some` pins one,
/// such as a VPN address. Returns whether the address players are sent to
/// changed, in which case running char and map servers need a restart.
#[tauri::command]
fn set_public_ip(app: State<App>, public_ip: Option<String>) -> Result<bool, String> {
    if let Some(ip) = &public_ip {
        net::check_address(ip)?;
    }
    let mut env = app.env().ok_or("Finish setup first.")?;
    let before = env.get("PUBLIC_IP");
    apply_public_ip(&mut env, public_ip);
    env.save().map_err(|e| e.to_string())?;
    Ok(env.get("PUBLIC_IP") != before)
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
    /// Moves `.env` to the update's commit.
    Pin(update::ServerUpdate),
}

/// Runs `start`, `restart`, `readdress` (after the address changed), `stop`,
/// `build`, `update`, `backup` or `restore` (of `file`), streaming its
/// output as `task-line` events.
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
        "start" | "restart" | "readdress" => {
            let mut cmd = up_command(&app)?;
            // After a rebuild or update the new image needs a full `up`,
            // so `db-init` applies its database upgrades first; that
            // recreates every rAthena container anyway.
            let new_image = kind != "start"
                && stack::runs_old_image(&app.dir, &app.env().and_then(|e| e.get("PACKETVER")).unwrap_or_default());
            if kind != "start" && !new_image {
                // The game servers read their configs only when they start;
                // the database keeps running. A new address only concerns
                // the char and map servers, which hand it to the client,
                // and the portal's "How to connect" page.
                let services: &[&str] = if kind == "readdress" { &stack::READDRESSED } else { &stack::GAME_SERVERS };
                cmd.args(["--no-deps", "--force-recreate"]).args(services);
            }
            // A mod's database files can change items or monsters players
            // already have, so the database is backed up before they load.
            if mods::new_databases(&app.mods_dir()).is_empty() {
                vec![Step::Compose(cmd)]
            } else {
                vec![Step::Backup(backup::Reason::Mods), Step::Compose(cmd)]
            }
        }
        "build" => {
            cmd.args(["--progress", "plain", "build"]);
            // A new build can upgrade the database on its next start.
            vec![Step::Backup(backup::Reason::Rebuild), Step::Compose(cmd)]
        }
        "update" => {
            let env = app.env().ok_or("Finish setup first.")?;
            let update = update::pending(&env).ok_or("The server is already on this version's rAthena.")?;
            // The new commit goes into `.env` only once its image has built,
            // so a failed or cancelled build leaves the update on offer.
            cmd.env("RATHENA_REPO", &update.repo)
                .env("RATHENA_COMMIT", &update.offered)
                .args(["--progress", "plain", "build"]);
            // A running server stays on the old image until "Restart on the
            // new version" (or the next Start); players aren't dropped by surprise.
            vec![Step::Backup(backup::Reason::Update), Step::Compose(cmd), Step::Pin(update)]
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
                Step::Pin(update) => {
                    let mut env = EnvFile::load(&ctx.dir.join(".env")).map_err(|e| e.to_string())?;
                    update::pin(&mut env, &update);
                    env.save().map_err(|e| e.to_string())
                }
            }
        });
        if let Err(e) = &result {
            ctx.line(e.clone());
        }
        let ok = result.is_ok();
        if ok && kind == "start" {
            forget_admin_password(&ctx.dir);
        }
        // An address change restarts the map server too, and a restore starts
        // the game servers again when they were running.
        let started = match kind.as_str() {
            "start" | "restart" | "readdress" => true,
            "restore" => stack::status(&ctx.dir)
                .unwrap_or_default()
                .iter()
                .any(|s| s.service == "map" && s.state == "running"),
            _ => false,
        };
        if ok && started {
            if let Err(e) = mods::record_applied(&ctx.dir.join(stack::MODS)) {
                ctx.line(format!("Couldn't record the loaded mods: {e}"));
            }
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

/// `docker compose up` for the whole stack, after re-detecting the LAN
/// address unless one is pinned.
fn up_command(app: &App) -> Result<std::process::Command, String> {
    let mut env = app.env().ok_or("Finish setup first.")?;
    if env.get("PUBLIC_IP_AUTO").as_deref() != Some("no") {
        apply_public_ip(&mut env, None);
        env.save().map_err(|e| e.to_string())?;
    }
    let mut cmd = stack::compose(&app.dir);
    cmd.args(["--progress", "plain", "up", "--detach"]);
    Ok(cmd)
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
fn open_backups(app: State<App>) -> Result<(), String> {
    std::fs::create_dir_all(&app.backups).map_err(|e| e.to_string())?;
    host::open(app.backups.as_os_str())
}

/// What the next client patch would carry, and the patches published so far.
#[tauri::command]
async fn client_patch(app: State<'_, App>) -> Result<clientpatch::Status, String> {
    let (files, patches) = (app.client_files.clone(), app.patches_dir());
    tauri::async_runtime::spawn_blocking(move || clientpatch::status(&files, &patches))
        .await
        .map_err(|e| e.to_string())?
}

/// Publishes what changed in the client-files folder as a new patch. The
/// portal serves it straight away; the server doesn't need to restart.
#[tauri::command]
async fn publish_client_patch(app: State<'_, App>) -> Result<clientpatch::Publish, String> {
    let (files, patches, lock) = (app.client_files.clone(), app.patches_dir(), app.publishing.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let _held = lock.try_lock().map_err(|_| "A patch is already being published.")?;
        clientpatch::publish(&files, &patches)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens the client-files folder in the file manager to add or change files.
#[tauri::command]
fn open_client_files(app: State<App>) -> Result<(), String> {
    clientpatch::ensure_folder(&app.client_files).map_err(|e| e.to_string())?;
    host::open(app.client_files.as_os_str())
}

/// The mod folders, which are on, and which changes wait for a restart.
/// Reads the map server's log for errors, so it runs off the main thread.
#[tauri::command]
async fn mods(app: State<'_, App>) -> Result<mods::Status, String> {
    let (dir, mods) = (app.dir.clone(), app.mods_dir());
    tauri::async_runtime::spawn_blocking(move || {
        let running = stack::status(&dir)
            .unwrap_or_default()
            .iter()
            .any(|s| s.service == "map" && s.state == "running");
        let log = if running {
            let mut logs = stack::compose(&dir);
            logs.args(["logs", "--no-log-prefix", "map"]);
            stack::output(logs).unwrap_or_default()
        } else {
            String::new()
        };
        mods::status(&mods, running, &log)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Switches a mod on or off; it applies when the game servers restart.
#[tauri::command]
fn set_mod_enabled(app: State<App>, name: String, enabled: bool) -> Result<(), String> {
    mods::set_enabled(&app.mods_dir(), &name, enabled)
}

/// Moves an enabled mod one place earlier or later in the load order.
#[tauri::command]
fn move_mod(app: State<App>, name: String, up: bool) -> Result<(), String> {
    mods::move_mod(&app.mods_dir(), &name, up)
}

/// Opens the mods folder in the file manager, to add mods by copying folders in.
#[tauri::command]
fn open_mods(app: State<App>) -> Result<(), String> {
    let dir = app.mods_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    host::open(dir.as_os_str())
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
        .setup(|app| {
            // A menu entry is a convenience: the app runs without one.
            if let Err(e) = menu::install(&app.path().app_data_dir()?) {
                eprintln!("Couldn't add the applications menu entry: {e}");
            }
            // Backups and client files sit beside the stack, not in it.
            let (dir, data) = match stack::stack_dir_override() {
                Some(dir) => (dir.clone(), dir),
                None => {
                    let data = app.path().app_data_dir()?;
                    let dir = data.join("stack");
                    let bundled = app.path().resource_dir()?.join("stack");
                    stack::install(&bundled, &dir)?;
                    (dir, data)
                }
            };
            app.manage(App {
                dir,
                backups: data.join("backups"),
                client_files: data.join("client-files"),
                publishing: Arc::new(Mutex::new(())),
                task: Arc::new(Mutex::new(None)),
                logs: Mutex::new(None),
                logs_generation: AtomicU64::new(0),
                closing: AtomicBool::new(false),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_url,
            overview,
            setup,
            set_public_ip,
            save_build_settings,
            skip_server_update,
            game_settings,
            save_game_settings,
            open_overrides,
            services,
            live,
            run_task,
            cancel_task,
            backups,
            open_backups,
            client_patch,
            publish_client_patch,
            open_client_files,
            mods,
            set_mod_enabled,
            move_mod,
            open_mods,
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
