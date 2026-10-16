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
let task = null; // "start", "restart", "readdress", "portal", "stop", "build", "update", "backup", "restore" or "test-restore" while one runs
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
  $("nav-patch").hidden = !overview.configured;
  $("nav-mods").hidden = !overview.configured;
  $("nav-news").hidden = !overview.configured;
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
    if (document.activeElement !== $("portal-port")) $("portal-port").value = overview.portal_port;
    $("admin-name").textContent = overview.admin_user;
    if (!buildFormDirty) fillBuildForm();
  }
  renderAddressChange(overview.configured && overview.address_change);
  renderUpdate(overview.configured && overview.server_update);
  // One bar at a time, in the order they're dealt with: update, rebuild, restart.
  const rebuild = overview.configured && !overview.server_update && overview.rebuild;
  renderRebuild(rebuild);
  const restart = overview.restart_pending || overview.portal_restart_pending;
  $("restart-bar").hidden = !overview.configured || !restart || !!overview.server_update || !!rebuild;
  $("restart-server").hidden = !overview.restart_pending;
  $("restart-portal").hidden = overview.restart_pending;
  $("restart-new").textContent = overview.restart_pending ? "Restart on the new build" : "Restart the portal";
  renderVersion(overview.configured && overview.server_version);
  updateButtons();
}

// ---- Server version: going back to the rAthena commit before this one ----

function renderVersion(version) {
  if (!version) return;
  const previous = version.previous;
  $("version-current").textContent = short(version.current);
  $("version-previous").hidden = !previous;
  $("version-changes").hidden = !previous?.changes_url;
  $("version-changes").dataset.url = previous?.changes_url || "";
  $("go-back").hidden = !previous || !$("go-back-confirm").hidden;
  $("no-previous").hidden = !!previous;
  if (!previous) $("go-back-confirm").hidden = true;
  else {
    $("version-previous-commit").textContent = short(previous.commit);
    $("go-back").textContent = `Go back to ${short(previous.commit)}`;
    $("go-back-to").textContent = short(previous.commit);
  }
}

$("version-changes").addEventListener("click", (e) => {
  e.preventDefault();
  invoke("open_url", { url: e.target.dataset.url });
});

$("go-back").addEventListener("click", () => {
  $("go-back-confirm").hidden = false;
  $("go-back").hidden = true;
});

$("go-back-no").addEventListener("click", () => {
  $("go-back-confirm").hidden = true;
  $("go-back").hidden = false;
});

$("go-back-yes").addEventListener("click", () => {
  $("go-back-confirm").hidden = true;
  showPage("dashboard");
  runTask("go-back");
});

// ---- Rebuild notice: images here that Start won't bring up to date ----

function renderRebuild(rebuild) {
  $("rebuild-bar").hidden = !rebuild;
  if (!rebuild) return;
  $("rebuild-title").textContent = rebuild.needed ? "Rebuild needed." : "Rebuild recommended.";
  const parts = [rebuild.server && "the game server", rebuild.portal && "the account portal"].filter(Boolean).join(" and ");
  $("rebuild-why").textContent = rebuild.needed
    ? "The game server was built for other build settings than the ones saved, for example because a rebuild failed or was cancelled."
    : `This version of the app changed ${parts}, and there was no ready-made version to download, so yours is still the old one.`;
}

$("rebuild-now").addEventListener("click", () => {
  showPage("dashboard");
  runTask("build");
});

// ---- Address change: players' files still point at the old address ----

// Shown next to the other bars, since it's about players, not the server.
function renderAddressChange(change) {
  $("address-bar").hidden = !change;
  if (!change) return;
  $("address-from").textContent = change.from;
  $("address-to").textContent = change.to;
  // A pinned address moves only when picked by hand.
  $("address-hint").hidden = !overview.public_ip_auto;
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    // Not allowed here; copy through a selected field instead.
  }
  const field = document.createElement("textarea");
  field.value = text;
  document.body.append(field);
  field.select();
  const copied = document.execCommand("copy");
  field.remove();
  if (!copied) throw new Error(`Couldn't copy. The link is ${text}`);
}

