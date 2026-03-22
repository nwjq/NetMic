export const UI_VERSION = "0.1.0";
export const WAVEFORM_POINTS = 128;
export const WAVEFORM_FPS = 20;
export const WAVEFORM_INTERVAL_MS = Math.floor(1000 / WAVEFORM_FPS);

export const defaultSnapshot = () => ({
  mode: "client",
  status: "idle",
  status_note: "准备就绪",
  client_config: {
    server_addr: "127.0.0.1",
    server_port: 43000,
    input_device: "系统默认",
    codec: "opus",
    sample_rate_hz: 48000,
    channels: 1,
    chunk_ms: 20,
    opus_bitrate_kbps: 48,
    jitter_buffer_ms: 100,
    auto_reconnect: true,
    pairing_token: "",
  },
  server_config: {
    listen_port: 43000,
    force_takeover: false,
    virtual_mic_enabled: true,
  },
  app_settings: {
    launch_at_login: false,
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
    mic_permission: "未知",
    virtual_mic_name: "NetMic Virtual Mic",
    virtual_mic_ready: false,
    virtual_mic_error: null,
    server_status_updated_ms: 0,
    last_error: null,
  },
  devices: {
    input: ["系统默认"],
  },
  logs: [
    {
      ts_ms: Date.now(),
      level: "info",
      message: `UI ${UI_VERSION} 已就绪（模拟数据）`,
    },
  ],
});

export const isBusy = (snapshot) =>
  ["connecting", "streaming", "listening", "connected"].includes(snapshot.status);

export const deepClone = (value) => JSON.parse(JSON.stringify(value));

const unrefTimer = (timer) => {
  if (timer && typeof timer.unref === "function") {
    timer.unref();
  }
  return timer;
};

