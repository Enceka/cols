"use strict";

const $ = (id) => document.getElementById(id);
const commands = ["help", "status", "listeners", "check-config", "show-config", "tail", "clear"];
const views = {
  overview: ["服务概览", "从这里了解运行环境，管理你的 CODOL 服务。"],
  hosting: ["开服设置", "准备资料、管理证书，让独立服务器一键运行。"],
  terminal: ["交互终端", "一个轻巧的服务控制台，让诊断与检查触手可及。"],
  logs: ["运行日志", "跟踪服务事件，在细节中找到线索。"],
  config: ["服务配置", "查看、校验和调整你的本地服务配置。"],
};
let currentView = "overview";
let logEntries = [];
let logLevel = "ALL";
let logsPaused = false;
let logWarning = null;
let configOriginal = "";
let configRevision = "";
let configLoaded = false;
let configBusy = false;
let commandBusy = false;
let polling = false;
let commandHistory = [];
let historyPosition = 0;
let historyDraft = "";
let toastTimer;
let hostingOriginal = "";
let hostingRevision = "";
let hostingBusy = false;
let hostingActive = false;
let hostingPhase = "stopped";
let hostingCertificateReady = false;

async function api(path, data) {
  const response = await fetch(path, {
    method: data === undefined ? "GET" : "POST",
    headers: data === undefined ? {} : { "Content-Type": "application/json" },
    body: data === undefined ? undefined : JSON.stringify(data),
    signal: AbortSignal.timeout(8000),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `请求失败 (${response.status})`);
  return result;
}

function toast(message, error = false) {
  clearTimeout(toastTimer);
  $("toast").textContent = message;
  $("toast").classList.toggle("error", error);
  $("toast").hidden = false;
  toastTimer = setTimeout(() => { $("toast").hidden = true; }, 4000);
}

function icon(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

function showView(view, updateHash = true) {
  if (!views[view]) view = "overview";
  currentView = view;
  for (const name of Object.keys(views)) $("view-" + name).hidden = name !== view;
  for (const button of document.querySelectorAll("[data-view]")) {
    const active = button.dataset.view === view;
    button.classList.toggle("active", active);
    if (active) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  }
  $("page-title").replaceChildren(document.createTextNode(views[view][0]));
  const dot = document.createElement("span");
  dot.className = "heading-dot";
  dot.textContent = ".";
  $("page-title").append(dot);
  $("page-description").textContent = views[view][1];
  $("breadcrumb-current").textContent = views[view][0];
  document.title = `${views[view][0]} · CODOL Control`;
  const slot = view === "terminal" ? "terminal-full-slot" : "overview-terminal-slot";
  $(slot).append($("terminal-card"));
  $("terminal-expand").hidden = view === "terminal";
  $("sidebar").classList.remove("open");
  $("menu-toggle").setAttribute("aria-expanded", "false");
  if (updateHash) history.replaceState(null, "", `#${view}`);
  if (view === "terminal") $("terminal-input").focus({ preventScroll: true });
}

function formatTime(time) {
  if (time === null || time === undefined) return "见日志原文";
  return new Date(time * 1000).toLocaleTimeString("zh-CN", { hour12: false });
}

function uptime(seconds) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor(seconds % 3600 / 60);
  const s = seconds % 60;
  return [h, m, s].map((value) => String(value).padStart(2, "0")).join(":");
}

function basename(path) {
  return String(path).split(/[\\/]/).pop();
}

function renderStatus(status) {
  renderHosting(status.hosting);
  $("metric-uptime").textContent = uptime(status.uptime_seconds);
  const live = status.hosting?.phase === "running";
  const listeners = live ? Object.entries(status.hosting.endpoints).map(([name, bind]) => ({ name, bind })) : [];
  $("metric-listeners").textContent = listeners.length;
  $("metric-log-source").textContent = status.log_file ? "服务端日志" : "面板日志";
  $("metric-log-note").textContent = status.log_file ? basename(status.log_file) : "操作事件实时记录";
  $("metric-log-note").title = status.log_file || "";
  $("metric-config").textContent = status.config_valid ? "校验通过" : "需要检查";
  $("metric-revision").textContent = status.revision.slice(0, 8);
  $("sidebar-address").textContent = status.address;
  $("sidebar-version").textContent = `v${status.version}`;
  $("config-filename").textContent = basename(status.config_path);
  $("endpoint-count").textContent = `${listeners.length} 个端点`;
  $("logs-source-note").textContent = status.log_file ? `服务端日志：${status.log_file} · 同时显示面板操作日志` : "当前来源：面板操作日志；开服后自动接入服务端日志。";
  $("endpoint-list").replaceChildren();
  for (const listener of listeners) {
    const row = document.createElement("div");
    row.className = "endpoint-row";
    const badge = document.createElement("span");
    badge.className = "endpoint-icon";
    badge.append(icon("server"));
    const content = document.createElement("div");
    const name = document.createElement("span");
    name.className = "endpoint-name";
    name.textContent = listener.name;
    const address = document.createElement("span");
    address.className = "endpoint-address";
    address.textContent = listener.bind;
    content.append(name, address);
    const state = document.createElement("span");
    state.className = "endpoint-state";
    state.textContent = "运行中";
    row.append(badge, content, state);
    $("endpoint-list").append(row);
  }
  if (!listeners.length) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    empty.textContent = "开服后显示实际服务端点";
    $("endpoint-list").append(empty);
  }
}

