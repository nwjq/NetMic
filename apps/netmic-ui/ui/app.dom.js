export const statusMap = {
  idle: { label: "空闲", color: "var(--muted)" },
  connecting: { label: "连接中", color: "var(--accent-2)" },
  listening: { label: "监听中", color: "var(--accent)" },
  connected: { label: "已连接", color: "var(--success)" },
  streaming: { label: "推流中", color: "var(--success)" },
  error: { label: "错误", color: "var(--danger)" },
};

export const formatNumber = (value, digits = 1) =>
  Number.isFinite(value) ? value.toFixed(digits) : "--";

export const formatMs = (value) => `${formatNumber(value)} ms`;

export const formatKbps = (value) => `${formatNumber(value)} kbps`;

export const formatDuration = (seconds) => {
  if (!Number.isFinite(seconds)) return "--";
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}m ${secs}s`;
};

export const formatTimestamp = (ms) => {
  const date = new Date(ms);
  return date.toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
};

export const renderStatusPill = ({ state, elements }) => {
  const config = statusMap[state.status] || statusMap.idle;
  elements.statusLabel.textContent = config.label;
  elements.statusNote.textContent = state.status_note || "";
  elements.statusPill.style.borderColor = config.color;
  elements.statusPill.querySelector(".status-dot").style.background = config.color;
};

export const renderModeButtons = ({ state, elements, isBusy }) => {
  elements.modeButtons.forEach((btn) => {
    const active = btn.dataset.mode === state.mode;
    btn.classList.toggle("active", active);
    btn.setAttribute("aria-selected", active ? "true" : "false");
    btn.disabled = isBusy(state);
  });
};

export const renderPrimaryAction = ({ state, elements, isBusy }) => {
  if (isBusy(state)) {
    elements.primaryAction.textContent = state.mode === "client" ? "停止推流" : "停止监听";
    return;
  }
  elements.primaryAction.textContent = state.mode === "client" ? "开始推流" : "开始监听";
};

export const renderLogs = ({ state, elements }) => {
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

export const renderConfig = ({
  state,
  elements,
  isBusy,
  sampleRates,
  chunkOptions,
  bufferOptions,
}) => {
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
};

export const renderStatus = ({ state, elements, ensureWaveformCanvas }) => {
  const lastError = state.runtime?.last_error;
  elements.statusConnection.innerHTML = `
    <h3>连接状态</h3>
    <p>${state.status_note || ""}</p>
    <div class="list">
      <div class="list-item">模式：${state.mode === "client" ? "Client" : "Server"}</div>
      <div class="list-item">状态：${statusMap[state.status]?.label || "--"}</div>
      <div class="list-item">连接时长：${formatDuration(state.runtime.connected_seconds)}</div>
      <div class="list-item">对端：${state.runtime.peer_addr || "未连接"}</div>
      ${lastError ? `<div class="list-item">错误提示：${lastError}</div>` : ""}
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
    <div class="waveform-block">
      <div class="waveform-header">
        <span>时域波形</span>
        <span class="waveform-note">20 FPS · 128 点</span>
      </div>
      <div class="waveform-shell">
        <canvas id="waveform-canvas" class="waveform-canvas"></canvas>
      </div>
    </div>
  `;
  if (ensureWaveformCanvas) {
    ensureWaveformCanvas();
  }

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