$("address-copy").addEventListener("click", async () => {
  try {
    await copyText(overview.address_change.link);
    $("address-copy").textContent = "Copied";
    setTimeout(() => ($("address-copy").textContent = "Copy the link"), 2000);
  } catch (err) {
    banner(String(err.message || err));
  }
});

$("address-dismiss").addEventListener("click", async () => {
  try {
    await invoke("dismiss_address_change");
  } catch (err) {
    banner(String(err));
  }
  await refreshOverview();
});

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
  invoke("open_url", { url: e.target.dataset.url });
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
  for (const id of ["start", "stop", "build", "build-save", "restart", "mods-restart", "mods-rebuild-button", "backup-now", "update", "skip-update", "restart-new", "go-back", "go-back-yes", "ip-yes", "portal-port-use"])
    $(id).disabled = !ready || !!task;
  for (const b of document.querySelectorAll("#backups button")) b.disabled = !ready || !!task;
  $("rebuild-now").disabled = !ready || !!task;
  $("cancel").hidden = !["build", "update", "go-back", "start", "restart", "readdress", "portal"].includes(task);
}

// ---- Pages: the main screen, Dashboard, Settings, Backups, Client patch, Mods and News ----

let page = "dashboard";

function showPage(next) {
  if (next) page = next;
  $("dashboard").hidden = next !== "dashboard";
  $("settings").hidden = next !== "settings";
  $("backups-page").hidden = next !== "backups";
  $("live-page").hidden = next !== "live";
  $("patch-page").hidden = next !== "patch";
  $("mods-page").hidden = next !== "mods";
  $("news-page").hidden = next !== "news";
  $("nav").textContent = page === "settings" ? "Back to server" : "Settings";
  $("nav-backups").textContent = page === "backups" ? "Back to server" : "Backups";
  $("nav-live").textContent = page === "live" ? "Back to server" : "Dashboard";
  $("nav-patch").textContent = page === "patch" ? "Back to server" : "Client patch";
  $("nav-mods").textContent = page === "mods" ? "Back to server" : "Mods";
  $("nav-news").textContent = page === "news" ? "Back to server" : "News";
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

let gameLocked = false;

async function loadGameSettings() {
  const s = await invoke("game_settings");
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

$("open-battle-conf").addEventListener("click", async () => {
  try {
    await invoke("open_battle_conf");
  } catch (err) {
    $("game-error").textContent = `Couldn't open the battle config: ${err}`;
    $("game-error").hidden = false;
  }
});

// The battle config window's "Restart game servers", run here so it shows
// in the activity log.
listen("restart-game-servers", async () => {
  if (task) return;
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

// A new portal port restarts only the portal; nobody in game is affected.
async function savePortalPort() {
  const error = $("portal-port-error");
  error.hidden = true;
  let changed = false;
  try {
    changed = await invoke("set_portal_port", { port: Number($("portal-port").value) });
  } catch (err) {
    error.textContent = err;
    error.hidden = false;
    return;
  }
  $("portal-port").blur();
  await refreshOverview();
  // The portal may be the one service that didn't start, held off its old port.
  if (changed && lastServices.some((s) => s.state === "running")) {
    showPage("dashboard");
    await runTask("portal");
  }
}

$("portal-port-use").addEventListener("click", savePortalPort);
$("portal-port").addEventListener("keydown", (e) => {
  if (e.key === "Enter") savePortalPort();
});

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
  invoke("open_url", { url: e.target.dataset.url });
});

// ---- Backups ----

const REASONS = {
  manual: "Backed up by hand",
  rebuild: "Before a rebuild",
  update: "Before a server update",
  "go-back": "Before going back to the previous server version",
  "before-restore": "Before a restore",
  mods: "Before loading a mod's database files",
  daily: "Daily, on stop or close",
};

function size(bytes) {
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(1)} MB`;
  if (bytes >= 1 << 10) return `${Math.round(bytes / (1 << 10))} KB`;
  return `${bytes} bytes`;
}

let confirming = null; // the backup whose restore is being confirmed

function plural(n, one, many) {
  return `${n} ${n === 1 ? one : many}`;
}

// "Tested 2026-10-15: loads, 12 accounts, 40 characters"
function testResult(t) {
  if (t.problem) return `Restore test ${t.tested} failed: ${t.problem}`;
  const c = t.counts;
  return `Tested ${t.tested}: loads, ${plural(c.accounts, "account", "accounts")}, ${plural(c.characters, "character", "characters")}`;
}

async function refreshBackups() {
  if (!overview?.configured) return;
  const list = await invoke("backups");
  $("no-backups").hidden = list.length > 0;
  $("backups").replaceChildren(
    ...list.map((b) => {
      const li = document.createElement("li");
      li.innerHTML =
        `<span><span class="when"></span><br><span class="what muted"></span><span class="tested"></span></span>` +
        `<span class="actions"><button class="test" title="Load it into a throwaway database to check it would restore. The live database isn't touched.">Test</button><button class="restore">Restore</button></span>`;
      li.querySelector(".when").textContent = b.created.slice(0, 16);
      li.querySelector(".what").textContent = `${REASONS[b.reason] || b.reason} · ${size(b.size)}`;
      if (b.test) {
        const tested = li.querySelector(".tested");
        tested.before(document.createElement("br"));
        tested.textContent = testResult(b.test);
        tested.className = b.test.problem ? "tested error" : "tested muted";
      }
      li.title = b.file;
      li.querySelector(".test").addEventListener("click", () => {
        confirming = null;
        showPage("dashboard");
        runTask("test-restore", b.file);
      });
      li.querySelector(".restore").addEventListener("click", () => {
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
        if (b.test?.problem) {
          const failed = document.createElement("span");
          failed.className = "error";
          failed.textContent = ` This backup failed its restore test on ${b.test.tested}: ${b.test.problem}`;
          p.append(failed);
        }
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

// ---- Client patch: publishing files to the players' patcher ----

const SHOWN = 50; // files listed per change kind before "and N more"

function fileItems(changes) {
  const items = [];
  for (const [kind, label] of [["added", "New"], ["changed", "Changed"], ["removed", "Removed"]]) {
    for (const path of changes[kind].slice(0, SHOWN)) {
      const li = document.createElement("li");
      li.innerHTML = `<span class="kind muted"></span><code></code>`;
      li.querySelector(".kind").textContent = label;
      li.querySelector("code").textContent = path;
      items.push(li);
    }
    if (changes[kind].length > SHOWN) {
      const li = document.createElement("li");
      li.className = "more muted";
      li.textContent = `and ${changes[kind].length - SHOWN} more ${label.toLowerCase()}`;
      items.push(li);
    }
  }
  return items;
}

const count = (n, one, many) => `${n} ${n === 1 ? one : many}`;

// A mod that changes thousands of entries would make a page-long list.
const firstLines = (lines, max = 100) =>
  lines.slice(0, max).join("\n\n") + (lines.length > max ? `\n\n…and ${lines.length - max} more` : "");

function summary(c) {
  const parts = [];
  if (c.added.length) parts.push(`${c.added.length} new`);
  if (c.changed.length) parts.push(`${c.changed.length} changed`);
  if (c.removed.length) parts.push(`${c.removed.length} removed`);
  return parts.join(", ");
}

let publishing = false;

async function refreshPatch() {
  let s;
  try {
    s = await invoke("client_patch");
  } catch (err) {
    $("patch-error").textContent = String(err);
    $("patch-error").hidden = false;
    return;
  }
  $("patch-folder").textContent = `Folder: ${s.folder} · Game mode: ${s.mode}`;
  $("patch-problems").hidden = s.problems.length === 0;
  $("patch-problems").replaceChildren(
    ...s.problems.map((p) => {
      const li = document.createElement("li");
      li.textContent = p;
      return li;
    }),
  );
  const c = s.changes;
  const total = c.added.length + c.changed.length + c.removed.length;
  const switched = s.switched_from && total
    ? `The server switched from ${s.switched_from} to ${s.mode}: publish to give players its files. `
    : "";
  $("patch-summary").textContent = switched + (s.problems.length
    ? "Fix the files above to publish."
    : total
      ? `${count(total, "file", "files")} to publish: ${summary(c)}.`
      : s.history.length
        ? "Nothing has changed since the last patch."
        : "The client files folder is empty.");
  $("patch-changes").replaceChildren(...fileItems(c));
  $("patch-mods").hidden = s.mods.length === 0;
  $("patch-mods").textContent = `With the client files of these mods, in load order: ${s.mods.join(", ")}.`;
  $("patch-clashes").hidden = s.clashes.length === 0;
  $("patch-clashes").querySelector("summary").textContent = `${count(s.clashes.length, "thing", "things")} set differently by two of them`;
  $("patch-clashes").querySelector("pre").textContent = firstLines(s.clashes);
  $("publish").disabled = publishing || total === 0 || s.problems.length > 0;

  $("no-patches").hidden = s.history.length > 0;
  $("patch-history").replaceChildren(
    ...s.history.map((p) => {
      const li = document.createElement("li");
      li.innerHTML = `<strong class="when"></strong> <span class="muted"></span>`;
      li.querySelector(".when").textContent = p.published;
      li.querySelector(".muted").textContent = `${summary(p) || "no file changes"} · ${p.patches.join(", ")}`;
      li.title = [...p.added, ...p.changed, ...p.removed].slice(0, 30).join("\n");
      return li;
    }),
  );
}

$("nav-patch").addEventListener("click", async () => {
  if (page === "patch") return showPage("dashboard");
  $("patch-error").hidden = true;
  $("patch-done").hidden = true;
  showPage("patch");
  await refreshPatch();
});

$("patch-refresh").addEventListener("click", () => {
  $("patch-done").hidden = true;
  refreshPatch();
});

$("open-client-files").addEventListener("click", async () => {
  $("patch-error").hidden = true;
  try {
    await invoke("open_client_files");
  } catch (err) {
    $("patch-error").textContent = `Couldn't open the folder: ${err}`;
    $("patch-error").hidden = false;
  }
  refreshPatch();
});