function stripAnsi(text) {
  return text.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "");
}

function renderRows(target, entries, emptyText) {
  const fragment = document.createDocumentFragment();
  for (const entry of entries) {
    const row = document.createElement("tr");
    const time = document.createElement("td");
    time.className = "log-time";
    time.textContent = formatTime(entry.time);
    const levelCell = document.createElement("td");
    const level = document.createElement("span");
    level.className = `level-badge level-${entry.level.toLowerCase()}`;
    level.textContent = entry.level;
    levelCell.append(level);
    const source = document.createElement("td");
    source.className = "log-source";
    source.textContent = entry.source;
    const message = document.createElement("td");
    message.className = "log-message";
    message.textContent = stripAnsi(entry.message);
    row.append(time, levelCell, source, message);
    fragment.append(row);
  }
  if (!entries.length) {
    const row = document.createElement("tr");
    const cell = document.createElement("td");
    cell.colSpan = 4;
    cell.className = "empty-state";
    cell.textContent = emptyText;
    row.append(cell);
    fragment.append(row);
  }
  target.replaceChildren(fragment);
}

function filteredLogs() {
  const search = $("log-search").value.trim().toLowerCase();
  return logEntries.filter((entry) => {
    const levelMatches = logLevel === "ALL" || entry.level === logLevel || (logLevel === "DEBUG" && entry.level === "TRACE");
    const text = `${entry.message} ${entry.source} ${entry.level}`.toLowerCase();
    return levelMatches && (!search || text.includes(search));
  });
}

function renderLogs() {
  const logs = filteredLogs();
  renderRows($("recent-log-body"), logEntries.slice(-4).reverse(), "暂时没有日志。执行一条终端命令开始记录。");
  renderRows($("log-body"), logs.slice().reverse(), "没有符合筛选条件的日志。");
  $("nav-log-count").textContent = logEntries.length;
  $("logs-total").textContent = logEntries.length;
  $("logs-showing").textContent = `显示 ${logs.length} / ${logEntries.length} 条日志`;
  $("log-warning").textContent = logWarning || "";
  $("log-warning").hidden = !logWarning;
}

function updateEditor() {
  const content = $("config-editor").value;
  const count = content.split("\n").length;
  const dirty = content !== configOriginal;
  $("line-numbers").textContent = Array.from({ length: count }, (_, index) => index + 1).join("\n");
  $("line-numbers").scrollTop = $("config-editor").scrollTop;
  $("editor-lines").textContent = `${count} 行 · ${new TextEncoder().encode(content).length} 字节`;
  $("editor-state").textContent = dirty ? "未保存" : "已保存";
  $("editor-state").classList.toggle("dirty", dirty);
  $("save-config").disabled = !dirty || !configLoaded || configBusy;
  $("validate-config").disabled = !configLoaded || configBusy;
  $("reload-config").disabled = configBusy;
}

function applyConfig(config, replaceDraft) {
  const filename = basename(config.path);
  $("editor-filename").textContent = filename;
  $("editor-path").textContent = config.path;
  $("editor-path").title = config.path;
  if (config.values) {
    $("summary-connections").textContent = config.values.max_connections;
    $("summary-timeout").textContent = `${config.values.idle_timeout_ms / 1000}s`;
  } else {
    $("summary-connections").textContent = "—";
    $("summary-timeout").textContent = "—";
  }
  if (replaceDraft || !configLoaded) {
    configOriginal = config.content;
    configRevision = config.revision;
    $("config-editor").value = config.content;
    configLoaded = true;
    updateEditor();
  }
}

