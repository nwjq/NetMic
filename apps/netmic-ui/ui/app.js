import { WAVEFORM_POINTS, createMockAdapter, defaultSnapshot, isBusy } from "./app.core.js";
import {
  renderAppActions,
  renderConfig,
  renderLogs,
  renderStatus,
  renderModeButtons,
  renderPrimaryAction,
  renderStatusPill,
} from "./app.dom.js";
import { bindActions, bindConfigInputs } from "./app.interactions.js";

const EVENT_SNAPSHOT = "netmic://snapshot";
const EVENT_WAVEFORM = "netmic://waveform";

let state = defaultSnapshot();
let currentTab = "config";
let waveformPoints = [];
let waveformCanvas = null;
let waveformCtx = null;
let pendingHarnessRenderReport = false;
let statusPoller = null;
let closeInterceptorAttached = false;
let windowState = { maximized: false };

const sampleRates = [16000, 24000, 32000, 44100, 48000];
const chunkOptions = [10, 20, 40, 60];
const bufferOptions = [40, 60, 80, 100, 150, 200, 300, 400];

waveformPoints = new Array(WAVEFORM_POINTS).fill(0);

const elements = {
  modeButtons: Array.from(document.querySelectorAll(".mode-btn")),
  navButtons: Array.from(document.querySelectorAll(".nav-btn")),
  views: {
    config: document.getElementById("view-config"),
    status: document.getElementById("view-status"),
    logs: document.getElementById("view-logs"),
  },
  statusPill: document.getElementById("status-pill"),
  statusLabel: document.getElementById("status-label"),
  statusNote: document.getElementById("status-note"),
  primaryAction: document.getElementById("primary-action"),
  windowMinimize: document.getElementById("window-minimize"),
  windowMaximize: document.getElementById("window-maximize"),
  windowClose: document.getElementById("window-close"),
  resetDefaults: document.getElementById("reset-defaults"),
  configConnection: document.getElementById("config-connection"),
  configAudio: document.getElementById("config-audio"),
  configClient: document.getElementById("config-client"),
  configServer: document.getElementById("config-server"),
  configApp: document.getElementById("config-app"),
  configFallbacks: document.getElementById("config-fallbacks"),
  statusConnection: document.getElementById("status-connection"),
  statusMetrics: document.getElementById("status-metrics"),
  statusAudio: document.getElementById("status-audio"),
  statusParams: document.getElementById("status-params"),
  statusEvents: document.getElementById("status-events"),
  logList: document.getElementById("log-list"),
  logFilter: document.getElementById("log-filter"),
  logClear: document.getElementById("log-clear"),
  logExport: document.getElementById("log-export"),
};


const resolveTauriApi = () => {
  if (!window.__TAURI__) return null;
  const legacyInvoke =
    typeof window.__TAURI__.invoke === "function" ? window.__TAURI__.invoke : null;
  const coreInvoke =
    typeof window.__TAURI__.core?.invoke === "function"
      ? window.__TAURI__.core.invoke
      : null;
  const invoke = legacyInvoke || coreInvoke;
  if (!invoke) return null;
  const event =
    window.__TAURI__.event && typeof window.__TAURI__.event.listen === "function"
      ? window.__TAURI__.event
      : null;
  const currentWindow =
    typeof window.__TAURI__.webviewWindow?.getCurrentWebviewWindow === "function"
      ? window.__TAURI__.webviewWindow.getCurrentWebviewWindow()
      : typeof window.__TAURI__.window?.getCurrentWindow === "function"
        ? window.__TAURI__.window.getCurrentWindow()
        : null;
  return { invoke, event, currentWindow };
};