$("publish").addEventListener("click", async () => {
  $("patch-error").hidden = true;
  $("patch-done").hidden = true;
  publishing = true;
  $("publish").disabled = true;
  $("publish").textContent = "Publishing…";
  try {
    const p = await invoke("publish_client_patch");
    $("patch-done").textContent = `Published ${p.patches.join(" and ")}. Players get it the next time they open the patcher.`;
    $("patch-done").hidden = false;
  } catch (err) {
    $("patch-error").textContent = String(err);
    $("patch-error").hidden = false;
  } finally {
    publishing = false;
    $("publish").textContent = "Publish client patch";
    await refreshPatch();
  }
});

// ---- Mods: server-side scripts and database files, applied on restart ----

function modsError(text) {
  $("mods-error").textContent = text || "";
  $("mods-error").hidden = !text;
}

function modItem(m) {
  const li = document.createElement("li");
  const label = document.createElement("label");
  label.className = "check";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = m.enabled;
  // A mod that can't load can still be switched off.
  box.disabled = !!m.problem && !m.enabled;
  box.addEventListener("change", async () => {
    modsError(null);
    box.disabled = true;
    try {
      await invoke("set_mod_enabled", { name: m.name, enabled: box.checked });
    } catch (err) {
      modsError(String(err));
    }
    await refreshMods();
  });
  const name = document.createElement("span");
  name.textContent = m.name;
  label.append(box, name);
  if (m.pending) {
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = m.enabled ? "loads on restart" : "unloads on restart";
    label.append(badge);
  }
  if (m.enabled) {
    const arrows = document.createElement("span");
    arrows.className = "order";
    for (const [up, text, title] of [[true, "↑", "Load earlier"], [false, "↓", "Load later"]]) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = text;
      b.title = title;
      b.addEventListener("click", async () => {
        modsError(null);
        try {
          await invoke("move_mod", { name: m.name, up });
        } catch (err) {
          modsError(String(err));
        }
        await refreshMods();
      });
      arrows.append(b);
    }
    label.append(arrows);
  }
  li.append(label);

  const parts = [];
  if (m.scripts.length) parts.push(count(m.scripts.length, "script", "scripts"));
  if (m.databases.length) parts.push(m.databases.join(", "));
  if (m.client_files) parts.push(count(m.client_files, "client file", "client files"));
  if (parts.length) {
    const what = document.createElement("p");
    what.className = "what muted";
    what.textContent = parts.join(" · ");
    what.title = [...m.scripts, ...m.databases].join("\n");
    li.append(what);
  }
  if (m.problem) {
    const p = document.createElement("p");
    p.className = "problem";
    p.textContent = m.problem;
    li.append(p);
  }
  if (m.clashes.length) {
    const details = document.createElement("details");
    details.dataset.key = `${m.name}/clashes`;
    const summary = document.createElement("summary");
    summary.textContent = `${count(m.clashes.length, "value", "values")} another mod also sets`;
    const p = document.createElement("p");
    p.className = "muted";
    p.textContent = "These mods set the same values differently. Each line ends with the one the server uses; the arrows change the load order. Changes show here right away and load on the next restart.";
    const pre = document.createElement("pre");
    pre.textContent = m.clashes.join("\n\n");
    details.append(summary, p, pre);
    li.append(details);
  }
  if (m.client_clashes.length) {
    const details = document.createElement("details");
    details.dataset.key = `${m.name}/client-clashes`;
    const summary = document.createElement("summary");
    summary.textContent = `${count(m.client_clashes.length, "client file entry", "client file entries")} another mod also sets`;
    const p = document.createElement("p");
    p.className = "muted";
    p.textContent = "Each line ends with the one the next client patch carries; the arrows change the load order.";
    const pre = document.createElement("pre");
    pre.textContent = firstLines(m.client_clashes);
    details.append(summary, p, pre);
    li.append(details);
  }
  if (m.item_warnings.length) {
    const details = document.createElement("details");
    details.dataset.key = `${m.name}/items`;
    const summary = document.createElement("summary");
    summary.textContent = `${count(m.item_warnings.length, "problem", "problems")} with its custom items`;
    const p = document.createElement("p");
    p.className = "muted";
    p.textContent = "A custom item needs an entry in db/item_db.yml for the server and one in client/SystemEN/itemInfo_C.lua for the players' clients, under the same Id.";
    const pre = document.createElement("pre");
    pre.textContent = m.item_warnings.join("\n\n");
    details.append(summary, p, pre);
    li.append(details);
  }
  if (m.errors.length) {
    const details = document.createElement("details");
    details.dataset.key = `${m.name}/errors`;
    const summary = document.createElement("summary");
    summary.textContent = `${count(m.errors.length, "error or warning", "errors or warnings")} in the map server's log`;
    const pre = document.createElement("pre");
    pre.textContent = m.errors.join("\n\n");
    details.append(summary, pre);
    li.append(details);
  }
  return li;
}

