const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const MAX_LINES = 3000;

const LABELS = {
  db: "Database",
  "db-init": "Database setup",
  login: "Login server",
  char: "Char server",
  map: "Map server",
  web: "Web server",
  portal: "Account portal",
};

let overview = null;
let task = null; // "start", "restart", "readdress", "stop", "build", "update", "backup" or "restore" while one runs
let view = "activity"; // or a service name
let logGeneration = 0;
const activity = [];
let serviceLines = [];

// ---- Log pane ----

function render() {
  const lines = view === "activity" ? activity : serviceLines;
  const log = $("log");
  const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
  log.textContent = lines.join("\n");
  if (atBottom) log.scrollTop = log.scrollHeight;
}

let renderQueued = false;
function queueRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    render();
  });
}

function push(lines, line) {
  lines.push(line);
  if (lines.length > MAX_LINES) lines.splice(0, lines.length - MAX_LINES);
}

async function showView(next) {
  view = next;
  $("tab-activity").classList.toggle("active", view === "activity");
  $("log-title").textContent = view === "activity" ? "" : `Logs: ${LABELS[view] || view}`;
  serviceLines = [];
  const generation = await invoke("follow_logs", { service: view === "activity" ? null : view });
  if (generation > logGeneration) {
    logGeneration = generation;
    serviceLines = [];
  }
  renderServices(lastServices);
  render();
}

// ---- Build progress ----

// BuildKit's plain output names each step as "#12 [build 4/6] RUN ...".
function trackProgress(line) {
  const m = line.match(/^#\d+ \[(?:[\w-]+ )?build (\d+)\/(\d+)\] (.*)/);
  if (!m) return;
  const [, step, total, what] = m;
  $("progress").hidden = false;
  $("progress-bar").style.width = `${(100 * (step - 1)) / total}%`;
  $("progress-text").textContent = `Building rAthena, step ${step} of ${total}: ${what.slice(0, 60)}`;
}

function clearProgress() {
  $("progress").hidden = true;
  $("progress-text").textContent = "";
}

// ---- Services ----

let lastServices = [];

function describe(s) {
  if (s.state === "absent") return ["off", "Not created"];
  if (s.state === "running") {
    if (s.health === "starting") return ["busy", "Starting"];
    if (s.health === "unhealthy") return ["bad", "Unhealthy"];
    return ["ok", "Running"];
  }
  if (s.state === "restarting") return ["bad", "Restarting"];
  if (s.state === "exited" && s.service === "db-init") {
    return s.exit_code === 0 ? ["ok", "Done"] : ["bad", `Failed (exit ${s.exit_code})`];
  }
  if (s.state === "exited") return s.exit_code === 0 ? ["off", "Stopped"] : ["bad", `Stopped (exit ${s.exit_code})`];
  return ["busy", s.state];
}

// `docker compose config --services` doesn't keep a stable order.
const ORDER = Object.keys(LABELS);
const rank = (name) => (ORDER.includes(name) ? ORDER.indexOf(name) : ORDER.length);

function renderServices(list) {
  list = [...list].sort((a, b) => rank(a.service) - rank(b.service));
  lastServices = list;
  const ul = $("services");
  ul.replaceChildren(
    ...list.map((s) => {
      const [kind, text] = describe(s);
      const li = document.createElement("li");
      li.className = s.service === view ? "selected" : "";
      li.title = s.status || "";
      li.innerHTML = `<span class="dot ${kind}"></span><span class="name"></span><span class="state muted"></span>`;
      li.querySelector(".name").textContent = LABELS[s.service] || s.service;
      li.querySelector(".state").textContent = text + (s.ports.length ? ` · ${s.ports.join(", ")}` : "");
      li.addEventListener("click", () => showView(s.service));
      return li;
    }),
  );

  const portal = list.find((s) => s.service === "portal" && s.state === "running" && s.ports.length);
  $("portal").hidden = !portal;
  if (portal) $("portal-link").dataset.url = `http://${overview.public_ip}:${portal.ports[0]}/`;
}

async function refreshServices() {
  if (!overview?.configured || overview.docker_error) return;
  try {
    renderServices(await invoke("services"));
  } catch (e) {
    banner(`Couldn't read the server's status: ${e}`);
  }
}

// ---- Live view: who's online, uptime, CPU and memory ----

let liveBusy = false;

const percent = (n) => `${n < 10 ? n.toFixed(1) : Math.round(n)}%`;

function duration(secs) {
  if (secs < 60) return "under a minute";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d) return `${d} d ${h} h`;
  if (h) return `${h} h ${m} min`;
  return `${m} min`;
}

