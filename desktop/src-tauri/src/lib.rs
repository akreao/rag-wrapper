mod backup;
mod battleconf;
mod clientpatch;
mod customnpcs;
mod envfile;
mod host;
mod images;
mod itemkit;
mod jobs;
mod live;
mod luamerge;
mod menu;
mod mods;
mod net;
mod news;
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
    /// The running start, stop, build, backup, restore or restore test, if any. Only one at a time.
    task: task::Slot,
    logs: Mutex<Option<Child>>,
    /// Bumped on every log switch so the UI can drop lines from the old stream.
    logs_generation: AtomicU64,
    /// Set once the window has asked to close and the stack is going down.
    closing: AtomicBool,
    /// The last clash check of the mods: what it checked and its output,
    /// which is None when the image couldn't run it.
    clash_check: Arc<ClashCheck>,
}

/// What the last clash check checked, and its output.
type ClashCheck = Mutex<Option<(String, Option<String>)>>;

impl App {
    fn env_path(&self) -> PathBuf {
        self.dir.join(".env")
    }

    fn env(&self) -> Option<EnvFile> {
        EnvFile::load(&self.env_path()).ok()
    }

    /// Whether the server runs Renewal (`PRERE=no`) rather than Pre-Renewal.
    fn renewal(&self) -> bool {
        self.env().and_then(|e| e.get("PRERE")).as_deref() == Some("no")
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
    portal_port: String,
    public_ip_auto: bool,
    addresses: Vec<net::Address>,
    packetver: String,
    prere: bool,
    /// The rAthena commit this app version comes with, when the server isn't on it yet.
    server_update: Option<update::ServerUpdate>,
    /// The game servers run an older image than the one built, and restart onto it when asked.
    restart_pending: bool,
    /// The same for the portal alone, after a Rebuild that changed only the portal.
    portal_restart_pending: bool,
    /// Images here that only a Rebuild brings up to date.
    rebuild: Option<images::Rebuild>,
    /// The server started on another address than before, and players
    /// haven't been told yet.
    address_change: Option<AddressChange>,
    task: Option<String>,
}

/// The address players' files carry and the one the server now runs on,
/// with the portal port only when that changed too.
#[derive(Serialize, Debug, PartialEq)]
struct AddressChange {
    from: String,
    to: String,
    /// The How to connect page at the new address.
    link: String,
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
        portal_port: env.as_ref().and_then(|e| e.get("PORTAL_PORT")).unwrap_or_else(|| "8080".into()),
        public_ip_auto: get("PUBLIC_IP_AUTO") != "no",
        addresses: net::lan_addresses(),
        packetver: get("PACKETVER"),
        prere: get("PRERE") != "no",
        server_update: env.as_ref().filter(|_| !get("ADMIN_USER").is_empty()).and_then(update::pending),
        restart_pending: docker_error.is_none() && stack::runs_old_image(&app.dir, &get("PACKETVER")),
        portal_restart_pending: docker_error.is_none() && stack::portal_runs_old_image(&app.dir),
        rebuild: env
            .as_ref()
            .filter(|_| docker_error.is_none() && !get("ADMIN_USER").is_empty())
            .and_then(|env| images::rebuild(&app.dir, env)),
        address_change: env.as_ref().and_then(address_change),
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