// rAthena's own custom NPCs, grouped as rAthena's scripts_custom.conf groups them.
function customNpcItem(n) {
  const li = document.createElement("li");
  const label = document.createElement("label");
  label.className = "check";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = n.enabled;
  // One that can't load can still be switched off.
  box.disabled = !!n.problem && !n.enabled;
  box.addEventListener("change", async () => {
    modsError(null);
    box.disabled = true;
    try {
      await invoke("set_custom_npc", { id: n.id, enabled: box.checked });
    } catch (err) {
      modsError(String(err));
    }
    await refreshMods();
  });
  const name = document.createElement("span");
  name.textContent = n.name;
  label.append(box, name);
  if (n.pending) {
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = n.enabled ? "loads on restart" : "unloads on restart";
    label.append(badge);
  }
  li.append(label);
  const what = document.createElement("p");
  what.className = "what muted";
  what.textContent = n.description;
  what.title = n.files.join("\n");
  li.append(what);
  if (n.warning) {
    const p = document.createElement("p");
    p.className = "warning";
    p.textContent = `⚠ ${n.warning}`;
    li.append(p);
  }
  if (n.problem) {
    const p = document.createElement("p");
    p.className = "problem";
    p.textContent = n.problem;
    li.append(p);
  }
  if (n.errors.length) {
    const details = document.createElement("details");
    details.dataset.key = `${n.id}/errors`;
    const summary = document.createElement("summary");
    summary.textContent = `${count(n.errors.length, "error or warning", "errors or warnings")} in the map server's log`;
    const pre = document.createElement("pre");
    pre.textContent = n.errors.join("\n\n");
    details.append(summary, pre);
    li.append(details);
  }
  return li;
}

