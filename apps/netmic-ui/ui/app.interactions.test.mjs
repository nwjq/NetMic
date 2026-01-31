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

test("bindActions triggers mode change when idle", async () => {
  const elements = {
    modeButtons: [createButton({ mode: "client" }), createButton({ mode: "server" })],
    navButtons: [createButton({ tab: "config" })],
    primaryAction: createButton(),
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
    setActiveTab: () => {},
    isBusy: () => false,
    renderLogs: () => {},
  });

  await elements.modeButtons[1].handlers.click();
  assert.equal(setModeCalled, "server");
  assert.deepEqual(setStateValue, { mode: "server" });
});

test("bindActions blocks mode change when busy", async () => {
  const elements = {
    modeButtons: [createButton({ mode: "client" })],
    navButtons: [],
    primaryAction: createButton(),
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
    setActiveTab: () => {},
    isBusy: () => true,
    renderLogs: () => {},
  });

  await elements.modeButtons[0].handlers.click();
  assert.equal(setModeCalled, null);
});

test("bindActions toggles start/stop via primaryAction", async () => {
  const elements = {
    modeButtons: [],
    navButtons: [],
    primaryAction: createButton(),
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
    setActiveTab: () => {},
    isBusy: () => busy,
    renderLogs: () => {},
  });

  await elements.primaryAction.handlers.click();
  assert.deepEqual(lastSnapshot, { status: "streaming" });
  busy = true;
  await elements.primaryAction.handlers.click();
  assert.deepEqual(lastSnapshot, { status: "idle" });
});

test("bindConfigInputs converts number and checkbox values", async () => {
  const numberInput = createInput({ field: "server_port", type: "number", value: "43001" });
  const checkboxInput = createInput({ field: "auto_reconnect", type: "checkbox", value: "on" });
  checkboxInput.checked = false;

  const root = {
    querySelectorAll: () => [numberInput, checkboxInput],
    getElementById: () => null,
  };

  let receivedConfig = null;
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