function row(cells) {
  const tr = document.createElement("tr");
  for (const [text, num] of cells) {
    const td = document.createElement("td");
    td.textContent = text;
    if (num) td.className = "num";
    tr.append(td);
  }
  return tr;
}

function renderLive(live) {
  const players = live.players || [];
  const cpu = live.usage.reduce((t, u) => t + u.cpu, 0);
  const mem = live.usage.reduce((t, u) => t + u.memory, 0);
  const parts = [];
  if (live.players) parts.push(players.length === 1 ? "1 player" : `${players.length} players`);
  if (live.uptime != null) parts.push(`up ${duration(live.uptime)}`);
  $("online-summary").textContent = parts.join(" · ");

  $("players").hidden = players.length === 0;
  $("no-players").hidden = players.length > 0;
  $("no-players").textContent = live.players
    ? "Nobody is online."
    : serverRunning()
      ? "Players show here once the char server is up."
      : "The server is off.";
  $("players").tBodies[0].replaceChildren(
    ...players.map((p) => row([[p.name], [p.class], [p.base_level, true], [p.job_level, true], [p.map]])),
  );

  const usage = [...live.usage].sort((a, b) => rank(a.service) - rank(b.service));
  $("usage").hidden = usage.length === 0;
  $("no-usage").hidden = usage.length > 0;
  $("usage").tBodies[0].replaceChildren(
    ...usage.map((u) => row([[LABELS[u.service] || u.service], [percent(u.cpu), true], [size(u.memory), true]])),
  );
  $("usage").tFoot.replaceChildren(row([["Total"], [percent(cpu), true], [size(mem), true]]));
}

// Only while the Dashboard page is showing; `docker stats` isn't free.
async function refreshLive() {
  if (!overview?.configured || overview.docker_error || page !== "live" || document.hidden || liveBusy) return;
  liveBusy = true;
  try {
    renderLive(await invoke("live"));
  } catch (e) {
    $("online-summary").textContent = `Couldn't read the live view: ${e}`;
  } finally {
    liveBusy = false;
  }
}

// ---- Overview, setup and settings ----

function banner(text) {
  $("banner").hidden = !text;
  $("banner").textContent = text || "";
}

// ---- Address players connect to: the LAN, a VPN, or a typed one ----

// What "Detect automatically" picks: never a VPN address.
const detectedLan = () => overview.addresses.find((a) => !a.vpn)?.ip;
const vpnOf = (ip) => overview.addresses.find((a) => a.ip === ip)?.vpn;

function fillAddresses(select, current, auto) {
  const option = (value, label) => {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = label;
    return o;
  };
  const group = (label, options) => {
    const g = document.createElement("optgroup");
    g.label = label;
    g.append(...options);
    return g;
  };
  const detected = detectedLan();
  const lan = overview.addresses.filter((a) => !a.vpn);
  const vpn = overview.addresses.filter((a) => a.vpn);
  const known = [...overview.addresses.map((a) => a.ip), "127.0.0.1"];
  select.replaceChildren(
    option("auto", `Detect automatically${detected ? ` (now ${detected})` : ""}`),
    ...(lan.length ? [group("Home network", lan.map((a) => option(a.ip, `${a.ip} (${a.interface})`)))] : []),
    ...(vpn.length ? [group("VPN", vpn.map((a) => option(a.ip, `${a.ip} (${a.vpn})`)))] : []),
    group("Other", [
      option("127.0.0.1", "127.0.0.1 (this computer only)"),
      ...(current && !auto && !known.includes(current) ? [option(current, `${current} (typed)`)] : []),
      option("other", "Type an address…"),
    ]),
  );
  select.value = auto || !current ? "auto" : current;
}

