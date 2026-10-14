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
let task = null; // "start", "restart", "stop" or "build" while one runs
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
  for (const id of ["start", "stop", "build", "build-save", "restart"]) $(id).disabled = !ready || !!task;
  $("cancel").hidden = !["build", "start", "restart"].includes(task);
}

// ---- Pages: the dashboard and Settings ----

let page = "dashboard";

function showPage(next) {
  if (next) page = next;
  $("dashboard").hidden = next !== "dashboard";
  $("settings").hidden = next !== "settings";
  $("nav").textContent = page === "settings" ? "Back to server" : "Settings";
}

$("nav").addEventListener("click", async () => {
  if (page === "dashboard") {
    $("game-saved").hidden = true;
    $("game-error").hidden = true;
    await loadGameSettings();
    showPage("settings");
  } else {
    showPage("dashboard");
  }
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

// ---- Tasks ----

async function runTask(kind) {
  activity.length = 0;
  clearProgress();
  await showView("activity");
  try {
    await invoke("run_task", { kind });
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
    const verb = { start: "Start", restart: "Restart", stop: "Stop", build: "Build" }[payload.kind];
    push(activity, payload.ok ? `\n${verb} finished.` : `\n${verb} failed. See the output above.`);
    if (view === "activity") render();
    await refreshOverview();
    await refreshServices();
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

refreshOverview().then(refreshServices);
setInterval(refreshServices, 3000);
