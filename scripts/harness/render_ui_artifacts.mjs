#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

import {
  renderConfig,
  renderLogs,
  renderPrimaryAction,
  renderStatus,
  renderStatusPill,
} from "../../apps/netmic-ui/ui/app.dom.js";

const SERVER_STATUS_POLL_MS = 1000;

const parseArgs = (argv) => {
  const args = { snapshot: "", outDir: "" };
  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--snapshot") {
      args.snapshot = argv[i + 1] || "";
      i += 1;
      continue;
    }
    if (arg === "--out-dir") {
      args.outDir = argv[i + 1] || "";
      i += 1;
      continue;
    }
    throw new Error(`未知参数：${arg}`);
  }
  if (!args.snapshot || !args.outDir) {
    throw new Error("用法：render_ui_artifacts.mjs --snapshot <path> --out-dir <path>");
  }
  return args;
};

const htmlToLines = (html) => {
  const compact = String(html)
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|div|h3|li)>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
  return compact
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
};

const makeButton = (mode) => ({
  dataset: { mode },
  disabled: false,
  classList: {
    toggle() {},
  },
  setAttribute() {},
});

const makeHolder = () => ({
  innerHTML: "",
  textContent: "",
  style: {},
  disabled: false,
  value: "all",
});

const createElements = () => {
  const statusDot = { style: {} };
  return {
    statusLabel: makeHolder(),
    statusNote: makeHolder(),
    statusPill: {
      style: {},
      querySelector(selector) {
        if (selector === ".status-dot") {
          return statusDot;
        }
        return { style: {} };
      },
    },
    primaryAction: makeHolder(),
    logFilter: makeHolder(),
    logList: makeHolder(),
    modeButtons: [makeButton("client"), makeButton("server")],
    configConnection: makeHolder(),
    configAudio: makeHolder(),
    configClient: makeHolder(),
    configServer: makeHolder(),
    configFallbacks: makeHolder(),
    statusConnection: makeHolder(),
    statusMetrics: makeHolder(),
    statusAudio: makeHolder(),
    statusParams: makeHolder(),
    statusEvents: makeHolder(),
  };
};

const isBusy = (snapshot) =>
  ["connecting", "streaming", "listening", "connected"].includes(snapshot.status);

const writeJson = (filePath, payload) => {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
};

const main = () => {
  const args = parseArgs(process.argv);
  const snapshot = JSON.parse(fs.readFileSync(args.snapshot, "utf8"));
  const elements = createElements();

  renderStatusPill({ state: snapshot, elements });
  renderPrimaryAction({ state: snapshot, elements, isBusy });
  renderStatus({
    state: snapshot,
    elements,
    ensureWaveformCanvas() {},
  });
  renderConfig({
    state: snapshot,
    elements,
    isBusy,
    sampleRates: [16000, 24000, 32000, 44100, 48000],
    chunkOptions: [10, 20, 40, 60],
    bufferOptions: [40, 60, 80, 100, 150, 200, 300, 400],
  });
  renderLogs({ state: snapshot, elements });

  const renderedAtMs = Date.now();
  const statusUpdatedMs = Number(snapshot.runtime?.server_status_updated_ms || 0);
  const statusAgeSec =
    statusUpdatedMs > 0 ? Math.max(0, Math.floor((renderedAtMs - statusUpdatedMs) / 1000)) : null;

  writeJson(path.join(args.outDir, "visible-status.json"), {
    status_label: elements.statusLabel.textContent,
    status_note: elements.statusNote.textContent,
    primary_action: elements.primaryAction.textContent,
    connection_lines: htmlToLines(elements.statusConnection.innerHTML),
    params_lines: htmlToLines(elements.statusParams.innerHTML),
    events_lines: htmlToLines(elements.statusEvents.innerHTML),
    effective_values: snapshot.effective || {},
  });

  writeJson(path.join(args.outDir, "visible-config.json"), {
    connection_lines: htmlToLines(elements.configConnection.innerHTML),
    audio_lines: htmlToLines(elements.configAudio.innerHTML),
    client_lines: htmlToLines(elements.configClient.innerHTML),
    server_lines: htmlToLines(elements.configServer.innerHTML),
    fallback_lines: htmlToLines(elements.configFallbacks.innerHTML),
    client_config_values: snapshot.client_config || {},
    server_config_values: snapshot.server_config || {},
    fallback_items: snapshot.fallbacks || [],
  });

  writeJson(path.join(args.outDir, "visible-logs.json"), {
    filter: elements.logFilter.value || "all",
    lines: htmlToLines(elements.logList.innerHTML),
  });

  writeJson(path.join(args.outDir, "refresh-check.json"), {
    ok: snapshot.mode !== "server" || statusUpdatedMs > 0,
    rendered_at_ms: renderedAtMs,
    server_status_updated_ms: statusUpdatedMs,
    server_status_age_sec: statusAgeSec,
    server_status_poll_ms: SERVER_STATUS_POLL_MS,
  });
};

main();