function renderCustomNpcs(c) {
  const box = $("custom-npcs");
  const open = new Set([...box.querySelectorAll("details[open]")].map((d) => d.dataset.key));
  const groups = new Map();
  for (const n of c.npcs) {
    if (!groups.has(n.group)) groups.set(n.group, []);
    groups.get(n.group).push(n);
  }
  const parts = [];
  for (const [group, npcs] of groups) {
    const h = document.createElement("h3");
    h.textContent = group;
    const ul = document.createElement("ul");
    ul.append(...npcs.map(customNpcItem));
    parts.push(h, ul);
  }
  box.replaceChildren(...parts);
  for (const d of box.querySelectorAll("details")) d.open = open.has(d.dataset.key);
  $("custom-npcs-by-hand").hidden = c.by_hand.length === 0;
  $("custom-npcs-by-hand").textContent = `Also loads, added by hand to custom_npcs.txt in the override folder: ${c.by_hand.join(", ")}.`;
}

async function refreshMods() {
  let s;
  try {
    s = await invoke("mods");
  } catch (err) {
    modsError(String(err));
    return;
  }
  $("mods-folder").textContent = `Folder: ${s.folder}`;
  $("no-mods").hidden = s.mods.length > 0;
  // Refreshing keeps the sections that were open.
  const open = new Set([...$("mods").querySelectorAll("details[open]")].map((d) => d.dataset.key));
  $("mods").replaceChildren(...s.mods.map(modItem));
  for (const d of $("mods").querySelectorAll("details")) d.open = open.has(d.dataset.key);
  const on = $("mods").querySelectorAll(".order");
  if (on.length) {
    on[0].firstChild.disabled = true;
    on[on.length - 1].lastChild.disabled = true;
  }
  renderCustomNpcs(s.custom_npcs);
  const rebuild = s.rebuild_needed || s.custom_npcs.rebuild_needed;
  $("mods-rebuild").hidden = !rebuild;
  $("mods-pending").hidden = !(s.restart_needed || s.custom_npcs.restart_needed) || rebuild;
}