const createTauriAdapter = (tauriApi) => {
  const { invoke, event, currentWindow } = tauriApi;
  return {
    async getStatus() {
      return invoke("get_status");
    },
    async setMode(mode) {
      return invoke("set_mode", { mode });
    },
    async setClientConfig(config) {
      return invoke("set_client_config", { config });
    },
    async setServerConfig(config) {
      return invoke("set_server_config", { config });
    },
    async setLaunchAtLogin(enabled) {
      return invoke("set_launch_at_login", { enabled });
    },
    async hideToTray() {
      return invoke("hide_to_tray");
    },
    async minimizeWindow() {
      if (!currentWindow || typeof currentWindow.minimize !== "function") return false;
      await currentWindow.minimize();
      return true;
    },
    async toggleMaximizeWindow() {
      if (!currentWindow || typeof currentWindow.toggleMaximize !== "function") {
        return false;
      }
      await currentWindow.toggleMaximize();
      if (typeof currentWindow.isMaximized === "function") {
        return currentWindow.isMaximized();
      }
      return false;
    },
    async isWindowMaximized() {
      if (!currentWindow || typeof currentWindow.isMaximized !== "function") {
        return false;
      }
      return currentWindow.isMaximized();
    },
    async resetDefaults() {
      return invoke("reset_defaults");
    },
    async start() {
      return invoke("start");
    },
    async stop() {
      return invoke("stop");
    },
    async forceDisconnect() {
      return invoke("force_disconnect");
    },
    async createVirtualMic() {
      return invoke("virtual_mic_create");
    },
    async removeVirtualMic() {
      return invoke("virtual_mic_remove");
    },
    async clearLogs() {
      return invoke("clear_logs");
    },
    async exportLogs() {
      return invoke("export_logs");
    },
    async reportHarnessRender(ack) {
      return invoke("report_harness_render", { ack });
    },
    async onCloseRequested(handler) {
      if (!currentWindow || typeof currentWindow.onCloseRequested !== "function") {
        return () => {};
      }
      return currentWindow.onCloseRequested(handler);
    },
    onSnapshot(handler) {
      if (!event || typeof event.listen !== "function") return;
      event.listen(EVENT_SNAPSHOT, (payload) => {
        if (payload && payload.payload) {
          handler(payload.payload);
        }
      });
    },
    onWaveform(handler) {
      if (!event || typeof event.listen !== "function") return;
      event.listen(EVENT_WAVEFORM, (payload) => {
        if (payload && payload.payload) {
          handler(payload.payload);
        }
      });
    },
  };
};

let hasTauri = false;
let adapter = createMockAdapter();

const activateTauriAdapter = () => {
  const tauriApi = resolveTauriApi();
  if (!tauriApi) return false;
  hasTauri = true;
  adapter = createTauriAdapter(tauriApi);
  return true;
};

const getState = () => state;

const setWindowState = (next) => {
  windowState = { ...windowState, ...next };
  renderAppActions({ elements, windowState });
};

const mergeSnapshot = (current, next) => {
  if (!current) return next;
  if (!next) return current;
  const currentUpdated = current.runtime?.server_status_updated_ms ?? 0;
  const nextUpdated = next.runtime?.server_status_updated_ms ?? 0;
  const nextStatus = next.status;
  const shouldKeepServerRuntime =
    next.mode === "server" &&
    nextStatus !== "idle" &&
    nextStatus !== "error" &&
    currentUpdated > 0 &&
    nextUpdated < currentUpdated;

  if (!shouldKeepServerRuntime) return next;

  const merged = { ...next };
  merged.runtime = { ...next.runtime, ...current.runtime };
  merged.metrics = current.metrics || next.metrics;
  merged.status = current.status;
  merged.status_note = current.status_note;
  if (Array.isArray(current.logs) && current.logs.length > (next.logs || []).length) {
    merged.logs = current.logs;
  }
  return merged;
};

const captureConfigFocus = () => {
  const active = document.activeElement;
  if (!active || !active.matches || !active.matches("[data-field]")) return null;
  const field = active.dataset?.field;
  if (!field) return null;
  return {
    field,
    type: active.type,
    value: active.value,
    checked: active.checked,
    selectionStart: typeof active.selectionStart === "number" ? active.selectionStart : null,
    selectionEnd: typeof active.selectionEnd === "number" ? active.selectionEnd : null,
  };
};

const restoreConfigFocus = (focusInfo) => {
  if (!focusInfo || !focusInfo.field) return;
  const target = document.querySelector(`[data-field="${focusInfo.field}"]`);
  if (!target || target.disabled) return;
  if (focusInfo.type === "checkbox") {
    target.checked = Boolean(focusInfo.checked);
  } else if (typeof focusInfo.value === "string") {
    target.value = focusInfo.value;
  }
  if (target.focus) {
    target.focus({ preventScroll: true });
  }
  if (
    typeof focusInfo.selectionStart === "number" &&
    typeof focusInfo.selectionEnd === "number" &&
    target.setSelectionRange
  ) {
    target.setSelectionRange(focusInfo.selectionStart, focusInfo.selectionEnd);
  }
};

const isConfigEditing = () => {
  if (document.querySelector("[data-field]:focus")) return true;
  const active = document.activeElement;
  if (!active) return false;
  if (active.matches && active.matches("[data-field]")) return true;
  if (active.closest) {
    return Boolean(active.closest("[data-field]"));
  }
  return false;
};

