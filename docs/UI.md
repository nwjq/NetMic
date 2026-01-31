# NetMic MVP UI 视图模型与 IPC（草案）

本文档描述当前 Tauri UI 的最小视图模型与 IPC 约定。
事实来源：`MVP.md` 第 3/7/8 节；实现位置：`apps/netmic-ui/`。

## 设计边界
- UI 仅覆盖 MVP 需求：配置 / 状态 / 日志。
- 参数安全范围仍以 `MVP.md` 为准；UI 只是输入与提示层。
- IPC 采用最小命令集：`get_status` / `set_config` / `start` / `stop` / `set_mode`。

## 视图模型（UiSnapshot）

### 顶层结构
- `mode: "client" | "server"`
- `status: "idle" | "connecting" | "listening" | "connected" | "streaming" | "error"`
- `status_note: String`
- `config: UiConfig`
- `effective: SessionParams`（`netmic-proto`）
- `fallbacks: UiFallbackEvent[]`
- `metrics: UiMetrics`
- `runtime: UiRuntime`
- `devices: UiDevices`
- `logs: UiLogEntry[]`

### UiConfig（请求参数 + UI 配置）
- `server_addr: String`
- `server_port: u16`
- `listen_port: u16`
- `input_device: String`
- `codec: String`（opus / pcm16）
- `sample_rate_hz: u32`
- `channels: u16`（MVP 固定为 1）
- `chunk_ms: u32`
- `opus_bitrate_kbps: u32`
- `jitter_buffer_ms: u32`
- `auto_reconnect: bool`
- `force_takeover: bool`（默认 false）
- `pairing_token: String`（预留）
- `virtual_mic_enabled: bool`

### UiMetrics（展示指标）
- `rtt_ms: f32`
- `packet_loss_pct: f32`
- `buffer_depth_ms: f32`
- `jitter_buffer_depth_ms: f32`
- `estimated_e2e_latency_ms: f32`
- `audio_rms: f32`
- `audio_peak: u32`
- `uplink_kbps: f32`

### UiWaveform（时域波形事件载荷）
- `ts_ms: u64`（毫秒时间戳）
- `points: f32[]`（长度 128，范围 [-1, 1]，20 FPS）

### UiRuntime（运行态）
- `peer_addr: Option<String>`
- `connected_seconds: u64`
- `reconnect_attempts: u32`
- `mic_permission: String`（macOS 权限状态：已授权 / 未授权 / 不可用 / 未知 / 不适用）
- `virtual_mic_name: String`

### UiDevices
- `input: String[]`（输入设备列表，运行时从系统枚举刷新）

### UiFallbackEvent
- `field: String`
- `requested: String`
- `applied: String`
- `reason: String`

### UiLogEntry
- `ts_ms: u64`
- `level: "info" | "warn" | "error"`
- `message: String`

## IPC 命令
- `get_status() -> UiSnapshot`
- `set_mode(mode: String) -> UiSnapshot`
- `set_config(config: UiConfig) -> UiSnapshot`
- `reset_defaults() -> UiSnapshot`
- `start() -> UiSnapshot`
- `stop() -> UiSnapshot`
- `force_disconnect() -> UiSnapshot`
- `clear_logs() -> UiSnapshot`
- `export_logs() -> { ok: bool }`

## 事件
- `netmic://snapshot`：UI 订阅后接收 `UiSnapshot` 推送
- `netmic://waveform`：客户端采集到的时域波形（20 FPS / 128 点）

## UI 对齐 MVP 的关键点
- 参数范围：采样率 / 帧长 / bitrate / buffer 必须限制在 `MVP.md` 范围内
- 回退提示：展示 `fallbacks` 列表用于提示“已回退到 X”
- 状态页：连接状态 + 指标 + 当前生效参数
- 日志页：按级别过滤 + 导出入口（MVP）

## macOS 权限说明
- 需要在 `apps/netmic-ui/src-tauri/Info.plist` 提供 `NSMicrophoneUsageDescription`