// The catch with one address for everyone: rAthena sends every player the
// same char and map address, so it has to be one all of them can reach.
function addressNote(value) {
  const vpns = [...new Set(overview.addresses.filter((a) => a.vpn).map((a) => a.vpn))];
  const vpn = vpnOf(value);
  if (vpn)
    return `Everyone connects through ${vpn} with this address, players at home too: the game sends every player the same address, so anyone not on ${vpn} can't get past the server list.`;
  if (value === "127.0.0.1") return "Only the game client on this computer can connect.";
  if (value === "other") return "The game sends every player this one address, so use one all of them can reach.";
  if (vpns.length)
    return `Friends on ${vpns.join(" or ")} can't connect with a home network address. Pick the ${vpns[0]} address to let them in; players at home then join ${vpns[0]} too.`;
  return "";
}

function showNote(id, value) {
  const note = addressNote(value);
  $(id).textContent = note;
  $(id).hidden = !note;
}

async function refreshOverview() {
  overview = await invoke("overview");
  task = overview.task;

  $("docker").textContent = overview.docker_version ? `Docker Compose ${overview.docker_version}` : "";
  banner(overview.docker_error);

  $("setup").hidden = overview.configured;
  $("nav").hidden = !overview.configured;
  $("nav-backups").hidden = !overview.configured;
  $("nav-live").hidden = !overview.configured;
  showPage(overview.configured ? page : null);
  if (!overview.configured) {
    if ($("setup-ip").value !== "other") {
      fillAddresses($("setup-ip"), null, true);
      showNote("setup-ip-note", "auto");
    }
  } else {
    if (pendingIp === undefined && $("ip").value !== "other") {
      fillAddresses($("ip"), overview.public_ip, overview.public_ip_auto);
      showNote("ip-note", $("ip").value);
    }
    $("connect-addr").textContent = `${overview.public_ip}:6900`;
    $("admin-name").textContent = overview.admin_user;
    if (!buildFormDirty) fillBuildForm();
  }
  renderUpdate(overview.configured && overview.server_update);
  $("restart-bar").hidden = !overview.configured || !overview.restart_pending || !!overview.server_update;
  updateButtons();
}

// ---- Server update: the rAthena commit this app version comes with ----

const short = (sha) => (sha ? sha.slice(0, 7) : "none");

function renderUpdate(update) {
  $("update-bar").hidden = !update;
  if (!update) return;
  $("update-to").textContent = short(update.offered);
  $("update-from").textContent = short(update.current);
  $("update-changes").hidden = !update.changes_url;
  $("update-changes").dataset.url = update.changes_url || "";
}

$("update-changes").addEventListener("click", (e) => {
  e.preventDefault();
  invoke("plugin:opener|open_url", { url: e.target.dataset.url });
});

$("skip-update").addEventListener("click", async () => {
  try {
    await invoke("skip_server_update");
  } catch (err) {
    banner(String(err));
  }
  await refreshOverview();
});

$("restart-new").addEventListener("click", () => {
  showPage("dashboard");
  runTask("restart");
});

$("update").addEventListener("click", () => {
  showPage("dashboard");
  runTask("update");
});

function updateButtons() {
  const ready = overview?.configured && !overview.docker_error;
  for (const id of ["start", "stop", "build", "build-save", "restart", "backup-now", "update", "skip-update", "restart-new", "ip-yes"])
    $(id).disabled = !ready || !!task;
  for (const b of document.querySelectorAll("#backups button")) b.disabled = !ready || !!task;
  $("cancel").hidden = !["build", "update", "start", "restart", "readdress"].includes(task);
}

// ---- Pages: the main screen, Dashboard, Settings and Backups ----

let page = "dashboard";

function showPage(next) {
  if (next) page = next;
  $("dashboard").hidden = next !== "dashboard";
  $("settings").hidden = next !== "settings";
  $("backups-page").hidden = next !== "backups";
  $("live-page").hidden = next !== "live";
  $("nav").textContent = page === "settings" ? "Back to server" : "Settings";
  $("nav-backups").textContent = page === "backups" ? "Back to server" : "Backups";
  $("nav-live").textContent = page === "live" ? "Back to server" : "Dashboard";
  if (next === "live") refreshLive();
}