const registerCloseInterceptor = async () => {
  if (closeInterceptorAttached || !adapter.onCloseRequested) return;
  closeInterceptorAttached = true;
  await adapter.onCloseRequested(async (event) => {
    if (event && typeof event.preventDefault === "function") {
      event.preventDefault();
    }
    try {
      await adapter.hideToTray();
    } catch (_) {
      // 关闭拦截失败时不抛出到 UI 主循环。
    }
  });
};

const syncWindowState = async () => {
  if (!adapter.isWindowMaximized) {
    setWindowState({ maximized: false });
    return;
  }
  try {
    const maximized = await adapter.isWindowMaximized();
    setWindowState({ maximized: Boolean(maximized) });
  } catch (_) {
    setWindowState({ maximized: false });
  }
};

const setState = (snapshot, options = {}) => {
  const focusInfo = captureConfigFocus();
  state = mergeSnapshot(state, snapshot);
  const skipConfig = options.skipConfig ?? isConfigEditing();
  render({ skipConfig });
  restoreConfigFocus(focusInfo);
};

const setActiveTab = (tab) => {
  currentTab = tab;
  elements.navButtons.forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.tab === tab);
  });
  Object.entries(elements.views).forEach(([key, view]) => {
    view.classList.toggle("active", key === tab);
  });
};

const ensureWaveformCanvas = () => {
  waveformCanvas = document.getElementById("waveform-canvas");
  if (!waveformCanvas) return;
  waveformCtx = waveformCanvas.getContext("2d");
  resizeWaveformCanvas();
  drawWaveform();
};

const resizeWaveformCanvas = () => {
  if (!waveformCanvas || !waveformCtx) return;
  const rect = waveformCanvas.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return;
  waveformCanvas.width = Math.floor(rect.width);
  waveformCanvas.height = Math.floor(rect.height);
};

const drawWaveform = () => {
  if (!waveformCanvas || !waveformCtx) return;
  const width = waveformCanvas.width;
  const height = waveformCanvas.height;
  if (width === 0 || height === 0) return;

  waveformCtx.clearRect(0, 0, width, height);
  waveformCtx.strokeStyle = "rgba(15, 138, 123, 0.4)";
  waveformCtx.lineWidth = 1;
  waveformCtx.beginPath();
  waveformCtx.moveTo(0, height / 2);
  waveformCtx.lineTo(width, height / 2);
  waveformCtx.stroke();

  const points = waveformPoints.length ? waveformPoints : new Array(WAVEFORM_POINTS).fill(0);
  waveformCtx.strokeStyle = "rgba(15, 138, 123, 0.9)";
  waveformCtx.lineWidth = 2;
  waveformCtx.beginPath();
  points.forEach((value, idx) => {
    const x = (idx / (points.length - 1)) * width;
    const y = height / 2 - value * (height * 0.4);
    if (idx === 0) {
      waveformCtx.moveTo(x, y);
    } else {
      waveformCtx.lineTo(x, y);
    }
  });
  waveformCtx.stroke();
};

const updateWaveform = (payload) => {
  if (!payload) return;
  const nextPoints = Array.isArray(payload) ? payload : payload.points;
  if (!Array.isArray(nextPoints) || nextPoints.length === 0) return;
  waveformPoints = nextPoints;
  drawWaveform();
};

const scheduleAfterRender = (callback) => {
  let fired = false;
  const run = () => {
    if (fired) return;
    fired = true;
    callback();
  };
  if (typeof window.requestAnimationFrame === "function") {
    window.requestAnimationFrame(run);
  }
  window.setTimeout(run, 16);
};

const scheduleInterval = (callback, delayMs) => {
  const timer = window.setInterval(callback, delayMs);
  if (timer && typeof timer.unref === "function") {
    timer.unref();
  }
  return timer;
};

const waitForTauriAdapter = async (timeoutMs = 1500, pollMs = 25) => {
  if (activateTauriAdapter()) return true;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    await new Promise((resolve) => window.setTimeout(resolve, pollMs));
    if (activateTauriAdapter()) return true;
  }
  return false;
};