    write_setup(&app.env_path(), || stack::volume_exists("db-data"), &admin_user, &admin_password, public_ip)
}

/// `setup`'s changes to the `.env` at `path`, made from `.env.example` when
/// there's none. `db_exists` says whether the database volume is there.
fn write_setup(
    path: &std::path::Path,
    db_exists: impl FnOnce() -> bool,
    admin_user: &str,
    admin_password: &str,
    public_ip: Option<String>,
) -> Result<(), String> {
    let fresh = !path.exists();
    if fresh {
        std::fs::write(path, stack::ENV_EXAMPLE).map_err(|e| e.to_string())?;
    }
    let mut env = EnvFile::load(path).map_err(|e| e.to_string())?;

    // Replace the example's placeholder passwords, unless a database made
    // with them already exists and would stop accepting new ones.
    if fresh && !db_exists() {
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
    env.set("ADMIN_USER", admin_user);
    env.set("ADMIN_PASSWORD", admin_password);
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

/// The battle config editor's files: `conf/battle/*.conf` from the
/// server's image with the overrides folder's values. The first read after
/// a build copies them out of the image, so it runs off the main thread.
#[tauri::command]
async fn battle_conf(app: State<'_, App>) -> Result<battleconf::BattleConf, String> {
    let (dir, overrides, env) = (app.dir.clone(), app.overrides_dir(), app.env());
    tauri::async_runtime::spawn_blocking(move || {
        let env = env.ok_or("Set up the server first.")?;
        battleconf::load(&dir, &overrides, &env, &map_log(&dir))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The running map server's log, or nothing when it isn't running.
fn map_log(dir: &std::path::Path) -> String {
    let running = stack::status(dir).unwrap_or_default().iter().any(|s| s.service == "map" && s.state == "running");
    if !running {
        return String::new();
    }
    let mut logs = stack::compose(dir);
    logs.args(["logs", "--no-log-prefix", "map"]);
    stack::output(logs).unwrap_or_default()
}

/// Saves battle config values (option to value) as overrides. Returns
/// whether anything changed, in which case the game servers need a restart.
#[tauri::command]
async fn save_battle_conf(app: State<'_, App>, changes: std::collections::BTreeMap<String, String>) -> Result<bool, String> {
    let (dir, overrides, env) = (app.dir.clone(), app.overrides_dir(), app.env());
    tauri::async_runtime::spawn_blocking(move || {
        let env = env.ok_or("Set up the server first.")?;
        battleconf::save(&dir, &overrides, &env, &changes)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens the battle config editor in its own window, or brings it to the
/// front. Async, because building a window from a sync command can
/// deadlock on Linux.
#[tauri::command]
async fn open_battle_conf(handle: AppHandle) -> Result<(), String> {
    if let Some(window) = handle.get_webview_window(BATTLE_CONF_WINDOW) {
        let _ = window.unminimize();
        return window.set_focus().map_err(|e| e.to_string());
    }
    tauri::WebviewWindowBuilder::new(&handle, BATTLE_CONF_WINDOW, tauri::WebviewUrl::App("battle-conf.html".into()))
        .title("Battle config - rag-wrapper")
        .inner_size(960.0, 760.0)
        .min_inner_size(640.0, 480.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The battle config editor's window label (see `capabilities/default.json`).
const BATTLE_CONF_WINDOW: &str = "battle-conf";

/// Opens a web page (the account portal, an update's changes) in the browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("Not a web address: {url}"));
    }
    host::open(url.as_ref())
}

/// The News screen's posts, from the overrides folder's `portal/news.md`.
#[tauri::command]
fn news(app: State<App>) -> news::News {
    news::load(&app.overrides_dir())
}

/// Writes the News screen's posts. The portal reads the file on every
/// request, so they show straight away. Returns the news as saved.
#[tauri::command]
fn save_news(app: State<App>, news: news::News) -> Result<news::News, String> {
    news::save(&app.overrides_dir(), &news)
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

/// Hides the address change warning once players have been told.
#[tauri::command]
fn dismiss_address_change(app: State<App>) -> Result<(), String> {
    let mut env = app.env().ok_or("Finish setup first.")?;
    env.set("ADDRESS_CHANGED_FROM", "");
    env.save().map_err(|e| e.to_string())
}

/// The port players open the account portal on. Returns whether it
/// changed, in which case a running portal needs restarting on it.
#[tauri::command]
fn set_portal_port(app: State<App>, port: u16) -> Result<bool, String> {
    if port < 1024 {
        return Err("Pick a port from 1024 to 65535.".into());
    }
    let mut env = app.env().ok_or("Finish setup first.")?;
    let before = env.get("PORTAL_PORT");
    env.set("PORTAL_PORT", &port.to_string());
    env.save().map_err(|e| e.to_string())?;
    Ok(env.get("PORTAL_PORT") != before)
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
    /// Loads a backup into a throwaway database to check it would restore.
    TestRestore(String),
    /// Moves `.env` to the update's commit.
    Pin(update::ServerUpdate),
    /// Pulls the prebuilt images, building any that aren't published with the command.
    Images(Vec<images::Image>, std::process::Command),
    /// Pulls the prebuilt images that aren't here yet, before a start.
    FetchMissing(Vec<images::Image>),
    /// Checks that nothing else holds the ports the stack publishes.
    Ports,
    /// Notes what images `up` just built were made from.
    Record(Vec<images::Image>),
}

/// Runs `start`, `restart`, `readdress` (after the address changed),
/// `portal` (after the portal port changed), `stop`,
/// `build`, `update`, `backup`, or `restore` or `test-restore` (of `file`), streaming its
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
        "start" | "restart" | "readdress" | "portal" => {
            let mut cmd = up_command(&app)?;
            // After a rebuild or update the new image needs a full `up`,
            // so `db-init` applies its database upgrades first; that
            // recreates every rAthena container anyway.
            let new_image = kind != "start"
                && (stack::runs_old_image(&app.dir, &app.env().and_then(|e| e.get("PACKETVER")).unwrap_or_default())
                    || stack::portal_runs_old_image(&app.dir));
            if kind != "start" && !new_image {
                // The game servers read their configs only when they start;
                // the database keeps running. A new address only concerns
                // the char and map servers, which hand it to the client,
                // and the portal's "How to connect" page.
                let services: &[&str] = match kind.as_str() {
                    "readdress" => &stack::READDRESSED,
                    "portal" => &["portal"],
                    _ => &stack::GAME_SERVERS,
                };
                cmd.args(["--no-deps", "--force-recreate"]).args(services);
            }
            // A first start downloads the images rather than building them.
            let wanted = images::current(&app.dir, &app.env().ok_or("Finish setup first.")?);
            // `up` builds any that couldn't be downloaded, from the current build files.
            let missing = wanted.iter().filter(|i| !images::exists(&i.local)).cloned().collect();
            let fetch = Step::FetchMissing(wanted);
            // A mod's database files can change items or monsters players
            // already have, so the database is backed up before they load.
            if mods::new_databases(&app.mods_dir()).is_empty() {
                vec![fetch, Step::Ports, Step::Compose(cmd), Step::Record(missing)]
            } else {
                vec![Step::Backup(backup::Reason::Mods), fetch, Step::Ports, Step::Compose(cmd), Step::Record(missing)]
            }
        }
        "build" => {
            let env = app.env().ok_or("Finish setup first.")?;
            // A new build can upgrade the database on its next start.
            vec![Step::Backup(backup::Reason::Rebuild), Step::Images(images::current(&app.dir, &env), cmd)]
        }
        "update" => {
            let env = app.env().ok_or("Finish setup first.")?;
            let update = update::pending(&env).ok_or("The server is already on this version's rAthena.")?;
            // The new commit goes into `.env` only once its image has built,
            // so a failed or cancelled build leaves the update on offer.
            cmd.env("RATHENA_REPO", &update.repo).env("RATHENA_COMMIT", &update.offered);
            let images = images::wanted(&app.dir, &env, &update.repo, &update.offered);
            // A running server stays on the old image until "Restart on the
            // new version" (or the next Start); players aren't dropped by surprise.
            vec![Step::Backup(backup::Reason::Update), Step::Images(images, cmd), Step::Pin(update)]
        }
        "stop" => {
            cmd.arg("down");
            vec![Step::Daily, Step::Compose(cmd)]
        }
        "backup" | "restore" | "test-restore" => {
            app.env().ok_or("Finish setup first.")?;
            match kind.as_str() {
                "backup" => vec![Step::Backup(backup::Reason::Manual)],
                "restore" => vec![Step::Restore(file.ok_or("Pick a backup to restore.")?)],
                _ => vec![Step::TestRestore(file.ok_or("Pick a backup to test.")?)],
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
                Step::Images(wanted, build) => images::fetch_or_build(&ctx, &wanted, build),
                Step::FetchMissing(wanted) => images::fetch_missing(&ctx, &wanted),
                Step::Ports => stack::check_ports(&ctx.dir),
                Step::Record(built) => {
                    images::record(&ctx.dir, &built);
                    Ok(())
                }
                Step::Backup(reason) => backup::create(&ctx, reason, None).map(|_| ()),
                Step::Daily => {
                    backup::daily(&ctx);
                    Ok(())
                }
                Step::Restore(file) => backup::restore(&ctx, &file),
                Step::TestRestore(file) => backup::test_restore(&ctx, &file),
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
            "start" | "restart" | "readdress" | "portal" => true,
            "restore" => stack::status(&ctx.dir)
                .unwrap_or_default()
                .iter()
                .any(|s| s.service == "map" && s.state == "running"),
            _ => false,
        };
        if ok && started {
            if let Ok(mut env) = EnvFile::load(&ctx.dir.join(".env")) {
                if record_address(&mut env) {
                    let _ = env.save();
                }
            }
            if let Err(e) = mods::record_applied(&ctx.dir.join(stack::MODS)) {
                ctx.line(format!("Couldn't record the loaded mods: {e}"));
            }
            let renewal = EnvFile::load(&ctx.dir.join(".env")).ok().and_then(|e| e.get("PRERE")).as_deref() == Some("no");
            if let Err(e) = customnpcs::record_applied(&ctx.dir, &ctx.dir.join(stack::OVERRIDES), renewal) {
                ctx.line(format!("Couldn't record the loaded custom NPCs: {e}"));
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

/// Where players reach the server: its address and the portal port, as
/// `ip:port`. Their patcher zip carries both (`clientinfo.xml` and the web
/// server settings the address, `rpatchur.yml` the portal's link).
fn player_address(env: &EnvFile) -> Option<String> {
    let ip = env.get("PUBLIC_IP").filter(|ip| !ip.is_empty())?;
    let port = env.get("PORTAL_PORT").filter(|p| !p.is_empty()).unwrap_or_else(|| "8080".into());
    Some(format!("{ip}:{port}"))
}

/// Notes where players reach the server it just started, in `LAST_ADDRESS`.
/// When that moved, the one players' files still have is kept in
/// `ADDRESS_CHANGED_FROM` until the warning is dismissed. Returns whether
/// anything changed.
fn record_address(env: &mut EnvFile) -> bool {
    let Some(now) = player_address(env) else {
        return false;
    };
    let last = env.get("LAST_ADDRESS").unwrap_or_default();
    if last == now {
        return false;
    }
    let from = env.get("ADDRESS_CHANGED_FROM").unwrap_or_default();
    if from == now {
        // Back where players' files point: nothing to tell them.
        env.set("ADDRESS_CHANGED_FROM", "");
    } else if from.is_empty() && !last.is_empty() {
        // A start before the app kept the address (last is empty) can't
        // tell whether it moved. A second move keeps the first address,
        // the one players who weren't told still have.
        env.set("ADDRESS_CHANGED_FROM", &last);
    }
    env.set("LAST_ADDRESS", &now);
    true
}

/// The change to warn about, from the old address to the one the server
/// last started on.
fn address_change(env: &EnvFile) -> Option<AddressChange> {
    let from = env.get("ADDRESS_CHANGED_FROM").filter(|a| !a.is_empty())?;
    let to = env.get("LAST_ADDRESS").filter(|a| !a.is_empty())?;
    let link = format!("http://{to}/connect");
    let ip = |a: &str| a.rsplit_once(':').map_or(a.to_owned(), |(ip, _)| ip.to_owned());
    let port = |a: &str| a.rsplit_once(':').map(|(_, p)| p.to_owned());
    // Only the address moved: the port would just be noise.
    if port(&from) == port(&to) {
        return Some(AddressChange { from: ip(&from), to: ip(&to), link });
    }
    Some(AddressChange { from, to, link })
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
    let (files, mods, patches, renewal) = (app.client_files.clone(), app.mods_dir(), app.patches_dir(), app.renewal());
    let name = settings::load(&app.overrides_dir()).server_name;
    tauri::async_runtime::spawn_blocking(move || clientpatch::status(&files, &mods::client_mods(&mods), &patches, renewal, &name))
        .await
        .map_err(|e| e.to_string())?
}

/// Publishes what changed in the client-files folder as a new patch. The
/// portal serves it straight away; the server doesn't need to restart.
#[tauri::command]
async fn publish_client_patch(app: State<'_, App>) -> Result<clientpatch::Publish, String> {
    let (files, mods, patches, lock, renewal) =
        (app.client_files.clone(), app.mods_dir(), app.patches_dir(), app.publishing.clone(), app.renewal());
    let name = settings::load(&app.overrides_dir()).server_name;
    tauri::async_runtime::spawn_blocking(move || {
        let _held = lock.try_lock().map_err(|_| "A patch is already being published.")?;
        clientpatch::publish(&files, &mods::client_mods(&mods), &patches, renewal, &name)
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

/// The Mods screen: the mod folders and rAthena's custom NPCs.
#[derive(Serialize)]
struct ModsPage {
    #[serde(flatten)]
    mods: mods::Status,
    custom_npcs: customnpcs::Status,
}

/// The mod folders and rAthena's custom NPCs, which are on, and which
/// changes wait for a restart. Reads the map server's log for errors, so it
/// runs off the main thread.
#[tauri::command]
async fn mods(app: State<'_, App>) -> Result<ModsPage, String> {
    let (dir, mods, env, cache) = (app.dir.clone(), app.mods_dir(), app.env(), app.clash_check.clone());
    let (client_files, renewal, overrides) = (app.client_files.clone(), app.renewal(), app.overrides_dir());
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
        let checked = check_clashes(&dir, &mods, env.as_ref(), &cache);
        let mut client_clashes: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for clash in clientpatch::clashes(&client_files, &mods::client_mods(&mods), renewal) {
            for name in clash.mods {
                client_clashes.entry(name).or_default().push(clash.text.clone());
            }
        }
        ModsPage {
            mods: mods::status(&mods, running, &log, checked.as_deref(), client_clashes),
            custom_npcs: customnpcs::status(&dir, &overrides, renewal, running, &log),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Runs `merge_mods.py --check` in the rAthena image on the mods as they
/// are now, so clashes show before a restart, when the enabled mods or
/// their database files changed since the last check. None when the image
/// isn't there or predates the check; the screen then shows the clashes
/// from the map server's latest start.
fn check_clashes(dir: &std::path::Path, mods: &std::path::Path, env: Option<&EnvFile>, cache: &ClashCheck) -> Option<String> {
    let env = env?;
    let image = images::current(dir, env).into_iter().next()?.local;
    let mode = if env.get("PRERE").as_deref() == Some("no") { "re" } else { "pre-re" };
    let key = format!("{image} {mode}\n{}", mods::clash_key(mods));
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((checked, output)) = cache.as_ref() {
        if *checked == key {
            return output.clone();
        }
    }
    let mut cmd = host::command("docker");
    cmd.args(["run", "--rm", "--network", "none", "--entrypoint", "merge_mods.py", "-v"])
        .arg(format!("{}:/rathena/mods:ro", mods.display()))
        .args([image.as_str(), "--check", "mods", mode]);
    let output = stack::output(cmd).ok();
    *cache = Some((key, output.clone()));
    output
}

/// Switches a mod on or off; it applies when the game servers restart.
#[tauri::command]
fn set_mod_enabled(app: State<App>, name: String, enabled: bool) -> Result<(), String> {
    mods::set_enabled(&app.mods_dir(), &name, enabled)
}

/// Switches one of rAthena's custom NPCs on or off, by its first script;
/// it applies when the game servers restart.
#[tauri::command]
fn set_custom_npc(app: State<App>, id: String, enabled: bool) -> Result<(), String> {
    customnpcs::set_enabled(&app.overrides_dir(), &id, enabled)
}

/// Moves an enabled mod one place earlier or later in the load order.
#[tauri::command]
fn move_mod(app: State<App>, name: String, up: bool) -> Result<(), String> {
    mods::move_mod(&app.mods_dir(), &name, up)
}

/// Starts a mod for custom items from the item kit's template and opens
/// its folder. It's switched off until the Mods screen switches it on.
#[tauri::command]
fn new_item_mod(app: State<App>, name: String) -> Result<(), String> {
    let root = itemkit::create(&app.mods_dir(), &name)?;
    host::open(root.as_os_str())
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
                clash_check: Arc::new(Mutex::new(None)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_url,
            overview,
            setup,
            set_public_ip,
            set_portal_port,
            save_build_settings,
            skip_server_update,
            dismiss_address_change,
            game_settings,
            save_game_settings,
            open_overrides,
            battle_conf,
            save_battle_conf,
            open_battle_conf,
            news,
            save_news,
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
            set_custom_npc,
            move_mod,
            open_mods,
            new_item_mod,
            follow_logs
        ])
        .on_window_event(|window, event| {
            // Only the main window's close stops the server; the battle
            // config editor just closes.
            if let (tauri::WindowEvent::CloseRequested { api, .. }, "main") = (event, window.label()) {
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
/// any build, start or restore test (a backup or restore is left to finish), backs up
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lib-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn keys(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('=').map(|(k, _)| k))
            .collect()
    }

    #[test]
    fn first_setup_writes_env_from_the_example() {
        let dir = temp_dir("setup");
        let path = dir.join(".env");
        write_setup(&path, || false, "admin", "secret_1", Some("10.8.0.1".into())).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let env = EnvFile::parse(&text);
        let example = EnvFile::parse(stack::ENV_EXAMPLE);
        // The example's settings and comments are kept, in their order.
        assert!(text.starts_with(stack::ENV_EXAMPLE.lines().next().unwrap()));
        assert_eq!(keys(&text)[..keys(stack::ENV_EXAMPLE).len()], keys(stack::ENV_EXAMPLE)[..]);

        let mut passwords = Vec::new();
        for key in ["DB_PASSWORD", "DB_ROOT_PASSWORD", "INTER_PASSWORD"] {
            let value = env.get(key).unwrap();
            assert_ne!(Some(&value), example.get(key).as_ref(), "{key} keeps the placeholder");
            // entrypoint.sh refuses anything else in INTER_PASSWORD.
            envfile::check_account_field(key, &value).unwrap();
            passwords.push(value);
        }
        passwords.sort();
        passwords.dedup();
        assert_eq!(passwords.len(), 3);

        assert_eq!(env.get("ADMIN_USER").as_deref(), Some("admin"));
        assert_eq!(env.get("ADMIN_PASSWORD").as_deref(), Some("secret_1"));
        assert_eq!(env.get("PUBLIC_IP").as_deref(), Some("10.8.0.1"));
        assert_eq!(env.get("PUBLIC_IP_AUTO").as_deref(), Some("no"));
        // A new install builds the bundled commit, so no update is offered.
        assert_eq!(env.get("RATHENA_COMMIT"), example.get("RATHENA_COMMIT"));
        assert!(update::pending(&env).is_none());

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn setup_keeps_the_passwords_an_existing_database_uses() {
        let dir = temp_dir("setup-db");
        let path = dir.join(".env");
        write_setup(&path, || true, "admin", "secret_1", None).unwrap();
        let env = EnvFile::load(&path).unwrap();
        let example = EnvFile::parse(stack::ENV_EXAMPLE);
        for key in ["DB_PASSWORD", "DB_ROOT_PASSWORD", "INTER_PASSWORD"] {
            assert_eq!(env.get(key), example.get(key));
        }

        // Running setup again only changes the admin account and address.
        let mut edited = EnvFile::load(&path).unwrap();
        edited.set("DB_PASSWORD", "kept");
        edited.save().unwrap();
        write_setup(&path, || panic!("not asked again"), "admin2", "secret_2", Some("192.168.0.5".into())).unwrap();
        let env = EnvFile::load(&path).unwrap();
        assert_eq!(env.get("DB_PASSWORD").as_deref(), Some("kept"));
        assert_eq!(env.get("ADMIN_USER").as_deref(), Some("admin2"));
        assert_eq!(env.get("PUBLIC_IP").as_deref(), Some("192.168.0.5"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn public_ip_follows_the_lan_unless_pinned() {
        let mut env = EnvFile::parse("PUBLIC_IP=127.0.0.1\n");
        apply_public_ip(&mut env, Some(" 10.8.0.1 ".into()));
        assert_eq!(env.get("PUBLIC_IP").as_deref(), Some("10.8.0.1"));
        assert_eq!(env.get("PUBLIC_IP_AUTO").as_deref(), Some("no"));

        apply_public_ip(&mut env, None);
        assert_eq!(env.get("PUBLIC_IP_AUTO").as_deref(), Some("yes"));
        let ip = env.get("PUBLIC_IP").unwrap();
        assert!(ip.parse::<std::net::Ipv4Addr>().is_ok(), "{ip}");
    }

    #[test]
    fn a_new_address_is_warned_about_until_dismissed() {
        // The first start the app sees only notes the address.
        let mut env = EnvFile::parse("PUBLIC_IP=192.168.0.5\n");
        assert!(record_address(&mut env));
        assert_eq!(env.get("LAST_ADDRESS").as_deref(), Some("192.168.0.5:8080"));
        assert_eq!(address_change(&env), None);
        assert!(!record_address(&mut env));

        // A start on another address warns.
        env.set("PUBLIC_IP", "192.168.0.7");
        assert!(record_address(&mut env));
        let change = AddressChange {
            from: "192.168.0.5".into(),
            to: "192.168.0.7".into(),
            link: "http://192.168.0.7:8080/connect".into(),
        };
        assert_eq!(address_change(&env), Some(change));

        // A second move keeps the address players were last given.
        env.set("PUBLIC_IP", "192.168.0.9");
        record_address(&mut env);
        assert_eq!(address_change(&env).unwrap().from, "192.168.0.5");
        assert_eq!(address_change(&env).unwrap().to, "192.168.0.9");

        // Moving back to it needs no warning.
        env.set("PUBLIC_IP", "192.168.0.5");
        record_address(&mut env);
        assert_eq!(address_change(&env), None);

        // Dismissing hides it, and the next start on the same address keeps it hidden.
        env.set("PUBLIC_IP", "10.8.0.1");
        record_address(&mut env);
        assert!(address_change(&env).is_some());
        env.set("ADDRESS_CHANGED_FROM", "");
        assert!(!record_address(&mut env));
        assert_eq!(address_change(&env), None);
    }

    #[test]
    fn a_new_portal_port_is_warned_about_with_the_port() {
        let mut env = EnvFile::parse("PUBLIC_IP=192.168.0.5\nPORTAL_PORT=8080\n");
        record_address(&mut env);
        env.set("PORTAL_PORT", "8090");
        assert!(record_address(&mut env));
        let change = AddressChange {
            from: "192.168.0.5:8080".into(),
            to: "192.168.0.5:8090".into(),
            link: "http://192.168.0.5:8090/connect".into(),
        };
        assert_eq!(address_change(&env), Some(change));
    }
}
