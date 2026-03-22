import test from "node:test";
import assert from "node:assert/strict";
import {
  SERVER_STATUS_STALE_ALERT_MS,
  renderAppActions,
  renderConfig,
  renderLogs,
  renderModeButtons,
  renderPrimaryAction,
  renderStatus,
  renderStatusPill,
  statusMap,
} from "./app.dom.js";

const createClassList = () => ({
  toggles: {},
  toggle(name, active) {
    this.toggles[name] = active;
  },
});

const createButton = (mode) => ({
  dataset: { mode },
  classList: createClassList(),
  disabled: false,
  attributes: {},
  setAttribute(name, value) {
    this.attributes[name] = value;
  },
});

const createElements = () => {
  const statusDot = { style: {} };
  const createWindowControl = () => {
    const label = { textContent: "" };
    return {
      textContent: "",
      title: "",
      attributes: {},
      querySelector(selector) {
        return selector === ".window-control-label" ? label : null;
      },
      setAttribute(name, value) {
        this.attributes[name] = value;
        if (name === "title") this.title = value;
      },
      label,
    };
  };
  return {
    modeButtons: [createButton("client"), createButton("server")],
    statusLabel: { textContent: "" },
    statusNote: { textContent: "" },
    statusPill: {
      style: {},
      querySelector(selector) {
        return selector === ".status-dot" ? statusDot : null;
      },
    },
    primaryAction: { textContent: "" },
    windowMinimize: createWindowControl(),
    windowMaximize: createWindowControl(),
    windowClose: createWindowControl(),
    logFilter: { value: "all" },
    logList: { innerHTML: "" },
  };
};

const createConfigElements = () => ({
  configConnection: { innerHTML: "" },
  configAudio: { innerHTML: "" },
  configClient: { innerHTML: "" },
  configServer: { innerHTML: "" },
  configApp: { innerHTML: "" },
  configFallbacks: { innerHTML: "" },
});

const createStatusElements = () => ({
  statusConnection: { innerHTML: "" },
  statusMetrics: { innerHTML: "" },
  statusAudio: { innerHTML: "" },
  statusParams: { innerHTML: "" },
  statusEvents: { innerHTML: "" },
});

test("renderStatusPill updates label and color", () => {
  const elements = createElements();
  const state = { status: "connecting", status_note: "连接中" };
  renderStatusPill({ state, elements });
  assert.equal(elements.statusLabel.textContent, statusMap.connecting.label);
  assert.equal(elements.statusNote.textContent, "连接中");
  assert.equal(elements.statusPill.style.borderColor, statusMap.connecting.color);
  const dot = elements.statusPill.querySelector(".status-dot");
  assert.equal(dot.style.background, statusMap.connecting.color);
});

test("renderModeButtons toggles active and disabled states", () => {
  const elements = createElements();
  const isBusy = () => false;
  renderModeButtons({ state: { mode: "client" }, elements, isBusy });
  assert.equal(elements.modeButtons[0].classList.toggles.active, true);
  assert.equal(elements.modeButtons[0].attributes["aria-selected"], "true");
  assert.equal(elements.modeButtons[0].disabled, false);
  assert.equal(elements.modeButtons[1].classList.toggles.active, false);

  const busy = () => true;
  renderModeButtons({ state: { mode: "server" }, elements, isBusy: busy });
  assert.equal(elements.modeButtons[0].disabled, true);
  assert.equal(elements.modeButtons[1].disabled, true);
});

test("renderPrimaryAction switches labels by mode/busy", () => {
  const elements = createElements();
  renderPrimaryAction({
    state: { mode: "client", status: "idle" },
    elements,
    isBusy: () => false,
  });
  assert.equal(elements.primaryAction.textContent, "开始推流");

  renderPrimaryAction({
    state: { mode: "client", status: "streaming" },
    elements,
    isBusy: () => true,
  });
  assert.equal(elements.primaryAction.textContent, "停止推流");

  renderPrimaryAction({
    state: { mode: "server", status: "idle" },
    elements,
    isBusy: () => false,
  });
  assert.equal(elements.primaryAction.textContent, "开始监听");
});

test("renderAppActions sets custom window control labels", () => {
  const elements = createElements();
  renderAppActions({ elements, windowState: { maximized: false } });
  assert.equal(elements.windowMinimize.label.textContent, "最小化");
  assert.equal(elements.windowMaximize.label.textContent, "最大化");
  assert.equal(elements.windowClose.label.textContent, "关闭");
  assert.match(elements.windowClose.title, /关闭到后台/);

  renderAppActions({ elements, windowState: { maximized: true } });
  assert.equal(elements.windowMaximize.label.textContent, "还原");
});

test("renderLogs respects log level filter", () => {
  const elements = createElements();
  const state = {
    logs: [
      { ts_ms: 1, level: "info", message: "hello-info" },
      { ts_ms: 2, level: "warn", message: "hello-warn" },
    ],
  };
  elements.logFilter.value = "warn";
  renderLogs({ state, elements });
  assert.ok(elements.logList.innerHTML.includes("hello-warn"));
  assert.ok(!elements.logList.innerHTML.includes("hello-info"));
});