const htmlToLines = (html) =>
  String(html || "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|div|h3|li)>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

const collectHarnessVisibleState = () => ({
  active_tab: currentTab,
  status_label: elements.statusLabel.textContent || "",
  status_note: elements.statusNote.textContent || "",
  primary_action: elements.primaryAction.textContent || "",
  connection_lines: htmlToLines(elements.statusConnection.innerHTML),
  metrics_lines: htmlToLines(elements.statusMetrics.innerHTML),
  audio_lines: htmlToLines(elements.statusAudio.innerHTML),
  params_lines: htmlToLines(elements.statusParams.innerHTML),
  events_lines: htmlToLines(elements.statusEvents.innerHTML),
  config_connection_lines: htmlToLines(elements.configConnection.innerHTML),
  config_audio_lines: htmlToLines(elements.configAudio.innerHTML),
  config_client_lines: htmlToLines(elements.configClient.innerHTML),
  config_server_lines: htmlToLines(elements.configServer.innerHTML),
  fallback_lines: htmlToLines(elements.configFallbacks.innerHTML),
  log_filter: elements.logFilter.value || "all",
  log_lines: htmlToLines(elements.logList.innerHTML),
});

const reportHarnessRender = () => {
  if ((!hasTauri || !adapter.reportHarnessRender) && !activateTauriAdapter()) return;
  if (!adapter.reportHarnessRender || pendingHarnessRenderReport) return;
  pendingHarnessRenderReport = true;
  scheduleAfterRender(() => {
    pendingHarnessRenderReport = false;
    adapter.reportHarnessRender({
      snapshot: state,
      visible: collectHarnessVisibleState(),
    }).catch(() => {
      // Harness ack 失败时不打断正常 UI 渲染。
    });
  });
};

const render = ({ skipConfig = false } = {}) => {
  renderModeButtons({ state, elements, isBusy });
  renderStatusPill({ state, elements });
  renderPrimaryAction({ state, elements, isBusy });
  renderAppActions({ elements, windowState });
  if (!skipConfig) {
    renderConfig({
      state,
      elements,
      isBusy,
      sampleRates,
      chunkOptions,
      bufferOptions,
    });
    bindConfigInputs({
      root: document,
      getState,
      setState,
      adapter,
    });
  }
  renderStatus({ state, elements, ensureWaveformCanvas });
  renderLogs({ state, elements });
  reportHarnessRender();
};

const registerAdapterListeners = () => {
  if (adapter.onSnapshot) {
    adapter.onSnapshot((snapshot) => {
      setState(snapshot);
    });
  }
  if (adapter.onWaveform) {
    adapter.onWaveform((payload) => {
      updateWaveform(payload);
    });
  }
};

const startStatusPoller = () => {
  if (statusPoller) return;
  statusPoller = scheduleInterval(async () => {
    if ((!hasTauri || !adapter.getStatus) && !activateTauriAdapter()) return;
    if (getState().mode !== "client") return;
    try {
      const snapshot = await adapter.getStatus();
      setState(snapshot, { skipConfig: true });
    } catch (_) {
      // 保持事件驱动为主，轮询失败时不打断 UI。
    }
  }, 1500);
};

const init = async () => {
  await waitForTauriAdapter();
  bindActions({
    elements,
    getState,
    setState,
    adapter,
    root: document,
    setActiveTab,
    isBusy,
    renderLogs: () => renderLogs({ state, elements }),
    setWindowState,
  });
  setActiveTab(currentTab);
  window.addEventListener("resize", () => {
    resizeWaveformCanvas();
    drawWaveform();
  });

  // 先注册事件监听，避免错过启动阶段的首个 snapshot。
  registerAdapterListeners();
  await registerCloseInterceptor();
  await syncWindowState();

  try {
    const snapshot = await adapter.getStatus();
    setState(snapshot);
  } catch (err) {
    const recovered = await waitForTauriAdapter();
    if (recovered && adapter.getStatus) {
      try {
        const snapshot = await adapter.getStatus();
        setState(snapshot);
      } catch (retryErr) {
        adapter = createMockAdapter();
        hasTauri = false;
        registerAdapterListeners();
        const snapshot = await adapter.getStatus();
        setState(snapshot);
        state.logs.unshift({
          ts_ms: Date.now(),
          level: "error",
          message: `Tauri IPC 初始化失败，已切换模拟模式：${String(retryErr)}`,
        });
        state.logs = state.logs.slice(0, 200);
        render();
      }
    } else {
      adapter = createMockAdapter();
      hasTauri = false;
      registerAdapterListeners();
      const snapshot = await adapter.getStatus();
      setState(snapshot);
      state.logs.unshift({
        ts_ms: Date.now(),
        level: "error",
        message: `Tauri IPC 初始化失败，已切换模拟模式：${String(err)}`,
      });
      state.logs = state.logs.slice(0, 200);
      render();
    }
  }
  if (!hasTauri) {
    state.logs.unshift({
      ts_ms: Date.now(),
      level: "warn",
      message: "未检测到 Tauri API，已进入模拟模式",
    });
    state.logs = state.logs.slice(0, 200);
    render();
  }

  startStatusPoller();
};

init();
