const UI_VERSION = "0.1.0";
const EVENT_SNAPSHOT = "netmic://snapshot";

const defaultSnapshot = () => ({
  mode: "client",
  status: "idle",
  status_note: "准备就绪",
  config: {
    server_addr: "127.0.0.1",
    server_port: 43000,
    listen_port: 43000,
    input_device: "系统默认",
    codec: "opus",
    sample_rate_hz: 48000,
    channels: 1,
    chunk_ms: 20,
    opus_bitrate_kbps: 48,
    jitter_buffer_ms: 100,
    auto_reconnect: true,
    force_takeover: false,
    pairing_token: "",
    virtual_mic_enabled: false,
  },
  effective: {
    codec: "opus",
    sample_rate_hz: 48000,
    channels: 1,
    chunk_ms: 20,
    opus_bitrate_kbps: 48,
    jitter_buffer_ms: 100,
  },
  fallbacks: [],
  metrics: {
    rtt_ms: 0,
    packet_loss_pct: 0,
    buffer_depth_ms: 0,
    jitter_buffer_depth_ms: 0,
    estimated_e2e_latency_ms: 0,
    audio_rms: 0,
    audio_peak: 0,
    uplink_kbps: 0,
  },
  runtime: {
    peer_addr: null,
    connected_seconds: 0,
    reconnect_attempts: 0,
    mic_permission: "unknown",
    virtual_mic_name: "NetMic Virtual Mic",
  },
  devices: {
    input: ["系统默认", "USB Mic"],
  },
  logs: [
    {
      ts_ms: Date.now(),
      level: "info",
      message: `UI ${UI_VERSION} 已就绪（模拟数据）`,
    },
  ],
});

let state = defaultSnapshot();
let currentTab = "config";

const sampleRates = [16000, 24000, 32000, 44100, 48000];
const chunkOptions = [10, 20, 40, 60];
const bufferOptions = [40, 60, 80, 100, 150, 200, 300, 400];

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
  resetDefaults: document.getElementById("reset-defaults"),
  configConnection: document.getElementById("config-connection"),
  configAudio: document.getElementById("config-audio"),
  configClient: document.getElementById("config-client"),
  configServer: document.getElementById("config-server"),
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

const formatNumber = (value, digits = 1) =>
  Number.isFinite(value) ? value.toFixed(digits) : "--";

const formatMs = (value) => `${formatNumber(value)} ms`;

const formatKbps = (value) => `${formatNumber(value)} kbps`;

