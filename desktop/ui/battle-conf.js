// The battle config editor window: rAthena's conf/battle/*.conf files,
// one at a time, with their comments. Saved values go to the overrides
// folder's battle_conf.txt (battleconf.rs).

const { invoke } = window.__TAURI__.core;
const { emit } = window.__TAURI__.event;
const currentWindow = window.__TAURI__.window.getCurrentWindow();

const $ = (id) => document.getElementById(id);
const FILE_KEY = "battle-conf-file";

let conf = null;
const edits = new Map(); // option key -> value not saved yet
const settings = new Map(); // option key -> setting, from conf

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") node.className = v;
    else node[k] = v;
  }
  node.append(...children);
  return node;
}

function showError(message) {
  $("error").textContent = message;
  $("error").hidden = !message;
}

async function load() {
  showError("");
  try {
    conf = await invoke("battle_conf");
  } catch (err) {
    $("loading").hidden = true;
    showError(String(err));
    return;
  }
  settings.clear();
  for (const f of conf.files) for (const g of f.groups) for (const s of g.options) settings.set(s.key, s);
  // Drop edits that now match what's saved (another save, a Settings change).
  for (const [key, value] of edits) if (!settings.has(key) || settings.get(key).value === value) edits.delete(key);

  const select = $("file");
  const chosen = select.value || remembered() || "exp.conf";
  select.replaceChildren(...conf.files.map((f) => el("option", { value: f.name })));
  select.value = conf.files.some((f) => f.name === chosen) ? chosen : conf.files[0]?.name ?? "";
  select.disabled = $("search").disabled = false;
  $("loading").hidden = true;
  $("about").hidden = false;
  render();
}

function remembered() {
  try {
    return localStorage.getItem(FILE_KEY);
  } catch {
    return null;
  }
}

const currentValue = (s) => (edits.has(s.key) ? edits.get(s.key) : s.value);

function renderFileNames() {
  for (const option of $("file").options) {
    const file = conf.files.find((f) => f.name === option.value);
    const keys = file.groups.flatMap((g) => g.options);
    const changed = keys.filter((s) => s.changed).length;
    const pending = keys.some((s) => edits.has(s.key));
    option.textContent = `${pending ? "• " : ""}${file.name}${changed ? ` (${changed} changed)` : ""}`;
  }
  $("save").textContent = edits.size ? `Save ${edits.size} change${edits.size === 1 ? "" : "s"}` : "Save";
  $("save").disabled = edits.size === 0;
}

function render() {
  renderFileNames();
  const query = $("search").value.trim().toLowerCase();
  // One letter matches nearly everything, so searching starts at two.
  const searching = query.length >= 2;
  const groups = $("groups");
  groups.replaceChildren();
  const intro = $("intro");

  if (!searching) {
    const file = conf.files.find((f) => f.name === $("file").value);
    intro.textContent = file?.intro ?? "";
    intro.hidden = !intro.textContent;
    for (const g of file?.groups ?? []) groups.append(renderGroup(g));
    $("no-match").hidden = true;
    return;
  }

  intro.hidden = true;
  let found = 0;
  for (const file of conf.files) {
    const matches = file.groups.filter(
      (g) => g.options.length && (g.comment.toLowerCase().includes(query) || g.options.some((s) => s.key.toLowerCase().includes(query))),
    );
    if (!matches.length) continue;
    found += matches.length;
    groups.append(el("h2", { class: "file-heading" }, file.name));
    for (const g of matches) groups.append(renderGroup(g));
  }
  $("no-match").hidden = found > 0;
}

function renderGroup(group) {
  const section = el("section", { class: group.options.length ? "group" : "group heading" });
  if (group.comment) section.append(el("pre", { class: "comment" }, group.comment));
  for (const s of group.options) section.append(renderSetting(s));
  return section;
}

function renderSetting(s) {
  const row = el("div", { class: "setting" });
  const value = currentValue(s);
  let input;
  if (s.kind === "switch") {
    input = el("select");
    const choices = ["yes", "no"];
    if (!choices.includes(value.toLowerCase())) choices.push(value);
    for (const c of choices) input.append(el("option", { value: c, textContent: c }));
    input.value = choices.find((c) => c.toLowerCase() === value.toLowerCase());
  } else {
    input = el("input", { value, spellcheck: false, maxLength: s.kind === "symbol" ? 1 : 12 });
    if (s.kind === "symbol") input.classList.add("symbol");
  }
  input.id = `opt-${s.key}`;
  input.addEventListener(s.kind === "switch" ? "change" : "input", () => {
    const v = input.value.trim();
    if (v === s.value) edits.delete(s.key);
    else edits.set(s.key, v);
    mark();
  });

  const reset = el("button", { type: "button", class: "reset", textContent: "Reset", title: `Back to ${s.default}` });
  reset.addEventListener("click", () => {
    if (s.default === s.value) edits.delete(s.key);
    else edits.set(s.key, s.default);
    row.replaceWith(renderSetting(s));
    renderFileNames();
  });

  const label = el("label", { htmlFor: input.id }, el("code", {}, s.key));
  const defaultText = el("span", { class: "default muted" }, `Default: ${s.default}`);
  row.append(label, input, defaultText, reset);
  if (s.note) row.append(el("p", { class: "note hint" }, s.note));

  function mark() {
    const v = edits.has(s.key) ? edits.get(s.key) : s.value;
    row.classList.toggle("changed", v !== s.default);
    row.classList.toggle("edited", edits.has(s.key));
    reset.hidden = v === s.default;
    renderFileNames();
  }
  mark();
  return row;
}

async function save() {
  showError("");
  $("saved").hidden = true;
  let changed;
  try {
    changed = await invoke("save_battle_conf", { changes: Object.fromEntries(edits) });
  } catch (err) {
    showError(String(err));
    return false;
  }
  edits.clear();
  await load();
  if (changed) {
    $("saved").hidden = false;
    $("restart").hidden = !(await serverRunning());
  }
  return true;
}

async function serverRunning() {
  try {
    const services = await invoke("services");
    return services.some((s) => ["login", "char", "map"].includes(s.service) && s.state === "running");
  } catch {
    return false;
  }
}

$("file").addEventListener("change", () => {
  try {
    localStorage.setItem(FILE_KEY, $("file").value);
  } catch {}
  $("search").value = "";
  render();
  $("editor").scrollTop = 0;
});
// Wait for a pause in typing: a short search matches most of the files.
let searchTimer;
$("search").addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(render, 200);
});
$("save").addEventListener("click", save);

// The main window runs the restart, so it shows in its activity log.
$("restart").addEventListener("click", async () => {
  $("saved").hidden = true;
  await emit("restart-game-servers");
});

// Refresh after the Settings screen saves, unless there are edits here.
window.addEventListener("focus", () => {
  if (conf && edits.size === 0) load();
});

currentWindow.onCloseRequested((event) => {
  if (edits.size === 0) return;
  event.preventDefault();
  $("closing").hidden = false;
});
$("close-save").addEventListener("click", async () => {
  $("closing").hidden = true;
  if (await save()) currentWindow.destroy();
});
$("close-discard").addEventListener("click", () => currentWindow.destroy());
$("close-cancel").addEventListener("click", () => {
  $("closing").hidden = true;
});

load();