test("renderStatus writes key status fields and events", () => {
  const elements = createStatusElements();
  const state = {
    mode: "client",
    status: "streaming",
    status_note: "推流中",
    server_config: { listen_port: 43000 },
    runtime: {
      connected_seconds: 65,
      peer_addr: "127.0.0.1:43000",
      last_error: "握手失败：服务端忙",
    },
    metrics: {
      rtt_ms: 5,
      packet_loss_pct: 0.5,
      buffer_depth_ms: 90,
      estimated_e2e_latency_ms: 110,
      audio_rms: 0.5,
      audio_peak: 1024,
      uplink_kbps: 48,
      jitter_buffer_depth_ms: 80,
    },
    effective: {
      codec: "opus",
      sample_rate_hz: 48000,
      channels: 1,
      chunk_ms: 20,
      opus_bitrate_kbps: 48,
      jitter_buffer_ms: 100,
    },
    logs: [
      { level: "info", message: "hello-status" },
      { level: "warn", message: "warn-status" },
    ],
  };
  let waveformCalled = false;
  renderStatus({
    state,
    elements,
    ensureWaveformCanvas: () => {
      waveformCalled = true;
    },
  });
  assert.ok(elements.statusConnection.innerHTML.includes("模式：Client"));
  assert.ok(elements.statusConnection.innerHTML.includes(statusMap.streaming.label));
  assert.ok(elements.statusConnection.innerHTML.includes("对端：127.0.0.1:43000"));
  assert.ok(elements.statusConnection.innerHTML.includes("错误提示：握手失败：服务端忙"));
  assert.ok(elements.statusParams.innerHTML.includes("Codec：opus"));
  assert.ok(elements.statusParams.innerHTML.includes("采样率：48000 Hz"));
  assert.ok(elements.statusEvents.innerHTML.includes("hello-status"));
  assert.equal(waveformCalled, true);
});

test("renderStatus shows stale alert when server status is outdated", () => {
  const elements = createStatusElements();
  const now = Date.now();
  const state = {
    mode: "server",
    status: "listening",
    status_note: "等待客户端连接",
    server_config: { listen_port: 43000 },
    runtime: {
      connected_seconds: 0,
      peer_addr: null,
      last_error: null,
      virtual_mic_name: "NetMic Virtual Mic",
      virtual_mic_ready: true,
      virtual_mic_error: null,
      server_status_updated_ms: now - SERVER_STATUS_STALE_ALERT_MS - 1200,
    },
    metrics: {
      rtt_ms: 0,
      packet_loss_pct: 0,
      buffer_depth_ms: 0,
      estimated_e2e_latency_ms: 0,
      audio_rms: 0,
      audio_peak: 0,
      uplink_kbps: 0,
      jitter_buffer_depth_ms: 0,
    },
    effective: {
      codec: "opus",
      sample_rate_hz: 48000,
      channels: 1,
      chunk_ms: 20,
      opus_bitrate_kbps: 48,
      jitter_buffer_ms: 100,
    },
    logs: [{ level: "warn", message: "status stale" }],
  };

  renderStatus({
    state,
    elements,
    ensureWaveformCanvas: () => {},
  });

  assert.ok(elements.statusConnection.innerHTML.includes("服务端状态已过期"));
  assert.ok(elements.statusConnection.innerHTML.includes("状态更新距今"));
});

test("renderConfig switches between client/server forms", () => {
  const elements = createConfigElements();
  const state = {
    mode: "client",
    client_config: {
      server_addr: "10.0.0.9",
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
      virtual_mic_enabled: false,
    },
    app_settings: {
      launch_at_login: true,
    },
    devices: { input: ["系统默认", "USB Mic"] },
    runtime: { mic_permission: "已授权", virtual_mic_name: "NetMic Virtual Mic" },
    fallbacks: [],
  };
  renderConfig({
    state,
    elements,
    isBusy: () => false,
    sampleRates: [16000, 48000],
    chunkOptions: [10, 20],
    bufferOptions: [40, 100],
  });
  assert.ok(elements.configConnection.innerHTML.includes("Server IP"));
  assert.ok(elements.configConnection.innerHTML.includes("10.0.0.9"));
  assert.ok(elements.configAudio.innerHTML.includes("Opus"));
  assert.ok(elements.configClient.innerHTML.includes("输入设备"));
  assert.ok(elements.configApp.innerHTML.includes("开机自启"));
  assert.ok(elements.configApp.innerHTML.includes("隐藏到后台"));

  state.mode = "server";
  renderConfig({
    state,
    elements,
    isBusy: () => true,
    sampleRates: [16000, 48000],
    chunkOptions: [10, 20],
    bufferOptions: [40, 100],
  });
  assert.ok(elements.configConnection.innerHTML.includes("监听端口"));
  assert.ok(elements.configConnection.innerHTML.includes("disabled"));
  assert.ok(elements.configApp.innerHTML.includes("托盘主开关"));
});