const formatDuration = (seconds) => {
  if (!Number.isFinite(seconds)) return "--";
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}m ${secs}s`;
};

const formatTimestamp = (ms) => {
  const date = new Date(ms);
  return date.toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
};

const statusMap = {
  idle: { label: "空闲", color: "var(--muted)" },
  connecting: { label: "连接中", color: "var(--accent-2)" },
  listening: { label: "监听中", color: "var(--accent)" },
  connected: { label: "已连接", color: "var(--success)" },
  streaming: { label: "推流中", color: "var(--success)" },
  error: { label: "错误", color: "var(--danger)" },
};

const isBusy = (snapshot) =>
  ["connecting", "streaming", "listening", "connected"].includes(snapshot.status);

const deepClone = (value) => JSON.parse(JSON.stringify(value));

const createMockAdapter = () => {
  let mockState = deepClone(state);
  let listeners = [];
  let ticker = null;
  let startAt = null;

  const emit = () => {
    const snapshot = deepClone(mockState);
    listeners.forEach((handler) => handler(snapshot));
  };

  const pushLog = (level, message) => {
    mockState.logs.unshift({ ts_ms: Date.now(), level, message });
    mockState.logs = mockState.logs.slice(0, 200);
  };

  const updateMetrics = () => {
    if (!isBusy(mockState)) return;
    const now = Date.now();
    if (!startAt) startAt = now;
    const t = (now - startAt) / 1000;
    mockState.runtime.connected_seconds = t;
    mockState.metrics.rtt_ms = 4 + Math.abs(Math.sin(t)) * 6;
    mockState.metrics.packet_loss_pct = Math.abs(Math.cos(t / 2)) * 1.8;
    mockState.metrics.buffer_depth_ms = 80 + Math.abs(Math.sin(t / 1.8)) * 40;
    mockState.metrics.jitter_buffer_depth_ms = 60 + Math.abs(Math.sin(t / 2.6)) * 30;
    mockState.metrics.estimated_e2e_latency_ms =
      mockState.metrics.buffer_depth_ms + mockState.metrics.rtt_ms + 12;
    mockState.metrics.audio_rms = Math.abs(Math.sin(t * 1.2)) * 0.7;
    mockState.metrics.audio_peak = Math.floor(6000 + Math.abs(Math.sin(t * 1.4)) * 12000);
    mockState.metrics.uplink_kbps =
      mockState.config.codec === "opus"
        ? mockState.config.opus_bitrate_kbps || 48
        : (mockState.config.sample_rate_hz * 16) / 1000;
  };

  const startTicker = () => {
    if (ticker) return;
    ticker = setInterval(() => {
      updateMetrics();
      emit();
    }, 1000);
  };

  const stopTicker = () => {
    if (ticker) {
      clearInterval(ticker);
      ticker = null;
    }
  };

  const applyConfig = (config) => {
    mockState.config = { ...mockState.config, ...config };
    mockState.effective = {
      codec: mockState.config.codec,
      sample_rate_hz: mockState.config.sample_rate_hz,
      channels: 1,
      chunk_ms: mockState.config.chunk_ms,
      opus_bitrate_kbps:
        mockState.config.codec === "opus" ? mockState.config.opus_bitrate_kbps : null,
      jitter_buffer_ms: mockState.config.jitter_buffer_ms,
    };
    mockState.fallbacks = [];
  };

  return {
    async getStatus() {
      return deepClone(mockState);
    },
    async setMode(mode) {
      mockState.mode = mode;
      mockState.status = "idle";
      mockState.status_note = "准备就绪";
      mockState.runtime.peer_addr = null;
      mockState.runtime.connected_seconds = 0;
      pushLog("info", `切换到 ${mode === "client" ? "Client" : "Server"} 模式`);
      emit();
      return deepClone(mockState);
    },
    async setConfig(config) {
      applyConfig(config);
      pushLog("info", "已更新配置（模拟）");
      emit();
      return deepClone(mockState);
    },
    async resetDefaults() {
      mockState = defaultSnapshot();
      pushLog("info", "已恢复默认配置（模拟）");
      emit();
      return deepClone(mockState);
    },
    async start() {
      if (mockState.mode === "client") {
        mockState.status = "streaming";
        mockState.status_note = "模拟推流中";
        mockState.runtime.peer_addr = `${mockState.config.server_addr}:${mockState.config.server_port}`;
      } else {
        mockState.status = "listening";
        mockState.status_note = "等待客户端连接";
        mockState.runtime.peer_addr = null;
      }
      pushLog("info", "开始运行（模拟）");
      startTicker();
      emit();
      return deepClone(mockState);
    },
    async stop() {
      mockState.status = "idle";
      mockState.status_note = "已停止";
      mockState.runtime.peer_addr = null;
      mockState.runtime.connected_seconds = 0;
      pushLog("warn", "已停止运行（模拟）");
      stopTicker();
      emit();
      return deepClone(mockState);
    },
    async forceDisconnect() {
      mockState.runtime.peer_addr = null;
      mockState.status = "listening";
      mockState.status_note = "已断开客户端";
      pushLog("warn", "已强制断开客户端（模拟）");
      emit();
      return deepClone(mockState);
    },
    async clearLogs() {
      mockState.logs = [];
      emit();
      return deepClone(mockState);
    },
    async exportLogs() {
      pushLog("info", "日志导出（模拟）");
      emit();
      return { ok: true };
    },
    onSnapshot(handler) {
      listeners.push(handler);
      startTicker();
    },
  };
};

const createTauriAdapter = () => {
  const { invoke, event } = window.__TAURI__;
  return {
    async getStatus() {
      return invoke("get_status");
    },
    async setMode(mode) {
      return invoke("set_mode", { mode });
    },
    async setConfig(config) {
      return invoke("set_config", { config });
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
    async clearLogs() {
      return invoke("clear_logs");
    },
    async exportLogs() {
      return invoke("export_logs");
    },
    onSnapshot(handler) {
      event.listen(EVENT_SNAPSHOT, (payload) => {
        if (payload && payload.payload) {
          handler(payload.payload);
        }
      });
    },
  };
};

const adapter = window.__TAURI__ ? createTauriAdapter() : createMockAdapter();

const setState = (snapshot) => {
  state = snapshot;
  render();
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

const renderStatusPill = () => {
  const config = statusMap[state.status] || statusMap.idle;
  elements.statusLabel.textContent = config.label;
  elements.statusNote.textContent = state.status_note || "";
  elements.statusPill.style.borderColor = config.color;
  elements.statusPill.querySelector(".status-dot").style.background = config.color;
};

const renderModeButtons = () => {
  elements.modeButtons.forEach((btn) => {
    const active = btn.dataset.mode === state.mode;
    btn.classList.toggle("active", active);
    btn.setAttribute("aria-selected", active ? "true" : "false");
    btn.disabled = isBusy(state);
  });
};

const renderPrimaryAction = () => {
  if (isBusy(state)) {
    elements.primaryAction.textContent = state.mode === "client" ? "停止推流" : "停止监听";
    return;
  }
  elements.primaryAction.textContent = state.mode === "client" ? "开始推流" : "开始监听";
};

const renderConfig = () => {
  const isRunning = isBusy(state);
  elements.configConnection.innerHTML = `
    <h3>连接配置</h3>
    <p>与服务端建立 UDP 会话（默认端口 43000）。</p>
    ${
      state.mode === "client"
        ? `
        <div class="inline-row">
          <div class="form-row">
            <label>Server IP</label>
            <input data-field="server_addr" value="${state.config.server_addr}" ${
              isRunning ? "disabled" : ""
            } />
          </div>
          <div class="form-row">
            <label>端口</label>
            <input type="number" data-field="server_port" value="${state.config.server_port}" ${
              isRunning ? "disabled" : ""
            } />
          </div>
        </div>
        `
        : `
        <div class="form-row">
          <label>监听端口</label>
          <input type="number" data-field="listen_port" value="${state.config.listen_port}" ${
            isRunning ? "disabled" : ""
          } />
        </div>
        `
    }
  `;

  elements.configAudio.innerHTML = `
    <h3>音频参数</h3>
    <p>所有参数必须在 MVP 安全范围内。</p>
    <div class="form-row">
      <label>编码器</label>
      <select data-field="codec" ${isRunning ? "disabled" : ""}>
        <option value="opus" ${state.config.codec === "opus" ? "selected" : ""}>Opus</option>
        <option value="pcm16" ${state.config.codec === "pcm16" ? "selected" : ""}>PCM16</option>
      </select>
    </div>
    <div class="inline-row">
      <div class="form-row">
        <label>采样率</label>
        <select data-field="sample_rate_hz" ${isRunning ? "disabled" : ""}>
          ${sampleRates
            .map(
              (rate) =>
                `<option value="${rate}" ${
                  state.config.sample_rate_hz === rate ? "selected" : ""
                }>${rate} Hz</option>`
            )
            .join("")}
        </select>
      </div>
      <div class="form-row">
        <label>帧长（chunk）</label>
        <select data-field="chunk_ms" ${isRunning ? "disabled" : ""}>
          ${chunkOptions
            .map(
              (ms) =>
                `<option value="${ms}" ${state.config.chunk_ms === ms ? "selected" : ""}>${
                  ms
                } ms</option>`
            )
            .join("")}
        </select>
      </div>
    </div>
    <div class="inline-row">
      <div class="form-row">
        <label>Opus 比特率</label>
        <input type="number" data-field="opus_bitrate_kbps" value="${
          state.config.opus_bitrate_kbps
        }" ${
    isRunning || state.config.codec !== "opus" ? "disabled" : ""
  } />
      </div>
      <div class="form-row">
        <label>缓冲（buffer）</label>
        <select data-field="jitter_buffer_ms" ${isRunning ? "disabled" : ""}>
          ${bufferOptions
            .map(
              (ms) =>
                `<option value="${ms}" ${
                  state.config.jitter_buffer_ms === ms ? "selected" : ""
                }>${ms} ms</option>`
            )
            .join("")}
        </select>
      </div>
    </div>
    <div class="badge">内部标准：PCM16 / mono / 48k</div>
  `;

  elements.configClient.innerHTML = `
    <h3>客户端设置</h3>
    <p>输入设备与重连策略。</p>
    <div class="form-row">
      <label>输入设备</label>
      <select data-field="input_device" ${isRunning ? "disabled" : ""}>
        ${state.devices.input
          .map(
            (device) =>
              `<option value="${device}" ${
                state.config.input_device === device ? "selected" : ""
              }>${device}</option>`
          )
          .join("")}
      </select>
    </div>
    <label class="toggle">
      <input type="checkbox" data-field="auto_reconnect" ${
        state.config.auto_reconnect ? "checked" : ""
      } ${isRunning ? "disabled" : ""} />
      自动重连（指数退避）
    </label>
    <div class="list-item">麦克风权限：${state.runtime.mic_permission}</div>
  `;

  elements.configServer.innerHTML = `
    <h3>服务端设置</h3>
    <p>虚拟麦克风与占用策略。</p>
    <label class="toggle">
      <input type="checkbox" data-field="virtual_mic_enabled" ${
        state.config.virtual_mic_enabled ? "checked" : ""
      } />
      启用虚拟麦克风（${state.runtime.virtual_mic_name}）
    </label>
    <label class="toggle">
      <input type="checkbox" data-field="force_takeover" ${
        state.config.force_takeover ? "checked" : ""
      } ${isRunning ? "disabled" : ""} />
      允许强制抢占（默认关闭）
    </label>
    <div class="form-row">
      <label>配对码（预留）</label>
      <input data-field="pairing_token" value="${state.config.pairing_token}" disabled />
    </div>
    <button class="ghost" id="force-disconnect" ${
      state.mode === "server" && state.runtime.peer_addr ? "" : "disabled"
    }>强制断开客户端</button>
  `;

  const fallbackList = state.fallbacks
    .map(
      (item) =>
        `<div class="list-item">${item.field}：${item.requested} → ${item.applied}（${
          item.reason
        }）</div>`
    )
    .join("");

  elements.configFallbacks.innerHTML = `
    <h3>回退记录</h3>
    <p>当参数不在安全范围内时自动回退。</p>
    <div class="list">
      ${fallbackList || "<div class=\"list-item\">暂无回退记录</div>"}
    </div>
  `;

  bindConfigInputs();
};

const renderStatus = () => {
  elements.statusConnection.innerHTML = `
    <h3>连接状态</h3>
    <p>${state.status_note || ""}</p>
    <div class="list">
      <div class="list-item">模式：${state.mode === "client" ? "Client" : "Server"}</div>
      <div class="list-item">状态：${statusMap[state.status]?.label || "--"}</div>
      <div class="list-item">连接时长：${formatDuration(state.runtime.connected_seconds)}</div>
      <div class="list-item">对端：${state.runtime.peer_addr || "未连接"}</div>
    </div>
  `;

  elements.statusMetrics.innerHTML = `
    <h3>网络指标</h3>
    <p>来自统计快照（实时刷新）。</p>
    <div class="stats">
      <div class="stat">
        <div class="label">RTT</div>
        <div class="value">${formatMs(state.metrics.rtt_ms)}</div>
      </div>
      <div class="stat">
        <div class="label">丢包率</div>
        <div class="value">${formatNumber(state.metrics.packet_loss_pct)}%</div>
      </div>
      <div class="stat">
        <div class="label">缓冲深度</div>
        <div class="value">${formatMs(state.metrics.buffer_depth_ms)}</div>
      </div>
      <div class="stat">
        <div class="label">端到端延迟</div>
        <div class="value">${formatMs(state.metrics.estimated_e2e_latency_ms)}</div>
      </div>
    </div>
  `;

  elements.statusAudio.innerHTML = `
    <h3>音频电平</h3>
    <p>RMS / Peak 仅作趋势参考。</p>
    <div class="stats">
      <div class="stat">
        <div class="label">RMS</div>
        <div class="value">${formatNumber(state.metrics.audio_rms, 2)}</div>
      </div>
      <div class="stat">
        <div class="label">Peak</div>
        <div class="value">${state.metrics.audio_peak || "--"}</div>
      </div>
      <div class="stat">
        <div class="label">上行带宽</div>
        <div class="value">${formatKbps(state.metrics.uplink_kbps)}</div>
      </div>
      <div class="stat">
        <div class="label">抖动缓冲</div>
        <div class="value">${formatMs(state.metrics.jitter_buffer_depth_ms)}</div>
      </div>
    </div>
  `;

  elements.statusParams.innerHTML = `
    <h3>当前生效参数</h3>
    <p>实际运行中的会话参数。</p>
    <div class="list">
      <div class="list-item">Codec：${state.effective.codec}</div>
      <div class="list-item">采样率：${state.effective.sample_rate_hz} Hz</div>
      <div class="list-item">声道：${state.effective.channels}</div>
      <div class="list-item">Chunk：${state.effective.chunk_ms} ms</div>
      <div class="list-item">Opus Bitrate：${
        state.effective.opus_bitrate_kbps ?? "--"
      }</div>
      <div class="list-item">Buffer：${state.effective.jitter_buffer_ms} ms</div>
    </div>
  `;

  elements.statusEvents.innerHTML = `
    <h3>最近事件</h3>
    <p>用于排障的关键提示。</p>
    <div class="list">
      ${
        state.logs
          .slice(0, 4)
          .map(
            (log) =>
              `<div class="list-item">[${log.level.toUpperCase()}] ${log.message}</div>`
          )
          .join("") || "<div class=\"list-item\">暂无事件</div>"
      }
    </div>
  `;
};

const renderLogs = () => {
  const filter = elements.logFilter.value || "all";
  const logs = state.logs.filter((log) => filter === "all" || log.level === filter);
  elements.logList.innerHTML = logs
    .map(
      (log) => `
      <div class="log-item">
        <div>${formatTimestamp(log.ts_ms)}</div>
        <div class="level level-${log.level}">${log.level.toUpperCase()}</div>
        <div>${log.message}</div>
      </div>
    `
    )
    .join("");
};

const render = () => {
  renderModeButtons();
  renderStatusPill();
  renderPrimaryAction();
  renderConfig();
  renderStatus();
  renderLogs();
};

const bindConfigInputs = () => {
  document.querySelectorAll("[data-field]").forEach((input) => {
    input.addEventListener("change", async (event) => {
      const target = event.target;
      const field = target.dataset.field;
      let value = target.value;
      if (target.type === "checkbox") {
        value = target.checked;
      }
      if (target.type === "number") {
        value = Number(value);
      }

      const nextConfig = { ...state.config, [field]: value };
      const snapshot = await adapter.setConfig(nextConfig);
      setState(snapshot);
    });
  });

  const forceButton = document.getElementById("force-disconnect");
  if (forceButton) {
    forceButton.addEventListener("click", async () => {
      const snapshot = await adapter.forceDisconnect();
      setState(snapshot);
    });
  }
};

const bindActions = () => {
  elements.modeButtons.forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (isBusy(state)) return;
      const snapshot = await adapter.setMode(btn.dataset.mode);
      setState(snapshot);
    });
  });

  elements.navButtons.forEach((btn) => {
    btn.addEventListener("click", () => setActiveTab(btn.dataset.tab));
  });

  elements.primaryAction.addEventListener("click", async () => {
    const snapshot = isBusy(state) ? await adapter.stop() : await adapter.start();
    setState(snapshot);
  });

  elements.resetDefaults.addEventListener("click", async () => {
    const snapshot = await adapter.resetDefaults();
    setState(snapshot);
  });

  elements.logFilter.addEventListener("change", renderLogs);
  elements.logClear.addEventListener("click", async () => {
    const snapshot = await adapter.clearLogs();
    setState(snapshot);
  });
  elements.logExport.addEventListener("click", async () => {
    await adapter.exportLogs();
  });
};

const init = async () => {
  bindActions();
  setActiveTab(currentTab);
  const snapshot = await adapter.getStatus();
  setState(snapshot);
  adapter.onSnapshot((snapshot) => {
    setState(snapshot);
  });
};

init();