$("nav-mods").addEventListener("click", async () => {
  if (page === "mods") return showPage("dashboard");
  modsError(null);
  showPage("mods");
  await refreshMods();
});

$("mods-refresh").addEventListener("click", () => {
  modsError(null);
  refreshMods();
});

$("open-mods").addEventListener("click", async () => {
  modsError(null);
  try {
    await invoke("open_mods");
  } catch (err) {
    modsError(`Couldn't open the folder: ${err}`);
  }
});

$("new-item-mod").addEventListener("submit", async (e) => {
  e.preventDefault();
  modsError(null);
  try {
    await invoke("new_item_mod", { name: $("new-item-mod-name").value });
  } catch (err) {
    modsError(String(err));
  }
  await refreshMods();
});

// A rebuild backs up first; the "Restart on the new build" bar then appears.
$("mods-rebuild-button").addEventListener("click", () => {
  showPage("dashboard");
  runTask("build");
});

$("mods-restart").addEventListener("click", () => {
  showPage("dashboard");
  runTask("restart");
});

// ---- News: the portal's news.md, one card per post ----

let newsText = ""; // the file as last read, so a save can spot a hand edit
let newsDirty = false;

function newsMessage(error, saved) {
  $("news-error").textContent = error || "";
  $("news-error").hidden = !error;
  $("news-saved").hidden = !saved;
}

