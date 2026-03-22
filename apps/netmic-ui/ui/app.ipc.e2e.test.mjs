import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { defaultSnapshot } from "./app.core.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const buildDom = async () => {
  const html = await readFile(join(__dirname, "index.html"), "utf-8");
  const dom = new JSDOM(html, { url: "http://localhost" });

  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.Event = dom.window.Event;
  global.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);

  const canvasProto = dom.window.HTMLCanvasElement.prototype;
  Object.defineProperty(canvasProto, "getContext", {
    configurable: true,
    value: () => ({
      clearRect() {},
      stroke() {},
      beginPath() {},
      moveTo() {},
      lineTo() {},
      fillRect() {},
    }),
  });

  return dom;
};

const createTauriStub = () => {
  let snapshot = defaultSnapshot();
  const calls = [];
  const listeners = new Map();
  let closeHandler = null;

  const invoke = async (command, args = {}) => {
    calls.push({ command, args });
    switch (command) {
      case "get_status":
        return { ...snapshot };
      case "set_mode": {
        const mode = args.mode === "server" ? "server" : "client";
        snapshot = {
          ...snapshot,
          mode,
          status: "idle",
          status_note: "准备就绪",
          runtime: { ...snapshot.runtime, peer_addr: null, connected_seconds: 0 },
        };
        return { ...snapshot };
      }
      case "start": {
        if (snapshot.mode === "server") {
          snapshot = { ...snapshot, status: "listening", status_note: "等待客户端连接" };
        } else {
          snapshot = { ...snapshot, status: "connecting", status_note: "正在建立连接" };
        }
        return { ...snapshot };
      }
      case "stop": {
        snapshot = { ...snapshot, status: "idle", status_note: "已停止" };
        return { ...snapshot };
      }
      case "set_client_config": {
        snapshot = {
          ...snapshot,
          client_config: { ...snapshot.client_config, ...args.config },
        };
        return { ...snapshot };
      }
      case "set_server_config": {
        snapshot = {
          ...snapshot,
          server_config: { ...snapshot.server_config, ...args.config },
        };
        return { ...snapshot };
      }
      case "set_launch_at_login": {
        snapshot = {
          ...snapshot,
          app_settings: {
            ...snapshot.app_settings,
            launch_at_login: Boolean(args.enabled),
          },
        };
        return { ...snapshot };
      }
      case "reset_defaults": {
        snapshot = defaultSnapshot();
        return { ...snapshot };
      }
      case "clear_logs":
        snapshot = { ...snapshot, logs: [] };
        return { ...snapshot };
      case "export_logs":
        return { ok: true };
      case "report_harness_render":
        return true;
      default:
        return { ...snapshot };
    }
  };

  const event = {
    listen(name, handler) {
      listeners.set(name, handler);
    },
  };

  const webviewWindow = {
    getCurrentWebviewWindow() {
      let maximized = false;
      return {
        async onCloseRequested(handler) {
          closeHandler = handler;
          return () => {
            closeHandler = null;
          };
        },
        async minimize() {
          calls.push({ command: "window.minimize", args: {} });
          return true;
        },
        async toggleMaximize() {
          maximized = !maximized;
          calls.push({ command: "window.toggleMaximize", args: { maximized } });
          return true;
        },
        async isMaximized() {
          calls.push({ command: "window.isMaximized", args: { maximized } });
          return maximized;
        },
      };
    },
  };

  const emit = (name, payload) => {
    const handler = listeners.get(name);
    if (handler) {
      handler({ payload });
    }
  };

  const close = async () => {
    if (!closeHandler) return;
    const event = {
      prevented: false,
      preventDefault() {
        this.prevented = true;
      },
    };
    await closeHandler(event);
    return event;
  };

  return { invoke, event, emit, calls, close, webviewWindow };
};

test("ipc adapter calls invoke and updates UI from snapshot", async (t) => {
  const dom = await buildDom();
  t.after(() => {
    dom.window.close();
    delete global.window;
    delete global.document;
    delete global.HTMLElement;
    delete global.Event;
  });
  const tauri = createTauriStub();
  global.window.__TAURI__ = {
    invoke: tauri.invoke,
    event: tauri.event,
    webviewWindow: tauri.webviewWindow,
  };

  await import("./app.js");

  const modeButtons = document.querySelectorAll(".mode-btn");
  const primary = document.getElementById("primary-action");
  const statusNote = document.getElementById("status-note");

  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.ok(tauri.calls.some((call) => call.command === "get_status"));
  assert.ok(tauri.calls.some((call) => call.command === "report_harness_render"));

  modeButtons[1].dispatchEvent(new window.Event("click"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.ok(tauri.calls.some((call) => call.command === "set_mode"));

  primary.dispatchEvent(new window.Event("click"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.ok(tauri.calls.some((call) => call.command === "start"));

  tauri.emit("netmic://snapshot", {
    ...defaultSnapshot(),
    status: "error",
    status_note: "来自 IPC 事件",
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(statusNote.textContent, "来自 IPC 事件");
  const renderAck = tauri.calls
    .filter((call) => call.command === "report_harness_render")
    .at(-1);
  assert.equal(renderAck.args.ack.visible.status_note, "来自 IPC 事件");
  assert.ok(renderAck.args.ack.visible.connection_lines.length > 0);
  assert.ok(
    renderAck.args.ack.visible.params_lines.some((line) => line.startsWith("Codec："))
  );
  assert.ok(renderAck.args.ack.visible.log_lines.length > 0);
});

test("render ack falls back when requestAnimationFrame never fires", async (t) => {
  const dom = await buildDom();
  t.after(() => {
    dom.window.close();
    delete global.window;
    delete global.document;
    delete global.HTMLElement;
    delete global.Event;
  });
  global.window.requestAnimationFrame = () => 1;
  const tauri = createTauriStub();
  global.window.__TAURI__ = {
    invoke: tauri.invoke,
    event: tauri.event,
    webviewWindow: tauri.webviewWindow,
  };

  await import(`./app.js?fallback=${Date.now()}`);

  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.ok(tauri.calls.some((call) => call.command === "report_harness_render"));
});

test("ipc adapter accepts core.invoke without event bridge", async (t) => {
  const dom = await buildDom();
  t.after(() => {
    dom.window.close();
    delete global.window;
    delete global.document;
    delete global.HTMLElement;
    delete global.Event;
  });
  const tauri = createTauriStub();
  global.window.__TAURI__ = {
    core: { invoke: tauri.invoke },
    webviewWindow: tauri.webviewWindow,
  };

  await import(`./app.js?invoke-only=${Date.now()}`);

  await new Promise((resolve) => setTimeout(resolve, 25));
  assert.ok(tauri.calls.some((call) => call.command === "get_status"));
  assert.ok(tauri.calls.some((call) => call.command === "report_harness_render"));
});

test("close request is intercepted and forwarded to hide_to_tray", async (t) => {
  const dom = await buildDom();
  t.after(() => {
    dom.window.close();
    delete global.window;
    delete global.document;
    delete global.HTMLElement;
    delete global.Event;
  });
  const tauri = createTauriStub();
  global.window.__TAURI__ = {
    invoke: tauri.invoke,
    event: tauri.event,
    webviewWindow: tauri.webviewWindow,
  };

  await import(`./app.js?close=${Date.now()}`);
  await new Promise((resolve) => setTimeout(resolve, 25));

  const closeEvent = await tauri.close();
  assert.equal(closeEvent.prevented, true);
  assert.ok(tauri.calls.some((call) => call.command === "hide_to_tray"));
});