$("nav").addEventListener("click", async () => {
  if (page === "settings") return showPage("dashboard");
  $("game-saved").hidden = true;
  $("game-error").hidden = true;
  await loadGameSettings();
  showPage("settings");
});

$("nav-live").addEventListener("click", () => showPage(page === "live" ? "dashboard" : "live"));

$("nav-backups").addEventListener("click", async () => {
  if (page === "backups") return showPage("dashboard");
  $("backup-error").hidden = true;
  await refreshBackups();
  showPage("backups");
});

const serverRunning = () => lastServices.some((s) => ["login", "char", "map"].includes(s.service) && s.state === "running");

// ---- Game settings: rAthena overrides, applied on restart ----

const RATE_FIELDS = ["base_exp", "job_exp", "drop_common", "drop_equip", "drop_card", "drop_mvp", "death_penalty"];
let gameLocked = false;

async function loadGameSettings() {
  const s = await invoke("game_settings");
  for (const f of RATE_FIELDS) $(f).value = s[f];
  $("server_name").value = s.server_name;
  $("motd").value = s.motd;
  $("player_commands").checked = s.player_commands;
  gameLocked = s.player_commands_locked;
  $("player_commands").disabled = gameLocked;
  $("commands-locked").hidden = !gameLocked;
}

$("game-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const error = $("game-error");
  error.hidden = true;
  $("game-saved").hidden = true;
  const settings = {
    server_name: $("server_name").value,
    motd: $("motd").value,
    player_commands: $("player_commands").checked,
    player_commands_locked: gameLocked,
  };
  for (const f of RATE_FIELDS) settings[f] = Number($(f).value);
  let changed;
  try {
    changed = await invoke("save_game_settings", { settings });
  } catch (err) {
    error.textContent = err;
    error.hidden = false;
    return;
  }
  await loadGameSettings();
  if (!changed) return;
  $("game-saved").hidden = false;
  $("restart").hidden = !serverRunning();
});

$("restart").addEventListener("click", async () => {
  $("game-saved").hidden = true;
  showPage("dashboard");
  await runTask("restart");
});

$("open-overrides").addEventListener("click", async () => {
  try {
    await invoke("open_overrides");
  } catch (err) {
    $("game-error").textContent = `Couldn't open the folder: ${err}`;
    $("game-error").hidden = false;
  }
});

// ---- Server build: PACKETVER and renewal, saved with a rebuild ----

const PRESETS = ["20250716", "20220406"];

function chosenPacketver() {
  const preset = $("packetver-preset").value;
  return preset === "custom" ? $("packetver").value.trim() : preset;
}

function fillBuildForm() {
  const custom = !PRESETS.includes(overview.packetver);
  $("packetver-preset").value = custom ? "custom" : overview.packetver;
  $("packetver").value = overview.packetver;
  $("mode").value = overview.prere ? "pre" : "re";
  updateBuildForm();
}

// The warning shows as soon as the form differs from the current build.
function updateBuildForm() {
  const custom = $("packetver-preset").value === "custom";
  $("packetver-custom").hidden = !custom;
  $("packetver").required = custom;
  const changed = chosenPacketver() !== overview.packetver || ($("mode").value === "pre") !== overview.prere;
  $("build-warning").hidden = !changed;
}

$("setup-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const error = $("setup-error");
  error.hidden = true;
  if ($("admin-pass").value !== $("admin-pass2").value) {
    error.textContent = "The passwords don't match.";
    error.hidden = false;
    return;
  }
  const choice = $("setup-ip").value;
  const ip = choice === "other" ? $("setup-ip-typed").value.trim() : choice;
  try {
    await invoke("setup", {
      adminUser: $("admin-user").value,
      adminPassword: $("admin-pass").value,
      publicIp: ip === "auto" ? null : ip,
    });
  } catch (err) {
    error.textContent = err;
    error.hidden = false;
    return;
  }
  await refreshOverview();
  await refreshServices();
});

