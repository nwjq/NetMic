import test from "node:test";
import assert from "node:assert/strict";
import { bindActions, bindConfigInputs } from "./app.interactions.js";

const createEmitter = () => ({
  handlers: {},
  addEventListener(event, handler) {
    this.handlers[event] = handler;
  },
});

const createButton = (dataset = {}) => ({
  dataset,
  ...createEmitter(),
});

const createInput = ({ field, type = "text", value = "" }) => ({
  dataset: { field },
  type,
  value,
  checked: false,
  ...createEmitter(),
});

const createAppInput = ({ field, type = "checkbox" }) => ({
  dataset: { appField: field },
  type,
  value: "on",
  checked: false,
  ...createEmitter(),
});

test("bindActions triggers mode change when idle", async () => {
  const root = createEmitter();
  const elements = {
    modeButtons: [createButton({ mode: "client" }), createButton({ mode: "server" })],
    navButtons: [createButton({ tab: "config" })],
    primaryAction: createButton(),
    windowMinimize: createButton(),
    windowMaximize: createButton(),
    windowClose: createButton(),
    resetDefaults: createButton(),
    logFilter: createEmitter(),
    logClear: createButton(),
    logExport: createButton(),
  };
  let state = { mode: "client", status: "idle" };
  let setModeCalled = null;
  let setStateValue = null;
  const adapter = {
    async setMode(mode) {
      setModeCalled = mode;
      return { mode };
    },
    async start() {
      return { status: "streaming" };
    },
    async stop() {
      return { status: "idle" };
    },
    async resetDefaults() {
      return { status: "idle" };
    },
    async hideToTray() {
      return true;
    },
    async minimizeWindow() {
      return true;
    },
    async toggleMaximizeWindow() {
      return false;
    },
    async clearLogs() {
      return { logs: [] };
    },
    async exportLogs() {},
  };

  bindActions({
    elements,
    getState: () => state,
    setState: (snapshot) => {
      setStateValue = snapshot;
      state = { ...state, ...snapshot };
    },
    adapter,
    root,
    setActiveTab: () => {},
    isBusy: () => false,
    renderLogs: () => {},
    setWindowState: () => {},
  });

  await elements.modeButtons[1].handlers.click();
  assert.equal(setModeCalled, "server");
  assert.deepEqual(setStateValue, { mode: "server" });
});

test("bindActions blocks mode change when busy", async () => {
  const root = createEmitter();
  const elements = {
    modeButtons: [createButton({ mode: "client" })],
    navButtons: [],
    primaryAction: createButton(),
    windowMinimize: createButton(),
    windowMaximize: createButton(),
    windowClose: createButton(),
    resetDefaults: createButton(),
    logFilter: createEmitter(),
    logClear: createButton(),
    logExport: createButton(),
  };
  const state = { mode: "client", status: "streaming" };
  let setModeCalled = null;
  const adapter = {
    async setMode(mode) {
      setModeCalled = mode;
      return { mode };
    },
    async start() {
      return {};
    },
    async stop() {
      return {};
    },
    async resetDefaults() {
      return {};
    },
    async hideToTray() {
      return true;
    },
    async minimizeWindow() {
      return true;
    },
    async toggleMaximizeWindow() {
      return false;
    },
    async clearLogs() {
      return {};
    },
    async exportLogs() {},
  };

  bindActions({
    elements,
    getState: () => state,
    setState: () => {},
    adapter,
    root,
    setActiveTab: () => {},
    isBusy: () => true,
    renderLogs: () => {},
    setWindowState: () => {},
  });

  await elements.modeButtons[0].handlers.click();
  assert.equal(setModeCalled, null);
});

test("bindActions toggles start/stop via primaryAction", async () => {
  const root = createEmitter();
  const elements = {
    modeButtons: [],
    navButtons: [],
    primaryAction: createButton(),
    windowMinimize: createButton(),
    windowMaximize: createButton(),
    windowClose: createButton(),
    resetDefaults: createButton(),
    logFilter: createEmitter(),
    logClear: createButton(),
    logExport: createButton(),
  };
  let state = { mode: "client", status: "idle" };
  let lastSnapshot = null;
  const adapter = {
    async start() {
      return { status: "streaming" };
    },
    async stop() {
      return { status: "idle" };
    },
    async resetDefaults() {
      return {};
    },
    async hideToTray() {
      return true;
    },
    async minimizeWindow() {
      return true;
    },
    async toggleMaximizeWindow() {
      return false;
    },
    async clearLogs() {
      return {};
    },
    async exportLogs() {},
    async setMode() {
      return {};
    },
  };

  let busy = false;
  bindActions({
    elements,
    getState: () => state,
    setState: (snapshot) => {
      lastSnapshot = snapshot;
      state = { ...state, ...snapshot };
    },
    adapter,
    root,
    setActiveTab: () => {},
    isBusy: () => busy,
    renderLogs: () => {},
    setWindowState: () => {},
  });

  await elements.primaryAction.handlers.click();
  assert.deepEqual(lastSnapshot, { status: "streaming" });
  busy = true;
  await elements.primaryAction.handlers.click();
  assert.deepEqual(lastSnapshot, { status: "idle" });
});