async function poll(showToast = false) {
  if (polling) return;
  polling = true;
  try {
    const [status, config, logs] = await Promise.all([
      api("/api/status"), api("/api/config"), logsPaused ? null : api("/api/logs"),
    ]);
    renderStatus(status);
    const clean = $("config-editor").value === configOriginal;
    applyConfig(config, !configLoaded || (clean && config.revision !== configRevision));
    if (logs) {
      logEntries = logs.entries;
      logWarning = logs.warning;
      renderLogs();
    }
    $("connection-text").textContent = "面板已连接";
    $("connection-badge").classList.remove("disconnected");
    $("connection-error").hidden = true;
    $("last-sync").textContent = `更新于 ${new Date().toLocaleTimeString("zh-CN", { hour12: false })}`;
    if (showToast) toast("数据已刷新");
  } catch (error) {
    $("connection-text").textContent = "连接中断";
    $("connection-badge").classList.add("disconnected");
    $("connection-error").textContent = `数据同步失败：${error.message}。请确认管理服务仍在运行。已加载的内容保留在页面中。`;
    $("connection-error").hidden = false;
    if (showToast) toast(error.message, true);
  } finally {
    polling = false;
  }
}

function appendTerminal(command, output, failed = false) {
  const entry = document.createElement("div");
  entry.className = `terminal-entry${failed ? " error" : ""}`;
  const prompt = document.createElement("div");
  prompt.className = "terminal-entry-prompt";
  prompt.textContent = `❯ ${command}`;
  const text = document.createElement("pre");
  text.className = "terminal-entry-output";
  text.textContent = output;
  entry.append(prompt, text);
  $("terminal-output").append(entry);
  while ($("terminal-output").childElementCount > 100) $("terminal-output").firstElementChild.remove();
  $("terminal-output").scrollTop = $("terminal-output").scrollHeight;
}

async function runCommand(command) {
  command = command.trim();
  if (!command || commandBusy) return;
  commandBusy = true;
  $("terminal-run").disabled = true;
  $("terminal-input").value = "";
  commandHistory.push(command);
  if (commandHistory.length > 100) commandHistory.shift();
  historyPosition = commandHistory.length;
  historyDraft = "";
  try {
    const result = await api("/api/terminal", { command });
    if (result.clear) $("terminal-output").replaceChildren();
    else appendTerminal(command, result.output, !result.ok);
    await poll();
  } catch (error) {
    appendTerminal(command, error.message, true);
  } finally {
    commandBusy = false;
    $("terminal-run").disabled = false;
    if (currentView === "terminal" || currentView === "overview") $("terminal-input").focus({ preventScroll: true });
  }
}

function feedback(message, error = false) {
  $("config-feedback").textContent = message;
  $("config-feedback").classList.toggle("error", error);
  $("config-feedback").hidden = false;
}

async function validateOrSave(save) {
  if (!configLoaded || configBusy) return;
  const content = $("config-editor").value;
  configBusy = true;
  updateEditor();
  try {
    const result = await api(save ? "/api/config" : "/api/config/validate", { content, revision: configRevision });
    if (save) {
      configOriginal = content;
      configRevision = result.revision;
      feedback(result.unchanged ? "配置内容未变化。" : `保存成功。备份：${result.backup}。重新启动观察服务后生效。`);
      toast(result.unchanged ? "配置内容未变化" : "配置已保存，原文件已备份");
      await poll();
    } else feedback(`校验通过。配置包含 ${result.listeners} 个监听端点，可以保存。`);
  } catch (error) {
    feedback(error.message, true);
  } finally {
    configBusy = false;
    updateEditor();
  }
}