$("setup-ip").addEventListener("change", () => {
  const other = $("setup-ip").value === "other";
  $("setup-ip-other").hidden = !other;
  if (other) $("setup-ip-typed").focus();
  showNote("setup-ip-note", $("setup-ip").value);
});

// The address picked while the server runs, waiting for "Change and
// restart": null for automatic, undefined when nothing is waiting.
let pendingIp;

function cancelAddress() {
  pendingIp = undefined;
  $("ip-confirm").hidden = true;
  $("ip-other").hidden = true;
  $("ip-typed").value = "";
  fillAddresses($("ip"), overview.public_ip, overview.public_ip_auto);
  showNote("ip-note", $("ip").value);
}

async function saveAddress(ip, restart) {
  $("ip-error").hidden = true;
  let changed = false;
  try {
    changed = await invoke("set_public_ip", { publicIp: ip });
  } catch (err) {
    $("ip-error").textContent = err;
    $("ip-error").hidden = false;
    return false;
  }
  pendingIp = undefined;
  $("ip-confirm").hidden = true;
  $("ip-other").hidden = true;
  $("ip-typed").value = "";
  await refreshOverview();
  if (changed && restart) {
    showPage("dashboard");
    await runTask("readdress");
  }
  return true;
}

async function chooseAddress(ip) {
  // A pinned address that's already the one in use needs no restart.
  const same = (ip ?? detectedLan() ?? "127.0.0.1") === overview.public_ip;
  if (!serverRunning() || same) return saveAddress(ip, false);
  pendingIp = ip;
  $("ip-error").hidden = true;
  $("ip-confirm").hidden = false;
}

$("ip").addEventListener("change", () => {
  const value = $("ip").value;
  showNote("ip-note", value);
  $("ip-confirm").hidden = true;
  pendingIp = undefined;
  $("ip-other").hidden = value !== "other";
  if (value === "other") {
    $("ip-typed").focus();
    return;
  }
  chooseAddress(value === "auto" ? null : value);
});

$("ip-use").addEventListener("click", () => chooseAddress($("ip-typed").value.trim()));
$("ip-typed").addEventListener("keydown", (e) => {
  if (e.key === "Enter") chooseAddress($("ip-typed").value.trim());
  if (e.key === "Escape") cancelAddress();
});
$("ip-yes").addEventListener("click", () => saveAddress(pendingIp, true));
$("ip-no").addEventListener("click", cancelAddress);

let buildFormDirty = false;
let restartAfterBuild = false;
for (const [id, event] of [["packetver-preset", "change"], ["packetver", "input"], ["mode", "change"]]) {
  $(id).addEventListener(event, () => {
    buildFormDirty = true;
    updateBuildForm();
  });
}

$("build-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const error = $("build-error");
  error.hidden = true;
  let changed;
  try {
    changed = await invoke("save_build_settings", {
      packetver: chosenPacketver(),
      prere: $("mode").value === "pre",
    });
  } catch (err) {
    error.textContent = err;
    error.hidden = false;
    return;
  }
  buildFormDirty = false;
  await refreshOverview();
  if (!changed) return;
  // A running server is restarted on the new build once it's ready.
  restartAfterBuild = lastServices.some((s) => s.state === "running");
  showPage("dashboard");
  await runTask("build");
});

$("portal-link").addEventListener("click", (e) => {
  e.preventDefault();
  invoke("plugin:opener|open_url", { url: e.target.dataset.url });
});

// ---- Backups ----

const REASONS = {
  manual: "Backed up by hand",
  rebuild: "Before a rebuild",
  update: "Before a server update",
  "before-restore": "Before a restore",
  daily: "Daily, on stop or close",
};