test("bindActions maps window shortcuts to custom window actions", async () => {
  const root = createEmitter();
  const elements = {
    modeButtons: [],
    navButtons: [],
    primaryAction: createButton(),
    windowMinimize: createButton(),
    windowMaximize: createButton(),
    windowClose: createButton(),
    resetDefaults: createButton(),
    logFilter: createEmitter(),
    logClear: createButton(),
    logExport: createButton(),
  };
  const calls = [];
  const adapter = {
    async setMode() {
      return {};
    },
    async start() {
      return {};
    },
    async stop() {
      return {};
    },
    async resetDefaults() {
      return {};
    },
    async hideToTray() {
      calls.push("hide");
      return true;
    },
    async minimizeWindow() {
      calls.push("minimize");
      return true;
    },
    async toggleMaximizeWindow() {
      calls.push("maximize");
      return true;
    },
    async clearLogs() {
      return {};
    },
    async exportLogs() {},
  };
  const windowState = [];

  bindActions({
    elements,
    getState: () => ({ mode: "client", status: "idle" }),
    setState: () => {},
    adapter,
    root,
    setActiveTab: () => {},
    isBusy: () => false,
    renderLogs: () => {},
    setWindowState: (next) => windowState.push(next),
  });

  const shortcut = async (key, options = {}) => {
    const event = {
      key,
      metaKey: false,
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      repeat: false,
      prevented: false,
      preventDefault() {
        this.prevented = true;
      },
      ...options,
    };
    await root.handlers.keydown(event);
    return event;
  };

  const minimizeEvent = await shortcut("m", { metaKey: true });
  const maximizeEvent = await shortcut("F11");
  const closeEvent = await shortcut("w", { ctrlKey: true });

  assert.equal(minimizeEvent.prevented, true);
  assert.equal(maximizeEvent.prevented, true);
  assert.equal(closeEvent.prevented, true);
  assert.deepEqual(calls, ["minimize", "maximize", "hide"]);
  assert.deepEqual(windowState, [{ maximized: true }]);
});

test("bindConfigInputs converts number and checkbox values", async () => {
  const numberInput = createInput({ field: "server_port", type: "number", value: "43001" });
  const checkboxInput = createInput({ field: "auto_reconnect", type: "checkbox", value: "on" });
  const appCheckbox = createAppInput({ field: "launch_at_login" });
  checkboxInput.checked = false;
  appCheckbox.checked = false;

  const root = {
    querySelectorAll: (selector) =>
      selector === "[data-app-field]" ? [appCheckbox] : [numberInput, checkboxInput],
    getElementById: () => null,
  };

  let receivedConfig = null;
  let launchAtLoginValue = null;
  const state = {
    mode: "client",
    client_config: { server_port: 43000, auto_reconnect: true },
    server_config: { listen_port: 43000, force_takeover: false, virtual_mic_enabled: false },
  };
  const adapter = {
    async setClientConfig(config) {
      receivedConfig = config;
      return { client_config: config };
    },
    async setServerConfig(config) {
      receivedConfig = config;
      return { server_config: config };
    },
    async setLaunchAtLogin(enabled) {
      launchAtLoginValue = enabled;
      return { app_settings: { launch_at_login: enabled } };
    },
    async forceDisconnect() {
      return {};
    },
  };

  bindConfigInputs({
    root,
    getState: () => state,
    setState: () => {},
    adapter,
  });

  await numberInput.handlers.change({ target: numberInput });
  assert.equal(receivedConfig.server_port, 43001);

  checkboxInput.checked = true;
  await checkboxInput.handlers.change({ target: checkboxInput });
  assert.equal(receivedConfig.auto_reconnect, true);

  appCheckbox.checked = true;
  await appCheckbox.handlers.change({ target: appCheckbox });
  assert.equal(launchAtLoginValue, true);
});

test("bindConfigInputs triggers forceDisconnect", async () => {
  const forceButton = createButton();
  const root = {
    querySelectorAll: () => [],
    getElementById: (id) => (id === "force-disconnect" ? forceButton : null),
  };
  let forced = false;
  const adapter = {
    async setClientConfig() {
      return {};
    },
    async setServerConfig() {
      return {};
    },
    async setLaunchAtLogin() {
      return {};
    },
    async forceDisconnect() {
      forced = true;
      return { status: "listening" };
    },
  };
  let lastSnapshot = null;

  bindConfigInputs({
    root,
    getState: () => ({
      mode: "server",
      client_config: {},
      server_config: { listen_port: 43000 },
    }),
    setState: (snapshot) => {
      lastSnapshot = snapshot;
    },
    adapter,
  });

  await forceButton.handlers.click();
  assert.equal(forced, true);
  assert.deepEqual(lastSnapshot, { status: "listening" });
});

test("bindActions forwards custom window controls", async () => {
  const elements = {
    modeButtons: [],
    navButtons: [],
    primaryAction: createButton(),
    windowMinimize: createButton(),
    windowMaximize: createButton(),
    windowClose: createButton(),
    resetDefaults: createButton(),
    logFilter: createEmitter(),
    logClear: createButton(),
    logExport: createButton(),
  };
  let hidden = 0;
  let minimized = 0;
  let maximizeState = null;
  const adapter = {
    async start() {
      return {};
    },
    async stop() {
      return {};
    },
    async hideToTray() {
      hidden += 1;
      return true;
    },
    async minimizeWindow() {
      minimized += 1;
      return true;
    },
    async toggleMaximizeWindow() {
      return true;
    },
    async resetDefaults() {
      return {};
    },
    async clearLogs() {
      return {};
    },
    async exportLogs() {},
    async setMode() {
      return {};
    },
  };

  bindActions({
    elements,
    getState: () => ({ mode: "client", status: "idle" }),
    setState: () => {},
    adapter,
    setActiveTab: () => {},
    isBusy: () => false,
    renderLogs: () => {},
    setWindowState: (snapshot) => {
      maximizeState = snapshot.maximized;
    },
  });

  await elements.windowMinimize.handlers.click();
  await elements.windowMaximize.handlers.click();
  await elements.windowClose.handlers.click();
  assert.equal(minimized, 1);
  assert.equal(maximizeState, true);
  assert.equal(hidden, 1);
});