for (const button of document.querySelectorAll("[data-view]")) button.addEventListener("click", () => showView(button.dataset.view));
for (const button of document.querySelectorAll("[data-go]")) button.addEventListener("click", () => showView(button.dataset.go));
for (const button of document.querySelectorAll("[data-command]")) button.addEventListener("click", () => runCommand(button.dataset.command));
$("terminal-expand").addEventListener("click", () => showView("terminal"));
$("terminal-clear").addEventListener("click", () => { $("terminal-output").replaceChildren(); $("terminal-input").focus({ preventScroll: true }); });
$("terminal-form").addEventListener("submit", (event) => { event.preventDefault(); runCommand($("terminal-input").value); });
$("terminal-input").addEventListener("keydown", (event) => {
  const input = event.target;
  if (event.key === "ArrowUp" || event.key === "ArrowDown") {
    event.preventDefault();
    if (historyPosition === commandHistory.length) historyDraft = input.value;
    historyPosition = Math.max(0, Math.min(commandHistory.length, historyPosition + (event.key === "ArrowUp" ? -1 : 1)));
    input.value = historyPosition === commandHistory.length ? historyDraft : commandHistory[historyPosition];
  } else if (event.key === "Tab") {
    const matches = commands.filter((command) => command.startsWith(input.value.trim()));
    if (matches.length === 1) { event.preventDefault(); input.value = matches[0]; }
  }
});
$("refresh").addEventListener("click", () => poll(true));
$("log-search").addEventListener("input", renderLogs);
for (const button of document.querySelectorAll("[data-level]")) button.addEventListener("click", () => {
  logLevel = button.dataset.level;
  for (const chip of document.querySelectorAll("[data-level]")) {
    chip.classList.toggle("active", chip === button);
    chip.setAttribute("aria-pressed", String(chip === button));
  }
  renderLogs();
});
$("pause-logs").addEventListener("click", () => {
  logsPaused = !logsPaused;
  $("pause-logs").textContent = logsPaused ? "恢复更新" : "暂停更新";
  $("logs-live").textContent = logsPaused ? "已暂停" : "实时更新";
  if (!logsPaused) poll();
});
$("download-logs").addEventListener("click", () => {
  const lines = filteredLogs().slice().reverse().map((entry) => `${entry.time ? formatTime(entry.time) + " " : ""}[${entry.level}] [${entry.source}] ${stripAnsi(entry.message)}`);
  const blob = new Blob([lines.join("\n") + "\n"], { type: "text/plain;charset=utf-8" });
  const link = document.createElement("a");
  const url = URL.createObjectURL(blob);
  link.href = url;
  link.download = `codol-logs-${new Date().toISOString().replace(/[:.]/g, "-")}.log`;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  toast(`已导出 ${lines.length} 条筛选结果`);
});
$("config-editor").addEventListener("input", () => { updateEditor(); $("config-feedback").hidden = true; });
$("config-editor").addEventListener("scroll", () => { $("line-numbers").scrollTop = $("config-editor").scrollTop; });
$("validate-config").addEventListener("click", () => validateOrSave(false));
$("save-config").addEventListener("click", () => validateOrSave(true));
async function reloadConfig() {
  if (configBusy) return;
  configBusy = true;
  updateEditor();
  try { applyConfig(await api("/api/config"), true); $("config-feedback").hidden = true; toast("配置已重新加载"); }
  catch (error) { feedback(error.message, true); }
  finally { configBusy = false; updateEditor(); }
}
$("reload-config").addEventListener("click", () => {
  if ($("config-editor").value !== configOriginal) $("reload-dialog").showModal();
  else reloadConfig();
});
$("confirm-reload").addEventListener("click", () => { $("reload-dialog").close(); reloadConfig(); });
$("cancel-reload").addEventListener("click", () => $("reload-dialog").close());
$("export-draft").addEventListener("click", () => {
  const url = URL.createObjectURL(new Blob([$("config-editor").value], { type: "text/plain;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = "codol-config-draft.toml";
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  toast("配置草稿已导出");
});
$("open-help").addEventListener("click", () => { $("sidebar").classList.remove("open"); $("help-dialog").showModal(); });
$("close-help").addEventListener("click", () => $("help-dialog").close());
$("menu-toggle").addEventListener("click", () => {
  const open = $("sidebar").classList.toggle("open");
  $("menu-toggle").setAttribute("aria-expanded", String(open));
});
document.addEventListener("click", (event) => {
  if (!event.target.closest("#sidebar") && !event.target.closest("#menu-toggle")) {
    $("sidebar").classList.remove("open");
    $("menu-toggle").setAttribute("aria-expanded", "false");
  }
});
document.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") { event.preventDefault(); showView("terminal"); }
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s" && currentView === "config") { event.preventDefault(); validateOrSave(true); }
});
window.addEventListener("hashchange", () => showView(location.hash.slice(1), false));
window.addEventListener("beforeunload", (event) => {
  if ((configLoaded && $("config-editor").value !== configOriginal) || hostingDirty()) { event.preventDefault(); event.returnValue = ""; }
});
document.addEventListener("visibilitychange", () => { if (!document.hidden) poll(); });
showView(location.hash.slice(1) || "overview", false);
updateEditor();
poll();
setInterval(() => { if (!document.hidden) poll(); }, 3000);

const hostingKeys = ["service_data_dir", "game_state", "map_catalog", "auth_port", "lobby_port", "stun_port", "stun_alt_port", "game_port", "resource_port", "room_bot_fill"];
function hostingSettings() {
  return Object.fromEntries(hostingKeys.map((key) => [key, key.endsWith("_port") || key === "room_bot_fill" ? Number($("hosting-" + key).value) : $("hosting-" + key).value.trim()]));
}
function hostingDirty() { return hostingOriginal !== "" && JSON.stringify(hostingSettings()) !== hostingOriginal; }
function hostingButtons() {
  $("hosting-save").disabled = hostingBusy || hostingActive || !hostingDirty();
  $("hosting-start").disabled = hostingBusy || hostingActive || !hostingRevision;
  $("hosting-stop").disabled = hostingBusy || !hostingActive || hostingPhase === "stopping";
  $("hosting-fields").disabled = hostingBusy || hostingActive;
  $("hosting-download").disabled = hostingBusy || !hostingCertificateReady;
  $("hosting-renew").disabled = hostingBusy || hostingActive;
  $("hosting-edit-state").textContent = hostingDirty() ? "未保存" : "已保存";
}
function renderHosting(value) {
  if (!value) return;
  const dirty = hostingDirty();
  if (!hostingRevision || !dirty) {
    for (const key of hostingKeys) $("hosting-" + key).value = value.settings[key];
    hostingOriginal = JSON.stringify(hostingSettings());
    hostingRevision = value.revision;
  }
  const labels = { stopped: "未开服", starting: "准备中", running: "运行中", stopping: "停止中", failed: "启动失败" };
  hostingPhase = value.phase;
  hostingActive = ["starting", "running", "stopping"].includes(value.phase);
  $("hosting-phase").textContent = labels[value.phase] || value.phase;
  $("hosting-message").textContent = value.message;
  $("hosting-error").textContent = value.error || "";
  $("hosting-error").hidden = !value.error;
  $("hosting-dot").className = "status-dot" + (value.phase === "running" ? "" : hostingActive ? " busy" : " offline");
  $("hosting-config-path").textContent = value.path;
  $("hosting-missing").textContent = value.missing.join("\n");
  $("hosting-missing").hidden = !value.missing.length;
  hostingCertificateReady = value.certificate.ready;
  const certificate = value.certificate.details;
  $("hosting-certificate-status").textContent = certificate ? `已准备 · 有效期至 ${certificate.expires}` : value.certificate.error || "首次开服自动生成，本机保存并持续复用。";
  $("hosting-fingerprint").textContent = certificate ? `SHA-256 ${certificate.fingerprint}` : "";
  $("hosting-download").disabled = hostingBusy || !value.certificate.ready;
  $("hosting-renew").disabled = hostingBusy || hostingActive;
  hostingButtons();
  if (value.phase === "stopping") $("hosting-stop").disabled = true;
}
function hostingFeedback(message, error = false) {
  $("hosting-feedback").textContent = message;
  $("hosting-feedback").hidden = false;
  $("hosting-feedback").classList.toggle("error", error);
}
async function saveHosting() {
  const value = await api("/api/hosting/config", { settings: hostingSettings(), revision: hostingRevision });
  hostingOriginal = "";
  hostingRevision = "";
  renderHosting(value);
  hostingFeedback("开服设置已保存；启动时会检查地图与全部服务端点。");
}
async function hostingAction(action) {
  if (hostingBusy) return;
  if (action === "start" && !$("hosting-form").checkValidity()) { showView("hosting"); $("hosting-form").reportValidity(); return; }
  hostingBusy = true;
  hostingButtons();
  try {
    if (action === "start" && hostingDirty()) await saveHosting();
    if (action === "save") await saveHosting();
    else renderHosting(await api("/api/hosting/" + action, {}));
    await poll();
  } catch (error) {
    hostingFeedback(error.message, true);
    toast(error.message, true);
    if (action !== "stop") showView("hosting");
  } finally { hostingBusy = false; hostingButtons(); }
}
$("hosting-form").addEventListener("input", hostingButtons);
$("hosting-form").addEventListener("submit", (event) => { event.preventDefault(); hostingAction("save"); });
$("hosting-start").addEventListener("click", () => hostingAction("start"));
$("hosting-stop").addEventListener("click", () => hostingAction("stop"));
$("hosting-download").addEventListener("click", async () => {
  try {
    const value = await api("/api/hosting/certificate");
    const url = URL.createObjectURL(new Blob([value.pem], { type: "application/x-pem-file" }));
    const link = document.createElement("a"); link.href = url; link.download = "codol-server.pem"; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  } catch (error) { toast(error.message, true); }
});
$("hosting-renew").addEventListener("click", async () => {
  if (hostingBusy) return;
  hostingBusy = true; hostingButtons();
  try { await api("/api/hosting/certificate", {}); await poll(); toast("本地证书已更新，客户端需要配置新的公开证书"); }
  catch (error) { toast(error.message, true); }
  finally { hostingBusy = false; hostingButtons(); }
});