function size(bytes) {
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(1)} MB`;
  if (bytes >= 1 << 10) return `${Math.round(bytes / (1 << 10))} KB`;
  return `${bytes} bytes`;
}

let confirming = null; // the backup whose restore is being confirmed

async function refreshBackups() {
  if (!overview?.configured) return;
  const list = await invoke("backups");
  $("no-backups").hidden = list.length > 0;
  $("backups").replaceChildren(
    ...list.map((b) => {
      const li = document.createElement("li");
      li.innerHTML = `<span><span class="when"></span><br><span class="what muted"></span></span><button>Restore</button>`;
      li.querySelector(".when").textContent = b.created.slice(0, 16);
      li.querySelector(".what").textContent = `${REASONS[b.reason] || b.reason} · ${size(b.size)}`;
      li.title = b.file;
      li.querySelector("button").addEventListener("click", () => {
        confirming = confirming === b.file ? null : b.file;
        refreshBackups();
      });
      if (confirming === b.file) {
        const p = document.createElement("p");
        p.className = "confirm notice";
        p.textContent =
          "Replace the database with this backup? Everything since then is lost: accounts, characters, items. " +
          "The servers stop while it loads; the current database is backed up first.";
        const yes = document.createElement("button");
        yes.className = "primary";
        yes.textContent = "Restore";
        yes.addEventListener("click", () => {
          confirming = null;
          li.querySelector(".confirm").remove();
          showPage("dashboard");
          runTask("restore", b.file);
        });
        const no = document.createElement("button");
        no.textContent = "Cancel";
        no.addEventListener("click", () => {
          confirming = null;
          refreshBackups();
        });
        p.append(document.createElement("br"), yes, no);
        li.append(p);
      }
      return li;
    }),
  );
  updateButtons();
}

// Progress shows in the Activity log on the main screen.
$("backup-now").addEventListener("click", () => {
  showPage("dashboard");
  runTask("backup");
});

$("open-backups").addEventListener("click", async () => {
  $("backup-error").hidden = true;
  try {
    await invoke("open_backups");
  } catch (err) {
    $("backup-error").textContent = `Couldn't open the folder: ${err}`;
    $("backup-error").hidden = false;
  }
});

// ---- Tasks ----

async function runTask(kind, file) {
  activity.length = 0;
  clearProgress();
  await showView("activity");
  try {
    await invoke("run_task", { kind, file: file ?? null });
  } catch (err) {
    push(activity, String(err));
    render();
  }
}

$("start").addEventListener("click", () => runTask("start"));
$("stop").addEventListener("click", () => runTask("stop"));
$("build").addEventListener("click", () => runTask("build"));
$("cancel").addEventListener("click", () => invoke("cancel_task"));
$("tab-activity").addEventListener("click", () => showView("activity"));

listen("task-line", ({ payload }) => {
  trackProgress(payload);
  push(activity, payload);
  if (view === "activity") queueRender();
});

listen("task", async ({ payload }) => {
  task = payload.running ? payload.kind : null;
  updateButtons();
  if (!payload.running) {
    clearProgress();
    const verb = { start: "Start", restart: "Restart", readdress: "Address change", stop: "Stop", build: "Build", update: "Server update", backup: "Backup", restore: "Restore" }[
      payload.kind
    ];
    push(activity, payload.ok ? `\n${verb} finished.` : `\n${verb} failed. See the output above.`);
    if (view === "activity") render();
    await refreshOverview();
    await refreshServices();
    await refreshBackups();
    refreshLive();
    if (payload.kind === "build" && restartAfterBuild) {
      restartAfterBuild = false;
      if (payload.ok) await runTask("start");
    }
  }
});

listen("log-line", ({ payload }) => {
  // A newer stream may start sending before follow_logs has returned.
  if (payload.generation < logGeneration) return;
  if (payload.generation > logGeneration) {
    logGeneration = payload.generation;
    serviceLines = [];
  }
  if (view === "activity") return;
  push(serviceLines, payload.line);
  queueRender();
});

// Closing the window stops the server first; show its output meanwhile.
listen("closing", async () => {
  task = "stop";
  updateButtons();
  banner("Stopping the server before closing…");
  showPage("dashboard");
  activity.length = 0;
  await showView("activity");
});

refreshOverview().then(refreshServices).then(refreshBackups).then(refreshLive);
setInterval(refreshServices, 3000);
setInterval(refreshLive, 5000);
document.addEventListener("visibilitychange", refreshLive);