function setNewsDirty(dirty) {
  newsDirty = dirty;
  $("news-save").disabled = !dirty;
  if (dirty) newsMessage(null, false);
}

function newsCard(post) {
  const card = document.createElement("section");
  card.className = "card post";
  const head = document.createElement("div");
  head.className = "post-head";
  const title = document.createElement("input");
  title.className = "title";
  title.value = post.title;
  title.placeholder = "Title";
  title.setAttribute("aria-label", "Title");
  const newest = document.createElement("span");
  newest.className = "newest muted";
  newest.textContent = "Shown in the patcher";
  const remove = document.createElement("button");
  remove.type = "button";
  remove.textContent = "Delete";
  remove.title = "Removed when you save. Reload brings it back until then.";
  remove.addEventListener("click", () => {
    card.remove();
    renumberNews();
    setNewsDirty(true);
  });
  head.append(title, newest, remove);
  const body = document.createElement("textarea");
  body.className = "body";
  body.rows = Math.min(16, Math.max(4, post.body.split("\n").length + 1));
  body.value = post.body;
  body.placeholder = "What's new";
  body.setAttribute("aria-label", "Text");
  card.append(head, body);
  return card;
}

// Only the first card is the newest post, which the patcher's window shows.
function renumberNews() {
  const cards = [...$("news-posts").children];
  cards.forEach((card, i) => (card.querySelector(".newest").hidden = i !== 0));
  $("no-news").hidden = cards.length > 0;
}

function showNews(news) {
  newsText = news.text;
  $("news-intro").value = news.intro;
  $("news-intro-box").open = !!news.intro;
  $("news-posts").replaceChildren(...news.posts.map(newsCard));
  renumberNews();
  setNewsDirty(false);
}

async function loadNews() {
  newsMessage(null, false);
  try {
    showNews(await invoke("news"));
  } catch (err) {
    newsMessage(String(err), false);
  }
}

$("nav-news").addEventListener("click", async () => {
  if (page === "news") return showPage("dashboard");
  // Unsaved edits stay while other screens are open.
  if (!newsDirty) await loadNews();
  showPage("news");
});

$("news-page").addEventListener("input", (e) => {
  if (e.target.matches("input, textarea")) setNewsDirty(true);
});

$("news-add").addEventListener("click", () => {
  // The date in italics on the first line, so players can tell how old a post is.
  const today = new Date().toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
  const card = newsCard({ title: "", body: `*${today}*\n\n` });
  $("news-posts").prepend(card);
  renumberNews();
  setNewsDirty(true);
  card.querySelector(".title").focus();
});

$("news-save").addEventListener("click", async () => {
  const posts = [...$("news-posts").children].map((card) => ({
    title: card.querySelector(".title").value,
    body: card.querySelector(".body").value,
  }));
  try {
    showNews(await invoke("save_news", { news: { intro: $("news-intro").value, posts, text: newsText } }));
    newsMessage(null, true);
  } catch (err) {
    newsMessage(String(err), false);
  }
});

$("news-reload").addEventListener("click", loadNews);

$("news-open").addEventListener("click", () => {
  // The portal's address is known only while it runs.
  if ($("portal").hidden) return newsMessage("Start the server to open the account portal.", false);
  invoke("open_url", { url: `${$("portal-link").dataset.url}news` });
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
    const verb = { start: "Start", restart: "Restart", readdress: "Address change", portal: "Portal port change", stop: "Stop", build: "Build", update: "Server update", backup: "Backup", restore: "Restore", "test-restore": "Restore test" }[
      payload.kind
    ];
    push(activity, payload.ok ? `\n${verb} finished.` : `\n${verb} failed. See the output above.`);
    if (view === "activity") render();
    await refreshOverview();
    await refreshServices();
    await refreshBackups();
    refreshLive();
    if (page === "mods") refreshMods();
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
// Clashes between mods show soon after their files change.
setInterval(() => {
  if (page === "mods") refreshMods();
}, 5000);
document.addEventListener("visibilitychange", refreshLive);
