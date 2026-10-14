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
let task = null; // "start", "restart", "stop", "build", "backup" or "restore" while one runs
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

function fillAddresses(select, current, auto) {
  const detected = overview.addresses[0]?.ip;
  const options = [
    ["auto", `Detect automatically${detected ? ` (now ${detected})` : ""}`],
    ...overview.addresses.map((a) => [a.ip, `${a.ip} (${a.interface})`]),
    ["127.0.0.1", "127.0.0.1 (this computer only)"],
  ];
  if (current && !auto && !options.some(([v]) => v === current)) options.push([current, current]);
  select.replaceChildren(
    ...options.map(([value, label]) => {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = label;
      return o;
    }),
  );
  select.value = auto || !current ? "auto" : current;
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
  showPage(overview.configured ? page : null);
  if (!overview.configured) {
    fillAddresses($("setup-ip"), null, true);
  } else {
    fillAddresses($("ip"), overview.public_ip, overview.public_ip_auto);
    $("connect-addr").textContent = `${overview.public_ip}:6900`;
    $("admin-name").textContent = overview.admin_user;
    if (!buildFormDirty) fillBuildForm();
  }
  updateButtons();
}

function updateButtons() {
  const ready = overview?.configured && !overview.docker_error;
  for (const id of ["start", "stop", "build", "build-save", "restart", "backup-now"]) $(id).disabled = !ready || !!task;
  for (const b of document.querySelectorAll("#backups button")) b.disabled = !ready || !!task;
  $("cancel").hidden = !["build", "start", "restart"].includes(task);
}

// ---- Pages: the main screen, Dashboard, Settings, Backups and Client patch ----

let page = "dashboard";

function showPage(next) {
  if (next) page = next;
  $("dashboard").hidden = next !== "dashboard";
  $("settings").hidden = next !== "settings";
  $("backups-page").hidden = next !== "backups";
  $("live-page").hidden = next !== "live";
  $("patch-page").hidden = next !== "patch";
  $("nav").textContent = page === "settings" ? "Back to server" : "Settings";
  $("nav-backups").textContent = page === "backups" ? "Back to server" : "Backups";
  $("nav-live").textContent = page === "live" ? "Back to server" : "Dashboard";
  $("nav-patch").textContent = page === "patch" ? "Back to server" : "Client patch";
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
  const ip = $("setup-ip").value;
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

$("ip").addEventListener("change", async () => {
  const ip = $("ip").value;
  try {
    await invoke("set_public_ip", { publicIp: ip === "auto" ? null : ip });
  } catch (err) {
    banner(String(err));
  }
  await refreshOverview();
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
  invoke("plugin:opener|open_url", { url: e.target.dataset.url });
});

// ---- Backups ----

const REASONS = {
  manual: "Backed up by hand",
  rebuild: "Before a rebuild",
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
  $("patch-folder").textContent = `Folder: ${s.folder}`;
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
  $("patch-summary").textContent = s.problems.length
    ? "Fix the files above to publish."
    : total
      ? `${count(total, "file", "files")} to publish: ${summary(c)}.`
      : s.history.length
        ? "Nothing has changed since the last patch."
        : "The client files folder is empty.";
  $("patch-changes").replaceChildren(...fileItems(c));
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
    const verb = { start: "Start", restart: "Restart", stop: "Stop", build: "Build", backup: "Backup", restore: "Restore" }[
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