export const createMockAdapter = () => {
  let mockState = deepClone(defaultSnapshot());
  let listeners = [];
  let waveListeners = [];
  let ticker = null;
  let waveTicker = null;
  let startAt = null;
  let windowMaximized = false;

  const emit = () => {
    const snapshot = deepClone(mockState);
    listeners.forEach((handler) => handler(snapshot));
  };

  const emitWaveform = (points) => {
    const payload = { ts_ms: Date.now(), points };
    waveListeners.forEach((handler) => handler(payload));
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
      mockState.client_config.codec === "opus"
        ? mockState.client_config.opus_bitrate_kbps || 48
        : (mockState.client_config.sample_rate_hz * 16) / 1000;
  };

  const updateWaveform = () => {
    if (!isBusy(mockState)) return;
    const now = Date.now();
    const t = now / 1000;
    const points = new Array(WAVEFORM_POINTS).fill(0).map((_, idx) => {
      const phase = t * 2 + idx / WAVEFORM_POINTS;
      return Math.sin(phase * Math.PI * 2) * 0.6;
    });
    emitWaveform(points);
  };

  const startTicker = () => {
    if (ticker) return;
    ticker = unrefTimer(setInterval(() => {
      updateMetrics();
      emit();
    }, 1000));
  };

  const startWaveTicker = () => {
    if (waveTicker) return;
    waveTicker = unrefTimer(setInterval(() => {
      updateWaveform();
    }, WAVEFORM_INTERVAL_MS));
  };

  const stopTicker = () => {
    if (ticker) {
      clearInterval(ticker);
      ticker = null;
    }
    if (waveTicker) {
      clearInterval(waveTicker);
      waveTicker = null;
    }
  };

  const applyClientConfig = (config) => {
    mockState.client_config = { ...mockState.client_config, ...config };
    mockState.effective = {
      codec: mockState.client_config.codec,
      sample_rate_hz: mockState.client_config.sample_rate_hz,
      channels: 1,
      chunk_ms: mockState.client_config.chunk_ms,
      opus_bitrate_kbps:
        mockState.client_config.codec === "opus"
          ? mockState.client_config.opus_bitrate_kbps
          : null,
      jitter_buffer_ms: mockState.client_config.jitter_buffer_ms,
    };
    mockState.fallbacks = [];
  };

  const applyServerConfig = (config) => {
    mockState.server_config = { ...mockState.server_config, ...config };
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
      mockState.runtime.last_error = null;
      mockState.runtime.mic_permission = mode === "server" ? "不适用" : "未知";
      pushLog("info", `切换到 ${mode === "client" ? "Client" : "Server"} 模式`);
      emit();
      return deepClone(mockState);
    },
    async setClientConfig(config) {
      applyClientConfig(config);
      pushLog("info", "已更新客户端配置（模拟）");
      emit();
      return deepClone(mockState);
    },
    async setServerConfig(config) {
      applyServerConfig(config);
      pushLog("info", "已更新服务端配置（模拟）");
      emit();
      return deepClone(mockState);
    },
    async setLaunchAtLogin(enabled) {
      mockState.app_settings = {
        ...mockState.app_settings,
        launch_at_login: Boolean(enabled),
      };
      pushLog(
        "info",
        mockState.app_settings.launch_at_login ? "已开启开机自启（模拟）" : "已关闭开机自启（模拟）"
      );
      emit();
      return deepClone(mockState);
    },
    async hideToTray() {
      pushLog("info", "已隐藏到后台（模拟）");
      emit();
      return true;
    },
    async minimizeWindow() {
      pushLog("info", "已最小化窗口（模拟）");
      emit();
      return true;
    },
    async toggleMaximizeWindow() {
      windowMaximized = !windowMaximized;
      pushLog("info", windowMaximized ? "已最大化窗口（模拟）" : "已还原窗口（模拟）");
      emit();
      return windowMaximized;
    },
    async isWindowMaximized() {
      return windowMaximized;
    },
    async onCloseRequested() {
      return () => {};
    },
    async resetDefaults() {
      const preservedSettings = deepClone(mockState.app_settings);
      mockState = defaultSnapshot();
      mockState.app_settings = preservedSettings;
      pushLog("info", "已恢复默认配置（模拟）");
      emit();
      return deepClone(mockState);
    },
    async start() {
      if (mockState.mode === "client") {
        mockState.status = "streaming";
        mockState.status_note = "模拟推流中";
        mockState.runtime.peer_addr = `${mockState.client_config.server_addr}:${mockState.client_config.server_port}`;
        mockState.runtime.last_error = null;
      } else {
        mockState.status = "listening";
        mockState.status_note = "等待客户端连接";
        mockState.runtime.peer_addr = null;
        mockState.runtime.last_error = null;
        if (mockState.server_config.virtual_mic_enabled) {
          mockState.runtime.virtual_mic_ready = true;
          mockState.runtime.virtual_mic_error = null;
        } else {
          mockState.runtime.virtual_mic_ready = false;
          mockState.runtime.virtual_mic_error = "未启用虚拟麦克风";
        }
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
      mockState.runtime.last_error = null;
      pushLog("warn", "已停止运行（模拟）");
      stopTicker();
      emit();
      return deepClone(mockState);
    },
    async forceDisconnect() {
      mockState.runtime.peer_addr = null;
      mockState.status = "listening";
      mockState.status_note = "已断开客户端";
      mockState.runtime.last_error = null;
      pushLog("warn", "已强制断开客户端（模拟）");
      emit();
      return deepClone(mockState);
    },
    async createVirtualMic() {
      mockState.runtime.virtual_mic_ready = true;
      mockState.runtime.virtual_mic_error = null;
      pushLog("info", "已创建虚拟麦克风（模拟）");
      emit();
      return deepClone(mockState);
    },
    async removeVirtualMic() {
      mockState.runtime.virtual_mic_ready = false;
      mockState.runtime.virtual_mic_error = "已移除虚拟麦克风（模拟）";
      pushLog("warn", "已移除虚拟麦克风（模拟）");
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
    onWaveform(handler) {
      waveListeners.push(handler);
      startWaveTicker();
    },
  };
};
